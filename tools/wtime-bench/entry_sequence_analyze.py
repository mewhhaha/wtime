#!/usr/bin/env python3
"""Analyze P141 per-invocation Register sequence timings without running Wasm."""
from __future__ import annotations

import argparse
import json
import math
from pathlib import Path
import sys
from typing import Any

import analyze as statistics
import entry_sequence_compare as pairs
import entry_sequence_policy as sequence


SCHEMA = "wtime-entry-sequence-analysis-v1"
METHOD = "paired-process-event-log-rank-bootstrap-v1"
EVENTS = (
    "cold_initial", "compile_initial", "warm_initial",
    "cold_revisit", "recompile", "warm_revisit",
)
METRICS = ("p50", "p99", "max")
U128_MAX = (1 << 128) - 1


def _need(condition: Any, message: str) -> None:
    if not condition:
        raise ValueError(message)


def _reference_map(report: dict[str, Any]) -> dict[str, dict[str, str]]:
    """Rehash every path/SHA reference carried by the collector identity."""
    found: dict[str, dict[str, str]] = {}

    def walk(value: Any, path: str) -> None:
        if isinstance(value, dict):
            if set(value) == {"path", "sha256"}:
                ref = pairs.reference(value["path"])
                _need(ref == value, f"stale input reference at {path}")
                found[path] = ref
                return
            for key, child in value.items():
                walk(child, f"{path}.{key}" if path else key)
        elif isinstance(value, list):
            for index, child in enumerate(value):
                walk(child, f"{path}[{index}]")

    walk(report["identity"], "identity")
    for row in report["identity"]["rows"]:
        ref = pairs.reference(row["fixture"])
        _need(ref["sha256"] == row["fixture_sha256"], "stale sequence fixture")
        found[f"fixture_{row['row_id']}"] = ref
    return found


def _setting(row: dict[str, Any], side: str) -> tuple[str, int]:
    if side == "baseline":
        return "eager", 0
    if side == "candidate":
        return "adaptive", row["schedule_threshold"]
    raise ValueError(f"unknown sequence side: {side}")


def _process_stats(payload: dict[str, Any], row: dict[str, Any], side: str) -> dict[str, list[float]]:
    schedule_threshold = row["schedule_threshold"]
    strategy, threshold = _setting(row, side)
    sequence.validate_payload(
        payload, schedule_threshold, strategy, threshold, row["input"], diagnostic=False
    )
    grouped: dict[str, list[float]] = {event: [] for event in EVENTS}
    for call in payload["per_call"]:
        event = sequence.event_class(
            {key: call[key] for key in ("body", "visit", "phase")}, schedule_threshold
        )
        grouped[event].append(call["elapsed_ns"])
    return {event: values for event, values in grouped.items() if values}


def _summary(values: list[float]) -> dict[str, Any]:
    _need(values, "cannot summarize an empty sample")
    return {
        **statistics._summary(values),
        "min": min(values),
        "max": max(values),
        "p99": statistics._percentile(values, 0.99),
        "count": len(values),
        "unit": "ns",
    }


