#!/usr/bin/env python3
"""Source-bound P012 continuation capture/resume/accounting runner draft.

Runs seven semantic workloads five times each through the staged public-API
driver. Provider fixtures are setup-only. Timings are accepted only from the
driver's separately named operations; no external Wasmtime ratio is produced.
"""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import math
import os
from pathlib import Path
import platform
import random
import subprocess
import sys
import tempfile
from typing import Any


SAMPLES = 5
MODE = "decoded"
PROFILE = "all-phase3-core"
PROVIDER_ID = "continuation_cross_module_provider"
PROVIDER_CASE = "continuation_cross_module_consumer"
MEASURED_IDS = {
    "continuation_depth_0", "continuation_depth_4", "continuation_depth_32",
    "continuation_captured_reference", "continuation_bound_nested",
    "continuation_one_shot", PROVIDER_CASE,
}


class MeasurementError(ValueError):
    pass


def _sha(path: Path) -> str:
    try:
        return hashlib.sha256(path.read_bytes()).hexdigest()
    except OSError as exc:
        raise MeasurementError(f"cannot hash {path}: {exc}") from exc


def _load_python(path: Path, module_name: str):
    spec = importlib.util.spec_from_file_location(module_name, path)
    if spec is None or spec.loader is None:
        raise MeasurementError(f"cannot load Python source {path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def load_model(path: Path):
    module = _load_python(path, "p012_continuation_model")
    for name in ("workload_entries", "verify_fixture_sources", "depth_oracle", "accounting_metadata"):
        if not callable(getattr(module, name, None)):
            raise MeasurementError(f"continuation model must expose {name}()")
    errors = module.verify_fixture_sources()
    if errors:
        raise MeasurementError(f"continuation WAT/model validation failed: {errors[:4]}")
    return module


def _snapshot(paths: list[Path]) -> dict[str, str]:
    resolved = sorted({path.resolve() for path in paths}, key=str)
    return {str(path): _sha(path) for path in resolved}


def _assert_stable(before: dict[str, str], after: dict[str, str]) -> None:
    if before == after:
        return
    changed = sorted(path for path in before.keys() & after.keys() if before[path] != after[path])
    missing = sorted(before.keys() - after.keys())
    added = sorted(after.keys() - before.keys())
    raise MeasurementError(
        f"P012 source/binary identity changed during run "
        f"(changed={changed[:4]}, missing={missing[:4]}, added={added[:4]})"
    )


def _rust_inputs(root: Path) -> list[Path]:
    root = root.resolve()
    required = [root / "Cargo.toml", root / "Cargo.lock",
                root / "tools/wtime-bench/Cargo.toml",
                root / "tools/wtime-bench/Cargo.lock"]
    missing = [str(path) for path in required if not path.is_file()]
    if missing:
        raise MeasurementError(f"missing Rust build identity files: {missing}")
    sources = []
    for subtree in (root / "crates", root / "tools/wtime-bench"):
        if not subtree.is_dir():
            raise MeasurementError(f"missing Rust source tree {subtree}")
        sources.extend(path for path in subtree.rglob("*.rs")
                       if "target" not in path.parts and ".git" not in path.parts)
    if not sources:
        raise MeasurementError("Rust runtime/benchmark source snapshot is empty")
    optional = [root / "rust-toolchain", root / "rust-toolchain.toml",
                root / ".cargo/config", root / ".cargo/config.toml"]
    return sorted(set(required + sources + [path for path in optional if path.is_file()]))


def _registry(path: Path, selection: str, model_entries: list[dict[str, Any]]) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    try:
        registry = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise MeasurementError(f"cannot read workload registry: {exc}") from exc
    if registry.get("schema_version") != 2:
        raise MeasurementError("P012 custom registry model requires schema_version 2")
    selected = registry.get("selections", {}).get(selection)
    if not isinstance(selected, dict) or selected.get("status") != "ready":
        raise MeasurementError(f"registry selection {selection!r} is not ready")
    ids = selected.get("workloads")
    if (not isinstance(ids, list) or any(not isinstance(value, str) for value in ids) or
            len(ids) != len(set(ids))):
        raise MeasurementError(f"registry selection {selection!r} has invalid workload ids")
    model_by_id = {entry.get("id"): entry for entry in model_entries}
    if len(model_by_id) != len(model_entries) or set(model_by_id) != MEASURED_IDS | {PROVIDER_ID}:
        raise MeasurementError("continuation model must define seven cases and one provider setup fixture")
    if set(ids) != MEASURED_IDS:
        raise MeasurementError(
            f"registry must select exactly seven continuation cases; "
            f"missing={sorted(MEASURED_IDS-set(ids))}, extra={sorted(set(ids)-MEASURED_IDS)}"
        )
    registry_rows = registry.get("workloads")
    if not isinstance(registry_rows, list):
        raise MeasurementError("registry workloads must be a list")
    rows_by_id = {row.get("id"): row for row in registry_rows if isinstance(row, dict)}
    if len(rows_by_id) != len([row for row in registry_rows if isinstance(row, dict)]):
        raise MeasurementError("registry has duplicate workload ids")
    selected_entries = []
    for workload in ids:
        if workload not in rows_by_id:
            raise MeasurementError(f"registry has no full workload row for {workload}")
        row = rows_by_id[workload]
        model = model_by_id[workload]
        # This custom v2 model carries continuation semantics that the generic
        # registry does not encode. Require exact agreement on its shared identity
        # projection while retaining the custom oracle below.
        registry_source, model_source = row.get("source"), model.get("source")
        if not isinstance(registry_source, dict) or any(
                registry_source.get(key) != model_source.get(key)
                for key in ("kind", "path", "sha256", "wasm_sha256")):
            raise MeasurementError(f"{workload}: registry source identity differs from custom continuation model")
        for key in ("required_features", "stages"):
            if row.get(key) != model.get(key):
                raise MeasurementError(f"{workload}: registry {key} differs from custom continuation model")
        selected_entries.append(model)
    # Provider is deliberately setup-only: it is excluded from selected ids and
    # must be named as the cross-module consumer's provider source.
    provider = model_by_id[PROVIDER_ID]
    if provider["id"] in ids or PROVIDER_CASE not in model_by_id:
        raise MeasurementError("provider fixture must remain setup-only for the cross-module consumer")
    return registry, selected_entries


def _fixture(entry: dict[str, Any], model_path: Path, encoded_dir: Path) -> tuple[Path, Path]:
    source = entry.get("source")
    if not isinstance(source, dict) or source.get("kind") != "wat_file":
        raise MeasurementError(f"{entry.get('id')}: invalid continuation source identity")
    wat = model_path.parent / source.get("path", "")
    wasm = encoded_dir / (Path(source.get("path", "")).stem + ".wasm")
    if not wat.is_file() or not wasm.is_file():
        raise MeasurementError(f"{entry['id']}: WAT/Wasm fixture is missing")
    if source.get("sha256") != _sha(wat):
        raise MeasurementError(f"{entry['id']}: WAT hash mismatch")
    if source.get("wasm_sha256") != _sha(wasm):
        raise MeasurementError(f"{entry['id']}: encoded Wasm hash mismatch")
    return wat, wasm


def _verify_encoding(encoder: Path, source_root: Path,
                     files: dict[str, tuple[Path, str]], timeout: float) -> list[dict[str, str]]:
    """Run the existing encoder outside measured calls and verify exact model hashes."""
    records = []
    try:
        with tempfile.TemporaryDirectory(prefix="p012-encode-check-") as directory:
            temp = Path(directory)
            for index, (name, (wat, expected_hash)) in enumerate(sorted(files.items())):
                output = temp / f"{index:02d}-{Path(name).stem}.wasm"
                command = [str(encoder), "encode", str(wat), "-o", str(output)]
                try:
                    process = subprocess.run(command, cwd=source_root, capture_output=True,
                                             text=True, check=False, timeout=timeout)
                except (OSError, subprocess.TimeoutExpired) as exc:
                    raise MeasurementError(f"continuation encoder failed for {name}: {exc}") from exc
                if process.returncode != 0:
                    raise MeasurementError(
                        f"continuation encoder failed for {name} (exit {process.returncode}): {process.stderr.strip()}"
                    )
                actual = _sha(output)
                if actual != expected_hash:
                    raise MeasurementError(f"current encoder output hash mismatch for {name}: {actual}")
                records.append({"fixture": name, "wat_sha256": _sha(wat),
                                "wasm_sha256": actual, "encoder_sha256": _sha(encoder)})
    except OSError as exc:
        raise MeasurementError(f"cannot create temporary encoder outputs: {exc}") from exc
    return records


def _wire_values(values: Any, label: str, workload: str) -> list[str]:
    if not isinstance(values, list):
        raise MeasurementError(f"{workload}: {label} must be a list of typed wire values")
    result = []
    for item in values:
        if not isinstance(item, dict) or item.get("type") not in ("i32", "i64") or type(item.get("value")) is not int:
            raise MeasurementError(f"{workload}: unsupported {label} oracle value {item!r}")
        result.append(f"{item['type']}:{item['value']}")
    return result


def _positive_ns(value: Any, name: str, workload: str) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value) or value <= 0:
        raise MeasurementError(f"{workload}: {name} must be finite positive driver-reported nanoseconds")
    return float(value)


