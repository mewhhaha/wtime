"""Pair decoded executables over GC/exception cold controls and GC guest stages."""
from __future__ import annotations

import hashlib
import importlib.util
import json
import math
import os
from pathlib import Path
import random
import subprocess
import sys
from typing import Any, Callable

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
PROFILE = "all-phase3-core"
MODE = "decoded"
STAGES = ("prepare", "instantiate", "first_call", "warm_call", "load_and_call")
TARGET_NS = 20_000_000.0
MAX_BATCH = 100_000
MAX_PRODUCT = 100_000_000
MAX_ATTEMPTS = 8
MEASUREMENT_POLICY = "sum_guest_invoke_intervals_excluding_each_oracle_and_gc_reset"
COLD_ADAPTER_PATH = Path(__file__).with_name("gc_cold_adapter.py")
CONTRACT_PATH = ROOT / "docs/benchmarks/performance/P033-contract.json"
LAYOUT_CONTRACT_PATH = ROOT / "docs/benchmarks/performance/P034-contract.json"
LAYOUT_MEASUREMENT_CONTRACT_PATH = ROOT / "docs/benchmarks/performance/P034-measurement-contract.json"
WARM_DRIVER_NAME = "gc_scalar_stages"
OLD_MODEL_NAME = "gc_workloads.py"
WARM_MODEL_NAME = "gc_warm_workloads.py"
LAYOUT_MODEL_NAME = "gc_layout_workloads.py"


class GcComparisonError(ValueError):
    pass


def sha(path: Path) -> str:
    digest = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def canonical(row: Any) -> str:
    return json.dumps(row, sort_keys=True, separators=(",", ":"), allow_nan=False)


def _positive_finite(value: Any, label: str, workload: str) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise GcComparisonError(f"{workload}: {label} is not numeric")
    if not math.isfinite(value) or value <= 0:
        raise GcComparisonError(f"{workload}: {label} must be finite and positive")
    return float(value)


def _expected_profile(entry: dict[str, Any]) -> str:
    if entry.get("oracle", {}).get("module") in {WARM_MODEL_NAME, LAYOUT_MODEL_NAME}:
        driver = entry.get("driver")
        if not isinstance(driver, dict) or driver.get("kind") != WARM_DRIVER_NAME:
            raise GcComparisonError(f"{entry.get('id')}: warm GC driver metadata is missing")
        if driver.get("profile") != PROFILE:
            raise GcComparisonError(f"{entry['id']}: GC warm profile must be {PROFILE}")
    return PROFILE


def selected_model_rows(registry: dict[str, Any], old_model, warm_model) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    """Require exact old-model parity and append-only equality for warm rows."""
    try:
        old_rows = old_model.entries()
        warm_rows = warm_model.entries()
    except Exception as exc:
        raise GcComparisonError(f"GC independent workload model is not fully hash-bound: {exc}") from exc
    if len(old_rows) != 45:
        raise GcComparisonError(f"expected the unchanged 45 legacy GC/exception model rows, found {len(old_rows)}")
    if len(warm_rows) != 36:
        raise GcComparisonError(f"expected 36 append-only GC warm rows, found {len(warm_rows)}")
    rows = registry.get("workloads")
    selections = registry.get("selections")
    selection = selections.get("gc-exceptions") if isinstance(selections, dict) else None
    ids = selection.get("workloads") if isinstance(selection, dict) else None
    if not isinstance(rows, list) or not isinstance(selection, dict) or selection.get("status") != "ready" or not isinstance(ids, list):
        raise GcComparisonError("gc-exceptions selection must be ready")
    if any(not isinstance(entry, dict) or not isinstance(entry.get("id"), str) for entry in rows):
        raise GcComparisonError("registry contains malformed workload IDs")
    row_ids = [entry["id"] for entry in rows]
    if len(row_ids) != len(set(row_ids)):
        raise GcComparisonError("registry contains duplicate workload IDs")
    if any(not isinstance(wid, str) for wid in ids) or len(ids) != 81 or len(ids) != len(set(ids)):
        raise GcComparisonError("gc-exceptions selection must contain 45 legacy and 36 warm unique IDs")
    if (any(not isinstance(row, dict) or not isinstance(row.get("id"), str) for row in old_rows)
            or any(not isinstance(row, dict) or not isinstance(row.get("id"), str) for row in warm_rows)):
        raise GcComparisonError("independent GC models contain malformed workload IDs")
    old_ids = [row["id"] for row in old_rows]
    warm_ids = [row["id"] for row in warm_rows]
    if ids != old_ids + warm_ids:
        raise GcComparisonError("selection is not the exact legacy-45 prefix plus the full warm model")
    by_id = {entry["id"]: entry for entry in rows}
    for expected in old_rows + warm_rows:
        actual = by_id.get(expected.get("id"))
        if actual is None or canonical(actual) != canonical(expected):
            raise GcComparisonError(f"{expected.get('id')}: registry row differs from its independent model")
    for row in old_rows:
        if row.get("stages") != ["load_and_call"] or row.get("reset_policy") != "fresh_instance_per_invocation":
            raise GcComparisonError(f"{row['id']}: P011 cold row contract was changed")
    for row in warm_rows:
        if row.get("stages") != list(STAGES) or row.get("reset_policy") != "fresh_instance_for_cold_stages_reused_instance_for_warm_call":
            raise GcComparisonError(f"{row['id']}: warm row stage/reset contract is invalid")
        if row.get("comparators", {}).get("wasmtime", {}).get("status") != "unsupported":
            raise GcComparisonError(f"{row['id']}: GC cannot declare Wasmtime support or parity")
    return old_rows, warm_rows


