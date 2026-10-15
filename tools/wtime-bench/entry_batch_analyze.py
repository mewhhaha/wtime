#!/usr/bin/env python3
"""Analyze whole Register entry lifetimes without rerunning or scaling Wasm."""
from __future__ import annotations

import argparse
import json
import math
from pathlib import Path
import sys

import analyze as statistics
import entry_batch_compare as pairs

SCHEMA = "wtime-register-entry-analysis-v1"
SCHEMA_SIDE = "wtime-register-entry-analysis-v2"
METHOD = "paired-log-median-rank-bootstrap-v1"
ROW_KEYS = ("fixture", "case", "input", "expected", "calls", "strategy", "threshold")


def bound_inputs(report):
    """Hash the retained records, actual executables, fixtures and collectors."""
    identity = report["identity"]
    references = {}
    for side, binding in identity["sides"].items():
        for key in ("build_record", "executable"):
            ref = binding[key]
            pairs.need(pairs.reference(ref["path"]) == ref, f"stale {side} {key}")
            references[f"{side}_{key}"] = ref
    pairs.need(pairs.reference(identity["archive_manifest"]["path"]) == identity["archive_manifest"],
               "stale archive manifest")
    references["archive_manifest"] = identity["archive_manifest"]
    for row in identity["rows"]:
        ref = pairs.reference(row["fixture"])
        pairs.need(ref["sha256"] == row["fixture_sha256"], "stale fixture")
        references[f"fixture_{row['row_id']}"] = ref
    for name, ref in identity["helpers"].items():
        pairs.need(pairs.reference(ref["path"]) == ref, "stale collector helper")
        references[f"collector_{name}"] = ref
    return references


def summary(values):
    pairs.need(values, "empty timing group")
    return {**statistics._summary(values), "min": min(values), "max": max(values),
            "p99": statistics._percentile(values, 0.99), "count": len(values), "unit": "ns"}


def analyze(report, collection_root, allow_smoke=False):
    pairs.need(type(allow_smoke) is bool, "allow_smoke must be boolean")
    pairs.validate_report(report, collection_root)
    report_hash = pairs.digest(report)
    analysis_sources = {name: pairs.reference(Path(__file__).parent / name)
                        for name in ("entry_batch_analyze.py", "analyze.py", "bootstrap.py")}
    identity = report["identity"]
    smoke = identity["smoke"]
    pairs.need(type(smoke) is bool and type(identity["same_source_alias"]) is bool,
               "invalid analysis evidence kind")
    pairs.need(report["timing_evidence"] is (not smoke), "timing evidence label differs")
    if smoke:
        pairs.need(allow_smoke, "smoke cannot supply candidate uncertainty")
        pairs.need(identity["pairs_per_row"] == 2, "invalid smoke sample count")
    else:
        pairs.need(identity["same_source_alias"] is False and identity["pairs_per_row"] >= 30,
                   "candidate analysis needs distinct sources and at least 30 pairs")
        pairs.need(identity["seeds"][0] != identity["seeds"][1], "independent run seeds required")
        baseline, candidate = (identity["sides"][side] for side in pairs.SIDES)
        pairs.need(baseline["kind"] == pairs.adapter.KIND and candidate["kind"] == pairs.adapter.CURRENT_KIND
                   and baseline["build_record"]["path"] != candidate["build_record"]["path"]
                   and baseline["executable"]["path"] != candidate["executable"]["path"]
                   and baseline["common"] == candidate["common"], "invalid production side identities")
    inputs = bound_inputs(report)
    rows = [{key: row[key] for key in pairs.row_keys(row)} for row in identity["rows"]]
    pairs.need(pairs.normalize_rows(rows) == identity["rows"], "analysis row/settings identity differs")
    runs = []
    for run in report["runs"]:
        rows = []
        for settings in identity["rows"]:
            samples = [pair for pair in run["pairs"] if pair["row_id"] == settings["row_id"]]
            pairs.need(len(samples) == identity["pairs_per_row"], "incomplete analysis row")
            raw = {side: {field: [pair["observations"][side]["payload"][field] for pair in samples]
                          for field in pairs.TIMINGS} for side in pairs.SIDES}
            pairs.need(all(value <= 2**128 - 1 for side in raw.values()
                           for values in side.values() for value in values), "timing exceeds Rust u128 range")
            seed = statistics._seed_for(run["seed"], (METHOD, identity["schema"], settings["row_id"],
                                                     settings["fixture_sha256"], pairs.digest(settings)))
            ratio = interval = None
            if not smoke:
                pairs.need(all(value > 0 for side in raw.values() for value in side["total_ns"]),
                           "candidate total must be positive for log ratios")
                logs = [math.log(candidate / baseline)
                        for baseline, candidate in zip(raw["baseline"]["total_ns"], raw["candidate"]["total_ns"])]
                ratio = math.exp(statistics._percentile(logs, 0.5))
                lo, hi = statistics._bootstrap_interval(logs, seed)
                interval = [lo, hi]
            rows.append({"row_id": settings["row_id"], "settings": settings, "pairs": len(samples),
                         "raw_unit": "ns", "metric": "entry_total", "bootstrap_seed": seed,
                         "candidate_over_baseline": ratio, "ci95": interval,
                         "bootstrap_resamples": 0 if smoke else statistics.BOOTSTRAP_RESAMPLES,
                         "baseline": {field: summary(values) for field, values in raw["baseline"].items()},
                         "candidate": {field: summary(values) for field, values in raw["candidate"].items()}})
        runs.append({"run_id": run["run_id"], "seed": run["seed"], "rows": rows})
    pairs.need(bound_inputs(report) == inputs, "analysis input changed during computation")
    pairs.need(pairs.digest(report) == report_hash, "report changed during analysis")
    pairs.need(all(pairs.reference(ref["path"]) == ref for ref in analysis_sources.values()),
               "analysis helper changed during computation")
    return {"schema": SCHEMA_SIDE if identity["schema"] == pairs.SCHEMA_SIDE else SCHEMA,
            "status": "passed", "metric": "entry_total", "raw_unit": "ns",
            "method": METHOD, "report_sha256": report_hash, "collection_identity_sha256": pairs.digest(identity),
            "analysis_source_refs": analysis_sources,
            "input_refs": inputs, "timing_evidence": not smoke, "gate_eligible": not smoke,
            "decision": None, "threshold_selection": None, "scaled_by_calls": False, "runs": runs,
            "limitations": ["Per-row uncertainty only; P066 owns semantic, control, cold-code, cache-churn and build gates."]}


