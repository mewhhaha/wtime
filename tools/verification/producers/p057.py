#!/usr/bin/env python3
"""Freeze and verify P057's bounded register-IR contract evidence."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import sys
from typing import Any

ROOT = Path(__file__).resolve().parents[3]
BENCH = ROOT / "tools/wtime-bench"
if str(BENCH) not in sys.path:
    sys.path.insert(0, str(BENCH))

PREDECESSOR = "target/performance/P057-before"
PREDECESSOR_MANIFEST_SHA256 = "4db2c20feb62b92ce80c6d69cac955c622e875b111f35d818be76b1b4337103d"
P001_MANIFEST_SHA256 = "7349596fc806c96c74cb9c40f1c08402589c1683660b0b152c713264f5808472"
P001_BINARY_SHA256 = "735e595f3a625da89401fdee298781b752fbe071e3d3d793df196c9e68b8e498"
P056_COMPLETION_SHA256 = "fcf8367fa9188b254724b1f1958fc912eb54f5f23bd051875813ca5e02a3d370"
P056_EVIDENCE_SHA256 = "1d0b3bef28b5ea5fb6b6fb15ec12919c03e180e1e5cae07a615962d864586b5d"
FEATURE_TESTS = (
    "p057_numeric_loop_has_bounded_typed_slots_and_source_boundaries",
    "p057_unsupported_types_and_effects_decline_without_guest_work",
    "p057_bad_metadata_and_aggregate_budgets_are_transactional",
    "p057_register_policy_stays_unavailable_until_execution_lands",
)
DEFAULT_TEST = "p057_register_policy_stays_unavailable_until_execution_lands"
CORE_CRATES = ("wtime-runtime", "wtime-types", "wtime-decode", "wtime-validate")
ROOT_SOURCES = {"Cargo.toml", "Cargo.lock", "rust-toolchain.toml"}


def _start_output(path: Path) -> Path:
    path = path.resolve()
    store = (ROOT / "target/verification").resolve()
    need(path.is_relative_to(store), "P057 output must stay under target/verification")
    path.mkdir(parents=True, exist_ok=False)
    source_copy = path / "helper.executed.py"
    with source_copy.open("xb") as stream:
        stream.write(Path(__file__).read_bytes())
    write_new(path / "invocation.json", {"argv": list(sys.argv), "cwd": str(Path.cwd().resolve())})
    return path


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


def need(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def runtime_source_map(manifest: dict[str, Any]) -> dict[str, str]:
    prefixes = tuple(f"crates/{crate}/" for crate in CORE_CRATES)
    expected = {}
    for row in manifest["files"].values():
        source_path = row.get("source_path", "")
        if (row.get("role") == "source" and
                (source_path in ROOT_SOURCES or
                 (source_path.endswith("Cargo.toml") and any(
                     source_path == f"crates/{crate}/Cargo.toml" for crate in CORE_CRATES)) or
                 (source_path.endswith(".rs") and source_path.startswith(prefixes)))):
            expected[source_path] = row["sha256"]
    return dict(sorted(expected.items()))


def freeze(output: Path) -> dict[str, Any]:
    output = _start_output(output)
    import baseline

    predecessor = (ROOT / PREDECESSOR).resolve(strict=True)
    manifest_path = predecessor / "manifest.json"
    need(sha(manifest_path) == PREDECESSOR_MANIFEST_SHA256,
         "P057 immediate-predecessor manifest identity changed")
    manifest = baseline.verify_archive(predecessor)
    need(manifest.get("mode") == "decoded", "P057 predecessor is not decoded mode")
    source_map = runtime_source_map(manifest)
    need(source_map, "P057 predecessor has no archived runtime source map")
    archived_source_paths = set(source_map)
    need(ROOT_SOURCES <= archived_source_paths, "predecessor omits root Cargo/toolchain inputs")
    for crate in CORE_CRATES:
        need(f"crates/{crate}/Cargo.toml" in archived_source_paths and any(
            path.startswith(f"crates/{crate}/") and path.endswith(".rs")
            for path in archived_source_paths), f"predecessor omits {crate} source inputs")

    p001 = (ROOT / "target/performance/baseline").resolve(strict=True)
    p001_manifest = baseline.verify_archive(p001)
    need(sha(p001 / "manifest.json") == P001_MANIFEST_SHA256
         and sha(p001 / "binary/wtime-bench") == P001_BINARY_SHA256,
         "immutable P001 archive identity changed")
    need(p001_manifest.get("smoke", {}).get("correctness_checked") is True,
         "immutable P001 archive lacks successful correctness smoke")

    completion_path = ROOT / "docs/benchmarks/performance/P056-completion.json"
    evidence_path = ROOT / "docs/benchmarks/performance/P056-evidence.json"
    need(sha(completion_path) == P056_COMPLETION_SHA256,
         "P056 completion reference changed")
    need(sha(evidence_path) == P056_EVIDENCE_SHA256,
         "P056 evidence reference changed")
    completion = json.loads(completion_path.read_text(encoding="utf-8"))
    evidence = json.loads(evidence_path.read_text(encoding="utf-8"))
    need(completion.get("task") == "P056" and completion.get("status") == "done",
         "P056 completion is not an immutable completed record")
    final_evidence_path = ROOT / "target/performance/P056/shared-finalization/attempt-0013/evidence.json"
    need(completion.get("artifact_hashes", {}).get(str(final_evidence_path)) == P056_EVIDENCE_SHA256,
         "P056 completion does not bind its final evidence")
    need(sha(final_evidence_path) == P056_EVIDENCE_SHA256,
         "local P056 final evidence differs from its immutable completion reference")
    need(evidence.get("task") == "P056" and evidence.get("status") == "passed",
         "P056 final evidence is not passed")

    report = {
        "schema": "wtime-p057-freeze-v1",
        "status": "passed",
        "timing_evidence": False,
        "predecessor": ref(manifest_path),
        "predecessor_mode": "decoded",
        "runtime_source_map": source_map,
        "runtime_source_file_count": len(source_map),
        "p001": {
            "manifest": ref(p001 / "manifest.json"),
            "binary": ref(p001 / "binary/wtime-bench"),
            "manifest_sha256_pinned": P001_MANIFEST_SHA256,
            "binary_sha256_pinned": P001_BINARY_SHA256,
        },
        "p056": {"completion": ref(completion_path), "evidence": ref(evidence_path),
                 "final_evidence": ref(final_evidence_path)},
    }
    write_new(output / "report.json", report)
    return report


def verify(output: Path, feature_log: Path, feature_stderr: Path, default_log: Path,
           default_build: Path, optional_build: Path) -> dict[str, Any]:
    output = _start_output(output)
    logs = {"feature": feature_log.resolve(strict=True),
            "default": default_log.resolve(strict=True)}
    feature_text = logs["feature"].read_text(encoding="utf-8", errors="replace")
    feature_stderr = feature_stderr.resolve(strict=True)
    default_text = logs["default"].read_text(encoding="utf-8", errors="replace")
    for name in FEATURE_TESTS:
        need(re.search(rf"^test\s+[^\n]*{re.escape(name)}\s+\.\.\. ok$",
                       feature_text, re.MULTILINE) is not None,
             f"feature test log lacks passing named case {name}")
    need(re.search(rf"^test\s+[^\n]*{re.escape(DEFAULT_TEST)}\s+\.\.\. ok$",
                   default_text, re.MULTILINE) is not None,
         "default test log lacks the default-safe named case")
    marker = re.compile(
        r"P057: (?P<events>[1-9][0-9]*) source events; "
        r"(?P<slots>[1-9][0-9]*) physical slots; logical peak "
        r"(?P<logical>[1-9][0-9]*); (?P<bytes>[1-9][0-9]*) bytes; "
        r"(?P<work>[1-9][0-9]*) work"
    )
    marker_text = feature_stderr.read_text(encoding="utf-8", errors="replace")
    markers = list(marker.finditer(marker_text))
    need(markers, "feature test stderr lacks the P057 bounded/source marker")
    measured = markers[-1].groupdict()
    result_counts = {key: int(value) for key, value in measured.items()}
    need(result_counts["events"] == 25 and result_counts["slots"] == 5
         and result_counts["logical"] == 2,
         "P057 event or slot marker differs from the bounded source layout")

    builds = {}
    for label, path, expected_mode, expected_features in (
        ("default", default_build, "default", []),
        ("register", optional_build, "optional-feature", ["wtime-runtime/register-executor"]),
    ):
        record = json.loads(path.resolve(strict=True).read_text(encoding="utf-8"))
        need(record.get("mode") == expected_mode and record.get("features") == expected_features,
             f"{label} build-budget report has the wrong feature configuration")
        need(record.get("source_stable") is True and record.get("budget_pass") is True
             and record.get("default_dependency_closure_pass") is True,
             f"{label} build-budget report failed source, budget, or dependency policy")
        builds[label] = ref(path)
    report = {
        "schema": "wtime-p057-verification-v1",
        "status": "passed",
        "timing_evidence": False,
        "performance_candidate": False,
        "speed_decision": None,
        "settings": {"instrumented": False,
                     "features": ["register-executor", "execution-reference"]},
        "counts": {"feature_named_cases": len(FEATURE_TESTS), "default_named_cases": 1},
        "bounded_ir_marker": result_counts,
        "test_logs": {name: ref(path) for name, path in logs.items()},
        "feature_stderr": ref(feature_stderr),
        "build_budgets": builds,
    }
    write_new(output / "report.json", report)
    return report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--feature-log", type=Path)
    parser.add_argument("--feature-stderr", type=Path)
    parser.add_argument("--default-log", type=Path)
    parser.add_argument("--default-build", type=Path)
    parser.add_argument("--optional-build", type=Path)
    args = parser.parse_args()
    try:
        if args.feature_log is None:
            result = freeze(args.output)
        else:
            required = (args.feature_stderr, args.default_log, args.default_build, args.optional_build)
            need(all(item is not None for item in required),
                 "verify mode requires both test logs and both build reports")
            result = verify(args.output, args.feature_log, args.feature_stderr,
                            args.default_log, args.default_build, args.optional_build)
        print(json.dumps({"status": result["status"], "schema": result["schema"]}, sort_keys=True))
        return 0
    except (OSError, ValueError, KeyError, json.JSONDecodeError) as error:
        print(f"p057.py: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
