"""Pure in-memory tests for the P144 activation-capacity metric contract."""
from __future__ import annotations

import unittest

from tools.verification.producers import p144


def valid_metrics() -> list[dict]:
    peak = 24
    return [
        {"case": "capacity_oracle", "before": 16, "after": peak,
         "capacity_not_length": True},
        {"case": "nested_oracle", "first": 16, "second": 24, "sum": 40,
         "reset_preserved_active": True},
        {"case": "pinned_eviction", "working_bytes": 24, "code_bytes": 256,
         "evictions": 3, "cleanup": True},
        {"case": "entry_peaks", "peaks": [peak, peak, peak, peak, 0, 0, peak, peak],
         "per_invoke_reset": True},
        {"case": "bounded_churn", "working_peak": peak, "code_bytes": 256,
         "evictions": 4, "cancel_peak": 0},
        {"case": "reentry", "cancel": False, "peak_frames": 1,
         "outer_released_before_callback": True, "cleanup": True},
        {"case": "reentry", "cancel": True, "peak_frames": 1,
         "outer_released_before_callback": True, "cleanup": True},
    ]


class P144MetricTests(unittest.TestCase):
    def test_accepts_complete_seven_record_capacity_profile(self):
        records = p144.validate_metrics(valid_metrics())
        self.assertEqual(set(records), {
            "capacity_oracle", "nested_oracle", "pinned_eviction", "entry_peaks",
            "bounded_churn", "reentry",
        })
        self.assertEqual(len(records["reentry"]), 2)

    def test_rejects_missing_duplicate_and_unknown_records(self):
        for change in ("missing", "duplicate", "unknown"):
            with self.subTest(change=change):
                rows = valid_metrics()
                if change == "missing":
                    rows.pop(0)
                elif change == "duplicate":
                    rows.append(dict(rows[0]))
                else:
                    rows[0]["case"] = "unrecognized"
                with self.assertRaises(p144.VerificationError):
                    p144.validate_metrics(rows)

    def test_rejects_capacity_length_substitution_and_bad_nested_sum_or_reset(self):
        rows = valid_metrics()
        rows[0]["capacity_not_length"] = False
        with self.assertRaises(p144.VerificationError):
            p144.validate_metrics(rows)

        rows = valid_metrics()
        rows[0]["after"] = rows[0]["before"]
        with self.assertRaises(p144.VerificationError):
            p144.validate_metrics(rows)

        for field, value in (("sum", 39), ("reset_preserved_active", False)):
            with self.subTest(field=field):
                rows = valid_metrics()
                rows[1][field] = value
                with self.assertRaises(p144.VerificationError):
                    p144.validate_metrics(rows)

    def test_keeps_working_capacity_separate_from_code_and_cancellation(self):
        for field, value in (("working_bytes", 256), ("code_bytes", 24),
                             ("evictions", 1), ("cleanup", False)):
            with self.subTest(case="pinned_eviction", field=field):
                rows = valid_metrics()
                rows[2][field] = value
                with self.assertRaises(p144.VerificationError):
                    p144.validate_metrics(rows)

        for field, value in (("working_peak", 23), ("code_bytes", 24),
                             ("evictions", 2), ("cancel_peak", 1)):
            with self.subTest(case="bounded_churn", field=field):
                rows = valid_metrics()
                rows[4][field] = value
                with self.assertRaises(p144.VerificationError):
                    p144.validate_metrics(rows)

    def test_requires_reentry_release_cleanup_frame_count_and_boolean_outcomes(self):
        mutations = (
            ("peak_frames", 2),
            ("outer_released_before_callback", False),
            ("cleanup", False),
            ("cancel", 1),
        )
        for field, value in mutations:
            with self.subTest(field=field):
                rows = valid_metrics()
                rows[5][field] = value
                with self.assertRaises(p144.VerificationError):
                    p144.validate_metrics(rows)

    def test_rejects_boolean_and_out_of_range_u64_capacity_values(self):
        for value in (True, -1, 1 << 64):
            with self.subTest(value=value):
                rows = valid_metrics()
                rows[0]["after"] = value
                with self.assertRaises(p144.VerificationError):
                    p144.validate_metrics(rows)

        rows = valid_metrics()
        rows[3]["peaks"][0] = False
        with self.assertRaises(p144.VerificationError):
            p144.validate_metrics(rows)


if __name__ == "__main__":
    unittest.main()
