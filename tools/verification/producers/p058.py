#!/usr/bin/env python3
"""Freeze and verify P058 stack-to-register lowering contract evidence."""
from __future__ import annotations

import argparse
from collections import Counter
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

PREDECESSOR = "target/performance/P058-before-visit-diagnostics"
PREDECESSOR_MANIFEST_SHA256 = "347e862290bcfd7c257f4981f3d36c9e9951e44fed1e978b04daaf3736cc95a3"
P001_MANIFEST_SHA256 = "7349596fc806c96c74cb9c40f1c08402589c1683660b0b152c713264f5808472"
P001_BINARY_SHA256 = "735e595f3a625da89401fdee298781b752fbe071e3d3d793df196c9e68b8e498"
P057_COMPLETION_SHA256 = "4ee850eeaeac86b302acd543eaace6b0fa96d02d2655cc112fb00c01b703d481"
P057_EVIDENCE_SHA256 = "8061225076d2226e948b23d51f951ef90a5443edf17aa0a685ab08e2cd716264"
P129_COMPLETION_SHA256 = "60779adf995091d41083b94615a76612770c4d4f20e0d11a8771feeae465270f"
P129_EVIDENCE_SHA256 = "0c3c659a972308ab7312091e62c3fa8568b7004a68040ef07adcecad02032060"
LOWER_TESTS = (
    "p058_local_numeric_slots_match_every_decoded_prefix",
    "p058_multivalue_edges_and_loop_visits_match_decoded_prefixes",
    "p058_declines_invalid_joins_and_preserves_shared_budgets",
)
DEFAULT_TEST = "p058_decoded_fixture_results_preserve_default_policy"
P057_TESTS = (
    "p057_numeric_loop_has_bounded_typed_slots_and_source_boundaries",
    "p057_unsupported_types_and_effects_decline_without_guest_work",
    "p057_bad_metadata_and_aggregate_budgets_are_transactional",
    "p057_register_policy_stays_unavailable_until_execution_lands",
)
P129_TESTS = (
    "p129_zero_skip_preserves_first_visit",
    "p129_later_loop_visit_keeps_exact_state_and_fuel",
    "p129_invalid_visit_bound_and_unreached_request_are_bounded",
)
CORE_CRATES = ("wtime-runtime", "wtime-types", "wtime-decode", "wtime-validate")
ROOT_SOURCES = {"Cargo.toml", "Cargo.lock", "rust-toolchain.toml"}


