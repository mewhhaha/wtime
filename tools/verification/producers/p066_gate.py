"""Pure composition of the P066 focused and supplementary timing decisions.

This module deliberately performs no collection, filesystem access, bootstrap,
or source verification. The caller binds and revalidates all evidence first.
"""
from __future__ import annotations

import math
from typing import Any

import entry_sequence_policy


SCHEMA = "wtime-p066-supplementary-gate-v1"
TOTAL_SCHEMAS = {"wtime-register-entry-analysis-v1", "wtime-register-entry-analysis-v2"}
TOTAL_METHOD = "paired-log-median-rank-bootstrap-v1"
SEQUENCE_SCHEMA = "wtime-entry-sequence-analysis-v1"
SEQUENCE_METHOD = "paired-process-event-log-rank-bootstrap-v1"
SEEDS = (66001, 66002)
BOOTSTRAPS = 10_000
LIMIT = 1.05
CALLS = (1, 10, 100, 10_000)
EVENTS = (
    "cold_initial", "compile_initial", "warm_initial",
    "cold_revisit", "recompile", "warm_revisit",
)
METRICS = ("p50", "p99", "max")


def _need(ok: Any, message: str) -> None:
    if not ok:
        raise ValueError(message)


def _number(value: Any, label: str) -> float:
    _need(type(value) in (int, float) and math.isfinite(value), f"{label} must be finite")
    return float(value)


def _interval(value: Any, label: str) -> tuple[float, float]:
    _need(isinstance(value, list) and len(value) == 2, f"{label} must be a two-value CI")
    low, high = (_number(x, label) for x in value)
    _need(0 < low <= high, f"{label} must be ordered and positive")
    return low, high


def _decision(focused: dict[str, Any]) -> str:
    _need(isinstance(focused, dict) and focused.get("status") == "passed"
          and focused.get("task") == "P066", "focused P066 result must be passed")
    if focused.get("schema") == "wtime-verification-statistics-v1":
        _need(focused.get("statistics_computations") == 1,
              "focused statistics must have been computed exactly once")
    result = focused.get("decision")
    if isinstance(result, dict):
        _need(result.get("status") == "passed" and result.get("task") == "P066",
              "focused candidate decision identity differs")
        value = result.get("decision")
        reasons = result.get("reasons")
        _need(isinstance(reasons, list), "focused decision must retain its reasons")
        _need((value == "adopted" and not reasons)
              or (value in ("rejected", "inconclusive") and bool(reasons)),
              "focused decision and reasons are inconsistent")
    else:
        value = result
        reasons = focused.get("reasons")
        _need(isinstance(reasons, list)
              and ((value == "adopted" and not reasons)
                   or (value in ("rejected", "inconclusive") and bool(reasons))),
              "focused decision and reasons are inconsistent")
    _need(value in ("adopted", "rejected", "inconclusive"),
          "focused decision is not a standard candidate outcome")
    return value


def _check_analysis_shell(value: Any, *, schemas: set[str], method: str, metric: str,
                          label: str) -> dict[str, Any]:
    _need(isinstance(value, dict), f"{label} analysis must be an object")
    _need(isinstance(value.get("schema"), str) and value.get("schema") in schemas
          and value.get("status") == "passed"
          and value.get("metric") == metric and value.get("method") == method,
          f"{label} schema, status, metric, or method differs")
    _need(value.get("timing_evidence") is True and value.get("gate_eligible") is True
          and value.get("decision") is None and value.get("threshold_selection") is None
          and value.get("scaled_by_calls") is False,
          f"{label} must be full supplementary evidence without a candidate decision")
    common = {"schema", "status", "metric", "raw_unit", "method", "report_sha256",
              "collection_identity_sha256", "analysis_source_refs", "input_refs",
              "timing_evidence", "gate_eligible", "decision", "threshold_selection",
              "scaled_by_calls", "runs", "limitations"}
    optional = {"analysis_helpers", "comparison", "invocation"}
    _need(common.issubset(value) and not (set(value) - common - optional),
          f"{label} top-level fields differ")
    return value


