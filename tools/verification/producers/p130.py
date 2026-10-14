#!/usr/bin/env python3
"""Freeze and verify P130 bounded parallel-move scheduling evidence."""
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

PREDECESSOR = "target/performance/P130-before"
PREDECESSOR_MANIFEST_SHA256 = "c9dbdd666e4485aea7d1003a063175295297b12f946f42e0060088673b67b794"
P001_MANIFEST_SHA256 = "7349596fc806c96c74cb9c40f1c08402589c1683660b0b152c713264f5808472"
P001_BINARY_SHA256 = "735e595f3a625da89401fdee298781b752fbe071e3d3d793df196c9e68b8e498"
P057_COMPLETION_SHA256 = "4ee850eeaeac86b302acd543eaace6b0fa96d02d2655cc112fb00c01b703d481"
P057_EVIDENCE_SHA256 = "8061225076d2226e948b23d51f951ef90a5443edf17aa0a685ab08e2cd716264"
P058_COMPLETION_SHA256 = "83e4626cecf912026f5bba6b34da49a8c49e53963e9d5e0d2a05d3e08e3b0443"
P058_EVIDENCE_SHA256 = "d77c102b292744889128bf4b1e4f1f66767e0a9681d063f0aa0841b4e2bc3a20"
P130_TESTS = (
    "p130_identity_and_acyclic_moves_preserve_simultaneous_results",
    "p130_two_and_three_slot_cycles_use_bounded_scratch",
    "p130_invalid_requests_and_budget_failures_publish_nothing",
    "p130_bounded_generated_graphs_match_simultaneous_oracle",
)
DEFAULT_TEST = "p130_register_policy_stays_unavailable_without_execution"
P057_TESTS = (
    "p057_numeric_loop_has_bounded_typed_slots_and_source_boundaries",
    "p057_unsupported_types_and_effects_decline_without_guest_work",
    "p057_bad_metadata_and_aggregate_budgets_are_transactional",
    "p057_register_policy_stays_unavailable_until_execution_lands",
)
P058_TESTS = (
    "p058_local_numeric_slots_match_every_decoded_prefix",
    "p058_multivalue_edges_and_loop_visits_match_decoded_prefixes",
    "p058_declines_invalid_joins_and_preserves_shared_budgets",
    "p058_decoded_fixture_results_preserve_default_policy",
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
    need(path.is_relative_to(store), "P130 output must stay under target/verification")
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


def _completed(task: str, completion_hash: str, evidence_hash: str,
               final_path: Path) -> dict[str, Any]:
    completion_path = ROOT / f"docs/benchmarks/performance/{task}-completion.json"
    evidence_path = ROOT / f"docs/benchmarks/performance/{task}-evidence.json"
    need(sha(completion_path) == completion_hash,
         f"{task} completion reference changed")
    need(sha(evidence_path) == evidence_hash,
         f"{task} evidence reference changed")
    completion = json.loads(completion_path.read_text(encoding="utf-8"))
    evidence = json.loads(evidence_path.read_text(encoding="utf-8"))
    need(completion.get("task") == task and completion.get("status") == "done",
         f"{task} completion is not an immutable completed record")
    need(completion.get("artifact_hashes", {}).get(str(final_path)) == evidence_hash
         and sha(final_path) == evidence_hash,
         f"local {task} final evidence differs from its completion reference")
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
         "P130 immediate-predecessor manifest identity changed")
    manifest = baseline.verify_archive(predecessor)
    need(manifest.get("mode") == "decoded", "P130 predecessor is not decoded mode")
    source_map = runtime_source_map(manifest)
    need(ROOT_SOURCES <= set(source_map), "P130 predecessor omits root Cargo/toolchain inputs")
    for crate in CORE_CRATES:
        need(f"crates/{crate}/Cargo.toml" in source_map and any(
            path.startswith(f"crates/{crate}/") and path.endswith(".rs") for path in source_map),
            f"P130 predecessor omits {crate} source inputs")

    p001 = (ROOT / "target/performance/baseline").resolve(strict=True)
    p001_manifest = baseline.verify_archive(p001)
    need(sha(p001 / "manifest.json") == P001_MANIFEST_SHA256
         and sha(p001 / "binary/wtime-bench") == P001_BINARY_SHA256,
         "immutable P001 archive identity changed")
    need(p001_manifest.get("smoke", {}).get("correctness_checked") is True,
         "immutable P001 archive lacks successful correctness smoke")

    p057 = _completed("P057", P057_COMPLETION_SHA256, P057_EVIDENCE_SHA256,
                      ROOT / "target/performance/P057/shared-finalization/attempt-0001/evidence.json")
    p058 = _completed("P058", P058_COMPLETION_SHA256, P058_EVIDENCE_SHA256,
                      ROOT / "target/performance/P058/shared-finalization/attempt-0002/evidence.json")
    report = {
        "schema": "wtime-p130-freeze-v1",
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
        "p057": p057,
        "p058": p058,
    }
    write_new(output / "report.json", report)
    return report


