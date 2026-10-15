from __future__ import annotations

import json
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "tools/wtime-bench"))
sys.path.insert(0, str(ROOT / "tools/verification/producers"))
import p066_gate
import entry_sequence_policy


def _summary(count: int, value: float = 100.0) -> dict:
    return {"median": value, "mad": 0.0, "p25": value, "p75": value,
            "min": value, "max": value, "p99": value, "count": count, "unit": "ns"}


def _settings(calls: int) -> dict:
    return {"fixture": f"/fixture/{calls}.wasm", "case": "synthetic", "input": 1,
            "expected": 2, "calls": calls, "baseline_strategy": "eager",
            "baseline_threshold": 0, "candidate_strategy": "adaptive",
            "candidate_threshold": 3, "fixture_sha256": f"{calls:064x}",
            "row_id": str(p066_gate.CALLS.index(calls)), "metric": "entry_total",
            "mode": "register", "feature_profile": "mvp", "source_fuel": None}


def _totals(interval=(0.9, 1.0)) -> dict:
    runs = []
    for run_id, seed in enumerate(p066_gate.SEEDS):
        rows = []
        for calls in p066_gate.CALLS:
            rows.append({"row_id": str(p066_gate.CALLS.index(calls)), "settings": _settings(calls),
                         "pairs": 30, "raw_unit": "ns", "metric": "entry_total",
                         "bootstrap_seed": 17 + calls, "candidate_over_baseline": 0.95,
                         "ci95": list(interval), "bootstrap_resamples": 10_000,
                         "baseline": {name: _summary(30) for name in
                                      ("total_ns", "prepare_ns", "instantiate_ns", "invoke_ns")},
                         "candidate": {name: _summary(30) for name in
                                       ("total_ns", "prepare_ns", "instantiate_ns", "invoke_ns")}})
        runs.append({"run_id": run_id, "seed": seed, "rows": rows})
    return {"schema": "wtime-register-entry-analysis-v2", "status": "passed",
            "metric": "entry_total", "raw_unit": "ns", "method": p066_gate.TOTAL_METHOD,
            "report_sha256": "a" * 64, "collection_identity_sha256": "b" * 64,
            "analysis_source_refs": {}, "input_refs": {}, "timing_evidence": True,
            "gate_eligible": True, "decision": None, "threshold_selection": None,
            "scaled_by_calls": False, "runs": runs, "limitations": []}


def _sequences(interval=(0.9, 1.0)) -> dict:
    counts = {}
    for position in entry_sequence_policy.schedule(3):
        event = entry_sequence_policy.event_class(position, 3)
        counts[event] = counts.get(event, 0) + 1
    classes = []
    for event in p066_gate.EVENTS:
        count = counts[event]
        metric_results = {
            "p50": {"candidate_over_baseline": 0.95, "ci95": list(interval),
                    "bootstrap_resamples": 10_000, "bootstrap_seed": 123, "alias_of": None},
            "p99": {"candidate_over_baseline": 0.95, "ci95": list(interval),
                    "bootstrap_resamples": 10_000, "bootstrap_seed": 124, "alias_of": None},
            "max": {"candidate_over_baseline": 0.95, "ci95": list(interval),
                    "bootstrap_resamples": 10_000, "bootstrap_seed": 125, "alias_of": None},
        }
        if count == 1:
            for metric in ("p99", "max"):
                metric_results[metric] = {**metric_results["p50"], "bootstrap_resamples": 0,
                                          "alias_of": "p50"}
        sides = {}
        for side in ("baseline", "candidate"):
            sides[side] = {"process_count": 30, "raw_call_count": count * 30,
                           "calls_per_process": count,
                           "process_metrics": {m: _summary(30) for m in p066_gate.METRICS}}
        if event == "cold_initial":
            for metric, baseline_value, candidate_value in (
                    ("p50", 100.0, 95.0), ("p99", 200.0, 190.0), ("max", 300.0, 285.0)):
                sides["baseline"]["process_metrics"][metric] = _summary(30, baseline_value)
                sides["candidate"]["process_metrics"][metric] = _summary(30, candidate_value)
            for metric in ("p99", "max"):
                metric_results[metric] = {**metric_results["p50"], "bootstrap_resamples": 0,
                                          "alias_of": "p50"}
        classes.append({"event_class": event, "raw_call_count_per_process": count,
                        "process_pairs": 30, "baseline": sides["baseline"],
                        "candidate": sides["candidate"], "metrics": metric_results})
    runs = [{"run_id": i, "seed": seed,
             "rows": [{"row_id": "0", "settings": {
                 "fixture": "/fixture/uniform66.wasm", "fixture_sha256": "c" * 64,
                 "input": 2147483647, "schedule_threshold": 3, "row_id": "0",
                 "case": "uniform66", "metric": "individual_entry_latency", "mode": "register",
                 "feature_profile": "mvp", "source_fuel": None,
             }, "process_pairs": 30, "classes": classes}]}
            for i, seed in enumerate(p066_gate.SEEDS)]
    return {"schema": p066_gate.SEQUENCE_SCHEMA, "status": "passed",
            "metric": "per_process_event_latency", "raw_unit": "ns",
            "method": p066_gate.SEQUENCE_METHOD, "report_sha256": "d" * 64,
            "collection_identity_sha256": "e" * 64, "analysis_source_refs": {},
            "input_refs": {}, "timing_evidence": True, "gate_eligible": True,
            "decision": None, "threshold_selection": None, "scaled_by_calls": False,
            "runs": runs, "limitations": []}