def _checkpoint(row: dict[str, Any], candidates: tuple[str, ...], workload: str) -> int:
    accounting = row.get("continuation_bytes")
    if not isinstance(accounting, dict):
        raise MeasurementError(f"{workload}: continuation byte checkpoints are missing")
    for name in candidates:
        value = accounting.get(name)
        if type(value) is int and value >= 0:
            return value
    raise MeasurementError(f"{workload}: missing continuation checkpoint {candidates}")


def _check_common(row: dict[str, Any], workload: str, expected_case: str,
                  expected_driver: str) -> None:
    if (row.get("schema_version") != 2 or row.get("case") != expected_case or
            row.get("driver") != expected_driver or row.get("mode") != MODE or
            row.get("profile") != PROFILE or row.get("instrumented") is not False):
        raise MeasurementError(f"{workload}: continuation driver identity/settings mismatch")


def _validate_capture_resume(entry: dict[str, Any], row: dict[str, Any], model) -> dict[str, Any]:
    workload = entry["id"]
    expected_suspend = entry.get("oracle", {}).get("expected_suspension")
    expected_resume = entry.get("oracle", {}).get("expected_resume")
    if not isinstance(expected_suspend, dict) or expected_suspend.get("kind") != "suspended":
        raise MeasurementError(f"{workload}: custom suspension oracle is missing")
    if not isinstance(expected_resume, dict) or expected_resume.get("kind") != "returned":
        raise MeasurementError(f"{workload}: custom resume oracle is missing")
    payload = _wire_values(expected_suspend.get("payload"), "expected payload", workload)
    result = _wire_values(expected_resume.get("result"), "expected result", workload)
    if row.get("payload") != payload or row.get("result") != result:
        raise MeasurementError(f"{workload}: suspension payload or resumed result differs from oracle")
    if workload.startswith("continuation_depth_"):
        depth = int(workload.rsplit("_", 1)[1])
        independent = model.depth_oracle(depth)
        frames = independent["captured_logical_frames"]
        locals_count = independent["captured_numeric_locals"]
        if row.get("logical_frames") != frames or row.get("numeric_locals") != locals_count:
            raise MeasurementError(f"{workload}: captured frame/local count differs from independent model")
        with_root = row.get("collection_with_root")
        live = with_root.get("live") if isinstance(with_root, dict) else None
        if type(live) is not int or live != 1:
            raise MeasurementError(f"{workload}: numeric continuation should retain exactly one live root")
        captured_bytes = _checkpoint(row, ("after_capture",), workload)
        per_frame = captured_bytes / frames
        timing_fields = ("setup_ns", "capture_ns", "resume_ns")
    elif workload == "continuation_captured_reference":
        with_root = row.get("collection_with_root")
        live = with_root.get("live") if isinstance(with_root, dict) else None
        if type(live) is not int or live != 2:
            raise MeasurementError("captured-reference continuation must retain exactly continuation+struct")
        captured = entry["oracle"].get("captured_reference", {})
        if captured.get("field_i32") != 77:
            raise MeasurementError("captured reference oracle lacks the expected struct field")
        captured_bytes = _checkpoint(row, ("after_capture",), workload)
        frames = None
        per_frame = None
        timing_fields = ("setup_ns", "capture_ns", "resume_ns")
    elif workload == PROVIDER_CASE:
        with_root = row.get("collection_with_root")
        live = with_root.get("live") if isinstance(with_root, dict) else None
        if type(live) is not int or live < 1:
            raise MeasurementError("cross-module continuation was not retained across collection")
        if row.get("exact_host_result_rejected") is not True:
            raise MeasurementError("cross-module negative host exact-result probe did not reject")
        probe_kind = row.get("exact_host_error_kind")
        if probe_kind is None and isinstance(row.get("exact_host_result"), dict):
            probe_kind = row["exact_host_result"].get("error_kind")
        if probe_kind is None:
            probe_kind = row.get("error_kind")
        if probe_kind != "Host":
            raise MeasurementError("cross-module negative host exact-result probe did not return Host error")
        captured_bytes = _checkpoint(row, ("after_capture",), workload)
        frames = None
        per_frame = None
        timing_fields = ("setup_ns", "capture_ns", "resume_ns")
    else:
        raise MeasurementError(f"{workload}: unsupported capture/resume case")
    before = _checkpoint(row, ("before_capture",), workload)
    transferred = _checkpoint(row, ("after_root_transfer", "after_transfer"), workload)
    after_resume = _checkpoint(row, ("after_resume",), workload)
    final = _checkpoint(row, ("after_drop_and_collect", "after_clear_and_collect",
                              "after_collect", "after_clear_collect"), workload)
    if before != 0 or captured_bytes <= before or transferred != captured_bytes or final != 0:
        raise MeasurementError(f"{workload}: continuation ownership/accounting checkpoints are inconsistent")
    after_collection = row.get("collection_after_drop", row.get("collection_after_cleanup", {}))
    if not isinstance(after_collection, dict) or after_collection.get("live") != 0:
        raise MeasurementError(f"{workload}: dropped continuation remained live after cleanup collection")
    timings = {name: _positive_ns(row.get(name), name, workload) for name in timing_fields}
    return {"driver_reported_times_ns": timings,
            "continuation_bytes": row["continuation_bytes"],
            "captured_logical_frames": frames,
            "captured_numeric_locals": row.get("numeric_locals"),
            "bytes_per_captured_frame": per_frame,
            "post_resume_bytes": after_resume,
            "collection_with_root": row["collection_with_root"],
            "collection_after_drop": after_collection}


