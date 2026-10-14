#!/usr/bin/env python3
"""Freeze and verify P129 bounded source-visit diagnostic evidence."""
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

PREDECESSOR = "target/performance/P129-before"
PREDECESSOR_MANIFEST_SHA256 = "fb881546b352cb170d19009bc18a091c16e7538e534b192559db50a86aa1146a"
P001_MANIFEST_SHA256 = "7349596fc806c96c74cb9c40f1c08402589c1683660b0b152c713264f5808472"
P001_BINARY_SHA256 = "735e595f3a625da89401fdee298781b752fbe071e3d3d793df196c9e68b8e498"
P057_COMPLETION_SHA256 = "4ee850eeaeac86b302acd543eaace6b0fa96d02d2655cc112fb00c01b703d481"
P057_EVIDENCE_SHA256 = "8061225076d2226e948b23d51f951ef90a5443edf17aa0a685ab08e2cd716264"
VISIT_TESTS = (
    "p129_zero_skip_preserves_first_visit",
    "p129_later_loop_visit_keeps_exact_state_and_fuel",
    "p129_invalid_visit_bound_and_unreached_request_are_bounded",
)
CURSOR_COMPAT_TESTS = (
    "p018_store_successor_preserves_memory_and_fuel",
    "p018_multivalue_branch_restores_order_and_label",
    "p018_restore_never_replays_completed_host_call",
    "p018_suspend_resume_preserves_saved_fuel_and_context",
    "p018_request_distinguishes_cross_module_body_indices",
    "p018_invalid_or_foreign_cursor_request_is_rejected",
    "p018_restored_nominal_local_remains_rooted_during_host_collection",
)
DEFAULT_TEST = "p057_register_policy_stays_unavailable_until_execution_lands"
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
    need(path.is_relative_to(store), "P129 output must stay under target/verification")
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
         "P129 immediate-predecessor manifest identity changed")
    manifest = baseline.verify_archive(predecessor)
    need(manifest.get("mode") == "decoded", "P129 predecessor is not decoded mode")
    source_map = runtime_source_map(manifest)
    need(ROOT_SOURCES <= set(source_map), "P129 predecessor omits root Cargo/toolchain inputs")
    for crate in CORE_CRATES:
        need(f"crates/{crate}/Cargo.toml" in source_map and any(
            path.startswith(f"crates/{crate}/") and path.endswith(".rs") for path in source_map),
            f"P129 predecessor omits {crate} source inputs")

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

    report = {
        "schema": "wtime-p129-freeze-v1",
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
    }
    write_new(output / "report.json", report)
    return report


def _require_tests(log: Path, names: tuple[str, ...], label: str) -> None:
    text = log.read_text(encoding="utf-8", errors="replace")
    for name in names:
        need(re.search(rf"^test\s+[^\n]*{re.escape(name)}\s+\.\.\. ok$",
                       text, re.MULTILINE) is not None,
             f"{label} log lacks passing named case {name}")


def verify(output: Path, visit_log: Path, visit_stderr: Path,
           compatibility_log: Path, default_log: Path,
           default_build: Path, optional_build: Path) -> dict[str, Any]:
    output = start_output(output)
    visit_log = visit_log.resolve(strict=True)
    visit_stderr = visit_stderr.resolve(strict=True)
    compatibility_log = compatibility_log.resolve(strict=True)
    default_log = default_log.resolve(strict=True)
    _require_tests(visit_log, VISIT_TESTS, "source-visit")
    _require_tests(compatibility_log, CURSOR_COMPAT_TESTS, "cursor-compatibility")
    _require_tests(default_log, (DEFAULT_TEST,), "default")

    marker = re.compile(
        r"P129: third source visit; (?P<events>[1-9][0-9]*) charged events; "
        r"(?P<roundtrips>[1-9][0-9]*) roundtrip; remaining fuel (?P<fuel>0)"
    )
    stderr_text = visit_stderr.read_text(encoding="utf-8", errors="replace")
    markers = list(marker.finditer(stderr_text))
    need(markers, "source-visit test stderr lacks the P129 exact prefix marker")
    observed = markers[-1].groupdict()
    need(int(observed["events"]) == 35 and int(observed["roundtrips"]) == 1,
         "P129 third-visit marker differs from the required charged prefix")

    builds = {}
    for label, path, expected_mode, expected_features in (
        ("default", default_build, "default", []),
        ("execution_reference", optional_build, "optional-feature",
         ["wtime-runtime/execution-reference"]),
    ):
        record = json.loads(path.resolve(strict=True).read_text(encoding="utf-8"))
        need(record.get("mode") == expected_mode and record.get("features") == expected_features,
             f"{label} build-budget report has the wrong feature configuration")
        need(record.get("source_stable") is True and record.get("budget_pass") is True
             and record.get("default_dependency_closure_pass") is True,
             f"{label} build-budget report failed source, budget, or dependency policy")
        builds[label] = ref(path)

    report = {
        "schema": "wtime-p129-verification-v1",
        "status": "passed",
        "timing_evidence": False,
        "performance_candidate": False,
        "speed_decision": None,
        "settings": {"instrumented": False, "feature_tests": ["execution-reference"],
                     "default_features": []},
        "counts": {"source_visit_named_cases": len(VISIT_TESTS),
                   "cursor_compatibility_named_cases": len(CURSOR_COMPAT_TESTS),
                   "default_named_cases": 1},
        "visit_marker": {"source_visit": "third", "charged_events": int(observed["events"]),
                         "roundtrips": int(observed["roundtrips"]),
                         "remaining_fuel": int(observed["fuel"])},
        "test_logs": {"source_visit": ref(visit_log),
                      "source_visit_stderr": ref(visit_stderr),
                      "cursor_compatibility": ref(compatibility_log),
                      "default": ref(default_log)},
        "build_budgets": builds,
    }
    write_new(output / "report.json", report)
    return report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--visit-log", type=Path)
    parser.add_argument("--visit-stderr", type=Path)
    parser.add_argument("--compatibility-log", type=Path)
    parser.add_argument("--default-log", type=Path)
    parser.add_argument("--default-build", type=Path)
    parser.add_argument("--optional-build", type=Path)
    args = parser.parse_args()
    try:
        if args.visit_log is None:
            result = freeze(args.output)
        else:
            required = (args.visit_stderr, args.compatibility_log, args.default_log,
                        args.default_build, args.optional_build)
            need(all(item is not None for item in required),
                 "verify mode requires all test logs and both build reports")
            result = verify(args.output, args.visit_log, args.visit_stderr,
                            args.compatibility_log, args.default_log,
                            args.default_build, args.optional_build)
        print(json.dumps({"status": result["status"], "schema": result["schema"]}, sort_keys=True))
        return 0
    except (OSError, ValueError, KeyError, json.JSONDecodeError) as error:
        print(f"p129.py: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
