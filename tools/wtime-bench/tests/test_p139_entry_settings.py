"""Contained synthetic tests for side-specific P139 Register settings."""
from __future__ import annotations

import copy
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

BENCH = Path(__file__).resolve().parents[1]
TESTS = Path(__file__).resolve().parent
sys.path.insert(0, str(BENCH))
sys.path.insert(0, str(TESTS))

import entry_batch_compare as compare
import entry_batch_analyze as analyzer
import checkpoint
import test_p136_entry_batch as collector_tests
import test_p137_entry_analysis as analysis_tests


SEEDS = [13901, 13902]
COUNTS = (1, 10, 100, 10000)


def _inside(root: Path, path: Path) -> Path:
    root = root.resolve()
    resolved = path.resolve()
    if not resolved.is_relative_to(root):
        raise AssertionError(f"synthetic path escapes temp root: {resolved}")
    return resolved


def _v2row(fixture: Path, calls: int, reverse: bool = False) -> dict:
    baseline = ("adaptive", 3) if reverse else ("eager", 0)
    candidate = ("eager", 0) if reverse else ("adaptive", 3)
    return {
        "fixture": str(fixture), "case": "p135_counter", "input": 7,
        "expected": 8, "calls": calls,
        "baseline_strategy": baseline[0], "baseline_threshold": baseline[1],
        "candidate_strategy": candidate[0], "candidate_threshold": candidate[1],
    }


