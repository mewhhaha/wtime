"""Focused statistics coverage for the measured thread steady-state unit."""
from __future__ import annotations

import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True
BENCH = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(BENCH))
import analyze  # noqa: E402


class ThreadStatisticsTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="wtime-thread-statistics-")
        self.root = Path(self.temporary.name).resolve()
        self.addCleanup(self.temporary.cleanup)

    def write_report(self, relative: str, report: dict) -> Path:
        path = (self.root / relative).resolve()
        self.assertTrue(path.is_relative_to(self.root), "synthetic report escaped fresh temp root")
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(report, sort_keys=True) + "\n", encoding="utf-8")
        return path

    @staticmethod
    def report():
        workload = "threads_ordinary_rw_n32"
        result = {"thread_oracle_validated": workload}
        digest = "a" * 64
        runs = []
        for run_index, seed in enumerate((6101, 6102)):
            pairs = []
            for pair_index in range(30):
                pairs.append({
                    "pair_id": f"{run_index}:{pair_index}",
                    "workload": workload,
                    "stage": "steady_state_group",
                    "input": 32,
                    "wasm_sha256": digest,
                    "expected": result,
                    "order": ["baseline", "candidate"],
                    "baseline": {"ns_per_operation": 1000.0, "batch": 8,
                                 "result": result, "mode": "decoded"},
                    "candidate": {"ns_per_operation": 1500.0, "batch": 8,
                                   "result": result, "mode": "decoded"},
                })
            runs.append({"seed": seed, "pairs": pairs})
        return {"schema_version": 2, "passed": True, "source_stable": True,
                "metadata": {"settings": {"instrumented": False,
                    "baseline_mode": "decoded", "candidate_mode": "decoded"}},
                "runs": runs}

    def test_two_full_runs_accept_exact_stage_and_preserve_known_ratio(self):
        path = self.write_report("thread-report.json", self.report())
        result = analyze.analyze(json.loads(path.read_text(encoding="utf-8")))
        self.assertEqual(len(result["per_run"]), 2)
        for run in result["per_run"]:
            self.assertEqual(run["stage"], "steady_state_group")
            self.assertEqual(run["pairs"], 30)
            self.assertAlmostEqual(run["candidate_over_baseline"]["median"], 1.5)

    def test_unknown_or_typo_stage_is_rejected(self):
        for stage in ("steady_state_groups", "load_and_call_typo"):
            report = self.report()
            report["runs"][0]["pairs"][0]["stage"] = stage
            with self.subTest(stage=stage), self.assertRaisesRegex(analyze.ReportError, "unknown stage"):
                analyze.analyze(report)

    def test_missing_second_run_is_rejected(self):
        report = self.report()
        report["runs"].pop()
        with self.assertRaisesRegex(analyze.ReportError, "exactly two runs"):
            analyze.analyze(report)


if __name__ == "__main__":
    unittest.main()
