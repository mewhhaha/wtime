"""Strict, subprocess-free validator for the P071 repeated-collection driver."""
from __future__ import annotations

import copy
import json
import math
from typing import Any

import gc_collect_workloads

MODE = "decoded"
PROFILE = "all-phase3-core"
DRIVER = "gc_collect_stages"
MEASUREMENT_POLICY = (
    "sum_guest_repeated_collect_intervals_excluding_setup_oracles_and_final_cleanup"
)


class GcCollectComparisonError(ValueError):
    """A collection observation differs from its declared workload contract."""


def _fail(entry: Any, message: str) -> GcCollectComparisonError:
    workload = entry.get("id", "unknown") if isinstance(entry, dict) else "unknown"
    return GcCollectComparisonError(f"{workload}: {message}")


def _canonical(value: Any, entry: Any, label: str) -> str:
    try:
        return json.dumps(value, sort_keys=True, separators=(",", ":"),
                          ensure_ascii=False, allow_nan=False)
    except (TypeError, ValueError) as exc:
        raise _fail(entry, f"{label} is not finite JSON data") from exc


def _int(value: Any, name: str, entry: Any) -> int:
    if type(value) is not int:
        raise _fail(entry, f"{name} must be a JSON integer")
    return value


def _positive(value: Any, name: str, entry: Any) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise _fail(entry, f"{name} must be numeric")
    if not math.isfinite(value) or value <= 0:
        raise _fail(entry, f"{name} must be finite and positive")
    return float(value)


def _close(actual: float, expected: float, name: str, entry: Any) -> None:
    if not math.isclose(actual, expected, rel_tol=1e-12, abs_tol=1e-6):
        raise _fail(entry, f"{name} differs from its declared timing interval")


def _model_row(entry: dict[str, Any]) -> tuple[dict[str, Any], int, int, int, int]:
    """Return canonical row, repetitions, nodes, live count and result."""
    if not isinstance(entry, dict):
        raise _fail(entry, "workload entry must be an object")
    workload = entry.get("id", "unknown")
    try:
        # Compare canonical JSON so bool/float substitutions cannot pass Python's
        # permissive `True == 1` or `1.0 == 1` object equality.
        model_row = next(row for row in gc_collect_workloads.entries()
                         if row.get("id") == workload)
        if _canonical(entry, entry, "workload entry") != _canonical(
                model_row, entry, "independent model row"):
            raise ValueError("workload entry differs from the independent collection model")
        repetitions = entry["params"][0]["baseline"]
        if type(repetitions) is not int:
            raise ValueError("repetitions are not a strict integer")
        record = gc_collect_workloads.FAMILIES[entry["family"]]
        nodes = record["count"]
        live = nodes + 1 if record["shape"] == "wide" else nodes
        result = gc_collect_workloads.oracle(workload, repetitions)
        if type(result) is not int or not -(1 << 31) <= result < (1 << 31):
            raise ValueError("independent result is not a signed i32")
        return model_row, repetitions, nodes, live, result
    except Exception as exc:
        if isinstance(exc, GcCollectComparisonError):
            raise
        raise _fail(entry, f"unknown or inconsistent independent collection row: {exc}") from exc