def _check_totals(totals: dict[str, Any]) -> list[dict[str, Any]]:
    totals = _check_analysis_shell(totals, schemas=TOTAL_SCHEMAS, method=TOTAL_METHOD,
                                   metric="entry_total", label="P137 totals")
    _need(totals.get("raw_unit") == "ns", "P137 raw unit must be ns")
    runs = totals.get("runs")
    _need(isinstance(runs, list) and len(runs) == 2, "P137 requires exactly two runs")
    expected: dict[int, dict[str, Any]] | None = None
    accepted = []
    for i, run in enumerate(runs):
        _need(isinstance(run, dict) and set(run) == {"run_id", "seed", "rows"}
              and type(run["run_id"]) is int and run["run_id"] == i
              and type(run["seed"]) is int and run["seed"] == SEEDS[i],
              "P137 run identity or seed differs")
        rows = run["rows"]
        _need(isinstance(rows, list) and len(rows) == 4, "P137 must contain exactly four rows per run")
        by_calls: dict[int, dict[str, Any]] = {}
        for row in rows:
            _need(isinstance(row, dict) and set(row) == {
                "row_id", "settings", "pairs", "raw_unit", "metric", "bootstrap_seed",
                "candidate_over_baseline", "ci95", "bootstrap_resamples", "baseline", "candidate",
            }, "P137 row fields differ")
            settings = row["settings"]
            _need(isinstance(settings, dict) and set(settings) == {
                "fixture", "case", "input", "expected", "calls", "baseline_strategy",
                "baseline_threshold", "candidate_strategy", "candidate_threshold",
                "fixture_sha256", "row_id", "metric", "mode", "feature_profile", "source_fuel",
            } and type(settings.get("calls")) is int and settings.get("calls") in CALLS
                  and type(settings.get("input")) is int
                  and type(settings.get("expected")) is int
                  and settings.get("baseline_strategy") == "eager"
                  and type(settings.get("baseline_threshold")) is int
                  and settings.get("baseline_threshold") == 0
                  and settings.get("candidate_strategy") == "adaptive"
                  and type(settings.get("candidate_threshold")) is int
                  and settings.get("candidate_threshold") == 3,
                  "P137 row settings must be eager/null versus adaptive/3")
            calls = settings["calls"]
            _need(type(calls) is int and calls not in by_calls, "duplicate or invalid P137 call count")
            _need(row["row_id"] == settings.get("row_id") == str(CALLS.index(calls))
                  and row["metric"] == "entry_total"
                  and row["raw_unit"] == "ns", "P137 row identity differs")
            _need(type(row["pairs"]) is int and row["pairs"] >= 30
                  and type(row["bootstrap_resamples"]) is int
                  and row["bootstrap_resamples"] == BOOTSTRAPS,
                  "P137 row lacks full pair or bootstrap counts")
            _need(type(row["bootstrap_seed"]) is int and row["bootstrap_seed"] >= 0,
                  "P137 bootstrap seed must be a nonnegative integer")
            ratio = _number(row["candidate_over_baseline"], "P137 ratio")
            _need(ratio > 0, "P137 ratio must be positive")
            _interval(row["ci95"], "P137 CI")
            for side in ("baseline", "candidate"):
                summary = row.get(side)
                _need(isinstance(summary, dict) and set(summary) == {
                    "total_ns", "prepare_ns", "instantiate_ns", "invoke_ns",
                }, f"P137 {side} phase summaries differ")
                for field, stats in summary.items():
                    _need(isinstance(stats, dict) and set(stats) == {
                        "median", "mad", "p25", "p75", "min", "max", "p99", "count", "unit",
                    } and stats.get("unit") == "ns"
                          and type(stats.get("count")) is int and stats.get("count") == row["pairs"],
                          f"P137 {side}/{field} summary count differs")
                    for key in ("min", "max", "median", "mad", "p25", "p75", "p99"):
                        value = _number(stats.get(key), f"P137 {side}/{field}/{key}")
                        _need(value >= 0, f"P137 {side}/{field}/{key} must be nonnegative")
                    _need(stats["min"] <= stats["median"] <= stats["max"]
                          and stats["min"] <= stats["p99"] <= stats["max"],
                          f"P137 {side}/{field} summary ordering differs")
            by_calls[calls] = row
        _need(set(by_calls) == set(CALLS), "P137 call-count matrix is incomplete")
        _need([row["settings"]["calls"] for row in rows] == list(CALLS),
              "P137 call-count rows are out of frozen order")
        signature = {calls: row["settings"] for calls, row in by_calls.items()}
        if expected is None:
            expected = signature
        else:
            _need(signature == expected, "P137 row/settings matrix differs between runs")
        accepted.append(by_calls)
    return accepted


