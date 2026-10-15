"""Contained synthetic checks for P137's offline Register entry analysis."""
from __future__ import annotations

import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

BENCH = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(BENCH))
import checkpoint
import entry_batch_analyze as analyzer
import entry_batch_compare as compare


class AnalysisFixture:
    """Build a complete but synthetic pair report under one temporary root."""

    def __init__(self, root: Path, *, smoke: bool = False):
        self.root = root.resolve()
        self.smoke = smoke
        self._mkdir("archive")
        self._mkdir("repo/tools/wtime-bench")
        self._mkdir("repo/crates")
        self._mkdir("repo/tools/wtime-conformance")
        self._write_json("archive/manifest.json", {"schema": 1, "files": {}})
        self.archive = self.root / "archive"
        self.repo = self.root / "repo"
        self.harness = self.repo / "tools/wtime-bench"
        self.baseline = self._build_record("baseline", "source-bound-register-entry-adapter")
        self.candidate = self._build_record("candidate", "source-bound-register-entry-candidate")
        self.fixture = self._write("fixture/counter.wasm", b"synthetic MVP fixture")
        self.collection = self.root / "collection"
        self._mkdir("collection")
        self.rows = [
            {"fixture": str(self.fixture), "case": "p135_counter", "input": 7, "expected": 8,
             "calls": calls, "strategy": strategy, "threshold": threshold}
            for calls in compare.COUNTS
            for strategy, threshold in (("eager", 0), ("adaptive", 3))
        ]
        self.pairs_per_row = 2 if smoke else 30
        self.seeds = [137001, 137002]
        self.identity = self._prepare()
        self.report = self._report()
        self._write_json("collection/report.json", self.report)
        self._write_checkpoint_and_helpers()

    def _path(self, relative: str) -> Path:
        path = (self.root / relative).resolve()
        if not path.is_relative_to(self.root):
            raise AssertionError(f"synthetic write escapes fresh root: {relative}")
        return path

    def _mkdir(self, relative: str) -> Path:
        path = self._path(relative)
        path.mkdir(parents=True, exist_ok=True)
        if not path.resolve().is_relative_to(self.root):
            raise AssertionError(f"synthetic directory escapes fresh root: {relative}")
        return path

    def _write(self, relative: str, data: bytes) -> Path:
        path = self._path(relative)
        path.parent.mkdir(parents=True, exist_ok=True)
        if not path.parent.resolve().is_relative_to(self.root):
            raise AssertionError(f"synthetic parent escapes fresh root: {relative}")
        path.write_bytes(data)
        return path

    def _write_json(self, relative: str, value) -> Path:
        data = (json.dumps(value, sort_keys=True, indent=2, allow_nan=False) + "\n").encode()
        return self._write(relative, data)

    def _build_record(self, name: str, kind: str) -> dict:
        base = self._mkdir(f"builds/{name}")
        executable = self._write(f"builds/{name}/binary/wtime-register-entry-adapter", b"synthetic executable")
        record = {
            "schema": ("wtime.source-bound-register-entry-adapter.v1" if name == "baseline"
                       else "wtime.source-bound-register-entry-candidate.v1"),
            "kind": kind,
            "mode": "register",
            "repository_root": str(self.repo),
            "executable": "binary/wtime-register-entry-adapter",
            "copied_core_sources": {"crates/wtime-runtime/src/lib.rs": {"sha256": "a" * 64}},
            "overlay_sources": {"src/protocol.rs": {"sha256": "b" * 64}},
            "features": list(compare.adapter.FEATURES),
            "profile": "release",
            "instrumented": False,
            "wasmtime_dependency": False,
            "feature_profile": "mvp",
            "source_fuel": None,
            "generated_manifest_sha256": "c" * 64,
            "isolated_lock_sha256": "d" * 64,
            "toolchain": {"rustc_vv": "synthetic rustc", "cargo_v": "synthetic cargo"},
        }
        self._write_json(f"builds/{name}/adapter.json", record)
        if not executable.is_file():
            raise AssertionError("synthetic executable was not created")
        return record

    def _prepare(self) -> dict:
        with mock.patch.object(compare.adapter, "verify", return_value=self.baseline), \
             mock.patch.object(compare.adapter, "verify_current", return_value=self.candidate):
            return compare.prepare(self.archive, self.root / "builds/baseline",
                                   self.root / "builds/candidate", self.repo, self.harness,
                                   self.rows, self.pairs_per_row, self.seeds,
                                   self.smoke, False)

    @staticmethod
    def _payload(row: dict, side: str) -> dict:
        base = 1000 if side == "baseline" else 500
        phases = [base // 10, base // 5, 3 * base // 10, base - 6 * base // 10]
        return {
            "schema": "wtime-entry-batch-v1", "schema_version": 1, "driver": "entry_batch",
            "case": row["case"], "input": row["input"], "expected": row["expected"],
            "result": row["expected"], "mode": "register", "strategy": row["strategy"],
            "threshold": None if row["strategy"] == "eager" else row["threshold"],
            "requested_calls": row["calls"], "actual_calls": row["calls"],
            "instrumented": False, "source_fuel": None, "feature_profile": "mvp",
            "fixture_calls": row["calls"], "total_ns": sum(phases), "prepare_ns": phases[0],
            "instantiate_ns": phases[1], "invoke_ns": phases[2] + phases[3],
        }

    def _report(self) -> dict:
        specs = compare.pair_specs(self.identity)
        runs = []
        for run_id, seed in enumerate(self.seeds):
            run_pairs = []
            for spec in (item for item in specs if item["run_id"] == run_id):
                row = self.identity["rows"][int(spec["row_id"])]
                observations = {}
                for side in compare.SIDES:
                    payload = self._payload(row, side)
                    stdout = json.dumps(payload, sort_keys=True, separators=(",", ":")) + "\n"
                    bound = self.identity["sides"][side]
                    argv = compare.command(bound["executable"]["path"], row)
                    raw = {"argv": argv, "returncode": 0, "stdout": stdout, "stderr": ""}
                    raw_rel = f"collection/raw/{spec['pair_id'].replace(':', '-')}-{side}.json"
                    self._write_json(raw_rel, raw)
                    observations[side] = {
                        **raw, "side": side, "source_binding": compare.digest(bound),
                        "fixture_sha256": row["fixture_sha256"], "payload": payload,
                        "raw_record": Path(raw_rel).relative_to(Path("collection")).as_posix(),
                    }
                run_pairs.append({**spec, "observations": observations})
            runs.append({"run_id": run_id, "seed": seed, "pairs": run_pairs})
        return {
            "schema": compare.SCHEMA, "passed": True, "source_stable": True,
            "identity": self.identity, "identity_sha256": compare.digest(self.identity),
            "metric": "entry_total", "timing_evidence": not self.smoke,
            "same_source_alias": False, "runs": runs, "pair_count": len(specs),
        }

    def _write_checkpoint_and_helpers(self) -> None:
        header = {"type": "header", "schema": checkpoint.SCHEMA, "identity": self.identity,
                  "identity_sha256": compare.digest(self.identity),
                  "run_seeds": {str(run): seed for run, seed in enumerate(self.seeds)}}
        events = [header]
        events.extend({"type": "pair", "run_id": run["run_id"], "seed": run["seed"], "pair": pair}
                      for run in self.report["runs"] for pair in run["pairs"])
        data = b"".join((json.dumps(event, sort_keys=True, separators=(",", ":")) + "\n").encode()
                        for event in events)
        self._write("collection/checkpoint.jsonl", data)
        for name, ref in self.identity["helpers"].items():
            source = Path(ref["path"]).resolve(strict=True)
            target = self._path(f"collection/helpers/{name}")
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(source.read_bytes())

    def update_observation(self, run_id: int, pair_index: int, side: str, mutate) -> None:
        pair = self.report["runs"][run_id]["pairs"][pair_index]
        obs = pair["observations"][side]
        mutate(obs["payload"])
        obs["stdout"] = json.dumps(obs["payload"], sort_keys=True, separators=(",", ":")) + "\n"
        raw_path = self._path(f"collection/{obs['raw_record']}")
        raw_path.write_text(json.dumps({key: obs[key] for key in ("argv", "returncode", "stdout", "stderr")},
                                       sort_keys=True, indent=2) + "\n")
        self._write_json("collection/report.json", self.report)


class P137EntryAnalysisTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory(prefix="p137-entry-analysis-")
        self.root = Path(self.tmp.name).resolve()

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def _fixture(self, smoke: bool = False) -> AnalysisFixture:
        return AnalysisFixture(self.root, smoke=smoke)

    def test_complete_two_run_report_keeps_each_row_and_reports_exact_point_ci(self) -> None:
        fixture = self._fixture()
        result = analyzer.analyze(fixture.report, fixture.collection)
        self.assertEqual(result["status"], "passed")
        self.assertTrue(result["timing_evidence"])
        self.assertTrue(result["gate_eligible"])
        self.assertFalse(result["scaled_by_calls"])
        self.assertEqual(len(result["runs"]), 2)
        for run in result["runs"]:
            self.assertEqual(run["run_id"], run["seed"] - 137001)
            self.assertEqual(len(run["rows"]), 8)
            self.assertEqual({row["settings"]["calls"] for row in run["rows"]}, set(compare.COUNTS))
            self.assertEqual({row["settings"]["strategy"] for row in run["rows"]}, {"eager", "adaptive"})
            for row in run["rows"]:
                self.assertEqual(row["pairs"], 30)
                self.assertEqual(row["raw_unit"], "ns")
                self.assertEqual(row["baseline"]["total_ns"]["median"], 1000)
                self.assertEqual(row["candidate"]["total_ns"]["median"], 500)
                self.assertEqual(row["candidate_over_baseline"], 0.5)
                self.assertEqual(row["ci95"], [0.5, 0.5])
                self.assertEqual(row["bootstrap_resamples"], 10_000)
                self.assertEqual(row["baseline"]["total_ns"]["unit"], "ns")
        self.assertEqual(result["schema"], "wtime-register-entry-analysis-v1")
        self.assertEqual(result["method"], "paired-log-median-rank-bootstrap-v1")
        self.assertIsNone(result["decision"])
        self.assertIsNone(result["threshold_selection"])
        self.assertEqual(result["report_sha256"], compare.digest(fixture.report))

    def test_pure_analysis_is_deterministic_and_never_launches_subprocesses(self) -> None:
        fixture = self._fixture()
        with mock.patch.object(compare.subprocess, "run", side_effect=AssertionError("benchmark launched")):
            one = analyzer.analyze(fixture.report, fixture.collection)
            two = analyzer.analyze(fixture.report, fixture.collection)
        self.assertEqual(one, two)

    def test_mutations_during_bootstrap_invalidate_report_and_bound_artifacts(self) -> None:
        report_fixture = AnalysisFixture(self.root / "report-mutation")
        original_interval = analyzer.statistics._bootstrap_interval
        first_call = True

        def mutate_report_during_bootstrap(log_ratios, seed):
            nonlocal first_call
            if first_call:
                first_call = False
                interval = original_interval(log_ratios, seed)
                report_fixture.report["identity"]["rows"][0]["input"] += 1
                return interval
            return (0.5, 0.5)

        with mock.patch.object(analyzer.statistics, "_bootstrap_interval",
                               side_effect=mutate_report_during_bootstrap):
            with self.assertRaisesRegex(ValueError, "report changed during analysis"):
                analyzer.analyze(report_fixture.report, report_fixture.collection)

        artifact_fixture = AnalysisFixture(self.root / "artifact-mutation")
        artifact = artifact_fixture._path("builds/candidate/binary/wtime-register-entry-adapter")
        changed = False

        def mutate_artifact_during_bootstrap(_log_ratios, _seed):
            nonlocal changed
            if not changed:
                changed = True
                artifact.write_bytes(artifact.read_bytes() + b"changed during bootstrap")
            return (0.5, 0.5)

        with mock.patch.object(analyzer.statistics, "_bootstrap_interval",
                               side_effect=mutate_artifact_during_bootstrap):
            with self.assertRaisesRegex(ValueError, "stale candidate executable"):
                analyzer.analyze(artifact_fixture.report, artifact_fixture.collection)

    def test_smoke_requires_opt_in_and_never_reports_uncertainty_or_gate_eligibility(self) -> None:
        fixture = self._fixture(smoke=True)
        with self.assertRaisesRegex(ValueError, "smoke cannot"):
            analyzer.analyze(fixture.report, fixture.collection)
        result = analyzer.analyze(fixture.report, fixture.collection, allow_smoke=True)
        self.assertFalse(result["timing_evidence"])
        self.assertFalse(result["gate_eligible"])
        self.assertFalse(result["scaled_by_calls"])
        for run in result["runs"]:
            for row in run["rows"]:
                self.assertEqual(row["pairs"], 2)
                self.assertIsNone(row["candidate_over_baseline"])
                self.assertIsNone(row["ci95"])
                self.assertEqual(row["bootstrap_resamples"], 0)

    def test_invalid_zero_totals_and_boolean_timing_samples_are_rejected(self) -> None:
        fixture = self._fixture()
        fixture.update_observation(0, 0, "candidate", lambda p: p.update(
            total_ns=0, prepare_ns=0, instantiate_ns=0, invoke_ns=0))
        with self.assertRaisesRegex(ValueError, "total must be positive"):
            analyzer.analyze(fixture.report, fixture.collection)

        fixture2 = self._fixture()
        fixture2.update_observation(0, 0, "baseline", lambda p: p.update(total_ns=True))
        with self.assertRaisesRegex(ValueError, "invalid integer total_ns"):
            analyzer.analyze(fixture2.report, fixture2.collection)

        fixture3 = self._fixture()
        fixture3.update_observation(0, 0, "candidate", lambda p: p.update(
            total_ns=2**128, prepare_ns=0, instantiate_ns=0, invoke_ns=2**128))
        with self.assertRaisesRegex(ValueError, "Rust u128 range"):
            analyzer.analyze(fixture3.report, fixture3.collection)

    def test_missing_duplicate_pairs_and_changed_artifact_or_fixture_are_rejected(self) -> None:
        fixture = self._fixture()
        missing = copy.deepcopy(fixture.report)
        missing["runs"][0]["pairs"].pop()
        with self.assertRaisesRegex(ValueError, "missing/duplicate pairs"):
            analyzer.analyze(missing, fixture.collection)
        duplicate = copy.deepcopy(fixture.report)
        duplicate["runs"][0]["pairs"][-1] = copy.deepcopy(duplicate["runs"][0]["pairs"][0])
        with self.assertRaisesRegex(ValueError, "duplicate/foreign pair IDs"):
            analyzer.analyze(duplicate, fixture.collection)

        binary = fixture._path("builds/candidate/binary/wtime-register-entry-adapter")
        binary.write_bytes(binary.read_bytes() + b"changed")
        with self.assertRaisesRegex(ValueError, "stale candidate executable"):
            analyzer.analyze(fixture.report, fixture.collection)
        fixture2 = self._fixture()
        fixture2._path("fixture/counter.wasm").write_bytes(fixture2.fixture.read_bytes() + b"changed")
        with self.assertRaisesRegex(ValueError, "stale fixture"):
            analyzer.analyze(fixture2.report, fixture2.collection)

    def test_run_analysis_revalidates_collection_and_writes_durable_offline_result(self) -> None:
        fixture = self._fixture()
        output = fixture.root / "analysis"
        with mock.patch.object(compare.adapter, "verify", return_value=fixture.baseline) as baseline_check, \
             mock.patch.object(compare.adapter, "verify_current", return_value=fixture.candidate) as candidate_check, \
             mock.patch.object(compare.subprocess, "run", side_effect=AssertionError("benchmark launched")):
            result = analyzer.run_analysis(fixture.collection, output)
        self.assertEqual(baseline_check.call_count, 1)
        self.assertEqual(candidate_check.call_count, 1)
        self.assertTrue((output / "report.json").is_file())
        self.assertTrue((output / "invocation.json").is_file())
        self.assertEqual(result, json.loads((output / "report.json").read_text()))
        self.assertEqual(result["comparison"]["sha256"], hashlib.sha256(
            (fixture.collection / "report.json").read_bytes()).hexdigest())
        self.assertTrue(result["gate_eligible"])
        self.assertEqual(len(result["analysis_helpers"]), 3)

    def test_run_analysis_rejects_stale_checkpoint_and_smoke_stays_ineligible(self) -> None:
        fixture = self._fixture()
        checkpoint_path = fixture._path("collection/checkpoint.jsonl")
        checkpoint_path.write_bytes(checkpoint_path.read_bytes() + b"{}\n")
        output = fixture.root / "analysis"
        with mock.patch.object(compare.adapter, "verify", return_value=fixture.baseline), \
             mock.patch.object(compare.adapter, "verify_current", return_value=fixture.candidate), \
             mock.patch.object(compare.subprocess, "run", side_effect=AssertionError("benchmark launched")):
            with self.assertRaisesRegex(ValueError, "checkpoint pairs differ"):
                analyzer.run_analysis(fixture.collection, output)
        self.assertFalse(output.exists())

        smoke = self._fixture(smoke=True)
        smoke_output = smoke.root / "smoke-analysis"
        with mock.patch.object(compare.adapter, "verify", return_value=smoke.baseline), \
             mock.patch.object(compare.adapter, "verify_current", return_value=smoke.candidate), \
             mock.patch.object(compare.subprocess, "run", side_effect=AssertionError("benchmark launched")):
            report = analyzer.run_analysis(smoke.collection, smoke_output, allow_smoke=True)
        self.assertFalse(report["timing_evidence"])
        self.assertFalse(report["gate_eligible"])
        self.assertTrue(all(row["ci95"] is None for run in report["runs"] for row in run["rows"]))


if __name__ == "__main__":
    unittest.main(verbosity=2)
