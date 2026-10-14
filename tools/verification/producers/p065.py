#!/usr/bin/env python3
"""Freeze P065's predecessor and summarize already-executed semantic evidence."""
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

PREDECESSOR = "target/performance/P065-before"
PREDECESSOR_COMMIT = "602d2c957ae7534fce2cc93ef07ce7b27011fa85"
PREDECESSOR_MANIFEST_SHA256 = "ac0778f1dc2138d71a175da3e04d50c7a9458975b1bb3fb74436ada442e8b529"
PROTOCOL_SHA256 = "ac7b9ce0d12a0a17b38acbfb964f8704c8f85c1927d89385a055b612f77b848c"
P001_MANIFEST_SHA256 = "7349596fc806c96c74cb9c40f1c08402589c1683660b0b152c713264f5808472"
P001_BINARY_SHA256 = "735e595f3a625da89401fdee298781b752fbe071e3d3d793df196c9e68b8e498"

COMPLETED = {
    "P064": {
        "completion_sha256": "a4444c92d6c80833fa2825f7ed3cb65cf5a1723c71ad481fc900d810a3fac9cc",
        "evidence_sha256": "2c112dae8233a37e0be25acfe7526bea40e3e771f61f0d18bd540e2cdf480b89",
        "final_evidence": "target/performance/P064/shared-finalization/attempt-0002/evidence.json",
    },
    "P131": {
        "completion_sha256": "ec2d44410b640798665329668508edf260cc50ea2126cbc7e5da3ad9873f3041",
        "evidence_sha256": "b1d0e3e58c7cd0891f9772fcff71f16e26d314029c27d98db1705484b2a2a1fc",
        "final_evidence": "target/performance/P131/shared-finalization/attempt-0001/evidence.json",
    },
}

P065_TESTS = (
    "p065_outer_reference_survives_reentrant_collection",
    "p065_capture_retains_numeric_state_and_source_cursor",
    "p065_cross_module_exact_suspend_and_resource_controls",
    "p065_reference_and_vector_frames_decline",
    "p065_exception_unwind_preserves_numeric_payload",
    "p065_root_barrier_recipes_are_complete",
)
DEFAULT_TEST = "p016_default_and_portable_keep_decoded"
REGISTER_TESTS = (
    "p131_prefix_values_and_source_boundaries_match",
    "p131_open_labels_and_memory_prefixes_match",
    "p131_invalid_barriers_and_budgets_decline",
    "p057_numeric_loop_has_bounded_typed_slots_and_source_boundaries",
    "p057_unsupported_types_and_effects_decline_without_guest_work",
    "p057_bad_metadata_and_aggregate_budgets_are_transactional",
    "p057_register_policy_stays_unavailable_until_execution_lands",
    "p058_local_numeric_slots_match_every_decoded_prefix",
    "p058_multivalue_edges_and_loop_visits_match_decoded_prefixes",
    "p058_declines_invalid_joins_and_preserves_shared_budgets",
    "p129_zero_skip_preserves_first_visit",
    "p129_later_loop_visit_keeps_exact_state_and_fuel",
    "p129_invalid_visit_bound_and_unreached_request_are_bounded",
    "p130_identity_and_acyclic_moves_preserve_simultaneous_results",
    "p130_two_and_three_slot_cycles_use_bounded_scratch",
    "p130_invalid_requests_and_budget_failures_publish_nothing",
    "p130_bounded_generated_graphs_match_simultaneous_oracle",
    "p060_swaps_rotations_and_backedges",
    "p060_each_br_table_target_and_function_return",
    "p060_structural_fuel_cancel_and_operand_limits",
    "p060_invalid_or_oversized_control_declines",
    "p061_integer_boundaries_and_source_traps",
    "p061_every_source_prefix_and_logical_limits",
    "p061_control_edges_and_repeated_visits",
    "p061_whole_function_fallback_and_owner_guards",
    "p061_cancel_before_entry",
    "p062_raw_bits_and_strict_float_rules",
    "p062_trapping_and_saturating_conversion_boundaries",
    "p062_mixed_numeric_prefixes_and_vector_fallback",
    "p062_opcode_boundaries_match_decoded",
    "p062_float_preparation_and_activation_guards",
    "p063_all_load_widths_and_signed_bits_match",
    "p063_all_store_widths_and_fuel_images_match",
    "p063_traps_preserve_prior_effects_and_boundary_priority",
    "p063_shared_wide_and_custom_pages_decline",
    "p063_bindings_growth_and_aliases_use_current_bytes",
    "p063_invalid_memory_metadata_and_binding_declines",
    "p064_direct_call_and_store_prefix_execute_once",
    "p064_host_reentry_keeps_state_and_fuel",
    "p064_tail_indirect_and_depth_errors_match",
    "p064_every_prefix_and_fallback_preserve_accounting",
    "p064_prefix_preparation_limits_and_owner_guards",
) + P065_TESTS