def _check_sequence(sequences: dict[str, Any]) -> list[dict[str, Any]]:
    sequences = _check_analysis_shell(sequences, schemas={SEQUENCE_SCHEMA}, method=SEQUENCE_METHOD,
                                      metric="per_process_event_latency", label="P142 sequences")
    _need(sequences.get("raw_unit") == "ns", "P142 raw unit must be ns")
    runs = sequences.get("runs")
    _need(isinstance(runs, list) and len(runs) == 2, "P142 requires exactly two runs")
    accepted: list[dict[str, Any]] = []
    expected_events = set(EVENTS)
    expected_counts: dict[str, int] = {}
    for position in entry_sequence_policy.schedule(3):
        event = entry_sequence_policy.event_class(position, 3)
        expected_counts[event] = expected_counts.get(event, 0) + 1
    _need(set(expected_counts) == expected_events, "P141 threshold 3 schedule lacks expected classes")
    canonical_settings = None
    for i, run in enumerate(runs):
        _need(isinstance(run, dict) and set(run) == {"run_id", "seed", "rows"}
              and type(run["run_id"]) is int and run["run_id"] == i
              and type(run["seed"]) is int and run["seed"] == SEEDS[i],
              "P142 run identity or seed differs")
        rows = run["rows"]
        _need(isinstance(rows, list) and len(rows) == 1, "P142 must contain one row per run")
        row = rows[0]
        _need(isinstance(row, dict) and set(row) == {"row_id", "settings", "process_pairs", "classes"}
              and isinstance(row.get("settings"), dict)
              and set(row["settings"]) == {
                  "fixture", "fixture_sha256", "input", "schedule_threshold", "row_id",
                  "case", "metric", "mode", "feature_profile", "source_fuel",
              }
              and row["row_id"] == "0" and row["settings"].get("row_id") == "0"
              and type(row["settings"].get("schedule_threshold")) is int
              and type(row["settings"].get("input")) is int
              and row["settings"].get("schedule_threshold") == 3,
              "P142 row identity or threshold differs")
        if canonical_settings is None:
            canonical_settings = row["settings"]
        else:
            _need(row["settings"] == canonical_settings, "P142 settings differ between runs")
        _need(type(row["process_pairs"]) is int and row["process_pairs"] >= 30,
              "P142 row lacks full process-pair count")
        classes = row["classes"]
        _need(isinstance(classes, list) and len(classes) == 6
              and [c.get("event_class") for c in classes if isinstance(c, dict)] == list(EVENTS),
              "P142 event-class set differs from exact threshold-3 schedule")
        per_class: dict[str, dict[str, Any]] = {}
        for item in classes:
            _need(isinstance(item, dict) and set(item) == {
                "event_class", "raw_call_count_per_process", "process_pairs",
                "baseline", "candidate", "metrics",
            }, "P142 class fields differ")
            event = item["event_class"]
            count = expected_counts[event]
            _need(type(item["raw_call_count_per_process"]) is int
                  and item["raw_call_count_per_process"] == count
                  and type(item["process_pairs"]) is int
                  and item["process_pairs"] == row["process_pairs"],
                  f"P142 {event} schedule or pair count differs")
            for side in ("baseline", "candidate"):
                summary = item[side]
                _need(isinstance(summary, dict) and set(summary) == {
                    "process_count", "raw_call_count", "calls_per_process", "process_metrics",
                } and type(summary["process_count"]) is int
                  and summary["process_count"] == row["process_pairs"]
                  and type(summary["calls_per_process"]) is int
                  and summary["calls_per_process"] == count
                  and type(summary["raw_call_count"]) is int
                  and summary["raw_call_count"] == count * row["process_pairs"],
                  f"P142 {event}/{side} process counts differ")
                metrics_summary = summary["process_metrics"]
                _need(isinstance(metrics_summary, dict) and set(metrics_summary) == set(METRICS),
                      f"P142 {event}/{side} process metrics differ")
                for metric in METRICS:
                    stats = metrics_summary[metric]
                    _need(isinstance(stats, dict) and set(stats) == {
                        "median", "mad", "p25", "p75", "min", "max", "p99", "count", "unit",
                    } and type(stats.get("count")) is int
                          and stats.get("count") == row["process_pairs"]
                          and stats.get("unit") == "ns", "P142 process summary count differs")
                    for key in ("min", "max", "median", "mad", "p25", "p75", "p99"):
                        value = _number(stats.get(key), f"P142 {event}/{side}/{metric}/{key}")
                        _need(value >= 0, f"P142 {event}/{side}/{metric}/{key} must be nonnegative")
                    _need(stats["min"] <= stats["median"] <= stats["max"]
                          and stats["min"] <= stats["p99"] <= stats["max"],
                          f"P142 {event}/{side}/{metric} summary ordering differs")
            metrics = item["metrics"]
            _need(isinstance(metrics, dict) and set(metrics) == set(METRICS),
                  f"P142 {event} statistic metrics differ")
            seen_metrics: dict[str, dict[str, Any]] = {}
            for metric in METRICS:
                result = metrics[metric]
                _need(isinstance(result, dict) and set(result) == {
                    "candidate_over_baseline", "ci95", "bootstrap_resamples", "bootstrap_seed", "alias_of",
                }, f"P142 {event}/{metric} interval fields differ")
                alias = result["alias_of"]
                if alias is not None:
                    _need(alias in seen_metrics and type(result["bootstrap_resamples"]) is int
                          and result["bootstrap_resamples"] == 0,
                          f"P142 {event}/{metric} alias does not point to a preceding statistic")
                    source = seen_metrics[alias]
                    for field in ("candidate_over_baseline", "ci95", "bootstrap_seed"):
                        _need(result[field] == source[field], f"P142 {event}/{metric} alias {field} differs")
                    _need(result["bootstrap_seed"] is not None,
                          f"P142 {event}/{metric} alias lost source bootstrap seed")
                    _need(type(result["bootstrap_seed"]) is int and result["bootstrap_seed"] >= 0,
                          f"P142 {event}/{metric} alias seed must be a nonnegative integer")
                else:
                    _need(type(result["bootstrap_resamples"]) is int
                          and result["bootstrap_resamples"] == BOOTSTRAPS
                          and type(result["bootstrap_seed"]) is int and result["bootstrap_seed"] >= 0,
                          f"P142 {event}/{metric} must retain its full bootstrap")
                    ratio = _number(result["candidate_over_baseline"], f"P142 {event}/{metric} ratio")
                    _need(ratio > 0, f"P142 {event}/{metric} ratio must be positive")
                    _interval(result["ci95"], f"P142 {event}/{metric} CI")
                if count == 1 and metric != "p50":
                    _need(alias == "p50", f"P142 single-call {event}/{metric} must alias p50 uncertainty")
                seen_metrics[metric] = result
            if count == 1:
                for side in ("baseline", "candidate"):
                    process_metrics = item[side]["process_metrics"]
                    _need(process_metrics["p50"] == process_metrics["p99"] == process_metrics["max"],
                          f"P142 single-call {event}/{side} percentiles must be identical")
            per_class[event] = item
        accepted.append(per_class)
    return accepted