def _validate_bound(entry: dict[str, Any], row: dict[str, Any]) -> dict[str, Any]:
    workload = entry["id"]
    _check_common(row, workload, workload, "bind")
    expected = _wire_values(entry["oracle"]["expected_resume"]["result"], "expected bind result", workload)
    if row.get("result") != expected or row.get("bind_result") != "i32:0":
        raise MeasurementError(f"{workload}: nested bind/resume result differs from oracle")
    if row.get("worker_calls") != entry["oracle"].get("expected_worker_calls"):
        raise MeasurementError(f"{workload}: bound worker call count differs from oracle")
    if _checkpoint(row, ("after_bind", "after_bind_stage"), workload) <= 0:
        raise MeasurementError(f"{workload}: bind did not retain its saved continuation chain")
    if _checkpoint(row, ("after_clear_and_collect", "after_collect", "after_clear_collect"), workload) != 0:
        raise MeasurementError(f"{workload}: cleared bound continuation remains retained")
    collection = row.get("collection", {})
    if not isinstance(collection, dict) or collection.get("live") != 0:
        raise MeasurementError(f"{workload}: bound continuation collection did not release state")
    return {"driver_reported_times_ns": {
                "setup_ns": _positive_ns(row.get("setup_ns"), "setup_ns", workload),
                "bind_ns": _positive_ns(row.get("bind_ns"), "bind_ns", workload),
                "resume_ns": _positive_ns(row.get("resume_ns"), "resume_ns", workload)},
            "result": row.get("result"), "bind_result": row.get("bind_result"),
            "worker_calls": row.get("worker_calls"),
            "continuation_bytes": row["continuation_bytes"], "collection": collection}