CORE_CRATES = ("wtime-runtime", "wtime-types", "wtime-decode", "wtime-validate")
TIER_CASES = ("call_overhead", "integer_loop", "branch_calls")
ROOT_SOURCES = {"Cargo.toml", "Cargo.lock", "rust-toolchain.toml"}
TIER_CHECK_EXPECTED = {
    "selected_cases": 3,
    "mode_runs_expected": 6,
    "mode_runs_executed": 6,
    "state_mode_runs_expected": 6,
    "state_mode_runs_executed": 6,
    "state_fuel_probes_expected": 68,
    "state_fuel_probes_executed": 68,
    "cursor_checks_executed": 1,
    "conformance_mode_runs_expected": 4,
    "conformance_mode_runs_executed": 4,
    "all_checks_expected": 89,
    "all_checks_attempted": 89,
    "all_checks_executed": 89,
    "failed": 0,
    "skipped": 0,
    "unsupported": 0,
}
MAX_BUILD_BYTES = 30_000_000


class P065Error(ValueError):
    """P065 evidence is absent, stale, malformed, or inconsistent."""


def need(condition: bool, message: str) -> None:
    if not condition:
        raise P065Error(message)


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
    need(path.is_relative_to(store), "P065 output must stay under target/verification")
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


def completed(task: str) -> dict[str, Any]:
    expected = COMPLETED[task]
    completion_path = ROOT / f"docs/benchmarks/performance/{task}-completion.json"
    evidence_path = ROOT / f"docs/benchmarks/performance/{task}-evidence.json"
    final_path = ROOT / expected["final_evidence"]
    need(sha(completion_path) == expected["completion_sha256"],
         f"{task} completion reference changed")
    need(sha(evidence_path) == expected["evidence_sha256"],
         f"{task} evidence reference changed")
    completion = json.loads(completion_path.read_text(encoding="utf-8"))
    evidence = json.loads(evidence_path.read_text(encoding="utf-8"))
    need(completion.get("task") == task and completion.get("status") == "done",
         f"{task} completion is not a durable done record")
    need(completion.get("artifact_hashes", {}).get(str(final_path)) == expected["evidence_sha256"]
         and sha(final_path) == expected["evidence_sha256"],
         f"{task} final evidence differs from its completion record")
    need(evidence.get("task") == task and evidence.get("status") == "passed"
         and evidence.get("passed") is True,
         f"{task} final evidence did not pass")
    if task == "P064":
        # P064's implementation passed verification but its performance decision
        # was rejected. That durable outcome is a valid semantic prerequisite.
        need(completion.get("outcome") == "rejected" and evidence.get("decision") == "rejected",
             "P064 record no longer has its reviewed rejected-performance outcome")
    else:
        need(completion.get("outcome") == "contract_only"
             and evidence.get("decision") == "contract_only",
             "P131 is not the expected correctness-only foundation")
    return {"completion": ref(completion_path), "evidence": ref(evidence_path),
            "final_evidence": ref(final_path), "status": evidence["status"],
            "outcome": completion["outcome"]}