def _focused(decision="adopted") -> dict:
    reasons = [] if decision == "adopted" else [f"focused {decision}"]
    inner = {"task": "P066", "status": "passed", "decision": decision, "reasons": reasons}
    return {"schema": "wtime-verification-statistics-v1", "task": "P066", "status": "passed",
            "statistics_computations": 1, "decision": inner}


class P066GateTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="p066-gate-")
        self.root = Path(self.tmp.name).resolve()
        helpers = self._inside(self.root / "helpers")
        helpers.mkdir(parents=True)
        for label, source in (("gate.py", Path(p066_gate.__file__)), ("test.py", Path(__file__))):
            target = self._inside(helpers / label)
            target.write_bytes(source.read_bytes())
        invocation = self._inside(helpers / "argv.json")
        invocation.write_text(json.dumps({"argv": sys.argv, "cwd": str(Path.cwd())}) + "\n")

    def tearDown(self):
        self.tmp.cleanup()

    def _inside(self, path: Path) -> Path:
        resolved = path.resolve()
        if not resolved.is_relative_to(self.root):
            raise AssertionError(f"test output escapes fresh temporary root: {resolved}")
        return resolved

    def _combine(self, focused=None, totals=None, sequences=None):
        return p066_gate.combine(focused or _focused(), totals or _totals(),
                                 sequences or _sequences())

    def test_full_adopted_gate_keeps_independent_rows_and_runs(self):
        result = self._combine()
        self.assertEqual(result["decision"], "adopted")
        self.assertEqual(len(result["supplementary"]["total_rows"]), 8)
        self.assertEqual(len(result["supplementary"]["sequence_rows"]), 24)
        self.assertFalse(result["statistics_recomputed"])

    def test_rejected_sequence_spike_dominates_focused_adoption(self):
        sequence = _sequences()
        sequence["runs"][1]["rows"][0]["classes"][2]["metrics"]["max"]["ci95"] = [1.06, 1.12]
        result = self._combine(sequences=sequence)
        self.assertEqual(result["decision"], "rejected")
        self.assertTrue(any("warm_initial/max" in reason
                            for reason in result["supplementary"]["rejections"]))

    def test_bound_crossing_is_inconclusive_and_focused_rejection_wins(self):
        totals = _totals()
        totals["runs"][0]["rows"][0]["ci95"] = [1.00, 1.06]
        result = self._combine(totals=totals)
        self.assertEqual(result["decision"], "inconclusive")
        result = self._combine(focused=_focused("rejected"))
        self.assertEqual(result["decision"], "rejected")

    def test_missing_rows_smoke_and_malformed_aliases_are_rejected(self):
        totals = _totals()
        totals["runs"][0]["rows"].pop()
        with self.assertRaises(ValueError):
            self._combine(totals=totals)
        smoke = _sequences()
        smoke["timing_evidence"] = False
        with self.assertRaises(ValueError):
            self._combine(sequences=smoke)
        malformed = _sequences()
        metric = malformed["runs"][0]["rows"][0]["classes"][2]["metrics"]["max"]
        metric["alias_of"] = "p99"
        metric["bootstrap_resamples"] = 0
        with self.assertRaises(ValueError):
            self._combine(sequences=malformed)
        bad_threshold = _totals()
        bad_threshold["runs"][0]["rows"][0]["settings"]["baseline_threshold"] = False
        with self.assertRaises(ValueError):
            self._combine(totals=bad_threshold)
        bad_count = _totals()
        bad_count["runs"][0]["rows"][0]["pairs"] = True
        with self.assertRaises(ValueError):
            self._combine(totals=bad_count)
        bad_resamples = _sequences()
        bad_resamples["runs"][0]["rows"][0]["classes"][4]["metrics"]["p99"]["bootstrap_resamples"] = False
        with self.assertRaises(ValueError):
            self._combine(sequences=bad_resamples)

    def test_single_call_metric_aliases_share_uncertainty(self):
        sequences = _sequences()
        item = sequences["runs"][0]["rows"][0]["classes"][4]
        item["metrics"]["p99"]["candidate_over_baseline"] = 2.0
        with self.assertRaises(ValueError):
            self._combine(sequences=sequences)

    def test_proportional_absolute_summaries_can_share_log_ratio_alias(self):
        sequences = _sequences()
        item = sequences["runs"][0]["rows"][0]["classes"][0]
        self.assertNotEqual(item["baseline"]["process_metrics"]["p50"],
                            item["baseline"]["process_metrics"]["p99"])
        self.assertEqual(item["metrics"]["p50"]["candidate_over_baseline"],
                         item["metrics"]["p99"]["candidate_over_baseline"])
        self.assertEqual(self._combine(sequences=sequences)["decision"], "adopted")
        item["metrics"]["p99"]["ci95"] = [0.91, 1.01]
        with self.assertRaises(ValueError):
            self._combine(sequences=sequences)


if __name__ == "__main__":
    unittest.main()
