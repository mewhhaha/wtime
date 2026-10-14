#!/usr/bin/env python3
"""Freeze the P134 Register predecessor and validate lazy-entry evidence."""
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

PREDECESSOR = "target/performance/P134-before"
PREDECESSOR_COMMIT = "4ed0c0b1c4e9f905db1948d256d093551329f0ff"
PREDECESSOR_MANIFEST_SHA256 = "c07542121418df9ff55d1e933dfb013b262befd53b1e5c557a5b9561cbdad755"
PROTOCOL_SHA256 = "ac7b9ce0d12a0a17b38acbfb964f8704c8f85c1927d89385a055b612f77b848c"
P001_MANIFEST_SHA256 = "7349596fc806c96c74cb9c40f1c08402589c1683660b0b152c713264f5808472"
P001_BINARY_SHA256 = "735e595f3a625da89401fdee298781b752fbe071e3d3d793df196c9e68b8e498"
P133_COMPLETION_SHA256 = "f7a9a1c7888434db84492f1ee921cc70e52aa8b39e39e3fcfe10a2479901add0"
P133_EVIDENCE_SHA256 = "a52be75954ba51759c88cfa10971136885947db484304ce17a6e737e878284e7"
P133_FINAL_EVIDENCE = "target/performance/P133/shared-finalization/attempt-0002/evidence.json"
P133_ARCHITECTURE_SHA256 = "01ca2597e01405af175aeea0567c61f8257df3485a0de9b5e3ac2a5396f5161f"
P133_CONTRACT_SHA256 = "99b1c35b1c0e95af52b96a954325d0cbcdc56b72680a166f89b599c7d1a268dc"

P133_TESTS = (
    "p133_cold_threshold_and_single_publish",
    "p133_concurrent_duplicate_uses_one_unlocked_builder",
    "p133_decline_poison_foreign_and_aborted_lease_fail_closed",
    "p133_fifo_eviction_pinned_lifetime_and_budget_churn",
)
P065_TESTS = (
    "p065_outer_reference_survives_reentrant_collection",
    "p065_capture_retains_numeric_state_and_source_cursor",
    "p065_cross_module_exact_suspend_and_resource_controls",
    "p065_reference_and_vector_frames_decline",
    "p065_exception_unwind_preserves_numeric_payload",
    "p065_root_barrier_recipes_are_complete",
)
P134_TESTS = (
    "p134_cold_hot_entry_threshold_and_diagnostics",
    "p134_shared_module_concurrency_and_store_ownership",
    "p134_host_effect_fuel_and_foreign_refusal",
    "p134_eviction_pin_and_continuation_resume_without_reentry",
    "p134_activation_pin_survives_eviction",
)
FULL_TESTS = P134_TESTS + P133_TESTS + P065_TESTS
DEFAULT_TEST = "p132_module_preparation_remains_policy_neutral"
CORE_CRATES = ("wtime-runtime", "wtime-types", "wtime-decode", "wtime-validate")
ROOT_SOURCES = {"Cargo.toml", "Cargo.lock", "rust-toolchain.toml"}
MAX_BUILD_BYTES = 30_000_000
MAX_WORK = 100_000
MAX_METADATA_BYTES = 1_048_576
MAX_CACHE_ENTRIES = 64


class P134Error(ValueError):
    """P134 evidence is absent, stale, malformed, or inconsistent."""


def need(condition: bool, message: str) -> None:
    if not condition:
        raise P134Error(message)


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
    need(path.is_relative_to(store), "P134 output must stay under target/verification")
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
        raise P134Error(f"{label} is not valid JSON: {exc}") from exc
    need(isinstance(value, dict), f"{label} must be a JSON object")
    return value


