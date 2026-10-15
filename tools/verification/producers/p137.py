#!/usr/bin/env python3
"""Source-bound, analysis-only verification of the closed P136 collection."""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import math
from pathlib import Path
import shutil
import sys
from typing import Any

ROOT = Path(__file__).resolve().parents[3]
P136_COMPLETION = ROOT / "docs/benchmarks/performance/P136-completion.json"
P136_EVIDENCE = ROOT / "docs/benchmarks/performance/P136-evidence.json"
P136_PROOF = ROOT / "docs/benchmarks/performance/P136-proof.json"
P136_CONTRACT = ROOT / "docs/benchmarks/performance/P136-contract.json"
P136_VERIFICATION = ROOT / "docs/benchmarks/performance/P136-verification.json"
P136_RECORDS = ROOT / "docs/benchmarks/performance/P136-records.json.gz"
P136_COLLECTION = ROOT / "target/verification/runs/P136/verify/attempt-0001/proof/collection"
P136_REPORT = P136_COLLECTION / "report.json"
P136_PROOF_REPORT = ROOT / "target/verification/runs/P136/verify/attempt-0001/proof/report.json"
P136_RECEIPT = ROOT / "target/performance/P136/shared-finalization/attempt-0001/evidence.json"
P136_COMPLETION_RECEIPT = ROOT / "target/performance/P136/shared-finalization/attempt-0001/completion.json"


class P137Error(ValueError):
    """P137 inputs or source-bound analysis are invalid."""


def need(ok: bool, message: str) -> None:
    if not ok:
        raise P137Error(message)


def sha(path: Path) -> str:
    h = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def ref(path: Path) -> dict[str, Any]:
    path = Path(path).resolve(strict=True)
    return {"path": str(path), "sha256": sha(path), "bytes": path.stat().st_size}


def pair_ref(path: Path) -> dict[str, str]:
    """Match the analyzer/collector's compact file-reference schema."""
    path = Path(path).resolve(strict=True)
    return {"path": str(path), "sha256": sha(path)}


def canonical_sha(value: Any) -> str:
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":"),
                                    allow_nan=False).encode()).hexdigest()


