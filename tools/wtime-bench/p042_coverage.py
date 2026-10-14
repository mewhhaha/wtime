#!/usr/bin/env python3
"""Collect source-bound, oracle-checked P042 indirect/call_ref cache counters.

This helper is inert unless --execute is supplied. Its diagnostic binary is
built in the reusable compact-diagnostic cache, copied into exclusive P042
evidence, and invoked only from that retained copy. It records no timing values.
"""
from __future__ import annotations
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

sys.dont_write_bytecode = True
SIZES = (0, 1, 17, 32, 1024, 10000)
PRIMARY = {f"{family}_n{n}" for family in ("scalar_indirect_calls", "scalar_call_ref") for n in SIZES if n > 0}
EXPECTED_POSITIVE_COMPACT_PRIMARY_IDS = set(PRIMARY)
EXPECTED_PRIMARY_MISS_IDS = set(PRIMARY)
CONTROL_IDS = {f"{family}_n0" for family in ("scalar_indirect_calls", "scalar_call_ref")} | {
    f"scalar_table_mutation_n{n}" for n in SIZES
}
P042_FIXTURE_COUNT = 7
SCALAR_ROW_COUNT = 85
COUNTED_INVOCATIONS_PER_ROW = 2
ORACLE_PREFLIGHTS_PER_ROW = 1
P042_FIXTURE_NAMES = {"call-ref", "host-indirect", "invalid-indirect", "table-alias-consumer",
                      "table-alias-provider", "table-mutations", "table64-indirect"}
FEATURES = "compact-executor,execution-reference,performance-counters"


class GateError(RuntimeError):
    pass

def need(ok: bool, message: str) -> None:
    if not ok:
        raise GateError(message)

def sha(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for block in iter(lambda: f.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()

def canonical_sha(value: object) -> str:
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":")).encode()).hexdigest()

def read_json(path: Path) -> dict:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except Exception as exc:
        raise GateError(f"invalid JSON {path}: {exc}") from exc
    need(isinstance(value, dict), f"expected JSON object: {path}")
    return value

def verify_ref(ref: dict, label: str) -> Path:
    need(isinstance(ref, dict) and isinstance(ref.get("path"), str), f"{label} reference is missing")
    path = Path(ref["path"]).resolve(strict=True)
    need(path.is_file() and sha(path) == ref.get("sha256") and path.stat().st_size == ref.get("bytes"),
         f"{label} path/SHA/bytes mismatch")
    return path

def validate_durable_fixture_provenance(prepare_receipt: dict, check_result: dict) -> tuple[Path, dict]:
    """Validate durable P042 provenance and its distinct source-only packet ref."""
    check_runtime = check_result.get("runtime_source_map")
    need(isinstance(check_runtime, dict) and len(check_runtime) == 29 and
         check_result.get("runtime_source_count") == 29,
         "P042 durable fixture provenance requires the complete 29-file runtime map")
    check_ref = check_result.get("fixture_provenance")
    check_path = verify_ref(check_ref, "P042 check-bound durable fixture provenance")
    durable = read_json(check_path)
    need(durable.get("schema") == "p042-durable-fixtures-v1" and
         durable.get("status") == "freshly_encoded_and_validated" and
         durable.get("receipt") == check_result.get("receipt") and
         durable.get("implementation_sources") == check_runtime,
         "P042 durable fixture provenance schema, receipt, or 29-file implementation map is invalid")
    need(durable.get("fixtures") == prepare_receipt.get("fixtures") and
         durable.get("tools") == prepare_receipt.get("tools") and
         durable.get("manual_oracles") == prepare_receipt.get("manual_oracles") and
         durable.get("licenses") == prepare_receipt.get("licenses"),
         "P042 durable fixture provenance does not bind the prepared fixtures/tools/oracles/licenses")
    packet_ref = durable.get("packet_provenance")
    prepared_packet_ref = prepare_receipt.get("provenance")
    need(packet_ref == prepared_packet_ref,
         "P042 durable provenance does not bind the source-only packet provenance reference")
    packet_path = verify_ref(packet_ref, "P042 source-only packet provenance")
    need(packet_path == verify_ref(prepared_packet_ref, "P042 prepared packet provenance"),
         "P042 source-only packet provenance path differs from the prepare receipt")
    return check_path, durable

def source_roles(root: Path, fixture_paths: set[Path]) -> dict[str, int]:
    crate_rs = [p for p in (root / "crates").rglob("*.rs") if p.is_file()]
    runtime_rs = [p for p in (root / "crates/wtime-runtime/src").rglob("*.rs") if p.is_file()]
    bench_rs = [p for p in (root / "tools/wtime-bench/src").rglob("*.rs") if p.is_file()]
    crate_manifests = [p for p in (root / "crates").rglob("Cargo.toml") if p.is_file()]
    static_files = [root / rel for rel in (
        "Cargo.toml", "Cargo.lock", "LICENSE-APACHE", "LICENSE-MIT",
        "tools/wtime-bench/Cargo.toml",
        "tools/wtime-bench/Cargo.lock", "tools/wtime-bench/workloads.json",
        "tools/wtime-bench/scalar_workloads.py", "tools/wtime-bench/run.py",
        "tools/wtime-bench/src/scalar_driver.rs",
    )]
    return {"production_crate_rust_sources": len(crate_rs),
            "runtime_rust_sources": len(runtime_rs), "bench_rust_sources": len(bench_rs),
            "production_crate_manifests": len(crate_manifests),
            "shared_manifests_registry_oracles_licenses": len(set(static_files)),
            "selected_wat_fixtures": len(fixture_paths),
            "total_source_files": len(set(crate_rs + bench_rs + crate_manifests + static_files + list(fixture_paths)))}

def source_map(root: Path, fixture_paths: set[Path]) -> dict[str, str]:
    paths = set()
    for base in (root / "crates", root / "tools/wtime-bench/src"):
        paths.update(p for p in base.rglob("*.rs") if p.is_file())
    paths.update(p for p in (root / "crates").rglob("Cargo.toml") if p.is_file())
    paths.update(root / rel for rel in (
        "Cargo.toml", "Cargo.lock", "LICENSE-APACHE", "LICENSE-MIT",
        "tools/wtime-bench/Cargo.toml",
        "tools/wtime-bench/Cargo.lock", "tools/wtime-bench/workloads.json",
        "tools/wtime-bench/scalar_workloads.py", "tools/wtime-bench/run.py",
        "tools/wtime-bench/src/scalar_driver.rs",
    ))
    paths.update(fixture_paths)
    result = {}
    for path in sorted(paths):
        need(path.is_file(), f"source input missing: {path}")
        result[path.relative_to(root).as_posix()] = sha(path)
    return result

def runtime_map(root: Path) -> dict[str, str]:
    """Return root-relative hashes for every runtime Rust source file."""
    base = root / "crates/wtime-runtime/src"
    need(base.is_dir(), "runtime source directory is missing")
    paths = sorted(p for p in base.rglob("*.rs") if p.is_file())
    need(paths, "runtime source map is empty")
    return {p.relative_to(root).as_posix(): sha(p) for p in paths}

def archive_runtime_map(manifest: dict) -> dict[str, str]:
    result = {}
    for archive_path, record in manifest.get("files", {}).items():
        if not isinstance(record, dict) or record.get("role") != "source":
            continue
        relative = record.get("source_path") or archive_path.removeprefix("source/")
        if relative.startswith("crates/wtime-runtime/src/"):
            result[relative] = record.get("sha256")
    return result