def _validate_one_shot(entry: dict[str, Any], row: dict[str, Any]) -> dict[str, Any]:
    workload = entry["id"]
    _check_common(row, workload, workload, "oneshot")
    expected_once = _wire_values(entry["oracle"]["expected_first_resume"]["result"],
                                 "expected first resume", workload)
    if row.get("first_result") != expected_once[0]:
        raise MeasurementError(f"{workload}: first continuation resume result differs from oracle")
    if row.get("result") != "trap" or row.get("error_kind") != "Trap":
        raise MeasurementError(f"{workload}: continuation reuse did not trap")
    message = row.get("error_message")
    if not isinstance(message, str) or "continuation" not in message.lower():
        raise MeasurementError(f"{workload}: second-use trap is not continuation-related")
    if "capture_result" in row and row["capture_result"] != "i32:0":
        raise MeasurementError(f"{workload}: one-shot capture result differs from the fixture oracle")
    if _checkpoint(row, ("after_clear_and_collect", "after_collect", "after_clear_collect"), workload) != 0:
        raise MeasurementError(f"{workload}: one-shot saved global retained continuation after cleanup")
    collection = row.get("collection", {})
    if not isinstance(collection, dict) or collection.get("live") != 0:
        raise MeasurementError(f"{workload}: one-shot cleanup collection retained live continuation state")
    return {"driver_reported_times_ns": {
                "setup_ns": _positive_ns(row.get("setup_ns"), "setup_ns", workload),
                "capture_ns": _positive_ns(row.get("capture_ns"), "capture_ns", workload),
                "resume_once_ns": _positive_ns(row.get("resume_once_ns"), "resume_once_ns", workload),
                "resume_reuse_ns": _positive_ns(row.get("resume_reuse_ns"), "resume_reuse_ns", workload)},
            "first_result": row.get("first_result"), "result": row.get("result"),
            "error_kind": row.get("error_kind"), "error_message": message,
            "continuation_bytes": row["continuation_bytes"], "collection": collection}


