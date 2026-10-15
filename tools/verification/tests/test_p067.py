from __future__ import annotations

import copy
import json
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "tools"))
sys.path.insert(0, str(ROOT / "tools/verification/producers"))
import p067


class P067CoverageAndDecisionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.eager_fixture = json.loads(
            (ROOT / "docs/benchmarks/performance/P067-eager-selection.json").read_text(
                encoding="utf-8"
            )
        )
        cls.adaptive_fixture = json.loads(
            (ROOT / "docs/benchmarks/performance/P067-adaptive-selection.json").read_text(
                encoding="utf-8"
            )
        )

    def selections(self):
        return copy.deepcopy(self.eager_fixture), copy.deepcopy(self.adaptive_fixture)

    def test_valid_selections_cover_expected_complete_portfolios(self):
        eager, adaptive = self.selections()
        self.assertEqual(
            p067.validate_coverage(eager, adaptive),
            {
                "scalar": 85,
                "memory": 53,
                "strict_float": 26,
                "representative": 9,
                "unique_eager": 174,
                "unique_adaptive": 95,
            },
        )

    def test_omitted_scalar_memory_strict_float_or_held_out_rows_fail(self):
        eager, adaptive = self.selections()
        for workload_id in (
            "integer_loop",
            "memory_fill_scan",
            "simd_float_arithmetic_n0",
            "representative_parser_hash_held_out",
        ):
            with self.subTest(workload_id=workload_id):
                changed_eager = copy.deepcopy(eager)
                changed_eager["rows"] = [
                    row for row in changed_eager["rows"] if row["id"] != workload_id
                ]
                with self.assertRaises((ValueError, RuntimeError)):
                    p067.validate_coverage(changed_eager, copy.deepcopy(adaptive))

    def test_protected_control_stage_and_role_are_enforced(self):
        eager, adaptive = self.selections()
        control = next(row for row in eager["rows"] if row["id"] == "call_overhead")
        control["stages"] = ["load_and_call"]
        with self.assertRaises((ValueError, RuntimeError)):
            p067.validate_coverage(eager, adaptive)

        eager, adaptive = self.selections()
        control = next(row for row in eager["rows"] if row["id"] == "prepare_large")
        control["role"] = "targeted"
        with self.assertRaises((ValueError, RuntimeError)):
            p067.validate_coverage(eager, adaptive)

    def test_adaptive_entry_threshold_must_remain_three(self):
        eager, adaptive = self.selections()
        adaptive["execution_settings"]["register_entry_threshold"]["candidate"] = 4
        with self.assertRaises((ValueError, RuntimeError)):
            p067.validate_coverage(eager, adaptive)

    def test_decision_requires_all_adoptions_and_rejection_dominates(self):
        adopted = p067.choose({"decoded": "adopted", "compact": "adopted"}, hard_gates=True)
        self.assertEqual(adopted["decision"], "adopted")
        self.assertEqual(adopted["recommendation"], "eligible_for_P108")
        self.assertEqual(adopted["default_execution"], "decoded")

        rejected = p067.choose(
            {"decoded": "adopted", "compact": "rejected"}, hard_gates=True
        )
        self.assertEqual(rejected["decision"], "rejected")
        self.assertEqual(rejected["recommendation"], "keep_register_opt_in")

        inconclusive = p067.choose(
            {"decoded": "adopted", "compact": "inconclusive"}, hard_gates=True
        )
        self.assertEqual(inconclusive["decision"], "inconclusive")

    def test_missing_unknown_and_boolean_decisions_are_rejected(self):
        for decisions in (
            {"decoded": "adopted"},
            {"decoded": "adopted", "compact": "unknown"},
            {"decoded": True, "compact": "adopted"},
        ):
            with self.subTest(decisions=decisions):
                with self.assertRaises((ValueError, RuntimeError)):
                    p067.choose(decisions, hard_gates=True)

    def test_hard_gate_must_be_explicitly_passed(self):
        decisions = {"decoded": "adopted", "compact": "adopted"}
        with self.assertRaises((ValueError, RuntimeError)):
            p067.choose(decisions, hard_gates=False)
        with self.assertRaises(TypeError):
            p067.choose(decisions)


if __name__ == "__main__":
    unittest.main()
