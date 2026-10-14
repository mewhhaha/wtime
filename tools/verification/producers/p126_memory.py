"""Run the P126 memory-alias foundation proof against two supplied binaries.

This producer never builds or promotes a candidate. It admits only the 18
fixed aliases, encodes the three pinned fixtures, checks both protocol modes,
and retains every actual driver response before proceeding.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
from typing import Any

ROOT = Path(__file__).resolve().parents[3]
BENCH = ROOT / "tools/wtime-bench"
sys.path.insert(0, str(BENCH))
import memory_scaling
import memory_workloads

COUNTS = (0, 1, 17, 32, 256, 1024)
FAMILIES = ("predictable", "shuffled", "read_scan")
STAGES = ("prepare", "instantiate", "first_call", "warm_call", "load_and_call")
MODES = ("decoded", "compact")
PROFILE = "all-phase3-core"
LOCK = ROOT / "docs/benchmarks/performance/P125-fixture-tools.json"


class P126Error(RuntimeError):
    pass


def sha(path: Path) -> str:
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def write(path: Path, value: Any) -> None:
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("x", encoding="utf-8") as stream:
        json.dump(value, stream, indent=2, sort_keys=True, allow_nan=False)
        stream.write("\n")


def _contained(root: Path, path: Path) -> Path:
    root = root.resolve()
    path = path.resolve()
    if path == root or root not in path.parents:
        raise P126Error(f"output path escapes or aliases output root: {path}")
    return path


def _expected_ids() -> set[str]:
    return {f"memory_widths_{family}_{count}" for family in FAMILIES for count in COUNTS}


def _load_rows() -> tuple[Any, list[dict[str, Any]]]:
    registry = memory_workloads
    rows = registry.entries()
    wanted = _expected_ids()
    selected = [row for row in rows if row.get("id") in wanted]
    actual = {row.get("id") for row in selected}
    if actual != wanted or len(selected) != 18:
        raise P126Error(f"memory model aliases missing/duplicated: missing={sorted(wanted-actual)}")
    by_id = {row["id"]: row for row in selected}
    for family in FAMILIES:
        for count in COUNTS:
            wid = f"memory_widths_{family}_{count}"
            row = by_id[wid]
            param = row.get("params", [{}])[0]
            if (param.get("baseline") != count or param.get("min") != count or
                    param.get("max") != count or row.get("stages") != list(STAGES)):
                raise P126Error(f"{wid}: alias value or five-stage declaration differs")
            if row.get("source", {}).get("path") not in {
                    "fixtures/memory/widths.wat",
                    "fixtures/memory/widths_shuffled.wat",
                    "fixtures/memory/read_scan.wat"}:
                raise P126Error(f"{wid}: unexpected fixture source")
    manifest = json.loads((BENCH / "workloads.json").read_text(encoding="utf-8"))
    listed = {row.get("id"): row for row in manifest.get("workloads", [])}
    if not wanted.issubset(listed):
        raise P126Error(f"workload manifest lacks P126 aliases: {sorted(wanted-set(listed))}")
    for wid in wanted:
        if listed[wid] != by_id[wid]:
            raise P126Error(f"workload manifest and independent model disagree for {wid}")
    selection = manifest.get("selections", {}).get("memory", {})
    selection_ids = selection.get("workloads", [])
    if (selection.get("status") != "ready" or len(selection_ids) != len(set(selection_ids)) or
            not wanted.issubset(selection_ids)):
        raise P126Error("ready memory selection must include each P126 alias exactly once")
    return registry, [by_id[wid] for wid in sorted(wanted)]


def _source_map() -> dict[str, str]:
    root = memory_scaling._repo_root(BENCH)
    records = memory_scaling.source_hashes(root, BENCH)
    records[Path(__file__).resolve().relative_to(root).as_posix()] = sha(Path(__file__).resolve())
    records["docs/benchmarks/performance/P125-fixture-tools.json"] = sha(LOCK)
    records["tools/wtime-bench/p027_memory_workloads.py"] = sha(BENCH / "p027_memory_workloads.py")
    return records


def _require_stable(before: dict[str, str], after: dict[str, str]) -> None:
    if before != after:
        changed = sorted(key for key in before.keys() | after.keys()
                         if before.get(key) != after.get(key))
        raise P126Error(f"source identity changed during P126 proof: {changed[:8]}")


def _archive_sources(output: Path, records: dict[str, str]) -> None:
    for name, digest in sorted(records.items()):
        source = ROOT / name
        if not source.is_file() or sha(source) != digest:
            raise P126Error(f"source changed before archival: {name}")
        target = _contained(output, output / "executed/sources" / name)
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, target)
        if sha(target) != digest:
            raise P126Error(f"archived source digest mismatch: {name}")


def _run(command: list[str], cwd: Path, timeout: int = 120) -> subprocess.CompletedProcess[str]:
    try:
        return subprocess.run(command, cwd=cwd, text=True, capture_output=True,
                              timeout=timeout, check=False)
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise P126Error(f"command could not complete: {command!r}: {exc}") from exc


def _command(output: Path, label: str, command: list[str], cwd: Path,
             timeout: int = 120) -> subprocess.CompletedProcess[str]:
    argv_path = _contained(output, output / "commands" / f"{label}.argv.json")
    write(argv_path, {"argv": command, "cwd": str(cwd)})
    result = _run(command, cwd, timeout)
    stdout_path = _contained(output, output / "commands" / f"{label}.stdout")
    stderr_path = _contained(output, output / "commands" / f"{label}.stderr")
    stdout_path.write_text(result.stdout, encoding="utf-8")
    stderr_path.write_text(result.stderr, encoding="utf-8")
    status_path = _contained(output, output / "commands" / f"{label}.status.json")
    write(status_path, {"exit_code": result.returncode})
    if result.returncode:
        raise P126Error(f"command failed ({result.returncode}): {label}: {result.stderr[-2000:]}")
    return result


def _tool(lock: dict[str, Any], name: str) -> tuple[Path, str]:
    spec = lock.get("tools", {}).get(name, {})
    path = Path(spec.get("path", "")).resolve(strict=True)
    digest = spec.get("sha256")
    if not isinstance(digest, str) or sha(path) != digest:
        raise P126Error(f"pinned fixture {name} identity differs")
    return path, digest


def _fixtures(output: Path, registry: Any, lock: dict[str, Any]) -> dict[str, dict[str, str]]:
    encoder, _ = _tool(lock, "encoder")
    validator, _ = _tool(lock, "validator")
    records: dict[str, dict[str, str]] = {}
    filenames = ("widths.wat", "widths_shuffled.wat", "read_scan.wat")
    for index, filename in enumerate(filenames):
        source = BENCH / "fixtures/memory" / filename
        expected = registry.WASM_SHA256.get(filename)
        if not source.is_file() or not isinstance(expected, str):
            raise P126Error(f"missing admitted P126 fixture/model hash: {filename}")
        dest = output / "fixtures" / filename
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, dest)
        if sha(source) != sha(dest):
            raise P126Error(f"fixture source changed while copying: {filename}")
        wasm = dest.with_suffix(".wasm")
        _command(output, f"encode-{index}", [str(encoder), "encode", str(dest), "-o", str(wasm)], ROOT)
        if not wasm.is_file() or sha(wasm) != expected:
            raise P126Error(f"encoded fixture digest mismatch for {filename}")
        _command(output, f"validate-{index}", [str(validator), "--profile", PROFILE,
                                               "check", str(wasm)], ROOT)
        records[filename] = {"wat_sha256": sha(source), "wasm_sha256": sha(wasm),
                             "wat_path": dest.relative_to(output).as_posix(),
                             "wasm_path": wasm.relative_to(output).as_posix(),
                             "validator_profile": PROFILE}
    return records


def _protocol(binary: Path, mode: str, output: Path) -> dict[str, Any]:
    result = _command(output, f"protocol-{mode}", [str(binary), "--protocol"], ROOT, 30)
    try:
        data = json.loads(result.stdout)
    except json.JSONDecodeError as exc:
        raise P126Error(f"{mode} --protocol returned invalid JSON") from exc
    if (not isinstance(data, dict) or data.get("schema_version") != 2 or
            data.get("instrumented") is not False or mode not in data.get("modes", []) or
            PROFILE not in data.get("features", [])):
        raise P126Error(f"{mode} binary protocol mode/instrumentation/profile mismatch")
    return data


def _raw_memory(executable: Path, fixture: Path, entry: dict[str, Any], input_value: int,
                expected_outcome: str, mode: str, profile: str, driver: str,
                stage: str, batch: int, output: Path, label: str
                ) -> tuple[list[str], dict[str, Any]]:
    command = [str(executable), "--memory-staged", str(fixture), entry["id"],
               f"i32:{input_value}", expected_outcome, stage, str(batch), mode, profile, driver]
    result = _command(output, label, command, ROOT, 180)
    try:
        data = json.loads(result.stdout)
    except json.JSONDecodeError as exc:
        raw_path = _contained(output, output / "observations" / "raw" / f"{label}.stdout")
        raw_path.parent.mkdir(parents=True, exist_ok=True)
        raw_path.write_text(result.stdout, encoding="utf-8")
        raise P126Error(f"memory response invalid JSON: {entry['id']}/{mode}/{stage}") from exc
    if not isinstance(data, dict):
        raw_path = _contained(output, output / "observations" / "raw" / f"{label}.json")
        raw_path.parent.mkdir(parents=True, exist_ok=True)
        write(raw_path, data)
        raise P126Error(f"memory response must be an object: {entry['id']}/{mode}/{stage}")
    raw_path = _contained(output, output / "observations" / "raw" / f"{label}.json")
    raw_path.parent.mkdir(parents=True, exist_ok=True)
    write(raw_path, data)
    return command, data


def run(decoded: Path, compact: Path, output: Path) -> dict[str, Any]:
    output = Path(output).resolve()
    output.mkdir(parents=True, exist_ok=False)
    decoded = Path(decoded).resolve(strict=True)
    compact = Path(compact).resolve(strict=True)
    if decoded == compact or not decoded.is_file() or not compact.is_file():
        raise P126Error("decoded and compact must be distinct existing files")
    source_before = _source_map()
    registry, entries = _load_rows()
    model_sha = hashlib.sha256(json.dumps(entries, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
    lock = json.loads(LOCK.read_text(encoding="utf-8"))
    binaries = {"decoded": decoded, "compact": compact}
    binary_before = {mode: sha(path) for mode, path in binaries.items()}
    _archive_sources(output, source_before)
    shutil.copyfile(LOCK, output / "executed/fixture-tools.lock.json")
    write(output / "executed/producer.argv.json", {"argv": sys.argv, "cwd": str(Path.cwd())})
    write(output / "executed/model-rows.json", {"sha256": model_sha, "rows": entries})
    write(output / "executed/binaries-before.json", {
        mode: {"path": str(path), "sha256": binary_before[mode]} for mode, path in binaries.items()})
    protocols: dict[str, Any] = {}
    for mode, path in binaries.items():
        protocols[mode] = _protocol(path, mode, output)
        write(output / "protocol" / f"{mode}.response.json", protocols[mode])
    fixture_rows = _fixtures(output, registry, lock)
    _require_stable(source_before, _source_map())
    if {mode: sha(path) for mode, path in binaries.items()} != binary_before:
        raise P126Error("a supplied binary changed before guest observations")
    observations: list[dict[str, Any]] = []
    for entry in entries:
        wid = entry["id"]
        input_value = entry["params"][0]["baseline"]
        expected_memory = registry.expected_memory(wid, input_value)
        outcome = memory_scaling._expected_outcome(expected_memory)
        if registry.oracle(wid, input_value) != expected_memory.get("result", "trap"):
            raise P126Error(f"independent scalar/memory model disagreement for {wid}")
        profile, driver = memory_scaling._profile(entry), memory_scaling._driver(entry)
        fixture_name = Path(entry["source"]["path"]).name
        fixture = output / fixture_rows[fixture_name]["wasm_path"]
        for mode, binary in binaries.items():
            for stage in STAGES:
                batch = 3 if stage == "warm_call" else 1
                label = f"{mode}.{wid}.{stage}"
                command, data = _raw_memory(binary, fixture, entry, input_value, outcome,
                                            mode, profile, driver, stage, batch,
                                            output, label)
                proof = memory_scaling._validate_observation(
                    data, entry, input_value, expected_memory, mode, profile, driver,
                    stage=stage, expected_batch=batch)
                record = {"workload": wid, "input": input_value, "mode": mode,
                          "stage": stage, "batch": batch, "argv": command,
                          "raw": data, "validated": proof}
                write(output / "observations" / mode / f"{wid}.{stage}.json", record)
                observations.append(record)
    source_after = _source_map()
    binary_after = {mode: sha(path) for mode, path in binaries.items()}
    _require_stable(source_before, source_after)
    if binary_after != binary_before:
        raise P126Error("a supplied binary changed during P126 proof")
    if len(observations) != 180:
        raise P126Error(f"expected 180 real observations, got {len(observations)}")
    logical_calls = sum(3 if row["stage"] == "warm_call" else 1 for row in observations)
    report = {"schema": "wtime-p126-memory-foundation-v1", "status": "passed",
              "performance_candidate": False, "speed_decision": None,
              "source_stable": True,
              "source_hashes": source_before, "model_rows_sha256": model_sha,
              "fixture_tools": lock, "fixtures": fixture_rows,
              "binaries": {mode: {"path": str(path), "sha256": binary_before[mode],
                                   "protocol": protocols[mode]} for mode, path in binaries.items()},
              "settings": {"profile": PROFILE, "instrumented": False,
                           "modes": list(MODES), "stages": list(STAGES),
                           "warm_batch": 3, "fresh_instance_policy": "driver-declared"},
              "counts": {"aliases": len(entries), "fixtures": len(fixture_rows),
                         "observations": len(observations),
                         "driver_commands": len(observations),
                         "logical_guest_invocations": logical_calls,
                         "by_mode": {m: sum(o["mode"] == m for o in observations) for m in MODES},
                         "by_stage": {s: sum(o["stage"] == s for o in observations) for s in STAGES}},
              "observations": observations}
    write(output / "report.json", report)
    return report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--decoded", type=Path, required=True)
    parser.add_argument("--compact", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        run(args.decoded, args.compact, args.output)
    except (P126Error, OSError, ValueError, KeyError) as exc:
        print(json.dumps({"status": "failed", "error": str(exc)}), file=sys.stderr)
        return 1
    print(json.dumps({"status": "passed", "observations": 180}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