def read_json(path: Path, label: str) -> dict[str, Any]:
    try:
        value = json.loads(Path(path).read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise P137Error(f"{label} is invalid: {exc}") from exc
    need(isinstance(value, dict), f"{label} must be an object")
    return value


def write_new(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with Path(path).open("x", encoding="utf-8") as stream:
        json.dump(value, stream, indent=2, sort_keys=True, allow_nan=False)
        stream.write("\n")


def load_module(path: Path, name: str):
    spec = importlib.util.spec_from_file_location(name, path)
    need(spec is not None and spec.loader is not None, f"cannot load helper {path}")
    module = importlib.util.module_from_spec(spec)
    sys.dont_write_bytecode = True
    spec.loader.exec_module(module)
    return module


def p136_module():
    return load_module(ROOT / "tools/verification/producers/p136.py", "p137_p136_producer")


def analyzer_module():
    bench = str(ROOT / "tools/wtime-bench")
    if bench not in sys.path:
        sys.path.insert(0, bench)
    return load_module(ROOT / "tools/wtime-bench/entry_batch_analyze.py", "p137_entry_batch_analyze")


def p136_sources() -> dict[str, str]:
    module = p136_module()
    return module.current_candidate_source_map()


def _verify_p136() -> tuple[dict[str, Any], dict[str, Any]]:
    """Validate published P136 refs and invoke its source-current public checker."""
    completion = read_json(P136_COMPLETION, "P136 completion")
    evidence = read_json(P136_EVIDENCE, "P136 evidence")
    proof = read_json(P136_PROOF, "P136 proof")
    need(completion.get("task") == "P136" and completion.get("status") == "done"
         and completion.get("outcome") == "contract_only", "P136 completion is not closed contract-only")
    need(evidence.get("task") == "P136" and evidence.get("status") == "passed"
         and evidence.get("passed") is True and evidence.get("candidate_promoted") is False
         and evidence.get("timing_evidence") is not True and evidence.get("outcome") == "contract_only",
         "P136 finalized evidence is invalid or promoting")
    need(proof.get("schema") == "wtime-p136-verification-v1" and proof.get("status") == "passed"
         and proof.get("timing_evidence") is False and proof.get("smoke") is True
         and proof.get("same_source_smoke") is False and proof.get("source_stable") is True,
         "P136 proof is not the expected source-bound smoke")
    need(completion.get("evidence") == [ref(P136_RECEIPT)]
         and completion.get("artifact_hashes") == {str(P136_RECEIPT.resolve()): sha(P136_RECEIPT)},
         "P136 completion does not bind its actual evidence receipt")
    need(read_json(P136_RECEIPT, "P136 finalized receipt") == evidence
         and read_json(P136_COMPLETION_RECEIPT, "P136 completion receipt") == completion,
         "P136 exported closure differs from retained finalization outputs")
    proof_ref = ref(P136_PROOF_REPORT)
    need(proof == read_json(P136_PROOF_REPORT, "P136 proof report"),
         "P136 exported proof differs from the retained source-bound report")
    # The public checker revalidates the collection journal, raw command records,
    # source/build/executable/fixture/helper closure, and inherited build budgets.
    verified = p136_module().verify_evidence(P136_PROOF_REPORT)
    need(verified == proof, "P136 public source-current checker returned different evidence")
    need(proof.get("collection") == ref(P136_REPORT), "P136 proof points at another collection")
    need(P136_COLLECTION.is_dir() and P136_REPORT.is_file(), "P136 retained collection is missing")
    return proof, {"completion": ref(P136_COMPLETION), "evidence": ref(P136_EVIDENCE),
                   "proof": ref(P136_PROOF), "proof_report": proof_ref,
                   "collection_report": ref(P136_REPORT), "contract": ref(P136_CONTRACT),
                   "verification_contract": ref(P136_VERIFICATION), "records": ref(P136_RECORDS)}


def freeze(output: Path) -> dict[str, Any]:
    output = _start_output(output, "freeze")
    proof, refs = _verify_p136()
    source_map = p136_sources()
    need(proof.get("runtime_source_map_sha256") == canonical_sha(source_map),
         "P136 current runtime/core source map is no longer current")
    need(len(source_map) == 732, "P136 current candidate source closure is not the expected 732 files")
    write_new(output / "current-source-map.json", {"source_map": source_map,
                                                     "source_map_sha256": canonical_sha(source_map)})
    result = {"schema": "wtime-p137-freeze-v1", "status": "passed", "timing_evidence": False,
              "outcome": "contract_only", "p136": refs,
              "p136_proof_identity_sha256": canonical_sha(proof),
              "p136_collection_identity_sha256": proof["collection"]["sha256"],
              "current_source_map_sha256": canonical_sha(source_map),
              "current_source_file_count": len(source_map),
              "current_source_map": ref(output / "current-source-map.json"),
              "counts": {"runs": 2, "rows": 8, "pairs": 32}, "timing_reused": True,
              "guest_or_build_launched": False}
    write_new(output / "report.json", result)
    return result


def _start_output(path: Path, phase: str) -> Path:
    path = Path(path).resolve()
    store = (ROOT / "target/verification").resolve()
    need(path.is_relative_to(store), "P137 output must stay under target/verification")
    path.mkdir(parents=True, exist_ok=False)
    (path / "helpers").mkdir()
    for helper in (Path(__file__), ROOT / "tools/verification/producers/p136.py",
                   ROOT / "tools/wtime-bench/entry_batch_analyze.py",
                   ROOT / "tools/wtime-bench/entry_batch_compare.py",
                   ROOT / "tools/wtime-bench/analyze.py", ROOT / "tools/wtime-bench/bootstrap.py"):
        if helper.exists():
            shutil.copyfile(helper, path / "helpers" / helper.name)
    write_new(path / "invocation.json", {"argv": list(sys.argv), "cwd": str(Path.cwd().resolve()),
                                         "phase": phase})
    return path


def validate_smoke_analysis(result: dict[str, Any]) -> None:
    """Check the complete non-promoting P136 smoke result envelope."""
    need(isinstance(result, dict) and result.get("schema") == "wtime-register-entry-analysis-v1"
         and result.get("status") == "passed" and result.get("metric") == "entry_total"
         and result.get("raw_unit") == "ns" and result.get("timing_evidence") is False
         and result.get("gate_eligible") is False and result.get("decision") is None
         and result.get("threshold_selection") is None and result.get("scaled_by_calls") is False,
         "P137 smoke analysis result is invalid or promoting")
    runs = result.get("runs")
    need(isinstance(runs, list) and len(runs) == 2, "P137 analysis result run count is incorrect")
    run_ids, seeds = set(), set()
    expected = {(count, strategy, threshold) for count in (1, 10, 100, 10000)
                for strategy, threshold in (("eager", 0), ("adaptive", 3))}
    total_pairs = 0
    for run in runs:
        need(isinstance(run, dict) and type(run.get("run_id")) is int
             and run["run_id"] in (0, 1) and run["run_id"] not in run_ids
             and type(run.get("seed")) is int and run["seed"] == (136001, 136002)[run["run_id"]]
             and run["seed"] not in seeds, "P137 analysis run identity is duplicate or malformed")
        run_ids.add(run["run_id"])
        seeds.add(run["seed"])
        rows = run.get("rows")
        need(isinstance(rows, list) and len(rows) == 8, "P137 analysis run matrix is incomplete")
        seen = set()
        for row in rows:
            need(isinstance(row, dict), "P137 analysis row is malformed")
            settings = row.get("settings")
            need(isinstance(settings, dict) and settings.get("case") == "p135_counter"
                 and type(settings.get("calls")) is int
                 and (settings["calls"], settings.get("strategy"), settings.get("threshold")) in expected,
                 "P137 analysis row settings differ from the frozen P136 matrix")
            key = (settings["calls"], settings["strategy"], settings["threshold"])
            need(key not in seen and row.get("row_id") == settings.get("row_id"),
                 "P137 analysis row is duplicated or has a mismatched identity")
            seen.add(key)
            need(type(row.get("pairs")) is int and row["pairs"] == 2
                 and row.get("candidate_over_baseline") is None and row.get("ci95") is None
                 and type(row.get("bootstrap_resamples")) is int and row["bootstrap_resamples"] == 0
                 and row.get("raw_unit") == "ns" and row.get("metric") == "entry_total",
                 "P137 smoke row claims uncertainty or a derived metric")
            need(type(row.get("bootstrap_seed")) is int,
                 "P137 bootstrap seed is malformed")
            for side in ("baseline", "candidate"):
                summaries = row.get(side)
                need(isinstance(summaries, dict) and set(summaries) == {
                    "total_ns", "prepare_ns", "instantiate_ns", "invoke_ns"
                }, "P137 raw phase summary fields are incomplete")
                for timing, summary in summaries.items():
                    need(isinstance(summary, dict) and summary.get("unit") == "ns"
                         and type(summary.get("count")) is int and summary["count"] == 2,
                         "P137 raw summary count or unit is malformed")
                    for field in ("median", "mad", "p25", "p75", "min", "max", "p99"):
                        value = summary.get(field)
                        need(type(value) in (int, float) and math.isfinite(value) and value >= 0,
                             f"P137 {side} {timing} summary is not finite nonnegative raw ns")
            total_pairs += row["pairs"]
        need(seen == expected, "P137 analysis run matrix has missing/foreign settings")
    need(total_pairs == 32, "P137 analysis has incomplete pair count")


def validate_analysis_binding(analysis: dict[str, Any], analysis_dir: Path) -> None:
    """Verify source identities and output-local copies for analyzer helpers."""
    names = ("entry_batch_analyze.py", "analyze.py", "bootstrap.py")
    current = {name: pair_ref(ROOT / "tools/wtime-bench" / name) for name in names}
    need(analysis.get("analysis_source_refs") == current,
         "P137 current analysis helper identities are stale")
    copied = analysis.get("analysis_helpers")
    need(isinstance(copied, dict) and set(copied) == set(names),
         "P137 retained analysis helper copies are incomplete")
    analysis_dir = analysis_dir.resolve(strict=True)
    for name in names:
        target = Path(copied[name].get("path", "")).resolve(strict=True)
        need(target.is_relative_to(analysis_dir) and copied[name] == pair_ref(target)
             and sha(target) == current[name]["sha256"],
             f"P137 retained {name} helper copy is stale or outside its analysis output")


def validate_analysis_against_collection(analysis: dict[str, Any]) -> None:
    """Recompute smoke summaries from retained raw pairs without launching guests."""
    analyzer = analyzer_module()
    report = read_json(P136_REPORT, "P136 pair report")
    need(analysis.get("comparison") == pair_ref(P136_REPORT), "P137 analysis references a different P136 report")
    expected = analyzer.analyze(report, P136_COLLECTION, allow_smoke=True)
    comparable = {key: value for key, value in analysis.items()
                  if key not in ("analysis_helpers", "comparison")}
    need(comparable == expected, "P137 analysis result does not recompute from retained P136 raw pairs")


def validate_evidence_content(receipt: dict[str, Any], analysis: dict[str, Any],
                              proof: dict[str, Any], refs: dict[str, Any],
                              source_map: dict[str, str]) -> None:
    need(receipt.get("schema") == "wtime-p137-verification-v1" and receipt.get("status") == "passed"
         and receipt.get("outcome") == "contract_only" and receipt.get("decision") is None
         and receipt.get("timing_evidence") is False and receipt.get("gate_eligible") is False
         and receipt.get("candidate_promoted") is False and receipt.get("guest_or_build_launched") is False,
         "P137 evidence envelope is invalid or promoting")
    need(receipt.get("p136") == refs and receipt.get("p136_proof_identity_sha256") == canonical_sha(proof)
         and receipt.get("current_source_map_sha256") == canonical_sha(source_map)
         and receipt.get("current_source_file_count") == len(source_map),
         "P137 evidence references or current source map are stale")
    validate_smoke_analysis(analysis)
    need(receipt.get("counts") == {"runs": 2, "rows": 8, "pairs": 32}
         and receipt.get("analysis_sha256") is not None,
         "P137 result counts are malformed")
    need(receipt.get("analysis_method") == analysis.get("method")
         and receipt.get("bootstrap_resamples") == 0 and receipt.get("build_budgets_reused") is True
         and receipt.get("build_budget_source") == proof.get("build_source_inputs_sha256"),
         "P137 method or inherited build-budget identity is stale")


def verify(freeze_report: Path, output: Path) -> dict[str, Any]:
    frozen = read_json(freeze_report, "P137 freeze report")
    need(frozen.get("schema") == "wtime-p137-freeze-v1" and frozen.get("status") == "passed"
         and frozen.get("timing_evidence") is False and frozen.get("outcome") == "contract_only",
         "P137 freeze receipt is invalid or promoting")
    output = _start_output(output, "verify")
    proof, refs = _verify_p136()
    source_map = p136_sources()
    need(frozen.get("p136") == refs and frozen.get("p136_proof_identity_sha256") == canonical_sha(proof)
         and frozen.get("current_source_map_sha256") == canonical_sha(source_map)
         and frozen.get("current_source_file_count") == 732
         and frozen.get("current_source_map") == ref(Path(frozen["current_source_map"]["path"]))
         and read_json(Path(frozen["current_source_map"]["path"]), "P137 source map").get("source_map") == source_map,
         "P137 frozen P136 or current source identity is stale")
    need(frozen.get("counts") == {"runs": 2, "rows": 8, "pairs": 32}, "P137 frozen counts changed")
    analyzer = analyzer_module()
    result = analyzer.run_analysis(P136_REPORT, output / "analysis", allow_smoke=True)
    validate_smoke_analysis(result)
    validate_analysis_binding(result, output / "analysis")
    validate_analysis_against_collection(result)
    need(p136_sources() == source_map, "P137 source map changed during analysis")
    analysis_ref = ref(output / "analysis/report.json")
    receipt = {"schema": "wtime-p137-verification-v1", "status": "passed",
               "outcome": "contract_only", "decision": None, "timing_evidence": False,
               "gate_eligible": False, "candidate_promoted": False, "p136": refs,
               "p136_proof_identity_sha256": canonical_sha(proof),
               "current_source_map_sha256": canonical_sha(source_map),
               "current_source_file_count": len(source_map), "analysis": analysis_ref,
               "analysis_sha256": sha(Path(analysis_ref["path"])),
               "counts": {"runs": 2, "rows": 8, "pairs": 32},
               "analysis_method": result["method"], "bootstrap_resamples": 0,
               "build_budgets_reused": True, "build_budget_source": proof["build_source_inputs_sha256"],
               "guest_or_build_launched": False}
    write_new(output / "report.json", receipt)
    validate_evidence_content(receipt, read_json(Path(analysis_ref["path"]), "P137 analysis output"),
                              proof, refs, source_map)
    return verify_evidence(output / "report.json", _verified_context=(proof, refs, source_map))


def verify_evidence(path: Path, *, _verified_context=None) -> dict[str, Any]:
    """No-launch public checker for a retained P137 analysis receipt."""
    receipt = read_json(path, "P137 evidence")
    if _verified_context is None:
        proof, refs = _verify_p136()
        source_map = p136_sources()
    else:
        proof, refs, source_map = _verified_context
    output_ref = receipt.get("analysis")
    need(isinstance(output_ref, dict) and output_ref == ref(Path(output_ref.get("path", ""))),
         "P137 analysis output reference is stale")
    analysis = read_json(Path(output_ref["path"]), "P137 analysis output")
    need(receipt.get("analysis_sha256") == sha(Path(output_ref["path"])),
         "P137 analysis output content hash is stale")
    validate_evidence_content(receipt, analysis, proof, refs, source_map)
    validate_analysis_binding(analysis, Path(output_ref["path"]).parent)
    validate_analysis_against_collection(analysis)
    need(p136_sources() == source_map, "P137 source-current closure changed during evidence review")
    return receipt


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    freeze_parser = sub.add_parser("freeze")
    freeze_parser.add_argument("--output", type=Path, required=True)
    verify_parser = sub.add_parser("verify")
    verify_parser.add_argument("--freeze", type=Path, required=True)
    verify_parser.add_argument("--output", type=Path, required=True)
    evidence_parser = sub.add_parser("check-evidence")
    evidence_parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        if args.command == "freeze":
            result = freeze(args.output)
        elif args.command == "verify":
            result = verify(args.freeze, args.output)
        else:
            result = verify_evidence(args.report)
        print(json.dumps(result, sort_keys=True, allow_nan=False))
        return 0
    except (P137Error, OSError, KeyError, TypeError, ValueError, RuntimeError) as exc:
        print(f"P137 verification failed: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
