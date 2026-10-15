import copy
import importlib.util
import json
import pathlib
import sys
import tempfile
import unittest
from unittest import mock


HERE = pathlib.Path(__file__).resolve().parent
REPO = HERE.parents[2]
TOOL_DIR = REPO / "tools/wtime-bench"
sys.dont_write_bytecode = True
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(TOOL_DIR))
SPEC = importlib.util.spec_from_file_location("p142_sequence_analyze", TOOL_DIR / "entry_sequence_analyze.py")
analyzer = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(analyzer)
policy = analyzer.sequence
collector = analyzer.pairs


class SequenceAnalysisTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="p142-analysis-")
        self.root = pathlib.Path(self.temp.name).resolve()
        helper_root = self.root / "helpers"
        self._checked(helper_root).mkdir()
        # Save exact executed source and invocation before constructing synthetic inputs.
        for source in (TOOL_DIR / "entry_sequence_analyze.py", HERE / "test_p142_sequence_analysis.py",
                       TOOL_DIR / "entry_sequence_policy.py", TOOL_DIR / "entry_sequence_compare.py"):
            target = helper_root / source.name
            self._checked(target).write_bytes(source.read_bytes())
        self._checked(helper_root / "command.json").write_text(json.dumps({
            "argv": [sys.executable, "-B", "-m", "unittest", *sys.argv[1:]],
        }))
        self.fixture = self.root / "uniform66.wasm"
        self._checked(self.fixture).write_bytes(b"synthetic-fixture-bytes")
        self.refs = {}
        for name in ("base-build.json", "base.exe", "candidate-build.json", "candidate.exe",
                     "manifest.json", "collector.py", "adapter.py", "checkpoint.py"):
            path = self.root / name
            self._checked(path).write_text(name)
            self.refs[name] = collector.reference(path)

    def tearDown(self):
        self.temp.cleanup()

    def _checked(self, path):
        resolved = pathlib.Path(path).resolve()
        self.assertTrue(resolved.is_relative_to(self.root), f"synthetic output escaped temp root: {resolved}")
        return resolved

    def _payload(self, n, input_value, strategy, threshold, base_ns, spike=None):
        calls = []
        for pos in policy.schedule(n):
            result = ((input_value + pos["body"] + (1 << 31)) % (1 << 32)) - (1 << 31)
            elapsed = base_ns
            if spike == (pos["body"], pos["visit"], pos["phase"]):
                elapsed = base_ns * 100
            calls.append({**pos, "result": result, "elapsed_ns": elapsed})
        return {
            "schema": policy.SCHEMA,
            "schema_version": 1,
            "driver": "entry_sequence",
            "case": "uniform66",
            "mode": "register",
            "feature_profile": "mvp",
            "source_fuel": None,
            "input": input_value,
            "strategy": strategy,
            "threshold": None if strategy == "eager" else threshold,
            "schedule_threshold": n,
            "requested_calls": 67 * (n + 1),
            "actual_calls": 67 * (n + 1),
            "instrumented": False,
            "diagnostic": False,
            "prepare_ns": 1,
            "instantiate_ns": 1,
            "per_call": calls,
        }

    def _report(self, *, smoke=False, pairs_per_row=None, row_specs=None):
        if row_specs is None:
            row_specs = [(1, 7)]
        if pairs_per_row is None:
            pairs_per_row = 2 if smoke else 30
        rows = collector.normalize_rows([
            {"fixture": str(self.fixture), "input": input_value, "schedule_threshold": n}
            for n, input_value in row_specs
        ])
        helpers = {
            "entry_sequence_compare.py": collector.reference(self.root / "collector.py"),
            "entry_batch_adapter.py": collector.reference(self.root / "adapter.py"),
            "checkpoint.py": collector.reference(self.root / "checkpoint.py"),
        }
        identity = {
            "schema": collector.SCHEMA,
            "metric": collector.METRIC,
            "smoke": smoke,
            "timing_evidence": not smoke,
            "gate_eligible": not smoke,
            "pairs_per_row": pairs_per_row,
            "seeds": [101, 202],
            "rows": rows,
            "sides": {
                "baseline": {"build_record": self.refs["base-build.json"], "executable": self.refs["base.exe"]},
                "candidate": {"build_record": self.refs["candidate-build.json"], "executable": self.refs["candidate.exe"]},
            },
            "archive_manifest": self.refs["manifest.json"],
            "adapter_root": str(self.root / "adapter"),
            "candidate_root": str(self.root / "candidate"),
            "repo": str(self.root),
            "harness": str(self.root / "harness"),
            "environment": {"selected_cpu": 0},
            "helpers": helpers,
        }
        runs = []
        for run_id, seed in enumerate(identity["seeds"]):
            run_pairs = []
            for row in rows:
                for sample in range(pairs_per_row):
                    pair_id = f"{run_id}:{row['row_id']}:{sample}"
                    observations = {}
                    for side in collector.SIDES:
                        if side == "baseline":
                            strategy, threshold, value = "eager", 0, 100
                        else:
                            strategy, threshold, value = "adaptive", row["schedule_threshold"], 200
                        spike = None
                        if side == "candidate" and run_id == 0 and sample == 0:
                            spike = (0, 1, "initial")
                        payload = self._payload(row["schedule_threshold"], row["input"],
                                                strategy, threshold, value, spike)
                        observations[side] = {"payload": payload}
                    run_pairs.append({"pair_id": pair_id, "run_id": run_id, "seed": seed,
                                      "row_id": row["row_id"], "order": list(collector.SIDES),
                                      "observations": observations})
            runs.append({"run_id": run_id, "seed": seed, "pairs": run_pairs})
        report = {
            "schema": collector.SCHEMA,
            "passed": True,
            "source_stable": True,
            "identity": identity,
            "identity_sha256": collector.digest(identity),
            "metric": collector.METRIC,
            "timing_evidence": not smoke,
            "gate_eligible": not smoke,
            "runs": runs,
            "pair_count": 2 * pairs_per_row * len(rows),
        }
        return report

    def _analyze(self, report, allow_smoke=False):
        with mock.patch.object(collector, "validate_report", return_value=report), \
             mock.patch.object(collector, "verify_current_collection",
                               side_effect=lambda path: (pathlib.Path(path), report)):
            return analyzer.analyze(report, self.root, allow_smoke=allow_smoke)

    def test_individual_spike_remains_visible_in_process_p99_and_max(self):
        report = self._report(row_specs=[(1, 7)])
        result = self._analyze(report)
        cls = next(c for c in result["runs"][0]["rows"][0]["classes"]
                   if c["event_class"] == "compile_initial")
        candidate = cls["candidate"]["process_metrics"]
        self.assertGreater(candidate["p99"]["max"], 200)
        self.assertEqual(candidate["max"]["max"], 20_000)
        self.assertEqual(cls["process_pairs"], 30)
        self.assertEqual(cls["metrics"]["p50"]["bootstrap_resamples"], 10_000)
        self.assertEqual(result["decision"], None)
        self.assertEqual(result["threshold_selection"], None)

    def test_rows_runs_classes_and_ci_are_separate_and_deterministic(self):
        report = self._report(row_specs=[(1, 7), (3, -9)])
        first = self._analyze(report)
        second = self._analyze(report)
        self.assertEqual(first["runs"], second["runs"])
        self.assertEqual(len(first["runs"]), 2)
        for run in first["runs"]:
            self.assertEqual(len(run["rows"]), 2)
            n1 = next(row for row in run["rows"] if row["settings"]["schedule_threshold"] == 1)
            n3 = next(row for row in run["rows"] if row["settings"]["schedule_threshold"] == 3)
            self.assertNotIn("cold_initial", {c["event_class"] for c in n1["classes"]})
            cold = next(c for c in n3["classes"] if c["event_class"] == "cold_initial")
            self.assertEqual(cold["raw_call_count_per_process"], 132)
            self.assertEqual(cold["baseline"]["raw_call_count"], 132 * 30)
            revisit = next(c for c in n1["classes"] if c["event_class"] == "recompile")
            self.assertEqual(revisit["metrics"]["p99"]["alias_of"], "p50")
            self.assertEqual(revisit["metrics"]["max"]["alias_of"], "p50")
            self.assertEqual(revisit["metrics"]["p99"]["bootstrap_resamples"], 0)
            self.assertEqual(revisit["metrics"]["p99"]["ci95"], revisit["metrics"]["p50"]["ci95"])

    def test_smoke_requires_opt_in_and_has_descriptive_null_uncertainty(self):
        report = self._report(smoke=True)
        with self.assertRaises(ValueError):
            self._analyze(report)
        result = self._analyze(report, allow_smoke=True)
        self.assertFalse(result["timing_evidence"])
        self.assertFalse(result["gate_eligible"])
        self.assertIsNone(result["decision"])
        self.assertIsNone(result["threshold_selection"])
        for cls in result["runs"][0]["rows"][0]["classes"]:
            for metric in cls["metrics"].values():
                self.assertIsNone(metric["candidate_over_baseline"])
                self.assertIsNone(metric["ci95"])
                self.assertEqual(metric["bootstrap_resamples"], 0)
        self.assertEqual(result["runs"][0]["rows"][0]["process_pairs"], 2)

        zero_smoke = copy.deepcopy(report)
        for run in zero_smoke["runs"]:
            for pair in run["pairs"]:
                for side in collector.SIDES:
                    for call in pair["observations"][side]["payload"]["per_call"]:
                        call["elapsed_ns"] = 0
        zero_result = self._analyze(zero_smoke, allow_smoke=True)
        self.assertTrue(all(
            cls[side]["process_metrics"][metric]["median"] == 0
            for run in zero_result["runs"] for row in run["rows"] for cls in row["classes"]
            for side in collector.SIDES for metric in ("p50", "p99", "max")
        ))

    def test_bad_sample_count_payload_shape_and_stale_fixture_are_rejected(self):
        too_few = self._report(pairs_per_row=29)
        with self.assertRaises(ValueError):
            self._analyze(too_few)
        malformed = self._report(smoke=True)
        malformed["runs"][0]["pairs"][0]["observations"]["candidate"]["payload"]["per_call"].pop()
        with self.assertRaises(ValueError):
            self._analyze(malformed, allow_smoke=True)
        zero_full = self._report()
        for run in zero_full["runs"]:
            for pair in run["pairs"]:
                for call in pair["observations"]["baseline"]["payload"]["per_call"]:
                    call["elapsed_ns"] = 0
        with self.assertRaises(ValueError):
            self._analyze(zero_full)
        stale = self._report(smoke=True)
        self._checked(self.fixture).write_bytes(b"changed fixture")
        with self.assertRaises(ValueError):
            self._analyze(stale, allow_smoke=True)


if __name__ == "__main__":
    unittest.main()