class P139EntrySettingsTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="p139-entry-settings-")
        self.root = Path(self.tmp.name).resolve()

    def tearDown(self):
        self.tmp.cleanup()

    def _collector(self, *, fail_once=False):
        fx = collector_tests.CollectorFixture(self, fail_once=fail_once)
        self.addCleanup(fx.close)
        return fx

    def _v2_collection(self, root: Path, *, smoke: bool, counts=COUNTS):
        root = _inside(self.root, root)
        fx = analysis_tests.AnalysisFixture(root / "seed", smoke=smoke)
        rows = [_v2row(fx.fixture, calls, reverse=(index % 2 == 1))
                for index, calls in enumerate(counts)]
        with mock.patch.object(compare.adapter, "verify", return_value=fx.baseline), \
             mock.patch.object(compare.adapter, "verify_current", return_value=fx.candidate):
            identity = compare.prepare(fx.archive, root / "seed/builds/baseline",
                                       root / "seed/builds/candidate", fx.repo, fx.harness,
                                       rows, 2 if smoke else 30, SEEDS, smoke, False)
        collection = _inside(self.root, root / "collection-v2")
        collection.mkdir(parents=True)
        # Preserve this test source and all synthetic inputs before materializing rows.
        audit = _inside(self.root, root / "audit")
        audit.mkdir(parents=True)
        (audit / "test_source.py").write_bytes(Path(__file__).read_bytes())
        (audit / "arguments.json").write_text(json.dumps({
            "rows": rows, "pairs_per_row": identity["pairs_per_row"], "seeds": SEEDS,
            "smoke": smoke, "output": str(collection),
        }, sort_keys=True, indent=2) + "\n")

        specs = compare.pair_specs(identity)
        runs = []
        for run_id, seed in enumerate(SEEDS):
            run_pairs = []
            for spec in (item for item in specs if item["run_id"] == run_id):
                row = identity["rows"][int(spec["row_id"])]
                observations = {}
                for side in compare.SIDES:
                    settings = compare.side_row(row, side)
                    total = 1000 if side == "baseline" else 500
                    phases = {"prepare_ns": total // 5,
                              "instantiate_ns": total // 5,
                              "invoke_ns": total - 2 * (total // 5)}
                    payload = {
                        "schema": "wtime-entry-batch-v1", "schema_version": 1,
                        "driver": "entry_batch", "case": row["case"],
                        "input": row["input"], "expected": row["expected"],
                        "result": row["expected"], "mode": "register",
                        "strategy": settings["strategy"],
                        "threshold": None if settings["strategy"] == "eager" else settings["threshold"],
                        "requested_calls": row["calls"], "actual_calls": row["calls"],
                        "instrumented": False, "source_fuel": None, "feature_profile": "mvp",
                        "fixture_calls": row["calls"], "total_ns": total,
                        "prepare_ns": phases["prepare_ns"],
                        "instantiate_ns": phases["instantiate_ns"], "invoke_ns": phases["invoke_ns"],
                    }
                    stdout = json.dumps(payload, sort_keys=True, separators=(",", ":")) + "\n"
                    bound = identity["sides"][side]
                    argv = compare.command(bound["executable"]["path"], row, side)
                    raw = {"argv": argv, "returncode": 0, "stdout": stdout, "stderr": ""}
                    rel = Path("raw") / (spec["pair_id"].replace(":", "-") + f"-{side}.json")
                    raw_path = _inside(self.root, collection / rel)
                    raw_path.parent.mkdir(parents=True, exist_ok=True)
                    raw_path.write_text(json.dumps(raw, sort_keys=True, indent=2) + "\n")
                    observations[side] = {
                        **raw, "side": side, "source_binding": compare.digest(bound),
                        "fixture_sha256": row["fixture_sha256"], "payload": payload,
                        "raw_record": rel.as_posix(),
                    }
                run_pairs.append({**spec, "observations": observations})
            runs.append({"run_id": run_id, "seed": seed, "pairs": run_pairs})
        report = {
            "schema": identity["schema"], "passed": True, "source_stable": True,
            "identity": identity, "identity_sha256": compare.digest(identity),
            "metric": "entry_total", "timing_evidence": not smoke,
            "same_source_alias": False, "runs": runs, "pair_count": len(specs),
        }
        (collection / "report.json").write_text(json.dumps(report, sort_keys=True, indent=2) + "\n")
        header = {"type": "header", "schema": checkpoint.SCHEMA,
                  "identity": identity, "identity_sha256": compare.digest(identity),
                  "run_seeds": {str(i): seed for i, seed in enumerate(SEEDS)}}
        events = [header]
        events.extend({"type": "pair", "run_id": run["run_id"], "seed": run["seed"], "pair": pair}
                      for run in runs for pair in run["pairs"])
        (collection / "checkpoint.jsonl").write_text("".join(
            json.dumps(event, sort_keys=True, separators=(",", ":")) + "\n" for event in events))
        for name, ref in identity["helpers"].items():
            source = Path(ref["path"]).resolve(strict=True)
            target = _inside(self.root, collection / "helpers" / name)
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(source.read_bytes())
        return fx, rows, identity, collection, report

    def test_p139_v1_and_v2_rows_normalize_without_mixing_schemas(self):
        fx = self._collector()
        legacy = fx.rows_for((1,))[0]
        self.assertEqual(compare.row_schema([legacy]), compare.SCHEMA)
        normalized_v1 = compare.normalize_rows([legacy])
        self.assertEqual(compare.side_row(normalized_v1[0], "baseline")["strategy"], legacy["strategy"])
        v2 = _v2row(fx.fixture, 1)
        normalized_v2 = compare.normalize_rows([v2])
        self.assertEqual(compare.row_schema(normalized_v2), compare.SCHEMA_SIDE)
        self.assertEqual(compare.side_row(normalized_v2[0], "baseline")["strategy"], "eager")
        self.assertEqual(compare.side_row(normalized_v2[0], "candidate")["threshold"], 3)
        with self.assertRaisesRegex(ValueError, "mixed"):
            compare.normalize_rows([legacy, v2])
        with self.assertRaisesRegex(ValueError, "explicit entry side"):
            compare.side_row(normalized_v2[0])

    def test_p139_collector_uses_distinct_eager_and_adaptive_side_argv(self):
        fx = self._collector()
        rows = [_v2row(fx.fixture, 10)]
        report = fx.invoke(rows=rows, pairs=2, smoke=True)
        self.assertEqual(report["schema"], compare.SCHEMA_SIDE)
        self.assertTrue(report["passed"])
        launched = [json.loads(line) for line in fx.argv_log.read_text().splitlines()]
        self.assertEqual(len(launched), 2 * 2 * 2)
        self.assertEqual({(args[7], int(args[8])) for args in launched}, {("eager", 0), ("adaptive", 3)})
        for run in report["runs"]:
            for pair in run["pairs"]:
                baseline = pair["observations"]["baseline"]
                candidate = pair["observations"]["candidate"]
                self.assertEqual(baseline["argv"][-2:], ["eager", "0"])
                self.assertEqual(candidate["argv"][-2:], ["adaptive", "3"])
                self.assertEqual(baseline["payload"]["strategy"], "eager")
                self.assertIsNone(baseline["payload"]["threshold"])
                self.assertEqual(candidate["payload"]["strategy"], "adaptive")
                self.assertEqual(candidate["payload"]["threshold"], 3)
        fx.assert_contained_outputs()

    def test_p139_report_rejects_swapped_side_argv_payload_and_binding(self):
        fx = self._collector()
        report = fx.invoke(rows=[_v2row(fx.fixture, 1)], pairs=2, smoke=True)
        pair = report["runs"][0]["pairs"][0]
        bad = copy.deepcopy(report)
        obs = bad["runs"][0]["pairs"][0]["observations"]["baseline"]
        obs["argv"] = pair["observations"]["candidate"]["argv"]
        with self.assertRaisesRegex(ValueError, "argv/settings"):
            compare.validate_report(bad, fx.output)
        bad = copy.deepcopy(report)
        obs = bad["runs"][0]["pairs"][0]["observations"]["baseline"]
        obs["payload"]["strategy"] = "adaptive"
        obs["payload"]["threshold"] = 3
        obs["stdout"] = json.dumps(obs["payload"], separators=(",", ":")) + "\n"
        with self.assertRaisesRegex(ValueError, "entry strategy differs"):
            compare.validate_report(bad, fx.output)
        bad = copy.deepcopy(report)
        bad["runs"][0]["pairs"][0]["observations"]["candidate"]["source_binding"] = "0" * 64
        with self.assertRaisesRegex(ValueError, "source/fixture"):
            compare.validate_report(bad, fx.output)

    def test_p139_rejects_bool_unknown_and_mixed_settings(self):
        fx = self._collector()
        valid = _v2row(fx.fixture, 10)
        cases = []
        invalid = dict(valid); invalid["baseline_threshold"] = True; cases.append((invalid, "invalid integer"))
        invalid = dict(valid); invalid["candidate_threshold"] = False; cases.append((invalid, "invalid integer"))
        invalid = dict(valid); invalid["extra"] = 1; cases.append((invalid, "row fields"))
        invalid = dict(valid); invalid["candidate_strategy"] = "unknown"; cases.append((invalid, "invalid strategy"))
        for row, message in cases:
            with self.subTest(row=row), self.assertRaisesRegex(ValueError, message):
                compare.normalize_rows([row])
        with self.assertRaisesRegex(ValueError, "mixed"):
            compare.normalize_rows([fx.rows_for((10,))[0], valid])

    def test_p139_checkpoint_resume_rejects_changed_side_identity_and_pair_corruption(self):
        fx = self._collector(fail_once=True)
        rows = [_v2row(fx.fixture, 1)]
        with self.assertRaisesRegex(ValueError, "entry command failed"):
            fx.invoke(rows=rows, pairs=2, smoke=True)
        changed = [dict(rows[0], candidate_threshold=5)]
        with self.assertRaisesRegex(ValueError, "settings identity mismatch"):
            fx.invoke(rows=changed, pairs=2, smoke=True, resume=True)
        report = fx.invoke(rows=rows, pairs=2, smoke=True, resume=True)
        self.assertEqual(report["pair_count"], 4)
        with self.assertRaisesRegex(ValueError, "missing/duplicate pairs"):
            bad = copy.deepcopy(report)
            bad["runs"][0]["pairs"].pop()
            bad["pair_count"] -= 1
            compare.validate_report(bad, fx.output)
        with self.assertRaisesRegex(ValueError, "duplicate/foreign pair IDs"):
            bad = copy.deepcopy(report)
            bad["runs"][0]["pairs"][1] = copy.deepcopy(bad["runs"][0]["pairs"][0])
            compare.validate_report(bad, fx.output)
        fx.assert_contained_outputs()

    def test_p139_v1_report_remains_analyzable(self):
        fx = analysis_tests.AnalysisFixture(_inside(self.root, self.root / "v1"), smoke=True)
        result = analyzer.analyze(fx.report, fx.collection, allow_smoke=True)
        self.assertEqual(result["schema"], analyzer.SCHEMA)
        self.assertFalse(result["timing_evidence"])
        self.assertFalse(result["gate_eligible"])
        self.assertTrue(all({"fixture", "case", "input", "expected", "calls", "strategy", "threshold"}
                            <= row["settings"].keys()
                            and "baseline_strategy" not in row["settings"]
                            and "candidate_strategy" not in row["settings"]
                            for run in result["runs"] for row in run["rows"]))

    def test_p139_v2_two_run_thirty_pair_analysis_keeps_rows_and_bootstraps_10000(self):
        _, rows, identity, collection, report = self._v2_collection(
            self.root / "candidate-analysis", smoke=False)
        self.assertEqual(identity["schema"], compare.SCHEMA_SIDE)
        self.assertEqual(len(rows), len(COUNTS))
        compare.validate_report(report, collection)
        with mock.patch.object(compare.subprocess, "run", side_effect=AssertionError("guest relaunched")):
            result = analyzer.analyze(report, collection)
        self.assertEqual(result["schema"], analyzer.SCHEMA_SIDE)
        self.assertTrue(result["timing_evidence"])
        self.assertTrue(result["gate_eligible"])
        self.assertFalse(result["scaled_by_calls"])
        self.assertEqual(len(result["runs"]), 2)
        for run in result["runs"]:
            self.assertEqual(len(run["rows"]), 4)
            self.assertEqual({row["settings"]["calls"] for row in run["rows"]}, set(COUNTS))
            for analyzed in run["rows"]:
                row = analyzed["settings"]
                self.assertIn(row["baseline_strategy"], {"eager", "adaptive"})
                self.assertIn(row["candidate_strategy"], {"eager", "adaptive"})
                self.assertEqual(analyzed["pairs"], 30)
                self.assertEqual(analyzed["baseline"]["total_ns"]["median"], 1000)
                self.assertEqual(analyzed["candidate"]["total_ns"]["median"], 500)
                self.assertEqual(analyzed["candidate_over_baseline"], 0.5)
                self.assertEqual(analyzed["ci95"], [0.5, 0.5])
                self.assertEqual(analyzed["bootstrap_resamples"], 10_000)

    def test_p139_smoke_keeps_raw_totals_and_null_uncertainty(self):
        _, _, _, collection, report = self._v2_collection(
            self.root / "smoke-analysis", smoke=True, counts=(10,))
        result = analyzer.analyze(report, collection, allow_smoke=True)
        self.assertEqual(result["schema"], analyzer.SCHEMA_SIDE)
        self.assertFalse(result["timing_evidence"])
        self.assertFalse(result["gate_eligible"])
        for run in result["runs"]:
            row = run["rows"][0]
            self.assertIsNone(row["candidate_over_baseline"])
            self.assertIsNone(row["ci95"])
            self.assertEqual(row["bootstrap_resamples"], 0)
            self.assertEqual(row["baseline"]["total_ns"]["median"], 1000)
            self.assertEqual(row["candidate"]["total_ns"]["median"], 500)
            self.assertEqual(row["baseline"]["total_ns"]["count"], 2)


if __name__ == "__main__":
    unittest.main()