def validate_observation(entry: dict[str, Any], wire: dict[str, Any],
                         expected_batch: int | None = None, *,
                         expected_mode: str = MODE,
                         expected_stage: str | None = None) -> dict[str, Any]:
    """Validate one exact ``gc_collect_stages`` wire object and normalize it."""
    if not isinstance(entry, dict) or not isinstance(wire, dict):
        raise GcCollectComparisonError("entry and collection driver observation must be objects")
    _, repetitions, nodes, live, result = _model_row(entry)
    workload = entry["id"]
    family = entry["family"]
    shape = gc_collect_workloads.FAMILIES[family]["shape"]
    cap = gc_collect_workloads.max_batch(nodes, repetitions)

    stage = expected_stage if expected_stage is not None else wire.get("stage")
    if expected_mode != MODE:
        raise _fail(entry, "repeated collection driver mode must be decoded")
    if stage not in gc_collect_workloads.STAGES:
        raise _fail(entry, "stage is not declared by the independent collection model")
    batch = _int(expected_batch if expected_batch is not None else wire.get("batch"),
                 "requested batch", entry)
    if not 1 <= batch <= cap:
        raise _fail(entry, "batch exceeds the model-derived repeated-collection cap")
    if stage != "warm_call" and batch != 1:
        raise _fail(entry, "non-warm stage batch must be one")

    expected_wire = {
        "schema_version": 2,
        "driver": DRIVER,
        "case": workload,
        "family": family,
        "shape": shape,
        "nodes": nodes,
        "live_objects": live,
        "input": repetitions,
        "mode": expected_mode,
        "profile": PROFILE,
        "stage": stage,
        "batch": batch,
        "max_batch": cap,
        "instrumented": False,
        "measurement_policy": MEASUREMENT_POLICY,
        "report_complete": True,
        "result": f"i32:{result}",
    }
    fields = set(expected_wire) | {
        "sample_ns", "prepare_ns", "instantiate_ns", "first_call_ns",
        "measured_guest_ns", "cleanup_ns", "calls_checked", "callbacks_checked",
        "callback_live", "callback_reclaimed", "callback_payload_min",
        "callback_payload_max", "final_live", "final_reclaimed",
        "final_payload_bytes", "final_continuation_bytes",
    }
    if set(wire) != fields:
        raise _fail(entry, "driver wire schema has missing or unknown fields")
    for key, wanted in expected_wire.items():
        actual = wire.get(key)
        if type(wanted) is int:
            actual = _int(actual, key, entry)
        elif type(wanted) is bool:
            if type(actual) is not bool:
                raise _fail(entry, f"{key} must be a JSON boolean")
        elif type(actual) is not type(wanted):
            raise _fail(entry, f"{key} has the wrong JSON type")
        if actual != wanted:
            raise _fail(entry, f"{key} differs from the model or requested observation")

    calls = batch + 1 if stage == "warm_call" else 1
    expected_counts = {
        "calls_checked": calls,
        "callbacks_checked": calls * repetitions,
        "callback_live": live,
        "callback_reclaimed": 0,
        "final_live": 0,
        "final_reclaimed": live,
        "final_payload_bytes": 0,
        "final_continuation_bytes": 0,
    }
    for key, wanted in expected_counts.items():
        if _int(wire.get(key), key, entry) != wanted:
            raise _fail(entry, f"{key} differs from the complete collection proof")
    payload_min = _int(wire.get("callback_payload_min"), "callback_payload_min", entry)
    payload_max = _int(wire.get("callback_payload_max"), "callback_payload_max", entry)
    if payload_min <= 0 or payload_min != payload_max:
        raise _fail(entry, "callback payload must remain positive and constant across rooted collections")

    times = {}
    for field in ("sample_ns", "prepare_ns", "instantiate_ns", "first_call_ns",
                  "measured_guest_ns", "cleanup_ns"):
        times[field] = _positive(wire.get(field), field, entry)
    measured_guest = times["measured_guest_ns"]
    if stage == "warm_call":
        _close(measured_guest, times["sample_ns"] * batch, "measured_guest_ns", entry)
        expected_interval = measured_guest / batch
    else:
        _close(measured_guest, times["first_call_ns"], "measured_guest_ns", entry)
        expected_interval = {
            "prepare": times["prepare_ns"],
            "instantiate": times["instantiate_ns"],
            "first_call": times["first_call_ns"],
            "load_and_call": (times["prepare_ns"] + times["instantiate_ns"]
                              + times["first_call_ns"]),
        }[stage]
    _close(times["sample_ns"], expected_interval, "sample_ns", entry)

    retained = copy.deepcopy(wire)
    return {
        "case": workload,
        "family": family,
        "stage": stage,
        "batch": batch,
        "ns_per_operation": times["sample_ns"],
        "result": result,
        "input": repetitions,
        "mode": expected_mode,
        "profile": PROFILE,
        "instrumented": False,
        "measurement_policy": MEASUREMENT_POLICY,
        "driver_result": retained,
    }


def revalidate_observation(entry: dict[str, Any], observation: dict[str, Any], *,
                           expected_batch: int | None = None,
                           expected_mode: str = MODE,
                           expected_stage: str | None = None) -> dict[str, Any]:
    """Recheck retained raw wire and require exact normalized-view equality."""
    if not isinstance(observation, dict) or not isinstance(observation.get("driver_result"), dict):
        raise _fail(entry, "retained observation is missing its complete driver_result")
    wire = observation["driver_result"]
    batch = expected_batch if expected_batch is not None else observation.get("batch")
    stage = expected_stage if expected_stage is not None else observation.get("stage")
    mode = expected_mode if expected_mode != MODE else observation.get("mode", MODE)
    normalized = validate_observation(entry, wire, batch,
                                      expected_mode=mode, expected_stage=stage)
    if _canonical(normalized, entry, "revalidated observation") != _canonical(
            observation, entry, "retained normalized observation"):
        raise _fail(entry, "normalized observation differs from its complete driver proof")
    return normalized