def _validate_probe(row: dict[str, Any], probe: str) -> dict[str, Any]:
    workload = "continuation_depth_0"
    driver_name = {"fuel": "probe_fuel", "fuel-positive": "probe_fuel-positive",
                   "budget": "probe_budget"}[probe]
    _check_common(row, workload, workload, driver_name)
    if probe in ("fuel", "budget"):
        expected_message = "fuel exhausted" if probe == "fuel" else "continuation byte budget"
        if row.get("error_kind") != "ResourceLimit" or row.get("error_message") != expected_message:
            raise MeasurementError(f"{probe} boundary probe failed at the wrong admission condition")
        if row.get("continuation_bytes") != 0:
            raise MeasurementError(f"{probe} failed capture published continuation bytes")
        gc_metrics = row.get("gc_payload_bytes")
        before = row.get("gc_payload_bytes_before", row.get("gc_payload_bytes_before_capture"))
        after = row.get("gc_payload_bytes_after", row.get("gc_payload_bytes_after_capture"))
        if isinstance(gc_metrics, dict):
            if before is None:
                before = gc_metrics.get("before_capture", gc_metrics.get("before"))
            if after is None:
                after = gc_metrics.get("after_failure", gc_metrics.get("after"))
        if probe == "budget" and (before is None or after is None):
            raise MeasurementError("zero-byte budget probe omitted before/after GC payload evidence")
        if ((before is not None and (type(before) is not int or before != 0)) or
                (after is not None and (type(after) is not int or after < 0))):
            raise MeasurementError(f"{probe} failure has invalid guest GC accounting")
        if row.get("continuation_bytes_after_collect") != 0 or row.get("gc_payload_bytes_after_collect") != 0:
            raise MeasurementError(f"{probe} failure leaked state after explicit collection")
        return {"probe": probe, "error_kind": row["error_kind"],
                "error_message": row["error_message"], "continuation_bytes": 0,
                "gc_payload_bytes_before": before, "gc_payload_bytes_after": after}
    if row.get("outcome") != "suspended" or row.get("payload") != ["i32:7"]:
        raise MeasurementError("five-fuel positive boundary did not successfully suspend with expected payload")
    if type(row.get("continuation_bytes")) is not int or row["continuation_bytes"] <= 0:
        raise MeasurementError("five-fuel positive boundary retained no continuation")
    gc_after = row.get("gc_payload_bytes_after_capture")
    if type(gc_after) is not int or gc_after <= 0:
        raise MeasurementError("five-fuel positive boundary retained no captured guest state")
    if row.get("continuation_bytes_after_collect") != 0:
        raise MeasurementError("five-fuel positive boundary did not release state after cleanup")
    return {"probe": probe, "outcome": row["outcome"], "payload": row["payload"],
            "continuation_bytes": row["continuation_bytes"],
            "gc_payload_bytes_after_capture": gc_after,
            "continuation_bytes_after_collect": row["continuation_bytes_after_collect"]}


