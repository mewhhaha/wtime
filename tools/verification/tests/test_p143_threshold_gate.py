"""In-memory validation tests for the P143 explicit/adaptive tier gate."""
from __future__ import annotations

import unittest

from tools.verification.producers import p143


def tier_report(threshold: int | None) -> dict:
    """Return a complete small report with the required corpus cardinalities."""
    modes = ("register", "reference", "reference", "register")
    conformance = []
    for index, mode in enumerate(modes):
        register_threshold = threshold if mode == "register" else None
        conformance.append({
            "mode": mode,
            "suite": "reference_types" if index < 2 else "interactions",
            "corpus_sources": [f"corpus-{index}.json"],
            "report": {
                "register_entry_threshold": register_threshold,
                "register_strategy": (
                    "adaptive" if mode == "register" and threshold is not None
                    else "eager" if mode == "register"
                    else "disabled"
                ),
                "selected": 12,
                "executed": 12,
                "failed": 0,
                "skipped": 0,
                "unsupported": 0,
                "executed_opcodes": ["i32.add", "local.get"],
                "source_hash": f"{index + 1:064x}",
            },
        })
    counts = {}
    for name, value in (("mode_runs", 6), ("state_mode_runs", 6),
                        ("state_fuel_probes", 68), ("conformance_mode_runs", 4),
                        ("all_checks", 89)):
        counts[f"{name}_expected"] = value
        counts[f"{name}_executed"] = value
    counts.update(failed=0, skipped=0, unsupported=0)
    return {
        "passed": True,
        "source_stable": True,
        "requested_tier": "register",
        "baseline_tier": "reference",
        "register_entry_threshold": threshold,
        "register_strategy": "eager" if threshold is None else "adaptive",
        "counts": counts,
        "conformance": conformance,
    }


class P143ThresholdGateTests(unittest.TestCase):
    def test_accepts_complete_explicit_and_adaptive_reports(self):
        self.assertEqual(p143.validate_tier(tier_report(None), None)["state_fuel_probes_executed"], 68)
        self.assertEqual(p143.validate_tier(tier_report(3), 3)["conformance_mode_runs_executed"], 4)
        decoded = tier_report(None)
        decoded['requested_tier'] = 'decoded'
        decoded['register_strategy'] = 'disabled'
        for row in decoded['conformance']:
            if row['mode'] == 'register':
                row['mode'] = 'decoded'
            row['report']['register_strategy'] = 'disabled'
        self.assertEqual(p143.validate_tier(decoded, None, 'decoded')['all_checks_executed'], 89)

    def test_rejects_wrong_or_missing_top_level_threshold_and_strategy(self):
        mutations = (
            ("register_entry_threshold", 7),
            ("register_entry_threshold", 3),  # explicit strategy must not receive a threshold
            ("register_strategy", "adaptive"),
        )
        for field, value in mutations:
            with self.subTest(field=field, value=value):
                report = tier_report(None)
                report[field] = value
                with self.assertRaises(p143.VerificationError):
                    p143.validate_tier(report, None)

        report = tier_report(3)
        report.pop("register_entry_threshold")
        with self.assertRaises(p143.VerificationError):
            p143.validate_tier(report, 3)

    def test_reference_and_non_register_modes_never_receive_threshold(self):
        report = tier_report(3)
        report["conformance"][1]["report"]["register_entry_threshold"] = 3
        with self.assertRaises(p143.VerificationError):
            p143.validate_tier(report, 3)

        report = tier_report(3)
        report["conformance"][1]["report"]["register_strategy"] = "eager"
        with self.assertRaises(p143.VerificationError):
            p143.validate_tier(report, 3)

    def test_rejects_missing_state_fuel_corpus_and_aggregate_counts(self):
        for key in ("state_fuel_probes_expected", "state_fuel_probes_executed",
                    "conformance_mode_runs_expected", "all_checks_executed"):
            with self.subTest(key=key):
                report = tier_report(3)
                report["counts"].pop(key)
                with self.assertRaises(p143.VerificationError):
                    p143.validate_tier(report, 3)

        report = tier_report(3)
        report["counts"]["state_fuel_probes_executed"] = 67
        with self.assertRaises(p143.VerificationError):
            p143.validate_tier(report, 3)

        for key in ("corpus_sources", "report"):
            with self.subTest(key=key):
                report = tier_report(3)
                if key == "corpus_sources":
                    report["conformance"][0].pop(key)
                else:
                    report["conformance"].pop()
                with self.assertRaises(p143.VerificationError):
                    p143.validate_tier(report, 3)

        report = tier_report(3)
        report["conformance"][0]["report"].pop("executed_opcodes")
        with self.assertRaises(p143.VerificationError):
            p143.validate_tier(report, 3)

    def test_rejects_failures_skips_unsupported_and_unstable_reports(self):
        for field in ("failed", "skipped", "unsupported"):
            with self.subTest(field=field):
                report = tier_report(3)
                report["counts"][field] = 1
                with self.assertRaises(p143.VerificationError):
                    p143.validate_tier(report, 3)

        report = tier_report(3)
        report["conformance"][2]["report"]["unsupported"] = 1
        with self.assertRaises(p143.VerificationError):
            p143.validate_tier(report, 3)

        report = tier_report(3)
        report["source_stable"] = False
        with self.assertRaises(p143.VerificationError):
            p143.validate_tier(report, 3)


if __name__ == "__main__":
    unittest.main()
