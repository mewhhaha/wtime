#!/usr/bin/env python3
"""Validate and analyze version 2 paired wtime benchmark reports."""
from __future__ import annotations

import argparse
import hashlib
import json
import math
from pathlib import Path
import random
import sys
from typing import Any
from bootstrap import draw_median


BOOTSTRAP_RESAMPLES = 10_000
MIN_PAIRS = 30
CALL_COUNTS = (1, 10, 100, 10_000)
STAGES = ("prepare", "instantiate", "first_call", "warm_call", "load_and_call", "run_after_setup",
          "steady_state_group")
_SEED = 0x504F3034


class ReportError(ValueError):
    """The input report is incomplete, inconsistent, or invalid."""


def _finite_number(value: Any, label: str, *, positive: bool = False) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ReportError(f"{label} must be a finite number")
    number = float(value)
    if not math.isfinite(number) or (positive and number <= 0):
        qualifier = "positive and finite" if positive else "finite"
        raise ReportError(f"{label} must be {qualifier}")
    return number


def _positive_int(value: Any, label: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value <= 0:
        raise ReportError(f"{label} must be a positive integer")
    return value


def _canonical(value: Any) -> str:
    try:
        return json.dumps(value, sort_keys=True, separators=(",", ":"), allow_nan=False)
    except (TypeError, ValueError) as exc:
        raise ReportError(f"value is not valid JSON data: {exc}") from exc


def _policy_settings(settings: Any, label: str) -> dict[str, str]:
    """Extract settings that must agree between sides while allowing policy names to differ."""
    if not isinstance(settings, dict):
        raise ReportError(f"{label} must be an object")
    normalized = {}
    for key, value in settings.items():
        name = str(key).lower().replace("-", "_")
        if name in ("features", "feature", "fuel", "quantum", "cancellation_quantum"):
            normalized["quantum" if name == "cancellation_quantum" else name] = _canonical(value)
    # Accept nested side-specific settings, as emitted by mode queries, while
    # ensuring the comparison does not cross feature/fuel/quantum policies.
    side_settings = [settings[name] for name in ("baseline", "candidate") if isinstance(settings.get(name), dict)]
    if len(side_settings) == 2:
        baseline = _policy_settings(side_settings[0], label + ".baseline")
        candidate = _policy_settings(side_settings[1], label + ".candidate")
        if baseline != candidate:
            raise ReportError(f"{label} mixes feature, fuel, or quantum settings")
        normalized.update(baseline)
    elif side_settings:
        normalized.update(_policy_settings(side_settings[0], label))
    return normalized


def _percentile(values: list[float], fraction: float) -> float:
    ordered = sorted(values)
    position = fraction * (len(ordered) - 1)
    low = int(position)
    high = min(low + 1, len(ordered) - 1)
    return ordered[low] + (ordered[high] - ordered[low]) * (position - low)


def _summary(values: list[float]) -> dict[str, float]:
    median = _percentile(values, 0.5)
    return {
        "median": median,
        "mad": _percentile([abs(value - median) for value in values], 0.5),
        "p25": _percentile(values, 0.25),
        "p75": _percentile(values, 0.75),
    }


def _bootstrap_interval(log_ratios: list[float], seed: int) -> tuple[float, float]:
    rng = random.Random(seed)
    count = len(log_ratios)
    medians = []
    ordered=sorted(log_ratios)
    for _ in range(BOOTSTRAP_RESAMPLES):
        medians.append(draw_median(ordered,rng))
    lo = _percentile(medians, 0.025)
    hi = _percentile(medians, 0.975)
    return math.exp(lo), math.exp(hi)


def _seed_for(seed: int, key: tuple[str, ...]) -> int:
    digest = hashlib.sha256((str(seed) + "\0" + "\0".join(key)).encode()).digest()
    return int.from_bytes(digest[:8], "big")


def _pair_key(pair: dict[str, Any], run_index: int,
              extra_stages: tuple[str, ...] = ()) -> tuple[str, str, str, str]:
    names = ("workload", "stage", "input")
    values = []
    for name in names:
        value = pair.get(name)
        if value is None or isinstance(value, (dict, list)):
            raise ReportError(f"run {run_index} pair {pair.get('pair_id')!r}: missing or invalid {name}")
        values.append(str(value))
    if values[1] not in STAGES and values[1] not in extra_stages:
        raise ReportError(f"unknown stage {values[1]!r}")
    fixture = pair.get("fixture", pair.get("wasm_sha256"))
    if not isinstance(fixture, (str, int)) or isinstance(fixture, bool):
        raise ReportError(f"run {run_index} pair {pair.get('pair_id')!r}: missing or invalid fixture/wasm_sha256")
    if "wasm_sha256" in pair:
        digest = pair["wasm_sha256"]
        if not isinstance(digest, str) or len(digest) != 64 or any(ch not in "0123456789abcdef" for ch in digest.lower()):
            raise ReportError(f"run {run_index} pair {pair.get('pair_id')!r}: invalid wasm_sha256")
    return (values[0], values[1], values[2], str(fixture))


def _validated_runs(report: dict[str, Any]) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    if report.get("schema_version") != 2:
        raise ReportError("report schema_version must be 2")
    if report.get("passed") is not True or report.get("source_stable") is not True:
        raise ReportError("report must have passed=true and source_stable=true")
    metadata = report.get("metadata")
    runs = report.get("runs")
    if not isinstance(metadata, dict) or not isinstance(runs, list) or len(runs) != 2:
        raise ReportError("report must contain metadata and exactly two runs")
    metadata_settings = metadata.get("settings")
    if not isinstance(metadata_settings, dict):
        raise ReportError("metadata.settings must be an object")
    if metadata_settings.get("instrumented") is not False:
        raise ReportError("metadata.settings.instrumented must be false for comparable production timings")
    metadata_policy = _policy_settings(metadata_settings, "metadata.settings")
    raw_pairs = []
    for run in runs:
        if isinstance(run, dict) and isinstance(run.get("pairs"), list):
            raw_pairs.extend(pair for pair in run["pairs"] if isinstance(pair, dict))
    live_ids = {str(pair.get("workload")) for pair in raw_pairs
                if str(pair.get("workload", "")).startswith("continuation_live_")}
    legacy_live_selection = report.get("selection") == "p072-live"
    focused_p072 = (report.get("schema") == "wtime-focused-comparison-v1"
                    and report.get("selection") == "P072")
    live_entries: dict[str, dict[str, Any]] = {}
    selected_ids: set[str] = set()
    if focused_p072:
        import selection_manifest
        declaration = report.get("declaration")
        if not isinstance(declaration, dict) or declaration.get("task") != "P072":
            raise ReportError("focused live-continuation report lacks its P072 selection declaration")
        try:
            selected_entries = selection_manifest.rows(declaration)
        except (ValueError, KeyError) as exc:
            raise ReportError(f"cannot resolve source-current P072 registry rows: {exc}") from exc
        selected_ids = {entry["id"] for entry in selected_entries}
    elif legacy_live_selection:
        import workloads
        try:
            selected_entries = workloads.selection("p072-live")
        except (ValueError, KeyError) as exc:
            raise ReportError(f"cannot resolve live-continuation registry selection: {exc}") from exc
        selected_ids = {entry["id"] for entry in selected_entries}
    elif live_ids:
        raise ReportError("live-continuation pairs require a P072 focused declaration")
    else:
        selected_entries = []
    if focused_p072 or legacy_live_selection or live_ids:
        live_entries = {entry["id"]: entry for entry in selected_entries
                        if entry.get("oracle", {}).get("module") == "continuation_live_workloads.py"}
        if not live_entries:
            raise ReportError("live-continuation report selection has no registered lifecycle rows")
        if live_ids - set(live_entries):
            raise ReportError("live-continuation pair is absent from its actual registry selection")
        if (focused_p072 or legacy_live_selection) and live_ids != set(live_entries):
            raise ReportError("live-continuation report omits or substitutes a selected registry row")
        if selected_ids and any(str(pair.get("workload")) not in selected_ids for pair in raw_pairs):
            raise ReportError("P072 report contains a workload outside its frozen selection")
    continuation_ids = {str(pair.get("workload")) for pair in raw_pairs
                        if str(pair.get("workload", "")).startswith("continuation_")
                        and not str(pair.get("workload", "")).startswith("continuation_live_")}
    continuation_entries = {}
    continuation_stages = {}
    if continuation_ids:
        import continuation_pairing
        import selection_manifest
        declaration = report.get("declaration")
        if (report.get("schema") != "wtime-focused-comparison-v1"
                or not isinstance(declaration, dict) or declaration.get("task") != report.get("selection")):
            raise ReportError("paired continuations require their focused selection declaration")
        try:
            selected = selection_manifest.rows(declaration)
            continuation_entries = {entry['id']: entry for entry in selected if continuation_pairing.matches(entry)}
            for entry in continuation_entries.values():
                continuation_pairing.expected(entry)
            continuation_stages = {row['id']: tuple(row['stages']) for row in declaration['rows']
                                   if row['id'] in continuation_entries}
        except (ValueError, KeyError, TypeError) as exc:
            raise ReportError(f"invalid paired-continuation selection: {exc}") from exc
        if continuation_ids != set(continuation_entries):
            raise ReportError("paired-continuation rows differ from their selection")
    seen_seeds: set[int] = set()
    result_runs = []
    reference_groups: set[tuple[str, str, str, str]] | None = None
    for run_index, run in enumerate(runs, 1):
        if not isinstance(run, dict):
            raise ReportError(f"run {run_index} must be an object")
        seed = run.get("seed")
        if isinstance(seed, bool) or not isinstance(seed, int):
            raise ReportError(f"run {run_index} seed must be an integer")
        if seed in seen_seeds:
            raise ReportError("the two runs must have distinct seeds")
        seen_seeds.add(seed)
        pairs = run.get("pairs")
        if not isinstance(pairs, list):
            raise ReportError(f"run {run_index} pairs must be an array")
        pair_ids: set[str] = set()
        indexed: dict[tuple[str, str, str, str], list[dict[str, Any]]] = {}
        for pair in pairs:
            if not isinstance(pair, dict):
                raise ReportError(f"run {run_index} contains a non-object pair")
            pair_id = pair.get("pair_id")
            if not isinstance(pair_id, (str, int)) or isinstance(pair_id, bool):
                raise ReportError(f"run {run_index} pair has invalid pair_id")
            pair_id = str(pair_id)
            if pair_id in pair_ids:
                raise ReportError(f"run {run_index} has duplicate pair_id {pair_id!r}")
            pair_ids.add(pair_id)
            live_entry = live_entries.get(str(pair.get("workload")))
            live_stages: tuple[str, ...] = ()
            if live_entry is not None:
                import continuation_live_workloads as live_model
                live_stages = live_model.STAGES
            continuation_entry = continuation_entries.get(str(pair.get("workload")))
            if continuation_entry is not None:
                live_stages = continuation_stages[continuation_entry['id']]
            key = _pair_key(pair, run_index, live_stages)
            if continuation_entry is not None:
                if key[1] not in live_stages or pair.get('input') != 0 or type(pair.get('input')) is not int:
                    raise ReportError("paired-continuation stage/input differs from its selection")
                if key[3] != continuation_entry['source']['wasm_sha256']:
                    raise ReportError("paired-continuation fixture differs from its selection")
            if key in indexed and any(item["pair_id"] == pair_id for item in indexed[key]):
                raise ReportError(f"run {run_index} has duplicate pair_id {pair_id!r}")
            expected = pair.get("expected")
            if "expected" not in pair:
                raise ReportError(f"pair {pair_id!r} is missing expected oracle result")
            _canonical(expected)
            if continuation_entry is not None and _canonical(expected) != _canonical(continuation_pairing.expected(continuation_entry)):
                raise ReportError("paired-continuation expected lifecycle differs from its oracle")
            if live_entry is not None:
                import continuation_live_workloads as live_model
                input_value = pair.get("input")
                wanted_input = live_entry.get("params", [{}])[0].get("baseline")
                if type(input_value) is not int or input_value != wanted_input:
                    raise ReportError(f"pair {pair_id!r} live-continuation input differs from registry")
                try:
                    lifecycle = live_model.oracle(live_entry["id"], wanted_input)
                except (ValueError, KeyError, TypeError) as exc:
                    raise ReportError(f"pair {pair_id!r} live-continuation registry oracle is invalid: {exc}") from exc
                if _canonical(expected) != _canonical(lifecycle):
                    raise ReportError(f"pair {pair_id!r} live-continuation expected lifecycle differs from oracle")
            normalized = dict(pair, pair_id=pair_id)
            order = pair.get("order")
            if not isinstance(order, (list, tuple)) or len(order) != 2 or set(order) != {"baseline", "candidate"}:
                raise ReportError(f"pair {pair_id!r} order must be a permutation of baseline and candidate")
            for side_name in ("baseline", "candidate"):
                side = pair.get(side_name)
                if not isinstance(side, dict):
                    raise ReportError(f"pair {pair_id!r} is missing {side_name} observation")
                _finite_number(side.get("ns_per_operation"), f"{side_name} ns_per_operation", positive=True)
                _positive_int(side.get("batch"), f"{side_name} batch")
                if continuation_entry is not None:
                    try:
                        continuation_pairing.revalidate_observation(continuation_entry, side)
                    except (ValueError, KeyError, TypeError) as exc:
                        raise ReportError(f"paired-continuation observation is invalid: {exc}") from exc
                    if side.get('stage') != key[1]:
                        raise ReportError("paired-continuation observation stage differs")
                if live_entry is not None:
                    import continuation_live_workloads as live_model
                    wire = side.get("raw_wire_observation")
                    if not isinstance(wire, dict):
                        raise ReportError(f"pair {pair_id!r} {side_name} live-continuation wire is missing")
                    try:
                        exact_view = live_model.validate_observation(live_entry, wire, key[1])
                    except (ValueError, KeyError, TypeError) as exc:
                        raise ReportError(f"pair {pair_id!r} {side_name} live-continuation wire is invalid: {exc}") from exc
                    if _canonical(side) != _canonical(exact_view):
                        raise ReportError(f"pair {pair_id!r} {side_name} live-continuation view differs from raw wire")
                else:
                    if "result" not in side:
                        raise ReportError(f"pair {pair_id!r} {side_name} observation is missing result")
                    if _canonical(side.get("result")) != _canonical(expected):
                        raise ReportError(f"pair {pair_id!r} {side_name} result does not match oracle")
                if "settings" in side and not isinstance(side["settings"], dict):
                    raise ReportError(f"pair {pair_id!r} {side_name}.settings must be an object")
            base_settings = pair["baseline"].get("settings")
            cand_settings = pair["candidate"].get("settings")
            if base_settings is not None and cand_settings is not None:
                if _policy_settings(base_settings, "baseline settings") != _policy_settings(cand_settings, "candidate settings"):
                    raise ReportError(f"pair {pair_id!r} mixes feature, fuel, or quantum settings")
            row_settings = pair.get("settings")
            if row_settings is not None and not isinstance(row_settings, dict):
                raise ReportError(f"pair {pair_id!r}.settings must be an object")
            effective_settings = row_settings or base_settings or cand_settings
            if effective_settings is not None and metadata_policy and _policy_settings(effective_settings, f"pair {pair_id!r}.settings") != metadata_policy:
                raise ReportError(f"pair {pair_id!r} settings differ from report metadata")
            if base_settings is not None and cand_settings is not None:
                if _policy_settings(base_settings, "baseline settings") != _policy_settings(cand_settings, "candidate settings"):
                    raise ReportError(f"pair {pair_id!r} mixes feature, fuel, or quantum settings")
            for side_name, side in (("baseline", pair["baseline"]), ("candidate", pair["candidate"])):
                mode = side.get("mode")
                expected_mode = metadata_settings.get(f"{side_name}_mode")
                if live_entry is not None:
                    mode = side["raw_wire_observation"].get("mode")
                if expected_mode is not None and mode != expected_mode:
                    raise ReportError(f"pair {pair_id!r} {side_name} mode differs from report metadata")
            indexed.setdefault(key, []).append(normalized)
        if len(pair_ids) == 0:
            raise ReportError(f"run {run_index} has no observations")
        groups = set(indexed)
        if continuation_entries:
            wanted_groups = {(entry['id'], stage, '0', entry['source']['wasm_sha256'])
                             for entry in continuation_entries.values()
                             for stage in continuation_stages[entry['id']]}
            if {group for group in groups if group[0] in continuation_entries} != wanted_groups:
                raise ReportError("paired-continuation groups differ from their declared schedule")
        if focused_p072 or legacy_live_selection:
            expected_live_groups = {
                (entry["id"], stage,
                 str(entry["params"][0]["baseline"]),
                 entry["source"]["wasm_sha256"])
                for entry in live_entries.values() for stage in entry["stages"]
            }
            actual_live_groups = {group for group in groups if group[0] in live_entries}
            if actual_live_groups != expected_live_groups:
                raise ReportError("live-continuation run groups differ from the complete registry schedule")
        if reference_groups is None:
            reference_groups = groups
        elif groups != reference_groups:
            missing = sorted(reference_groups - groups)
            added = sorted(groups - reference_groups)
            raise ReportError(f"run groups differ (missing={missing[:3]}, added={added[:3]})")
        for key, observations in indexed.items():
            if len(observations) < MIN_PAIRS:
                raise ReportError(f"run {run_index} {key} has {len(observations)} pairs; at least {MIN_PAIRS} required")
        result_runs.append({"seed": seed, "groups": indexed})
    return metadata, result_runs


def _decision(stage: str, median_ratio: float, upper: float, lower: float) -> str:
    if stage == "warm_call":
        if median_ratio <= 0.95 and upper < 1.0:
            return "supported_improvement"
        if upper < 1.0:
            return "improvement_below_5_percent"
        if lower > 1.0:
            return "regression"
        return "inconclusive"
    limit = 1.25 if stage == "prepare" else (
        1.05 if stage in ("first_call", "instantiate", "load_and_call", "steady_state_group") else 1.05)
    if upper <= limit:
        return "within_budget"
    if lower > limit:
        return "over_budget"
    return "inconclusive"


def _analyze_group(key: tuple[str, str, str, str], rows: list[dict[str, Any]], seed: int, run_seed: int) -> dict[str, Any]:
    baseline = [_finite_number(row["baseline"]["ns_per_operation"], "baseline timing", positive=True) for row in rows]
    candidate = [_finite_number(row["candidate"]["ns_per_operation"], "candidate timing", positive=True) for row in rows]
    ratios = [cand / base for base, cand in zip(baseline, candidate)]
    logs = [math.log(ratio) for ratio in ratios]
    ratio_summary = _summary(ratios)
    lo, hi = _bootstrap_interval(logs, _seed_for(seed ^ run_seed, key))
    return {
        "workload": key[0], "stage": key[1], "input": key[2], "fixture": key[3],
        "pairs": len(rows), "baseline_ns_per_operation": _summary(baseline),
        "candidate_ns_per_operation": _summary(candidate), "candidate_over_baseline": ratio_summary,
        "paired_log_ratio": _summary(logs), "median_ratio_95ci": {"lower": lo, "upper": hi},
        "decision": _decision(key[1], ratio_summary["median"], hi, lo),
    }


def _break_even(run_results: list[dict[str, Any]]) -> list[dict[str, Any]]:
    by_run: dict[int, dict[tuple[str, str, str, str], dict[str, Any]]] = {}
    for result in run_results:
        run_number = result["run"]
        by_run.setdefault(run_number, {})[(result["workload"], result["stage"], result["input"], result["fixture"])] = result
    out = []
    for run_number, groups in sorted(by_run.items()):
        identities = sorted({(key[0], key[2], key[3]) for key in groups if key[1] == "warm_call"})
        for identity in identities:
            warm = groups.get((identity[0], "warm_call", identity[1], identity[2]))
            prepare = groups.get((identity[0], "prepare", identity[1], identity[2]))
            instantiate = groups.get((identity[0], "instantiate", identity[1], identity[2]))
            if warm is None:
                continue
            supported = warm["decision"] == "supported_improvement"
            if not supported:
                crossover = None
                reason = "warm savings are not statistically supported"
            elif prepare is None or instantiate is None:
                crossover = None
                reason = "prepare and instantiate measurements are required for comparable cold cost"
            else:
                extra_ns = 0.0
                for cold in (prepare, instantiate):
                    if cold is not None:
                        extra_ns += cold["candidate_ns_per_operation"]["median"] - cold["baseline_ns_per_operation"]["median"]
                saved_ns = warm["baseline_ns_per_operation"]["median"] - warm["candidate_ns_per_operation"]["median"]
                crossover = max(0, math.ceil(extra_ns / saved_ns)) if saved_ns > 0 else None
                reason = "finite crossover" if crossover is not None else "no positive warm savings"
            cold_keys = [(identity[0], stage, identity[1], identity[2]) for stage in ("prepare", "instantiate")]
            cold_complete = all(key in groups for key in cold_keys)
            base_cold = sum(groups[key]["baseline_ns_per_operation"]["median"] for key in cold_keys) if cold_complete else None
            cand_cold = sum(groups[key]["candidate_ns_per_operation"]["median"] for key in cold_keys) if cold_complete else None
            base_warm = warm["baseline_ns_per_operation"]["median"]
            cand_warm = warm["candidate_ns_per_operation"]["median"]
            out.append({
                "run": run_number, "workload": identity[0], "input": identity[1], "fixture": identity[2],
                "extra_prepare_and_instance_ns": cand_cold - base_cold if cold_complete else None,
                "saved_warm_ns_per_call": base_warm - cand_warm,
                "break_even_calls": crossover, "break_even": reason,
                "totals_ns": {str(calls): {"baseline": base_cold + base_warm * calls if cold_complete else None,
                                           "candidate": cand_cold + cand_warm * calls if cold_complete else None}
                              for calls in CALL_COUNTS},
            })
    return out


def _family_aggregates(results: list[dict[str, Any]]) -> list[dict[str, Any]]:
    grouped: dict[tuple[int, str, str], list[dict[str, Any]]] = {}
    for result in results:
        grouped.setdefault((result["run"], result["stage"], result["workload"]), []).append(result)
    # First take each workload's median across its inputs; then weight workload medians equally.
    per_workload = []
    for (run, stage, workload), entries in sorted(grouped.items()):
        value = _percentile([entry["candidate_over_baseline"]["median"] for entry in entries], 0.5)
        per_workload.append({"run": run, "stage": stage, "workload": workload, "median_ratio": value})
    by_stage: dict[tuple[int, str], list[dict[str, Any]]] = {}
    for entry in per_workload:
        by_stage.setdefault((entry["run"], entry["stage"]), []).append(entry)
    aggregate = []
    for (run, stage), entries in sorted(by_stage.items()):
        geomean = math.exp(sum(math.log(entry["median_ratio"]) for entry in entries) / len(entries))
        aggregate.append({"run": run, "stage": stage, "workloads": len(entries),
                          "equally_weighted_geomean_candidate_over_baseline": geomean,
                          "workload_medians": entries})
    return aggregate


def analyze(report: dict[str, Any]) -> dict[str, Any]:
    """Validate a schema-v2 report and return deterministic per-run statistics."""
    metadata, runs = _validated_runs(report)
    results = []
    for run_number, run in enumerate(runs, 1):
        for key, rows in sorted(run["groups"].items()):
            result = _analyze_group(key, rows, _SEED, run["seed"])
            result["run"] = run_number
            result["seed"] = run["seed"]
            results.append(result)
    return {
        "schema_version": 1,
        "source_schema_version": 2,
        "metadata": metadata,
        "analysis": {"method": "paired candidate/baseline log ratios; median bootstrap",
                     "sampling": "exact resampled median-rank distribution v1 for groups up to128 pairs; explicit resamples above128",
                     "bootstrap_resamples": BOOTSTRAP_RESAMPLES, "confidence_level": 0.95,
                     "minimum_pairs_per_group_per_run": MIN_PAIRS,
                     "bootstrap_seed": _SEED,
                     "batch_tail_latency_claims": "not calculated from batch averages"},
        "per_run": results,
        "family_aggregates": _family_aggregates(results),
        "break_even": _break_even(results),
    }


def _validate_provenance(metadata: dict[str, Any]) -> None:
    """Require source and executable identities at the file/CLI boundary."""
    for name in ("baseline_binary_sha256", "candidate_binary_sha256"):
        digest = metadata.get(name)
        if not isinstance(digest, str) or len(digest) != 64 or any(ch not in "0123456789abcdef" for ch in digest.lower()):
            raise ReportError(f"metadata.{name} must be a SHA-256 digest")
    sources = metadata.get("candidate_sources")
    if not isinstance(sources, dict) or not sources:
        raise ReportError("metadata.candidate_sources must be a nonempty source-hash map")
    for path, digest in sources.items():
        if not isinstance(path, str) or not path or not isinstance(digest, str) or len(digest) != 64 or any(ch not in "0123456789abcdef" for ch in digest.lower()):
            raise ReportError("metadata.candidate_sources contains an invalid source hash")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", required=True, type=Path, help="directory containing report.json")
    parser.add_argument("--output", required=True, type=Path, help="analysis JSON output file")
    args = parser.parse_args(argv)
    try:
        report_path = args.input if args.input.is_file() else args.input / "report.json"
        report = json.loads(report_path.read_text())
        if not isinstance(report, dict):
            raise ReportError("report root must be an object")
        metadata = report.get("metadata")
        if not isinstance(metadata, dict):
            raise ReportError("report metadata must be an object")
        _validate_provenance(metadata)
        result = analyze(report)
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(result, indent=2, sort_keys=True, allow_nan=False) + "\n")
    except (OSError, json.JSONDecodeError, ReportError) as exc:
        print(f"analyze.py: {exc}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
