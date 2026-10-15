#!/usr/bin/env python3
"""Collect raw, fresh-lifetime Register entry pairs without scaling timings."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import random
import re
import subprocess
import sys

import checkpoint
import entry_batch_adapter as adapter

SCHEMA = "wtime-register-entry-pairs-v1"
SCHEMA_SIDE = "wtime-register-entry-pairs-v2"
COUNTS = (1, 10, 100, 10000)
TIMINGS = ("total_ns", "prepare_ns", "instantiate_ns", "invoke_ns")
SIDES = ("baseline", "candidate")
ROW_KEYS = ("fixture", "case", "input", "expected", "calls", "strategy", "threshold")
SIDE_ROW_KEYS = ("fixture", "case", "input", "expected", "calls",
                 "baseline_strategy", "baseline_threshold", "candidate_strategy", "candidate_threshold")


def need(condition, message):
    if not condition:
        raise ValueError(message)


def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":"),
                                    allow_nan=False).encode()).hexdigest()


def integer(value, name, minimum=0, maximum=None):
    need(type(value) is int and value >= minimum and (maximum is None or value <= maximum),
         f"invalid integer {name}")
    return value


def reference(path):
    path = Path(path).resolve(strict=True)
    return {"path": str(path), "sha256": adapter.sha256_file(path)}


def write_new(path, value):
    with Path(path).open("x", encoding="utf-8") as stream:
        json.dump(value, stream, sort_keys=True, indent=2, allow_nan=False)
        stream.write("\n")


def row_keys(row):
    return SIDE_ROW_KEYS if "baseline_strategy" in row else ROW_KEYS


def side_row(row, side=None):
    """Project an explicit side into the unchanged single-process wire protocol."""
    if row_keys(row) == ROW_KEYS:
        need(side is None or side in SIDES, "unknown entry side")
        return row
    need(side in SIDES, "side-specific rows require an explicit entry side")
    return {**row, "strategy": row[side + "_strategy"], "threshold": row[side + "_threshold"]}


def row_schema(rows):
    need(isinstance(rows, list) and rows, "explicit rows are required")
    need(all(isinstance(row, dict) for row in rows), "entry rows must be objects")
    keys = row_keys(rows[0])
    need(all(row_keys(row) == keys for row in rows), "mixed entry row schemas")
    return SCHEMA_SIDE if keys == SIDE_ROW_KEYS else SCHEMA


def normalize_rows(rows):
    need(isinstance(rows, list) and rows, "explicit rows are required")
    row_schema(rows)
    result = []
    for index, row in enumerate(rows):
        need(isinstance(row, dict) and set(row) == set(row_keys(row)),
            "row fields differ from the entry contract")
        fixture = Path(row["fixture"]).resolve(strict=True)
        need(fixture.is_file(), "fixture is not a file")
        case = row["case"]
        need(isinstance(case, str) and re.fullmatch(r"[A-Za-z0-9_-]{1,128}", case), "invalid case")
        for key in ("input", "expected"):
            integer(row[key], key, -(2**31), 2**31 - 1)
        need(type(row["calls"]) is int and row["calls"] in COUNTS, "unsupported calls")
        for side in SIDES:
            setting = side_row(row, side)
            integer(setting["threshold"], "threshold", 0, 2**64 - 1)
            need((setting["strategy"] == "eager" and setting["threshold"] == 0)
                 or (setting["strategy"] == "adaptive" and setting["threshold"] > 0),
                 "invalid strategy/threshold")
        result.append({**row, "fixture": str(fixture), "fixture_sha256": adapter.sha256_file(fixture),
                       "row_id": str(index), "metric": "entry_total", "mode": "register",
                       "feature_profile": "mvp", "source_fuel": None})
    need(len({digest({k: v for k, v in row.items() if k != "row_id"}) for row in result}) == len(result),
         "duplicate rows")
    return result


def common_identity(record):
    overlay = record.get("overlay_sources", {})
    need(isinstance(overlay, dict) and overlay, "missing compiled harness sources")
    return {"overlay": {key: value["sha256"] for key, value in overlay.items()},
            **{key: record.get(key) for key in (
                "features", "profile", "instrumented", "wasmtime_dependency", "feature_profile",
                "source_fuel", "generated_manifest_sha256", "isolated_lock_sha256", "toolchain")}}


def bind_side(root, record):
    root = Path(root).resolve(strict=True)
    binary = (root / adapter._safe_rel(record["executable"])).resolve(strict=True)
    need(binary.is_relative_to(root), "executable escapes build record")
    core = {key: value["sha256"] for key, value in record["copied_core_sources"].items()}
    need(core, "missing core source map")
    return {"build_record": reference(root / "adapter.json"), "executable": reference(binary),
            "kind": record["kind"], "core_sources": core, "common": common_identity(record)}


def prepare(archive, adapter_root, candidate_root, repo, harness, rows, pairs, seeds,
            smoke, same_source_smoke):
    integer(pairs, "pairs", 1)
    need(type(smoke) is bool and type(same_source_smoke) is bool, "smoke flags must be booleans")
    need((smoke and pairs == 2) or (not smoke and pairs >= 30), "invalid smoke/candidate pair count")
    need(not same_source_smoke or smoke, "same-source alias cannot be candidate evidence")
    need(isinstance(seeds, (list, tuple)) and len(seeds) == 2, "two run seeds are required")
    for seed in seeds:
        integer(seed, "seed", 0, 2**63 - 1)
    need(seeds[0] != seeds[1], "run seeds must be independent")
    baseline = adapter.verify(Path(adapter_root), Path(archive), Path(harness))
    if same_source_smoke:
        need(Path(candidate_root).resolve() == Path(adapter_root).resolve(), "smoke alias must use exact adapter")
        candidate = baseline
    else:
        need(Path(candidate_root).resolve() != Path(adapter_root).resolve(), "candidate build must be distinct")
        candidate = adapter.verify_current(Path(candidate_root), Path(repo), Path(harness))
    sides = {"baseline": bind_side(adapter_root, baseline), "candidate": bind_side(candidate_root, candidate)}
    common = sides["baseline"]["common"]
    need(common == sides["candidate"]["common"], "protocol/driver/config/toolchain identities differ")
    need(common["features"] == adapter.FEATURES and common["profile"] == "release"
         and common["instrumented"] is False and common["wasmtime_dependency"] is False
         and common["feature_profile"] == "mvp" and common["source_fuel"] is None,
         "timing build configuration is invalid")
    need(not {"execution-reference", "performance-counters"}.intersection(common["features"]),
         "instrumented timing binary")
    if not same_source_smoke:
        need(sides["baseline"]["executable"]["path"] != sides["candidate"]["executable"]["path"],
             "same binary cannot be candidate evidence")
    return {"schema": row_schema(rows), "metric": "entry_total", "smoke": smoke,
            "same_source_alias": same_source_smoke, "timing_evidence": not smoke,
            "pairs_per_row": pairs, "seeds": list(seeds), "rows": normalize_rows(rows), "sides": sides,
            "archive_manifest": reference(Path(archive) / "manifest.json"),
            "helpers": {name: reference(Path(__file__).parent / name) for name in (
                "entry_batch_compare.py", "entry_batch_adapter.py", "checkpoint.py")}}


def command(binary, row, side=None):
    row = side_row(row, side)
    return [str(binary), "--entry-batch", row["fixture"], row["case"], str(row["input"]),
            str(row["expected"]), str(row["calls"]), "register", row["strategy"], str(row["threshold"])]


def validate_payload(payload, row, side=None):
    row = side_row(row, side)
    need(isinstance(payload, dict), "entry response is not an object")
    fields = {"schema", "schema_version", "driver", "case", "input", "expected", "result", "mode",
              "strategy", "threshold", "requested_calls", "actual_calls", "instrumented",
              "source_fuel", "feature_profile", "fixture_calls", *TIMINGS}
    need(set(payload) == fields, "unexpected response fields or divided timings")
    need(payload["schema"] == "wtime-entry-batch-v1" and type(payload["schema_version"]) is int
         and payload["schema_version"] == 1 and payload["driver"] == "entry_batch", "entry schema differs")
    for key in ("case", "mode", "strategy", "feature_profile"):
        need(payload[key] == row[key], f"entry {key} differs")
    for key in ("input", "expected", "result"):
        integer(payload[key], key, -(2**31), 2**31 - 1)
        need(payload[key] == row["expected" if key == "result" else key], f"entry {key} differs")
    for key in ("requested_calls", "actual_calls"):
        integer(payload[key], key, 1)
        need(payload[key] == row["calls"], "entry calls differ")
    threshold = None if row["strategy"] == "eager" else row["threshold"]
    if threshold is not None:
        integer(payload["threshold"], "threshold", 1, 2**64 - 1)
    need(payload["threshold"] == threshold, "entry threshold differs")
    need(payload["instrumented"] is False and payload["source_fuel"] is None, "instrumented/fuel timing")
    if row["case"] == "p135_counter":
        integer(payload["fixture_calls"], "fixture_calls", 1)
        need(payload["fixture_calls"] == row["calls"], "fixture calls differ")
    else:
        need(payload["fixture_calls"] is None, "unexpected fixture counter")
    for key in TIMINGS:
        integer(payload[key], key)
    need(payload["total_ns"] == sum(payload[key] for key in TIMINGS[1:]), "phase timings do not sum to total")
    return payload


def pair_specs(identity):
    specs = []
    for run, seed in enumerate(identity["seeds"]):
        rng = random.Random(seed)
        for row in identity["rows"]:
            for index in range(identity["pairs_per_row"]):
                specs.append({"pair_id": f"{run}:{row['row_id']}:{index}", "run_id": run, "seed": seed,
                              "row_id": row["row_id"], "order": list(SIDES) if rng.getrandbits(1) else list(reversed(SIDES))})
    return specs


def validate_pair(pair, spec, identity, output):
    need(isinstance(pair, dict) and set(pair) == {*spec, "observations"}, "missing/extra pair fields")
    need(all(pair[key] == value and type(pair[key]) is type(value) for key, value in spec.items()),
         "pair/run/seed/row/order join differs")
    observations = pair["observations"]
    need(isinstance(observations, dict) and set(observations) == set(SIDES), "missing or extra pair side")
    row = identity["rows"][int(spec["row_id"])]
    for side in SIDES:
        obs = observations[side]
        need(isinstance(obs, dict) and set(obs) == {"side", "source_binding", "fixture_sha256", "argv",
             "returncode", "stdout", "stderr", "payload", "raw_record"}, "invalid observation fields")
        bound = identity["sides"][side]
        need(obs["side"] == side and obs["source_binding"] == digest(bound)
             and obs["fixture_sha256"] == row["fixture_sha256"], "observation source/fixture join differs")
        need(obs["argv"] == command(bound["executable"]["path"], row, side), "side argv/settings differ")
        need(type(obs["returncode"]) is int and obs["returncode"] == 0, "entry command failed")
        need(isinstance(obs["stdout"], str) and isinstance(obs["stderr"], str), "missing raw output")
        payload = json.loads(obs["stdout"])
        need(payload == obs["payload"], "raw JSON differs from preserved payload")
        validate_payload(payload, row, side)
        rel = adapter._safe_rel(obs["raw_record"])
        raw = (Path(output) / rel).resolve(strict=True)
        need(raw.is_relative_to(Path(output).resolve()) and raw.is_file(), "raw observation escapes output")
        expected = {key: obs[key] for key in ("argv", "returncode", "stdout", "stderr")}
        need(json.loads(raw.read_text()) == expected, "raw command record differs")


def validate_report(report, output):
    need(isinstance(report, dict) and set(report) == {"schema", "passed", "source_stable", "identity",
         "identity_sha256", "metric", "timing_evidence", "same_source_alias", "runs", "pair_count"},
         "invalid paired report fields")
    identity = report["identity"]
    need(report["schema"] in (SCHEMA, SCHEMA_SIDE)
         and report["schema"] == identity.get("schema") == row_schema(identity["rows"])
         and report["metric"] == "entry_total"
         and report["passed"] is True and report["source_stable"] is True, "incomplete paired report")
    need(all(adapter.sha256_file(Path(row["fixture"])) == row["fixture_sha256"]
             for row in identity["rows"]), "stale fixture")
    rows = [{key: row[key] for key in row_keys(row)} for row in identity["rows"]]
    need(normalize_rows(rows) == identity["rows"], "paired row/settings identity differs")
    need(report["identity_sha256"] == digest(identity), "paired identity digest differs")
    need(report["timing_evidence"] is identity["timing_evidence"]
         and report["same_source_alias"] is identity["same_source_alias"], "smoke labeling differs")
    specs = pair_specs(identity)
    need(isinstance(report["runs"], list) and len(report["runs"]) == 2, "missing independent run")
    actual = []
    for run, item in enumerate(report["runs"]):
        need(set(item) == {"run_id", "seed", "pairs"} and type(item["run_id"]) is int
             and item["run_id"] == run and type(item["seed"]) is int
             and item["seed"] == identity["seeds"][run], "run identity differs")
        need(isinstance(item["pairs"], list), "missing run pairs")
        need(all(pair.get("run_id") == run for pair in item["pairs"]), "pair stored in wrong run")
        actual.extend(item["pairs"])
    need(type(report["pair_count"]) is int and report["pair_count"] == len(specs) == len(actual),
         "missing/duplicate pairs")
    by_id = {pair["pair_id"]: pair for pair in actual}
    need(len(by_id) == len(actual) and set(by_id) == {spec["pair_id"] for spec in specs}, "duplicate/foreign pair IDs")
    for spec in specs:
        validate_pair(by_id[spec["pair_id"]], spec, identity, output)
    return report


def verify_collection(output, archive, adapter_root, candidate_root, repo, harness, rows,
                      pairs, seeds, *, smoke=True, same_source_smoke=False):
    """Revalidate sources and the complete report/journal without launching Wasm."""
    output = Path(output).resolve(strict=True)
    report = json.loads((output / "report.json").read_text())
    identity = prepare(archive, adapter_root, candidate_root, repo, harness, rows, pairs,
                       seeds, smoke, same_source_smoke)
    need(report.get("identity") == identity, "collection inputs are stale")
    validate_report(report, output)
    raw = (output / "checkpoint.jsonl").read_bytes()
    need(raw.endswith(b"\n"), "checkpoint has incomplete tail")
    events = [json.loads(line) for line in raw.splitlines()]
    need(events and events[0] == {"type": "header", "schema": checkpoint.SCHEMA,
         "identity": identity, "identity_sha256": digest(identity),
         "run_seeds": {str(run): seed for run, seed in enumerate(seeds)}}, "checkpoint header differs")
    expected = [{"type": "pair", "run_id": item["run_id"], "seed": item["seed"], "pair": pair}
                for item in report["runs"] for pair in item["pairs"]]
    need(events[1:] == expected, "checkpoint pairs differ from complete report")
    helpers = output / "helpers"
    need({p.name: adapter.sha256_file(p) for p in helpers.iterdir()} == {
        name: ref["sha256"] for name, ref in identity["helpers"].items()}, "executed helper copy differs")
    return report


def observe(side, spec, row, identity, output):
    # Each invocation is a new process; no guest preflight or warmup is issued.
    side_dir = output / "commands" / spec["pair_id"].replace(":", "-") / side
    need(side_dir.resolve().is_relative_to(output), "command output escapes collection root")
    side_dir.mkdir(parents=True, exist_ok=True)
    attempt = 1
    while True:
        raw_dir = side_dir / f"attempt-{attempt:04d}"
        try:
            raw_dir.mkdir(exist_ok=False)
            break
        except FileExistsError:
            attempt += 1
    argv = command(identity["sides"][side]["executable"]["path"], row, side)
    write_new(raw_dir / "invocation.json", {"argv": argv})
    result = subprocess.run(argv, text=True, capture_output=True, check=False)
    raw = {"argv": argv, "returncode": result.returncode, "stdout": result.stdout, "stderr": result.stderr}
    write_new(raw_dir / "result.json", raw)
    need(result.returncode == 0, f"{side} entry command failed: {result.stderr}")
    payload = validate_payload(json.loads(result.stdout), row, side)
    return {**raw, "side": side, "source_binding": digest(identity["sides"][side]),
            "fixture_sha256": row["fixture_sha256"], "payload": payload,
            "raw_record": (raw_dir / "result.json").relative_to(output).as_posix()}


def collect(archive, adapter_root, candidate_root, repo, harness, rows, pairs, seeds, output,
            *, smoke=True, same_source_smoke=False, resume=False):
    output = Path(output).absolute()
    need(output == output.resolve(), "output contains symlink or relative path escape")
    checkpoint.Journal.preflight(output, resume=resume)
    protected = [Path(p).resolve(strict=True) for p in (archive, adapter_root, candidate_root, harness)]
    protected.extend(Path(row["fixture"]).resolve(strict=True) for row in rows)
    protected.extend((Path(repo).resolve() / "crates", Path(repo).resolve() / "tools/wtime-conformance"))
    need(all(not output.is_relative_to(path) and not path.is_relative_to(output) for path in protected),
         "output overlaps benchmark inputs")
    identity = prepare(archive, adapter_root, candidate_root, repo, harness, rows, pairs, seeds, smoke, same_source_smoke)
    journal = checkpoint.Journal(output, identity, dict(enumerate(seeds)), resume=resume)
    try:
        helpers = output / "helpers"
        if not resume:
            helpers.mkdir()
            for name, ref in identity["helpers"].items():
                (helpers / name).write_bytes(Path(ref["path"]).read_bytes())
        else:
            need({p.name: adapter.sha256_file(p) for p in helpers.iterdir()} == {
                name: ref["sha256"] for name, ref in identity["helpers"].items()}, "executed helper copy differs")
        specs = pair_specs(identity)
        completed = [pair for run in journal.runs() for pair in run["pairs"]]
        existing = {pair["pair_id"]: pair for pair in completed}
        expected = {spec["pair_id"]: spec for spec in specs}
        need(set(existing).issubset(expected), "checkpoint contains foreign pairs")
        need([pair["pair_id"] for pair in completed] == [spec["pair_id"] for spec in specs[:len(completed)]],
             "checkpoint is not a complete pair prefix")
        for pair_id, pair in existing.items():
            validate_pair(pair, expected[pair_id], identity, output)
        for spec in specs:
            if spec["pair_id"] in existing:
                continue
            pair = {**spec, "observations": {}}
            row = identity["rows"][int(spec["row_id"])]
            # Retain failed/incomplete command directories. Resume uses a new
            # attempt suffix, never overwrites a completed side's raw evidence.
            for side in spec["order"]:
                pair["observations"][side] = observe(side, spec, row, identity, output)
            validate_pair(pair, spec, identity, output)
            journal.append(spec["run_id"], spec["seed"], pair)
        after = prepare(archive, adapter_root, candidate_root, repo, harness, rows, pairs, seeds, smoke, same_source_smoke)
        need(after == identity, "sources/binaries/fixtures changed during collection")
        report = {"schema": identity["schema"], "passed": True, "source_stable": True, "identity": identity,
                  "identity_sha256": digest(identity), "metric": "entry_total",
                  "timing_evidence": not smoke, "same_source_alias": same_source_smoke,
                  "runs": journal.runs(), "pair_count": len(specs)}
        validate_report(report, output)
        journal.write_report_once(report)
        return report
    finally:
        journal.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("archive", "adapter", "candidate", "repo", "harness", "rows", "output"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--pairs", type=int, required=True)
    parser.add_argument("--seeds", nargs=2, type=int, required=True)
    parser.add_argument("--smoke", action="store_true")
    parser.add_argument("--same-source-smoke", action="store_true")
    parser.add_argument("--resume", action="store_true")
    args = parser.parse_args()
    report = collect(args.archive, args.adapter, args.candidate, args.repo, args.harness,
                     json.loads(args.rows.read_text()), args.pairs, args.seeds, args.output,
                     smoke=args.smoke, same_source_smoke=args.same_source_smoke, resume=args.resume)
    print(json.dumps({key: report[key] for key in ("passed", "pair_count", "timing_evidence", "same_source_alias")}))


if __name__ == "__main__":
    main()