def _run_json(command: list[str], timeout: float, label: str) -> dict[str, Any]:
    try:
        process = subprocess.run(command, capture_output=True, text=True,
                                 check=False, timeout=timeout)
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise MeasurementError(f"{label} invocation failed: {exc}") from exc
    if process.returncode != 0:
        raise MeasurementError(f"{label} failed with status {process.returncode}: {process.stderr.strip()}")
    try:
        row = json.loads(process.stdout)
    except json.JSONDecodeError as exc:
        raise MeasurementError(f"{label} did not return one JSON object: {exc}") from exc
    if not isinstance(row, dict):
        raise MeasurementError(f"{label} output must be a JSON object")
    return row


def measure(*, candidate: Path, encoder: Path, model_path: Path,
            registry_path: Path, encoded_dir: Path, source_root: Path | None = None,
            selection: str = "continuations", samples: int = SAMPLES,
            seed: int = 0x5012, timeout: float = 180.0) -> dict[str, Any]:
    if samples != SAMPLES:
        raise MeasurementError("P012 integrity run requires exactly five observations per measured case")
    if timeout <= 0 or not candidate.is_file() or not encoder.is_file():
        raise MeasurementError("candidate/encoder must exist and timeout must be positive")
    source_root = source_root or registry_path.resolve().parents[2]
    model = load_model(model_path)
    model_entries = model.workload_entries()
    registry, entries = _registry(registry_path, selection, model_entries)
    schema_path = model_path.parent / "workloads.py"
    if not schema_path.is_file():
        raise MeasurementError(f"Python workload schema source is missing: {schema_path}")
    model_by_id = {entry["id"]: entry for entry in model_entries}
    provider_entry = model_by_id[PROVIDER_ID]
    fixtures = {entry["id"]: _fixture(entry, model_path, encoded_dir) for entry in model_entries}
    files_to_encode = {
        entry["source"]["path"]: (fixtures[entry["id"]][0], entry["source"]["wasm_sha256"])
        for entry in model_entries
    }
    encoder_checks = _verify_encoding(encoder, source_root, files_to_encode, timeout)
    rust_sources = _rust_inputs(source_root)
    runner_path = Path(__file__).resolve()
    fixture_wat = {pair[0] for pair in fixtures.values()}
    fixture_wasm = {pair[1] for pair in fixtures.values()}
    source_paths = [candidate, encoder, runner_path, model_path, schema_path,
                    registry_path, *rust_sources, *fixture_wat, *fixture_wasm]
    before = _snapshot(source_paths)
    digest = lambda path: before[str(path.resolve())]
    protocol = _run_json([str(candidate), "--protocol"], timeout, "candidate handshake")
    if protocol.get("instrumented") is not False or MODE not in protocol.get("modes", []):
        raise MeasurementError("continuation timing requires an uninstrumented decoded binary")
    rng = random.Random(seed)
    rng.shuffle(entries)
    cases = []
    for entry in entries:
        workload = entry["id"]
        wat, wasm = fixtures[workload]
        if workload == PROVIDER_CASE:
            provider_wat, provider_wasm = fixtures[PROVIDER_ID]
            observations = []
            for sample in range(samples):
                command = [str(candidate), "--continuation-cross-module",
                           str(provider_wasm), str(wasm), MODE, PROFILE]
                row = _run_json(command, timeout, workload)
                _check_common(row, workload, workload, "cross_module")
                common = _validate_capture_resume(entry, row, model)
                observations.append({"sample": sample, "order": ["candidate"],
                                     "command": command, **common,
                                     "raw_driver_result": row})
            case_entry = {
                "workload": workload,
                "setup_provider_only": {"workload": PROVIDER_ID,
                                        "wat_sha256": digest(provider_wat),
                                        "wasm_sha256": digest(provider_wasm)},
                "fixture": {"wat_sha256": digest(wat), "wasm_sha256": digest(wasm)},
                "expected": entry.get("oracle"),
                "stages": entry.get("stages"),
                "wasmtime_capability": {"status": "unsupported", "timing": "not_measured",
                    "reason": "No equivalent configured Wasmtime stack-switching execution is available."},
                "observations": observations,
            }
        elif workload == "continuation_bound_nested":
            observations = []
            for sample in range(samples):
                command = [str(candidate), "--continuation-driver", str(wasm), workload,
                           MODE, PROFILE, "bind"]
                row = _run_json(command, timeout, workload)
                _check_common(row, workload, workload, "bind")
                validated = _validate_bound(entry, row)
                observations.append({"sample": sample, "order": ["candidate"],
                                     "command": command, **validated,
                                     "raw_driver_result": row})
            case_entry = {"workload": workload, "fixture": {"wat_sha256": digest(wat),
                           "wasm_sha256": digest(wasm)}, "expected": entry.get("oracle"),
                          "stages": entry.get("stages"), "observations": observations,
                          "wasmtime_capability": {"status": "unsupported", "timing": "not_measured",
                             "reason": "No equivalent configured Wasmtime stack-switching execution is available."}}
        elif workload == "continuation_one_shot":
            observations = []
            for sample in range(samples):
                command = [str(candidate), "--continuation-driver", str(wasm), workload,
                           MODE, PROFILE, "oneshot"]
                row = _run_json(command, timeout, workload)
                validated = _validate_one_shot(entry, row)
                observations.append({"sample": sample, "order": ["candidate"],
                                     "command": command, **validated,
                                     "raw_driver_result": row})
            case_entry = {"workload": workload, "fixture": {"wat_sha256": digest(wat),
                           "wasm_sha256": digest(wasm)}, "expected": entry.get("oracle"),
                          "stages": entry.get("stages"), "observations": observations,
                          "wasmtime_capability": {"status": "unsupported", "timing": "not_measured",
                             "reason": "No equivalent configured Wasmtime stack-switching execution is available."}}
        else:
            driver = "root" if workload == "continuation_captured_reference" else "depth"
            observations = []
            for sample in range(samples):
                command = [str(candidate), "--continuation-driver", str(wasm), workload,
                           MODE, PROFILE, driver]
                row = _run_json(command, timeout, workload)
                _check_common(row, workload, workload, driver)
                validated = _validate_capture_resume(entry, row, model)
                observations.append({"sample": sample, "order": ["candidate"],
                                     "command": command, **validated,
                                     "raw_driver_result": row})
            case_entry = {"workload": workload, "fixture": {"wat_sha256": digest(wat),
                           "wasm_sha256": digest(wasm)}, "expected": entry.get("oracle"),
                          "stages": entry.get("stages"), "observations": observations,
                          "wasmtime_capability": {"status": "unsupported", "timing": "not_measured",
                             "reason": "No equivalent configured Wasmtime stack-switching execution is available."}}
        cases.append(case_entry)

    depth_entry = model_by_id["continuation_depth_0"]
    _, depth0_wasm = fixtures["continuation_depth_0"]
    boundary_rows = []
    for probe in ("fuel", "fuel-positive", "budget"):
        command = [str(candidate), "--continuation-probe", str(depth0_wasm),
                   "continuation_depth_0", MODE, PROFILE, probe]
        row = _run_json(command, timeout, f"continuation boundary {probe}")
        validated = _validate_probe(row, probe)
        boundary_rows.append({"probe": probe, "command": command, "result": validated,
                              "raw_driver_result": row})
    after = _snapshot(source_paths)
    _assert_stable(before, after)
    depth_samples = [observation for case in cases if case["workload"].startswith("continuation_depth_")
                     for observation in case["observations"]]
    return {
        "schema_version": 1,
        "task": "P012",
        "selection": selection,
        "passed": True,
        "source_stable": True,
        "metadata": {
            "candidate_binary": str(candidate.resolve()), "candidate_binary_sha256": digest(candidate),
            "encoder_binary": str(encoder.resolve()), "encoder_binary_sha256": digest(encoder),
            "runner_source": str(runner_path), "runner_source_sha256": digest(runner_path),
            "model_source": str(model_path.resolve()), "model_source_sha256": digest(model_path),
            "python_schema_source": str(schema_path.resolve()), "python_schema_sha256": digest(schema_path),
            "registry_source": str(registry_path.resolve()), "registry_sha256": digest(registry_path),
            "rust_and_cargo_inputs": {str(path.resolve()): digest(path) for path in rust_sources},
            "fixture_wat_sha256": {str(path.resolve()): digest(path) for path in sorted(fixture_wat)},
            "fixture_wasm_sha256": {str(path.resolve()): digest(path) for path in sorted(fixture_wasm)},
            "encoder_reproduction_checks": encoder_checks,
            "source_snapshot": before,
            "mode": MODE, "profile": PROFILE, "instrumented": False,
            "samples_per_case": samples, "seed": seed,
            "host": {"platform": platform.platform(), "python": platform.python_version(),
                     "cpu_count": os.cpu_count(),
                     "affinity": sorted(os.sched_getaffinity(0)) if hasattr(os, "sched_getaffinity") else None},
            "timing_interpretation": {
                "setup_ns": "driver-reported compile/instantiate setup; engine construction is excluded",
                "capture_ns": "one guest run invocation ending in suspension",
                "resume_ns": "one later guest resume invocation",
                "bind_ns": "separately timed guest continuation binding operation",
                "resume_once_ns": "first guest resume operation on saved continuation",
                "resume_reuse_ns": "second-use guest trap path, timed separately",
                "accounting": "Store.continuation_bytes and collection observations are outside operation timers",
                "subprocess_wall_time": "not reported as guest timing",
            },
            "wasmtime": {"status": "unsupported", "timing": "not_measured",
                         "reason": "The configured comparator has no equivalent stack-switching support result."},
        },
        "case_execution_order": [case["workload"] for case in cases],
        "cases": cases,
        "boundary_probes": boundary_rows,
        "depth_frame_accounting": [{
            "workload": observation["raw_driver_result"]["case"],
            "sample": observation["sample"],
            "logical_frames": observation["captured_logical_frames"],
            "continuation_bytes_after_capture": observation["continuation_bytes"]["after_capture"],
            "bytes_per_captured_frame": observation["bytes_per_captured_frame"],
        } for observation in depth_samples],
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--encoder", type=Path, required=True)
    parser.add_argument("--model", type=Path, required=True)
    parser.add_argument("--registry", type=Path, required=True)
    parser.add_argument("--encoded-dir", type=Path, required=True)
    parser.add_argument("--source-root", type=Path)
    parser.add_argument("--selection", default="continuations")
    parser.add_argument("--samples", type=int, default=SAMPLES)
    parser.add_argument("--seed", type=int, default=0x5012)
    parser.add_argument("--timeout", type=float, default=180.0)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        report = measure(candidate=args.candidate, encoder=args.encoder,
                         model_path=args.model, registry_path=args.registry,
                         encoded_dir=args.encoded_dir, source_root=args.source_root,
                         selection=args.selection, samples=args.samples,
                         seed=args.seed, timeout=args.timeout)
        args.output.parent.mkdir(parents=True, exist_ok=True)
        temporary = args.output.with_suffix(args.output.suffix + ".tmp")
        temporary.write_text(json.dumps(report, indent=2, sort_keys=True, allow_nan=False) + "\n")
        temporary.replace(args.output)
    except (OSError, MeasurementError, TypeError, ValueError, KeyError) as exc:
        print(f"continuation_scaling.py: {exc}", file=sys.stderr)
        return 2
    print(json.dumps({"output": str(args.output), "cases": len(report["cases"]),
                      "samples_per_case": args.samples, "source_stable": report["source_stable"]},
                     sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
