"""Source-bound P128 fixture/oracle proof runner; it never builds binaries."""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import math
import os
from pathlib import Path
import shutil
import subprocess
import sys
from typing import Any, Callable

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
BENCH = ROOT / "tools/wtime-bench"
PRODUCER_REL = "tools/verification/producers/p128_scalar.py"
LOCK_REL = "docs/benchmarks/performance/P125-fixture-tools.json"
REGISTRY_REL = "tools/wtime-bench/workloads.json"
EXPECTED_REGISTRY_SHA256 = "8917f14d06b2dd69d1abb1885d8186cdc11191924aef8ee073e356cec5cfd668"
P125_LOCK_SHA256 = "cbbec91d1a4b7f4b6379ac11c2ad37059cfefaa27480beff958c0e3eaa80ff38"
MODES = ("decoded", "compact")
STAGES = ("prepare", "instantiate", "first_call", "warm_call", "load_and_call")
PROFILE = "mvp"
CORE_CRATES = ("wtime-runtime", "wtime-types", "wtime-decode", "wtime-validate")


class P128Error(RuntimeError):
    pass


def sha(path: Path) -> str:
    digest = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def _canonical_sha(value: Any) -> str:
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":"), allow_nan=False).encode()).hexdigest()


def _write_new(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("x", encoding="utf-8") as stream:
        json.dump(value, stream, indent=2, sort_keys=True, allow_nan=False)
        stream.write("\n")


def _contained(root: Path, path: Path) -> Path:
    root, path = root.resolve(), path.resolve()
    if path == root or root not in path.parents:
        raise P128Error(f"output path escapes or aliases proof root: {path}")
    return path


def _load_model(tool_dir: Path):
    path = tool_dir / "scalar_p128_workloads.py"
    spec = importlib.util.spec_from_file_location("wtime_p128_scalar_model", path)
    if spec is None or spec.loader is None:
        raise P128Error(f"cannot import P128 model module: {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def _source_maps(root: Path) -> tuple[dict[str, str], dict[str, str]]:
    """Return compiled focused sources and proof-only producer/tool sources."""
    bench = root / "tools/wtime-bench"
    if str(bench) not in sys.path:
        sys.path.insert(0, str(bench))
    path = bench / "focused_compare.py"
    spec = importlib.util.spec_from_file_location(f"p128_focused_compare_{id(root)}", path)
    if spec is None or spec.loader is None:
        raise P128Error(f"cannot load focused source identity helper: {path}")
    focused = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(focused)
    compiled = focused.sources()
    if not isinstance(compiled, dict) or not compiled:
        raise P128Error("focused source identity helper returned an empty map")
    proof = dict(compiled)
    for name in (PRODUCER_REL, LOCK_REL):
        source = root / name
        if not source.is_file():
            raise P128Error(f"missing P128 proof input: {name}")
        proof[name] = sha(source)
    return dict(sorted(compiled.items())), dict(sorted(proof.items()))


def _runtime_sources(compiled: dict[str, str]) -> dict[str, str]:
    result = {name: digest for name, digest in compiled.items()
              if name.startswith("crates/wtime-runtime/src/") and name.endswith(".rs")}
    if not result:
        raise P128Error("current focused source map omits runtime Rust sources")
    return dict(sorted(result.items()))


def _core_sources(compiled: dict[str, str]) -> dict[str, str]:
    result = {name: digest for name, digest in compiled.items()
              if any(name.startswith(f"crates/{crate}/src/") and name.endswith(".rs")
                     for crate in CORE_CRATES)}
    missing = [crate for crate in CORE_CRATES
               if not any(name.startswith(f"crates/{crate}/src/") for name in result)]
    if missing:
        raise P128Error(f"current source map omits core crate sources: {missing}")
    return dict(sorted(result.items()))


def _verify_record(root: Path, record_path: Path, mode: str, compiled: dict[str, str]) -> tuple[dict[str, Any], Path, str]:
    record_path = record_path.resolve(strict=True)
    if root.resolve() not in record_path.parents:
        raise P128Error(f"{mode} production record is outside the workspace")
    try:
        record_bytes = record_path.read_bytes()
        record = json.loads(record_bytes)
    except (OSError, json.JSONDecodeError) as exc:
        raise P128Error(f"cannot read {mode} runtime-only production record: {exc}") from exc
    expected_features = [] if mode == "decoded" else ["compact-executor"]
    expected_cargo = ["runtime-only"] + expected_features
    if (record.get("schema") != "wtime-runtime-only-production-v1" or
            record.get("phase") != "build" or record.get("mode") != mode or
            record.get("exit_code") != 0 or record.get("source_stable") is not True or
            record.get("instrumented") is not False or
            record.get("runtime_features") != expected_features or
            record.get("feature_flags") != expected_features or
            record.get("harness_features") != ["runtime-only"] or
            record.get("cargo_features") != expected_cargo):
        raise P128Error(f"{mode} record is not a successful uninstrumented runtime-only build")
    build_command = record.get("command")
    expected_feature_arg = ",".join(expected_cargo)
    if (not isinstance(build_command, list) or len(build_command) < 5 or
            build_command[0:2] != ["cargo", "build"] or "--locked" not in build_command or
            "--offline" not in build_command or "--no-default-features" not in build_command or
            "--features" not in build_command or
            build_command[build_command.index("--features") + 1] != expected_feature_arg):
        raise P128Error(f"{mode} record command does not prove the expected runtime-only features")
    if record.get("source_identities") != compiled:
        raise P128Error(f"{mode} build record does not match current compiled source identities")
    runtime = _runtime_sources(compiled)
    core = _core_sources(compiled)
    if record.get("runtime_source_map") != runtime or record.get("core_source_map") != core:
        raise P128Error(f"{mode} build record runtime/core source map is stale")
    binary = Path(record.get("binary_path", "")).resolve(strict=True)
    if root.resolve() not in binary.parents:
        raise P128Error(f"{mode} production binary is outside the workspace")
    if not binary.is_file() or sha(binary) != record.get("binary_sha256"):
        raise P128Error(f"{mode} production binary does not match its record")
    protocol = record.get("protocol")
    if (not isinstance(protocol, dict) or protocol.get("schema_version") != 2 or
            protocol.get("engine") != "wtime" or protocol.get("instrumented") is not False or
            mode not in protocol.get("modes", []) or PROFILE not in protocol.get("features", [])):
        raise P128Error(f"{mode} build record lacks the requested uninstrumented MVP protocol")
    if record.get("protocol_exit") != 0 or not isinstance(record.get("protocol_sha256"), str):
        raise P128Error(f"{mode} build record lacks a successful protocol receipt")
    if record.get("protocol_command") != [str(binary), "--protocol"]:
        raise P128Error(f"{mode} build record protocol command is not bound to its binary")
    protocol_stdout = Path(record.get("protocol_stdout", "")).resolve(strict=True)
    if root.resolve() not in protocol_stdout.parents:
        raise P128Error(f"{mode} protocol response is outside the workspace")
    if sha(protocol_stdout) != record["protocol_sha256"]:
        raise P128Error(f"{mode} build record protocol bytes are stale")
    return record, binary, hashlib.sha256(record_bytes).hexdigest()


def _archive_sources(root: Path, output: Path, sources: dict[str, str]) -> None:
    for name, digest in sorted(sources.items()):
        source = root / name
        if not source.is_file() or sha(source) != digest:
            raise P128Error(f"source changed before archival: {name}")
        target = _contained(output, output / "executed/sources" / name)
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, target)
        if sha(target) != digest:
            raise P128Error(f"archived source hash mismatch: {name}")


def _run(command: list[str], cwd: Path, timeout: int = 180) -> subprocess.CompletedProcess[str]:
    try:
        return subprocess.run(command, cwd=cwd, text=True, capture_output=True,
                              timeout=timeout, check=False)
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise P128Error(f"command failed to start or timed out: {command!r}: {exc}") from exc


def _command(output: Path, label: str, command: list[str], cwd: Path,
             commands: list[dict[str, Any]], runner: Callable, timeout: int = 180
             ) -> subprocess.CompletedProcess[str]:
    argv_path = _contained(output, output / "commands" / f"{label}.argv.json")
    _write_new(argv_path, {"argv": command, "cwd": str(cwd)})
    result = runner(command, cwd, timeout)
    stdout_path = _contained(output, output / "commands" / f"{label}.stdout")
    stderr_path = _contained(output, output / "commands" / f"{label}.stderr")
    stdout_path.write_text(result.stdout or "", encoding="utf-8")
    stderr_path.write_text(result.stderr or "", encoding="utf-8")
    status_path = _contained(output, output / "commands" / f"{label}.status.json")
    _write_new(status_path, {"exit_code": result.returncode})
    commands.append({"label": label, "argv": command, "exit_code": result.returncode,
                     "stdout_sha256": sha(stdout_path), "stderr_sha256": sha(stderr_path),
                     "argv_path": argv_path.relative_to(output).as_posix(),
                     "stdout_path": stdout_path.relative_to(output).as_posix(),
                     "stderr_path": stderr_path.relative_to(output).as_posix(),
                     "status_path": status_path.relative_to(output).as_posix()})
    if result.returncode:
        raise P128Error(f"command failed ({result.returncode}): {label}: {(result.stderr or '')[-2000:]}")
    return result


def _tool(lock: dict[str, Any], name: str, root: Path | None = None) -> tuple[Path, str]:
    item = lock.get("tools", {}).get(name, {})
    path = Path(item.get("path", "")).resolve(strict=True)
    digest = item.get("sha256")
    if root is not None and root.resolve() not in path.parents:
        raise P128Error(f"pinned P125 {name} tool is outside the workspace")
    if not isinstance(digest, str) or sha(path) != digest:
        raise P128Error(f"pinned P125 {name} tool hash mismatch")
    return path, digest


def _load_lock(root: Path) -> tuple[dict[str, Any], Path]:
    path = root / LOCK_REL
    if not path.is_file() or sha(path) != P125_LOCK_SHA256:
        raise P128Error("P125 fixture-tools lock is missing or changed")
    lock = json.loads(path.read_text(encoding="utf-8"))
    receipt = lock.get("source_receipt", {})
    receipt_path = Path(receipt.get("path", "")).resolve(strict=True)
    if root.resolve() not in receipt_path.parents:
        raise P128Error("P125 fixture-tools receipt is outside the workspace")
    if not isinstance(receipt.get("sha256"), str) or sha(receipt_path) != receipt["sha256"]:
        raise P128Error("P125 fixture-tools source receipt is stale")
    for name in ("encoder", "validator"):
        _tool(lock, name, root)
    return lock, path


def _fixtures(output: Path, tool_dir: Path, model: Any, lock: dict[str, Any],
              commands: list[dict[str, Any]], runner: Callable) -> dict[str, dict[str, str]]:
    encoder, encoder_hash = _tool(lock, "encoder", tool_dir.parents[1])
    validator, validator_hash = _tool(lock, "validator", tool_dir.parents[1])
    result: dict[str, dict[str, str]] = {}
    for index, (family, metadata) in enumerate(model.FIXTURES.items()):
        wat_name = metadata["file"]
        source = tool_dir / "fixtures/scalar" / wat_name
        if not source.is_file() or sha(source) != metadata["wat_sha256"]:
            raise P128Error(f"admitted WAT changed or missing: {wat_name}")
        staged_wat = _contained(output, output / "fixtures/scalar" / wat_name)
        staged_wat.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, staged_wat)
        if sha(staged_wat) != metadata["wat_sha256"]:
            raise P128Error(f"WAT changed while staging: {wat_name}")
        wasm = staged_wat.with_suffix(".wasm")
        _command(output, f"encode-{index}", [str(encoder), "encode", str(staged_wat), "-o", str(wasm)],
                 output, commands, runner)
        if not wasm.is_file() or sha(wasm) != metadata["wasm_sha256"]:
            raise P128Error(f"admitted Wasm hash mismatch after encoding: {wat_name}")
        _command(output, f"validate-{index}", [str(validator), "--profile", PROFILE, "check", str(wasm)],
                 output, commands, runner)
        result[family] = {
            "wat_sha256": sha(staged_wat), "wasm_sha256": sha(wasm),
            "wat_path": staged_wat.relative_to(output).as_posix(),
            "wasm_path": wasm.relative_to(output).as_posix(),
            "encoder_sha256": encoder_hash, "validator_sha256": validator_hash,
            "validation_profile": PROFILE,
        }
    if len(result) != 3:
        raise P128Error(f"expected exactly 3 admitted P128 fixtures, got {len(result)}")
    return result