def load_module(path: Path, name: str):
    spec = importlib.util.spec_from_file_location(name, path)
    need(spec is not None and spec.loader is not None, f"cannot load oracle module {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module

def n_for(row: dict) -> int:
    args = row.get("oracle_arguments", {})
    if isinstance(args, dict) and type(args.get("n")) is int:
        n = args["n"]
    else:
        params = row.get("params", [])
        matches = [p for p in params if isinstance(p, dict) and p.get("name") == "n"]
        need(len(matches) == 1 and type(matches[0].get("baseline")) is int,
             f"{row.get('id')} has no single integer n baseline/oracle argument")
        n = matches[0]["baseline"]
    params = row.get("params", [])
    matches = [p for p in params if isinstance(p, dict) and p.get("name") == "n"]
    if matches:
        need(matches[0].get("min", n) <= n <= matches[0].get("max", n),
             f"{row.get('id')} oracle input is outside registered bounds")
    need(0 <= n <= 0x7fffffff, f"{row.get('id')} is not a nonnegative i32 input")
    return n

def validate_counter_observation(wid: str, n: int, role: str, family: str, counters: object) -> dict:
    fields = ("compact_steps", "compact_bound_calls", "compact_indirect_hits",
              "compact_indirect_misses", "compact_indirect_evictions")
    need(isinstance(counters, dict), f"{wid} counter object is missing")
    need(all(key in counters for key in fields),
         f"{wid} runtime JSON lacks required P042 compact indirect/cache fields")
    for key in fields:
        need(type(counters[key]) is int and counters[key] >= 0,
             f"{wid} {key} is not a nonnegative integer")
    if role == "primary":
        need(n >= 1, f"P042 primary workload unexpectedly has n=0: {wid}")
        need(counters["compact_steps"] > 0,
             f"P042 primary row {wid} did not execute compact steps")
        need(counters["compact_indirect_misses"] > 0,
             f"P042 primary row {wid} did not exercise an initial compatibility miss")
        need(counters["compact_indirect_hits"] > 0,
             f"P042 primary row {wid} did not exercise the same-instance repeat hit")
    if n == 0 and family in {"scalar_indirect_calls", "scalar_call_ref", "scalar_table_mutation"}:
        need(counters["compact_indirect_hits"] == 0 and counters["compact_indirect_misses"] == 0,
             f"{wid} is an n=0 call control and must truthfully record zero indirect-cache lookups")
    return counters

def validate_primary_coverage(rows: list[dict]) -> dict:
    primary = [row for row in rows if row.get("role") == "primary"]
    need(len(primary) == 10 and {row.get("id") for row in primary} == PRIMARY,
         "P042 coverage must contain the exact ten nonzero indirect/call_ref primary IDs")
    step_ids = sorted(row["id"] for row in primary if row["counters"]["compact_steps"] > 0)
    hit_ids = sorted(row["id"] for row in primary if row["counters"]["compact_indirect_hits"] > 0)
    miss_ids = sorted(row["id"] for row in primary if row["counters"]["compact_indirect_misses"] > 0)
    need(set(step_ids) == PRIMARY and set(hit_ids) == EXPECTED_POSITIVE_COMPACT_PRIMARY_IDS and
         set(miss_ids) == EXPECTED_PRIMARY_MISS_IDS,
         "P042 primary compact-step/miss/hit counter coverage is incomplete")
    return {"compact_step_primary_ids": step_ids,
            "compact_indirect_hit_primary_ids": hit_ids,
            "compact_indirect_miss_primary_ids": miss_ids,
            "same_instance_repeat_count": 2,
            "hit_proof": "Each primary diagnostic first performs one disposable independent-oracle preflight, then performs two oracle-checked run invocations on the same fresh instance after one counter reset; n>=1 ensures the second counted invocation revisits its call site.",
            "oracle_preflight_guest_invocations": ORACLE_PREFLIGHTS_PER_ROW,
            "counted_guest_invocations": COUNTED_INVOCATIONS_PER_ROW,
            "total_guest_invocations": ORACLE_PREFLIGHTS_PER_ROW + COUNTED_INVOCATIONS_PER_ROW}

def expected_for(row: dict, n: int, scalar_oracle, run_oracle) -> int:
    oracle = row.get("oracle")
    need(isinstance(oracle, dict) and oracle.get("independent") is True,
         f"{row.get('id')} is missing an independent registry oracle")
    module = oracle.get("module")
    wid = row["id"]
    if module == "scalar_workloads.py":
        value = scalar_oracle.oracle(wid, n)
    elif module == "run.py":
        value = run_oracle.oracle(wid, n)
    else:
        raise GateError(f"{wid} names unsupported oracle module {module!r}")
    need(type(value) is int and -0x80000000 <= value <= 0x7fffffff,
         f"{wid} oracle did not return an i32")
    return value

def registry_inputs(root: Path, contract_path: Path):
    workloads_path = root / "tools/wtime-bench/workloads.json"
    registry = read_json(workloads_path)
    selection = registry.get("selections", {}).get("scalar", {}).get("workloads")
    need(isinstance(selection, list) and len(selection) == 85 and len(set(selection)) == 85,
         "registered scalar selection must contain exactly 85 unique rows")
    need(all(isinstance(x, str) for x in selection), "scalar selection must contain workload IDs")
    entries = registry.get("workloads")
    need(isinstance(entries, list), "workloads registry rows are missing")
    by_id = {row.get("id"): row for row in entries if isinstance(row, dict)}
    need(len(by_id) == len(entries), "workload IDs are missing or duplicated")
    rows = [by_id.get(wid) for wid in selection]
    need(all(isinstance(row, dict) for row in rows), "scalar selection references a missing workload")
    for row in rows:
        driver = row.get("driver") or {}
        need(isinstance(driver, dict), f"{row['id']} driver declaration must be an object")
        kind = driver.get("kind", "plain")
        profile = driver.get("profile", "mvp")
        need(kind in {"plain", "host_tick", "table_a", "table_b"},
             f"{row['id']} has unsupported diagnostic driver {kind}")
        need(profile in {"mvp", "stable-core", "all-phase3-core"},
             f"{row['id']} has unsupported diagnostic profile {profile}")
    contract = read_json(contract_path)
    need(contract.get("schema_version") == 1 and contract.get("selection") == "scalar" and
         set(contract.get("primary_workloads", [])) == PRIMARY and
         set(contract.get("protected_workloads", [])) == set(selection) - PRIMARY and
         len(contract.get("primary_workloads", [])) == 10 and
         len(contract.get("protected_workloads", [])) == 75 and
         CONTROL_IDS <= set(contract.get("protected_workloads", [])),
         "P042 contract must partition 10 primary/75 protected, including n=0 and all table-mutation controls")
    fixture_paths = set()
    bench = root / "tools/wtime-bench"
    for row in rows:
        source = row.get("source")
        need(isinstance(source, dict) and isinstance(source.get("path"), str),
             f"{row['id']} has no registered WAT fixture")
        wat = bench / source["path"]
        need(wat.is_file(), f"{row['id']} WAT fixture is missing")
        need(sha(wat) == source.get("sha256"), f"{row['id']} WAT hash differs from registry")
        fixture_paths.add(wat)
    return registry, rows, contract, fixture_paths

def run_logged(command: list[str], cwd: Path, env: dict, out: Path, label: str, journal: list[dict]) -> dict:
    need(not (out / f"{label}.stdout.log").exists() and not (out / f"{label}.stderr.log").exists(),
         f"refusing to overwrite {label} logs")
    completed = subprocess.run(command, cwd=cwd, env=env, text=True, capture_output=True, timeout=600)
    stdout_path = out / f"{label}.stdout.log"
    stderr_path = out / f"{label}.stderr.log"
    stdout_path.write_text(completed.stdout, encoding="utf-8")
    stderr_path.write_text(completed.stderr, encoding="utf-8")
    record = {"label": label, "argv": command, "cwd": str(cwd), "exit_code": completed.returncode,
              "stdout": {"path": str(stdout_path.resolve()), "sha256": sha(stdout_path), "bytes": stdout_path.stat().st_size},
              "stderr": {"path": str(stderr_path.resolve()), "sha256": sha(stderr_path), "bytes": stderr_path.stat().st_size}}
    journal.append(record)
    journal_path = out / "commands.partial.json"
    journal_tmp = out / "commands.partial.json.tmp"
    journal_tmp.write_text(json.dumps({"schema": "wtime-p042-command-journal-v1",
        "completed_commands": len(journal), "commands": journal}, indent=2, sort_keys=True) + "\n",
        encoding="utf-8")
    journal_tmp.replace(journal_path)
    return record

def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--workspace-root", type=Path, required=True)
    parser.add_argument("--contract", type=Path)
    parser.add_argument("--prepare-receipt", type=Path,
                        help="completed P042 prepare receipt that pins copied encoder, validator and seven fixtures")
    parser.add_argument("--check-result", type=Path,
                        help="passed P042 source-bound check (29-file runtime map and validation closure)")
    parser.add_argument("--encoder", type=Path, help="optional exact-path assertion against the prepare receipt")
    parser.add_argument("--validator", type=Path, help="optional exact-path assertion against the prepare receipt")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--execute", action="store_true", help="required to build/run the diagnostic collector")
    args = parser.parse_args()
    root = args.workspace_root.resolve(strict=True)
    contract_path = (args.contract or root / "docs/benchmarks/performance/P042-diagnostic-contract.json").resolve()
    out = (args.output or root / "target/performance/P042/coverage").resolve()
    if not args.execute:
        print(json.dumps({"status": "inert", "would_collect": SCALAR_ROW_COUNT, "primary": 10, "protected": 75,
                          "oracle_preflight_guest_invocations": SCALAR_ROW_COUNT * ORACLE_PREFLIGHTS_PER_ROW,
                          "counted_guest_invocations": SCALAR_ROW_COUNT * COUNTED_INVOCATIONS_PER_ROW,
                          "total_guest_invocations": SCALAR_ROW_COUNT * (ORACLE_PREFLIGHTS_PER_ROW + COUNTED_INVOCATIONS_PER_ROW),
                          "features": FEATURES, "output": str(out),
                          "contract": str(contract_path),
                          "prepare_receipt": str(args.prepare_receipt) if args.prepare_receipt else None,
                          "encoder_default": "P042 prepare receipt tools.encoder.path",
                          "validator_default": "P042 prepare receipt tools.validator.path",
                          "note": "No build, binary invocation, or file output occurs without --execute."}, indent=2))
        return 0
    need(root.is_dir() and (root / "Cargo.toml").is_file(), "workspace root is not a Rust workspace")
    need(contract_path.is_file(), f"P042 predeclared contract is missing: {contract_path}")
    need(args.prepare_receipt is not None and args.check_result is not None, "--execute requires completed P042 prepare and source-bound check records")
    prepare_receipt_path = args.prepare_receipt.resolve(strict=True)
    prepare_receipt = read_json(prepare_receipt_path)
    check_result_path = args.check_result.resolve(strict=True)
    check_result = read_json(check_result_path)
    need(prepare_receipt.get("schema") == "p042-fresh-fixture-validation-v1" and
         prepare_receipt.get("status") == "passed" and prepare_receipt.get("phase") == "prepare",
         "P042 prepare receipt is not a passing fresh-fixture validation closure")
    need(check_result.get("schema") == "p042-source-bound-check-v1" and
         check_result.get("phase") == "check" and check_result.get("status") == "passed" and
         check_result.get("passed") is True and check_result.get("source_stable") is True,
         "P042 source-bound check is not a passing validation closure")
    check_receipt_path = verify_ref(check_result.get("receipt"), "P042 check-bound prepare receipt")
    need(sha(check_receipt_path) == sha(prepare_receipt_path),
         "P042 check and coverage inputs bind different prepare receipts")
    check_p041_ref = check_result.get("p041_evidence")
    p041_ref = prepare_receipt.get("p041_evidence")
    p041_path = verify_ref(p041_ref, "P042 closed P041 evidence")
    need(check_p041_ref == p041_ref, "P042 check and prepare bind different P041 evidence")
    p041_proof = read_json(p041_path)
    p041_evidence_sha = p041_ref["sha256"]
    predecessor_runtime = prepare_receipt.get("runtime_predecessor")
    need(p041_proof.get("schema") == "p041-instance-call-evidence-v1" and
         p041_proof.get("task") == "P041" and p041_proof.get("status") == "passed" and
         p041_proof.get("passed") is True and p041_proof.get("source_stable") is True and
         isinstance(predecessor_runtime, dict) and len(predecessor_runtime) == 28 and
         p041_proof.get("runtime_source_map") == predecessor_runtime and
         p041_proof.get("runtime_source_count") == 28 and
         p041_proof.get("runtime_source_sha256") == canonical_sha(predecessor_runtime),
         "P042 predecessor does not match the completed 28-file P041 source map")
    check_p039_ref = check_result.get("p039_receipt")
    check_p039_path = verify_ref(check_p039_ref, "P042 check-bound P039 prepare receipt")
    need(check_p039_ref == prepare_receipt.get("p039_receipt"),
         "P042 check and prepare bind different P039 prepare receipt")
    check_fixture_provenance_ref = check_result.get("fixture_provenance")
    check_fixture_provenance, durable_provenance = validate_durable_fixture_provenance(
        prepare_receipt, check_result)
    test_contract_ref = check_result.get("test_contract")
    test_contract_path = verify_ref(test_contract_ref, "P042 source-bound test contract")
    test_contract = read_json(test_contract_path)
    need(test_contract_path != contract_path.resolve(strict=True) and
         test_contract.get("schema") == "p042-source-bound-test-contract-v1" and
         test_contract.get("nonzero_cache_coverage_required") is True and
         test_contract.get("nonzero_compact_coverage_required") is True and
         len(test_contract.get("named_tests", [])) == 10 and
         len(set(test_contract["named_tests"])) == 10 and
         len(test_contract.get("public_tests", [])) == 7 and
         len(test_contract.get("private_tests", [])) == 3 and
         set(test_contract["public_tests"]).isdisjoint(test_contract["private_tests"]) and
         set(test_contract["public_tests"]) | set(test_contract["private_tests"]) == set(test_contract["named_tests"]),
         "P042 source-bound test manifest and diagnostic workload-role contract are invalid")
    for test_source in test_contract.get("test_sources", []):
        verify_ref(test_source, "P042 source-bound focused test source")
    check_runtime = check_result.get("runtime_source_map")
    need(isinstance(check_runtime, dict) and len(check_runtime) == 29 and
         check_result.get("runtime_source_count") == 29,
         "P042 check does not bind the complete 29-file runtime map")
    if "runtime_source_sha256" in check_result:
        need(check_result["runtime_source_sha256"] == canonical_sha(check_runtime),
             "P042 check runtime source map digest is inconsistent")
    receipt_tools = prepare_receipt.get("tools")
    need(isinstance(receipt_tools, dict), "P042 prepare receipt tools map is missing")
    tool_refs = {name: receipt_tools.get(name) for name in ("encoder", "validator")}
    need(all(isinstance(ref, dict) and isinstance(ref.get("path"), str) and
             isinstance(ref.get("sha256"), str) and isinstance(ref.get("bytes"), int)
             for ref in tool_refs.values()), "P042 prepare receipt does not pin copied tools")
    encoder_receipt_path = verify_ref(tool_refs["encoder"], "P042 encoder copy")
    validator_receipt_path = verify_ref(tool_refs["validator"], "P042 validator copy")
    need(tool_refs["encoder"].get("original_sha256") == tool_refs["encoder"]["sha256"] and
         tool_refs["validator"].get("original_sha256") == tool_refs["validator"]["sha256"] and
         oct(encoder_receipt_path.stat().st_mode & 0o7777) == tool_refs["encoder"].get("mode") and
         oct(validator_receipt_path.stat().st_mode & 0o7777) == tool_refs["validator"].get("mode") and
         os.access(encoder_receipt_path, os.X_OK) and os.access(validator_receipt_path, os.X_OK),
         "P042 receipt tool copies do not match the pinned executable identities")
    encoder, validator = encoder_receipt_path, validator_receipt_path
    if args.encoder is not None:
        need(args.encoder.resolve(strict=True) == encoder, "encoder path differs from the P042 receipt")
    if args.validator is not None:
        need(args.validator.resolve(strict=True) == validator, "validator path differs from the P042 receipt")
    encoder_sha, validator_sha = sha(encoder), sha(validator)
    fixture_receipt_rows = prepare_receipt.get("fixtures")
    need(isinstance(fixture_receipt_rows, dict) and len(fixture_receipt_rows) == P042_FIXTURE_COUNT,
         "P042 receipt must bind all seven fresh correctness fixtures")
    need(set(fixture_receipt_rows) == P042_FIXTURE_NAMES,
         "P042 receipt must bind the exact seven staged fixture names")
    prepared_fixture_refs = {str(test_contract_path): test_contract_ref["sha256"]}
    for fixture_name, fixture_row in fixture_receipt_rows.items():
        need(isinstance(fixture_row, dict), f"P042 fixture receipt row malformed: {fixture_name}")
        need(set(fixture_row.get("commands", {})) == {"encode", "validate"},
             f"P042 fixture does not bind encode and validation: {fixture_name}")
        for key in ("source", "wasm", "manual_oracle", "provenance"):
            fixture_ref = fixture_row.get(key)
            fixture_path = verify_ref(fixture_ref, f"P042 fixture {key} {fixture_name}")
            prepared_fixture_refs[str(fixture_path)] = fixture_ref["sha256"]
        need(fixture_row["manual_oracle"] == prepare_receipt.get("manual_oracles") and
             fixture_row["provenance"] == prepare_receipt.get("provenance"),
             f"P042 fixture-level oracle/provenance refs differ from receipt: {fixture_name}")
        for kind, command in fixture_row["commands"].items():
            log_path = verify_ref(command.get("log"), f"P042 {fixture_name} {kind} command log")
            log = read_json(log_path)
            need(log.get("exit_code") == 0 and log.get("command") == command.get("command"),
                 f"P042 fixture command record mismatch: {fixture_name} {kind}")
            if kind == "validate":
                need(log.get("stdout", "").strip() == "valid",
                     f"P042 fixture validator did not affirm valid: {fixture_name}")
    manual_oracles = verify_ref(prepare_receipt.get("manual_oracles"), "P042 manual oracle")
    provenance = verify_ref(prepare_receipt.get("provenance"), "P042 fixture provenance")
    packet_manifest = verify_ref(prepare_receipt.get("packet_manifest"), "P042 fixture packet manifest")
    packet_proof = read_json(provenance)
    manual_oracle_proof = read_json(manual_oracles)
    need(packet_proof.get("schema") == "p042-fixture-source-packet-v1" and
         packet_proof.get("status") == "draft_source_only_unencoded_unvalidated_unexecuted" and
         manual_oracle_proof.get("schema") == "p042-static-manual-oracles-v1" and
         manual_oracle_proof.get("status") == "source-derived; WAT unencoded and unvalidated; no runtime observations",
         "P042 staged fixture/oracle provenance must retain its source-only claims")
    licenses = prepare_receipt.get("licenses")
    need(isinstance(licenses, dict) and set(licenses) == {"LICENSE.txt"},
         "P042 prepare receipt must bind the staged dual MIT/Apache license")
    license_paths = {name: verify_ref(ref, "P042 license " + name) for name, ref in licenses.items()}
    license_hashes_before = {name: sha(path) for name, path in license_paths.items()}
    project_license = next(iter(license_paths.values()))
    license_text = project_license.read_text(encoding="utf-8")
    need("MIT License" in license_text and "Apache License" in license_text,
         "P042 fixture license must preserve both MIT and Apache terms")
    need(not out.exists(), f"exclusive output already exists: {out}")
    registry, rows, contract, fixture_paths = registry_inputs(root, contract_path)
    registered_ids = set(registry["selections"]["scalar"]["workloads"])
    need(PRIMARY <= registered_ids, "P042 indirect/call_ref primary workload IDs are missing from the scalar registry")
    contract_sha_before = sha(contract_path)
    repo_driver = root / "tools/wtime-bench/src/scalar_driver.rs"
    need(repo_driver.is_file(), "integrated instrumented scalar diagnostic driver is missing")
    driver_path = repo_driver
    before = source_map(root, fixture_paths)
    driver_sha_before = sha(repo_driver)
    runtime_before = {relative: digest for relative, digest in before.items()
                      if relative.startswith("crates/wtime-runtime/src/")}
    need(isinstance(predecessor_runtime, dict) and len(predecessor_runtime) == 28 and
         all(isinstance(k, str) and isinstance(v, str) and len(v) == 64 and
             all(c in "0123456789abcdef" for c in v) for k, v in predecessor_runtime.items()) and
         len(runtime_before) == 29 and runtime_before != predecessor_runtime and
         "crates/wtime-runtime/src/execution/compact/indirect.rs" in runtime_before,
         "P042 collector must bind the 28-file P041 predecessor and integrated 29-file runtime separately")
    p039_receipt_ref = prepare_receipt.get("p039_receipt")
    p039_receipt_path = verify_ref(p039_receipt_ref, "pinned P039 prepare receipt")
    p039_receipt = read_json(p039_receipt_path)
    need(p039_receipt.get("schema") == "p039-fresh-fixture-validation-v1" and
         p039_receipt.get("status") == "passed" and p039_receipt.get("phase") == "prepare" and
         check_p039_path == p039_receipt_path,
         "P042 prepare/check do not bind the same passed P039 fixture/tool receipt")
    before_manifest_path = verify_ref(prepare_receipt.get("before_manifest"), "P042-before manifest")
    p039_receipt_sha = p039_receipt_ref["sha256"]
    before_manifest_sha = prepare_receipt["before_manifest"]["sha256"]
    before_manifest = read_json(before_manifest_path)
    need(before_manifest.get("mode") == "compact" and archive_runtime_map(before_manifest) == predecessor_runtime,
         "P042-before archive is not compact or does not match the 28-file P041 predecessor")
    need(check_runtime == runtime_map(root), "P042 check runtime map differs from current installed sources")
    archive_command_path = verify_ref(prepare_receipt.get("archive_command"), "P042 compact archive command")
    archive_command = read_json(archive_command_path)
    need(archive_command.get("exit_code") == 0,
         "P042 prepare receipt does not bind a successful compact before-archive command")
    before_archive_root = before_manifest_path.parent
    for relative, digest in predecessor_runtime.items():
        archived_source = before_archive_root / "source" / relative
        need(archived_source.is_file() and sha(archived_source) == digest,
             f"P042-before archived runtime source mismatch: {relative}")
    perf_path = root / "crates/wtime-runtime/src/perf.rs"
    frame_path = root / "crates/wtime-runtime/src/execution.rs"
    compact_execute_path = root / "crates/wtime-runtime/src/execution/compact/execute.rs"
    perf_text, frame_text, compact_execute_text = (p.read_text(encoding="utf-8")
        for p in (perf_path, frame_path, compact_execute_path))
    driver_text = driver_path.read_text(encoding="utf-8")
    cache_fields = ("compact_indirect_hits", "compact_indirect_misses", "compact_indirect_evictions")
    need('pub compact_steps: u64' in perf_text and 'compact_bound_calls' in perf_text and
         all(field in perf_text for field in cache_fields) and "store.performance_counters().to_json()" in driver_text and
         r'\"compact_indirect_hits\"' in perf_text and
         r'\"compact_indirect_misses\"' in perf_text and
         r'\"compact_indirect_evictions\"' in perf_text and
         "wtime-compact-scalar-cache-diagnostic-v1" in driver_text and
         "--scalar-cache-diagnostic" in driver_text,
         "P042 two-invocation diagnostic schema/cache counters are missing")
    indirect_cache_path = root / "crates/wtime-runtime/src/execution/compact/indirect.rs"
    indirect_module_path = root / "crates/wtime-runtime/src/execution/compact/mod.rs"
    need(indirect_cache_path.is_file() and indirect_module_path.is_file(),
         "P042 cache leaf/module source is missing")
    counter_runtime_sources = {
        "perf.rs": {"path": str(perf_path), "sha256": sha(perf_path)},
        "execution.rs": {"path": str(frame_path), "sha256": sha(frame_path)},
        "compact_execute.rs": {"path": str(compact_execute_path), "sha256": sha(compact_execute_path)},
        "compact_indirect.rs": {"path": str(indirect_cache_path), "sha256": sha(indirect_cache_path)},
        "compact_mod.rs": {"path": str(indirect_module_path), "sha256": sha(indirect_module_path)},
    }
    roles_before = source_roles(root, fixture_paths)
    # Require every collected source role, and derive the map cardinality from
    # those actual files instead of an arbitrary lower bound.
    need(roles_before["production_crate_rust_sources"] > 0 and
         roles_before["production_crate_manifests"] > 0 and
         roles_before["runtime_rust_sources"] > 0 and roles_before["bench_rust_sources"] > 0 and
         roles_before["selected_wat_fixtures"] == len(fixture_paths) and
         len(before) == roles_before["total_source_files"],
         "input source inventory does not match runtime, harness, manifest, license, oracle, and WAT roles")
    scalar_oracle = load_module(root / "tools/wtime-bench/scalar_workloads.py", "p042_scalar_oracle")
    run_oracle = load_module(root / "tools/wtime-bench/run.py", "p042_run_oracle")
    oracles = {"scalar_workloads.py": sha(root / "tools/wtime-bench/scalar_workloads.py"),
               "run.py": sha(root / "tools/wtime-bench/run.py")}
    out.mkdir(parents=True, exist_ok=False)
    source_copy = out / "collect.executed.py"
    shutil.copyfile(Path(__file__).resolve(), source_copy)
    helper_sha = sha(source_copy)
    need(helper_sha == sha(Path(__file__).resolve()), "executed collector snapshot differs from helper source")
    staged_driver_copy = out / "scalar_driver.staged.rs"
    shutil.copyfile(driver_path, staged_driver_copy)
    staged_driver_sha = sha(staged_driver_copy)
    need(staged_driver_sha == sha(driver_path), "staged diagnostic-driver snapshot changed")
    tool_copy_dir = out / "tools"
    tool_copy_dir.mkdir()
    encoder_copy, validator_copy = tool_copy_dir / "wtime-conformance", tool_copy_dir / "wtime-cli"
    shutil.copy2(encoder, encoder_copy)
    shutil.copy2(validator, validator_copy)
    need(sha(encoder_copy) == encoder_sha == tool_refs["encoder"]["sha256"] and
         sha(validator_copy) == validator_sha == tool_refs["validator"]["sha256"] and
         os.access(encoder_copy, os.X_OK) and os.access(validator_copy, os.X_OK),
         "exclusive encoder/validator copies do not match P042 prepare receipt")
    prepare_receipt_sha = sha(prepare_receipt_path)
    check_result_sha = sha(check_result_path)
    license_identity = {"path": str(project_license), "sha256": sha(project_license),
                        "bytes": project_license.stat().st_size,
                        "manual_oracles_path": str(manual_oracles),
                        "manual_oracles_sha256": sha(manual_oracles),
                        "manual_oracles_bytes": manual_oracles.stat().st_size}
    license_sha_before = license_identity["sha256"]
    manual_oracles_sha_before = license_identity["manual_oracles_sha256"]
    build_dir = root / "target/performance/build-caches/compact-diagnostic-p042"
    binary_copy = out / "wtime-bench"
    command_journal: list[dict] = []
    fixture_dir = out / "fixtures"
    fixture_records = {}
    fixture_sources = {}
    for row in rows:
        rel = row["source"]["path"]
        fixture_sources[rel] = root / "tools/wtime-bench" / rel
    need(len(fixture_sources) == 17, "P042 scalar selection must encode exactly 17 unique fixtures")
    env = os.environ.copy()
    env.pop("CARGO_TARGET_DIR", None)
    build_argv = ["cargo", "build", "--locked", "--manifest-path", "tools/wtime-bench/Cargo.toml",
                  "--features", FEATURES, "--target-dir", str(build_dir)]
    for index, (rel, wat) in enumerate(sorted(fixture_sources.items())):
        destination = fixture_dir / Path(rel).with_suffix(".wasm")
        destination.parent.mkdir(parents=True, exist_ok=True)
        expected_hashes = {row["source"]["wasm_sha256"] for row in rows if row["source"]["path"] == rel}
        need(len(expected_hashes) == 1, f"{rel} has inconsistent registered Wasm hashes")
        encode_argv = [str(encoder_copy), "encode", str(wat), "-o", str(destination)]
        encoded = run_logged(encode_argv, root, env, out, f"encode-fixture-{index:02d}", command_journal)
        if encoded["exit_code"] != 0:
            (out / "coverage.partial.json").write_text(json.dumps({
                "schema": "wtime-p042-compact-indirect-cache-coverage-v1", "status": "failed", "completed": False,
                "reason": f"fixture encoding failed: {rel}", "encoder_path": str(encoder),
                "encoder_sha256": encoder_sha, "validator_path": str(validator),
                "validator_sha256": validator_sha, "prepare_receipt_path": str(prepare_receipt_path),
                "prepare_receipt_sha256": prepare_receipt_sha, "commands": command_journal,
                "helper_sha256": helper_sha, "source_before": before,
                "source_before_sha256": canonical_sha(before), "rows": []}, indent=2, sort_keys=True) + "\n")
            raise GateError(f"fixture encoding failed: {rel}; no build or row invocation was run")
        need(destination.is_file() and sha(destination) == next(iter(expected_hashes)),
             f"fresh encoder output hash differs from workload registry: {rel}")
        validation_argv = [str(validator_copy), "--profile", "all-phase3-core", "check", str(destination)]
        validated = run_logged(validation_argv, root, env, out, f"validate-fixture-{index:02d}", command_journal)
        validation_stdout = Path(validated["stdout"]["path"]).read_text(encoding="utf-8").strip()
        if validated["exit_code"] != 0 or validation_stdout != "valid":
            (out / "coverage.partial.json").write_text(json.dumps({
                "schema": "wtime-p042-compact-indirect-cache-coverage-v1", "status": "failed", "completed": False,
                "reason": f"fresh fixture failed all-phase3-core validation: {rel}",
                "source_before": before, "source_before_sha256": canonical_sha(before),
                "prepare_receipt_path": str(prepare_receipt_path),
                "prepare_receipt_sha256": prepare_receipt_sha,
                "encoder": {"path": str(encoder), "sha256": encoder_sha},
                "validator": {"path": str(validator), "sha256": validator_sha},
                "license_identity": license_identity,
                "failed_fixture": {"wat_path": str(wat), "wat_sha256": sha(wat),
                    "wasm_path": str(destination), "wasm_sha256": sha(destination),
                    "registered_wasm_sha256": next(iter(expected_hashes)),
                    "encode_command": encoded["argv"], "encode_exit_code": encoded["exit_code"],
                    "encode_stdout": encoded["stdout"], "encode_stderr": encoded["stderr"],
                    "validation_command": validated["argv"], "validation_profile": "all-phase3-core",
                    "validation_stdout_text": validation_stdout,
                    "validation_exit_code": validated["exit_code"],
                    "validation_stdout": validated["stdout"], "validation_stderr": validated["stderr"],
                    "oracle_workload_ids": sorted(row["id"] for row in rows
                        if row["source"]["path"] == rel)},
                "commands": command_journal, "helper_sha256": helper_sha, "rows": []},
                indent=2, sort_keys=True) + "\n", encoding="utf-8")
            raise GateError(f"fresh fixture failed all-phase3-core validation: {rel}")
        fixture_records[rel] = {"wat_path": str(wat), "wat_sha256": sha(wat),
                                "wasm_path": str(destination), "wasm_sha256": sha(destination),
                                "registered_wasm_sha256": next(iter(expected_hashes)),
                                "encode_command": encoded["argv"], "encode_exit_code": encoded["exit_code"],
                                "encode_stdout": encoded["stdout"], "encode_stderr": encoded["stderr"],
                                "validation_command": validated["argv"], "validation_profile": "all-phase3-core",
                                "validation_result": "valid", "validation_exit_code": validated["exit_code"],
                                "validation_stdout": validated["stdout"], "validation_stderr": validated["stderr"],
                                "license_identity": license_identity,
                                "oracle_workload_ids": sorted(row["id"] for row in rows
                                    if row["source"]["path"] == rel)}
    need(all(sha(Path(path)) == digest for path, digest in prepared_fixture_refs.items()) and
         sha(p041_path) == p041_evidence_sha and sha(p039_receipt_path) == p039_receipt_sha and
         sha(packet_manifest) == prepare_receipt["packet_manifest"]["sha256"] and
         sha(check_fixture_provenance) == check_fixture_provenance_ref["sha256"] and
         sha(before_manifest_path) == before_manifest_sha and
         sha(contract_path) == contract_sha_before and sha(encoder) == encoder_sha and
         sha(validator) == validator_sha and sha(encoder_copy) == encoder_sha and
         sha(validator_copy) == validator_sha and sha(prepare_receipt_path) == prepare_receipt_sha and
         sha(check_result_path) == check_result_sha and sha(project_license) == license_sha_before and
         all(sha(license_paths[name]) == digest for name, digest in license_hashes_before.items()) and
         sha(manual_oracles) == license_identity["manual_oracles_sha256"] and sha(provenance) == prepare_receipt["provenance"]["sha256"],
         "contract, P042 receipt, or pinned tools changed before diagnostic build")
    build = run_logged(build_argv, root, env, out, "build-diagnostic", command_journal)
    if build["exit_code"] != 0:
        record = {"schema": "wtime-p042-compact-indirect-cache-coverage-v1", "status": "failed", "completed": False,
                  "reason": "diagnostic build failed; no row command was run", "source_before": before,
                  "source_before_sha256": canonical_sha(before), "encoder_path": str(encoder),
                  "encoder_sha256": encoder_sha, "validator_path": str(validator),
                  "validator_sha256": validator_sha, "prepare_receipt_path": str(prepare_receipt_path),
                  "prepare_receipt_sha256": prepare_receipt_sha, "fixture_records": fixture_records,
                  "driver_sha256": sha(driver_path), "staged_driver_copy_sha256": staged_driver_sha,
                  "contract_sha256": contract_sha_before, "oracle_sources": oracles,
                  "commands": command_journal, "helper_sha256": helper_sha, "rows": []}
        (out / "coverage.partial.json").write_text(json.dumps(record, indent=2, sort_keys=True) + "\n")
        raise GateError("diagnostic build failed; see coverage.partial.json")
    built_binary = build_dir / "debug/wtime-bench"
    need(built_binary.is_file() and os.access(built_binary, os.X_OK), "instrumented diagnostic binary was not produced")
    built_binary_sha = sha(built_binary)
    need(len(command_journal) == 1 + 2 * len(fixture_records) and len(fixture_records) == 17,
         "all 17 scalar fixtures must be encoded and validated before retaining the diagnostic binary")
    need(not binary_copy.exists(), "exclusive retained diagnostic binary already exists")
    shutil.copy2(built_binary, binary_copy)
    need(binary_copy.is_file() and os.access(binary_copy, os.X_OK) and sha(binary_copy) == built_binary_sha,
         "retained diagnostic binary copy differs from reusable-cache build")
    binary_sha = sha(binary_copy)
    module_map = source_map(root, fixture_paths)
    need(module_map == before, "source/fixture inputs changed during diagnostic build")
    diagnostic_rows = []
    row_by_id = {row["id"]: row for row in rows}
    ordered_ids = list(registry["selections"]["scalar"]["workloads"])
    for index, wid in enumerate(ordered_ids):
        row = row_by_id[wid]
        n = n_for(row)
        expected = expected_for(row, n, scalar_oracle, run_oracle)
        source = row["source"]
        wasm = Path(fixture_records[source["path"]]["wasm_path"])
        driver_decl = row.get("driver") or {}
        driver = driver_decl.get("kind", "plain")
        profile = driver_decl.get("profile", "mvp")
        need(driver in {"plain", "host_tick", "table_a", "table_b"}, f"{wid} has unsupported diagnostic driver {driver}")
        need(profile in {"mvp", "stable-core", "all-phase3-core"}, f"{wid} has unsupported diagnostic profile {profile}")
        need(source_map(root, fixture_paths) == before and
             all(sha(Path(path)) == digest for path, digest in prepared_fixture_refs.items()) and
             sha(p041_path) == p041_evidence_sha and sha(p039_receipt_path) == p039_receipt_sha and
             sha(packet_manifest) == prepare_receipt["packet_manifest"]["sha256"] and
             sha(check_fixture_provenance) == check_fixture_provenance_ref["sha256"] and
             sha(before_manifest_path) == before_manifest_sha and
             sha(contract_path) == contract_sha_before and
             sha(prepare_receipt_path) == prepare_receipt_sha and sha(check_result_path) == check_result_sha and sha(repo_driver) == driver_sha_before and
             sha(driver_path) == staged_driver_sha and sha(encoder) == encoder_sha and
             sha(validator) == validator_sha and sha(encoder_copy) == encoder_sha and
             sha(validator_copy) == validator_sha and sha(binary_copy) == binary_sha and
             sha(project_license) == license_sha_before and
             all(sha(license_paths[name]) == digest for name, digest in license_hashes_before.items()) and
             sha(manual_oracles) == manual_oracles_sha_before and
             sha(provenance) == prepare_receipt["provenance"]["sha256"] and
             sha(wasm) == source.get("wasm_sha256"),
             f"source, driver, binary, encoder, contract, or fixture changed before row {wid}")
        argv = [str(binary_copy), "--scalar-cache-diagnostic", str(wasm), wid, str(n), str(expected), "compact", profile, driver]
        cmd = run_logged(argv, root, env, out, f"row-{index:02d}-{wid}", command_journal)
        if cmd["exit_code"] != 0:
            partial = {"schema": "wtime-p042-compact-indirect-cache-coverage-v1", "status": "failed", "completed": False,
                       "reason": f"diagnostic row failed: {wid}", "source_before": before,
                       "source_before_sha256": canonical_sha(before), "binary_sha256": binary_sha,
                       "driver_sha256": sha(driver_path), "staged_driver_copy_sha256": staged_driver_sha,
                       "contract_sha256": contract_sha_before,
                       "oracle_sources": oracles, "commands": command_journal,
                       "helper_sha256": helper_sha, "rows": diagnostic_rows}
            (out / "coverage.partial.json").write_text(json.dumps(partial, indent=2, sort_keys=True) + "\n")
            raise GateError(f"diagnostic row failed: {wid}; see coverage.partial.json")
        stdout = Path(cmd["stdout"]["path"]).read_text(encoding="utf-8").strip()
        lines = [line for line in stdout.splitlines() if line.strip()]
        need(len(lines) == 1, f"{wid} diagnostic stdout must contain exactly one JSON line")
        try:
            obs = json.loads(lines[0])
        except Exception as exc:
            raise GateError(f"{wid} diagnostic output is not JSON: {exc}") from exc
        need(obs.get("schema") == "wtime-compact-scalar-cache-diagnostic-v1" and
             obs.get("diagnostic") is True and obs.get("instrumented") is True and
             obs.get("case") == wid and obs.get("input") == f"i32:{n}" and
             obs.get("expected") == f"i32:{expected}" and obs.get("result") == f"i32:{expected}" and
             obs.get("results") == [f"i32:{expected}", f"i32:{expected}"] and
             obs.get("invocations") == 2 and obs.get("driver") == driver and obs.get("profile") == profile and
             obs.get("requested_mode") == "compact" and obs.get("resolved_mode") == "compact",
             f"{wid} result/oracle/feature-mode diagnostic identity mismatch")
        need(not any(k in obs for k in ("elapsed_ns", "measured_guest_ns", "ns_per_operation", "duration_ns")),
             f"{wid} diagnostic endpoint unexpectedly emitted timing data")
        family = row.get("family")
        role = "primary" if wid in PRIMARY else "protected"
        counters = validate_counter_observation(wid, n, role, family, obs.get("counters"))
        diagnostic_rows.append({"id": wid, "family": family, "role": "primary" if wid in PRIMARY else "protected",
            "fixture": {"path": source["path"], "wat_sha256": source.get("sha256"),
                        "wasm_sha256": source.get("wasm_sha256"), "actual_wasm_sha256": sha(wasm),
                        "wasm_path": str(wasm), "encode_command": fixture_records[source["path"]]["encode_command"]},
            "driver": driver, "profile": profile, "input_i32": n, "oracle_module": row["oracle"]["module"],
            "oracle_module_sha256": oracles[row["oracle"]["module"]],
            "oracle_identity": {"module": row["oracle"]["module"], "function": row["oracle"].get("function"),
                                "independent": row["oracle"].get("independent")}, "expected_i32": expected,
            "result_i32": expected, "results_i32": [expected, expected],
            "oracle_preflight_guest_invocations": ORACLE_PREFLIGHTS_PER_ROW,
            "counted_guest_invocations": COUNTED_INVOCATIONS_PER_ROW,
            "total_guest_invocations": ORACLE_PREFLIGHTS_PER_ROW + COUNTED_INVOCATIONS_PER_ROW,
            "counters": counters,
            "stdout": cmd["stdout"], "stderr": cmd["stderr"], "exit_code": cmd["exit_code"]})
        # Durable partial progress; exclusive run directory prevents overwriting another attempt.
        partial = {"schema": "wtime-p042-compact-indirect-cache-coverage-v1", "status": "running", "completed": False,
                   "source_before": before, "source_before_sha256": canonical_sha(before),
                   "build_cache_path": str(built_binary), "build_cache_binary_sha256": built_binary_sha,
                   "binary_path": str(binary_copy), "binary_sha256": binary_sha,
                   "driver_path": str(repo_driver), "driver_sha256": sha(repo_driver),
                   "staged_driver_sha256": sha(driver_path), "staged_driver_copy_path": str(staged_driver_copy),
                   "staged_driver_copy_sha256": staged_driver_sha, "contract_path": str(contract_path),
                   "contract_sha256": contract_sha_before, "workloads_path": str(root / "tools/wtime-bench/workloads.json"),
                   "workloads_sha256": sha(root / "tools/wtime-bench/workloads.json"),
                   "oracle_sources": oracles, "encoder_path": str(encoder), "encoder_sha256": encoder_sha,
                   "validator_path": str(validator), "validator_sha256": validator_sha,
                   "prepare_receipt_path": str(prepare_receipt_path), "prepare_receipt_sha256": prepare_receipt_sha,
                   "source_roles": roles_before, "license_identity": license_identity,
                   "fixtures": fixture_records, "features": FEATURES, "commands": command_journal,
                   "helper_path": str(source_copy), "helper_sha256": helper_sha, "rows": diagnostic_rows}
        (out / "coverage.partial.json").write_text(json.dumps(partial, indent=2, sort_keys=True) + "\n")
    after = source_map(root, fixture_paths)
    need(after == before and all(sha(Path(path)) == digest for path, digest in prepared_fixture_refs.items()) and
         sha(p041_path) == p041_evidence_sha and sha(p039_receipt_path) == p039_receipt_sha and
         sha(packet_manifest) == prepare_receipt["packet_manifest"]["sha256"] and
         sha(check_fixture_provenance) == check_fixture_provenance_ref["sha256"] and
         sha(before_manifest_path) == before_manifest_sha and
         sha(contract_path) == contract_sha_before and
         sha(prepare_receipt_path) == prepare_receipt_sha and sha(check_result_path) == check_result_sha and sha(repo_driver) == driver_sha_before and
         sha(driver_path) == staged_driver_sha and sha(encoder) == encoder_sha and
         sha(validator) == validator_sha and sha(encoder_copy) == encoder_sha and
         sha(validator_copy) == validator_sha and sha(binary_copy) == binary_sha and
         sha(project_license) == license_sha_before and
         all(sha(license_paths[name]) == digest for name, digest in license_hashes_before.items()) and
         sha(manual_oracles) == manual_oracles_sha_before and
         sha(provenance) == prepare_receipt["provenance"]["sha256"] and
         sha(Path(__file__).resolve()) == helper_sha and sha(staged_driver_copy) == staged_driver_sha,
         "runtime/harness/registry/fixtures/contract/tools changed during diagnostic collection")
    for rel, record in fixture_records.items():
        need(sha(Path(record["wasm_path"])) == record["registered_wasm_sha256"],
             f"encoded diagnostic fixture changed during collection: {rel}")
    need(len(diagnostic_rows) == SCALAR_ROW_COUNT and {r["id"] for r in diagnostic_rows} == set(ordered_ids),
         "diagnostic coverage did not include the complete registered scalar selection")
    need(sum(r["role"] == "primary" for r in diagnostic_rows) == 10 and
         sum(r["role"] == "protected" for r in diagnostic_rows) == 75,
         "diagnostic coverage partition does not match P042 contract")
    primary_coverage = validate_primary_coverage(diagnostic_rows)
    compact_step_primary_ids = primary_coverage["compact_step_primary_ids"]
    cache_hit_primary_ids = primary_coverage["compact_indirect_hit_primary_ids"]
    cache_miss_primary_ids = primary_coverage["compact_indirect_miss_primary_ids"]
    final = {"schema": "wtime-p042-compact-indirect-cache-coverage-v1", "status": "passed", "completed": True,
        "source_before": before, "source_before_sha256": canonical_sha(before),
        "source_after": after, "source_after_sha256": canonical_sha(after),
        "runtime_source_map": runtime_before, "runtime_source_count": len(runtime_before),
        "runtime_source_sha256": canonical_sha(runtime_before),
        "p042_prepare_predecessor_runtime_source_map": predecessor_runtime,
        "p042_prepare_predecessor_runtime_source_count": len(predecessor_runtime),
        "p042_prepare_predecessor_runtime_source_sha256": canonical_sha(predecessor_runtime),
        "p041_evidence": p041_ref,
        "p039_receipt": p039_receipt_ref,
        "p042_fixture_packet_manifest": prepare_receipt["packet_manifest"],
        "p042_fixture_packet_provenance": prepare_receipt["provenance"],
        "p042_fixture_manual_oracles": prepare_receipt["manual_oracles"],
        "p042_prepare_receipt": {"path": str(prepare_receipt_path), "sha256": prepare_receipt_sha,
                                  "bytes": prepare_receipt_path.stat().st_size},
        "p042_durable_fixture_provenance": check_fixture_provenance_ref,
        "p042_source_bound_check": {"path": str(check_result_path), "sha256": sha(check_result_path),
                                    "bytes": check_result_path.stat().st_size},
        "p042_before_manifest": prepare_receipt["before_manifest"],
        "p042_archive_command": prepare_receipt["archive_command"],
        "p042_prepared_fixture_refs": prepared_fixture_refs,
        "p042_source_bound_test_contract": test_contract_ref,
        "passed": True, "source_stable": True,
        "binary": {"path": str(binary_copy), "sha256": binary_sha,
                   "bytes": binary_copy.stat().st_size, "mode": oct(binary_copy.stat().st_mode & 0o7777)},
        "binary_path": str(binary_copy), "binary_sha256": binary_sha,
        "binary_bytes": binary_copy.stat().st_size,
        "build_cache_path": str(built_binary), "build_cache_binary_sha256": built_binary_sha,
        "binary_retained_before_guest_calls": True,
        "commands_completed_before_binary_retention": 1 + 2 * len(fixture_records),
        "build_features": FEATURES, "timing_evidence": False,
        "diagnostic_endpoint": {"schema": "wtime-compact-scalar-cache-diagnostic-v1", "instrumented": True,
                                "source_path": str(repo_driver), "source_sha256": driver_sha_before,
                                "counter_fields": ["compact_steps", "compact_bound_calls", "compact_indirect_hits",
                                                  "compact_indirect_misses", "compact_indirect_evictions"],
                                "invocations_per_row": 2, "same_instance": True, "counter_reset_count_per_row": 1,
                                "counter_runtime_sources": counter_runtime_sources},
        "driver_path": str(repo_driver), "driver_sha256": sha(repo_driver),
        "staged_driver_path": str(driver_path), "staged_driver_sha256": sha(driver_path),
        "staged_driver_copy_path": str(staged_driver_copy), "staged_driver_copy_sha256": staged_driver_sha,
        "contract_path": str(contract_path), "contract_sha256": contract_sha_before,
        "workloads_path": str(root / "tools/wtime-bench/workloads.json"),
        "workloads_sha256": sha(root / "tools/wtime-bench/workloads.json"),
        "oracle_sources": oracles, "encoder_path": str(encoder), "encoder_sha256": encoder_sha,
        "encoder_bytes": encoder.stat().st_size, "validator_path": str(validator),
        "validator_sha256": validator_sha, "validator_bytes": validator.stat().st_size,
        "encoder_copy_path": str(encoder_copy), "encoder_copy_sha256": sha(encoder_copy),
        "encoder_copy_bytes": encoder_copy.stat().st_size,
        "validator_copy_path": str(validator_copy), "validator_copy_sha256": sha(validator_copy),
        "validator_copy_bytes": validator_copy.stat().st_size,
        "prepare_tools": tool_refs,
        "prepare_receipt_path": str(prepare_receipt_path), "prepare_receipt_sha256": prepare_receipt_sha,
        "source_roles": roles_before, "license_identity": license_identity,
        "licenses": {name: {"path": str(path), "sha256": sha(path), "bytes": path.stat().st_size}
                     for name, path in license_paths.items()},
        "fixture_provenance": {"path": str(provenance), "sha256": sha(provenance), "bytes": provenance.stat().st_size},
        "prepared_correctness_fixture_count": len(fixture_receipt_rows),
        "fixtures": fixture_records, "helper_path": str(source_copy), "helper_sha256": helper_sha,
        "command_count": len(command_journal), "commands": command_journal,
        "selected_count": len(diagnostic_rows), "fixture_count": len(fixture_records),
        "oracle_checked_count": SCALAR_ROW_COUNT,
        "primary_count": 10, "protected_count": 75,
        "oracle_preflight_guest_invocations": len(diagnostic_rows) * ORACLE_PREFLIGHTS_PER_ROW,
        "counted_guest_invocations": len(diagnostic_rows) * COUNTED_INVOCATIONS_PER_ROW,
        "total_guest_invocations": len(diagnostic_rows) * (ORACLE_PREFLIGHTS_PER_ROW + COUNTED_INVOCATIONS_PER_ROW),
        "compact_step_primary_ids": compact_step_primary_ids,
        "compact_indirect_hit_primary_ids": cache_hit_primary_ids,
        "compact_indirect_miss_primary_ids": cache_miss_primary_ids,
        "expected_positive_compact_indirect_hit_primary_ids": sorted(EXPECTED_POSITIVE_COMPACT_PRIMARY_IDS),
        "expected_positive_compact_indirect_miss_primary_ids": sorted(EXPECTED_PRIMARY_MISS_IDS),
        "protected_zero_call_controls": [{"id":r["id"],"compact_indirect_hits":r["counters"]["compact_indirect_hits"],"compact_indirect_misses":r["counters"]["compact_indirect_misses"]} for r in diagnostic_rows if r["role"]=="protected" and r["input_i32"]==0 and r["family"] in ("scalar_indirect_calls","scalar_call_ref","scalar_table_mutation")],
        "rows": diagnostic_rows}
    expected_command_count = 1 + 2 * len(fixture_records) + len(diagnostic_rows)
    need(len(command_journal) == expected_command_count and all(c["exit_code"] == 0 for c in command_journal),
         "expected one diagnostic build, encode+validation per unique scalar fixture, and 85 rows with one disposable oracle preflight plus two counted same-instance invocations each (85 preflight + 170 counted = 255 total guest invocations)")
    (out / "coverage.json").write_text(json.dumps(final, indent=2, sort_keys=True) + "\n")
    (out / "coverage.partial.json").unlink()
    (out / "commands.partial.json").unlink()
    print(json.dumps({"status": "passed", "selected": 85, "primary": 10,
                      "protected": 75,
                      "oracle_preflight_guest_invocations": len(diagnostic_rows) * ORACLE_PREFLIGHTS_PER_ROW,
                      "counted_guest_invocations": len(diagnostic_rows) * COUNTED_INVOCATIONS_PER_ROW,
                      "total_guest_invocations": len(diagnostic_rows) * (ORACLE_PREFLIGHTS_PER_ROW + COUNTED_INVOCATIONS_PER_ROW),
                      "commands": len(command_journal),
                      "output": str(out / "coverage.json"),
                      "binary_sha256": binary_sha}, indent=2))
    return 0

if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except GateError as error:
        print(f"P042 coverage collection stopped: {error}", file=sys.stderr)
        raise SystemExit(1)
