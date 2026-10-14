#!/usr/bin/env python3
"""Freeze and verify the bounded P131 register source-prefix index."""
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

PREDECESSOR = "target/performance/P131-before"
PREDECESSOR_COMMIT = "bdbf60e2eec4295b58d0fb9c342a77d840066f03"
PREDECESSOR_MANIFEST_SHA256 = "3b7c089fe2fd3161e5063bc97698fd38b4865f6750a20b7183cae1fb5caea41f"
PROTOCOL_SHA256 = "ac7b9ce0d12a0a17b38acbfb964f8704c8f85c1927d89385a055b612f77b848c"
P001_MANIFEST_SHA256 = "7349596fc806c96c74cb9c40f1c08402589c1683660b0b152c713264f5808472"
P001_BINARY_SHA256 = "735e595f3a625da89401fdee298781b752fbe071e3d3d793df196c9e68b8e498"

COMPLETED = {
    "P057": {
        "completion_sha256": "4ee850eeaeac86b302acd543eaace6b0fa96d02d2655cc112fb00c01b703d481",
        "evidence_sha256": "8061225076d2226e948b23d51f951ef90a5443edf17aa0a685ab08e2cd716264",
        "final_evidence": "target/performance/P057/shared-finalization/attempt-0001/evidence.json",
    },
    "P058": {
        "completion_sha256": "83e4626cecf912026f5bba6b34da49a8c49e53963e9d5e0d2a05d3e08e3b0443",
        "evidence_sha256": "d77c102b292744889128bf4b1e4f1f66767e0a9681d063f0aa0841b4e2bc3a20",
        "final_evidence": "target/performance/P058/shared-finalization/attempt-0002/evidence.json",
    },
    "P063": {
        "completion_sha256": "bb8662140ffb606ac073be4479dbd21394a36b57101602a4f2f49dbd26686b6d",
        "evidence_sha256": "783f7cb4d1259f72ddc3ac2fd8bd2dc70ca4f691efa8bcaf77b05cd36e52a016",
        "final_evidence": "target/performance/P063/shared-finalization/attempt-0001/evidence.json",
    },
    "P129": {
        "completion_sha256": "60779adf995091d41083b94615a76612770c4d4f20e0d11a8771feeae465270f",
        "evidence_sha256": "0c3c659a972308ab7312091e62c3fa8568b7004a68040ef07adcecad02032060",
        "final_evidence": "target/performance/P129/shared-finalization/attempt-0002/evidence.json",
    },
}

P131_TESTS = (
    "p131_prefix_values_and_source_boundaries_match",
    "p131_open_labels_and_memory_prefixes_match",
    "p131_invalid_barriers_and_budgets_decline",
)
DEFAULT_TEST = "p131_default_policy_remains_decoded"
REGISTER_TESTS = P131_TESTS + (
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
)
CORE_CRATES = ("wtime-runtime", "wtime-types", "wtime-decode", "wtime-validate")
ROOT_SOURCES = {"Cargo.toml", "Cargo.lock", "rust-toolchain.toml"}
MAX_WORK = 100_000
MAX_BYTES = 1_048_576


