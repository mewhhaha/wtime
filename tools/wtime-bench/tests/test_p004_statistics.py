"""Focused integrity and statistical tests for P004."""
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import analyze


def make_report(ratios=(0.90,), stages=("prepare", "instantiate", "warm_call"), count=30):
    runs = []
    for run_number, seed in enumerate((1234, 5678)):
        pairs = []
        for stage in stages:
            for index in range(count):
                ratio = ratios[index % len(ratios)]
                baseline = 1000.0 if stage == "warm_call" else (100.0 if stage == "prepare" else 20.0)
                pairs.append({
                    "pair_id": f"{run_number}-{stage}-{index}",
                    "workload": "integer_loop", "stage": stage, "input": 128,
                    "wasm_sha256": "a" * 64, "expected": 42,
                    "order": ["baseline", "candidate"] if index % 2 else ["candidate", "baseline"],
                    "baseline": {"ns_per_operation": baseline, "batch": 10, "result": 42, "mode": "decoded"},
                    "candidate": {"ns_per_operation": baseline * ratio, "batch": 12, "result": 42, "mode": "native"},
                })
        runs.append({"seed": seed, "pairs": pairs})
    return {"schema_version": 2, "passed": True, "source_stable": True,
            "metadata": {"settings": {"instrumented": False, "features": "mvp", "fuel": False,
                                         "cancellation_quantum": 1, "baseline_mode": "decoded",
                                         "candidate_mode": "native"}}, "runs": runs}


class P004StatisticsTests(unittest.TestCase):
    def test_known_paired_ratio_is_deterministic_and_break_even_is_reported(self):
        report = make_report()
        first = analyze.analyze(report)
        second = analyze.analyze(report)
        warm = [row for row in first["per_run"] if row["stage"] == "warm_call"]
        self.assertEqual(len(warm), 2)
        for row in warm:
            self.assertAlmostEqual(row["candidate_over_baseline"]["median"], 0.9)
            self.assertEqual(row["decision"], "supported_improvement")
            self.assertEqual(row["median_ratio_95ci"]["lower"], 0.9)
            self.assertEqual(row["median_ratio_95ci"]["upper"], 0.9)
        self.assertEqual(first, second)
        self.assertEqual(first["analysis"]["bootstrap_resamples"], 10_000)
        self.assertEqual(len(first["break_even"]), 2)
        self.assertEqual(first["break_even"][0]["break_even_calls"], 0)
        self.assertEqual(set(first["break_even"][0]["totals_ns"]), {"1", "10", "100", "10000"})

    def test_noisy_warm_control_is_inconclusive(self):
        ratios = [0.80, 1.20] * 15
        result = analyze.analyze(make_report(ratios=ratios, stages=("warm_call",)))
        for row in result["per_run"]:
            self.assertEqual(row["decision"], "inconclusive")
            self.assertGreater(row["median_ratio_95ci"]["upper"], 1.0)
        self.assertTrue(all(row["break_even_calls"] is None for row in result["break_even"]))

    def test_incomplete_groups_and_mixed_settings_are_rejected(self):
        incomplete = make_report()
        incomplete["runs"][1]["pairs"].pop()
        with self.assertRaisesRegex(analyze.ReportError, "at least 30"):
            analyze.analyze(incomplete)

        mixed = make_report()
        mixed["runs"][0]["pairs"][0]["baseline"]["settings"] = {"mode": "decoded", "features": "mvp", "fuel": False, "quantum": 1}
        mixed["runs"][0]["pairs"][0]["candidate"]["settings"] = {"mode": "native", "features": "mvp", "fuel": False, "quantum": 2}
        with self.assertRaisesRegex(analyze.ReportError, "mixes feature, fuel, or quantum"):
            analyze.analyze(mixed)

    def test_nonpositive_and_nonfinite_measurements_are_rejected(self):
        for value in (0, -1, float("nan"), float("inf")):
            report = make_report()
            report["runs"][0]["pairs"][0]["candidate"]["ns_per_operation"] = value
            with self.subTest(value=value), self.assertRaises(analyze.ReportError):
                analyze.analyze(report)

    def test_duplicate_pair_ids_are_rejected_within_run(self):
        report = make_report()
        report["runs"][0]["pairs"][1]["pair_id"] = report["runs"][0]["pairs"][0]["pair_id"]
        with self.assertRaisesRegex(analyze.ReportError, "duplicate pair_id"):
            analyze.analyze(report)

    def test_partial_report_and_missing_result_are_rejected(self):
        report = make_report()
        report["passed"] = False
        with self.assertRaisesRegex(analyze.ReportError, "passed=true"):
            analyze.analyze(report)

        report = make_report()
        del report["runs"][0]["pairs"][0]["baseline"]["result"]
        with self.assertRaisesRegex(analyze.ReportError, "missing result"):
            analyze.analyze(report)

    def test_missing_cold_stages_do_not_claim_a_break_even(self):
        result = analyze.analyze(make_report(stages=("warm_call",)))
        self.assertEqual(len(result["break_even"]), 2)
        for row in result["break_even"]:
            self.assertIsNone(row["break_even_calls"])
            self.assertIn("measurements are required", row["break_even"])
            self.assertIsNone(row["totals_ns"]["1"]["baseline"])


if __name__ == "__main__":
    unittest.main()