def _protocol(binary: Path, mode: str, output: Path, commands: list[dict[str, Any]],
              runner: Callable, cwd: Path) -> dict[str, Any]:
    response = _command(output, f"protocol-{mode}", [str(binary), "--protocol"],
                        cwd, commands, runner, timeout=30)
    raw_path = _contained(output, output / "protocol" / f"{mode}.stdout")
    raw_path.parent.mkdir(parents=True, exist_ok=True)
    raw_path.write_text(response.stdout or "", encoding="utf-8")
    try:
        data = json.loads(response.stdout)
    except json.JSONDecodeError as exc:
        raise P128Error(f"{mode} protocol returned invalid JSON") from exc
    response_path = _contained(output, output / "protocol" / f"{mode}.response.json")
    _write_new(response_path, data)
    if (not isinstance(data, dict) or data.get("schema_version") != 2 or
            data.get("engine") != "wtime" or data.get("instrumented") is not False or
            mode not in data.get("modes", []) or PROFILE not in data.get("features", [])):
        raise P128Error(f"{mode} binary protocol mode/instrumentation/profile mismatch")
    return data


def _validate_observation(data: dict[str, Any], row: dict[str, Any], mode: str,
                          stage: str, batch: int, expected: int) -> dict[str, Any]:
    n = row["oracle_arguments"]["n"]
    wanted = {
        "schema_version": 2, "case": row["id"], "stage": stage,
        "input": f"i32:{n}", "expected": f"i32:{expected}", "result": f"i32:{expected}",
        "mode": mode, "profile": PROFILE, "driver": "plain", "diagnostic": False,
    }
    if any(data.get(key) != value for key, value in wanted.items()):
        raise P128Error(f"observation identity/oracle mismatch for {row['id']} {mode}/{stage}: {data!r}")
    actual_batch = data.get("batch")
    if type(actual_batch) is not int or actual_batch != batch:
        raise P128Error(f"batch mismatch for {row['id']} {mode}/{stage}: {actual_batch!r} != {batch}")
    elapsed = data.get("ns_per_operation")
    if isinstance(elapsed, bool) or not isinstance(elapsed, (int, float)) or not math.isfinite(elapsed) or elapsed <= 0:
        raise P128Error(f"missing or invalid driver response duration for {row['id']} {mode}/{stage}")
    return {"workload": row["id"], "input": n, "expected": expected,
            "result": expected, "mode": mode, "stage": stage, "batch": batch,
            "timing_value_used": False}