def freeze(output: Path) -> dict[str, Any]:
    output = start_output(output)
    import baseline

    predecessor = (ROOT / PREDECESSOR).resolve(strict=True)
    manifest_path = predecessor / "manifest.json"
    need(sha(manifest_path) == PREDECESSOR_MANIFEST_SHA256,
         "P065 immediate predecessor manifest identity changed")
    manifest = baseline.verify_archive(predecessor)
    need(manifest.get("mode") == "register", "P065 predecessor must be Register mode")
    need(manifest.get("repository_commit") == PREDECESSOR_COMMIT,
         "P065 predecessor is not the immutable P064 successor commit")
    policy = manifest.get("execution_policy", {})
    need(policy.get("requested_mode") == "register" and policy.get("resolved_mode") == "register",
         "P065 predecessor execution policy is not exactly Register")
    need(policy.get("protocol_modes") == ["decoded", "portable", "register"]
         and policy.get("protocol_sha256") == PROTOCOL_SHA256,
         "P065 predecessor benchmark protocol changed")
    source_map = runtime_source_map(manifest)
    need(ROOT_SOURCES <= set(source_map), "P065 predecessor omits root Cargo/toolchain inputs")
    for crate in CORE_CRATES:
        need(f"crates/{crate}/Cargo.toml" in source_map and any(
            path.startswith(f"crates/{crate}/") and path.endswith(".rs") for path in source_map),
            f"P065 predecessor omits {crate} source inputs")

    p001 = (ROOT / "target/performance/baseline").resolve(strict=True)
    p001_manifest = baseline.verify_archive(p001)
    need(sha(p001 / "manifest.json") == P001_MANIFEST_SHA256
         and sha(p001 / "binary/wtime-bench") == P001_BINARY_SHA256,
         "immutable P001 archive identity changed")
    need(p001_manifest.get("smoke", {}).get("correctness_checked") is True
         and p001_manifest.get("smoke", {}).get("returncode") == 0,
         "immutable P001 archive lacks a successful correctness smoke")

    prerequisites = {task.lower(): completed(task) for task in ("P064", "P131")}
    runtime_path = output / "runtime.json"
    write_new(runtime_path, source_map)
    report = {
        "schema": "wtime-p065-freeze-v1",
        "status": "passed",
        "timing_evidence": False,
        "predecessor": ref(manifest_path),
        "predecessor_manifest_sha256_pinned": PREDECESSOR_MANIFEST_SHA256,
        "predecessor_mode": "register",
        "predecessor_commit": PREDECESSOR_COMMIT,
        "protocol_sha256": PROTOCOL_SHA256,
        "runtime_source_map": source_map,
        "runtime_source_map_sha256": canonical_sha(source_map),
        "runtime_source_file_count": len(source_map),
        "runtime_map": ref(runtime_path),
        "p001": {
            "manifest": ref(p001 / "manifest.json"),
            "binary": ref(p001 / "binary/wtime-bench"),
            "manifest_sha256_pinned": P001_MANIFEST_SHA256,
            "binary_sha256_pinned": P001_BINARY_SHA256,
            "correctness_checked": True,
        },
        "prerequisites": prerequisites,
    }
    write_new(output / "report.json", report)
    return report


def require_tests(path: Path, names: tuple[str, ...], label: str) -> None:
    text = path.read_text(encoding="utf-8", errors="replace")
    for name in names:
        need(re.search(rf"^test\s+[^\n]*{re.escape(name)}\s+\.\.\. ok$",
                       text, re.MULTILINE) is not None,
             f"{label} log lacks passing named case {name}")


def load_json(path: Path, label: str) -> dict[str, Any]:
    path = path.resolve(strict=True)
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise P065Error(f"{label} is not valid JSON: {exc}") from exc
    need(isinstance(value, dict), f"{label} must be a JSON object")
    return value


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


P065_MARKERS = (
    "P065: sole outer root survives collection across a Register call exit",
    "P065: numeric local=13 and operand=99 captured at suspend source_pc=5",
    "P065: cross-module exact suspension preserves one-shot fuel cancellation and byte budgets",
    "P065: reference and vector layouts take whole-function fallback",
    "P065: numeric exception payload=20 unwinds after a Register exit",
)


def check_marker(stderr: Path) -> list[str]:
    text = stderr.read_text(encoding="utf-8", errors="replace")
    for marker in P065_MARKERS:
        need(text.count(marker) == 1, f"P065 stderr must contain exactly one success marker: {marker}")
    return list(P065_MARKERS)