def selected_layout_model_rows(registry: dict[str, Any], old_rows: list[dict[str, Any]],
                               warm_rows: list[dict[str, Any]], layout_model) -> list[dict[str, Any]]:
    """Validate P034's additive layout model while preserving the P033 control prefix."""
    try:
        layout_rows = layout_model.entries()
    except Exception as exc:
        raise GcComparisonError(f"GC layout model is not fully hash-bound: {exc}") from exc
    if len(layout_rows) != 36:
        raise GcComparisonError(f"expected 36 append-only GC layout rows, found {len(layout_rows)}")
    selections = registry.get("selections", {})
    control = selections.get("gc-exceptions") if isinstance(selections, dict) else None
    selected = selections.get("gc-layouts") if isinstance(selections, dict) else None
    if (not isinstance(control, dict) or control.get("status") != "ready"
            or control.get("workloads") != [row["id"] for row in old_rows + warm_rows]
            or not isinstance(selected, dict) or selected.get("status") != "ready"):
        raise GcComparisonError("gc-layouts must preserve the exact ready gc-exceptions control prefix")
    expected_rows = old_rows + warm_rows + layout_rows
    expected_ids = [row["id"] for row in expected_rows]
    selected_ids = selected.get("workloads")
    if selected_ids != expected_ids or len(selected_ids) != 117:
        raise GcComparisonError("gc-layouts must append exactly 36 model rows after the unchanged 81-row control")
    rows = registry.get("workloads")
    if not isinstance(rows, list):
        raise GcComparisonError("workload registry is malformed")
    by_id = {row.get("id"): row for row in rows if isinstance(row, dict)}
    if len(by_id) != len(rows):
        raise GcComparisonError("registry contains malformed or duplicate workload IDs")
    for expected in expected_rows:
        actual = by_id.get(expected["id"])
        if actual is None or canonical(actual) != canonical(expected):
            raise GcComparisonError(f"{expected['id']}: registry row differs from its independent model")
    for row in layout_rows:
        if row.get("stages") != list(STAGES) or row.get("reset_policy") != "fresh_instance_for_cold_stages_reused_instance_for_warm_call":
            raise GcComparisonError(f"{row['id']}: layout stage/reset contract is invalid")
        if row.get("comparators", {}).get("wasmtime", {}).get("status") != "unsupported":
            raise GcComparisonError(f"{row['id']}: GC layout cannot declare Wasmtime support or parity")
    return layout_rows


def validate_contract(contract: dict[str, Any], selected_ids: list[str], warm_ids: list[str]) -> dict[str, Any]:
    """Require the predeclared 12/69 P033 partition over all 81 selected IDs."""
    if not isinstance(contract, dict) or set(contract) != {
            "schema_version", "selection", "primary_workloads", "protected_workloads"}:
        raise GcComparisonError("P033 contract schema has missing or unknown fields")
    primary = contract.get("primary_workloads")
    protected = contract.get("protected_workloads")
    if (not isinstance(primary, list) or any(not isinstance(wid, str) for wid in primary)
            or not isinstance(protected, list) or any(not isinstance(wid, str) for wid in protected)
            or any(not isinstance(wid, str) for wid in selected_ids)
            or any(not isinstance(wid, str) for wid in warm_ids)):
        raise GcComparisonError("P033 contract and selection IDs must be string arrays")
    if (type(contract.get("schema_version")) is not int or contract.get("schema_version") != 1
            or contract.get("selection") != "gc-exceptions"
            or not isinstance(primary, list) or len(primary) != 12 or len(set(primary)) != 12
            or not isinstance(protected, list) or len(protected) != 69 or len(set(protected)) != 69
            or set(primary) & set(protected)
            or set(primary) | set(protected) != set(selected_ids)
            or len(selected_ids) != 81 or len(set(selected_ids)) != 81):
        raise GcComparisonError("P033 contract must partition the frozen 81-row selection into 12 primary and 69 protected workloads")
    if set(primary) - set(warm_ids):
        raise GcComparisonError("P033 primary workloads must be selected warm GC rows")
    if not set(warm_ids) <= set(primary) | set(protected):
        raise GcComparisonError("P033 contract omitted a selected warm GC row")
    return {"primary_workloads": list(primary), "protected_workloads": list(protected),
            "primary_workload_count": len(primary), "protected_workload_count": len(protected)}


def validate_layout_contract(contract: dict[str, Any], old_rows: list[dict[str, Any]],
                              warm_rows: list[dict[str, Any]], layout_rows: list[dict[str, Any]],
                              selected_ids: list[str], layout_model,
                              registry: dict[str, Any], measurement: dict[str, Any]) -> dict[str, Any]:
    """Validate the gate's four-key 24/93 contract and derive the P034 matrix."""
    primary = contract.get("primary_workloads")
    protected = contract.get("protected_workloads")
    layout_ids = [row["id"] for row in layout_rows]
    expected_primary = [row["id"] for row in layout_rows
                        if row["params"][0]["baseline"] in layout_model.PRIMARY_INPUTS]
    expected_protected = ([row["id"] for row in old_rows + warm_rows]
                          + [row["id"] for row in layout_rows
                             if row["params"][0]["baseline"] in layout_model.HELD_OUT_INPUTS])
    if (set(contract) != {"schema_version", "selection", "primary_workloads", "protected_workloads"}
            or contract.get("schema_version") != 1 or contract.get("selection") != "gc-layouts"
            or not isinstance(primary, list) or not isinstance(protected, list)
            or primary != expected_primary or protected != expected_protected
            or set(primary) & set(protected) or set(primary) | set(protected) != set(selected_ids)):
        raise GcComparisonError("P034 gate contract must contain exactly the four declared fields and partition the selection")
    expected_group_count = len(old_rows) + (len(warm_rows) + len(layout_rows)) * len(STAGES)
    expected_pair_count = expected_group_count * 2 * 30
    expected_calibrations = (len(warm_rows) + len(layout_rows)) * 2
    expected_probes = len(old_rows + warm_rows + layout_rows) * 2
    if (len(layout_ids) != 36 or len(old_rows) != 45 or len(warm_rows) != 36
            or len(set(selected_ids)) != 117 or len(primary) != 24 or len(protected) != 93
            or expected_group_count != 405 or expected_pair_count != 24300
            or expected_calibrations != 144 or expected_probes != 234):
        raise GcComparisonError("P034 selected ID inventory is not 117 unique rows")
    rows = registry.get("workloads", [])
    selections = registry.get("selections", {})
    old_rows_prefix = rows[:350]
    old_selections = {name: value for name, value in selections.items() if name != "gc-layouts"}
    expected_old_rows_sha = "8e3c6235358590cc617865bff1f781cd08a1ca97636276651c1153bc2e1cd50f"
    expected_old_selections_sha = "b0600763b909ca211fd85ea870b768bbfec5991718d24ecba363995be5d0a5ff"
    actual_old_rows_sha = layout_model.canonical_sha(old_rows_prefix)
    actual_old_selections_sha = layout_model.canonical_sha(old_selections)
    actual_new_rows_sha = layout_model.canonical_sha(layout_rows)
    compact_bytes = len(layout_model.compact_registry_bytes(registry))
    calibration = measurement.get("calibration", {})
    group_derivation = measurement.get("group_derivation", {})
    if (len(rows) != 386 or len(old_rows_prefix) != 350
            or actual_old_rows_sha != expected_old_rows_sha
            or actual_old_selections_sha != expected_old_selections_sha
            or measurement.get("task") != "P034"
            or measurement.get("schema_version") != 1
            or measurement.get("selection") != "gc-layouts"
            or measurement.get("candidate_mode") != MODE or measurement.get("profile") != PROFILE
            or measurement.get("registry_rows") != 386
            or measurement.get("old_prefix_rows") != 350
            or measurement.get("selection_count") != 117
            or measurement.get("primary_count") != 24
            or measurement.get("protected_count") != 93
            or measurement.get("stages_new_rows") != list(STAGES)
            or measurement.get("runs") != 2 or measurement.get("pairs_per_group") != 30
            or measurement.get("old_logical_workloads_sha256") != expected_old_rows_sha
            or measurement.get("old_logical_selections_sha256") != expected_old_selections_sha
            or measurement.get("new_rows_sha256") != actual_new_rows_sha
            or measurement.get("gate_contract_canonical_sha256") != layout_model.canonical_sha(contract)
            or measurement.get("groups_per_run") != expected_group_count
            or measurement.get("groups_total") != expected_group_count * 2
            or measurement.get("pairs_total") != expected_pair_count
            or measurement.get("warm_calibrations_total") != expected_calibrations
            or measurement.get("workload_sides_per_run") != expected_probes
            or group_derivation != {"old_cold": 45, "old_warm": 180, "new_layout": 180}
            or calibration.get("target_ns") != TARGET_NS
            or calibration.get("max_attempts") != MAX_ATTEMPTS
            or calibration.get("shared_matched_batch") is not True
            or calibration.get("max_batch_rule") != "min(100000,100000000//max(n,1))"
            or measurement.get("allocation_rule") != "one object per invocation even n0; collect/release outside each guest interval"
            or measurement.get("compact_proposed_bytes") != compact_bytes
            or compact_bytes >= 524288):
        raise GcComparisonError("P034 measurement contract or frozen predecessor registry proof does not match selected rows")
    return {"primary_workloads": list(primary), "protected_workloads": list(protected),
            "primary_workload_count": len(primary), "protected_workload_count": len(protected),
            "workload_count": len(selected_ids), "groups_per_run": expected_group_count,
            "groups_total": expected_group_count * 2, "pairs_per_group": 30,
            "pairs_total": expected_pair_count, "warm_calibrations_total": expected_calibrations,
            "correctness_probes_total": expected_probes}