def _raw_observation(binary: Path, fixture: Path, row: dict[str, Any], mode: str,
                     stage: str, batch: int, expected: int, output: Path,
                     label: str, commands: list[dict[str, Any]], runner: Callable,
                     cwd: Path) -> tuple[list[str], dict[str, Any], dict[str, str]]:
    n = row["oracle_arguments"]["n"]
    command = [str(binary), "--scalar-driver", str(fixture), row["id"], str(n),
               str(expected), stage, str(batch), mode, PROFILE, "plain"]
    completed = _command(output, label, command, cwd, commands, runner)
    raw_stdout = _contained(output, output / "observations/raw" / f"{label}.stdout")
    raw_stdout.parent.mkdir(parents=True, exist_ok=True)
    raw_stdout.write_text(completed.stdout or "", encoding="utf-8")
    try:
        data = json.loads(completed.stdout)
    except json.JSONDecodeError as exc:
        raise P128Error(f"{row['id']} {mode}/{stage} returned invalid JSON; stdout is retained") from exc
    if not isinstance(data, dict):
        raw_json = _contained(output, output / "observations/raw" / f"{label}.json")
        _write_new(raw_json, data)
        raise P128Error(f"{row['id']} {mode}/{stage} response must be a JSON object")
    raw_json = _contained(output, output / "observations/raw" / f"{label}.json")
    _write_new(raw_json, data)
    return command, data, {
        "stdout_path": raw_stdout.relative_to(output).as_posix(),
        "stdout_sha256": sha(raw_stdout),
        "json_path": raw_json.relative_to(output).as_posix(),
        "json_sha256": sha(raw_json),
    }