def check_tier_report(path: Path) -> tuple[dict[str, Any], dict[str, Any]]:
    report = load_json(path, "tier_check report")
    need(report.get("schema_version") == 1 and report.get("passed") is True
         and report.get("source_stable") is True,
         "tier_check did not pass with stable implementation sources")
    need(report.get("selection") == "integration" and report.get("requested_tier") == "register"
         and report.get("baseline_tier") == "reference",
         "tier_check selected the wrong tier or semantic selection")
    need(report.get("instrumented") is True and report.get("diagnostic_instrumentation", {}).get("enabled") is True,
         "tier_check lacks required instrumented reference/register execution")
    modes = report.get("modes")
    need(isinstance(modes, list) and {"reference", "register"} <= set(modes),
         "tier_check protocol does not expose both reference and register modes")
    counts = report.get("counts")
    need(isinstance(counts, dict), "tier_check counts are missing")
    for name, expected in TIER_CHECK_EXPECTED.items():
        need(type(counts.get(name)) is int and counts[name] == expected,
             f"tier_check count {name}={counts.get(name)!r}, expected {expected}")
    need(isinstance(report.get("errors"), list) and report["errors"] == [],
         "tier_check reported errors")
    cases = report.get("cases")
    need(isinstance(cases, list) and len(cases) == len(TIER_CASES)
         and {case.get("id") for case in cases} == set(TIER_CASES),
         "tier_check integration semantic cases are missing or changed")
    for case in cases:
        need(case.get("equal") is True and case.get("runs", {}).get("reference", {}).get("result")
             == case.get("runs", {}).get("register", {}).get("result"),
             f"tier_check semantic case did not match reference: {case.get('id')}")
    for key in ("implementation_sources_sha256", "binary", "fixtures", "fixture_identities_before",
                "fixture_identities_after", "conformance", "state_checks", "state_fuel_checks",
                "cursor_checks", "snapshot", "decoded_source_op_diagnostics"):
        need(key in report, f"tier_check report omits source-bound evidence {key}")
    need(report["fixture_identities_before"] == report["fixture_identities_after"],
         "tier_check fixture identities changed during integration checks")
    need(len(report["conformance"]) == 4
         and {(item.get("suite"), item.get("mode")) for item in report["conformance"]}
         == {(suite, mode) for suite in ("reference_types", "interactions")
             for mode in ("reference", "register")},
         "tier_check did not run both conformance suites in both modes")
    for item in report["conformance"]:
        result = item.get("report", {})
        need(result.get("executed", 0) > 0 and result.get("failed") == 0
             and result.get("skipped") == 0 and result.get("unsupported") == 0,
             f"tier_check conformance did not fully pass: {item.get('suite')} {item.get('mode')}")
        command = item.get("command", [])
        need(isinstance(command, list) and len(command) >= 2 and command[-2] == "--report",
             "tier_check conformance record does not retain its report command")
        report_path = Path(command[-1])
        if not report_path.is_absolute():
            report_path = ROOT / report_path
        need(item.get("report_sha256") == sha(report_path),
             f"tier_check conformance report hash changed: {item.get('suite')} {item.get('mode')}")
    need(len(report["state_checks"]) == 6
         and {(item.get("case"), item.get("mode")) for item in report["state_checks"]}
         == {(case, mode) for case in ("state_normal", "state_trap", "state_graph")
             for mode in ("reference", "register")},
         "tier_check did not run full state checks in both modes")
    need(len(report["state_fuel_checks"]) == 68
         and len(report["cursor_checks"]) == 1
         and len(report["decoded_source_op_diagnostics"]) == 3,
         "tier_check fuel/cursor/dispatch diagnostic work is incomplete")
    build = report.get("build")
    need(isinstance(build, dict)
         and build.get("feature_sets", {}).get("policy")
         == ["execution-reference", "register-executor"]
         and build.get("feature_sets", {}).get("benchmark_harness")
         == ["register-executor", "runtime-only"]
         and build.get("feature_sets", {}).get("conformance_policy")
         == ["execution-reference", "register-executor"],
         "tier_check did not build the exact Register/reference feature set")
    summary = {
        "passed": True,
        "source_stable": True,
        "requested_tier": "register",
        "selection": "integration",
        "baseline_tier": "reference",
        "modes": sorted(set(modes)),
        "counts": {name: counts[name] for name in TIER_CHECK_EXPECTED},
        "implementation_sources_sha256": report["implementation_sources_sha256"],
        "binary": report["binary"],
        "conformance_binary": report["conformance_binary"],
        "diagnostic_instrumentation": {
            "enabled": True,
            "feature": report["diagnostic_instrumentation"]["feature"],
            "binary_sha256": report["diagnostic_instrumentation"]["binary_sha256"],
        },
        "fixture_identities_sha256": canonical_sha(report["fixture_identities_before"]),
        "conformance_source_count": sum(len(row.get("corpus_sources", {})) for row in report["conformance"]),
        "semantic_case_ids": sorted(row["id"] for row in cases),
        "errors": [],
    }
    need(len(summary["implementation_sources_sha256"]) == 64
         and len(summary["binary"].get("sha256", "")) == 64
         and len(summary["conformance_binary"].get("sha256", "")) == 64,
         "tier_check source or binary identity is malformed")
    need(summary["conformance_source_count"] > 0,
         "tier_check conformance reports did not bind corpus source files")
    return summary, report


