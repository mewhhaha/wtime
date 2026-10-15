#!/usr/bin/env python3
"""Source-bound verification for side-specific Register entry settings."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import shutil
import sys
from typing import Any

ROOT = Path(__file__).resolve().parents[3]
ARCHIVE_REL = Path("target/performance/P139-before")
ARCHIVE_COMMIT = "1a41f327580bc3fdbaa8460bb8625ba46ca5a798"
ARCHIVE_MANIFEST_SHA256 = "327564c81bdef90aa3e5958a1b373c7b3778173eb5f2370f16474f14d59b58b4"
SEEDS = (139001, 139002)
CALL_COUNTS = (1, 10, 100, 10000)
ADAPTIVE_THRESHOLDS = (3, 10)
P135_FIXTURE = ROOT / "tools/wtime-bench/fixtures/p135/counter.wasm"
P135_PROVENANCE = ROOT / "tools/wtime-bench/fixtures/p135/provenance.json"
P136_COLLECTION = ROOT / "target/verification/runs/P136/verify/attempt-0001/proof/collection"
P138_PROOF = ROOT / "docs/benchmarks/performance/P138-proof.json"
P138_DEFAULT_BUILD = ROOT / "target/verification/runs/P138/verify/attempt-0002/default-build.json"
P138_REGISTER_BUILD = ROOT / "target/verification/runs/P138/verify/attempt-0002/register-build.json"
P135_PROOF = ROOT / "docs/benchmarks/performance/P135-proof.json"
P135_VALIDATOR_LOGS = ROOT / "target/recovery/P135-fixtures-0002/executed/logs"

P139_FILES = (
    "tools/verification/producers/p139.py",
    "tools/verification/producers/p138.py",
    "tools/verification/producers/p135.py",
    "tools/checks/build_budget.py",
    "tools/checks/dependency_budget.py",
    "tools/verification/tests/test_p139_entry_batch.py",
    "tools/wtime-bench/tests/test_p139_entry_settings.py",
    "tools/wtime-bench/tests/test_p136_entry_batch.py",
    "tools/wtime-bench/tests/test_p137_entry_analysis.py",
    "tools/wtime-bench/entry_batch_adapter.py",
    "tools/wtime-bench/entry_batch_compare.py",
    "tools/wtime-bench/entry_batch_analyze.py",
    "tools/wtime-bench/checkpoint.py",
    "tools/wtime-bench/analyze.py",
    "tools/wtime-bench/bootstrap.py",
    "tools/wtime-bench/baseline.py",
    "tools/wtime-bench/src/protocol.rs",
    "tools/wtime-bench/src/entry_batch_driver.rs",
    "tools/wtime-bench/fixtures/p135/counter.wasm",
    "tools/wtime-bench/fixtures/p135/counter.wat",
    "tools/wtime-bench/fixtures/p135/LICENSE.txt",
    "tools/wtime-bench/fixtures/p135/provenance.json",
    "docs/benchmarks/performance/P135-proof.json",
    "docs/benchmarks/performance/P139-contract.json",
    "docs/benchmarks/performance/P139-verification.json",
)
RETAINED_PROOFS = {
    "P136": ROOT / "target/verification/runs/P136/verify/attempt-0001/proof/report.json",
    "P137": ROOT / "target/verification/runs/P137/verify/attempt-0002/proof/report.json",
    "P138": ROOT / "target/verification/runs/P138/verify/attempt-0002/proof/report.json",
}


class P139Error(ValueError):
    """Invalid or stale P139 evidence."""


def need(ok: bool, message: str) -> None:
    if not ok:
        raise P139Error(message)


def sha(path: Path) -> str:
    digest = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def canonical_sha(value: Any) -> str:
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":"),
                                    allow_nan=False).encode()).hexdigest()


def ref(path: Path) -> dict[str, Any]:
    path = Path(path).resolve(strict=True)
    need(path.is_file(), f"expected a file reference: {path}")
    return {"path": str(path), "sha256": sha(path), "bytes": path.stat().st_size}


def compact_ref(path: Path) -> dict[str, str]:
    path = Path(path).resolve(strict=True)
    need(path.is_file(), f"expected a file reference: {path}")
    return {"path": str(path), "sha256": sha(path)}


def read_json(path: Path, label: str) -> dict[str, Any]:
    try:
        value = json.loads(Path(path).read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise P139Error(f"{label} is invalid: {exc}") from exc
    need(isinstance(value, dict), f"{label} must be a JSON object")
    return value


def read_json_rows(path: Path, label: str) -> list[dict[str, Any]]:
    try:
        value = json.loads(Path(path).read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise P139Error(f"{label} is invalid: {exc}") from exc
    need(isinstance(value, list) and all(isinstance(row, dict) for row in value),
         f"{label} must be an array of objects")
    return value


def write_new(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with Path(path).open("x", encoding="utf-8") as stream:
        json.dump(value, stream, indent=2, sort_keys=True, allow_nan=False)
        stream.write("\n")


def _inside(path: Path, root: Path) -> bool:
    try:
        path.resolve().relative_to(root.resolve())
        return True
    except ValueError:
        return False


def _verification_output(path: Path, *, create: bool) -> Path:
    path = Path(path).absolute()
    store = (ROOT / "target/verification").resolve()
    need(path == path.resolve(), "output path contains a symlink or path escape")
    need(path.is_relative_to(store), "P139 output must stay under target/verification")
    if create:
        path.mkdir(parents=True, exist_ok=False)
        (path / "helpers").mkdir()
        write_new(path / "invocation.json", {"argv": list(sys.argv),
                                              "cwd": str(Path.cwd().resolve())})
    else:
        need(path.is_dir(), "P139 output directory is missing")
        need(_inside(path, store), "P139 output directory escaped target/verification")
    return path


def smoke_rows(fixture: Path = P135_FIXTURE) -> list[dict[str, Any]]:
    """Return the complete two-policy smoke matrix in collector input form."""
    fixture = Path(fixture).resolve(strict=True)
    rows = []
    for calls in CALL_COUNTS:
        for threshold in ADAPTIVE_THRESHOLDS:
            rows.append({
                "fixture": str(fixture), "case": "p135_counter", "input": 7, "expected": 8,
                "calls": calls, "baseline_strategy": "eager", "baseline_threshold": 0,
                "candidate_strategy": "adaptive", "candidate_threshold": threshold,
            })
    return rows


def validate_rows(rows: Any, *, fixture: Path = P135_FIXTURE) -> list[dict[str, Any]]:
    expected = smoke_rows(fixture)
    need(isinstance(rows, list) and len(rows) == 8, "P139 smoke row matrix must contain eight rows")
    need(rows == expected, "P139 smoke rows differ from the declared side-specific matrix")
    return expected


def validate_archive_identity(manifest: dict[str, Any], manifest_sha256: str) -> None:
    need(isinstance(manifest, dict) and manifest.get("schema") == 1
         and manifest.get("mode") == "register", "P139 predecessor is not a Register archive")
    need(manifest.get("repository_commit") == ARCHIVE_COMMIT,
         "P139 predecessor commit differs from the pinned P138-complete commit")
    need(manifest_sha256 == ARCHIVE_MANIFEST_SHA256,
         "P139 predecessor manifest hash differs from the pinned archive")


def validate_prerequisite_envelope(task: str, completion: dict[str, Any],
                                  evidence: dict[str, Any], proof: dict[str, Any]) -> None:
    need(completion.get("task") == task and completion.get("status") == "done"
         and completion.get("outcome") == "contract_only",
         f"{task} completion is not closed contract-only")
    need(evidence.get("task") == task and evidence.get("status") == "passed"
         and evidence.get("passed") is True and evidence.get("outcome") == "contract_only"
         and evidence.get("timing_evidence") is not True
         and evidence.get("candidate_promoted") is not True,
         f"{task} evidence is invalid or promoting")
    need(proof.get("status") == "passed" and proof.get("timing_evidence") is False,
         f"{task} proof is invalid or timing evidence")


def _prerequisite(task: str) -> dict[str, Any]:
    folder = ROOT / "docs/benchmarks/performance"
    completion_path = folder / f"{task}-completion.json"
    evidence_path = folder / f"{task}-evidence.json"
    proof_path = folder / f"{task}-proof.json"
    completion = read_json(completion_path, f"{task} completion")
    evidence = read_json(evidence_path, f"{task} evidence")
    proof = read_json(proof_path, f"{task} proof")
    validate_prerequisite_envelope(task, completion, evidence, proof)
    final_evidence = Path(completion["evidence"][0]["path"])
    need(completion.get("artifact_hashes") == {str(final_evidence): completion["evidence"][0]["sha256"]}
         and ref(final_evidence) == completion["evidence"][0],
         f"{task} completion does not bind its retained evidence")
    need(read_json(final_evidence, f"{task} retained evidence") == evidence,
         f"{task} retained evidence differs from exported evidence")
    retained_proof = RETAINED_PROOFS[task]
    need(read_json(retained_proof, f"{task} retained proof") == proof,
         f"{task} exported proof differs from retained proof")
    if task == "P136":
        need(proof.get("schema") == "wtime-p136-verification-v1"
             and proof.get("smoke") is True and proof.get("same_source_smoke") is False
             and proof.get("source_stable") is True and proof.get("counts") == {
                 "runs": 2, "rows": 8, "pairs": 32, "named_cases": 6},
             "P136 retained evidence is not the expected distinct-source smoke")
    elif task == "P137":
        need(proof.get("schema") == "wtime-p137-verification-v1"
             and proof.get("guest_or_build_launched") is False
             and proof.get("counts") == {"runs": 2, "rows": 8, "pairs": 32},
             "P137 retained evidence is not the expected analysis-only smoke")
    else:
        need(proof.get("schema") == "wtime-p138-verification-v1"
             and proof.get("instrumented") is True and proof.get("benchmark_samples") == 0
             and proof.get("candidate_promoted") is False,
             "P138 retained evidence is not non-timing diagnostics")
    return {"completion": ref(completion_path), "evidence": ref(evidence_path),
            "proof": ref(proof_path), "retained_evidence": ref(final_evidence),
            "retained_proof": ref(retained_proof), "proof_identity_sha256": canonical_sha(proof)}


def _runtime_source_map() -> dict[str, str]:
    import importlib.util

    path = ROOT / "tools/verification/producers/p138.py"
    spec = importlib.util.spec_from_file_location("p139_p138_source_map", path)
    need(spec is not None and spec.loader is not None, "cannot load P138 runtime source mapper")
    module = importlib.util.module_from_spec(spec)
    sys.dont_write_bytecode = True
    spec.loader.exec_module(module)
    return module.current_runtime_source_map()


def _validate_fixture_provenance() -> dict[str, Any]:
    """Bind P135 fixture bytes and the actual recorded MVP validator receipts."""
    provenance = read_json(P135_PROVENANCE, "P135 fixture provenance")
    proof = read_json(P135_PROOF, "P135 fixture validation proof")
    need(proof.get("status") == "passed" and proof.get("timing_evidence") is False,
         "P135 retained fixture proof is not passed")
    need(provenance.get("license") == "MIT" and provenance.get("license_file") == "LICENSE.txt"
         and provenance.get("official_validator") == "wasm-tools validate BINARY --features mvp"
         and provenance.get("wtime_validator") == "target/debug/wtime-probe mvp BINARY (decoded core decoder)"
         and provenance.get("errors_count") == 0,
         "P135 fixture provenance or validation summary changed")
    fixture = provenance.get("fixtures", {}).get("counter", {})
    wat = P135_FIXTURE.with_suffix(".wat")
    license_path = P135_FIXTURE.parent / "LICENSE.txt"
    need(fixture.get("wat") == wat.name and fixture.get("wasm") == P135_FIXTURE.name
         and fixture.get("exports", {}).get("run") == "(i32)->i32"
         and fixture.get("exports", {}).get("count", "").startswith("memory ")
         and sha(P135_FIXTURE) == fixture.get("wasm_sha256")
         and sha(wat) == fixture.get("wat_sha256")
         and sha(license_path) == provenance.get("license_sha256"),
         "P135 counter fixture, WAT, or MIT license hash differs from provenance")
    receipts = {}
    for stem, markers in (("build_validate", ("counter_wasm_tools_exit=0", "late_wasm_tools_exit=0",
                                               '"status":"valid"', "counter_wtime_mvp_exit=0",
                                               "late_wtime_mvp_exit=0")),
                          ("decoder_listing", ('(export "count" (memory 0))',
                                                '(export "run" (func 0))',
                                                "counter_print_exit=0", "late_print_exit=0"))):
        stdout, exit_path = (P135_VALIDATOR_LOGS / f"{stem}.{suffix}" for suffix in ("stdout", "exit"))
        need(exit_path.read_text(encoding="utf-8").strip() == "exit_code=0"
             and all(marker in stdout.read_text(encoding="utf-8") for marker in markers),
             f"P135 recorded {stem} validator did not pass")
        receipts[stem] = {"stdout": ref(stdout), "exit": ref(exit_path)}
    return {"provenance": ref(P135_PROVENANCE), "proof": ref(P135_PROOF),
            "wasm": ref(P135_FIXTURE), "wat": ref(wat), "license": ref(license_path),
            "validator_outputs": receipts}


def _fixture_refs_match(frozen: dict[str, Any]) -> dict[str, Any]:
    actual = _validate_fixture_provenance()
    need(frozen.get("fixture_refs") == actual,
         "P139 P135 fixture/provenance/validator references changed")
    return actual


def _current_p139_sources() -> dict[str, str]:
    result = {}
    for relative in P139_FILES:
        path = ROOT / relative
        if path.is_file():
            result[relative] = sha(path)
    required = set(P139_FILES)
    need(required.issubset(result), "P139 current source closure is incomplete")
    return dict(sorted(result.items()))


def _validate_p138_build_budgets(runtime_source_map: dict[str, str]) -> dict[str, Any]:
    import importlib.util

    proof = read_json(P138_PROOF, "P138 proof")
    need(proof.get("runtime_source_map_sha256") == canonical_sha(runtime_source_map),
         "current runtime/core source map differs from the P138 source proof")
    p138_path = ROOT / "tools/verification/producers/p138.py"
    spec = importlib.util.spec_from_file_location("p139_p138_build_check", p138_path)
    need(spec is not None and spec.loader is not None, "cannot load P138 build-budget checker")
    module = importlib.util.module_from_spec(spec)
    sys.dont_write_bytecode = True
    spec.loader.exec_module(module)
    budgets = proof.get("build_budgets", {})
    result = {}
    for name, path, mode, features in (
        ("default", P138_DEFAULT_BUILD, "default", []),
        ("register", P138_REGISTER_BUILD, "optional-feature", ["wtime-runtime/register-executor"]),
    ):
        claim = budgets.get(name, {})
        checked = module.check_build(path, mode, features)
        need(claim.get("artifact") == checked["artifact"]
             and claim.get("budget_pass") is checked["budget_pass"]
             and claim.get("default_dependency_closure_pass") is checked["default_dependency_closure_pass"]
             and claim.get("source_stable") is checked["source_stable"]
             and claim.get("source_file_count") == checked["source_file_count"]
             and claim.get("source_sha256") == checked["source_sha256"],
             f"P138 {name} build-budget evidence is stale")
        result[name] = {**checked, "reused": True}
    return result


def _load_archive() -> tuple[Path, dict[str, Any], dict[str, Any]]:
    archive = ROOT / ARCHIVE_REL
    import importlib.util

    path = ROOT / "tools/wtime-bench/baseline.py"
    spec = importlib.util.spec_from_file_location("p139_baseline_archive", path)
    need(spec is not None and spec.loader is not None, "cannot load immutable archive verifier")
    baseline = importlib.util.module_from_spec(spec)
    sys.dont_write_bytecode = True
    spec.loader.exec_module(baseline)
    manifest = baseline.verify_archive(archive)
    manifest_path = archive / "manifest.json"
    manifest_sha = sha(manifest_path)
    validate_archive_identity(manifest, manifest_sha)
    return archive, manifest, ref(manifest_path)


def freeze(output: Path) -> dict[str, Any]:
    output = _verification_output(output, create=True)
    # Preserve executed helpers before any metadata or archive validation runs.
    helper_refs = _copy_helpers(output)
    archive, manifest, archive_ref = _load_archive()
    prerequisites = {task: _prerequisite(task) for task in ("P136", "P137", "P138")}
    runtime_map = _runtime_source_map()
    budgets = _validate_p138_build_budgets(runtime_map)
    fixture_refs = _validate_fixture_provenance()
    rows = smoke_rows()
    write_new(output / "rows.json", rows)
    write_new(output / "runtime-source-map.json", {"source_map": runtime_map,
                                                       "source_map_sha256": canonical_sha(runtime_map)})
    p139_sources = _current_p139_sources()
    write_new(output / "p139-source-map.json", {"source_map": p139_sources,
                                                   "source_map_sha256": canonical_sha(p139_sources)})
    result = {
        "schema": "wtime-p139-freeze-v1", "status": "passed", "outcome": "contract_only",
        "timing_evidence": False, "archive": archive_ref, "archive_commit": manifest["repository_commit"],
        "archive_manifest_sha256": archive_ref["sha256"], "prerequisites": prerequisites,
        "p138_build_budgets": budgets,
        "runtime_source_map": ref(output / "runtime-source-map.json"),
        "runtime_source_map_sha256": canonical_sha(runtime_map),
        "p139_source_map": ref(output / "p139-source-map.json"),
        "p139_source_map_sha256": canonical_sha(p139_sources),
        "fixture_refs": fixture_refs,
        "rows": ref(output / "rows.json"), "row_count": 8,
        "helper_refs": helper_refs,
        "counts": {"runs": 2, "rows": 8, "pairs": 32, "side_processes": 64},
        "timing_reused": False, "build_budgets_reused": True,
    }
    write_new(output / "report.json", result)
    return result


def _load_module(path: Path, name: str):
    import importlib.util
    spec = importlib.util.spec_from_file_location(name, path)
    need(spec is not None and spec.loader is not None, f"cannot import {path}")
    module = importlib.util.module_from_spec(spec)
    sys.dont_write_bytecode = True
    spec.loader.exec_module(module)
    return module


def _helpers():
    sys.path.insert(0, str(ROOT / "tools/wtime-bench"))
    import entry_batch_adapter as adapter
    import entry_batch_compare as pairs
    import entry_batch_analyze as analyzer
    return adapter, pairs, analyzer


def _validate_analysis(analysis_path: Path, report: dict[str, Any], collection_root: Path,
                       expected_rows: list[dict[str, Any]]) -> dict[str, Any]:
    adapter, pairs, analyzer = _helpers()
    analysis_path = Path(analysis_path).resolve(strict=True)
    need(analysis_path.name == "report.json" and analysis_path.is_file(),
         "analysis must be a report.json file, not a directory alias")
    analysis_dir = analysis_path.parent
    result = read_json(analysis_path, "P139 smoke analysis")
    need(result.get("schema") == analyzer.SCHEMA_SIDE and result.get("status") == "passed"
         and result.get("metric") == "entry_total" and result.get("raw_unit") == "ns"
         and result.get("timing_evidence") is False and result.get("gate_eligible") is False
         and result.get("decision") is None and result.get("threshold_selection") is None
         and result.get("scaled_by_calls") is False,
         "P139 analysis is not the non-promoting v2 smoke result")
    need(result.get("method") == analyzer.METHOD,
         "P139 smoke analysis method is incorrect")
    need(set(result) == {"schema", "status", "metric", "raw_unit", "method", "report_sha256",
         "collection_identity_sha256", "analysis_source_refs", "input_refs", "timing_evidence",
         "gate_eligible", "decision", "threshold_selection", "scaled_by_calls", "runs",
         "limitations", "analysis_helpers", "comparison"},
         "P139 analysis contains missing or unexpected fields")
    need(result.get("report_sha256") == pairs.digest(report)
         and result.get("collection_identity_sha256") == pairs.digest(report["identity"]),
         "P139 analysis does not bind the exact collected report and identity")
    need(result.get("comparison") == pairs.reference(collection_root / "report.json"),
         "P139 analysis references another collection report")
    need(result.get("input_refs") == analyzer.bound_inputs(report),
         "P139 analysis input file references differ from collected inputs")
    expected_analysis_sources = {
        name: pairs.reference(ROOT / "tools/wtime-bench" / name)
        for name in ("entry_batch_analyze.py", "analyze.py", "bootstrap.py")
    }
    need(result.get("analysis_source_refs") == expected_analysis_sources,
         "P139 analyzer source references are stale")
    copied_helpers = result.get("analysis_helpers")
    need(isinstance(copied_helpers, dict) and set(copied_helpers) == set(expected_analysis_sources),
         "P139 analysis helper file refs are incomplete")
    for name, expected in expected_analysis_sources.items():
        target = Path(copied_helpers[name].get("path", "")).resolve(strict=True)
        need(target.is_file() and target.is_relative_to(analysis_dir)
             and copied_helpers[name] == pairs.reference(target)
             and sha(target) == expected["sha256"],
             f"P139 analysis helper file ref is stale or outside output: {name}")
    need(pairs.normalize_rows(expected_rows) == report["identity"]["rows"],
         "P139 collected rows differ from the frozen matrix")
    need(isinstance(result.get("runs"), list) and len(result["runs"]) == 2,
         "P139 analysis must retain both smoke runs")
    expected_by_id = {row["row_id"]: row for row in report["identity"]["rows"]}
    run_ids, seeds = set(), set()
    for run in result["runs"]:
        need(isinstance(run, dict) and type(run.get("run_id")) is int
             and run["run_id"] in (0, 1) and run["run_id"] not in run_ids
             and type(run.get("seed")) is int and run["seed"] == SEEDS[run["run_id"]]
             and run["seed"] not in seeds,
             "P139 analysis run IDs or seeds are invalid")
        need(set(run) == {"run_id", "seed", "rows"}, "P139 analysis run fields differ")
        run_ids.add(run["run_id"])
        seeds.add(run["seed"])
        rows = run.get("rows")
        need(isinstance(rows, list) and len(rows) == 8, "P139 analysis row matrix is incomplete")
        row_ids = set()
        for row in rows:
            need(isinstance(row, dict), "P139 analysis row is malformed")
            need(set(row) == {"row_id", "settings", "pairs", "raw_unit", "metric", "bootstrap_seed",
                 "candidate_over_baseline", "ci95", "bootstrap_resamples", "baseline", "candidate"},
                 "P139 analysis row fields differ")
            row_id = row.get("row_id")
            need(row_id in expected_by_id and row_id not in row_ids, "P139 analysis row ID is duplicate or foreign")
            row_ids.add(row_id)
            expected_settings = expected_by_id[row_id]
            need(row.get("settings") == expected_settings and row.get("pairs") == 2
                 and row.get("raw_unit") == "ns" and row.get("metric") == "entry_total"
                 and row.get("candidate_over_baseline") is None and row.get("ci95") is None
                 and row.get("bootstrap_resamples") == 0,
                 "P139 analysis row is not an exact descriptive side-settings smoke")
            pair_samples = [pair for pair in report["runs"][run["run_id"]]["pairs"]
                            if pair["row_id"] == row_id]
            need(len(pair_samples) == 2, "P139 raw report row lacks exactly two pairs")
            for side in ("baseline", "candidate"):
                side_stats = row.get(side)
                need(isinstance(side_stats, dict) and set(side_stats) == {
                    "total_ns", "prepare_ns", "instantiate_ns", "invoke_ns"},
                    f"P139 {side} raw phase summaries are incomplete")
                for field, summary in side_stats.items():
                    raw_values = [pair["observations"][side]["payload"][field] for pair in pair_samples]
                    need(isinstance(summary, dict) and summary == analyzer.summary(raw_values),
                         "P139 summary differs from its raw two-sample descriptive statistics")
    return result


def _validate_adapters(baseline_root: Path, candidate_root: Path):
    adapter, _, _ = _helpers()
    archive = ROOT / ARCHIVE_REL
    harness = ROOT / "tools/wtime-bench"
    baseline = adapter.verify(baseline_root, archive, harness)
    candidate = adapter.verify_current(candidate_root, ROOT, harness)
    need(baseline.get("kind") == adapter.KIND and candidate.get("kind") == adapter.CURRENT_KIND
         and baseline.get("source_origin") != candidate.get("source_origin"),
         "P139 sides are not individually identified archived/current adapters")
    baseline_binary = (baseline_root / adapter._safe_rel(baseline["executable"])).resolve(strict=True)
    candidate_binary = (candidate_root / adapter._safe_rel(candidate["executable"])).resolve(strict=True)
    need(ref(baseline_root / "adapter.json") != ref(candidate_root / "adapter.json")
         and baseline_binary != candidate_binary and baseline_binary.is_file() and candidate_binary.is_file(),
         "P139 baseline and candidate must have distinct build-record and executable files")
    return baseline, candidate


def _validate_collection(collection_root: Path, baseline_root: Path, candidate_root: Path,
                         rows: list[dict[str, Any]]) -> dict[str, Any]:
    _, pairs, _ = _helpers()
    report_path = Path(collection_root).resolve(strict=True) / "report.json"
    report = read_json(report_path, "P139 pair collection")
    need(report.get("schema") == pairs.SCHEMA_SIDE and report.get("timing_evidence") is False
         and report.get("same_source_alias") is False and report.get("identity", {}).get("smoke") is True,
         "P139 collection is not a distinct-source v2 smoke")
    verified = pairs.verify_collection(collection_root, ROOT / ARCHIVE_REL, baseline_root,
                                       candidate_root, ROOT, ROOT / "tools/wtime-bench", rows, 2,
                                       SEEDS, smoke=True, same_source_smoke=False)
    need(verified == report, "P139 collector revalidation differs from retained report")
    need(report.get("pair_count") == 32 and report["identity"].get("pairs_per_row") == 2
         and report["identity"].get("seeds") == list(SEEDS)
         and len(report["identity"].get("rows", [])) == 8,
         "P139 collection count matrix is incomplete")
    need(report["identity"].get("schema") == pairs.SCHEMA_SIDE,
         "P139 collection identity is not v2")
    return report


def _copy_helpers(output: Path) -> dict[str, dict[str, Any]]:
    names = [Path(__file__), ROOT / "tools/verification/producers/p138.py",
             ROOT / "tools/verification/producers/p135.py", ROOT / "tools/checks/build_budget.py",
             ROOT / "tools/checks/dependency_budget.py",
             ROOT / "tools/wtime-bench/baseline.py", ROOT / "tools/wtime-bench/entry_batch_adapter.py",
             ROOT / "tools/wtime-bench/entry_batch_compare.py",
             ROOT / "tools/wtime-bench/entry_batch_analyze.py",
             ROOT / "tools/wtime-bench/checkpoint.py", ROOT / "tools/wtime-bench/analyze.py",
             ROOT / "tools/wtime-bench/bootstrap.py"]
    refs = {}
    helper_dir = output / "helpers"
    for source in names:
        target = helper_dir / source.name
        shutil.copyfile(source, target)
        refs[source.name] = ref(target)
    return refs


def _expected_helper_sources() -> dict[str, Path]:
    paths = [Path(__file__), ROOT / "tools/verification/producers/p138.py",
             ROOT / "tools/verification/producers/p135.py", ROOT / "tools/checks/build_budget.py",
             ROOT / "tools/checks/dependency_budget.py", ROOT / "tools/wtime-bench/baseline.py",
             ROOT / "tools/wtime-bench/entry_batch_adapter.py",
             ROOT / "tools/wtime-bench/entry_batch_compare.py",
             ROOT / "tools/wtime-bench/entry_batch_analyze.py",
             ROOT / "tools/wtime-bench/checkpoint.py", ROOT / "tools/wtime-bench/analyze.py",
             ROOT / "tools/wtime-bench/bootstrap.py"]
    return {path.name: path for path in paths}


def _validate_helper_refs(helpers: Any, output_dir: Path) -> None:
    expected = _expected_helper_sources()
    need(isinstance(helpers, dict) and set(helpers) == set(expected),
         "P139 helper file refs are missing or unexpected")
    output_dir = Path(output_dir).resolve(strict=True)
    for name, source in expected.items():
        helper_ref = helpers[name]
        path = Path(helper_ref.get("path", "")).resolve(strict=True)
        need(path.is_file() and path.is_relative_to(output_dir / "helpers")
             and helper_ref == ref(path) and helper_ref["sha256"] == sha(source),
             f"P139 helper file ref is stale or outside the evidence output: {name}")


def verify(freeze_report: Path, baseline_root: Path, candidate_root: Path,
           collection_root: Path, analysis_path: Path, output: Path) -> dict[str, Any]:
    frozen = read_json(freeze_report, "P139 freeze report")
    need(frozen.get("schema") == "wtime-p139-freeze-v1" and frozen.get("status") == "passed"
         and frozen.get("timing_evidence") is False and frozen.get("archive_commit") == ARCHIVE_COMMIT,
         "P139 freeze receipt is invalid")
    output = _verification_output(output, create=True)
    helper_refs = _copy_helpers(output)
    freeze_dir = Path(freeze_report).resolve(strict=True).parent
    need(frozen.get("rows") == ref(freeze_dir / "rows.json")
         and frozen.get("runtime_source_map") == ref(freeze_dir / "runtime-source-map.json")
         and frozen.get("p139_source_map") == ref(freeze_dir / "p139-source-map.json"),
         "P139 freeze file references are stale")
    rows = validate_rows(read_json_rows(freeze_dir / "rows.json", "P139 frozen rows"), fixture=P135_FIXTURE)
    runtime_map = _runtime_source_map()
    sources = _current_p139_sources()
    need(frozen.get("runtime_source_map_sha256") == canonical_sha(runtime_map)
         and frozen.get("p139_source_map_sha256") == canonical_sha(sources)
         and read_json(freeze_dir / "runtime-source-map.json", "P139 frozen runtime map").get("source_map") == runtime_map
         and read_json(freeze_dir / "p139-source-map.json", "P139 frozen P139 source map").get("source_map") == sources,
         "P139 frozen source maps are stale")
    _, archive_manifest, archive_ref = _load_archive()
    need(frozen.get("archive") == archive_ref and frozen.get("archive_manifest_sha256") == ARCHIVE_MANIFEST_SHA256,
         "P139 frozen archive reference differs")
    prereqs = {task: _prerequisite(task) for task in ("P136", "P137", "P138")}
    need(frozen.get("prerequisites") == prereqs, "P139 prerequisite evidence changed after freeze")
    budgets = _validate_p138_build_budgets(runtime_map)
    need(frozen.get("p138_build_budgets") == budgets, "P139 inherited P138 build-budget proof changed")

    baseline_root, candidate_root = Path(baseline_root).resolve(strict=True), Path(candidate_root).resolve(strict=True)
    need(baseline_root != candidate_root and not baseline_root.is_relative_to(candidate_root)
         and not candidate_root.is_relative_to(baseline_root),
         "P139 adapter outputs must be separate roots")
    baseline, candidate = _validate_adapters(baseline_root, candidate_root)
    collection = _validate_collection(collection_root, baseline_root, candidate_root, rows)
    analysis = _validate_analysis(analysis_path, collection, Path(collection_root), rows)
    need(_runtime_source_map() == runtime_map and _current_p139_sources() == sources,
         "P139 source inputs changed during source-bound verification")
    result = {
        "schema": "wtime-p139-verification-v1", "status": "passed", "outcome": "contract_only",
        "timing_evidence": False, "gate_eligible": False, "candidate_promoted": False,
        "decision": None, "threshold_selection": None, "guest_or_build_launched": False,
        "archive": archive_ref, "freeze": ref(Path(freeze_report)), "prerequisites": prereqs,
        "baseline_build_record": ref(baseline_root / "adapter.json"),
        "baseline_executable_sha256": baseline["executable_sha256"],
        "candidate_build_record": ref(candidate_root / "adapter.json"),
        "candidate_executable_sha256": candidate["executable_sha256"],
        "baseline_kind": baseline["kind"], "candidate_kind": candidate["kind"],
        "collection": ref(Path(collection_root) / "report.json"),
        "collection_identity_sha256": collection["identity_sha256"],
        "analysis": ref(analysis_path), "analysis_sha256": sha(analysis_path),
        "fixture_refs": frozen.get("fixture_refs"),
        "runtime_source_map_sha256": canonical_sha(runtime_map),
        "p139_source_map_sha256": canonical_sha(sources),
        "runtime_source_map": frozen["runtime_source_map"],
        "p139_source_map": frozen["p139_source_map"],
        "p138_build_budgets": budgets,
        "counts": {"runs": 2, "rows": 8, "pairs": 32, "side_processes": 64},
        "settings": {"baseline": {"strategy": "eager", "threshold": 0},
                     "candidate_thresholds": [3, 10], "calls": list(CALL_COUNTS)},
        "helper_refs": helper_refs,
    }
    write_new(output / "report.json", result)
    return verify_evidence(output / "report.json")


def verify_evidence(report_path: Path) -> dict[str, Any]:
    _, pairs, _ = _helpers()
    report_path = Path(report_path).resolve(strict=True)
    report = read_json(report_path, "P139 evidence report")
    need(report.get("schema") == "wtime-p139-verification-v1" and report.get("status") == "passed"
         and report.get("outcome") == "contract_only" and report.get("timing_evidence") is False
         and report.get("gate_eligible") is False and report.get("candidate_promoted") is False
         and report.get("decision") is None and report.get("threshold_selection") is None
         and report.get("guest_or_build_launched") is False,
         "P139 evidence envelope is invalid or promoting")
    freeze_ref = report.get("freeze", {})
    freeze_path = Path(freeze_ref.get("path", "")).resolve(strict=True)
    need(freeze_ref == ref(freeze_path), "P139 freeze file reference is stale")
    frozen = read_json(freeze_path, "P139 freeze report")
    runtime_map_ref = report.get("runtime_source_map", {})
    p139_map_ref = report.get("p139_source_map", {})
    runtime_map_path = Path(runtime_map_ref.get("path", "")).resolve(strict=True)
    p139_map_path = Path(p139_map_ref.get("path", "")).resolve(strict=True)
    need(runtime_map_ref == ref(runtime_map_path) == frozen.get("runtime_source_map")
         and p139_map_ref == ref(p139_map_path) == frozen.get("p139_source_map"),
         "P139 source map file refs are stale")
    runtime_map = read_json(runtime_map_path, "P139 retained runtime source map").get("source_map")
    p139_map = read_json(p139_map_path, "P139 retained tooling source map").get("source_map")
    need(isinstance(runtime_map, dict) and isinstance(p139_map, dict)
         and canonical_sha(runtime_map) == report.get("runtime_source_map_sha256")
         and canonical_sha(p139_map) == report.get("p139_source_map_sha256")
         and _runtime_source_map() == runtime_map and _current_p139_sources() == p139_map,
         "P139 retained source maps no longer match current inputs")
    archive, _, archive_ref = _load_archive()
    need(report.get("archive") == archive_ref == frozen.get("archive"),
         "P139 archive identity is stale")
    prereqs = {task: _prerequisite(task) for task in ("P136", "P137", "P138")}
    need(report.get("prerequisites") == prereqs == frozen.get("prerequisites"),
         "P139 immutable prerequisite refs have changed")
    fixture_refs = _fixture_refs_match(frozen)
    budgets = _validate_p138_build_budgets(runtime_map)
    need(report.get("p138_build_budgets") == budgets == frozen.get("p138_build_budgets"),
         "P139 reused P138 build-budget proof has changed")
    analysis_ref = report.get("analysis", {})
    analysis_path = Path(analysis_ref.get("path", "")).resolve(strict=True)
    need(analysis_ref == ref(analysis_path) and report.get("analysis_sha256") == sha(analysis_path),
         "P139 analysis report file reference is stale")
    collection_ref = report.get("collection", {})
    collection_path = Path(collection_ref.get("path", "")).resolve(strict=True)
    need(collection_ref == ref(collection_path), "P139 collection report file reference is stale")
    for key in ("baseline_build_record", "candidate_build_record"):
        record_ref = report.get(key, {})
        path = Path(record_ref.get("path", "")).resolve(strict=True)
        need(path.name == "adapter.json" and path.is_file() and record_ref == ref(path),
             f"P139 {key} is not a current file reference")
    helpers = report.get("helper_refs", {})
    outdir = report_path.parent.resolve()
    _validate_helper_refs(helpers, outdir)
    baseline_root = Path(report["baseline_build_record"]["path"]).resolve().parent
    candidate_root = Path(report["candidate_build_record"]["path"]).resolve().parent
    baseline, candidate = _validate_adapters(baseline_root, candidate_root)
    rows = validate_rows(read_json_rows(Path(frozen["rows"]["path"]), "P139 frozen rows"))
    collection = _validate_collection(collection_path.parent, baseline_root, candidate_root, rows)
    analysis = _validate_analysis(analysis_path, collection, collection_path.parent, rows)
    need(report.get("fixture_refs") == fixture_refs == frozen.get("fixture_refs"),
         "P139 fixture provenance references differ from frozen inputs")
    need(report.get("baseline_executable_sha256") == baseline.get("executable_sha256")
         and report.get("candidate_executable_sha256") == candidate.get("executable_sha256"),
         "P139 archived/current executable identities changed")
    need(report.get("collection_identity_sha256") == pairs.digest(collection["identity"])
         and report.get("counts") == {"runs": 2, "rows": 8, "pairs": 32, "side_processes": 64},
         "P139 report count or collection identity differs")
    return report


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    freeze_parser = sub.add_parser("freeze")
    freeze_parser.add_argument("--output", type=Path, required=True)
    verify_parser = sub.add_parser("verify")
    verify_parser.add_argument("--freeze", type=Path, required=True)
    verify_parser.add_argument("--baseline", type=Path, required=True)
    verify_parser.add_argument("--candidate", type=Path, required=True)
    verify_parser.add_argument("--collection", type=Path, required=True)
    verify_parser.add_argument("--analysis", type=Path, required=True)
    verify_parser.add_argument("--output", type=Path, required=True)
    evidence_parser = sub.add_parser("check-evidence")
    evidence_parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        if args.command == "freeze":
            result = freeze(args.output)
        elif args.command == "verify":
            result = verify(args.freeze, args.baseline, args.candidate,
                            args.collection, args.analysis, args.output)
        else:
            result = verify_evidence(args.report)
        print(json.dumps(result, sort_keys=True, allow_nan=False))
        return 0
    except (OSError, KeyError, TypeError, ValueError, RuntimeError, json.JSONDecodeError) as exc:
        print(f"P139 verification failed: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