def need(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def sha(path: Path) -> str:
    digest = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


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
    need(path.is_relative_to(store), "P058 output must stay under target/verification")
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


def freeze(output: Path) -> dict[str, Any]:
    output = start_output(output)
    import baseline

    predecessor = (ROOT / PREDECESSOR).resolve(strict=True)
    manifest_path = predecessor / "manifest.json"
    need(sha(manifest_path) == PREDECESSOR_MANIFEST_SHA256,
         "P058 immediate-predecessor manifest identity changed")
    manifest = baseline.verify_archive(predecessor)
    need(manifest.get("mode") == "decoded", "P058 predecessor is not decoded mode")
    source_map = runtime_source_map(manifest)
    need(ROOT_SOURCES <= set(source_map), "P058 predecessor omits root Cargo/toolchain inputs")
    for crate in CORE_CRATES:
        need(f"crates/{crate}/Cargo.toml" in source_map and any(
            path.startswith(f"crates/{crate}/") and path.endswith(".rs") for path in source_map),
            f"P058 predecessor omits {crate} source inputs")

    p001 = (ROOT / "target/performance/baseline").resolve(strict=True)
    p001_manifest = baseline.verify_archive(p001)
    need(sha(p001 / "manifest.json") == P001_MANIFEST_SHA256
         and sha(p001 / "binary/wtime-bench") == P001_BINARY_SHA256,
         "immutable P001 archive identity changed")
    need(p001_manifest.get("smoke", {}).get("correctness_checked") is True,
         "immutable P001 archive lacks successful correctness smoke")

    completion_path = ROOT / "docs/benchmarks/performance/P057-completion.json"
    evidence_path = ROOT / "docs/benchmarks/performance/P057-evidence.json"
    final_evidence_path = ROOT / "target/performance/P057/shared-finalization/attempt-0001/evidence.json"
    need(sha(completion_path) == P057_COMPLETION_SHA256,
         "P057 completion reference changed")
    need(sha(evidence_path) == P057_EVIDENCE_SHA256,
         "P057 evidence reference changed")
    completion = json.loads(completion_path.read_text(encoding="utf-8"))
    evidence = json.loads(evidence_path.read_text(encoding="utf-8"))
    need(completion.get("task") == "P057" and completion.get("status") == "done",
         "P057 completion is not an immutable completed record")
    need(completion.get("artifact_hashes", {}).get(str(final_evidence_path)) == P057_EVIDENCE_SHA256
         and sha(final_evidence_path) == P057_EVIDENCE_SHA256,
         "local P057 final evidence differs from its immutable completion reference")
    need(evidence.get("task") == "P057" and evidence.get("status") == "passed",
         "P057 final evidence is not passed")

    p129_completion_path = ROOT / "docs/benchmarks/performance/P129-completion.json"
    p129_evidence_path = ROOT / "docs/benchmarks/performance/P129-evidence.json"
    p129_final_evidence_path = ROOT / "target/performance/P129/shared-finalization/attempt-0002/evidence.json"
    need(sha(p129_completion_path) == P129_COMPLETION_SHA256,
         "P129 completion reference changed")
    need(sha(p129_evidence_path) == P129_EVIDENCE_SHA256,
         "P129 evidence reference changed")
    p129_completion = json.loads(p129_completion_path.read_text(encoding="utf-8"))
    p129_evidence = json.loads(p129_evidence_path.read_text(encoding="utf-8"))
    need(p129_completion.get("task") == "P129" and p129_completion.get("status") == "done",
         "P129 completion is not an immutable completed record")
    need(p129_completion.get("artifact_hashes", {}).get(str(p129_final_evidence_path))
         == P129_EVIDENCE_SHA256 and sha(p129_final_evidence_path) == P129_EVIDENCE_SHA256,
         "local P129 final evidence differs from its immutable completion reference")
    need(p129_evidence.get("task") == "P129" and p129_evidence.get("status") == "passed",
         "P129 final evidence is not passed")

    report = {
        "schema": "wtime-p058-freeze-v1",
        "status": "passed",
        "timing_evidence": False,
        "predecessor": ref(manifest_path),
        "predecessor_mode": "decoded",
        "runtime_source_map": source_map,
        "runtime_source_file_count": len(source_map),
        "p001": {"manifest": ref(p001 / "manifest.json"),
                 "binary": ref(p001 / "binary/wtime-bench"),
                 "manifest_sha256_pinned": P001_MANIFEST_SHA256,
                 "binary_sha256_pinned": P001_BINARY_SHA256},
        "p057": {"completion": ref(completion_path), "evidence": ref(evidence_path),
                 "final_evidence": ref(final_evidence_path)},
        "p129": {"completion": ref(p129_completion_path), "evidence": ref(p129_evidence_path),
                 "final_evidence": ref(p129_final_evidence_path)},
    }
    write_new(output / "report.json", report)
    return report


def require_tests(path: Path, names: tuple[str, ...], label: str) -> None:
    text = path.read_text(encoding="utf-8", errors="replace")
    for name in names:
        need(re.search(rf"^test\s+[^\n]*{re.escape(name)}\s+\.\.\. ok$",
                       text, re.MULTILINE) is not None,
             f"{label} log lacks passing named case {name}")


def verify(output: Path, lower_log: Path, lower_stderr: Path,
           p057_log: Path, p129_log: Path, default_log: Path,
           default_build: Path, register_build: Path) -> dict[str, Any]:
    output = start_output(output)
    logs = {"lowering": lower_log.resolve(strict=True),
            "p057_compatibility": p057_log.resolve(strict=True),
            "p129_compatibility": p129_log.resolve(strict=True),
            "default": default_log.resolve(strict=True)}
    require_tests(logs["lowering"], LOWER_TESTS, "P058 lowering")
    require_tests(logs["lowering"], (DEFAULT_TEST,), "P058 default fixture")
    require_tests(logs["p057_compatibility"], P057_TESTS, "P057 compatibility")
    require_tests(logs["p129_compatibility"], P129_TESTS, "P129 compatibility")
    require_tests(logs["default"], (DEFAULT_TEST,), "default")

    lower_stderr = lower_stderr.resolve(strict=True)
    marker = re.compile(
        r"P058: (?P<dynamic_prefixes>[1-9][0-9]*) dynamic source prefixes; "
        r"(?P<source_events>[1-9][0-9]*) source events; "
        r"(?P<retained_bytes>[1-9][0-9]*) bytes; (?P<work>[1-9][0-9]*) work"
    )
    marker_text = lower_stderr.read_text(encoding="utf-8", errors="replace")
    markers = list(marker.finditer(marker_text))
    need(markers, "P058 lowering test stderr lacks its bounded source-prefix markers")
    runs = [{name: int(value) for name, value in match.groupdict().items()}
            for match in markers]
    actual_pairs = Counter((row["dynamic_prefixes"], row["source_events"]) for row in runs)
    expected_pairs = Counter({(16, 16): 1, (6, 7): 1, (27, 16): 1,
                              (66, 25): 1, (11, 15): 1, (10, 15): 1, (6, 15): 1})
    need(actual_pairs == expected_pairs,
         f"P058 marker coverage differs: observed {sorted(actual_pairs.elements())}")
    need(sum(row["dynamic_prefixes"] for row in runs) == 142
         and sum(row["source_events"] for row in runs) == 109,
         "P058 aggregate dynamic-prefix or source-event totals changed")
    values = {
        "model_runs": len(runs),
        "dynamic_prefixes": sum(row["dynamic_prefixes"] for row in runs),
        "source_events": sum(row["source_events"] for row in runs),
        "retained_bytes": sum(row["retained_bytes"] for row in runs),
        "work": sum(row["work"] for row in runs),
        "runs": sorted(runs, key=lambda row: (row["dynamic_prefixes"], row["source_events"],
                                                row["retained_bytes"], row["work"])),
    }

    builds = {}
    for label, path, mode, features in (
        ("default", default_build, "default", []),
        ("register", register_build, "optional-feature", ["wtime-runtime/register-executor"]),
    ):
        record = json.loads(path.resolve(strict=True).read_text(encoding="utf-8"))
        need(record.get("mode") == mode and record.get("features") == features,
             f"{label} build-budget report has the wrong feature configuration")
        need(record.get("source_stable") is True and record.get("budget_pass") is True
             and record.get("default_dependency_closure_pass") is True,
             f"{label} build-budget report failed source, budget, or dependency policy")
        builds[label] = ref(path)

    report = {
        "schema": "wtime-p058-verification-v1",
        "status": "passed",
        "timing_evidence": False,
        "performance_candidate": False,
        "speed_decision": None,
        "settings": {"instrumented": False,
                     "lowering_features": ["register-executor", "execution-reference"],
                     "default_features": []},
        "counts": {"lowering_named_cases": len(LOWER_TESTS),
                   "fixture_policy_named_cases": 1,
                   "p057_compatibility_named_cases": len(P057_TESTS),
                   "p129_compatibility_named_cases": len(P129_TESTS),
                   "default_named_cases": 1},
        "lowering_marker": values,
        "test_logs": {name: ref(path) for name, path in logs.items()},
        "lowering_stderr": ref(lower_stderr),
        "build_budgets": builds,
    }
    write_new(output / "report.json", report)
    return report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--lower-log", type=Path)
    parser.add_argument("--lower-stderr", type=Path)
    parser.add_argument("--p057-log", type=Path)
    parser.add_argument("--p129-log", type=Path)
    parser.add_argument("--default-log", type=Path)
    parser.add_argument("--default-build", type=Path)
    parser.add_argument("--register-build", type=Path)
    args = parser.parse_args()
    try:
        if args.lower_log is None:
            result = freeze(args.output)
        else:
            required = (args.lower_stderr, args.p057_log, args.p129_log, args.default_log,
                        args.default_build, args.register_build)
            need(all(item is not None for item in required),
                 "verify mode requires all test logs and both build reports")
            result = verify(args.output, args.lower_log, args.lower_stderr,
                            args.p057_log, args.p129_log, args.default_log,
                            args.default_build, args.register_build)
        print(json.dumps({"status": result["status"], "schema": result["schema"]}, sort_keys=True))
        return 0
    except (OSError, ValueError, KeyError, json.JSONDecodeError) as error:
        print(f"p058.py: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