def _logical_fixture_path(raw: Any, workload: str) -> str:
    """Validate a registry fixture path and return a stable repo-relative source key."""
    if not isinstance(raw, str):
        raise GcComparisonError(f"{workload}: fixture source path must be a string")
    relative = Path(raw)
    if (relative.is_absolute() or ".." in relative.parts or len(relative.parts) != 3
            or relative.parts[0] != "fixtures" or relative.parts[1] not in {"gc", "exceptions"}
            or relative.suffix != ".wat"):
        raise GcComparisonError(f"{workload}: fixture source must be a safe fixtures/gc or fixtures/exceptions WAT path")
    return (Path("tools/wtime-bench") / relative).as_posix()


def _input(entry: dict[str, Any]) -> int:
    params = entry.get("params")
    if (not isinstance(params, list) or len(params) != 1 or not isinstance(params[0], dict)
            or params[0].get("name") != "n" or params[0].get("type") != "i32"):
        raise GcComparisonError(f"{entry.get('id')}: exactly one i32 n parameter is required")
    value = params[0].get("baseline")
    if (type(value) is not int or value < 0 or type(params[0].get("min")) is not int
            or type(params[0].get("max")) is not int
            or params[0].get("min") != value or params[0].get("max") != value):
        raise GcComparisonError(f"{entry['id']}: n must be a fixed nonnegative i32")
    return value


def _normalize_cold(entry: dict[str, Any], sample: dict[str, Any], model) -> dict[str, Any]:
    """Normalize cold adapter's wire result to the model's integer/exception result."""
    n = _input(entry)
    expected = model.oracle(entry["id"], n)
    row = sample.get("wire_observation")
    if not isinstance(row, dict):
        raise GcComparisonError(f"{entry['id']}: cold proof omitted its raw driver observation")
    if isinstance(expected, int):
        if (row.get("outcome", "returned") != "returned" or row.get("result") != f"i32:{expected}"
                or row.get("tag") is not None or row.get("payload") is not None):
            raise GcComparisonError(f"{entry['id']}: cold result differs from the independent oracle")
        normalized = expected
    elif isinstance(expected, dict) and expected.get("kind") == "exception":
        payload = row.get("payload")
        want_payload = [f"i32:{value}" for value in expected.get("payload", [])]
        if (row.get("outcome") != "exception" or row.get("result") is not None
                or row.get("tag") != expected.get("tag") or payload != want_payload):
            raise GcComparisonError(f"{entry['id']}: cold normalized exception differs from the oracle")
        normalized = expected
    else:
        raise GcComparisonError(f"{entry['id']}: unsupported independent oracle result")
    if (sample.get("workload") != entry["id"] or sample.get("expected") != expected
            or sample.get("stage") != "load_and_call" or sample.get("batch") != 1
            or sample.get("mode") != MODE or sample.get("profile") != PROFILE
            or sample.get("result") != row.get("result")
            or sample.get("outcome") != row.get("outcome", "returned")
            or sample.get("tag") != row.get("tag") or sample.get("payload") != row.get("payload")):
        raise GcComparisonError(f"{entry['id']}: cold adapter wire-copy/stage/settings mismatch")
    _positive_finite(sample.get("ns_per_operation"), "cold ns_per_operation", entry["id"])
    result = dict(sample)
    result["expected"] = normalized
    result["result"] = normalized
    result["outcome"] = "exception" if isinstance(normalized, dict) else "returned"
    return result


