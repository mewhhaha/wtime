#!/usr/bin/env python3
"""Source-bound P013 shared-memory measurement runner.

The runner does not claim a Wasmtime comparison: the checked-in capability
metadata says this build lacks threads/shared-memory support. Worker timings
cover the driver's start-barrier-to-last-result interval only. Wait samples
cover guest notify invocation start through waiter guest return.
"""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import platform
import subprocess
import sys
import tempfile
from typing import Any


class ThreadMeasurementError(ValueError):
    pass


def sha256(path: Path) -> str:
    try:
        return hashlib.sha256(path.read_bytes()).hexdigest()
    except OSError as exc:
        raise ThreadMeasurementError(f"cannot hash {path}: {exc}") from exc


def load_module(path: Path, name: str):
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise ThreadMeasurementError(f"cannot load Python module {path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


def load_model(path: Path):
    module = load_module(path, "p013_thread_workloads")
    for name in ("entries", "oracle", "effective_workers", "summarize_latency", "encode_fixtures"):
        if not callable(getattr(module, name, None)):
            raise ThreadMeasurementError(f"thread model must expose {name}()")
    return module


def source_snapshot(paths: list[Path]) -> dict[str, str]:
    unique = {path.resolve() for path in paths}
    missing = sorted(str(path) for path in unique if not path.is_file())
    if missing:
        raise ThreadMeasurementError(f"required source identity files are missing: {missing[:5]}")
    return {str(path): sha256(path) for path in sorted(unique, key=str)}


def assert_stable(before: dict[str, str], after: dict[str, str]) -> None:
    if before != after:
        changed = sorted(path for path in before.keys() & after.keys() if before[path] != after[path])
        missing, added = sorted(before.keys() - after.keys()), sorted(after.keys() - before.keys())
        raise ThreadMeasurementError(
            f"source/binary identity changed during run (changed={changed[:4]}, "
            f"missing={missing[:4]}, added={added[:4]})"
        )


def _schema_and_registry(schema_path: Path, registry_path: Path, selection_name: str,
                         model_entries: list[dict[str, Any]]) -> list[dict[str, Any]]:
    schema = load_module(schema_path, "p013_workload_schema")
    try:
        registry = schema.load_registry(registry_path)
        selected = schema.selection(selection_name, registry)
    except Exception as exc:
        raise ThreadMeasurementError(f"workload schema/selection rejected registry: {exc}") from exc
    if len(model_entries) != 18 or len(selected) != 18:
        raise ThreadMeasurementError("P013 requires all 18 registered thread workloads")
    by_id = {entry.get("id"): entry for entry in model_entries}
    selected_by_id = {entry.get("id"): entry for entry in selected}
    if len(by_id) != len(model_entries) or len(selected_by_id) != len(selected):
        raise ThreadMeasurementError("duplicate or malformed workload IDs")
    if set(by_id) != set(selected_by_id):
        raise ThreadMeasurementError("registry selection does not cover the full thread model")
    for workload, entry in by_id.items():
        if selected_by_id[workload] != entry:
            raise ThreadMeasurementError(f"{workload}: full registry row differs from model entry")
    if registry["selections"][selection_name]["status"] != "ready":
        raise ThreadMeasurementError("thread workload selection is not ready")
    return selected


def validate_fixtures(entries: list[dict[str, Any]], model_path: Path,
                      encoded_dir: Path, encoder: Path, model) -> tuple[dict[str, tuple[Path, Path]], dict[str, str]]:
    fixtures: dict[str, tuple[Path, Path]] = {}
    expected_encoded: dict[str, str] = {}
    for entry in entries:
        workload = entry.get("id")
        source = entry.get("source")
        if not isinstance(workload, str) or not isinstance(source, dict) or source.get("kind") != "wat_file":
            raise ThreadMeasurementError("thread workload has malformed source identity")
        relative = source.get("path")
        if not isinstance(relative, str) or Path(relative).is_absolute() or ".." in Path(relative).parts:
            raise ThreadMeasurementError(f"{workload}: unsafe or missing fixture path")
        wat = model_path.parent / relative
        wasm = encoded_dir / (Path(relative).stem + ".wasm")
        if not wat.is_file() or not wasm.is_file():
            raise ThreadMeasurementError(f"{workload}: missing WAT/Wasm fixture")
        if sha256(wat) != source.get("sha256"):
            raise ThreadMeasurementError(f"{workload}: WAT fixture hash mismatch")
        if sha256(wasm) != source.get("wasm_sha256") or source.get("encoding_status") != "encoded":
            raise ThreadMeasurementError(f"{workload}: Wasm fixture hash/status mismatch")
        fixtures[workload] = (wat, wasm)
        expected_encoded[Path(relative).name] = source["wasm_sha256"]

    # Re-encode every unique fixture outside all guest timers and bind the
    # produced bytes to both model hashes and the driver input artifacts.
    with tempfile.TemporaryDirectory(prefix="p013-thread-encode-") as temp:
        try:
            encoded = model.encode_fixtures(encoder, temp)
        except Exception as exc:
            raise ThreadMeasurementError(f"fixture re-encoding failed: {exc}") from exc
        if encoded != expected_encoded:
            raise ThreadMeasurementError("encoder output hashes differ from full model fixture set")
    return fixtures, expected_encoded


def _run(command: list[str], timeout: float) -> subprocess.CompletedProcess[str]:
    try:
        return subprocess.run(command, capture_output=True, text=True, check=False, timeout=timeout)
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise ThreadMeasurementError(f"driver command failed: {exc}") from exc


def run_object(command: list[str], timeout: float) -> dict[str, Any]:
    proc = _run(command, timeout)
    if proc.returncode != 0:
        raise ThreadMeasurementError(f"command failed (exit {proc.returncode}): {proc.stderr.strip()}")
    try:
        value = json.loads(proc.stdout)
    except json.JSONDecodeError as exc:
        raise ThreadMeasurementError(f"command stdout is not one JSON object: {exc}") from exc
    if not isinstance(value, dict):
        raise ThreadMeasurementError("command JSON root must be an object")
    return value


def run_json(command: list[str], timeout: float) -> dict[str, Any]:
    row = run_object(command, timeout)
    if row.get("schema_version") != 1 or row.get("status") != "ok":
        raise ThreadMeasurementError(f"driver returned incomplete/unsupported result: {row}")
    return row


def validate_protocol(protocol: dict[str, Any]) -> dict[str, Any]:
    if (protocol.get("schema_version") != 2 or protocol.get("engine") != "wtime" or
            protocol.get("instrumented") is not False or
            not isinstance(protocol.get("modes"), list) or "decoded" not in protocol["modes"]):
        raise ThreadMeasurementError(f"driver protocol does not prove uninstrumented decoded mode: {protocol}")
    return protocol


def _validate_atomic(entry: dict[str, Any], result: dict[str, Any], model) -> dict[str, Any]:
    settings = entry.get("thread_config", {})
    requested, iterations = settings.get("requested_workers"), settings.get("iterations_per_worker")
    quota, workers = result.get("worker_quota"), result.get("effective_workers")
    if type(quota) is not int or quota < 1 or type(workers) is not int:
        raise ThreadMeasurementError(f"{entry['id']}: invalid runtime quota/effective worker metadata")
    expected_workers = model.effective_workers(requested, quota)
    if workers != expected_workers:
        raise ThreadMeasurementError(f"{entry['id']}: driver effective worker count differs from quota oracle")
    if (result.get("case") != entry["id"] or result.get("requested_workers") != requested or
            result.get("iterations_per_worker") != iterations or
            result.get("timing_boundary") != "start_barrier_to_last_worker_result_receive; setup/readback/join excluded"):
        raise ThreadMeasurementError(f"{entry['id']}: driver case/configuration/timing boundary mismatch")
    expected = model.oracle(entry["id"], workers, iterations)
    walls, returns, memory, trials = (result.get("trial_wall_ns"), result.get("trial_worker_returns"),
                                      result.get("trial_final_memory_values"), result.get("trials"))
    if type(trials) is not int or trials != 21 or any(
            not isinstance(values, list) or len(values) != trials for values in (walls, returns, memory)):
        raise ThreadMeasurementError(f"{entry['id']}: exactly 21 complete independent trials are required")
    seeds = result.get("trial_seeds")
    if (not isinstance(seeds, list) or len(seeds) != trials or
            any(type(value) is not int or value <= 0 for value in seeds) or len(set(seeds)) != trials):
        raise ThreadMeasurementError(f"{entry['id']}: trial seeds must be positive and unique")
    if any(type(value) is not int or value <= 0 for value in walls):
        raise ThreadMeasurementError(f"{entry['id']}: wall observations must be positive integer nanoseconds")
    if any(row != expected["worker_returns"] for row in returns):
        raise ThreadMeasurementError(f"{entry['id']}: worker return vectors differ from oracle")
    if any(row != expected["final_memory_values"] for row in memory):
        raise ThreadMeasurementError(f"{entry['id']}: final memory differs from oracle")
    if result.get("total_operations_per_trial") != workers * iterations:
        raise ThreadMeasurementError(f"{entry['id']}: operation count is inconsistent")
    return {"effective_workers": workers, "worker_quota": quota, "expected": expected,
            "trial_wall_ns": walls, "trial_worker_returns": returns,
            "trial_final_memory_values": memory, "trial_seeds": seeds}


def _validate_sc(entry: dict[str, Any], result: dict[str, Any], model) -> dict[str, Any]:
    expected = model.oracle("threads_sc_store_buffering", 2, 1024)
    trials, walls, outcomes = result.get("trials"), result.get("trial_wall_ns"), result.get("trial_results")
    if (result.get("case") != entry["id"] or type(trials) is not int or trials != 1024 or
            result.get("memories_per_trial") != 2 or result.get("reset_per_trial") is not True or
            not isinstance(walls, list) or len(walls) != trials or
            not isinstance(outcomes, list) or len(outcomes) != trials):
        raise ThreadMeasurementError("SC litmus must retain all 1024 two-memory reset trials")
    seeds = result.get("trial_seeds")
    if (not isinstance(seeds, list) or len(seeds) != trials or
            any(type(seed) is not int or seed <= 0 for seed in seeds) or len(set(seeds)) != trials):
        raise ThreadMeasurementError("SC litmus trial seeds must be positive and unique")
    if any(type(ns) is not int or ns <= 0 for ns in walls):
        raise ThreadMeasurementError("SC wall observations must be positive integer nanoseconds")
    if any(not isinstance(pair, list) or len(pair) != 2 or
           any(type(value) is not int or value not in (0, 1) for value in pair) for pair in outcomes):
        raise ThreadMeasurementError("SC litmus returned malformed values")
    if any(pair == expected["forbidden_outcome"] for pair in outcomes):
        raise ThreadMeasurementError("SC litmus observed forbidden [0,0] result")
    if result.get("forbidden_result") != expected["forbidden_outcome"]:
        raise ThreadMeasurementError("SC forbidden outcome declaration differs from independent oracle")
    return {"trial_wall_ns": walls, "trial_results": outcomes,
            "forbidden_outcome": expected["forbidden_outcome"], "trial_seeds": seeds}


def _validate_wait_latency(entry: dict[str, Any], raw: dict[str, Any], model,
                           sample_count: int, timeout_ns: int) -> dict[str, Any]:
    samples = raw.get("notify_to_return_ns")
    if (raw.get("case") != entry["id"] or raw.get("sample_kind") != "one_wait_notify_round_trip" or
            raw.get("samples") != sample_count or raw.get("sample_count") != sample_count or
            raw.get("timeout_ns") != timeout_ns or raw.get("registration_observed") is not True or
            raw.get("outcome") != "notified" or not isinstance(samples, list) or len(samples) != sample_count):
        raise ThreadMeasurementError("wait driver settings/status do not match 1024 individual observations")
    evidence = "successful notify returned one for the address before waiter return"
    if raw.get("registration_evidence") != evidence:
        raise ThreadMeasurementError("wait registration evidence differs from accepted public-API proof")
    observations = [{"sample_kind": raw["sample_kind"], "registration_observed": True,
                     "outcome": raw["outcome"], "notify_to_return_ns": value} for value in samples]
    try:
        summary = model.summarize_latency(observations, minimum_samples=100)
    except Exception as exc:
        raise ThreadMeasurementError(f"invalid wait-latency sample: {exc}") from exc
    if summary["sample_count"] != 1024 or summary["p99_status"] != "reported":
        raise ThreadMeasurementError("wait-latency summary lacks the required 1024 individual samples")
    return {"latency": summary, "registration_evidence": evidence,
            "registration_limit": (
                "There is no independent public pre-timer registration observer. A successful notify "
                "returning one proves registration at that call's linearization point; each reported "
                "interval starts before the guest notify call and ends at guest waiter return, so it "
                "includes notify invocation overhead. Failed notify(1)==0 probes are excluded."
            )}


def _validate_wait_cleanup(raw: dict[str, Any], timeout_ns: int, model) -> dict[str, Any]:
    expected = {
        "schema_version": 1, "status": "ok", "case": "threads_wait_cleanup",
        "timeout_ns": timeout_ns, "timeout_result": model.wait_oracle("timeout_cleanup")["wait_result"],
        "value_mismatch_result": model.wait_oracle("value_mismatch"),
        "zero_timeout_result": model.wait_oracle("zero_timeout"),
        "timeout_notify_after": model.wait_oracle("timeout_cleanup")["notify_after_cleanup"],
        "cancellation_error_kind": "ResourceLimit",
        "cancellation_notify_after": model.wait_oracle("cancel_cleanup")["notify_after_cleanup"],
        "registration_observer_available": False,
    }
    for key, value in expected.items():
        if raw.get(key) != value:
            raise ThreadMeasurementError(f"wait cleanup field {key!r} was {raw.get(key)!r}, expected {value!r}")
    if raw.get("cancellation_message") not in {"execution cancelled", "execution cancelled during atomic wait"}:
        raise ThreadMeasurementError("incorrect cancellation error message")
    elapsed = raw.get("timeout_elapsed_ns")
    if type(elapsed) is not int or elapsed < timeout_ns:
        raise ThreadMeasurementError("finite wait did not remain blocked through the requested timeout")
    return {**raw, "timeout_elapsed_ns": elapsed}


def _source_paths(source_root: Path, bench_root: Path, driver: Path, driver_source: Path,
                  encoder: Path, schema: Path, model_path: Path, registry: Path,
                  fixtures: dict[str, tuple[Path, Path]], runner: Path) -> list[Path]:
    paths = [driver, encoder, driver_source, schema, model_path, registry, runner,
             source_root / "Cargo.toml", source_root / "Cargo.lock",
             source_root / "rust-toolchain.toml"]
    # Bind every manifest/lockfile in the relevant workspace and both Rust
    # implementation trees, including the encoder's own source and manifest.
    paths.extend(source_root.glob("**/Cargo.toml"))
    paths.extend(source_root.glob("**/Cargo.lock"))
    paths.extend((source_root / "crates").rglob("*.rs"))
    paths.extend((source_root / "tools" / "wtime-bench" / "src").rglob("*.rs"))
    paths.extend((source_root / "tools" / "wtime-conformance" / "src").rglob("*.rs"))
    paths.extend((source_root / ".cargo").rglob("*") if (source_root / ".cargo").exists() else [])
    for wat, wasm in fixtures.values():
        paths.extend((wat, wasm))
    return paths


def measure(*, driver: Path, driver_source: Path, encoder: Path, source_root: Path,
            model_path: Path, registry_path: Path, encoded_dir: Path, selection_name: str = "threads",
            trials: int = 21, latency_samples: int = 1024, timeout_ns: int = 1_000_000_000,
            process_timeout: float = 180.0, seed: int = 0x5013) -> dict[str, Any]:
    bench_root = model_path.parent
    schema_path = bench_root / "workloads.py"
    if trials != 21 or latency_samples != 1024:
        raise ThreadMeasurementError("P013 evidence requires exactly 21 trials and 1024 wait observations")
    if timeout_ns < 1_000_000 or timeout_ns > 100_000_000:
        raise ThreadMeasurementError("cleanup timeout must be in the driver's 1..100 ms range")
    for path in (driver, driver_source, encoder, source_root, model_path, registry_path, encoded_dir):
        if not path.exists():
            raise ThreadMeasurementError(f"required P013 input does not exist: {path}")
    model = load_model(model_path)
    entries = model.entries()
    selected = _schema_and_registry(schema_path, registry_path, selection_name, entries)
    fixtures, reencoded_hashes = validate_fixtures(selected, model_path, encoded_dir, encoder, model)
    paths = _source_paths(source_root, bench_root, driver, driver_source, encoder, schema_path,
                          model_path, registry_path, fixtures, Path(__file__))
    before = source_snapshot(paths)
    protocol = validate_protocol(run_object([str(driver), "--protocol"], process_timeout))
    cases = []
    for entry in selected:
        workload = entry["id"]
        wat, wasm = fixtures[workload]
        comparator = entry.get("comparators", {}).get("wasmtime", {})
        capability = {"status": comparator.get("status", "unsupported"),
                      "reason": comparator.get("reason", "no threads-capable Wasmtime probe is available"),
                      "timing": "not_measured"}
        common = {"workload": workload, "stage": "steady_state_group",
                  "registry_stages": entry.get("stages"),
                  "fixture": {"wat_sha256": sha256(wat), "wasm_sha256": sha256(wasm)},
                  "capability": capability}
        if workload == "threads_wait_notify_latency":
            raw = run_json([str(driver), "--wait-latency", str(wasm), "1024", str(timeout_ns)], process_timeout)
            validated = _validate_wait_latency(entry, raw, model, 1024, timeout_ns)
            cleanup_raw = run_json([str(driver), "--wait-cleanup", str(wasm), str(timeout_ns)], process_timeout)
            cleanup = _validate_wait_cleanup(cleanup_raw, timeout_ns, model)
            cases.append({**common, **validated, "raw_wait_latency": raw,
                          "wait_cleanup": cleanup, "raw_wait_cleanup": cleanup_raw})
        elif workload == "threads_sc_store_buffering":
            raw = run_json([str(driver), "--sc-litmus", str(wasm), "1024", str(seed)], process_timeout)
            cases.append({**common, **_validate_sc(entry, raw, model), "raw_result": raw})
        else:
            config = entry["thread_config"]
            raw = run_json([str(driver), "--thread-case", str(wasm), workload,
                            str(config["requested_workers"]), str(config["iterations_per_worker"]),
                            "21", str(seed)], process_timeout)
            cases.append({**common, **_validate_atomic(entry, raw, model), "raw_result": raw})
    after = source_snapshot(paths)
    assert_stable(before, after)
    return {
        "schema_version": 1, "task": "P013", "selection": selection_name,
        "passed": True, "source_stable": True,
        "metadata": {
            "driver_binary": str(driver.resolve()), "driver_binary_sha256": before[str(driver.resolve())],
            "encoder_binary": str(encoder.resolve()), "encoder_binary_sha256": before[str(encoder.resolve())],
            "driver_source_sha256": before[str(driver_source.resolve())],
            "workload_model": str(model_path.resolve()), "workload_model_sha256": before[str(model_path.resolve())],
            "registry_sha256": before[str(registry_path.resolve())],
            "schema_sha256": before[str(schema_path.resolve())],
            "reencoded_fixture_wasm_sha256": reencoded_hashes,
            "source_snapshot": before, "protocol_handshake": protocol,
            "seed": seed, "independent_trials_per_case": 21,
            "individual_wait_samples": 1024, "wait_timeout_ns": timeout_ns,
            "execution_mode": "decoded", "instrumented": False,
            "steady_state_group": {
                "timed_boundary": "start_barrier_to_last_worker_result_receive",
                "setup_readback_join_excluded": True,
                "trial_count": 21,
                "stage_label_note": "steady_state_group names the timed worker group; no cold load/compile timing is claimed",
            },
            "wait_timing": "guest notify call start through guest waiter return; notify invocation overhead included",
            "host": {
                "platform": platform.platform(), "python": platform.python_version(),
                "cpu_count": os.cpu_count(),
                "affinity": sorted(os.sched_getaffinity(0)) if hasattr(os, "sched_getaffinity") else None,
                "effective_worker_quota": "reported by runtime thread::available_parallelism per case",
            },
            "wasmtime": {"timing": "not_measured", "reason": "registered capability is unsupported without threads/shared memory"},
        },
        "cases": cases,
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--driver", type=Path, required=True)
    parser.add_argument("--driver-source", type=Path, required=True)
    parser.add_argument("--encoder", type=Path, required=True)
    parser.add_argument("--source-root", type=Path, required=True)
    parser.add_argument("--model", type=Path, required=True)
    parser.add_argument("--registry", type=Path, required=True)
    parser.add_argument("--encoded-dir", type=Path, required=True)
    parser.add_argument("--selection", default="threads")
    parser.add_argument("--trials", type=int, default=21)
    parser.add_argument("--latency-samples", type=int, default=1024)
    parser.add_argument("--timeout-ns", type=int, default=10_000_000)
    parser.add_argument("--process-timeout", type=float, default=180.0)
    parser.add_argument("--seed", type=int, default=0x5013)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    if args.output.exists():
        raise SystemExit("refusing to overwrite thread evidence")
    try:
        report = measure(driver=args.driver, driver_source=args.driver_source, encoder=args.encoder,
                         source_root=args.source_root, model_path=args.model,
                         registry_path=args.registry, encoded_dir=args.encoded_dir,
                         selection_name=args.selection, trials=args.trials,
                         latency_samples=args.latency_samples, timeout_ns=args.timeout_ns,
                         process_timeout=args.process_timeout, seed=args.seed)
        args.output.parent.mkdir(parents=True, exist_ok=True)
        temporary = args.output.with_suffix(args.output.suffix + ".tmp")
        temporary.write_text(json.dumps(report, indent=2, sort_keys=True, allow_nan=False) + "\n")
        temporary.replace(args.output)
    except (OSError, ThreadMeasurementError, TypeError, ValueError, KeyError) as exc:
        print(f"thread_scaling.py: {exc}", file=sys.stderr)
        return 2
    print(json.dumps({"output": str(args.output), "cases": len(report["cases"]),
                      "source_stable": report["source_stable"]}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
