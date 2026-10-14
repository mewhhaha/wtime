#!/usr/bin/env python3
"""Freeze the P136 predecessor and verify source-bound paired smoke receipts."""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import sys
from typing import Any

ROOT = Path(__file__).resolve().parents[3]
ARCHIVE_REL = "target/performance/P136-before"
ARCHIVE_COMMIT = "d2bcb28107d56f7c321312f82b4bfee8a11e970e"
ARCHIVE_MANIFEST_SHA256 = "9fe9882d9c7e23b833d769a8edddabcfe92bf04c0633d153c1e5e532e7537335"
HARNESS_REL = "tools/wtime-bench"
CALL_COUNTS = (1, 10, 100, 10000)
STRATEGIES = (("eager", 0), ("adaptive", 3))
PAIR_COUNT = 2
RUN_SEEDS = (136001, 136002)
CASES = (
    "p136_source_bound_register_adapter_rejects_foreign_or_stale_inputs",
    "p136_candidate_and_adapter_share_protocol_driver_config_and_toolchain",
    "p136_register_settings_forward_to_both_distinct_sides",
    "p136_fixed_fresh_batch_rows_keep_exact_calls_without_calibration",
    "p136_pairs_join_only_identical_fixture_and_settings",
    "p136_receipts_reject_stale_duplicate_missing_or_smoke_only_pairs",
)


class P136Error(ValueError):
    """P136 inputs or retained source-bound evidence are invalid."""


def need(condition: bool, message: str) -> None:
    if not condition:
        raise P136Error(message)


def sha(path: Path) -> str:
    digest = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def canonical_sha(value: Any) -> str:
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":"),
                                   allow_nan=False).encode("utf-8")).hexdigest()


def ref(path: Path) -> dict[str, Any]:
    path = Path(path).resolve(strict=True)
    return {"path": str(path), "sha256": sha(path), "bytes": path.stat().st_size}