def validate_warm_observation(entry: dict[str, Any], row: dict[str, Any], *, stage: str,
                              batch: int, mode: str, expected: int,
                              expected_max_batch: int | None = None) -> dict[str, Any]:
    """Fail-closed validation for root's schema-v2 GC scalar driver wire row."""
    workload = entry.get("id", "unknown")
    n = _input(entry)
    profile = _expected_profile(entry)
    wanted = {"schema_version": 2, "driver": WARM_DRIVER_NAME, "case": workload,
              "family": entry.get("family"), "input": n, "mode": mode,
              "profile": profile, "stage": stage, "batch": batch,
              "instrumented": False, "result": f"i32:{expected}",
              "measurement_policy": MEASUREMENT_POLICY, "report_complete": True}
    if not isinstance(row, dict) or any(row.get(key) != value for key, value in wanted.items()):
        raise GcComparisonError(f"{workload}/{stage}: GC warm response identity/policy/result mismatch")
    for field in ("input", "batch", "max_batch", "calls_checked", "total_reclaimed",
                  "reclaimed_per_call", "live_after_reset", "payload_bytes_after_reset",
                  "continuation_bytes_after_reset"):
        if type(row.get(field)) is not int:
            raise GcComparisonError(f"{workload}/{stage}: {field} must be a JSON integer")
    if row.get("instrumented") is not False or row.get("report_complete") is not True:
        raise GcComparisonError(f"{workload}/{stage}: instrumented/completeness flags are invalid")
    if type(expected) is not int:
        raise GcComparisonError(f"{workload}/{stage}: independent result oracle must be i32")
    if stage not in STAGES or type(batch) is not int or not 1 <= batch <= MAX_BATCH:
        raise GcComparisonError(f"{workload}/{stage}: invalid stage or batch")
    cap = min(MAX_BATCH, MAX_PRODUCT // max(n, 1))
    if batch > cap or row.get("max_batch") != cap:
        raise GcComparisonError(f"{workload}/{stage}: call/input product exceeds the driver cap")
    if stage != "warm_call" and batch != 1:
        raise GcComparisonError(f"{workload}/{stage}: cold stage batch must be one")
    if expected_max_batch is not None and cap != expected_max_batch:
        raise GcComparisonError(f"{workload}: driver max_batch differs from calibration cap")
    calls = row.get("calls_checked")
    wanted_calls = batch + 1 if stage == "warm_call" else 1
    if type(calls) is not int or calls != wanted_calls:
        raise GcComparisonError(f"{workload}/{stage}: timed/priming call count mismatch")
    expectation = entry.get("gc_expectation")
    module_name = entry.get("oracle", {}).get("module")
    if (not isinstance(expectation, dict)
            or ("oracle_result" in expectation and expectation.get("oracle_result") != expected)
            or ("oracle_result" not in expectation and module_name != LAYOUT_MODEL_NAME)):
        raise GcComparisonError(f"{workload}: GC reset/result oracle metadata is missing or inconsistent")
    per_call = expectation.get("allocation_count")
    if type(per_call) is not int or per_call < 0 or row.get("reclaimed_per_call") != per_call:
        raise GcComparisonError(f"{workload}/{stage}: per-call GC reclaim count differs from model")
    if row.get("total_reclaimed") != wanted_calls * per_call:
        raise GcComparisonError(f"{workload}/{stage}: total GC reclaim count differs from exact calls")
    for field in ("live_after_reset", "payload_bytes_after_reset", "continuation_bytes_after_reset"):
        if type(row.get(field)) is not int or row[field] != 0:
            raise GcComparisonError(f"{workload}/{stage}: {field} must be exactly zero")
    sample_ns = _positive_finite(row.get("sample_ns"), "sample_ns", workload)
    measured_ns = _positive_finite(row.get("measured_guest_ns"), "measured_guest_ns", workload)
    for field in ("prepare_ns", "instantiate_ns", "first_call_ns", "reset_ns"):
        _positive_finite(row.get(field), field, workload)
    expected_interval = {
        "prepare": row["prepare_ns"],
        "instantiate": row["instantiate_ns"],
        "first_call": row["first_call_ns"],
        "warm_call": measured_ns / batch,
        "load_and_call": row["prepare_ns"] + row["instantiate_ns"] + row["first_call_ns"],
    }[stage]
    if not math.isclose(sample_ns, expected_interval, rel_tol=1e-9, abs_tol=1e-6):
        raise GcComparisonError(f"{workload}/{stage}: sample_ns differs from its declared timing interval")
    expected_guest = measured_ns if stage == "warm_call" else row["first_call_ns"]
    if not math.isclose(measured_ns, expected_guest, rel_tol=1e-9, abs_tol=1e-6):
        raise GcComparisonError(f"{workload}/{stage}: measured_guest_ns interval is inconsistent")
    normalized = dict(row)
    normalized.update(ns_per_operation=sample_ns, expected=expected,
                      result=expected, raw_wire_observation=row)
    return normalized


def _calibration_cap(input_value: int) -> int:
    return min(MAX_BATCH, MAX_PRODUCT // max(input_value, 1))


def calibrate_shared(entry: dict[str, Any], mode: str, expected: int,
                     observe: Callable[[str, int], dict[str, Any]], *,
                     target_ns: float = TARGET_NS, max_attempts: int = MAX_ATTEMPTS) -> dict[str, Any]:
    """Choose one exact batch confirmed on both sides; preserve failed attempts."""
    if not math.isfinite(target_ns) or target_ns <= 0 or type(max_attempts) is not int or not 1 <= max_attempts <= MAX_ATTEMPTS:
        raise GcComparisonError("invalid GC warm calibration target/attempt limit")
    n = _input(entry)
    cap = _calibration_cap(n)
    batch = 1
    attempts: list[dict[str, Any]] = []
    for number in range(1, max_attempts + 1):
        sides = []
        for side in ("baseline", "candidate"):
            raw = observe(side, batch)
            normalized = validate_warm_observation(entry, raw, stage="warm_call", batch=batch,
                                                   mode=mode, expected=expected,
                                                   expected_max_batch=cap)
            sides.append({"side": side, "sample_ns": normalized["ns_per_operation"],
                          "measured_guest_ns": raw["measured_guest_ns"],
                          "calls_checked": raw["calls_checked"],
                          "max_batch": raw["max_batch"], "wire": raw})
        attempts.append({"attempt": number, "batch": batch, "sides": sides})
        if all(side["measured_guest_ns"] >= target_ns for side in sides):
            return {"target_ns": target_ns, "max_batch": cap, "max_attempts": max_attempts,
                    "paired_batch": batch, "attempts": attempts, "sides": sides,
                    "policy": "shared_exact_batch_both_sides_confirmed"}
        if number == max_attempts or batch >= cap:
            break
        fastest = min(side["sample_ns"] for side in sides)
        if fastest <= target_ns * 1.1 / cap:
            grown = cap
        else:
            projected = target_ns / fastest * 1.1
            grown = cap if not math.isfinite(projected) or projected >= cap else math.ceil(projected)
        batch = min(cap, max(batch + 1, grown))
    raise GcComparisonError(
        f"{entry['id']}: shared GC warm calibration failed to confirm both sides at {target_ns:g} ns; "
        f"audit={json.dumps(attempts, sort_keys=True, allow_nan=False)}")


def pair_count(old_count: int = 45, warm_count: int = 36, pairs: int = 30) -> dict[str, int]:
    if (type(old_count) is not int or old_count != 45 or type(warm_count) is not int or warm_count != 36
            or type(pairs) is not int or pairs != 30):
        raise GcComparisonError("P033 requires 45 legacy rows, 36 warm rows, and exactly 30 paired samples")
    groups_per_run = old_count + warm_count * len(STAGES)
    events_per_run = groups_per_run * pairs
    return {"legacy_workload_count": old_count, "warm_workload_count": warm_count,
            "groups_per_run": groups_per_run, "groups_total": groups_per_run * 2,
            "pairs_per_group": pairs, "pair_events_per_run": events_per_run,
            "pair_event_count": events_per_run * 2,
            "side_observation_count": events_per_run * 2 * 2}


def layout_pair_count(old_count: int = 45, warm_count: int = 36,
                      layout_count: int = 36, pairs: int = 30) -> dict[str, int]:
    if (type(old_count) is not int or old_count != 45
            or type(warm_count) is not int or warm_count != 36
            or type(layout_count) is not int or layout_count != 36
            or type(pairs) is not int or pairs != 30):
        raise GcComparisonError("P034 requires 45 legacy rows, 36 warm rows, 36 layout rows, and 30 pairs")
    groups_per_run = old_count + (warm_count + layout_count) * len(STAGES)
    events_per_run = groups_per_run * pairs
    return {"legacy_workload_count": old_count, "warm_workload_count": warm_count,
            "layout_workload_count": layout_count, "groups_per_run": groups_per_run,
            "groups_total": groups_per_run * 2, "pairs_per_group": pairs,
            "pair_events_per_run": events_per_run, "pair_event_count": events_per_run * 2,
            "side_observation_count": events_per_run * 4}


def _load_file(path: Path, name: str):
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise GcComparisonError(f"cannot load staged adapter {path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


def _model_for_entry(entry: dict[str, Any], old_model, warm_model, layout_model):
    module_name = entry.get("oracle", {}).get("module")
    if module_name == OLD_MODEL_NAME:
        return old_model
    if module_name == WARM_MODEL_NAME:
        return warm_model
    if module_name == LAYOUT_MODEL_NAME and layout_model is not None:
        return layout_model
    raise GcComparisonError(f"{entry.get('id')}: unknown or unselected GC model module {module_name!r}")


def _is_staged_entry(entry: dict[str, Any]) -> bool:
    return entry.get("oracle", {}).get("module") in {WARM_MODEL_NAME, LAYOUT_MODEL_NAME}


def run(args, growth_explanation=None) -> dict[str, Any]:
    """Dedicated comparison entry point; intended to be dispatched by compare.run."""
    # All imports are delayed so validators remain usable in small synthetic tests.
    import baseline
    import checkpoint
    import compare
    import gc_scaling
    import gc_workloads
    import gc_warm_workloads
    import workloads
    gc_layout_workloads = None
    if args.selection == "gc-layouts":
        import gc_layout_workloads as gc_layout_workloads_module
        gc_layout_workloads = gc_layout_workloads_module

    if growth_explanation is None:
        growth_explanation = compare._read_build_growth_explanation(
            getattr(args, "build_growth_explanation_file", None))
    if args.selection not in {"gc-exceptions", "gc-layouts"}:
        raise GcComparisonError("GC comparison only accepts gc-exceptions or gc-layouts")
    if type(args.pairs) is not int or args.pairs != 30:
        raise GcComparisonError("P033/P034 are predeclared for exactly 30 paired samples")
    if type(getattr(args, "seed", None)) is not int or args.seed < 1:
        raise GcComparisonError("P033 requires a positive integer seed")
    checkpoint.Journal.preflight(args.output)
    manifest = baseline.verify_archive(args.baseline)
    baseline_info = compare._baseline_identity(args.baseline, manifest, HERE / "src/protocol.rs", baseline)
    if baseline_info["kind"] != "current-source-archive":
        raise GcComparisonError("P033 GC comparison requires a current-source archive; legacy adapters are unsupported")
    if baseline_info["mode"] != MODE or args.candidate_mode != MODE:
        raise GcComparisonError("P033 comparison is pinned to the decoded execution policy")
    frozen = baseline_info["frozen"]
    compare._verify_scalar_harness_sources(frozen, manifest)

    frozen_sources = manifest.get("files", {})
    required_model_paths = ["source/tools/wtime-bench/workloads.json",
                            "source/tools/wtime-bench/gc_workloads.py",
                            "source/tools/wtime-bench/gc_warm_workloads.py",
                            "source/tools/wtime-bench/gc_scaling.py"]
    if args.selection == "gc-layouts":
        required_model_paths.append("source/tools/wtime-bench/gc_layout_workloads.py")
    required_contract_paths = []
    if args.selection == "gc-layouts":
        required_contract_paths.extend([
            "source/docs/benchmarks/performance/P034-contract.json",
            "source/docs/benchmarks/performance/P034-measurement-contract.json",
        ])
    for rel in required_model_paths + required_contract_paths:
        record = frozen_sources.get(rel)
        path = frozen / rel
        logical_path = rel.removeprefix("source/")
        if (not isinstance(record, dict) or record.get("role") != "source"
                or record.get("source_path") != logical_path or not path.is_file()
                or sha(path) != record.get("sha256")):
            raise GcComparisonError(f"current-source archive is missing or mismatches P033 input {rel}")
        live_rel = ROOT / rel.removeprefix("source/")
        if not live_rel.is_file() or sha(live_rel) != sha(path):
            raise GcComparisonError(f"GC comparison source differs from its frozen archive: {rel}")

    registry = workloads.load_registry()
    old_entries, warm_entries = selected_model_rows(registry, gc_workloads, gc_warm_workloads)
    layout_entries = []
    if args.selection == "gc-layouts":
        layout_entries = selected_layout_model_rows(registry, old_entries, warm_entries,
                                                    gc_layout_workloads)
    selected = old_entries + warm_entries + layout_entries
    default_contract = CONTRACT_PATH if args.selection == "gc-exceptions" else LAYOUT_CONTRACT_PATH
    contract_value = getattr(args, "contract", None)
    contract_path = Path(contract_value) if contract_value is not None else default_contract
    contract = json.loads(contract_path.read_text())
    if args.selection == "gc-layouts":
        archived_contract_path = "source/docs/benchmarks/performance/P034-contract.json"
        archived_contract_sha = frozen_sources[archived_contract_path]["sha256"]
        if sha(contract_path) != archived_contract_sha:
            raise GcComparisonError("P034 gate contract differs from the immutable source archive")
    measurement_contract_path = None
    measurement_contract = None
    if args.selection == "gc-exceptions":
        contract_info = validate_contract(contract, [entry["id"] for entry in selected],
                                          [entry["id"] for entry in warm_entries])
    else:
        measurement_contract_path = LAYOUT_MEASUREMENT_CONTRACT_PATH
        if sha(measurement_contract_path) != frozen_sources[
                "source/docs/benchmarks/performance/P034-measurement-contract.json"]["sha256"]:
            raise GcComparisonError("P034 measurement contract differs from the immutable source archive")
        measurement_contract = json.loads(measurement_contract_path.read_text())
        contract_info = validate_layout_contract(contract, old_entries, warm_entries, layout_entries,
                                                 [entry["id"] for entry in selected],
                                                 gc_layout_workloads, registry, measurement_contract)
    if baseline_info.get("kind") == "legacy-api-adapter":
        raise GcComparisonError("legacy API adapters do not implement the current GC staged protocol")

    candidate = Path(args.candidate).resolve()
    if not candidate.is_file() or not os.access(candidate, os.X_OK):
        raise GcComparisonError("candidate benchmark executable is missing or not executable")
    encoder = ROOT / "target/debug/wtime-conformance"
    if not encoder.is_file() or not os.access(encoder, os.X_OK):
        raise GcComparisonError("existing conformance encoder is missing or not executable")
    allowed_cpus = sorted(os.sched_getaffinity(0))
    cpu = args.cpu if args.cpu is not None else allowed_cpus[0]
    if type(cpu) is not int or cpu not in allowed_cpus:
        raise GcComparisonError("selected CPU is outside the available affinity")
    initial_binary = sha(candidate)
    baseline_binary = sha(baseline_info["executable"])
    encoder_hash = sha(encoder)
    current_sources = compare.sources()
    fixture_sources: dict[str, dict[str, Any]] = {}

    # Full current-source build identity plus explicit GC fixtures: the shared
    # helper's generic snapshot currently omits the GC WAT trees.
    candidate_sources = dict(current_sources)
    output = Path(args.output).resolve()
    fixture_by_id: dict[str, dict[str, Any]] = {}
    encoder_records = []
    unique_sources: dict[tuple[str, str], tuple[Path, str, str]] = {}
    for entry in selected:
        module = _model_for_entry(entry, gc_workloads, gc_warm_workloads, gc_layout_workloads)
        root = Path(module.__file__).parent
        source = entry["source"]
        logical_wat = _logical_fixture_path(source.get("path"), entry["id"])
        wat = root / source["path"]
        if not wat.is_file() or sha(wat) != source["sha256"] or not source.get("wasm_sha256"):
            raise GcComparisonError(f"{entry['id']}: WAT/Wasm source identity differs from the registered model")
        prior = fixture_sources.get(logical_wat)
        if prior and prior["sha256"] != source["sha256"]:
            raise GcComparisonError(f"{entry['id']}: conflicting source identity for {logical_wat}")
        fixture_sources[logical_wat] = {"path": wat, "sha256": source["sha256"]}
        if module is not gc_workloads:
            source_wasm = wat.with_suffix(".wasm")
            logical_wasm = str(Path(logical_wat).with_suffix(".wasm"))
            if not source_wasm.is_file() or sha(source_wasm) != source["wasm_sha256"]:
                raise GcComparisonError(f"{entry['id']}: frozen Wasm fixture differs from model identity")
            fixture_sources[logical_wasm] = {"path": source_wasm, "sha256": source["wasm_sha256"]}
        key = (str(wat.resolve()), source["wasm_sha256"])
        unique_sources[key] = (wat, source["sha256"], source["wasm_sha256"])
        setup_paths = []
        for setup in entry.get("setup_modules", []):
            logical_setup = _logical_fixture_path(setup.get("path"), entry["id"])
            module_wat = root / setup["path"]
            if not module_wat.is_file() or sha(module_wat) != setup["sha256"] or not setup.get("wasm_sha256"):
                raise GcComparisonError(f"{entry['id']}: setup-module WAT/Wasm identity mismatch")
            prior = fixture_sources.get(logical_setup)
            if prior and prior["sha256"] != setup["sha256"]:
                raise GcComparisonError(f"{entry['id']}: conflicting setup source identity for {logical_setup}")
            fixture_sources[logical_setup] = {"path": module_wat, "sha256": setup["sha256"]}
            unique_sources[(str(module_wat.resolve()), setup["wasm_sha256"])] = (
                module_wat, setup["sha256"], setup["wasm_sha256"])
            setup_paths.append((module_wat, setup, logical_setup))
        fixture_by_id[entry["id"]] = {"entry": entry, "wat": wat,
                                        "logical_wat": logical_wat, "setup_wats": setup_paths}

    fixture_provenance = []
    for workload, fixture in fixture_by_id.items():
        entry = fixture["entry"]
        source = entry["source"]
        setup_provenance = []
        for setup_wat, setup, logical_setup in fixture["setup_wats"]:
            setup_provenance.append({"source_path": logical_setup, "wat_sha256": setup["sha256"],
                                     "wasm_sha256": setup["wasm_sha256"]})
        fixture_provenance.append({"workload": workload, "source_path": fixture["logical_wat"],
            "wat_sha256": source["sha256"], "wasm_sha256": source["wasm_sha256"],
            "setup_modules": setup_provenance, "profile": PROFILE})

    # Require every GC source fixture to be bound by the immutable archive.
    for logical_path, source_record in fixture_sources.items():
        archive_key = f"source/{logical_path}"
        archive_record = manifest.get("files", {}).get(archive_key)
        archived_path = Path(frozen) / archive_key
        if (not isinstance(archive_record, dict) or archive_record.get("role") != "source"
                or archive_record.get("source_path") != logical_path
                or archive_record.get("sha256") != source_record["sha256"]
                or not archived_path.is_file() or sha(archived_path) != source_record["sha256"]):
            raise GcComparisonError(f"current-source archive does not bind GC fixture {logical_path}")
    for logical_path, source_record in fixture_sources.items():
        if current_sources.get(logical_path) != source_record["sha256"]:
            raise GcComparisonError(f"candidate source map does not bind fixture {logical_path}")
    for logical_path, digest in current_sources.items():
        if not logical_path.startswith("tools/wtime-bench/"):
            continue
        archive_key = f"source/{logical_path}"
        record = manifest.get("files", {}).get(archive_key)
        archived = Path(frozen) / archive_key
        if (not isinstance(record, dict) or record.get("role") != "source"
                or record.get("source_path") != logical_path or record.get("sha256") != digest
                or not archived.is_file() or sha(archived) != digest):
            raise GcComparisonError(f"current-source archive does not bind harness source {logical_path}")
    candidate_sources = current_sources

    baseline_exe = baseline_info["executable"]
    capabilities = {"baseline": compare.process(baseline_exe, ["--protocol"], cpu),
                    "candidate": compare.process(candidate, ["--protocol"], cpu)}
    for side, capability in capabilities.items():
        selected_mode = baseline_info["mode"] if side == "baseline" else args.candidate_mode
        modes = capability.get("modes") if isinstance(capability, dict) else None
        features = capability.get("features") if isinstance(capability, dict) else None
        if (not isinstance(capability, dict)
                or type(capability.get("schema_version")) is not int or capability.get("schema_version") != 2
                or capability.get("instrumented") is not False
                or not isinstance(modes, list) or any(not isinstance(mode, str) for mode in modes)
                or not isinstance(features, list) or any(not isinstance(feature, str) for feature in features)
                or selected_mode not in modes or PROFILE not in features):
            raise GcComparisonError(f"{side}: protocol does not prove uninstrumented decoded all-phase3-core support")

    cold_adapter = _load_file(COLD_ADAPTER_PATH, "p033_gc_cold_adapter")
    cold_adapter.process = compare.process
    cold_adapter.ComparisonError = compare.ComparisonError

    try:
        output.mkdir(parents=True, exist_ok=False)
    except FileExistsError as exc:
        raise GcComparisonError("output path appeared after preflight; refusing to reuse it") from exc
    encoded_dir = output / "fixtures" / "gc"
    encoded_dir.mkdir(parents=True, exist_ok=False)
    for index, (wat, expected_wat, expected_wasm) in enumerate(unique_sources.values()):
        if sha(wat) != expected_wat:
            raise GcComparisonError(f"fixture WAT changed during fixture preflight: {wat}")
        wasm = encoded_dir / f"{index:03d}-{wat.stem}.wasm"
        proc = subprocess.run([str(encoder), "encode", str(wat), "-o", str(wasm)],
                              cwd=ROOT, text=True, capture_output=True,
                              preexec_fn=lambda: os.sched_setaffinity(0, {cpu}), timeout=60)
        if proc.returncode != 0:
            raise GcComparisonError(f"fixture encoding failed for {wat}: {proc.stderr[-2000:]}")
        actual_wasm = sha(wasm)
        if actual_wasm != expected_wasm:
            raise GcComparisonError(f"fixture encoded Wasm hash mismatch for {wat}: {actual_wasm}")
        logical_source = next((logical for logical, record in fixture_sources.items()
                               if record["path"] == wat), None)
        if logical_source is None:
            raise GcComparisonError(f"encoded WAT has no repository-relative provenance: {wat.name}")
        encoder_records.append({"source_path": logical_source, "wat_sha256": expected_wat,
                                "wasm_artifact": wasm.name, "wasm_sha256": actual_wasm})
        for fixture in fixture_by_id.values():
            if fixture["wat"] == wat:
                fixture["path"] = wasm
            for setup_wat, setup, _logical_setup in fixture["setup_wats"]:
                if setup_wat == wat:
                    setup["encoded_path"] = wasm
    for case, fixture in fixture_by_id.items():
        fixture["setup_modules"] = [setup["encoded_path"] for _, setup, _ in fixture.pop("setup_wats")]
        if "path" not in fixture:
            raise GcComparisonError(f"{case}: encoded fixture path was not resolved")

    # Rehash every relevant source/encoder before the first observation.
    if sha(encoder) != encoder_hash or compare.sources() != current_sources:
        raise GcComparisonError("source tree or conformance encoder changed during fixture encoding")
    for logical_path, source_record in fixture_sources.items():
        if sha(source_record["path"]) != source_record["sha256"]:
            raise GcComparisonError(f"GC fixture source changed during encoding: {logical_path}")

    settings = {"instrumented": False, "baseline_mode": baseline_info["mode"],
                "candidate_mode": args.candidate_mode, "feature_policy": PROFILE,
                "fuel_policy": False, "cancellation_policy": 1, "profiles": [PROFILE],
                "warm_measurement_policy": MEASUREMENT_POLICY,
                "warm_target_ns": TARGET_NS, "warm_max_batch": MAX_BATCH,
                "warm_max_product": MAX_PRODUCT, "warm_max_attempts": MAX_ATTEMPTS,
                "warm_calibration": "shared_exact_batch_both_sides_confirmed",
                "wasmtime_gc_status": "unsupported", "pair_count": 30,
                "primary_workload_count": contract_info["primary_workload_count"],
                "protected_workload_count": contract_info["protected_workload_count"]}
    identity = {"selection": args.selection, "settings": settings,
                "baseline_archive_kind": baseline_info["kind"],
                "baseline_manifest_sha256": sha(Path(args.baseline) / "manifest.json"),
                "frozen_manifest_sha256": sha(Path(frozen) / "manifest.json"),
                "baseline_binary_sha256": baseline_binary,
                "candidate_binary_sha256": initial_binary,
                "candidate_sources": candidate_sources,
                "protocol_sha256": sha(HERE / "src/protocol.rs"),
                "contract_sha256": sha(contract_path), "contract": contract_info,
                "fixture_encoder_sha256": encoder_hash, "fixture_encodings": encoder_records,
                "fixtures": fixture_provenance,
                "selected_ids": [entry["id"] for entry in selected],
                "capabilities": capabilities}
    if measurement_contract_path is not None:
        identity["measurement_contract_sha256"] = sha(measurement_contract_path)
    if growth_explanation is not None:
        identity.update(growth_explanation["metadata"])
    seeds = {run_id: args.seed + run_id * 1_000_003 for run_id in (0, 1)}
    journal = checkpoint.Journal(output, identity, seeds, prepared=True)
    report = {"schema_version": 2, "selection": args.selection, "passed": False,
              "source_stable": False, "metadata": identity, "runs": [],
              "warm_calibrations": [], "correctness_probes": [], "error": None}
    report_path = output / "report.json"
    try:
        compare._verify_build_growth_explanation(growth_explanation)
        # Preflight every workload on both executables before paired timing.
        for entry in selected:
            n = _input(entry)
            fixture = fixture_by_id[entry["id"]]
            for side, executable in (("baseline", baseline_exe), ("candidate", candidate)):
                mode = baseline_info["mode"] if side == "baseline" else args.candidate_mode
                probe_stage = "load_and_call" if not _is_staged_entry(entry) else "first_call"
                if not _is_staged_entry(entry):
                    cold_sample = cold_adapter.cold_observation(executable, fixture, entry, n,
                                                               probe_stage, 1, mode, cpu)
                    observation = _normalize_cold(entry, cold_sample, gc_workloads)
                else:
                    entry_model = _model_for_entry(entry, gc_workloads, gc_warm_workloads,
                                                   gc_layout_workloads)
                    expected = entry_model.oracle(entry["id"], n)
                    raw = compare.process(executable,
                        ["--gc-scalar-driver", fixture["path"], entry["id"], entry["family"],
                         f"i32:{n}", mode, PROFILE, probe_stage, 1], cpu)
                    observation = validate_warm_observation(
                        entry, raw, stage=probe_stage, batch=1, mode=mode, expected=expected)
                report["correctness_probes"].append({"case": entry["id"], "side": side,
                    "timing_excluded": True, "status": "passed", "stage": probe_stage,
                    "observation": observation})

        for run_id in (0, 1):
            run_seed = seeds[run_id]
            run = {"run_id": run_id, "seed": run_seed, "pairs": []}
            report["runs"].append(run)
            for entry in selected:
                n = _input(entry)
                is_warm = _is_staged_entry(entry)
                fixture = fixture_by_id[entry["id"]]
                entry_model = _model_for_entry(entry, gc_workloads, gc_warm_workloads,
                                               gc_layout_workloads)
                expected = entry_model.oracle(entry["id"], n)
                for stage in entry["stages"]:
                    batch = 1
                    calibration = None
                    if is_warm and stage == "warm_call":
                        def observe(side, requested):
                            executable = baseline_exe if side == "baseline" else candidate
                            mode = baseline_info["mode"] if side == "baseline" else args.candidate_mode
                            return compare.process(executable,
                                ["--gc-scalar-driver", fixture["path"], entry["id"], entry["family"],
                                 f"i32:{n}", mode, PROFILE, "warm_call", str(requested)], cpu)
                        calibration = calibrate_shared(entry, MODE, expected, observe)
                        batch = calibration["paired_batch"]
                        report["warm_calibrations"].append({"run_id": run_id,
                            "workload": entry["id"], "stage": stage, **calibration})
                    schedule_seed = run_seed + int(fixture["entry"]["source"]["wasm_sha256"][:8], 16) + STAGES.index(stage)
                    rng = random.Random(schedule_seed)
                    for sample in range(args.pairs):
                        order = (["baseline", "candidate"] if rng.getrandbits(1)
                                 else ["candidate", "baseline"])
                        pair = {"pair_id": f"{run_id}:{entry['id']}:{stage}:{sample}",
                                "workload": entry["id"], "stage": stage, "input": n,
                                "expected": expected,
                                "wat_sha256": entry["source"]["sha256"],
                                "wasm_sha256": entry["source"]["wasm_sha256"],
                                "order": order, "batch": batch,
                                "wasmtime": {"status": "unsupported"}}
                        if not is_warm:
                            for side in order:
                                executable = baseline_exe if side == "baseline" else candidate
                                mode = baseline_info["mode"] if side == "baseline" else args.candidate_mode
                                sample_row = cold_adapter.cold_observation(executable, fixture, entry, n,
                                    stage, 1, mode, cpu)
                                pair[side] = _normalize_cold(entry, sample_row, gc_workloads)
                        else:
                            for side in order:
                                executable = baseline_exe if side == "baseline" else candidate
                                mode = baseline_info["mode"] if side == "baseline" else args.candidate_mode
                                raw = compare.process(executable,
                                    ["--gc-scalar-driver", fixture["path"], entry["id"], entry["family"],
                                     f"i32:{n}", mode, PROFILE, stage, str(batch)], cpu)
                                pair[side] = validate_warm_observation(entry, raw, stage=stage,
                                    batch=batch, mode=mode, expected=expected)
                        run["pairs"].append(pair)
                        journal.append(run_id, run_seed, pair)

        compare._verify_build_growth_explanation(growth_explanation)
        if (compare.sources() != current_sources or sha(candidate) != initial_binary
                or sha(baseline_exe) != baseline_binary or sha(encoder) != encoder_hash):
            raise GcComparisonError("baseline/candidate source, executable, or encoder changed during comparison")
        if sha(contract_path) != identity["contract_sha256"]:
            raise GcComparisonError("P033 contract changed during comparison")
        if (measurement_contract_path is not None
                and sha(measurement_contract_path) != identity["measurement_contract_sha256"]):
            raise GcComparisonError("P034 measurement contract changed during comparison")
        for logical_path, source_record in fixture_sources.items():
            if sha(source_record["path"]) != source_record["sha256"]:
                raise GcComparisonError(f"fixture source changed during comparison: {logical_path}")
        expected_counts = (pair_count(pairs=args.pairs) if args.selection == "gc-exceptions"
                           else layout_pair_count(pairs=args.pairs))
        observed_pairs = sum(len(run["pairs"]) for run in report["runs"])
        expected_warm_rows = len(warm_entries) + len(layout_entries)
        expected_selected = len(old_entries) + expected_warm_rows
        if (observed_pairs != expected_counts["pair_event_count"]
                or len(report["warm_calibrations"]) != expected_warm_rows * 2
                or len(report["correctness_probes"]) != expected_selected * 2):
            raise GcComparisonError("GC observed pairs/calibrations/preflight probes differ from the selected frozen matrix")
        report["counts"] = expected_counts
        report["passed"] = True
        report["source_stable"] = True
        report.pop("error", None)
    except Exception as exc:
        report["error"] = f"{type(exc).__name__}: {exc}"
        report["counts"] = {"pair_events": sum(len(run["pairs"]) for run in report["runs"]),
                             "warm_calibration_records": len(report["warm_calibrations"])}
    finally:
        journal.close()
        journal.write_report_once(report)
    if not report["passed"]:
        raise GcComparisonError(report["error"])
    return report