def _row_class_analysis(
    settings: dict[str, Any],
    run: dict[str, Any],
    samples: list[dict[str, Any]],
    smoke: bool,
) -> list[dict[str, Any]]:
    n = settings["schedule_threshold"]
    expected_classes = {sequence.event_class(pos, n) for pos in sequence.schedule(n)}
    grouped_samples: list[dict[str, dict[str, list[float]]]] = []
    for pair in samples:
        grouped_sides = {}
        for side in pairs.SIDES:
            grouped = _process_stats(pair["observations"][side]["payload"], settings, side)
            _need(set(grouped) == expected_classes, "event classes differ from exact schedule")
            grouped_sides[side] = grouped
        grouped_samples.append(grouped_sides)

    per_side: dict[str, dict[str, dict[str, list[float]]]] = {s: {} for s in pairs.SIDES}
    for event in expected_classes:
        per_side_event: dict[str, list[dict[str, float]]] = {s: [] for s in pairs.SIDES}
        for pair_index, _pair in enumerate(samples):
            for side in pairs.SIDES:
                latencies = grouped_samples[pair_index][side][event]
                _need(latencies, f"empty event class {event}")
                per_side_event[side].append({
                    metric: (max(latencies) if metric == "max" else statistics._percentile(
                        latencies, 0.5 if metric == "p50" else 0.99
                    ))
                    for metric in METRICS
                })
        for side in pairs.SIDES:
            per_side[side][event] = {
                metric: [process[metric] for process in per_side_event[side]]
                for metric in METRICS
            }

    output = []
    for event in EVENTS:
        if event not in expected_classes:
            continue
        calls_per_process = len(grouped_samples[0]["baseline"][event])
        side_summaries: dict[str, Any] = {}
        for side in pairs.SIDES:
            side_summaries[side] = {
                "process_count": len(samples),
                "raw_call_count": calls_per_process * len(samples),
                "calls_per_process": calls_per_process,
                "process_metrics": {
                    metric: _summary(per_side[side][event][metric]) for metric in METRICS
                },
            }

        metrics: dict[str, Any] = {}
        computed_vectors: dict[tuple[float, ...], str] = {}
        for metric in METRICS:
            baseline = per_side["baseline"][event][metric]
            candidate = per_side["candidate"][event][metric]
            _need(len(baseline) == len(candidate) == len(samples), "unpaired process summaries")
            _need(all(type(x) in (int, float) and math.isfinite(x) and x >= 0
                      for x in baseline + candidate), "process metrics must be finite nonnegative values")
            if not smoke:
                _need(all(x > 0 for x in baseline + candidate),
                      "full analysis requires positive process metrics for log ratios")
            logs = None if smoke else tuple(math.log(c / b) for b, c in zip(baseline, candidate))
            signature = (tuple(baseline), tuple(candidate)) if smoke else logs
            alias = computed_vectors.get(signature)
            seed = statistics._seed_for(run["seed"], (
                METHOD, settings["row_id"], settings["fixture_sha256"], event, metric,
            ))
            if alias is not None:
                previous = metrics[alias]
                metrics[metric] = {
                    "candidate_over_baseline": previous["candidate_over_baseline"],
                    "ci95": previous["ci95"],
                    "bootstrap_resamples": 0,
                    "bootstrap_seed": previous["bootstrap_seed"],
                    "alias_of": alias,
                }
                continue
            if smoke:
                metrics[metric] = {
                    "candidate_over_baseline": None,
                    "ci95": None,
                    "bootstrap_resamples": 0,
                    "bootstrap_seed": None,
                    "alias_of": None,
                }
                computed_vectors[signature] = metric
                continue
            median_log = statistics._percentile(list(logs), 0.5)
            low, high = statistics._bootstrap_interval(list(logs), seed)
            metrics[metric] = {
                "candidate_over_baseline": math.exp(median_log),
                "ci95": [low, high],
                "bootstrap_resamples": statistics.BOOTSTRAP_RESAMPLES,
                "bootstrap_seed": seed,
                "alias_of": None,
            }
            computed_vectors[signature] = metric
        output.append({
            "event_class": event,
            "raw_call_count_per_process": calls_per_process,
            "process_pairs": len(samples),
            "baseline": side_summaries["baseline"],
            "candidate": side_summaries["candidate"],
            "metrics": metrics,
        })
    return output


def analyze(report: dict[str, Any], collection_root: str | Path, allow_smoke: bool = False) -> dict[str, Any]:
    _need(type(allow_smoke) is bool, "allow_smoke must be boolean")
    verified_root, verified_report = pairs.verify_current_collection(collection_root)
    _need(verified_report == report, "provided report differs from current collection")
    pairs.validate_report(report, verified_root)
    analysis_sources = {
        "entry_sequence_analyze.py": pairs.reference(Path(__file__)),
        "entry_sequence_compare.py": pairs.reference(Path(pairs.__file__)),
        "entry_sequence_policy.py": pairs.reference(Path(sequence.__file__)),
        "analyze.py": pairs.reference(Path(statistics.__file__)),
        "bootstrap.py": pairs.reference(Path(statistics.__file__).with_name("bootstrap.py")),
    }
    report_digest = pairs.digest(report)
    inputs_before = _reference_map(report)
    identity = report["identity"]
    smoke = identity["smoke"]
    _need(type(smoke) is bool, "smoke must be boolean")
    _need(report["identity_sha256"] == pairs.digest(identity), "identity digest mismatch")
    _need(report["timing_evidence"] is (not smoke)
          and report["gate_eligible"] is (not smoke), "smoke/full evidence labels differ")
    if smoke:
        _need(allow_smoke, "smoke analysis requires --allow-smoke")
        _need(identity["pairs_per_row"] == 2, "smoke requires exactly two pairs per row")
    else:
        _need(identity["pairs_per_row"] >= 30, "full analysis requires at least 30 process pairs per row")
        _need(identity["seeds"][0] != identity["seeds"][1], "full analysis requires two independent seeds")

    normalized = pairs.normalize_rows([_row_for_normalize(row) for row in identity["rows"]])
    _need(normalized == identity["rows"], "normalized row identity changed")
    _need(len(report["runs"]) == 2, "analysis requires exactly two independent runs")
    runs_out = []
    for run_index, run in enumerate(report["runs"]):
        _need(run["run_id"] == run_index and run["seed"] == identity["seeds"][run_index],
              "run identity differs from frozen seed")
        row_results = []
        for settings in identity["rows"]:
            samples = [pair for pair in run["pairs"] if pair["row_id"] == settings["row_id"]]
            _need(len(samples) == identity["pairs_per_row"], "incomplete process pairs for row")
            class_results = _row_class_analysis(settings, run, samples, smoke)
            row_results.append({
                "row_id": settings["row_id"],
                "settings": settings,
                "process_pairs": len(samples),
                "classes": class_results,
            })
        runs_out.append({"run_id": run["run_id"], "seed": run["seed"], "rows": row_results})

    _need(_reference_map(report) == inputs_before, "input reference changed during analysis")
    _need(pairs.digest(report) == report_digest, "raw report changed during analysis")
    verified_root_after, verified_report_after = pairs.verify_current_collection(collection_root)
    _need(verified_root_after == verified_root and verified_report_after == report,
          "current collection changed during analysis")
    _need(all(pairs.reference(ref["path"]) == ref for ref in analysis_sources.values()),
          "analysis source changed during computation")
    return {
        "schema": SCHEMA,
        "status": "passed",
        "metric": "per_process_event_latency",
        "raw_unit": "ns",
        "method": METHOD,
        "report_sha256": report_digest,
        "collection_identity_sha256": pairs.digest(identity),
        "analysis_source_refs": analysis_sources,
        "input_refs": inputs_before,
        "timing_evidence": not smoke,
        "gate_eligible": not smoke,
        "decision": None,
        "threshold_selection": None,
        "scaled_by_calls": False,
        "runs": runs_out,
        "limitations": [
            "Per-process event summaries retain invocation distributions; paired inference uses process pairs only.",
            "P066 owns candidate decisions, cold/churn gates, and threshold selection.",
        ],
    }