def _load_registry(root: Path) -> tuple[dict[str, Any], str, dict[str, str]]:
    path = root / REGISTRY_REL
    raw_sha = sha(path)
    if raw_sha != EXPECTED_REGISTRY_SHA256:
        raise P128Error(f"shared registry identity changed: {raw_sha}")
    registry = json.loads(path.read_text(encoding="utf-8"))
    rows = registry.get("workloads")
    selectors = registry.get("selections")
    if not isinstance(rows, list) or len(rows) != 459 or not isinstance(selectors, dict):
        raise P128Error("shared registry must retain its exact 459-row baseline shape")
    if any(row.get("id", "").startswith("p128_") for row in rows):
        raise P128Error("P128 task aliases unexpectedly entered the shared registry")
    selector_hashes = {name: _canonical_sha(value) for name, value in sorted(selectors.items())}
    return registry, raw_sha, selector_hashes


def run(decoded_record_path: Path, compact_record_path: Path, output: Path, *,
        root: Path = ROOT, source_reader: Callable | None = None,
        command_runner: Callable | None = None) -> dict[str, Any]:
    root = Path(root).resolve(strict=True)
    output = Path(output).resolve()
    if root != output and root not in output.parents:
        raise P128Error(f"proof output must be inside workspace root: {output}")
    output.mkdir(parents=True, exist_ok=False)
    tool_dir = root / "tools/wtime-bench"
    model = _load_model(tool_dir)
    rows = model.entries()
    wanted_ids = {f"{family}_n{n}" for family in model.FIXTURES for n in model.FIXED_INPUTS}
    if len(rows) != 18 or {row["id"] for row in rows} != wanted_ids:
        raise P128Error("P128 model must provide exactly the 18 admitted aliases")
    if tuple(model.STAGES) != STAGES or tuple(model.MODES) != MODES or model.PROFILE != PROFILE:
        raise P128Error("P128 model stage, mode, or MVP profile declaration changed")

    source_reader = source_reader or _source_maps
    command_runner = command_runner or _run
    compiled_before, source_before = source_reader(root)
    registry, registry_sha_before, selection_hashes = _load_registry(root)
    lock, lock_path = _load_lock(root)
    decoded_record_path = Path(decoded_record_path).resolve(strict=True)
    compact_record_path = Path(compact_record_path).resolve(strict=True)
    decoded_record = _verify_record(root, decoded_record_path, "decoded", compiled_before)
    compact_record = _verify_record(root, compact_record_path, "compact", compiled_before)
    records = {"decoded": decoded_record, "compact": compact_record}
    binary_paths = {mode: item[1] for mode, item in records.items()}
    binary_sha_before = {mode: sha(path) for mode, path in binary_paths.items()}
    if binary_paths["decoded"] == binary_paths["compact"]:
        raise P128Error("decoded and compact runtime-only binaries must be distinct")

    # Bind all inputs before running protocol, encoder, validator, or guest commands.
    _archive_sources(root, output, source_before)
    _write_new(output / "executed/registry.before.json", registry)
    shutil.copyfile(lock_path, output / "executed/P125-fixture-tools.json")
    if sha(output / "executed/P125-fixture-tools.json") != P125_LOCK_SHA256:
        raise P128Error("P125 fixture-tools lock changed while archiving")
    record_receipts = {}
    for mode, (record, binary, record_sha) in records.items():
        archive = _contained(output, output / "executed/production-records" / f"{mode}.json")
        archive.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(decoded_record_path if mode == "decoded" else compact_record_path, archive)
        if sha(archive) != record_sha:
            raise P128Error(f"{mode} production record changed while archiving")
        record_receipts[mode] = {
            "path": str(decoded_record_path if mode == "decoded" else compact_record_path),
            "sha256": record_sha,
            "archived_path": archive.relative_to(output).as_posix(),
            "binary_path": str(binary), "binary_sha256": binary_sha_before[mode],
            "runtime_source_sha256": _canonical_sha(record["runtime_source_map"]),
        }
    _write_new(output / "executed/production-records/index.json", record_receipts)
    _write_new(output / "executed/binaries.before.json", record_receipts)
    _write_new(output / "executed/producer.argv.json", {"argv": sys.argv, "cwd": str(Path.cwd())})
    _write_new(output / "executed/model-rows.json", {
        "rows_sha256": _canonical_sha(rows), "rows": rows,
        "model_source_sha256": source_before.get("tools/wtime-bench/scalar_p128_workloads.py"),
        "runner_source_sha256": source_before.get("tools/wtime-bench/p128_scalar_scaling.py"),
    })
    commands: list[dict[str, Any]] = []
    protocols = {mode: _protocol(binary_paths[mode], mode, output, commands, command_runner, root)
                 for mode in MODES}
    if any(protocols[mode] != records[mode][0].get("protocol") for mode in MODES):
        raise P128Error("runtime-only production protocol differs from the current binary response")
    fixture_rows = _fixtures(output, tool_dir, model, lock, commands, command_runner)

    if source_reader(root) != (compiled_before, source_before):
        raise P128Error("current source identity changed before P128 observations")
    if {mode: sha(path) for mode, path in binary_paths.items()} != binary_sha_before:
        raise P128Error("a production binary changed before P128 observations")

    observations: list[dict[str, Any]] = []
    for row in rows:
        n = row["oracle_arguments"]["n"]
        expected = model.oracle(row["id"], n)
        if expected != row["expected_i32"] or model.source_event_count(n) != row["source_event_model"]["dynamic"]:
            raise P128Error(f"independent model row disagrees before execution: {row['id']}")
        fixture_name = Path(row["source"]["path"]).name
        fixture = output / fixture_rows[Path(fixture_name).stem]["wasm_path"]
        for mode in MODES:
            for stage in STAGES:
                batch = 3 if stage == "warm_call" else 1
                label = f"{mode}.{row['id']}.{stage}"
                command, raw, raw_refs = _raw_observation(
                    binary_paths[mode], fixture, row, mode, stage, batch, expected,
                    output, label, commands, command_runner, root)
                proof = _validate_observation(raw, row, mode, stage, batch, expected)
                proof.update({
                    "argv": command,
                    "argv_path": f"commands/{label}.argv.json",
                    "raw_stdout_path": raw_refs["stdout_path"],
                    "raw_stdout_sha256": raw_refs["stdout_sha256"],
                    "raw_json_path": raw_refs["json_path"],
                    "raw_json_sha256": raw_refs["json_sha256"],
                })
                observations.append(proof)
    compiled_after, source_after = source_reader(root)
    registry_after = sha(root / REGISTRY_REL)
    lock_after = sha(lock_path)
    record_sha_after = {
        "decoded": sha(decoded_record_path), "compact": sha(compact_record_path)}
    binary_sha_after = {mode: sha(path) for mode, path in binary_paths.items()}
    if compiled_after != compiled_before or source_after != source_before:
        raise P128Error("source identity changed during P128 proof")
    if registry_after != registry_sha_before or lock_after != P125_LOCK_SHA256:
        raise P128Error("registry or fixture-tool lock changed during P128 proof")
    if any(record_sha_after[mode] != records[mode][2] for mode in MODES):
        raise P128Error("a supplied production build record changed during P128 proof")
    if binary_sha_after != binary_sha_before:
        raise P128Error("a supplied runtime-only binary changed during P128 proof")

    expected_pairs = {(row["id"], mode, stage) for row in rows for mode in MODES for stage in STAGES}
    actual_pairs = {(row["workload"], row["mode"], row["stage"]) for row in observations}
    if len(observations) != 180 or actual_pairs != expected_pairs or len(actual_pairs) != 180:
        raise P128Error(f"expected exactly 180 distinct observations, got {len(observations)}")
    observation_commands = [item for item in commands if item["label"].split(".", 1)[0] in MODES and
                            item["label"].count(".") == 2]
    if len(observation_commands) != 180 or any(item["exit_code"] != 0 for item in commands):
        raise P128Error("all 180 single-observation commands and preparation commands must exit zero")
    by_mode = {mode: sum(item["mode"] == mode for item in observations) for mode in MODES}
    by_stage = {stage: sum(item["stage"] == stage for item in observations) for stage in STAGES}
    if by_mode != {"decoded": 90, "compact": 90} or by_stage != {stage: 36 for stage in STAGES}:
        raise P128Error("P128 observation matrix is incomplete")

    report = {
        "schema": "wtime-p128-scalar-foundation-v1", "status": "passed",
        "performance_candidate": False, "speed_decision": None,
        "source_stable": True, "instrumented": False,
        "timing_analysis": False,
        "note": "Existing stage-driver latency fields are retained in raw stdout only; this proof does not analyze or compare timing.",
        "source_hashes_before": source_before, "source_hashes_after": source_after,
        "registry": {"path": REGISTRY_REL, "sha256": registry_sha_before,
                     "rows": 459, "selection_sha256": selection_hashes,
                     "unchanged": registry_after == registry_sha_before},
        "fixture_tools_lock_sha256": P125_LOCK_SHA256,
        "fixtures": fixture_rows,
        "binaries": record_receipts,
        "settings": {"profile": PROFILE, "modes": list(MODES), "stages": list(STAGES),
                     "instrumented": False, "warm_batch": 3,
                     "runtime_only": True, "single_command_per_row_mode_stage": True},
        "counts": {"aliases": len(rows), "fixtures": len(fixture_rows),
                    "observations": len(observations), "observation_commands": len(observation_commands),
                    "preparation_commands": len(commands) - len(observation_commands),
                    "commands_total": len(commands), "bad_exits": 0, "skipped": 0,
                    "by_mode": by_mode, "by_stage": by_stage},
        "commands": commands, "observations": observations,
    }
    _write_new(output / "report.json", report)
    return report