def verify_current_collection(comparison):
    """Revalidate the original source context; this operation never launches Wasm."""
    comparison = Path(comparison).resolve(strict=True)
    root = comparison if comparison.is_dir() else comparison.parent
    report = json.loads((root / "report.json").read_text())
    identity = report["identity"]
    sides = identity["sides"]
    baseline_root = Path(sides["baseline"]["build_record"]["path"]).parent
    candidate_root = Path(sides["candidate"]["build_record"]["path"]).parent
    candidate = json.loads((candidate_root / "adapter.json").read_text())
    # The normal current build records the actual repository. Same-source smoke
    # has no current candidate record and uses the collector helper's location.
    repo = Path(candidate.get("repository_root", Path(identity["helpers"]["entry_batch_compare.py"]["path"]).parents[2]))
    archive = Path(identity["archive_manifest"]["path"]).parent
    rows = [{key: row[key] for key in pairs.row_keys(row)} for row in identity["rows"]]
    verified = pairs.verify_collection(root, archive, baseline_root, candidate_root, repo,
                                       repo / "tools/wtime-bench", rows, identity["pairs_per_row"], identity["seeds"],
                                       smoke=identity["smoke"], same_source_smoke=identity["same_source_alias"])
    pairs.need(report == verified, "collection changed during source verification")
    return root, report


def run_analysis(comparison, output, allow_smoke=False):
    output = Path(output).absolute()
    pairs.need(output == output.resolve(), "analysis output contains a path escape")
    pairs.need(not output.exists(), "analysis output already exists")
    root, report = verify_current_collection(comparison)
    pairs.need(not output.is_relative_to(root) and not root.is_relative_to(output), "analysis output overlaps collection")
    protected = [Path(ref["path"]).resolve() for ref in bound_inputs(report).values()]
    protected.extend(Path(side["build_record"]["path"]).resolve().parent for side in report["identity"]["sides"].values())
    protected.append(Path(report["identity"]["archive_manifest"]["path"]).resolve().parent)
    candidate_record = json.loads(Path(report["identity"]["sides"]["candidate"]["build_record"]["path"]).read_text())
    repo = Path(candidate_record.get("repository_root", Path(report["identity"]["helpers"]["entry_batch_compare.py"]["path"]).parents[2])).resolve()
    protected.extend(repo / path for path in ("crates", "tools/wtime-conformance", "tools/wtime-bench"))
    pairs.need(all(not path.is_relative_to(output) and not output.is_relative_to(path) for path in protected),
               "analysis output overlaps input artifacts")
    output.mkdir(parents=True, exist_ok=False)
    helper_dir = output / "helpers"
    helper_dir.mkdir()
    helper_refs = {}
    for name in ("entry_batch_analyze.py", "analyze.py", "bootstrap.py"):
        source = Path(__file__).parent / name
        data = source.read_bytes()
        target = helper_dir / name
        target.write_bytes(data)
        helper_refs[name] = pairs.reference(target)
    pairs.write_new(output / "invocation.json", {"argv": list(sys.argv), "cwd": str(Path.cwd())})
    result = analyze(report, root, allow_smoke)
    pairs.need(all((Path(__file__).parent / name).read_bytes() == Path(ref["path"]).read_bytes()
                   for name, ref in helper_refs.items()), "analysis helper changed during computation")
    result["analysis_helpers"] = helper_refs
    result["comparison"] = pairs.reference(root / "report.json")
    pairs.write_new(output / "report.json", result)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--comparison", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--allow-smoke", action="store_true")
    args = parser.parse_args()
    result = run_analysis(args.comparison, args.output, args.allow_smoke)
    print(json.dumps({key: result[key] for key in ("status", "metric", "timing_evidence", "gate_eligible", "decision")}))


if __name__ == "__main__":
    main()