def _row_for_normalize(row: dict[str, Any]) -> dict[str, Any]:
    """Project the exact raw row fields accepted by the collector normalizer."""
    return {key: row[key] for key in ("fixture", "input", "schedule_threshold")}


def verify_current_collection(path: str | Path) -> tuple[Path, dict[str, Any]]:
    """Delegate source, binary, fixture and raw-pair checks to the collector."""
    return pairs.verify_current_collection(path)


def run_analysis(comparison: str | Path, output: str | Path, allow_smoke: bool = False) -> dict[str, Any]:
    output = Path(output).absolute()
    _need(output == output.resolve(), "analysis output contains a path escape")
    _need(not output.exists(), "analysis output already exists")
    root, report = verify_current_collection(comparison)
    _need(not output.is_relative_to(root) and not root.is_relative_to(output), "output overlaps collection")
    refs = _reference_map(report)
    protected = [Path(ref["path"]).resolve() for ref in refs.values()]
    identity = report["identity"]
    protected.extend(Path(identity[key]).resolve() for key in ("adapter_root", "candidate_root"))
    protected.append(Path(identity["archive_manifest"]["path"]).resolve().parent)
    repo = Path(identity["repo"]).resolve()
    protected.extend(repo / path for path in (
        "crates", "tools/wtime-conformance", "tools/wtime-bench",
    ))
    _need(all(not p.is_relative_to(output) and not output.is_relative_to(p) for p in protected),
          "analysis output overlaps bound inputs")
    output.mkdir(parents=True, exist_ok=False)
    helper_dir = output / "helpers"
    _need(helper_dir.resolve().is_relative_to(output), "helper output escapes analysis root")
    helper_dir.mkdir()
    helper_sources = {
        "entry_sequence_analyze.py": Path(__file__).resolve(),
        "analyze.py": Path(statistics.__file__).resolve(),
        "bootstrap.py": Path(statistics.__file__).with_name("bootstrap.py").resolve(),
        "entry_sequence_policy.py": Path(sequence.__file__).resolve(),
        "entry_sequence_compare.py": Path(pairs.__file__).resolve(),
    }
    source_refs = {}
    for name, source in helper_sources.items():
        target = helper_dir / name
        _need(target.resolve().is_relative_to(output), "helper copy escapes analysis root")
        target.write_bytes(source.read_bytes())
        source_refs[name] = pairs.reference(target)
    invocation = output / "invocation.json"
    _need(invocation.resolve().is_relative_to(output), "invocation escapes analysis root")
    with invocation.open("x", encoding="utf-8") as stream:
        json.dump({"argv": list(sys.argv), "cwd": str(Path.cwd())}, stream, sort_keys=True)
        stream.write("\n")
    result = analyze(report, root, allow_smoke)
    _need(_reference_map(report) == refs, "bound source changed during analysis")
    _need(all(pairs.reference(ref["path"]) == ref for ref in source_refs.values()),
          "analysis source copy changed")
    result["analysis_source_refs"] = source_refs
    result["invocation"] = pairs.reference(invocation)
    result["comparison"] = pairs.reference(root / "report.json")
    report_path = output / "report.json"
    _need(report_path.resolve().is_relative_to(output), "analysis report escapes output root")
    with report_path.open("x", encoding="utf-8") as stream:
        json.dump(result, stream, sort_keys=True, indent=2, allow_nan=False)
        stream.write("\n")
    return result


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--comparison", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--allow-smoke", action="store_true")
    args = parser.parse_args()
    result = run_analysis(args.comparison, args.output, args.allow_smoke)
    print(json.dumps({key: result[key] for key in (
        "status", "metric", "timing_evidence", "gate_eligible", "decision"
    )}))


if __name__ == "__main__":
    main()