def load_json(path: Path, label: str) -> dict[str, Any]:
    try:
        result = json.loads(Path(path).read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise P136Error(f"{label} is not valid JSON: {exc}") from exc
    need(isinstance(result, dict), f"{label} must be a JSON object")
    return result


def write_new(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("x", encoding="utf-8") as stream:
        json.dump(value, stream, indent=2, sort_keys=True, allow_nan=False)
        stream.write("\n")


def start_output(path: Path) -> Path:
    path = Path(path).resolve()
    store = (ROOT / "target/verification").resolve()
    need(path.is_relative_to(store), "P136 output must stay under target/verification")
    path.mkdir(parents=True, exist_ok=False)
    helper = path / "helper.executed.py"
    helper.write_bytes(Path(__file__).read_bytes())
    write_new(path / "invocation.json", {"argv": list(sys.argv), "cwd": str(Path.cwd().resolve())})
    return path


def load_module(path: Path, name: str):
    spec = importlib.util.spec_from_file_location(name, path)
    need(spec is not None and spec.loader is not None, f"cannot load helper {path}")
    module = importlib.util.module_from_spec(spec)
    sys.dont_write_bytecode = True
    spec.loader.exec_module(module)
    return module


def validate_archive() -> dict[str, Any]:
    baseline = load_module(ROOT / "tools/wtime-bench/baseline.py", "p136_archive_validator")
    archive = (ROOT / ARCHIVE_REL).resolve(strict=True)
    manifest_path = archive / "manifest.json"
    need(sha(manifest_path) == ARCHIVE_MANIFEST_SHA256, "P136-before manifest identity changed")
    manifest = baseline.verify_archive(archive)
    need(manifest.get("repository_commit") == ARCHIVE_COMMIT
         and manifest.get("mode") == "register"
         and manifest.get("execution_policy", {}).get("resolved_mode") == "register",
         "P136-before is not the pinned immutable Register predecessor")
    p135 = load_module(ROOT / "tools/verification/producers/p135.py", "p136_runtime_map_helper")
    runtime = p135.runtime_source_map(manifest)
    need({"Cargo.toml", "Cargo.lock", "rust-toolchain.toml"} <= set(runtime),
         "P136-before omits root Cargo/toolchain source inputs")
    for crate in ("wtime-runtime", "wtime-types", "wtime-decode", "wtime-validate"):
        need(f"crates/{crate}/Cargo.toml" in runtime
             and any(path.startswith(f"crates/{crate}/") and path.endswith(".rs") for path in runtime),
             f"P136-before omits {crate} runtime source")
    return {"archive": ref(manifest_path), "archive_path": str(archive),
            "repository_commit": ARCHIVE_COMMIT, "mode": "register",
            "file_count": len(manifest.get("files", {})),
            "archive_runtime_source_map": runtime,
            "archive_runtime_source_map_sha256": canonical_sha(runtime)}


def current_candidate_source_map() -> dict[str, str]:
    adapter = _adapter_module()
    paths = adapter._current_source_paths(ROOT)
    return {rel: sha(path) for rel, path in paths.items()}


def freeze(output: Path) -> dict[str, Any]:
    output = start_output(output)
    archive = validate_archive()
    runtime = current_candidate_source_map()
    runtime_hash = canonical_sha(runtime)
    runtime_path = output / "runtime.json"
    write_new(runtime_path, {"runtime_source_map": runtime,
                             "runtime_source_map_sha256": runtime_hash})
    report = {"schema": "wtime-p136-freeze-v1", "status": "passed",
              "timing_evidence": False, **archive, "runtime_source_map": runtime,
              "runtime_source_map_sha256": runtime_hash, "runtime_map": ref(runtime_path)}
    write_new(output / "report.json", report)
    return report


def _load_inputs(freeze_report: Path) -> tuple[dict[str, Any], Path, Path]:
    frozen = load_json(freeze_report, "P136 freeze report")
    need(frozen.get("schema") == "wtime-p136-freeze-v1" and frozen.get("status") == "passed"
         and frozen.get("timing_evidence") is False,
         "P136 freeze receipt is invalid or promoting")
    actual = validate_archive()
    need(frozen.get("archive") == actual["archive"]
         and frozen.get("archive_path") == actual["archive_path"]
         and frozen.get("repository_commit") == ARCHIVE_COMMIT,
         "P136 archive identity changed after freeze")
    current_runtime = current_candidate_source_map()
    need(frozen.get("runtime_source_map") == current_runtime
         and frozen.get("runtime_source_map_sha256") == canonical_sha(current_runtime)
         and frozen.get("runtime_map") == ref(Path(frozen["runtime_map"]["path"]))
         and load_json(Path(frozen["runtime_map"]["path"]), "P136 frozen runtime map").get("runtime_source_map")
         == current_runtime,
         "P136 frozen runtime map is stale or altered")
    return frozen, Path(actual["archive_path"]), ROOT / HARNESS_REL


def _adapter_module():
    return load_module(ROOT / "tools/wtime-bench/entry_batch_adapter.py", "p136_entry_batch_adapter")


def _collector_module():
    bench_dir = str(ROOT / "tools/wtime-bench")
    if bench_dir not in sys.path:
        sys.path.insert(0, bench_dir)
    return load_module(ROOT / "tools/wtime-bench/entry_batch_compare.py", "p136_entry_batch_compare")


def _rows(fixture: Path) -> list[dict[str, Any]]:
    return [{"fixture": str(fixture.resolve(strict=True)), "case": "p135_counter", "input": 7,
             "expected": 8, "calls": count, "strategy": strategy, "threshold": threshold}
            for strategy, threshold in STRATEGIES for count in CALL_COUNTS]


def _validate_collection(collector, report: dict[str, Any], *, output: Path, adapter: dict,
                         candidate: dict, rows: list[dict[str, Any]]) -> None:
    need(report.get("timing_evidence") is False and report.get("same_source_alias") is False,
         "P136 smoke collection is marked as candidate timing evidence")
    need(report.get("pair_count") == len(rows) * PAIR_COUNT * len(RUN_SEEDS),
         "P136 smoke pair count or independent run count is incorrect")
    identity = report.get("identity", {})
    need(identity.get("pairs_per_row") == PAIR_COUNT
         and identity.get("seeds") == list(RUN_SEEDS)
         and identity.get("rows") == collector.normalize_rows(rows),
         "P136 smoke report does not retain the exact explicit fixture/settings matrix")
    need(identity.get("sides", {}).get("baseline", {}).get("executable", {}).get("sha256") == adapter.get("executable_sha256")
         and identity.get("sides", {}).get("candidate", {}).get("executable", {}).get("sha256") == candidate.get("executable_sha256"),
         "P136 smoke report executable identities differ from verified build records")
    need(report.get("schema") == "wtime-register-entry-pairs-v1"
         and report.get("passed") is True and report.get("source_stable") is True
         and report.get("metric") == "entry_total",
         "P136 smoke collection did not pass source-bound validation")
    collector.validate_report(report, output / "collection")
    report_path = output / "collection/report.json"
    need(report_path.is_file(), "P136 collector did not retain its canonical report.json")
    need(load_json(report_path, "P136 retained collection report") == report,
         "P136 in-memory collection differs from retained report.json")


def verify(output: Path, freeze_report: Path, *, smoke: bool = True) -> dict[str, Any]:
    need(smoke, "P136 verification requires the declared non-promoting smoke")
    output = start_output(output)
    frozen, archive, harness = _load_inputs(freeze_report)
    adapter_api = _adapter_module()
    adapter_root = output / "archived-adapter"
    candidate_root = output / "candidate-adapter"
    adapter = adapter_api.build(archive, harness, adapter_root)
    adapter = adapter_api.verify(adapter_root, archive, harness)
    candidate = adapter_api.build_current(ROOT, harness, candidate_root)
    candidate = adapter_api.verify_current(candidate_root, ROOT, harness)

    fixture = harness / "fixtures/p135/counter.wasm"
    need(fixture.is_file(), "P135 counter fixture is missing from the P136 harness")
    rows = _rows(fixture)
    collector = _collector_module()
    comparison = collector.collect(archive, adapter_root, candidate_root, ROOT, harness,
                                   rows, PAIR_COUNT, list(RUN_SEEDS), output / "collection",
                                   smoke=True, same_source_smoke=False, resume=False)
    _validate_collection(collector, comparison, output=output, adapter=adapter, candidate=candidate, rows=rows)
    comparison = collector.verify_collection(output / "collection", archive, adapter_root, candidate_root,
                                             ROOT, harness, rows, PAIR_COUNT, list(RUN_SEEDS),
                                             smoke=True, same_source_smoke=False)

    # Verify the builder inputs again after guest execution to reject source drift.
    adapter_api.verify(adapter_root, archive, harness)
    candidate = adapter_api.verify_current(candidate_root, ROOT, harness)
    need(candidate.get("executable_sha256") == comparison["identity"]["sides"]["candidate"]["executable"]["sha256"],
         "P136 candidate source or executable changed during smoke collection")
    freeze_identity = load_json(freeze_report, "P136 freeze report")
    need(freeze_identity.get("archive") == ref(archive / "manifest.json"),
         "P136 predecessor archive changed during verification")

    p135 = load_module(ROOT / "tools/verification/producers/p135.py", "p136_p135_producer")
    inherited, build_inputs = p135.validate_p134_build_budget_reuse()
    build_inputs_sha = canonical_sha(build_inputs)
    need(build_inputs_sha == p135.canonical_sha(build_inputs),
         "P136 reused core build source-input identity is malformed")
    adapter_ref = ref(adapter_root / "adapter.json")
    candidate_ref = ref(candidate_root / "adapter.json")
    collection_ref = ref(output / "collection/report.json")
    report = {
        "schema": "wtime-p136-verification-v1", "status": "passed", "timing_evidence": False,
        "decision": "integrity", "smoke": True, "same_source_smoke": False,
        "runtime_source_map_sha256": frozen["runtime_source_map_sha256"],
        "predecessor": freeze_identity["archive"], "adapter": adapter_ref,
        "candidate": candidate_ref, "collection": collection_ref,
        "counts": {"rows": len(rows), "pairs": comparison["pair_count"], "runs": len(RUN_SEEDS),
                   "named_cases": len(CASES)},
        "core_build_budgets_reused": True, "inherited_core_builds": inherited,
        "build_source_inputs_sha256": build_inputs_sha,
        "build_budget_note": "P134/P135 core build-budget evidence is reused only; P136 makes no core build-time claim.",
        "adapter_executable_sha256": adapter["executable_sha256"],
        "candidate_executable_sha256": candidate["executable_sha256"],
        "protocol_sha256": adapter["protocol_sha256"],
        "entry_batch_driver_sha256": adapter["entry_batch_driver_sha256"],
        "source_stable": True,
    }
    write_new(output / "report.json", report)
    write_new(output / "evidence.json", report)
    return verify_evidence(output / "report.json")


def verify_evidence(path: Path) -> dict[str, Any]:
    """Strictly validate a completed P136 verification summary and its artifacts."""
    report = load_json(path, "P136 verification evidence")
    need(report.get("schema") == "wtime-p136-verification-v1" and report.get("status") == "passed"
         and report.get("timing_evidence") is False and report.get("decision") == "integrity"
         and report.get("smoke") is True and report.get("same_source_smoke") is False
         and report.get("source_stable") is True,
         "P136 evidence envelope is invalid or promoting")
    for field in ("adapter", "candidate", "collection", "predecessor"):
        record = report.get(field)
        need(isinstance(record, dict), f"P136 evidence omits {field} identity")
        artifact = Path(record.get("path", "")).resolve(strict=True)
        need(record == ref(artifact), f"P136 {field} artifact hash/size/path is stale")
    counts = report.get("counts", {})
    need(counts.get("rows") == 8 and counts.get("pairs") == 32 and counts.get("runs") == 2
         and counts.get("named_cases") == len(CASES),
         "P136 evidence count summary is incomplete")
    adapter = load_json(Path(report["adapter"]["path"]), "P136 adapter manifest")
    candidate = load_json(Path(report["candidate"]["path"]), "P136 candidate manifest")
    need(adapter.get("kind") == "source-bound-register-entry-adapter"
         and candidate.get("kind") == "source-bound-register-entry-candidate"
         and report.get("adapter_executable_sha256") == adapter.get("executable_sha256")
         and report.get("candidate_executable_sha256") == candidate.get("executable_sha256"),
         "P136 evidence does not bind distinct verified Register executables")
    collection = load_json(Path(report["collection"]["path"]), "P136 collection report")
    need(collection.get("schema") == "wtime-register-entry-pairs-v1"
         and collection.get("passed") is True and collection.get("source_stable") is True
         and collection.get("timing_evidence") is False and collection.get("same_source_alias") is False
         and collection.get("identity", {}).get("smoke") is True,
         "P136 evidence collection is stale, incomplete, or promoting")
    collection_path = Path(report["collection"]["path"]).resolve(strict=True)
    output = collection_path.parent
    adapter_root = Path(report["adapter"]["path"]).resolve(strict=True).parent
    candidate_root = Path(report["candidate"]["path"]).resolve(strict=True).parent
    archive = (ROOT / ARCHIVE_REL).resolve(strict=True)
    harness = (ROOT / HARNESS_REL).resolve(strict=True)
    rows = _rows(harness / "fixtures/p135/counter.wasm")
    collector = _collector_module()
    actual = collector.verify_collection(output, archive, adapter_root, candidate_root, ROOT, harness,
                                         rows, PAIR_COUNT, list(RUN_SEEDS),
                                         smoke=True, same_source_smoke=False)
    need(actual == collection, "P136 collection report or journal differs from current source-bound inputs")
    adapter_api = _adapter_module()
    adapter_now = adapter_api.verify(adapter_root, archive, harness)
    candidate_now = adapter_api.verify_current(candidate_root, ROOT, harness)
    need(adapter_now.get("executable_sha256") == report.get("adapter_executable_sha256")
         and candidate_now.get("executable_sha256") == report.get("candidate_executable_sha256")
         and report.get("protocol_sha256") == adapter_now.get("protocol_sha256")
         and report.get("entry_batch_driver_sha256") == adapter_now.get("entry_batch_driver_sha256"),
         "P136 adapter or candidate source/executable identity changed")
    p135 = load_module(ROOT / "tools/verification/producers/p135.py", "p136_p135_producer_evidence")
    inherited, build_inputs = p135.validate_p134_build_budget_reuse()
    need(report.get("core_build_budgets_reused") is True
         and report.get("inherited_core_builds") == inherited
         and report.get("build_source_inputs_sha256") == canonical_sha(build_inputs),
         "P136 reused core build-budget source identity is stale")
    need(report.get("predecessor") == ref(archive / "manifest.json"),
         "P136 predecessor manifest identity changed")
    source_map = current_candidate_source_map()
    need(report.get("runtime_source_map_sha256") == canonical_sha(source_map),
         "P136 current candidate core source map changed")
    return report


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    freeze_parser = sub.add_parser("freeze")
    freeze_parser.add_argument("--archive", type=Path, default=ROOT / ARCHIVE_REL)
    freeze_parser.add_argument("--output", type=Path, required=True)
    verify_parser = sub.add_parser("verify")
    verify_parser.add_argument("--freeze", dest="freeze_report", type=Path, required=True)
    verify_parser.add_argument("--output", type=Path, required=True)
    verify_parser.add_argument("--smoke", action="store_true")
    args = parser.parse_args(argv)
    try:
        if args.command == "freeze":
            need(args.archive.resolve(strict=True) == (ROOT / ARCHIVE_REL).resolve(strict=True),
                 "P136 freeze accepts only the pinned immediate-predecessor archive")
            result = freeze(args.output)
        else:
            result = verify(args.output, args.freeze_report, smoke=args.smoke)
        print(json.dumps(result, sort_keys=True, allow_nan=False))
        return 0
    except (P136Error, OSError, KeyError, TypeError, ValueError, RuntimeError) as exc:
        print(f"P136 verification failed: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
