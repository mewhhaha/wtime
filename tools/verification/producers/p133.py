#!/usr/bin/env python3
"""Freeze the P133 Register predecessor and validate bounded-cache evidence."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import sys
from typing import Any

ROOT = Path(__file__).resolve().parents[3]
BENCH = ROOT / "tools/wtime-bench"
if str(BENCH) not in sys.path:
    sys.path.insert(0, str(BENCH))

PREDECESSOR = "target/performance/P133-before"
PREDECESSOR_COMMIT = "12dc7e7db2b0130fd81efa67939008cccc2013c2"
PREDECESSOR_MANIFEST_SHA256 = "076bedce1f7db694739398407116aad8272a92394f955ee0ac0689bf99ad1ad1"
PROTOCOL_SHA256 = "ac7b9ce0d12a0a17b38acbfb964f8704c8f85c1927d89385a055b612f77b848c"
P001_MANIFEST_SHA256 = "7349596fc806c96c74cb9c40f1c08402589c1683660b0b152c713264f5808472"
P001_BINARY_SHA256 = "735e595f3a625da89401fdee298781b752fbe071e3d3d793df196c9e68b8e498"
P132_COMPLETION_SHA256 = "cb25eddad3763398838fe254c85670d672d08b6c4640ec3f98b882f2f02f6bf8"
P132_EVIDENCE_SHA256 = "8c5fccf8198bb7a79ba730957998cf6465d379d6e47497a69d3b9130eeb692f9"
P132_FINAL_EVIDENCE = "target/performance/P132/shared-finalization/attempt-0003/evidence.json"
P132_ARCHITECTURE_SHA256 = "8ab897646c2e813337a1ef6f6c628726bdb155a21ae85724d73fea66ff62e690"
P132_CONTRACT_SHA256 = "da41e2b7f8853700355717a60e4301c8b29bdf3c69c8bca74ede9f35aa3aa53d"

P133_TESTS = (
    "p133_cold_threshold_and_single_publish",
    "p133_concurrent_duplicate_uses_one_unlocked_builder",
    "p133_decline_poison_foreign_and_aborted_lease_fail_closed",
    "p133_fifo_eviction_pinned_lifetime_and_budget_churn",
)
DEFAULT_TEST = "p132_module_preparation_remains_policy_neutral"
CORE_CRATES = ("wtime-runtime", "wtime-types", "wtime-decode", "wtime-validate")
ROOT_SOURCES = {"Cargo.toml", "Cargo.lock", "rust-toolchain.toml"}
MAX_BUILD_BYTES = 30_000_000
MAX_ENTRIES = 64
MAX_WORK = 100_000
MAX_BYTES = 1_048_576


class P133Error(ValueError):
    """P133 evidence is absent, stale, malformed, or inconsistent."""


def need(condition: bool, message: str) -> None:
    if not condition:
        raise P133Error(message)


def sha(path: Path) -> str:
    digest = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def canonical_sha(value: Any) -> str:
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":"),
                                   allow_nan=False).encode("utf-8")).hexdigest()


def ref(path: Path) -> dict[str, Any]:
    path = Path(path).resolve(strict=True)
    return {"path": str(path), "sha256": sha(path), "bytes": path.stat().st_size}


def write_new(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("x", encoding="utf-8") as stream:
        json.dump(value, stream, indent=2, sort_keys=True, allow_nan=False)
        stream.write("\n")


def start_output(path: Path) -> Path:
    path = path.resolve()
    store = (ROOT / "target/verification").resolve()
    need(path.is_relative_to(store), "P133 output must stay under target/verification")
    path.mkdir(parents=True, exist_ok=False)
    with (path / "helper.executed.py").open("xb") as stream:
        stream.write(Path(__file__).read_bytes())
    write_new(path / "invocation.json", {"argv": list(sys.argv), "cwd": str(Path.cwd().resolve())})
    return path


def runtime_source_map(manifest: dict[str, Any]) -> dict[str, str]:
    prefixes = tuple(f"crates/{crate}/" for crate in CORE_CRATES)
    sources: dict[str, str] = {}
    for row in manifest["files"].values():
        source_path = row.get("source_path", "")
        if (row.get("role") == "source" and
                (source_path in ROOT_SOURCES or
                 any(source_path == f"crates/{crate}/Cargo.toml" for crate in CORE_CRATES) or
                 (source_path.endswith(".rs") and source_path.startswith(prefixes)))):
            sources[source_path] = row["sha256"]
    return dict(sorted(sources.items()))


def load_json(path: Path, label: str) -> dict[str, Any]:
    path = Path(path).resolve(strict=True)
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise P133Error(f"{label} is not valid JSON: {exc}") from exc
    need(isinstance(value, dict), f"{label} must be a JSON object")
    return value


def validate_p132() -> dict[str, Any]:
    completion_path = ROOT / "docs/benchmarks/performance/P132-completion.json"
    evidence_path = ROOT / "docs/benchmarks/performance/P132-evidence.json"
    architecture_path = ROOT / "docs/architecture/register-preparation.md"
    final_path = ROOT / P132_FINAL_EVIDENCE
    need(sha(completion_path) == P132_COMPLETION_SHA256,
         "P132 completion record identity changed")
    need(sha(evidence_path) == P132_EVIDENCE_SHA256 and sha(final_path) == P132_EVIDENCE_SHA256,
         "P132 evidence identity changed")
    need(sha(architecture_path) == P132_ARCHITECTURE_SHA256,
         "P132 preparation architecture note identity changed")
    completion = load_json(completion_path, "P132 completion")
    evidence = load_json(evidence_path, "P132 evidence")
    contract_path = ROOT / "docs/benchmarks/performance/P132-verification.json"
    contract = load_json(contract_path, "P132 verification contract")
    need(completion.get("task") == "P132" and completion.get("status") == "done"
         and completion.get("outcome") == "contract_only",
         "P132 is not a durable completed correctness foundation")
    need(completion.get("artifact_hashes", {}).get(str(final_path)) == P132_EVIDENCE_SHA256,
         "P132 completion does not bind its final evidence")
    required_phases = contract.get("completion", {}).get("required_phases")
    need(contract.get("task") == "P132" and sha(contract_path) == P132_CONTRACT_SHA256
         and required_phases == ["freeze", "verify"]
         and completion.get("outcome") == contract.get("completion", {}).get("outcome"),
         "P132 completion does not match its pinned verification contract")
    contract_ref = evidence.get("contract", {})
    need(evidence.get("schema") == "wtime-verification-evidence-v1"
         and evidence.get("task") == "P132" and evidence.get("status") == "passed"
         and evidence.get("passed") is True and evidence.get("synthetic") is False
         and evidence.get("candidate_promoted") is False
         and evidence.get("decision") == "contract_only"
         and evidence.get("outcome") == contract.get("completion", {}).get("outcome")
         and set(evidence.get("phases", {})) == set(required_phases)
         and contract_ref.get("path") == str(contract_path.resolve())
         and contract_ref.get("sha256") == P132_CONTRACT_SHA256
         and contract_ref.get("bytes") == contract_path.stat().st_size,
         "P132 evidence is not a passed correctness-only foundation")
    return {
        "completion": ref(completion_path), "evidence": ref(evidence_path),
        "final_evidence": ref(final_path), "architecture_note": ref(architecture_path),
        "verification_contract": ref(contract_path),
        "outcome": "contract_only",
    }


def freeze(output: Path) -> dict[str, Any]:
    output = start_output(output)
    import baseline

    predecessor = (ROOT / PREDECESSOR).resolve(strict=True)
    manifest_path = predecessor / "manifest.json"
    need(sha(manifest_path) == PREDECESSOR_MANIFEST_SHA256,
         "P133 immediate predecessor manifest identity changed")
    manifest = baseline.verify_archive(predecessor)
    need(manifest.get("repository_commit") == PREDECESSOR_COMMIT
         and manifest.get("mode") == "register",
         "P133 immediate predecessor is not the pinned Register archive")
    policy = manifest.get("execution_policy", {})
    need(policy.get("requested_mode") == "register" and policy.get("resolved_mode") == "register"
         and policy.get("protocol_modes") == ["decoded", "portable", "register"]
         and policy.get("protocol_sha256") == PROTOCOL_SHA256,
         "P133 predecessor protocol is not exactly Register")
    source_map = runtime_source_map(manifest)
    need(ROOT_SOURCES <= set(source_map), "P133 predecessor omits root Cargo/toolchain inputs")
    for crate in CORE_CRATES:
        need(f"crates/{crate}/Cargo.toml" in source_map and any(
            path.startswith(f"crates/{crate}/") and path.endswith(".rs") for path in source_map),
            f"P133 predecessor omits {crate} source files")

    p001 = (ROOT / "target/performance/baseline").resolve(strict=True)
    p001_manifest = baseline.verify_archive(p001)
    need(sha(p001 / "manifest.json") == P001_MANIFEST_SHA256
         and sha(p001 / "binary/wtime-bench") == P001_BINARY_SHA256,
         "immutable P001 archive identity changed")
    need(p001_manifest.get("smoke", {}).get("correctness_checked") is True
         and p001_manifest.get("smoke", {}).get("returncode") == 0,
         "immutable P001 archive has no successful correctness smoke")
    p132 = validate_p132()
    runtime_doc = {"runtime_source_map": source_map,
                   "runtime_source_map_sha256": canonical_sha(source_map)}
    runtime_path = output / "runtime.json"
    write_new(runtime_path, runtime_doc)
    report = {
        "schema": "wtime-p133-freeze-v1", "status": "passed", "timing_evidence": False,
        "predecessor": ref(manifest_path),
        "predecessor_manifest_sha256_pinned": PREDECESSOR_MANIFEST_SHA256,
        "predecessor_mode": "register", "predecessor_commit": PREDECESSOR_COMMIT,
        "protocol_sha256": PROTOCOL_SHA256,
        "runtime_source_map": source_map,
        "runtime_source_map_sha256": canonical_sha(source_map),
        "runtime_source_file_count": len(source_map), "runtime_map": ref(runtime_path),
        "p001": {"manifest": ref(p001 / "manifest.json"),
                 "binary": ref(p001 / "binary/wtime-bench"),
                 "manifest_sha256_pinned": P001_MANIFEST_SHA256,
                 "binary_sha256_pinned": P001_BINARY_SHA256,
                 "correctness_checked": True,
                 "max_work": MAX_WORK, "max_metadata_bytes": MAX_BYTES,
                 "max_cache_entries": MAX_ENTRIES},
        "p132": p132,
    }
    write_new(output / "report.json", report)
    return report


def require_tests(path: Path, names: tuple[str, ...], label: str) -> None:
    text = path.read_text(encoding="utf-8", errors="replace")
    for name in names:
        need(re.search(rf"^test\s+[^\n]*{re.escape(name)}\s+\.\.\. ok$",
                       text, re.MULTILINE) is not None,
             f"{label} log lacks passing named case {name}")


def validate_case_records(path: Path) -> dict[str, dict[str, Any]]:
    records: dict[str, dict[str, Any]] = {}
    for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        if not line.startswith("P133_CASE "):
            continue
        try:
            record = json.loads(line[len("P133_CASE "):])
        except json.JSONDecodeError as exc:
            raise P133Error(f"malformed P133_CASE record: {exc}") from exc
        need(isinstance(record, dict), "P133_CASE record must be a JSON object")
        name = record.get("name")
        need(name in P133_TESTS, f"unexpected P133_CASE name: {name!r}")
        need(name not in records, f"duplicate P133_CASE record for {name}")
        need(record.get("passed") is True, f"P133 case record did not pass: {name}")
        records[name] = record
    need(set(records) == set(P133_TESTS),
         f"P133 case records differ from required cases: {sorted(records)}")
    expected = {
        P133_TESTS[0]: {
            "builders": 1, "threshold": 3, "same_artifact": True,
            "decoded_policy_and_fuel_preserved": True,
        },
        P133_TESTS[1]: {
            "builders": 1, "duplicate_fallbacks": 2,
            "cache_lock_available": True, "canonical_lock_available": True,
        },
        P133_TESTS[2]: {
            "decline_builders": 2, "cooldown_entries": 128,
            "foreign_owner_refused": True, "invalid_refused": True,
            "abort_cleared_lease": True, "poison_fail_closed": True,
            "counter_saturates": True,
        },
        P133_TESTS[3]: {
            "max_entries": MAX_ENTRIES, "fixture_bodies": 66,
            "last_drop_refunds_only_bytes": True, "fifo_preserved": True,
            "metadata_refusal": True, "monotonic_work": True,
            "negative_churn_bounded": True,
        },
    }
    for name, fields in expected.items():
        record = records[name]
        for field, expected_value in fields.items():
            actual = record.get(field)
            need(type(actual) is type(expected_value) and actual == expected_value,
                 f"P133 {name} has invalid {field}: {actual!r}")
    pinned_bytes = records[P133_TESTS[3]].get("pinned_bytes")
    need(type(pinned_bytes) is int and pinned_bytes > 0,
         "P133 pinned-byte observation must be a positive integer")
    return records


def check_build(path: Path, label: str, mode: str, features: list[str]) -> dict[str, Any]:
    record = load_json(path, f"{label} build budget")
    need(record.get("mode") == mode and record.get("features") == features,
         f"{label} build budget has the wrong feature configuration")
    need(record.get("source_stable") is True and record.get("budget_pass") is True
         and record.get("default_dependency_closure_pass") is True,
         f"{label} build budget failed source, hard-budget, or default-dependency policy")
    byte_count = record.get("stripped_bytes")
    need(type(byte_count) is int and 0 < byte_count <= MAX_BUILD_BYTES,
         f"{label} build budget has an invalid or over-limit stripped binary size")
    return {"artifact": ref(Path(path)), "mode": mode, "features": features,
            "stripped_bytes": byte_count, "budget_pass": True,
            "default_dependency_closure_pass": True}


def verify(output: Path, register_log: Path, register_stderr: Path,
           combined_log: Path, default_log: Path, default_build: Path,
           register_build: Path, freeze_report: Path) -> dict[str, Any]:
    output = start_output(output)
    frozen = load_json(freeze_report, "P133 freeze report")
    need(frozen.get("schema") == "wtime-p133-freeze-v1" and frozen.get("status") == "passed"
         and frozen.get("timing_evidence") is False,
         "P133 freeze report is invalid")
    need(frozen.get("predecessor_commit") == PREDECESSOR_COMMIT
         and frozen.get("predecessor_manifest_sha256_pinned") == PREDECESSOR_MANIFEST_SHA256
         and frozen.get("predecessor_mode") == "register"
         and frozen.get("protocol_sha256") == PROTOCOL_SHA256,
         "P133 freeze does not bind its exact immediate predecessor")
    need(frozen.get("runtime_source_map_sha256") == canonical_sha(frozen.get("runtime_source_map")),
         "P133 frozen source map hash is invalid")
    runtime_ref = frozen.get("runtime_map", {})
    runtime_path = Path(runtime_ref.get("path", "")).resolve(strict=True)
    need(runtime_path.is_relative_to(ROOT / "target/verification") and sha(runtime_path) == runtime_ref.get("sha256"),
         "P133 frozen runtime map artifact is invalid")
    runtime_doc = load_json(runtime_path, "P133 frozen runtime map")
    need(runtime_doc.get("runtime_source_map") == frozen.get("runtime_source_map"),
         "P133 runtime map differs from freeze report")

    register_log, register_stderr = Path(register_log).resolve(strict=True), Path(register_stderr).resolve(strict=True)
    combined_log, default_log = Path(combined_log).resolve(strict=True), Path(default_log).resolve(strict=True)
    need(register_stderr.is_relative_to(ROOT / "target/verification"),
         "P133 case records must come from a retained verification stderr log")
    require_tests(register_log, P133_TESTS, "full Register/reference")
    require_tests(combined_log, P133_TESTS, "full combined Compact/Register/reference")
    require_tests(default_log, (DEFAULT_TEST,), "default public policy")
    cases = validate_case_records(register_stderr)
    builds = {
        "default": check_build(default_build, "default", "default", []),
        "register": check_build(register_build, "register", "optional-feature",
                                 ["wtime-runtime/register-executor"]),
    }
    report = {
        "schema": "wtime-p133-verification-v1", "status": "passed",
        "timing_evidence": False, "performance_candidate": False,
        "decision": "contract_only",
        "limitations": ["Bounded lazy Register preparation-cache semantics and build/dependency budgets only; no timing or performance promotion is claimed."],
        "settings": {"register_features": ["register-executor", "execution-reference"],
                     "combined_features": ["compact-executor", "register-executor", "execution-reference"],
                     "default_features": [], "immediate_predecessor": "register"},
        "limits": {"cache_entries": MAX_ENTRIES, "attempted_work": MAX_WORK,
                   "retained_metadata_bytes": MAX_BYTES},
        "counts": {"p133_named_cases": len(P133_TESTS), "default_named_cases": 1},
        "case_records": cases,
        "freeze_report": ref(Path(freeze_report)),
        "runtime_source_map_sha256": frozen["runtime_source_map_sha256"],
        "test_logs": {"register": ref(register_log), "register_stderr": ref(register_stderr),
                      "combined": ref(combined_log), "default": ref(default_log)},
        "build_budgets": builds,
    }
    write_new(output / "report.json", report)
    return report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="phase", required=True)
    freeze_parser = subparsers.add_parser("freeze")
    freeze_parser.add_argument("--output", type=Path, required=True)
    verify_parser = subparsers.add_parser("verify")
    verify_parser.add_argument("--output", type=Path, required=True)
    verify_parser.add_argument("--register-log", type=Path, required=True)
    verify_parser.add_argument("--register-stderr", type=Path, required=True)
    verify_parser.add_argument("--combined-log", type=Path, required=True)
    verify_parser.add_argument("--default-log", type=Path, required=True)
    verify_parser.add_argument("--default-build", type=Path, required=True)
    verify_parser.add_argument("--register-build", type=Path, required=True)
    verify_parser.add_argument("--freeze-report", type=Path, required=True)
    args = parser.parse_args()
    try:
        if args.phase == "freeze":
            result = freeze(args.output)
        else:
            result = verify(args.output, args.register_log, args.register_stderr,
                            args.combined_log, args.default_log, args.default_build,
                            args.register_build, args.freeze_report)
        print(json.dumps({"status": result["status"], "schema": result["schema"]}, sort_keys=True))
        return 0
    except (OSError, ValueError, json.JSONDecodeError) as exc:
        print(f"P133 verification failed: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