def validate_p133() -> dict[str, Any]:
    completion_path = ROOT / "docs/benchmarks/performance/P133-completion.json"
    evidence_path = ROOT / "docs/benchmarks/performance/P133-evidence.json"
    architecture_path = ROOT / "docs/architecture/register-cache.md"
    contract_path = ROOT / "docs/benchmarks/performance/P133-verification.json"
    final_path = ROOT / P133_FINAL_EVIDENCE
    need(sha(completion_path) == P133_COMPLETION_SHA256,
         "P133 completion record identity changed")
    need(sha(evidence_path) == P133_EVIDENCE_SHA256 and sha(final_path) == P133_EVIDENCE_SHA256,
         "P133 evidence identity changed")
    need(sha(architecture_path) == P133_ARCHITECTURE_SHA256,
         "P133 cache architecture note identity changed")
    need(sha(contract_path) == P133_CONTRACT_SHA256,
         "P133 verification contract identity changed")
    completion = load_json(completion_path, "P133 completion")
    evidence = load_json(evidence_path, "P133 evidence")
    contract = load_json(contract_path, "P133 verification contract")
    need(completion.get("task") == "P133" and completion.get("status") == "done"
         and completion.get("outcome") == "contract_only",
         "P133 is not a durable completed cache foundation")
    need(completion.get("artifact_hashes", {}).get(str(final_path)) == P133_EVIDENCE_SHA256,
         "P133 completion does not bind its final evidence")
    required_phases = contract.get("completion", {}).get("required_phases")
    contract_outcome = contract.get("completion", {}).get("outcome")
    need(contract.get("task") == "P133" and required_phases == ["freeze", "verify"]
         and contract_outcome == "contract_only",
         "P133 pinned contract has the wrong task, outcome, or phases")
    contract_ref = evidence.get("contract", {})
    need(evidence.get("schema") == "wtime-verification-evidence-v1"
         and evidence.get("task") == "P133" and evidence.get("status") == "passed"
         and evidence.get("passed") is True and evidence.get("synthetic") is False
         and evidence.get("candidate_promoted") is False
         and evidence.get("decision") == "contract_only"
         and evidence.get("outcome") == contract_outcome
         and set(evidence.get("phases", {})) == set(required_phases)
         and contract_ref.get("path") == str(contract_path.resolve())
         and contract_ref.get("sha256") == P133_CONTRACT_SHA256
         and contract_ref.get("bytes") == contract_path.stat().st_size,
         "P133 evidence is not a passed freeze/verify-only foundation")
    return {
        "completion": ref(completion_path), "evidence": ref(evidence_path),
        "final_evidence": ref(final_path), "architecture_note": ref(architecture_path),
        "verification_contract": ref(contract_path), "outcome": contract_outcome,
        "required_phases": required_phases,
    }