def require_tests(path: Path, names: tuple[str, ...], label: str) -> None:
    text = path.read_text(encoding="utf-8", errors="replace")
    for name in names:
        need(re.search(rf"^test\s+[^\n]*{re.escape(name)}\s+\.\.\. ok$",
                       text, re.MULTILINE) is not None,
             f"{label} log lacks passing named case {name}")


def verify(output: Path, move_log: Path, move_stderr: Path,
           p058_log: Path, p057_log: Path, default_log: Path,
           default_build: Path, register_build: Path) -> dict[str, Any]:
    output = start_output(output)
    logs = {"moves": move_log.resolve(strict=True),
            "p058_compatibility": p058_log.resolve(strict=True),
            "p057_compatibility": p057_log.resolve(strict=True),
            "default": default_log.resolve(strict=True)}
    require_tests(logs["moves"], P130_TESTS, "P130 parallel-move")
    require_tests(logs["moves"], (DEFAULT_TEST,), "P130 public default")
    require_tests(logs["p058_compatibility"], P058_TESTS, "P058 compatibility")
    require_tests(logs["p057_compatibility"], P057_TESTS, "P057 compatibility")
    require_tests(logs["default"], (DEFAULT_TEST,), "default")

    move_stderr = move_stderr.resolve(strict=True)
    marker = re.compile(
        r"P130: (?P<graphs>[1-9][0-9]*) generated move graphs; "
        r"(?P<scheduled_steps>[1-9][0-9]*) scheduled steps; "
        r"(?P<scratch_cycles>[1-9][0-9]*) scratch cycles; "
        r"(?P<charged_work>[1-9][0-9]*) charged work; "
        r"(?P<retained_bytes>[1-9][0-9]*) retained bytes"
    )
    matches = list(marker.finditer(move_stderr.read_text(encoding="utf-8", errors="replace")))
    need(len(matches) == 1, "P130 move tests must emit exactly one aggregate marker")
    values = {name: int(value) for name, value in matches[0].groupdict().items()}
    need(values["graphs"] == 4096, "P130 marker must cover all 4096 generated move graphs")

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
        "schema": "wtime-p130-verification-v1",
        "status": "passed",
        "timing_evidence": False,
        "performance_candidate": False,
        "speed_decision": None,
        "settings": {"instrumented": False,
                     "features": ["register-executor", "execution-reference"],
                     "default_features": []},
        "counts": {"move_named_cases": len(P130_TESTS),
                   "p058_compatibility_named_cases": len(P058_TESTS),
                   "p057_compatibility_named_cases": len(P057_TESTS),
                   "default_named_cases": 1},
        "move_marker": values,
        "test_logs": {name: ref(path) for name, path in logs.items()},
        "move_stderr": ref(move_stderr),
        "build_budgets": builds,
    }
    write_new(output / "report.json", report)
    return report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--move-log", type=Path)
    parser.add_argument("--move-stderr", type=Path)
    parser.add_argument("--p058-log", type=Path)
    parser.add_argument("--p057-log", type=Path)
    parser.add_argument("--default-log", type=Path)
    parser.add_argument("--default-build", type=Path)
    parser.add_argument("--register-build", type=Path)
    args = parser.parse_args()
    try:
        if args.move_log is None:
            result = freeze(args.output)
        else:
            required = (args.move_stderr, args.p058_log, args.p057_log, args.default_log,
                        args.default_build, args.register_build)
            need(all(item is not None for item in required),
                 "verify mode requires all test logs and both build reports")
            result = verify(args.output, args.move_log, args.move_stderr,
                            args.p058_log, args.p057_log, args.default_log,
                            args.default_build, args.register_build)
        print(json.dumps({"status": result["status"], "schema": result["schema"]}, sort_keys=True))
        return 0
    except (OSError, ValueError, KeyError, json.JSONDecodeError) as error:
        print(f"p130.py: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
