#!/usr/bin/env python3
"""Collect correctness and compact-memory path counters for P043 workloads."""
from __future__ import annotations
import argparse
import hashlib
import importlib.util
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

sys.dont_write_bytecode = True
PRIMARY = {"memory_widths_predictable", "memory_widths_shuffled"}
SELECTION = "memory"
SCHEMA = "wtime-p043-compact-memory-diagnostic-v1"
FEATURES = "compact-executor,execution-reference,performance-counters"
COUNTERS = ("compact_memory_accesses", "compact_shared_memory_accesses", "compact_steps",
            "compact_fallback_frames", "read_fixed_operations", "read_vec_operations")
ACTIVE_JOURNAL: list[dict] = []

class GateError(RuntimeError):
    pass

def need(ok: bool, message: str) -> None:
    if not ok:
        raise GateError(message)

def sha(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()

def canonical_sha(value: object) -> str:
    raw = json.dumps(value, sort_keys=True, separators=(",", ":")).encode()
    return hashlib.sha256(raw).hexdigest()

def sections(path: Path) -> dict[int, bytes]:
    data = path.read_bytes()
    need(data[:8] == b'\x00asm\x01\x00\x00\x00', 'invalid diagnostic Wasm header')
    result = {}
    offset = 8
    while offset < len(data):
        kind = data[offset]
        offset += 1
        length = shift = 0
        for _ in range(5):
            need(offset < len(data), 'truncated diagnostic section length')
            byte = data[offset]
            offset += 1
            length |= (byte & 127) << shift
            if byte < 128:
                break
            shift += 7
        else:
            raise GateError('invalid diagnostic section length')
        need(offset + length <= len(data) and kind not in result, 'invalid or duplicate diagnostic section')
        result[kind] = data[offset:offset + length]
        offset += length
    return result

def ref(path: Path) -> dict[str, object]:
    return {"path": str(path.resolve()), "sha256": sha(path), "bytes": path.stat().st_size}

def read_json(path: Path) -> dict:
    value = json.loads(path.read_text())
    need(isinstance(value, dict), f"expected JSON object: {path}")
    return value

def load_module(path: Path, name: str):
    spec = importlib.util.spec_from_file_location(name, path)
    need(spec is not None and spec.loader is not None, f"cannot load oracle module {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module

def source_files(root: Path, fixture_paths: set[Path]) -> list[Path]:
    files = set((root / "crates/wtime-runtime/src").rglob("*.rs"))
    files.update((root / "tools/wtime-bench/p043-diagnostics/src").rglob("*.rs"))
    files.update(root / rel for rel in (
        "tools/wtime-bench/p043-diagnostics/Cargo.toml",
        "tools/wtime-bench/p043-diagnostics/Cargo.lock",
        "tools/wtime-bench/p043_coverage.py",
        "tools/wtime-bench/memory_workloads.py",
        "tools/wtime-bench/p027_memory_workloads.py",
        "tools/wtime-bench/run.py",
        "tools/wtime-bench/workloads.py",
        "tools/wtime-bench/workloads.json",
    ))
    files.update(fixture_paths)
    missing = sorted(str(path) for path in files if not path.is_file())
    need(not missing, "required P043 source inputs missing: " + ", ".join(missing))
    return sorted(files)

def snapshot(root: Path, paths: list[Path], destination: Path) -> dict[str, str]:
    result = {}
    for path in paths:
        relative = path.relative_to(root)
        output = destination / relative
        output.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(path, output)
        result[relative.as_posix()] = sha(path)
    return result

def run_logged(argv: list[str], cwd: Path, env: dict[str, str], out: Path,
               label: str, journal: list[dict], timeout: int = 600):
    try:
        proc = subprocess.run(argv, cwd=cwd, env=env, capture_output=True,
                              text=True, timeout=timeout)
        stdout, stderr, code = proc.stdout, proc.stderr, proc.returncode
    except subprocess.TimeoutExpired as exc:
        stdout = exc.stdout.decode(errors="replace") if isinstance(exc.stdout, bytes) else (exc.stdout or "")
        stderr = exc.stderr.decode(errors="replace") if isinstance(exc.stderr, bytes) else (exc.stderr or "")
        code = None
        proc = None
    stdout_path = out / f"{label}.stdout.log"
    stderr_path = out / f"{label}.stderr.log"
    stdout_path.write_text(stdout)
    stderr_path.write_text(stderr)
    row = {"label": label, "argv": argv, "cwd": str(cwd), "exit_code": code,
           "stdout": ref(stdout_path), "stderr": ref(stderr_path)}
    journal.append(row)
    if proc is None:
        raise GateError(f"command timed out: {label}")
    return proc, row

def n_for(row: dict) -> int:
    params = row.get("params", [])
    need(len(params) == 1 and type(params[0].get("baseline")) is int,
         f"{row.get('id')} needs one fixed i32 baseline")
    return params[0]["baseline"]

def expected_memory_for(workload_id: str, n: int, memory, run_oracle):
    if workload_id == "memory_fill_scan":
        image = bytearray(65_536)
        for i in range(n):
            image[i * 4:i * 4 + 4] = ((3 * i) & 0xffffffff).to_bytes(4, "little")
        return {"result": run_oracle.oracle(workload_id, n), "initial_pages": 1,
                "page_size_bytes": 65_536,
                "full_memory_sha256": hashlib.sha256(image).hexdigest()}
    return memory.expected_memory(workload_id, n)

def check_observation(row: dict, got: dict, expected_result: str, expected_memory: dict) -> dict:
    workload_id = row["id"]
    role = "primary" if workload_id in PRIMARY else "protected"
    need(got.get("schema") == SCHEMA and got.get("instrumented") is True
         and got.get("mode") == "compact", f"{workload_id} diagnostic identity mismatch")
    need(got.get("case") == workload_id and got.get("result") == expected_result,
         f"{workload_id} guest result differs from independent oracle")
    images = got.get("all_memory_images")
    need(isinstance(images, list) and images and got.get("memory_count") == len(images),
         f"{workload_id} missing full memory images")
    actual_hashes = []
    for image in images:
        need(isinstance(image, dict) and isinstance(image.get("hex"), str),
             f"{workload_id} malformed memory image")
        raw = bytes.fromhex(image["hex"])
        actual_hashes.append(hashlib.sha256(raw).hexdigest())
        need(len(raw) == image["pages"] * (1 << image["page_size_log2"]),
             f"{workload_id} memory image/page metadata mismatch")
    if expected_memory.get("memories"):
        expected_hashes = [item["full_memory_sha256"] for item in expected_memory["memories"]]
    elif expected_memory.get("all_memory_sha256"):
        expected_hashes = expected_memory["all_memory_sha256"]
    else:
        expected_hashes = [expected_memory["full_memory_sha256"]]
    need(actual_hashes == expected_hashes,
         f"{workload_id} full memory digest(s) differ from independent oracle")
    expected_pages = ([item["pages"] for item in expected_memory["memories"]]
                      if expected_memory.get("memories") else
                      [expected_memory.get("pages_after", expected_memory.get("initial_pages"))] * len(images))
    need([item["pages"] for item in images] == expected_pages,
         f"{workload_id} memory page count differs from independent oracle")
    expected_log2 = 0 if expected_memory.get("page_size_bytes") == 1 else 16
    need(all(item["page_size_log2"] == expected_log2 for item in images),
         f"{workload_id} memory page size differs from independent oracle")
    expected_index = expected_memory.get("exported_memory_index", expected_memory.get("memory_index", 0))
    need(got.get("exported_memory_index") == expected_index,
         f"{workload_id} selected memory export index differs from independent oracle")
    counters = got.get("counters")
    need(isinstance(counters, dict), f"{workload_id} counter snapshot missing")
    for key in COUNTERS:
        need(type(counters.get(key)) is int and counters[key] >= 0,
             f"{workload_id} invalid/missing {key}")
    need(counters["compact_shared_memory_accesses"] == 0,
         f"{workload_id} selected workload unexpectedly exercised shared memory")
    need(got.get("counter_reset_count") == 1 and got.get("counted_guest_invocations") == 1
         and got.get("oracle_preflight_guest_invocations") == 1,
         f"{workload_id} must separate one preflight from one fresh counted invocation")
    if role == "primary":
        need(counters["compact_memory_accesses"] > 0 and counters["compact_steps"] > 0,
             f"{workload_id} primary did not execute compact memory instructions")
    return {"id": workload_id, "role": role, "result": got["result"],
            "memory_count": got["memory_count"], "memory_pages": [x["pages"] for x in images],
            "memory_page_size_log2": [x["page_size_log2"] for x in images],
            "all_memory_sha256": actual_hashes, "counters": counters,
            "invocation_protocol": {"oracle_preflight_guest_invocations": 1,
                                    "counted_guest_invocations": 1, "counter_reset_count": 1}}

def execute(args, out: Path) -> dict:
    root = Path(__file__).resolve().parents[2]
    contract_path = args.contract.resolve(strict=True)
    contract = read_json(contract_path)
    need(set(contract) == {"schema_version", "selection", "primary_workloads", "protected_workloads"}
         and contract.get("schema_version") == 1 and contract.get("selection") == SELECTION,
         "P043 contract schema/selection mismatch")
    need(contract["primary_workloads"] == ["memory_widths_predictable", "memory_widths_shuffled"],
         "P043 primary workload set is not the frozen pair")
    need(len(contract["protected_workloads"]) == 33
         and set(contract["primary_workloads"]).isdisjoint(contract["protected_workloads"]),
         "P043 protected workload set must contain 33 disjoint rows")
    registry = read_json(root / "tools/wtime-bench/workloads.json")
    ids = registry["selections"][SELECTION]["workloads"]
    need(len(ids) == 35 and len(set(ids)) == 35, "memory selection must contain 35 unique workload IDs")
    need(set(ids) == set(contract["primary_workloads"]) | set(contract["protected_workloads"]),
         "registry and frozen contract row IDs differ")
    by_id = {row["id"]: row for row in registry["workloads"]}
    need(set(ids) <= set(by_id), "memory selection references missing registry rows")
    rows = [by_id[workload_id] for workload_id in ids]

    memory = load_module(root / "tools/wtime-bench/memory_workloads.py", "p043_memory_oracles")
    run_oracle = load_module(root / "tools/wtime-bench/run.py", "p043_run_oracles")
    oracle_entries = memory.entries()
    oracle_rows = {row["id"]: row for row in oracle_entries}
    if "memory_fill_scan" not in oracle_rows:
        fill_row = next((row for row in registry["workloads"]
                         if row.get("id") == "memory_fill_scan"), None)
        need(fill_row is not None, "memory_fill_scan is absent from memory and registry oracles")
        oracle_rows["memory_fill_scan"] = fill_row
    need(set(ids) == set(oracle_rows), "memory oracle entries differ from selection")
    fixture_paths = set()
    for workload_id in ids:
        source = oracle_rows[workload_id].get("source", {})
        rel = source.get("path")
        need(isinstance(rel, str) and rel.startswith("fixtures/"),
             f"{workload_id} source fixture missing")
        path = root / "tools/wtime-bench" / rel
        need(path.is_file() and sha(path) == source.get("sha256"),
             f"{workload_id} WAT fixture hash differs from oracle module")
        fixture_paths.add(path)
    all_sources = source_files(root, fixture_paths)
    before = {path.relative_to(root).as_posix(): sha(path) for path in all_sources}
    encoder = args.encoder.resolve(strict=True)
    need(encoder.is_file() and os.access(encoder, os.X_OK), "encoder must be an executable file")
    out_inputs = out / "inputs"
    out_inputs.mkdir(parents=True, exist_ok=True)
    helper_copy = out_inputs / "p043_coverage.py"
    contract_copy = out_inputs / "P043-contract.json"
    encoder_copy = out_inputs / "encoder"
    driver = root / "tools/wtime-bench/p043-diagnostics/src/memory_diagnostic_driver.rs"
    manifest = root / "tools/wtime-bench/p043-diagnostics/Cargo.toml"
    lock = root / "tools/wtime-bench/p043-diagnostics/Cargo.lock"
    for path in (Path(__file__).resolve(), driver, manifest, lock):
        need(path.is_file(), f"required P043 source is missing: {path}")
    shutil.copy2(Path(__file__).resolve(), helper_copy)
    shutil.copy2(contract_path, contract_copy)
    shutil.copy2(encoder, encoder_copy)
    encoder_copy.chmod(encoder_copy.stat().st_mode | 0o111)
    exact = [driver, manifest, lock]
    exact.extend(path for path in all_sources if path not in exact)
    source_snapshot = snapshot(root, exact, out_inputs / "source-snapshot")
    input_hashes = {"collector": ref(helper_copy), "encoder": ref(encoder_copy),
                    "encoder_original": ref(encoder), "contract": ref(contract_copy),
                    "contract_original": ref(contract_path), "sources": source_snapshot}

    env = os.environ.copy()
    env.pop("CARGO_TARGET_DIR", None)
    journal = ACTIVE_JOURNAL
    fixture_dir = out / "fixtures"
    fixture_dir.mkdir()
    fixture_records = {}
    for workload_id in ids:
        oracle_row = oracle_rows[workload_id]
        wat = root / "tools/wtime-bench" / oracle_row["source"]["path"]
        wasm = fixture_dir / f"{workload_id}.wasm"
        proc, encode_record = run_logged([str(encoder_copy), "encode", str(wat), "-o", str(wasm)],
                                         root, env, out, f"encode-{workload_id}", journal, 120)
        need(proc.returncode == 0, f"fixture encoding failed for {workload_id}")
        registered_hash = oracle_row["source"].get("wasm_sha256")
        need(isinstance(registered_hash, str) and sha(wasm) == registered_hash,
             f"fresh encoding hash mismatch for {workload_id}")
        fixture_records[workload_id] = {"wat": ref(wat), "wasm": ref(wasm),
                                        "registered_wasm_sha256": registered_hash,
                                        "encode_command": encode_record}
        if workload_id == 'memory_fill_scan':
            # The historical control has an unexported memory. Expose it only
            # in the untimed diagnostic copy; prove all other sections match.
            original_wat = wat.read_text()
            need(original_wat.count('(memory 1 1)') == 1, 'legacy memory declaration changed')
            adapted_wat = fixture_dir / 'memory_fill_scan-diagnostic.wat'
            adapted_wasm = fixture_dir / 'memory_fill_scan-diagnostic.wasm'
            adapted_wat.write_text(original_wat.replace('(memory 1 1)', '(memory (export "memory") 1 1)', 1))
            proc, adaptation = run_logged([str(encoder_copy), 'encode', str(adapted_wat), '-o', str(adapted_wasm)],
                                          root, env, out, 'encode-legacy-export-adapter', journal, 120)
            need(proc.returncode == 0, 'legacy diagnostic export encoding failed')
            original_sections = sections(wasm)
            adapted_sections = sections(adapted_wasm)
            need(set(original_sections) == set(adapted_sections)
                 and all(original_sections[k] == adapted_sections[k] for k in original_sections if k != 7)
                 and original_sections.get(7) != adapted_sections.get(7),
                 'diagnostic export adapter changed more than the export section')
            fixture_records[workload_id]['diagnostic_adapter'] = {
                'wat': ref(adapted_wat), 'wasm': ref(adapted_wasm), 'command': adaptation,
                'changed_sections': [7], 'code_and_data_unchanged': True,
                'timing_evidence': False}

    build_dir = out / "target"
    build = ["cargo", "build", "--locked", "--manifest-path",
             "tools/wtime-bench/p043-diagnostics/Cargo.toml", "--features", FEATURES,
             "--target-dir", str(build_dir)]
    proc, build_record = run_logged(build, root, env, out, "build-diagnostic", journal, 900)
    need(proc.returncode == 0, f"P043 diagnostic build failed: {out / 'build-diagnostic.stderr.log'}")
    binary = build_dir / "debug/wtime-bench"
    need(binary.is_file() and os.access(binary, os.X_OK), "diagnostic executable missing")
    shutil.copy2(binary, out / "wtime-bench")
    binary_copy = out / "wtime-bench"
    rows_out = []
    for row in rows:
        workload_id = row["id"]
        n = n_for(row)
        expected = run_oracle.oracle(workload_id, n) if workload_id == "memory_fill_scan" else memory.oracle(workload_id, n)
        expected_result = "trap" if expected == "trap" else f"i32:{expected}"
        expected_memory = expected_memory_for(workload_id, n, memory, run_oracle)
        wasm = fixture_dir / f"{workload_id}.wasm"
        if workload_id == 'memory_fill_scan':
            wasm = fixture_dir / 'memory_fill_scan-diagnostic.wasm'
        proc, run_record = run_logged([str(binary_copy), "--memory-diagnostic", str(wasm), workload_id,
                                       str(n), expected_result, "all-phase3-core"], root, env, out,
                                      f"row-{workload_id}", journal, 120)
        need(proc.returncode == 0, f"diagnostic row failed: {workload_id}")
        lines = Path(run_record["stdout"]["path"]).read_text().splitlines()
        need(len(lines) == 1, f"{workload_id} must emit exactly one JSON line")
        observation = json.loads(lines[0])
        result = check_observation(row, observation, expected_result, expected_memory)
        result["stdout_log"] = run_record["stdout"]
        result["stderr_log"] = run_record["stderr"]
        rows_out.append(result)
    after = {path.relative_to(root).as_posix(): sha(path) for path in all_sources}
    need(before == after, "P043 sources changed during diagnostic collection")
    runtime_before = {path: digest for path, digest in before.items()
                      if path.startswith("crates/wtime-runtime/src/")}
    runtime_after = {path: digest for path, digest in after.items()
                     if path.startswith("crates/wtime-runtime/src/")}
    need(runtime_before == runtime_after, "runtime sources changed during P043 diagnostics")
    need(len(rows_out) == 35 and sum(row["role"] == "primary" for row in rows_out) == 2
         and sum(row["role"] == "protected" for row in rows_out) == 33,
         "incomplete P043 diagnostic selection")
    return {"schema": SCHEMA, "status": "passed", "source_stable": True,
            "instrumented": True, "diagnostic_only": True, "timing_samples": 0,
            "selection": SELECTION, "selected_count": 35, "primary_count": 2,
            "protected_count": 33, "feature_set": FEATURES,
            "contract": ref(contract_path), "input_hashes": input_hashes,
            "source_map_before": before, "source_map_after": after,
            "runtime_source_map_before": runtime_before,
            "runtime_source_map_after": runtime_after,
            "source_map_sha256": canonical_sha(after), "fixtures": fixture_records,
            "binary": ref(binary_copy), "build_command": build_record,
            "commands": journal, "rows": rows_out, "helper": ref(helper_copy)}

def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--encoder", type=Path, required=True)
    parser.add_argument("--contract", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True,
                        help="new exclusive output directory for all logs and coverage.json")
    parser.add_argument("--execute", action="store_true",
                        help="required to encode, build, and execute the 35 diagnostic rows")
    args = parser.parse_args()
    if not args.execute:
        print("inert: P043 diagnostics require --execute; no inputs were read or outputs written")
        return 0
    output = args.output.resolve()
    root = Path(__file__).resolve().parents[2]
    need(output.is_relative_to(root) or output.is_relative_to(Path("/tmp")),
         "output must be inside the repository or /tmp")
    need(not output.exists(), "refusing to overwrite diagnostic output")
    output.mkdir(parents=True)
    (output / "inputs").mkdir()
    shutil.copy2(Path(__file__).resolve(), output / "inputs/p043_coverage.py")
    ACTIVE_JOURNAL.clear()
    try:
        result = execute(args, output)
        (output / "coverage.json").write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
        print(f"P043 diagnostic coverage recorded: {output / 'coverage.json'} (no timing samples)")
        return 0
    except Exception as exc:
        failure = {"schema": SCHEMA, "status": "failed", "error": f"{type(exc).__name__}: {exc}",
                   "commands": ACTIVE_JOURNAL}
        (output / "failure.json").write_text(json.dumps(failure, indent=2, sort_keys=True) + "\n")
        print(f"P043 diagnostic collection failed; retained output: {output}", file=sys.stderr)
        print(failure["error"], file=sys.stderr)
        return 2

if __name__ == "__main__":
    raise SystemExit(main())
