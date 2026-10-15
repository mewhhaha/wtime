#!/usr/bin/env python3
"""Analyze two-run Wasmtime comparisons across the original and proposed tiers.

This is a mechanical report reader: every timing number in its output comes
from one of the eight immutable reports named by the input manifest.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import random
import statistics
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import bootstrap  # noqa: E402
import bootstrap  # noqa: E402

TIERS = {
    "decoded": ("decoded", None),
    "compact": ("compact", None),
    "register-eager": ("register", None),
    "register-adaptive-3": ("register", 3),
}
RUNS = (1, 2)
WORKLOADS = ("call_overhead", "integer_loop", "memory_fill_scan", "branch_calls")
STAGES = ("prepare", "instantiate", "first_call", "warm_call", "load_and_call")
ENGINES = ("wtime", "wasmtime")
GROUPS = tuple((workload, stage) for workload in WORKLOADS for stage in STAGES) + (("prepare_large", "prepare"),)
RESAMPLES = 10000


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def percentile(values, fraction):
    values = sorted(values)
    if not values:
        raise ValueError("cannot summarize an empty sample")
    position = fraction * (len(values) - 1)
    low = int(position)
    high = min(low + 1, len(values) - 1)
    return values[low] + (values[high] - values[low]) * (position - low)


def _finite_positive(value, where):
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value) or value <= 0:
        raise ValueError(f"{where} must be positive and finite")
    return float(value)


def _policy(report, tier):
    if report.get("passed") is not True or report.get("source_stable") is not True:
        raise ValueError("report must have passed and be source stable")
    if report.get("wasm_correctness_checked") is not True:
        raise ValueError("report must record Wasm correctness checks")
    if report.get("samples") != 30 or report.get("wasmtime_version") != "49.0.2":
        raise ValueError("report must use 30 samples and Wasmtime 49.0.2")
    settings = report.get("settings")
    if not isinstance(settings, dict):
        raise ValueError("report settings must be an object")
    mode, threshold = TIERS[tier]
    if settings.get("wtime_mode") != mode or settings.get("register_entry_threshold") != threshold:
        raise ValueError(f"report settings do not match tier {tier}")
    expected = {"profile": "release", "wasmtime_features": ["std", "runtime", "cranelift"],
                "opcode_coverage": False, "cranelift_optimization": "speed", "fuel": False,
                "epoch_interruption": False, "wasmtime_cache": False,
                "wasmtime_parallel_compilation": False, "wtime_features": "mvp"}
    if any((settings.get(key) is not value if isinstance(value, bool) else settings.get(key) != value)
           for key, value in expected.items()):
        raise ValueError("report settings do not match the required comparison configuration")


def _read_manifest(manifest_path):
    manifest_path = Path(manifest_path).resolve()
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    refs = manifest.get("reports") if isinstance(manifest, dict) else None
    if not isinstance(refs, list) or len(refs) != 8:
        raise ValueError("manifest must contain exactly eight report references in reports")
    seen = set()
    entries = {}
    for ref in refs:
        if not isinstance(ref, dict) or set(("tier", "run", "path", "sha256", "bytes")) - ref.keys():
            raise ValueError("each report reference needs tier, run, path, sha256, and bytes")
        tier, run = ref["tier"], ref["run"]
        if tier not in TIERS or type(run) is not int or run not in RUNS:
            raise ValueError("report reference has an unsupported tier or run")
        key = (tier, run)
        if key in seen:
            raise ValueError(f"duplicate report reference: {key}")
        seen.add(key)
        path = (manifest_path.parent / ref["path"]).resolve()
        data = path.read_bytes()
        if type(ref["bytes"]) is not int or len(data) != ref["bytes"]:
            raise ValueError(f"report byte count mismatch: {path}")
        if ref["sha256"] != sha256(data):
            raise ValueError(f"report SHA-256 mismatch: {path}")
        report = json.loads(data)
        _policy(report, tier)
        entries[key] = (path, ref["sha256"], len(data), report)
    expected = {(tier, run) for tier in TIERS for run in RUNS}
    if seen != expected:
        raise ValueError("manifest must contain both runs for each required tier")
    return entries


def _validate_groups(report):
    rows = report.get("raw_samples")
    if not isinstance(rows, list):
        raise ValueError("raw_samples must be a list")
    collected = {group: {engine: {} for engine in ENGINES} for group in GROUPS}
    for row in rows:
        if not isinstance(row, dict):
            raise ValueError("raw sample must be an object")
        workload, stage, engine = row.get("workload"), row.get("stage"), row.get("engine")
        group = (workload, stage)
        if group not in collected or engine not in ENGINES:
            raise ValueError(f"unexpected timing group or engine: {group!r}, {engine!r}")
        sample = row.get("sample")
        if type(sample) is not int or not 0 <= sample < 30:
            raise ValueError("sample index must be an integer from 0 through 29")
        batch = row.get("batch")
        if type(batch) is not int or batch < 1 or (stage != "warm_call" and batch != 1):
            raise ValueError("batch must be positive and cold-stage batches must equal one")
        if sample in collected[group][engine]:
            raise ValueError(f"duplicate sample: {group}, {engine}, {sample}")
        collected[group][engine][sample] = _finite_positive(row.get("ns_per_operation"), "ns_per_operation")
    expected = set(range(30))
    for group, engines in collected.items():
        for engine in ENGINES:
            if set(engines[engine]) != expected:
                raise ValueError(f"missing paired samples in {group}, {engine}")
    return collected


def paired_interval(ratios, seed, resamples=RESAMPLES):
    """Median and percentile bootstrap CI using the shared exact-rank sampler."""
    if not ratios or any(not math.isfinite(value) or value <= 0 for value in ratios):
        raise ValueError("paired ratios must be positive and finite")
    observed = statistics.median(ratios)
    ordered = sorted(ratios)
    rng = random.Random(seed)
    draws = sorted(bootstrap.draw_median(ordered, rng) for _ in range(resamples))
    return {"median": observed, "ci95": [percentile(draws, .025), percentile(draws, .975)],
            "resamples": resamples, "seed": seed}


def analyze(manifest_path):
    entries = _read_manifest(manifest_path)
    source_map = None
    fixture_identity = None
    rustc_identity = None
    hardware_model = None
    seeds = {tier: set() for tier in TIERS}
    results = []
    for tier in TIERS:
        for run in RUNS:
            path, digest, size, report = entries[(tier, run)]
            current_sources = report.get("implementation_sources")
            if not isinstance(current_sources, dict) or not current_sources:
                raise ValueError("implementation_sources must be a nonempty object")
            if any(not isinstance(name, str) or not isinstance(digest, str) or
                   len(digest) != 64 or any(ch not in "0123456789abcdef" for ch in digest)
                   for name, digest in current_sources.items()):
                raise ValueError("implementation_sources must map paths to lowercase SHA-256 values")
            if "tools/wtime-bench/run.py" not in current_sources or not any(
                    name.startswith("crates/") and name.endswith(".rs") for name in current_sources):
                raise ValueError("implementation_sources must include the benchmark runner and crate Rust files")
            if source_map is None:
                source_map = current_sources
            elif current_sources != source_map:
                raise ValueError("implementation source identity differs across reports")
            workloads = report.get("workloads")
            if not isinstance(workloads, dict) or set(workloads) != set(WORKLOADS) | {"prepare_large"}:
                raise ValueError("reports must contain the same five comparison fixtures")
            identity_keys = ("input", "expected_result", "wasm_sha256", "wasm_bytes", "wat_sha256")
            identity = {}
            for name, fixture in workloads.items():
                if not isinstance(fixture, dict) or any(key not in fixture for key in identity_keys):
                    raise ValueError(f"fixture identity is incomplete for {name}")
                identity[name] = {key: fixture[key] for key in identity_keys}
            if fixture_identity is None:
                fixture_identity = identity
            elif identity != fixture_identity:
                raise ValueError("fixture keys or identities differ across reports")
            current_rustc = report.get("rustc")
            current_model = report.get("hardware", {}).get("cpu_model")
            if current_rustc is None or not isinstance(current_model, str) or not current_model:
                raise ValueError("report must include rustc identity and hardware model")
            if rustc_identity is None:
                rustc_identity, hardware_model = current_rustc, current_model
            elif current_rustc != rustc_identity or current_model != hardware_model:
                raise ValueError("rustc or hardware model differs across reports")
            seed = report.get("seed")
            if type(seed) is not int:
                raise ValueError("report seed must be an integer")
            seeds[tier].add(seed)
            samples = _validate_groups(report)
            groups = []
            for workload, stage in GROUPS:
                wtime = [samples[(workload, stage)]["wtime"][i] for i in range(30)]
                wasmtime = [samples[(workload, stage)]["wasmtime"][i] for i in range(30)]
                paired = [w / wasm for w, wasm in zip(wtime, wasmtime)]
                stable_key = f"wasmtime-v1:{tier}:run{run}:{workload}:{stage}"
                seed = int.from_bytes(hashlib.sha256(stable_key.encode()).digest()[:8], "big")
                groups.append({
                    "workload": workload, "stage": stage,
                    "engines": {name: {"median_ns": statistics.median(values),
                                       "p25_ns": percentile(values, .25),
                                       "p75_ns": percentile(values, .75), "samples": len(values)}
                                for name, values in (("wtime", wtime), ("wasmtime", wasmtime))},
                    "paired_wtime_over_wasmtime": paired_interval(paired, seed),
                    "ratio_of_engine_medians": statistics.median(wtime) / statistics.median(wasmtime),
                })
            results.append({"tier": tier, "run": run, "report": {"path": str(path), "sha256": digest, "bytes": size},
                            "recorded_at_utc": report.get("recorded_at_utc"), "settings": report["settings"],
                            "timing_boundaries": {k: report["settings"].get(k) for k in
                                ("preparation", "instantiation", "first_call", "warm_call", "load_and_call")},
                            "groups": groups})
    if any(len(tier_seeds) != 2 for tier_seeds in seeds.values()):
        raise ValueError("the two report seeds must differ within each tier")
    return {"schema": "wtime-wasmtime-analysis-v1", "informational_only": True,
            "decision": None, "promotion": None, "methods": {
                "pairing": "within report and sample index; runs kept separate",
                "paired_ratio": "median of wtime/wasmtime ratios paired by sample",
                "interval": "95% percentile bootstrap interval of median, exact-rank sampler from bootstrap.py",
                "resamples": RESAMPLES, "seed": "first 64 bits of SHA-256(stable group key)",
                "engine_summaries": "per-report median, interpolated p25/p75; ratio_of_engine_medians is distinct"},
            "tiers": list(TIERS), "source_identity": source_map, "fixture_identity": fixture_identity,
            "rustc": rustc_identity, "hardware_model": hardware_model, "runs": results,
            "limitations": ["Synthetic shared-cloud MVP workloads only; not a full application suite.",
                            "Cloud CPU affinity does not guarantee an otherwise idle physical core.",
                            "Register adaptive first-call measurements reuse a compiled module; warm-call measurements include five adaptive training calls, so adaptive state is shared across instances.",
                            "Fuel, epoch interruption, Wasmtime cache, and parallel compilation were disabled for every report.",
                            "Preparation excludes file I/O, engine initialization, and module drop; load-and-call excludes file I/O and engine initialization.",
                            "This timing comparison does not measure dependency footprint or build cost."]}


def markdown(result):
    run_groups = {(run["tier"], run["run"]): {(g["workload"], g["stage"]): g for g in run["groups"]}
                  for run in result["runs"]}
    lines = ["# Wasmtime comparison", "", "Informational comparison from the eight referenced reports. Runs remain independent; no proposal is promoted.", "",
             "## Warm execution", "", "Each cell gives the range of paired-ratio medians across the two runs (wtime / Wasmtime). Workloads are shown separately.", "",
             "| Workload | " + " | ".join(TIERS) + " |", "|---|" + "---:|" * len(TIERS)]
    for workload in WORKLOADS:
        cells = []
        for tier in TIERS:
            values = [run_groups[(tier, run)][(workload, "warm_call")]["paired_wtime_over_wasmtime"]["median"] for run in RUNS]
            cells.append(f"{min(values):.4g}–{max(values):.4g}")
        lines.append(f"| {workload} | " + " | ".join(cells) + " |")
    lines.extend(["", "## Default-tier preparation", "", "Observed medians for each report; ratios are paired by sample.", "",
                  "| Fixture | wtime ns run 1 | wtime ns run 2 | Wasmtime ns run 1 | Wasmtime ns run 2 | Paired ratio run 1 | Paired ratio run 2 |",
                  "|---|---:|---:|---:|---:|---:|---:|"])
    for workload in (*WORKLOADS, "prepare_large"):
        row = []
        for tier, run in (("decoded", 1), ("decoded", 2)):
            group = run_groups[(tier, run)][(workload, "prepare")]
            row.extend([group["engines"]["wtime"]["median_ns"], group["engines"]["wasmtime"]["median_ns"],
                        group["paired_wtime_over_wasmtime"]["median"]])
        lines.append(f"| {workload} | {row[0]:.6g} | {row[3]:.6g} | {row[1]:.6g} | {row[4]:.6g} | {row[2]:.5g} | {row[5]:.5g} |")
    lines.extend(["", "## Default-tier load and call", "", "This includes compilation through the first invocation; file I/O and engine initialization are excluded.", "",
                  "| Workload | wtime ns run 1 | wtime ns run 2 | Wasmtime ns run 1 | Wasmtime ns run 2 | Paired ratio run 1 | Paired ratio run 2 |",
                  "|---|---:|---:|---:|---:|---:|---:|"])
    for workload in WORKLOADS:
        row = []
        for run in RUNS:
            group = run_groups[("decoded", run)][(workload, "load_and_call")]
            row.extend([group["engines"]["wtime"]["median_ns"], group["engines"]["wasmtime"]["median_ns"],
                        group["paired_wtime_over_wasmtime"]["median"]])
        lines.append(f"| {workload} | {row[0]:.6g} | {row[3]:.6g} | {row[1]:.6g} | {row[4]:.6g} | {row[2]:.5g} | {row[5]:.5g} |")
    lines.extend(["", "## Per-run results", "", "Ratios are wtime / Wasmtime. The 95% interval is bootstrapped from sample-paired ratios. The ratio of medians is reported separately.", ""])
    for run in result["runs"]:
        lines.extend([f"### {run['tier']}, run {run['run']}", "",
                      "| Workload | Stage | wtime median ns (p25–p75) | Wasmtime median ns (p25–p75) | Paired ratio median [95% interval] | Ratio of medians |",
                      "|---|---|---:|---:|---:|---:|"])
        for g in run["groups"]:
            a, b = g["engines"]["wtime"], g["engines"]["wasmtime"]
            p = g["paired_wtime_over_wasmtime"]
            lines.append(f"| {g['workload']} | {g['stage']} | {a['median_ns']:.6g} ({a['p25_ns']:.6g}–{a['p75_ns']:.6g}) | {b['median_ns']:.6g} ({b['p25_ns']:.6g}–{b['p75_ns']:.6g}) | {p['median']:.5g} [{p['ci95'][0]:.5g}, {p['ci95'][1]:.5g}] | {g['ratio_of_engine_medians']:.5g} |")
        lines.append("")
    lines.extend(["## Limits", "", *[f"- {item}" for item in result["limitations"]], ""])
    return "\n".join(lines)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--markdown", type=Path, required=True)
    args = parser.parse_args()
    result = analyze(args.manifest)
    for path, contents in ((args.output, json.dumps(result, indent=2) + "\n"), (args.markdown, markdown(result))):
        path.parent.mkdir(parents=True, exist_ok=True)
        with path.open("x", encoding="utf-8") as stream:
            stream.write(contents)


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as exc:
        print(f"wasmtime_report: {exc}", file=sys.stderr)
        raise SystemExit(2)