def need(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


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
    path = path.resolve(strict=True)
    return {"path": str(path), "sha256": sha(path), "bytes": path.stat().st_size}


def write_new(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("x", encoding="utf-8") as stream:
        json.dump(value, stream, indent=2, sort_keys=True, allow_nan=False)
        stream.write("\n")


def start_output(path: Path) -> Path:
    path = path.resolve()
    store = (ROOT / "target/verification").resolve()
    need(path.is_relative_to(store), "P131 output must stay under target/verification")
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
    need(sha(evidence_path) == expected["evidence_sha256"], f"{task} evidence reference changed")
    completion = json.loads(completion_path.read_text(encoding="utf-8"))
    evidence = json.loads(evidence_path.read_text(encoding="utf-8"))
    need(completion.get("task") == task and completion.get("status") == "done",
         f"{task} completion is not an immutable completed record")
    need(completion.get("artifact_hashes", {}).get(str(final_path)) == expected["evidence_sha256"]
         and sha(final_path) == expected["evidence_sha256"],
         f"{task} final evidence differs from its completion reference")
    need(evidence.get("task") == task and evidence.get("status") == "passed",
         f"{task} final evidence is not passed")
    return {"completion": ref(completion_path), "evidence": ref(evidence_path),
            "final_evidence": ref(final_path)}


def freeze(output: Path) -> dict[str, Any]:
    output = start_output(output)
    import baseline

    predecessor = (ROOT / PREDECESSOR).resolve(strict=True)
    manifest_path = predecessor / "manifest.json"
    need(sha(manifest_path) == PREDECESSOR_MANIFEST_SHA256,
         "P131 predecessor manifest identity changed")
    manifest = baseline.verify_archive(predecessor)
    need(manifest.get("mode") == "register", "P131 predecessor must be register mode")
    need(manifest.get("repository_commit") == PREDECESSOR_COMMIT,
         "P131 predecessor is not the immutable bdbf60e P063 commit")
    policy = manifest.get("execution_policy", {})
    need(policy.get("requested_mode") == "register" and policy.get("resolved_mode") == "register",
         "P131 predecessor execution policy is not exactly register")
    need(policy.get("protocol_modes") == ["decoded", "portable", "register"]
         and policy.get("protocol_sha256") == PROTOCOL_SHA256,
         "P131 predecessor benchmark protocol changed")
    source_map = runtime_source_map(manifest)
    need(ROOT_SOURCES <= set(source_map), "P131 predecessor omits root Cargo/toolchain inputs")
    for crate in CORE_CRATES:
        need(f"crates/{crate}/Cargo.toml" in source_map and any(
            path.startswith(f"crates/{crate}/") and path.endswith(".rs") for path in source_map),
            f"P131 predecessor omits {crate} source inputs")

    p001 = (ROOT / "target/performance/baseline").resolve(strict=True)
    p001_manifest = baseline.verify_archive(p001)
    need(sha(p001 / "manifest.json") == P001_MANIFEST_SHA256
         and sha(p001 / "binary/wtime-bench") == P001_BINARY_SHA256,
         "immutable P001 archive identity changed")
    need(p001_manifest.get("smoke", {}).get("correctness_checked") is True,
         "immutable P001 archive lacks a successful correctness smoke")
    prerequisites = {task.lower(): completed(task) for task in ("P057", "P058", "P063", "P129")}
    report = {
        "schema": "wtime-p131-freeze-v1",
        "status": "passed",
        "timing_evidence": False,
        "predecessor": ref(manifest_path),
        "predecessor_manifest_sha256_pinned": PREDECESSOR_MANIFEST_SHA256,
        "predecessor_mode": "register",
        "predecessor_commit": PREDECESSOR_COMMIT,
        "protocol_sha256": PROTOCOL_SHA256,
        "runtime_source_map": source_map,
        "runtime_source_file_count": len(source_map),
        "runtime_source_map_sha256": canonical_sha(source_map),
        "p001": {"manifest": ref(p001 / "manifest.json"),
                 "binary": ref(p001 / "binary/wtime-bench"),
                 "manifest_sha256_pinned": P001_MANIFEST_SHA256,
                 "binary_sha256_pinned": P001_BINARY_SHA256},
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


def verify(output: Path, register_log: Path, register_stderr: Path,
           default_log: Path, default_build: Path, register_build: Path,
           freeze_report: Path) -> dict[str, Any]:
    output = start_output(output)
    freeze_report = freeze_report.resolve(strict=True)
    frozen = json.loads(freeze_report.read_text(encoding="utf-8"))
    need(frozen.get("schema") == "wtime-p131-freeze-v1" and frozen.get("status") == "passed",
         "P131 freeze receipt is invalid")
    need(frozen.get("predecessor_commit") == PREDECESSOR_COMMIT
         and frozen.get("predecessor_mode") == "register"
         and frozen.get("protocol_sha256") == PROTOCOL_SHA256,
         "P131 freeze receipt does not bind the exact predecessor")
    need(frozen.get("runtime_source_map_sha256") == canonical_sha(frozen.get("runtime_source_map")),
         "P131 frozen runtime source map hash is invalid")

    register_log, register_stderr = register_log.resolve(strict=True), register_stderr.resolve(strict=True)
    default_log = default_log.resolve(strict=True)
    require_tests(register_log, REGISTER_TESTS, "register/reference")
    require_tests(default_log, (DEFAULT_TEST,), "default policy")
    marker = re.compile(
        r"P131: (?P<prefixes>[1-9][0-9]*) prefix source boundaries; "
        r"max_work=(?P<max_work>[1-9][0-9]*); "
        r"max_bytes=(?P<max_bytes>[1-9][0-9]*); "
        r"open_labels=(?P<open_labels>[1-9][0-9]*); "
        r"memory_prefixes=(?P<memory_prefixes>[1-9][0-9]*)"
    )
    matches = list(marker.finditer(register_stderr.read_text(encoding="utf-8", errors="replace")))
    need(len(matches) == 1, "P131 tests must emit exactly one aggregate prefix marker")
    values = {name: int(value) for name, value in matches[0].groupdict().items()}
    need(values["max_work"] <= MAX_WORK, "P131 max_work exceeds the declared bound")
    need(values["max_bytes"] <= MAX_BYTES, "P131 max_bytes exceeds the declared bound")

    builds: dict[str, Any] = {}
    for label, path, mode, features in (
        ("default", default_build, "default", []),
        ("register", register_build, "optional-feature", ["wtime-runtime/register-executor"]),
    ):
        path = path.resolve(strict=True)
        record = json.loads(path.read_text(encoding="utf-8"))
        need(record.get("mode") == mode and record.get("features") == features,
             f"{label} build-budget report has the wrong feature configuration")
        need(record.get("source_stable") is True and record.get("budget_pass") is True
             and record.get("default_dependency_closure_pass") is True,
             f"{label} build-budget report failed source, budget, or dependency policy")
        builds[label] = ref(path)

    report = {
        "schema": "wtime-p131-verification-v1",
        "status": "passed",
        "timing_evidence": False,
        "performance_candidate": False,
        "speed_decision": None,
        "settings": {"instrumented": False,
                     "features": ["register-executor", "execution-reference"],
                     "default_features": []},
        "counts": {"register_compatibility_named_cases": len(REGISTER_TESTS),
                   "p131_named_cases": len(P131_TESTS),
                   "default_named_cases": 1},
        "prefix_marker": values,
        "freeze_report": ref(freeze_report),
        "runtime_source_map_sha256": frozen["runtime_source_map_sha256"],
        "test_logs": {"register": ref(register_log), "register_stderr": ref(register_stderr),
                      "default": ref(default_log)},
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
                            args.default_log, args.default_build, args.register_build,
                            args.freeze_report)
        print(json.dumps({"status": result["status"], "schema": result["schema"]}, sort_keys=True))
        return 0
    except (OSError, ValueError, json.JSONDecodeError) as exc:
        print(f"P131 verification failed: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