def verify(output: Path, register_log: Path, register_stderr: Path,
           default_log: Path, combined_log: Path, tier_report: Path,
           default_build: Path, register_build: Path, freeze_report: Path) -> dict[str, Any]:
    output = start_output(output)
    frozen = load_json(freeze_report, "P065 freeze report")
    need(frozen.get("schema") == "wtime-p065-freeze-v1" and frozen.get("status") == "passed"
         and frozen.get("timing_evidence") is False,
         "P065 freeze receipt is invalid")
    need(frozen.get("predecessor_commit") == PREDECESSOR_COMMIT
         and frozen.get("predecessor_manifest_sha256_pinned") == PREDECESSOR_MANIFEST_SHA256
         and frozen.get("predecessor_mode") == "register"
         and frozen.get("protocol_sha256") == PROTOCOL_SHA256,
         "P065 freeze receipt does not bind the exact immediate predecessor")
    need(frozen.get("runtime_source_map_sha256") == canonical_sha(frozen.get("runtime_source_map")),
         "P065 frozen runtime source map hash is invalid")

    register_log = register_log.resolve(strict=True)
    register_stderr = register_stderr.resolve(strict=True)
    default_log = default_log.resolve(strict=True)
    combined_log = combined_log.resolve(strict=True)
    require_tests(register_log, REGISTER_TESTS, "register/reference")
    require_tests(default_log, (DEFAULT_TEST,), "default policy")
    require_tests(combined_log, P065_TESTS, "combined compact/register/reference")
    marker = check_marker(register_stderr)

    builds = {
        "default": check_build(default_build, "default", "default", []),
        "register": check_build(register_build, "register", "optional-feature",
                                 ["wtime-runtime/register-executor"]),
    }
    tier_summary, tier_value = check_tier_report(tier_report)
    tier_ref = ref(Path(tier_report))
    # The full tier report is retained and hash-bound. Its ns_per_operation fields
    # remain diagnostic-only and are deliberately excluded from the proof summary.
    report = {
        "schema": "wtime-p065-verification-v1",
        "status": "passed",
        "timing_evidence": False,
        "performance_candidate": False,
        "speed_decision": None,
        "settings": {
            "register_features": ["register-executor", "execution-reference"],
            "combined_features": ["compact-executor", "register-executor", "execution-reference"],
            "default_features": [],
            "tier": "register",
            "selection": "integration",
            "tier_check_observations_are_diagnostic_only": True,
        },
        "counts": {
            "register_named_cases": len(REGISTER_TESTS),
            "p065_named_cases": 5,
            "p065_private_named_cases": len(P065_TESTS) - 5,
            "combined_p065_named_cases": len(P065_TESTS),
            "default_named_cases": 1,
            "tier_check": tier_summary["counts"],
        },
        "p065_markers": marker,
        "freeze_report": ref(Path(freeze_report)),
        "runtime_source_map_sha256": frozen["runtime_source_map_sha256"],
        "test_logs": {
            "register": ref(register_log),
            "register_stderr": ref(register_stderr),
            "default": ref(default_log),
            "combined_p065": ref(combined_log),
        },
        "build_budgets": builds,
        "tier_check": tier_summary,
        "tier_check_report": tier_ref,
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
    verify_parser.add_argument("--default-log", type=Path, required=True)
    verify_parser.add_argument("--combined-log", type=Path, required=True)
    verify_parser.add_argument("--tier-report", type=Path, required=True)
    verify_parser.add_argument("--default-build", type=Path, required=True)
    verify_parser.add_argument("--register-build", type=Path, required=True)
    verify_parser.add_argument("--freeze-report", type=Path, required=True)
    args = parser.parse_args()
    try:
        if args.phase == "freeze":
            result = freeze(args.output)
        else:
            result = verify(args.output, args.register_log, args.register_stderr,
                            args.default_log, args.combined_log, args.tier_report,
                            args.default_build, args.register_build, args.freeze_report)
        print(json.dumps({"status": result["status"], "schema": result["schema"]}, sort_keys=True))
        return 0
    except (OSError, ValueError, json.JSONDecodeError) as exc:
        print(f"P065 verification failed: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
