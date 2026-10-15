#!/usr/bin/env python3
"""Source-bound paired collector for P141 individual-entry latency rows."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import random
import subprocess
import sys

import checkpoint
import entry_batch_adapter as adapter
import entry_sequence_policy as policy

SCHEMA = "wtime-entry-sequence-pairs-v1"
METRIC = "individual_entry_latency"
SIDES = ("baseline", "candidate")
ROW_FIELDS = {"fixture", "input", "schedule_threshold"}
HELPER_NAMES = ("entry_sequence_compare.py", "entry_sequence_policy.py",
                "entry_batch_adapter.py", "entry_batch_compare.py", "checkpoint.py")
IDENTITY_FIELDS = {"schema", "metric", "smoke", "timing_evidence", "gate_eligible", "pairs_per_row",
                   "seeds", "rows", "sides", "archive_manifest", "adapter_root", "candidate_root",
                   "repo", "harness", "environment", "helpers"}


def helper_paths():
    return {"entry_sequence_compare.py": Path(__file__),
            "entry_sequence_policy.py": Path(policy.__file__),
            "entry_batch_adapter.py": Path(adapter.__file__),
            "entry_batch_compare.py": Path(adapter.__file__).with_name("entry_batch_compare.py"),
            "checkpoint.py": Path(checkpoint.__file__)}


def need(condition, message):
    if not condition:
        raise ValueError(message)


def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":"),
                                    ensure_ascii=True, allow_nan=False).encode()).hexdigest()


def reference(path):
    path = Path(path).resolve(strict=True)
    return {"path": str(path), "sha256": adapter.sha256_file(path)}


def write_new(path, value):
    with Path(path).open("x", encoding="utf-8") as stream:
        json.dump(value, stream, sort_keys=True, indent=2, allow_nan=False)
        stream.write("\n")


def integer(value, name, minimum=0, maximum=None):
    need(type(value) is int and value >= minimum and (maximum is None or value <= maximum),
         f"invalid integer {name}")
    return value


def normalize_rows(rows):
    need(isinstance(rows, list) and rows, "explicit sequence rows are required")
    result = []
    for index, row in enumerate(rows):
        need(isinstance(row, dict) and set(row) == ROW_FIELDS,
             "sequence row fields differ from contract")
        fixture = Path(row["fixture"]).resolve(strict=True)
        need(fixture.is_file(), "sequence fixture is not a file")
        value = integer(row["input"], "input", -(1 << 31), (1 << 31) - 1)
        threshold = row["schedule_threshold"]
        policy.schedule(threshold)
        result.append({"fixture": str(fixture), "fixture_sha256": adapter.sha256_file(fixture),
                       "input": value, "schedule_threshold": threshold, "row_id": str(index),
                       "case": "uniform66", "metric": METRIC, "mode": "register",
                       "feature_profile": "mvp", "source_fuel": None})
    keys = [(row["fixture_sha256"], row["input"], row["schedule_threshold"]) for row in result]
    need(len(keys) == len(set(keys)), "duplicate sequence rows")
    return result


def common_identity(record):
    overlay = record.get("overlay_sources")
    need(isinstance(overlay, dict) and overlay, "missing compiled sequence protocol overlay")
    return {"overlay": {key: value.get("sha256") for key, value in overlay.items()},
            **{key: record.get(key) for key in (
                "features", "profile", "instrumented", "wasmtime_dependency", "feature_profile",
                "source_fuel", "generated_manifest_sha256", "isolated_lock_sha256", "toolchain")}}


def bind_side(root, record):
    root = Path(root).resolve(strict=True)
    binary = (root / adapter._safe_rel(record["executable"])).resolve(strict=True)
    need(binary.is_relative_to(root) and binary.is_file(), "sequence executable escapes build record")
    copied = record.get("copied_core_sources")
    need(isinstance(copied, dict) and copied, "missing compiled core source map")
    return {"build_record": reference(root / "adapter.json"), "executable": reference(binary),
            "kind": record.get("kind"), "core_sources": {key: value.get("sha256")
                                                            for key, value in copied.items()},
            "common": common_identity(record)}


def environment(cpu=None):
    allowed = sorted(os.sched_getaffinity(0))
    selected = allowed[0] if cpu is None else integer(cpu, "cpu", 0)
    need(selected in allowed, "selected CPU is outside permitted affinity")
    try:
        model = next((line.split(":", 1)[1].strip() for line in Path("/proc/cpuinfo").read_text().splitlines()
                      if line.startswith("model name")), "unknown")
    except OSError:
        model = "unknown"
    try:
        quota = Path("/sys/fs/cgroup/cpu.max").read_text().strip()
    except OSError:
        quota = "unknown"
    return {"selected_cpu": selected, "available_cpus": allowed, "cpu_model": model,
            "cpu_quota": quota, "architecture": platform.machine(), "kernel": platform.release(),
            "physical_core_idle": "unknown/shared cloud"}


def prepare(archive, adapter_root, candidate_root, repo, harness, rows, pairs, seeds,
            *, smoke, cpu=None):
    integer(pairs, "pairs", 1)
    need(type(smoke) is bool and ((smoke and pairs == 2) or (not smoke and pairs >= 30)),
         "smoke requires exactly two pairs; candidate requires at least thirty")
    need(isinstance(seeds, (list, tuple)) and len(seeds) == 2, "two run seeds are required")
    seeds = [integer(seed, "seed", 0, (1 << 63) - 1) for seed in seeds]
    need(seeds[0] != seeds[1], "run seeds must be distinct")
    normalized = normalize_rows(rows)
    archive = Path(archive).resolve(strict=True)
    adapter_root = Path(adapter_root).resolve(strict=True)
    candidate_root = Path(candidate_root).resolve(strict=True)
    repo = Path(repo).resolve(strict=True)
    harness = Path(harness).resolve(strict=True)
    before = adapter.verify(adapter_root, archive, harness)
    after = adapter.verify_current(candidate_root, repo, harness)
    need(before.get("kind") == adapter.KIND and before.get("mode") == "register",
         "baseline is not an archived Register adapter")
    need(after.get("kind") == adapter.CURRENT_KIND and after.get("mode") == "register"
         and after.get("source_origin") == "current-workspace",
         "candidate is not a current-workspace Register build")
    sides = {"baseline": bind_side(adapter_root, before),
             "candidate": bind_side(candidate_root, after)}
    need(sides["baseline"]["executable"]["path"] != sides["candidate"]["executable"]["path"],
         "baseline and candidate executables must be distinct")
    need(sides["baseline"]["common"] == sides["candidate"]["common"],
         "sequence protocol, overlay, feature or toolchain identities differ")
    common = sides["baseline"]["common"]
    need(common["features"] == adapter.FEATURES and common["profile"] == "release"
         and common["instrumented"] is False and common["wasmtime_dependency"] is False
         and common["feature_profile"] == "mvp" and common["source_fuel"] is None
         and not {"execution-reference", "performance-counters"}.intersection(common["features"]),
         "sequence timing build is instrumented or has an unsupported profile")
    need(sides["baseline"]["kind"] == adapter.KIND
         and sides["candidate"]["kind"] == adapter.CURRENT_KIND,
         "sequence source kinds differ")
    helpers = {name: reference(path) for name, path in helper_paths().items()}
    return {"schema": SCHEMA, "metric": METRIC, "smoke": smoke,
            "timing_evidence": not smoke, "gate_eligible": not smoke,
            "pairs_per_row": pairs, "seeds": seeds, "rows": normalized,
            "sides": sides, "archive_manifest": reference(archive / "manifest.json"),
            "adapter_root": str(adapter_root), "candidate_root": str(candidate_root),
            "repo": str(repo), "harness": str(harness), "environment": environment(cpu),
            "helpers": helpers}


def command(binary, row, side, cpu):
    need(side in SIDES, "unknown sequence side")
    strategy = "eager" if side == "baseline" else "adaptive"
    threshold = 0 if side == "baseline" else row["schedule_threshold"]
    return ["taskset", "-c", str(cpu), str(binary), "--entry-sequence", row["fixture"],
            str(row["input"]), str(row["schedule_threshold"]), "register", strategy, str(threshold)]


def validate_payload(payload, row, side):
    strategy = "eager" if side == "baseline" else "adaptive"
    threshold = 0 if side == "baseline" else row["schedule_threshold"]
    need(policy.validate_payload(payload, row["schedule_threshold"], strategy, threshold,
                                 row["input"], diagnostic=False) is payload,
         "sequence payload differs")
    return payload


def pair_specs(identity):
    result = []
    for run_id, seed in enumerate(identity["seeds"]):
        rng = random.Random(seed)
        for row in identity["rows"]:
            for index in range(identity["pairs_per_row"]):
                result.append({"pair_id": f"{run_id}:{row['row_id']}:{index}",
                               "run_id": run_id, "seed": seed, "row_id": row["row_id"],
                               "order": list(SIDES) if rng.getrandbits(1) else list(reversed(SIDES))})
    return result


def validate_pair(pair, spec, identity, output):
    need(isinstance(pair, dict) and set(pair) == {*spec, "observations"}, "invalid sequence pair fields")
    need(all(type(pair[key]) is type(value) and pair[key] == value for key, value in spec.items()),
         "sequence pair/run/seed/order identity differs")
    observations = pair["observations"]
    need(isinstance(observations, dict) and set(observations) == set(SIDES),
         "sequence pair is missing a side")
    row = identity["rows"][int(spec["row_id"])]
    for side in SIDES:
        obs = observations[side]
        need(isinstance(obs, dict) and set(obs) == {"side", "source_binding", "fixture_sha256", "argv",
             "returncode", "stdout", "stderr", "payload", "raw_record"},
             "invalid sequence observation shape")
        bound = identity["sides"][side]
        need(obs["side"] == side and obs["source_binding"] == digest(bound)
             and obs["fixture_sha256"] == row["fixture_sha256"], "sequence source/fixture binding differs")
        need(obs["argv"] == command(bound["executable"]["path"], row, side,
                                     identity["environment"]["selected_cpu"]),
             "sequence argv differs from frozen side settings")
        need(type(obs["returncode"]) is int and obs["returncode"] == 0
             and isinstance(obs["stdout"], str) and isinstance(obs["stderr"], str),
             "sequence command result is invalid")
        payload = json.loads(obs["stdout"])
        need(digest(payload) == digest(obs["payload"]), "sequence raw stdout differs from parsed payload")
        validate_payload(payload, row, side)
        raw_path = (Path(output) / adapter._safe_rel(obs["raw_record"])).resolve(strict=True)
        need(raw_path.is_relative_to(Path(output).resolve()) and raw_path.is_file(),
             "sequence command record escapes output")
        need(digest(json.loads(raw_path.read_text())) == digest({key: obs[key] for key in
             ("argv", "returncode", "stdout", "stderr")}), "sequence raw command record differs")


def validate_report(report, output):
    expected_fields = {"schema", "passed", "source_stable", "identity", "identity_sha256", "metric",
                       "timing_evidence", "gate_eligible", "runs", "pair_count"}
    need(isinstance(report, dict) and set(report) == expected_fields, "invalid sequence report fields")
    identity = report["identity"]
    need(isinstance(identity, dict) and set(identity) == IDENTITY_FIELDS,
         "sequence identity fields differ")
    need(report["schema"] == SCHEMA == identity.get("schema") and report["metric"] == METRIC
         and report["passed"] is True and report["source_stable"] is True,
         "sequence report identity/status differs")
    need(type(identity["smoke"]) is bool and type(identity["timing_evidence"]) is bool
         and type(identity["gate_eligible"]) is bool
         and type(identity["pairs_per_row"]) is int
         and ((identity["smoke"] and identity["pairs_per_row"] == 2)
              or (not identity["smoke"] and identity["pairs_per_row"] >= 30)),
         "sequence pair count or smoke identity is invalid")
    seeds = identity["seeds"]
    need(isinstance(seeds, list) and len(seeds) == 2
         and all(type(seed) is int and 0 <= seed <= (1 << 63) - 1 for seed in seeds)
         and seeds[0] != seeds[1], "sequence run seeds are invalid")
    need(report["timing_evidence"] is identity["timing_evidence"]
         and report["gate_eligible"] is identity["gate_eligible"]
         and report["timing_evidence"] is (not identity["smoke"])
         and report["gate_eligible"] is (not identity["smoke"]),
         "sequence smoke labeling differs")
    need(all(adapter.sha256_file(Path(row["fixture"])) == row["fixture_sha256"]
             for row in identity["rows"]), "sequence fixture changed")
    raw_rows = [{key: row[key] for key in ROW_FIELDS} for row in identity["rows"]]
    need(normalize_rows(raw_rows) == identity["rows"], "normalized sequence row identity differs")
    need(report["identity_sha256"] == digest(identity), "sequence identity digest differs")
    specs = pair_specs(identity)
    need(isinstance(report["runs"], list) and len(report["runs"]) == 2,
         "sequence report requires two independent runs")
    all_pairs = []
    for run_id, run in enumerate(report["runs"]):
        need(isinstance(run, dict) and set(run) == {"run_id", "seed", "pairs"}
             and type(run["run_id"]) is int and run["run_id"] == run_id
             and type(run["seed"]) is int and run["seed"] == identity["seeds"][run_id]
             and isinstance(run["pairs"], list), "sequence run identity differs")
        need(all(pair.get("run_id") == run_id for pair in run["pairs"]),
             "sequence pair stored under wrong run")
        all_pairs.extend(run["pairs"])
    need(type(report["pair_count"]) is int and report["pair_count"] == len(specs) == len(all_pairs),
         "sequence report has missing/duplicate pairs")
    by_id = {pair.get("pair_id"): pair for pair in all_pairs}
    need(len(by_id) == len(all_pairs) and set(by_id) == {spec["pair_id"] for spec in specs},
         "sequence pair IDs are duplicate or foreign")
    for spec in specs:
        validate_pair(by_id[spec["pair_id"]], spec, identity, output)
    return report


def _write_helpers(output, identity):
    target = output / "helpers"
    target.mkdir()
    for name, ref in identity["helpers"].items():
        path = target / name
        path.write_bytes(Path(ref["path"]).read_bytes())
        need(adapter.sha256_file(path) == ref["sha256"], "copied sequence helper changed")


def observe(side, spec, row, identity, output):
    side_dir = output / "commands" / spec["pair_id"].replace(":", "-") / side
    need(side_dir.resolve().is_relative_to(output.resolve()), "sequence command directory escapes output")
    side_dir.mkdir(parents=True, exist_ok=True)
    attempt = 1
    while True:
        attempt_dir = side_dir / f"attempt-{attempt:04d}"
        try:
            attempt_dir.mkdir(exist_ok=False)
            break
        except FileExistsError:
            attempt += 1
    argv = command(identity["sides"][side]["executable"]["path"], row, side,
                   identity["environment"]["selected_cpu"])
    write_new(attempt_dir / "invocation.json", {"argv": argv})
    try:
        result = subprocess.run(argv, text=True, capture_output=True, check=False)
        raw = {"argv": argv, "returncode": result.returncode,
               "stdout": result.stdout, "stderr": result.stderr}
    except OSError as exc:
        raw = {"argv": argv, "returncode": None, "stdout": "", "stderr": str(exc)}
    write_new(attempt_dir / "result.json", raw)
    need(type(raw["returncode"]) is int and raw["returncode"] == 0,
         f"{side} sequence command failed: {raw['stderr']}")
    payload = validate_payload(json.loads(raw["stdout"]), row, side)
    return {**raw, "side": side, "source_binding": digest(identity["sides"][side]),
            "fixture_sha256": row["fixture_sha256"], "payload": payload,
            "raw_record": (attempt_dir / "result.json").relative_to(output).as_posix()}


def collect(archive, adapter_root, candidate_root, repo, harness, rows, pairs, seeds, output,
            *, smoke=False, resume=False, cpu=None):
    output = Path(output).absolute()
    need(output == output.resolve(), "output contains symlink or path escape")
    checkpoint.Journal.preflight(output, resume=resume)
    protected = [Path(value).resolve(strict=True) for value in (archive, adapter_root, candidate_root, harness)]
    protected.extend(Path(row["fixture"]).resolve(strict=True) for row in rows)
    protected.extend((Path(repo).resolve(strict=True) / "crates",
                      Path(repo).resolve(strict=True) / "tools/wtime-conformance"))
    need(all(not output.is_relative_to(path) and not path.is_relative_to(output) for path in protected),
         "sequence output overlaps benchmark inputs")
    identity = prepare(archive, adapter_root, candidate_root, repo, harness, rows, pairs, seeds,
                       smoke=smoke, cpu=cpu)
    if resume:
        _validate_checkpoint_header(output, identity)
    journal = checkpoint.Journal(output, identity, dict(enumerate(identity["seeds"])), resume=resume)
    try:
        if resume:
            _validate_helper_copies(output, identity)
        else:
            _write_helpers(output, identity)
        specs = pair_specs(identity)
        complete = [pair for run in journal.runs() for pair in run["pairs"]]
        existing = {pair["pair_id"]: pair for pair in complete}
        expected = {spec["pair_id"]: spec for spec in specs}
        need(set(existing) <= set(expected), "checkpoint contains foreign sequence pairs")
        need([pair["pair_id"] for pair in complete] ==
             [spec["pair_id"] for spec in specs[:len(complete)]],
             "checkpoint is not a complete pair prefix")
        for pair in complete:
            validate_pair(pair, expected[pair["pair_id"]], identity, output)
        for spec in specs:
            if spec["pair_id"] in existing:
                continue
            row = identity["rows"][int(spec["row_id"])]
            pair = {**spec, "observations": {}}
            for side in spec["order"]:
                pair["observations"][side] = observe(side, spec, row, identity, output)
            validate_pair(pair, spec, identity, output)
            journal.append(spec["run_id"], spec["seed"], pair)
        after = prepare(archive, adapter_root, candidate_root, repo, harness, rows, pairs, seeds,
                        smoke=smoke, cpu=identity["environment"]["selected_cpu"])
        need(after == identity, "sequence sources, binaries or fixtures changed during collection")
        specs_report = journal.runs()
        report = {"schema": SCHEMA, "passed": True, "source_stable": True, "identity": identity,
                  "identity_sha256": digest(identity), "metric": METRIC,
                  "timing_evidence": not smoke, "gate_eligible": not smoke,
                  "runs": specs_report, "pair_count": len(specs)}
        validate_report(report, output)
        journal.write_report_once(report)
        return report
    finally:
        journal.close()


def _validate_helper_copies(output, identity):
    directory = output / "helpers"
    need(directory.is_dir(), "sequence helper archive is missing")
    expected = {name: ref["sha256"] for name, ref in identity["helpers"].items()}
    members = list(directory.iterdir())
    need(all(not path.is_symlink() and path.is_file()
             and path.resolve().is_relative_to(output.resolve()) for path in members),
         "sequence helper archive contains a non-file or escaping member")
    actual = {path.name: adapter.sha256_file(path) for path in members}
    need(actual == expected, "sequence helper archive differs")


def _validate_checkpoint_header(output, identity):
    path = Path(output) / "checkpoint.jsonl"
    with path.open("rb") as stream:
        first = stream.readline()
    need(first.endswith(b"\n"), "sequence checkpoint header is incomplete")
    header = json.loads(first)
    expected = {"type": "header", "schema": checkpoint.SCHEMA, "identity": identity,
                "identity_sha256": digest(identity),
                "run_seeds": {str(run): seed for run, seed in enumerate(identity["seeds"])} }
    need(digest(header) == digest(expected), "sequence checkpoint header differs")


def verify_collection(archive, adapter_root, candidate_root, repo, harness, rows, pairs,
                      seeds, output, *, smoke=False, cpu=None):
    output = Path(output).resolve(strict=True)
    report = json.loads((output / "report.json").read_text())
    identity = prepare(archive, adapter_root, candidate_root, repo, harness, rows, pairs, seeds,
                       smoke=smoke, cpu=cpu)
    need(digest(report.get("identity")) == digest(identity), "sequence collection inputs are stale")
    validate_report(report, output)
    journal = (output / "checkpoint.jsonl").read_bytes()
    need(journal.endswith(b"\n"), "sequence checkpoint has incomplete tail")
    events = [json.loads(line) for line in journal.splitlines()]
    header = {"type": "header", "schema": checkpoint.SCHEMA, "identity": identity,
              "identity_sha256": digest(identity),
              "run_seeds": {str(run): seed for run, seed in enumerate(identity["seeds"])} }
    need(events and digest(events[0]) == digest(header), "sequence checkpoint header differs")
    expected = [{"type": "pair", "run_id": run["run_id"], "seed": run["seed"], "pair": pair}
                for run in report["runs"] for pair in run["pairs"]]
    need(digest(events[1:]) == digest(expected), "sequence checkpoint rows differ from report")
    _validate_helper_copies(output, identity)
    return report


def verify_current_collection(path):
    """Replay an existing report using its frozen input identity; launch nothing."""
    output = Path(path).resolve(strict=True)
    report = json.loads((output / "report.json").read_text())
    identity = report.get("identity")
    need(isinstance(identity, dict) and identity.get("schema") == SCHEMA,
         "not a P142 sequence collection")
    report = verify_collection(identity["archive_manifest"]["path"].rsplit("/", 1)[0],
        identity["adapter_root"], identity["candidate_root"], identity["repo"], identity["harness"],
        [{key: row[key] for key in ROW_FIELDS} for row in identity["rows"]],
        identity["pairs_per_row"], identity["seeds"], output,
        smoke=identity["smoke"], cpu=identity["environment"]["selected_cpu"])
    return output, report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("archive", "baseline", "candidate", "repo", "harness", "rows", "output"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--pairs", type=int, required=True)
    parser.add_argument("--seeds", nargs=2, type=int, required=True)
    parser.add_argument("--cpu", type=int)
    parser.add_argument("--smoke", action="store_true")
    parser.add_argument("--resume", action="store_true")
    args = parser.parse_args()
    report = collect(args.archive, args.baseline, args.candidate, args.repo, args.harness,
                     json.loads(args.rows.read_text()), args.pairs, args.seeds, args.output,
                     smoke=args.smoke, resume=args.resume, cpu=args.cpu)
    print(json.dumps({key: report[key] for key in ("passed", "pair_count", "timing_evidence", "gate_eligible")}))


if __name__ == "__main__":
    main()