def freeze(output: Path) -> dict[str, Any]:
    output = start_output(output)
    import baseline

    predecessor = (ROOT / PREDECESSOR).resolve(strict=True)
    manifest_path = predecessor / "manifest.json"
    need(sha(manifest_path) == PREDECESSOR_MANIFEST_SHA256,
         "P134 immediate predecessor manifest identity changed")
    manifest = baseline.verify_archive(predecessor)
    need(manifest.get("repository_commit") == PREDECESSOR_COMMIT
         and manifest.get("mode") == "register",
         "P134 immediate predecessor is not the pinned Register archive")
    policy = manifest.get("execution_policy", {})
    need(policy.get("requested_mode") == "register" and policy.get("resolved_mode") == "register"
         and policy.get("protocol_modes") == ["decoded", "portable", "register"]
         and policy.get("protocol_sha256") == PROTOCOL_SHA256,
         "P134 predecessor protocol is not exactly Register")
    source_map = runtime_source_map(manifest)
    need(ROOT_SOURCES <= set(source_map), "P134 predecessor omits root Cargo/toolchain inputs")
    for crate in CORE_CRATES:
        need(f"crates/{crate}/Cargo.toml" in source_map and any(
            path.startswith(f"crates/{crate}/") and path.endswith(".rs") for path in source_map),
            f"P134 predecessor omits {crate} source files")

    p001 = (ROOT / "target/performance/baseline").resolve(strict=True)
    p001_manifest = baseline.verify_archive(p001)
    need(sha(p001 / "manifest.json") == P001_MANIFEST_SHA256
         and sha(p001 / "binary/wtime-bench") == P001_BINARY_SHA256,
         "immutable P001 archive identity changed")
    need(p001_manifest.get("smoke", {}).get("correctness_checked") is True
         and p001_manifest.get("smoke", {}).get("returncode") == 0,
         "immutable P001 archive has no successful correctness smoke")
    p133 = validate_p133()
    runtime_doc = {"runtime_source_map": source_map,
                   "runtime_source_map_sha256": canonical_sha(source_map)}
    runtime_path = output / "runtime.json"
    write_new(runtime_path, runtime_doc)
    report = {
        "schema": "wtime-p134-freeze-v1", "status": "passed", "timing_evidence": False,
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
                 "max_work": MAX_WORK, "max_metadata_bytes": MAX_METADATA_BYTES,
                 "max_cache_entries": MAX_CACHE_ENTRIES},
        "p133": p133,
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
        if not line.startswith("P134_CASE "):
            continue
        try:
            record = json.loads(line[len("P134_CASE "):])
        except json.JSONDecodeError as exc:
            raise P134Error(f"malformed P134_CASE record: {exc}") from exc
        need(isinstance(record, dict), "P134_CASE record must be a JSON object")
        name = record.get("name")
        need(name in P134_TESTS, f"unexpected P134_CASE name: {name!r}")
        need(name not in records, f"duplicate P134_CASE record for {name}")
        need(record.get("passed") is True, f"P134 case record did not pass: {name}")
        records[name] = record
    need(set(records) == set(P134_TESTS),
         f"P134 case records differ from required cases: {sorted(records)}")
    expected = {
        P134_TESTS[0]: {
            "threshold": 3, "cold_fallback_frames": 2, "hot_dispatches": 6,
            "fuel_prefixes": 4, "zero_threshold_refused": True,
        },
        P134_TESTS[1]: {
            "stores": 2, "calls": 40, "optimized_path_ran": True,
            "isolated_memory": True, "foreign_store_refused": True,
        },
        P134_TESTS[2]: {
            "fuel_prefixes": 12, "policies": 4, "callback_once": True,
            "reentry_gc_and_memory_preserved": True, "cancellation_preserved": True,
        },
        P134_TESTS[3]: {
            "capture_resumes": 24, "exact_and_nominal": True,
            "source_state_preserved": True, "resume_dispatches": 0,
            "one_shot_preserved": True,
        },
        P134_TESTS[4]: {
            "eager_artifact_absent": True, "builders_per_body": 1,
            "shared_concurrent_builders": 1, "resume_entry_lookups": 0,
            "dispatch_arc_count": 1, "last_drop_refunds_only_bytes": True,
            "foreign_owner_refused": True,
        },
    }
    for name, fields in expected.items():
        record = records[name]
        for field, expected_value in fields.items():
            actual = record.get(field)
            need(type(actual) is type(expected_value) and actual == expected_value,
                 f"P134 {name} has invalid {field}: {actual!r}")
    pinned_bytes = records[P134_TESTS[4]].get("pinned_bytes")
    need(type(pinned_bytes) is int and 0 < pinned_bytes <= MAX_METADATA_BYTES,
         "P134 pinned-byte observation is outside the positive cache metadata bound")
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
    frozen = load_json(freeze_report, "P134 freeze report")
    need(frozen.get("schema") == "wtime-p134-freeze-v1" and frozen.get("status") == "passed"
         and frozen.get("timing_evidence") is False,
         "P134 freeze report is invalid")
    need(frozen.get("predecessor_commit") == PREDECESSOR_COMMIT
         and frozen.get("predecessor_manifest_sha256_pinned") == PREDECESSOR_MANIFEST_SHA256
         and frozen.get("predecessor_mode") == "register"
         and frozen.get("protocol_sha256") == PROTOCOL_SHA256,
         "P134 freeze does not bind its exact immediate predecessor")
    need(frozen.get("runtime_source_map_sha256") == canonical_sha(frozen.get("runtime_source_map")),
         "P134 frozen source map hash is invalid")
    runtime_ref = frozen.get("runtime_map", {})
    runtime_path = Path(runtime_ref.get("path", "")).resolve(strict=True)
    need(runtime_path.is_relative_to(ROOT / "target/verification")
         and sha(runtime_path) == runtime_ref.get("sha256"),
         "P134 frozen runtime map artifact is invalid")
    runtime_doc = load_json(runtime_path, "P134 frozen runtime map")
    need(runtime_doc.get("runtime_source_map") == frozen.get("runtime_source_map"),
         "P134 runtime map differs from freeze report")

    register_log = Path(register_log).resolve(strict=True)
    register_stderr = Path(register_stderr).resolve(strict=True)
    combined_log = Path(combined_log).resolve(strict=True)
    default_log = Path(default_log).resolve(strict=True)
    need(register_stderr.is_relative_to(ROOT / "target/verification"),
         "P134 case records must come from a retained verification stderr log")
    require_tests(register_log, FULL_TESTS, "full Register/reference")
    require_tests(combined_log, FULL_TESTS, "full combined Compact/Register/reference")
    require_tests(default_log, (DEFAULT_TEST,), "default public policy")
    cases = validate_case_records(register_stderr)
    builds = {
        "default": check_build(default_build, "default", "default", []),
        "register": check_build(register_build, "register", "optional-feature",
                                 ["wtime-runtime/register-executor"]),
    }
    report = {
        "schema": "wtime-p134-verification-v1", "status": "passed",
        "timing_evidence": False, "performance_candidate": False,
        "decision": "contract_only",
        "limitations": ["Opt-in lazy Register entry, cache integration, frame/pin and continuation semantics, plus default/optional build budgets only; no timing or performance promotion is claimed."],
        "settings": {"register_features": ["register-executor", "execution-reference"],
                     "combined_features": ["compact-executor", "register-executor", "execution-reference"],
                     "default_features": [], "immediate_predecessor": "register"},
        "limits": {"cache_entries": MAX_CACHE_ENTRIES, "attempted_work": MAX_WORK,
                   "retained_metadata_bytes": MAX_METADATA_BYTES},
        "counts": {"p134_named_cases": len(P134_TESTS),
                   "p133_regression_named_cases": len(P133_TESTS),
                   "p065_regression_named_cases": len(P065_TESTS),
                   "default_named_cases": 1},
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
        print(f"P134 verification failed: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
