"""Focused schema and retention checks for the P079 metrics validator."""

from __future__ import annotations

import copy
import sys
import unittest
from pathlib import Path


sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "tools"))
from verification.producers import p079  # noqa: E402


def snapshot(*, live, payload, fields, slots, free, slot_count):
    return {
        "live": live,
        "payload_charge_bytes": payload,
        "field_capacity_bytes": fields,
        "slot_capacity_bytes": slots,
        "free_capacity_bytes": free,
        "slot_count": slot_count,
    }


def valid_rows():
    zero = snapshot(live=0, payload=0, fields=0, slots=0, free=0, slot_count=0)
    peak = snapshot(live=1024, payload=65536, fields=32768, slots=8192, free=0, slot_count=1024)
    cleared = snapshot(live=0, payload=0, fields=0, slots=8192, free=8192, slot_count=1024)
    small = snapshot(live=4, payload=512, fields=256, slots=8192, free=8192, slot_count=1024)
    failure_before = snapshot(live=1, payload=64, fields=32, slots=64, free=0, slot_count=1)
    failure_final = snapshot(live=0, payload=0, fields=0, slots=64, free=64, slot_count=1)
    return [
        {
            "case": "gc_large_small",
            "before": zero,
            "large": peak,
            "cleared": cleared,
            "small": small,
            "final": copy.deepcopy(cleared),
            "metadata_lifetime": "arena",
            "root_hashmap_capacity": 0,
        },
        {
            "case": "gc_failure",
            "before": failure_before,
            "final": failure_final,
            "object_failure_atomic": True,
            "byte_failure_atomic": True,
        },
        {
            "case": "continuation_large_small",
            "capture_bytes": [2048, 512, 2048, 512],
            "physical_scan_matches": True,
            "final_continuation_bytes": 0,
            "final_gc_payload_bytes": 0,
            "final_states": 0,
            "registry_capacity": 16,
            "registry_lifetime": "store",
        },
    ]


class P079MetricsTests(unittest.TestCase):
    def test_accepts_realistic_three_case_retention_report(self):
        result = p079.validate_metrics(valid_rows())
        self.assertEqual(set(result), p079.CASES)
        self.assertEqual(result["gc_large_small"]["small"]["live"], 4)

    def test_rejects_missing_duplicate_and_unknown_cases(self):
        rows = valid_rows()
        with self.subTest("missing"):
            with self.assertRaisesRegex(ValueError, "exactly three"):
                p079.validate_metrics(rows[:2])
        with self.subTest("duplicate"):
            duplicate = copy.deepcopy(rows)
            duplicate[2]["case"] = "gc_failure"
            with self.assertRaisesRegex(ValueError, "missing, duplicate or unknown"):
                p079.validate_metrics(duplicate)
        with self.subTest("unknown"):
            unknown = copy.deepcopy(rows)
            unknown[2]["case"] = "gc_unlisted"
            with self.assertRaisesRegex(ValueError, "missing, duplicate or unknown"):
                p079.validate_metrics(unknown)

    def test_rejects_boolean_and_non_integer_capacity_values(self):
        boolean = valid_rows()
        boolean[0]["large"]["slot_count"] = True
        with self.assertRaisesRegex(ValueError, "nonnegative integer"):
            p079.validate_metrics(boolean)

        fractional = valid_rows()
        fractional[2]["capture_bytes"][1] = 512.0
        with self.assertRaisesRegex(ValueError, "nonnegative integer"):
            p079.validate_metrics(fractional)

    def test_rejects_residual_payload_after_collection(self):
        rows = valid_rows()
        rows[0]["cleared"]["payload_charge_bytes"] = 8
        with self.assertRaisesRegex(ValueError, "collection retained object payload"):
            p079.validate_metrics(rows)

    def test_rejects_slot_or_free_capacity_growth_during_small_reuse(self):
        slot_growth = valid_rows()
        slot_growth[0]["small"]["slot_capacity_bytes"] += 8
        with self.assertRaisesRegex(ValueError, "slot capacity changed"):
            p079.validate_metrics(slot_growth)

        free_growth = valid_rows()
        free_growth[0]["small"]["free_capacity_bytes"] += 8
        with self.assertRaisesRegex(ValueError, "free-list capacity grew"):
            p079.validate_metrics(free_growth)

    def test_rejects_continuation_physical_accounting_mismatch(self):
        rows = valid_rows()
        rows[2]["physical_scan_matches"] = False
        with self.assertRaisesRegex(ValueError, "differs from owned capacities"):
            p079.validate_metrics(rows)

    def test_rejects_repeated_continuation_size_drift(self):
        rows = valid_rows()
        rows[2]["capture_bytes"] = [2048, 512, 3072, 512]
        with self.assertRaisesRegex(ValueError, "large/small physical state changed"):
            p079.validate_metrics(rows)


if __name__ == "__main__":
    unittest.main()
