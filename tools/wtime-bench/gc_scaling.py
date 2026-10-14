#!/usr/bin/env python3
"""Source-bound P011 GC/exception measurement runner draft.

Five independent decoded candidate invocations are retained for each selected
workload. Driver-reported setup, invocation, host-collection and collection
times remain separate; no Wasmtime timing or speedup is claimed.
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
MAX_MODULE_BYTES = 4 * 1024 * 1024


class MeasurementError(ValueError):
    pass


def sha256(path: Path) -> str:
    try:
        return hashlib.sha256(path.read_bytes()).hexdigest()
    except OSError as exc:
        raise MeasurementError(f"cannot hash {path}: {exc}") from exc


def load_model(path: Path):
    spec = importlib.util.spec_from_file_location("p011_gc_workloads", path)
    if spec is None or spec.loader is None:
        raise MeasurementError(f"cannot load independent workload model {path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    if not callable(getattr(module, "entries", None)) or not callable(getattr(module, "oracle", None)):
        raise MeasurementError("GC workload model must expose entries() and oracle()")
    return module


def load_workload_schema(path: Path):
    schema_path = path.parent / "workloads.py"
    spec = importlib.util.spec_from_file_location("p011_workload_schema", schema_path)
    if spec is None or spec.loader is None:
        raise MeasurementError(f"cannot load workload schema helpers {schema_path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    if not callable(getattr(module, "load_registry", None)) or not callable(getattr(module, "selection", None)):
        raise MeasurementError("workloads.py must expose load_registry() and selection()")
    try:
        registry = module.load_registry(path)
    except Exception as exc:
        raise MeasurementError(f"workload registry schema validation failed: {exc}") from exc
    return module, registry, schema_path


def _canonical_row(row: dict[str, Any]) -> str:
    return json.dumps(row, sort_keys=True, separators=(",", ":"), allow_nan=False)


def _source_snapshot(paths: list[Path]) -> dict[str, str]:
    resolved = sorted({path.resolve() for path in paths}, key=str)
    return {str(path): sha256(path) for path in resolved}


def _assert_stable(before: dict[str, str], after: dict[str, str]) -> None:
    if before == after:
        return
    changed = sorted(name for name in before.keys() & after.keys() if before[name] != after[name])
    missing = sorted(before.keys() - after.keys())
    added = sorted(after.keys() - before.keys())
    raise MeasurementError(
        f"P011 source or executable identity changed during measurement "
        f"(changed={changed[:4]}, missing={missing[:4]}, added={added[:4]})"
    )


def _repository_inputs(source_root: Path) -> list[Path]:
    root = source_root.resolve()
    required = [root / "Cargo.toml", root / "Cargo.lock",
                root / "tools/wtime-bench/Cargo.toml",
                root / "tools/wtime-bench/Cargo.lock"]
    missing = [str(path) for path in required if not path.is_file()]
    if missing:
        raise MeasurementError(f"missing Rust source identity inputs: {missing}")
    rust_sources: list[Path] = []
    for subtree in (root / "crates", root / "tools/wtime-bench"):
        if not subtree.is_dir():
            raise MeasurementError(f"missing Rust source tree {subtree}")
        rust_sources.extend(path for path in subtree.rglob("*.rs")
                            if "target" not in path.parts and ".git" not in path.parts)
    if not rust_sources:
        raise MeasurementError("runtime/benchmark Rust source snapshot is empty")
    optional = [root / "rust-toolchain", root / "rust-toolchain.toml",
                root / ".cargo/config", root / ".cargo/config.toml"]
    return sorted(set(required + [p for p in optional if p.is_file()] + rust_sources))


def _entry_fixtures(entry: dict[str, Any], model_path: Path,
                    encoded_dir: Path) -> tuple[Path, Path, list[tuple[Path, Path]]]:
    source = entry.get("source")
    if not isinstance(source, dict) or source.get("kind") != "wat_file":
        raise MeasurementError(f"{entry.get('id')}: invalid WAT source metadata")
    if source.get("encoding_status") != "encoded":
        raise MeasurementError(f"{entry.get('id')}: fixture is not marked encoded")
    wat = model_path.parent / source.get("path", "")
    wasm = encoded_dir / (Path(source.get("path", "")).stem + ".wasm")
    if not wat.is_file() or not wasm.is_file():
        raise MeasurementError(f"{entry.get('id')}: WAT or encoded Wasm fixture is missing")
    if wat.stat().st_size > MAX_MODULE_BYTES or wasm.stat().st_size > MAX_MODULE_BYTES:
        raise MeasurementError(f"{entry.get('id')}: fixture exceeds the 4 MiB driver bound")
    if source.get("sha256") != sha256(wat):
        raise MeasurementError(f"{entry['id']}: WAT hash differs from workload model")
    if source.get("wasm_sha256") != sha256(wasm):
        raise MeasurementError(f"{entry['id']}: Wasm hash differs from encoded fixture identity")
    setup: list[tuple[Path, Path]] = []
    setup_modules = entry.get("setup_modules", [])
    if not isinstance(setup_modules, list):
        raise MeasurementError(f"{entry['id']}: setup_modules must be a list")
    for module in setup_modules:
        if not isinstance(module, dict) or module.get("kind", "wat_file") != "wat_file":
            raise MeasurementError(f"{entry['id']}: malformed setup module")
        module_wat = model_path.parent / module.get("path", "")
        module_wasm = encoded_dir / (Path(module.get("path", "")).stem + ".wasm")
        if not module_wat.is_file() or not module_wasm.is_file():
            raise MeasurementError(f"{entry['id']}: setup module WAT/Wasm is missing")
        if module.get("sha256") != sha256(module_wat):
            raise MeasurementError(f"{entry['id']}: setup module WAT hash mismatch")
        if module.get("wasm_sha256") != sha256(module_wasm):
            raise MeasurementError(f"{entry['id']}: setup module Wasm hash mismatch")
        setup.append((module_wat, module_wasm))
    return wat, wasm, setup


def _verify_encoder_outputs(encoder: Path, source_root: Path,
                           expected: list[tuple[Path, str]], timeout: float) -> list[dict[str, str]]:
    """Re-encode outside measurement and require byte identity with the staged Wasm."""
    unique = {str(wat.resolve()): (wat.resolve(), digest)
              for wat, digest in expected}
    records: list[dict[str, str]] = []
    try:
        with tempfile.TemporaryDirectory(prefix="p011-encode-check-") as temp_name:
            temp = Path(temp_name)
            for index, (wat, expected_hash) in enumerate(sorted(unique.values(), key=lambda pair: str(pair[0]))):
                output = temp / f"{index:02d}-{wat.stem}.wasm"
                command = [str(encoder), "encode", str(wat), "-o", str(output)]
                try:
                    proc = subprocess.run(command, cwd=source_root, capture_output=True,
                                          text=True, check=False, timeout=timeout)
                except (OSError, subprocess.TimeoutExpired) as exc:
                    raise MeasurementError(f"fixture encoder failed for {wat}: {exc}") from exc
                if proc.returncode != 0:
                    raise MeasurementError(
                        f"fixture encoder failed for {wat} (exit {proc.returncode}): {proc.stderr.strip()}"
                    )
                actual_hash = sha256(output)
                if actual_hash != expected_hash:
                    raise MeasurementError(
                        f"current encoder output differs from staged fixture for {wat}: {actual_hash}"
                    )
                records.append({"wat": str(wat), "wasm_sha256": actual_hash,
                                "encoder_sha256": sha256(encoder)})
    except OSError as exc:
        raise MeasurementError(f"cannot create temporary encoder output: {exc}") from exc
    return records


def _expected_input(entry: dict[str, Any]) -> int:
    params = entry.get("params")
    if not isinstance(params, list) or len(params) != 1 or params[0].get("name") != "n":
        raise MeasurementError(f"{entry.get('id')}: expected one i32 n parameter")
    param = params[0]
    if param.get("type") != "i32":
        raise MeasurementError(f"{entry['id']}: driver supports only i32 workload inputs")
    n = param.get("baseline")
    if type(n) is not int or not 0 <= n <= 0x7fff_ffff:
        raise MeasurementError(f"{entry['id']}: invalid i32 baseline input")
    if param.get("min") != n or param.get("max") != n:
        raise MeasurementError(f"{entry['id']}: workload input must be fixed by the registry model")
    return n


def _positive_ns(value: Any, label: str, workload: str) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value) or value <= 0:
        raise MeasurementError(f"{workload}: {label} must be finite positive nanoseconds")
    return float(value)


def _check_gc_collection(entry: dict[str, Any], row: dict[str, Any], workload: str) -> None:
    expectation = entry.get("gc_expectation")
    if not isinstance(expectation, dict):
        return
    collection = row.get("collection")
    payload = row.get("gc_payload_bytes")
    is_cross = workload.startswith("gc_descriptor_cross_module_")
    continuation = row.get("continuation_bytes")
    if (not isinstance(collection, dict) or not isinstance(payload, dict) or
            (not is_cross and not isinstance(continuation, dict))):
        raise MeasurementError(f"{workload}: GC/collection retention fields are missing")
    live, reclaimed = collection.get("live"), collection.get("reclaimed")
    if type(live) is not int or live != 0 or type(reclaimed) is not int or reclaimed < 0:
        raise MeasurementError(f"{workload}: post-invocation collection state is invalid")
    count = expectation.get("allocation_count")
    if type(count) is not int or count < 0:
        raise MeasurementError(f"{workload}: model allocation count is invalid")
    if workload.startswith("gc_continuation_root_"):
        # CollectionStats counts two guest GC allocations plus the consumed
        # continuation record released after resume.
        if reclaimed != count + 1:
            raise MeasurementError(f"{workload}: expected {count} GC allocations plus one consumed continuation")
    elif reclaimed != count:
        raise MeasurementError(f"{workload}: reclaimed {reclaimed}, expected {count}")
    for name in ("after_collect",):
        value = payload.get(name)
        if type(value) is not int or value != 0:
            raise MeasurementError(f"{workload}: GC payload remains after explicit collection")
    if not is_cross and continuation.get("after_collect") != 0:
        raise MeasurementError(f"{workload}: continuation state remains after explicit collection")
    if is_cross:
        before_collect = payload.get("before_collect")
        if type(before_collect) is not int or before_collect <= 0:
            raise MeasurementError(f"{workload}: provider graph was not retained before collection")
    elif payload.get("before_invoke") != 0:
        raise MeasurementError(f"{workload}: fresh Store retained GC payload before invocation")
    after_invoke = payload.get("after_invoke")
    if not is_cross and count > 0 and (type(after_invoke) is not int or after_invoke <= 0):
        raise MeasurementError(f"{workload}: expected GC allocations were not retained before collection")


def _check_exception_cleanup(row: dict[str, Any], workload: str) -> None:
    collection = row.get("collection")
    payload = row.get("gc_payload_bytes")
    continuation = row.get("continuation_bytes")
    if not all(isinstance(value, dict) for value in (collection, payload, continuation)):
        raise MeasurementError(f"{workload}: exception cleanup metrics are missing")
    if (type(collection.get("live")) is not int or collection["live"] != 0 or
            type(collection.get("reclaimed")) is not int or collection["reclaimed"] < 0):
        raise MeasurementError(f"{workload}: exception cleanup collection state is invalid")
    if (type(payload.get("after_collect")) is not int or payload["after_collect"] != 0 or
            type(continuation.get("after_collect")) is not int or continuation["after_collect"] != 0):
        raise MeasurementError(f"{workload}: exception cleanup left GC or continuation state")


def _check_root_lifecycle(entry: dict[str, Any], row: dict[str, Any], expected_result: int,
                          workload: str) -> dict[str, float]:
    if row.get("driver") != "root_lifecycle" or row.get("outcome") is not None:
        raise MeasurementError(f"{workload}: root lifecycle driver/result shape mismatch")
    if row.get("result") != f"i32:{expected_result}":
        raise MeasurementError(f"{workload}: root checksum differs from independent oracle")
    if row.get("root_category") != "host_value_root":
        raise MeasurementError(f"{workload}: root identity was not normalized")
    rooted, released = row.get("rooted_collection"), row.get("released_collection")
    if not isinstance(rooted, dict) or not isinstance(released, dict):
        raise MeasurementError(f"{workload}: root lifecycle collection states are missing")
    if (rooted.get("live"), rooted.get("reclaimed")) != (1, 0):
        raise MeasurementError(f"{workload}: host root did not retain exactly one object")
    if (released.get("live"), released.get("reclaimed")) != (0, 1):
        raise MeasurementError(f"{workload}: dropping host root did not reclaim its object")
    payload = row.get("gc_payload_bytes")
    if not isinstance(payload, dict) or payload.get("before") != 0 or payload.get("after_checksum") != 0:
        raise MeasurementError(f"{workload}: checksum objects were not collected before host root")
    if type(payload.get("after_rooted_collect")) is not int or payload["after_rooted_collect"] <= 0:
        raise MeasurementError(f"{workload}: rooted object has no retained GC payload")
    if payload.get("after_release") != 0:
        raise MeasurementError(f"{workload}: GC payload remains after root release")
    expected_after_checksum = _expected_input(entry)
    collect = row.get("collect_ns")
    if not isinstance(collect, dict):
        raise MeasurementError(f"{workload}: separate collection timings are missing")
    times = {"setup_ns": row.get("setup_ns"), "checksum_ns": row.get("checksum_ns"),
             "make_ns": row.get("make_ns"),
             "collect_after_checksum_ns": collect.get("after_checksum"),
             "collect_with_root_ns": collect.get("with_root"),
             "collect_after_release_ns": collect.get("after_release")}
    for label, value in times.items():
        times[label] = _positive_ns(value, label, workload)
    seq = entry.get("collector_stage", {}).get("sequence", [])
    if len(seq) != 4 or seq[0].get("expected", {}).get("reclaimed") != "n":
        raise MeasurementError(f"{workload}: root lifecycle oracle contract is malformed")
    return times


def _validate_row(entry: dict[str, Any], row: dict[str, Any], n: int,
                  model, expected_wat: str, expected_wasm: str) -> dict[str, Any]:
    workload = entry["id"]
    is_cross = workload.startswith("gc_descriptor_cross_module_")
    protocol_case = "gc_descriptor_cross_module" if is_cross else workload
    if (row.get("schema_version") != 2 or row.get("case") != protocol_case or
            row.get("input") != f"i32:{n}" or row.get("mode") != MODE or
            row.get("profile") != PROFILE or row.get("instrumented") is not False):
        raise MeasurementError(f"{workload}: protocol case/input/settings mismatch")
    if (row.get("driver") != ("cross_module" if is_cross else
                               "root_lifecycle" if workload.startswith("gc_root_lifecycle_") else "single")):
        raise MeasurementError(f"{workload}: driver path identity mismatch")
    capability = row.get("wasmtime")
    if (not isinstance(capability, dict) or capability.get("status") != "unsupported" or
            not capability.get("reason")):
        raise MeasurementError(f"{workload}: Wasmtime must remain explicitly unsupported without timing")
    complete = row.get("report_complete")
    if complete is not None and complete is not True:
        raise MeasurementError(f"{workload}: driver marked report incomplete")
    if (not is_cross and not workload.startswith("gc_root_lifecycle_") and complete is not True):
        raise MeasurementError(f"{workload}: driver did not mark a complete row")
    oracle = model.oracle(workload, n)
    if isinstance(oracle, int):
        expected_result = f"i32:{oracle}"
        if row.get("result") != expected_result:
            raise MeasurementError(f"{workload}: result differs from independent checksum oracle")
        if row.get("outcome", "returned") != "returned":
            raise MeasurementError(f"{workload}: expected a returned checksum")
        if row.get("tag") is not None or row.get("payload") is not None:
            raise MeasurementError(f"{workload}: unexpected exception identity/payload")
    elif isinstance(oracle, dict) and oracle.get("kind") == "exception":
        if row.get("outcome") != "exception" or row.get("result") is not None:
            raise MeasurementError(f"{workload}: expected an uncaught exception outcome")
        if row.get("tag") != oracle.get("tag"):
            raise MeasurementError(f"{workload}: exception tag differs from independent oracle")
        expected_payload = [f"i32:{value}" for value in oracle.get("payload", [])]
        if row.get("payload") != expected_payload:
            raise MeasurementError(f"{workload}: exception payload differs from independent oracle")
    else:
        raise MeasurementError(f"{workload}: unsupported independent oracle shape")

    if is_cross and row.get("normalized_provider_object") != "descriptor_child_graph":
        raise MeasurementError(f"{workload}: cross-module identity was not normalized")
    timings: dict[str, float]
    if workload.startswith("gc_root_lifecycle_"):
        timings = _check_root_lifecycle(entry, row, oracle, workload)
    else:
        timings = {"setup_ns": _positive_ns(row.get("setup_ns"), "setup_ns", workload)}
        invoke_name = "invoke_ns"
        if is_cross:
            timings[invoke_name] = _positive_ns(row.get("invoke_ns"), invoke_name, workload)
        else:
            timings[invoke_name] = _positive_ns(row.get("invoke_ns"), invoke_name, workload)
        collection = row.get("collection")
        if not isinstance(collection, dict):
            raise MeasurementError(f"{workload}: explicit collection observation is missing")
        timings["collect_ns"] = _positive_ns(collection.get("ns"), "collection.ns", workload)
        _check_gc_collection(entry, row, workload)
        if workload.startswith("exception_"):
            _check_exception_cleanup(row, workload)
        if workload.startswith("gc_host_reentry_collect_"):
            host = row.get("host_collection")
            expected_host = entry.get("collector_stage", {}).get("expected", {})
            if not isinstance(host, dict) or (host.get("live"), host.get("reclaimed")) != (
                    expected_host.get("live"), expected_host.get("reclaimed")):
                raise MeasurementError(f"{workload}: reentrant collection did not preserve temporary root")
            payload = host.get("gc_payload_bytes")
            if type(payload) is not int or payload <= 0:
                raise MeasurementError(f"{workload}: temporary object has no retained GC payload")
            timings["host_collect_ns"] = _positive_ns(host.get("ns"), "host_collection.ns", workload)
        if workload.startswith("gc_continuation_root_"):
            host = row.get("host_collection")
            expected_host = entry.get("collector_stage", {}).get("expected", {})
            minimum_live = expected_host.get("live_min")
            if (not isinstance(host, dict) or type(minimum_live) is not int or
                    type(host.get("live")) is not int or host["live"] < minimum_live or
                    host.get("reclaimed") != expected_host.get("reclaimed")):
                raise MeasurementError(f"{workload}: suspended continuation did not retain its root graph")
            timings["host_collect_ns"] = _positive_ns(host.get("ns"), "host_collection.ns", workload)
    return {
        "workload": workload,
        "input": f"i32:{n}",
        "expected": oracle,
        "fixture": {"wat_sha256": expected_wat, "wasm_sha256": expected_wasm},
        "protocol_case": protocol_case,
        "driver": row["driver"],
        "driver_reported_times_ns": timings,
        "raw_driver_result": row,
    }


def _invoke(candidate: Path, entry: dict[str, Any], wasm: Path,
            setup: list[tuple[Path, Path]], n: int, timeout: float) -> tuple[dict[str, Any], list[str]]:
    workload = entry["id"]
    if workload.startswith("gc_descriptor_cross_module_"):
        if len(setup) != 1 or setup[0][0].stem != "descriptor_provider":
            raise MeasurementError(f"{workload}: cross-module provider fixture missing")
        command = [str(candidate), "--gc-exception-cross-module", str(setup[0][1]), str(wasm),
                   f"i32:{n}", MODE, PROFILE]
    else:
        command = [str(candidate), "--gc-exception-driver", str(wasm), workload,
                   f"i32:{n}", MODE, PROFILE]
    try:
        proc = subprocess.run(command, capture_output=True, text=True, check=False, timeout=timeout)
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise MeasurementError(f"candidate invocation failed for {workload}: {exc}") from exc
    if proc.returncode != 0:
        raise MeasurementError(f"candidate invocation failed for {workload} (exit {proc.returncode}): {proc.stderr.strip()}")
    try:
        result = json.loads(proc.stdout)
    except json.JSONDecodeError as exc:
        raise MeasurementError(f"candidate output for {workload} is not one JSON object: {exc}") from exc
    if not isinstance(result, dict):
        raise MeasurementError(f"candidate output for {workload} must be a JSON object")
    return result, command


def measure(*, candidate: Path, encoder: Path, model_path: Path, registry_path: Path,
            encoded_dir: Path, source_root: Path | None = None,
            selection: str = "gc-exceptions", samples: int = SAMPLES,
            seed: int = 0x5011, timeout: float = 120.0) -> dict[str, Any]:
    if samples != SAMPLES:
        raise MeasurementError("P011 integrity report requires exactly five observations per workload")
    if timeout <= 0:
        raise MeasurementError("process timeout must be positive")
    if not candidate.is_file() or not encoder.is_file():
        raise MeasurementError("candidate and fixture encoder executables must exist")
    source_root = source_root or registry_path.resolve().parents[2]
    source_inputs = _repository_inputs(source_root)
    if not model_path.is_file() or not registry_path.is_file():
        raise MeasurementError("workload model and registry must exist")
    model = load_model(model_path)
    model_entries = model.entries()
    by_id = {entry.get("id"): entry for entry in model_entries if isinstance(entry, dict)}
    if len(by_id) != len(model_entries) or len(by_id) != 45:
        raise MeasurementError("P011 model must contain exactly 45 unique workload entries")
    schema, registry, schema_path = load_workload_schema(registry_path)
    try:
        selected_rows = schema.selection(selection, registry)
    except Exception as exc:
        raise MeasurementError(f"registry selection {selection!r} is not launchable: {exc}") from exc
    selected_ids = [row.get("id") for row in selected_rows]
    if (len(selected_rows) != 45 or len(selected_ids) != len(set(selected_ids)) or
            set(selected_ids) != set(by_id)):
        raise MeasurementError("registry selection must contain all 45 unique P011 workload entries")
    for registry_row in selected_rows:
        workload = registry_row["id"]
        if _canonical_row(registry_row) != _canonical_row(by_id[workload]):
            raise MeasurementError(f"{workload}: selected registry row differs from independent model record")
    entries = list(selected_rows)
    fixtures = {entry["id"]: _entry_fixtures(entry, model_path, encoded_dir) for entry in entries}
    wat_paths = {pair[0] for pair in fixtures.values()}
    wasm_paths = {pair[1] for pair in fixtures.values()}
    for _, _, setup in fixtures.values():
        for module_wat, module_wasm in setup:
            wat_paths.add(module_wat)
            wasm_paths.add(module_wasm)
    expected_encoding = []
    for entry in entries:
        source = entry["source"]
        wat, _, setup = fixtures[entry["id"]]
        expected_encoding.append((wat, source["wasm_sha256"]))
        for module in entry.get("setup_modules", []):
            expected_encoding.append((model_path.parent / module["path"], module["wasm_sha256"]))
    encoder_verification = _verify_encoder_outputs(encoder, source_root,
                                                   expected_encoding, timeout)
    runner_path = Path(__file__).resolve()
    source_paths = [candidate, encoder, model_path, registry_path, schema_path, runner_path,
                    *source_inputs, *wat_paths, *wasm_paths]
    before = _source_snapshot(source_paths)
    digest = lambda path: before[str(path.resolve())]
    rng = random.Random(seed)
    rng.shuffle(entries)
    cases = []
    for entry in entries:
        workload = entry["id"]
        n = _expected_input(entry)
        wat, wasm, setup = fixtures[workload]
        comparator = entry.get("comparators", {}).get("wasmtime", {})
        if comparator.get("status") != "unsupported":
            raise MeasurementError(f"{workload}: unsupported Wasmtime policy must remain explicit")
        observations = []
        for sample in range(samples):
            raw, command = _invoke(candidate, entry, wasm, setup, n, timeout)
            normalized = _validate_row(entry, raw, n, model,
                                       digest(wat), digest(wasm))
            observations.append({"sample": sample,
                                 "order": ["candidate"],
                                "command": command,
                                 "driver_reported_times_ns": normalized["driver_reported_times_ns"],
                                 "result": normalized})
        cases.append({
            "workload": workload,
            "family": entry.get("family"),
            "input": f"i32:{n}",
            "required_features": entry.get("required_features", []),
            "entrypoint": entry.get("entrypoint"),
            "expected_outcome": entry.get("expected_outcome"),
            "expected": model.oracle(workload, n),
            "gc_expectation": entry.get("gc_expectation"),
            "collector_stage": entry.get("collector_stage"),
            "exception_expectation": entry.get("exception_expectation"),
            "fixture": {"wat_sha256": digest(wat), "wasm_sha256": digest(wasm)},
            "setup_modules": [{
                "path": str(module_wat.resolve()), "wat_sha256": digest(module_wat),
                "wasm_sha256": digest(module_wasm),
            } for module_wat, module_wasm in setup],
            "wasmtime_capability": {
                "status": "unsupported",
                "reason": comparator.get("reason"),
                "timing": "not_measured",
            },
            "observations": observations,
        })
    after = _source_snapshot(source_paths)
    _assert_stable(before, after)
    return {
        "schema_version": 1,
        "task": "P011",
        "selection": selection,
        "passed": True,
        "source_stable": True,
        "metadata": {
            "candidate_binary": str(candidate.resolve()),
            "candidate_binary_sha256": digest(candidate),
            "encoder_binary": str(encoder.resolve()),
            "encoder_binary_sha256": digest(encoder),
            "runner_source": str(runner_path),
            "runner_source_sha256": digest(runner_path),
            "model_source": str(model_path.resolve()),
            "model_source_sha256": digest(model_path),
            "registry_source": str(registry_path.resolve()),
            "registry_source_sha256": digest(registry_path),
            "workload_schema_source": str(schema_path.resolve()),
            "workload_schema_source_sha256": digest(schema_path),
            "runtime_and_benchmark_sources": {
                str(path.resolve()): digest(path) for path in source_inputs
            },
            "fixture_wat_sha256": {str(path.resolve()): digest(path) for path in sorted(wat_paths)},
            "fixture_wasm_sha256": {str(path.resolve()): digest(path) for path in sorted(wasm_paths)},
            "encoder_reproduction_checks": encoder_verification,
            "source_snapshot": before,
            "mode": MODE,
            "profile": PROFILE,
            "instrumented": False,
            "seed": seed,
            "observations_per_workload": samples,
            "host": {
                "platform": platform.platform(),
                "python": platform.python_version(),
                "cpu_count": os.cpu_count(),
                "affinity": sorted(os.sched_getaffinity(0)) if hasattr(os, "sched_getaffinity") else None,
            },
            "timing_interpretation": {
                "setup_ns": "driver-reported module compilation, Store/linker setup and instantiation; Engine construction is excluded",
                "invoke_ns": "driver-reported Store::invoke duration for one guest call",
                "collection_ns": "driver-reported explicit Store::collect duration, separate from guest invocation",
                "host_collection_ns": "driver-reported reentrant collection call inside the host callback",
                "root_lifecycle": "checksum, make and each collect are separately timed observations",
                "process_wall_time": "not used as guest execution time; subprocess startup and JSON handling are not timed",
            },
            "continuation_collection_accounting": (
                "model allocation_count is guest GC allocations; CollectionStats.reclaimed "
                "must include those plus one consumed continuation record"
            ),
            "wasmtime": {
                "status": "unsupported",
                "reason": "The pinned Wasmtime build does not include the required GC/exception support.",
                "timing": "not_measured",
            },
        },
        "case_execution_order": [case["workload"] for case in cases],
        "cases": cases,
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--encoder", type=Path, required=True)
    parser.add_argument("--model", type=Path, required=True)
    parser.add_argument("--registry", type=Path, required=True)
    parser.add_argument("--encoded-dir", type=Path, required=True)
    parser.add_argument("--source-root", type=Path)
    parser.add_argument("--selection", default="gc-exceptions")
    parser.add_argument("--samples", type=int, default=SAMPLES)
    parser.add_argument("--seed", type=int, default=0x5011)
    parser.add_argument("--timeout", type=float, default=120.0)
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
        print(f"p011_gc_exception_runner.py: {exc}", file=sys.stderr)
        return 2
    print(json.dumps({"output": str(args.output), "cases": len(report["cases"]),
                      "samples_per_case": args.samples,
                      "source_stable": report["source_stable"]}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