def combine(focused: dict[str, Any], totals: dict[str, Any], sequences: dict[str, Any]) -> dict[str, Any]:
    """Validate evidence shapes and combine gate outcomes without recomputation."""
    focused_decision = _decision(focused)
    total_runs = _check_totals(totals)
    sequence_runs = _check_sequence(sequences)
    rejected: list[str] = []
    inconclusive: list[str] = []
    total_rows = []
    for run_index, by_calls in enumerate(total_runs):
        for calls in CALLS:
            row = by_calls[calls]
            low, high = _interval(row["ci95"], f"P137 run {run_index + 1}/{calls} CI")
            state = "rejected" if low > LIMIT else "inconclusive" if high > LIMIT else "passed"
            if state == "rejected":
                rejected.append(f"entry_total calls={calls} run={run_index + 1} lower={low}")
            elif state == "inconclusive":
                inconclusive.append(f"entry_total calls={calls} run={run_index + 1} upper={high}")
            total_rows.append({"run": run_index + 1, "calls": calls, "ci95": [low, high], "state": state})
    sequence_rows = []
    for run_index, by_event in enumerate(sequence_runs):
        for event in EVENTS:
            item = by_event[event]
            for metric in ("p99", "max"):
                result = item["metrics"][metric]
                low, high = _interval(result["ci95"], f"P142 run {run_index + 1}/{event}/{metric} CI")
                state = "rejected" if low > LIMIT else "inconclusive" if high > LIMIT else "passed"
                if state == "rejected":
                    rejected.append(f"{event}/{metric} run={run_index + 1} lower={low}")
                elif state == "inconclusive":
                    inconclusive.append(f"{event}/{metric} run={run_index + 1} upper={high}")
                sequence_rows.append({"run": run_index + 1, "event_class": event,
                                      "metric": metric, "ci95": [low, high], "state": state})
    if focused_decision == "rejected" or rejected:
        decision = "rejected"
    elif focused_decision == "inconclusive" or inconclusive:
        decision = "inconclusive"
    else:
        decision = "adopted"
    return {
        "schema": SCHEMA,
        "status": "passed",
        "task": "P066",
        "decision": decision,
        "focused_decision": focused_decision,
        "supplementary": {
            "threshold": 3,
            "total_rows": total_rows,
            "sequence_rows": sequence_rows,
            "limits": {"ci95_upper_max": LIMIT, "pairs_per_row_min": 30,
                       "bootstrap_resamples": BOOTSTRAPS},
            "rejections": rejected,
            "inconclusive": inconclusive,
        },
        "statistics_recomputed": False,
        "timing_evidence": True,
        "gate_eligible": True,
    }
