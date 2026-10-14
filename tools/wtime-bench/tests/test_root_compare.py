from __future__ import annotations

import copy
import math
from pathlib import Path
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(HERE))

import root_compare
import root_workloads


def sample(family: str, n: int, stage: str, batch: int = 1) -> tuple[dict, dict]:
    entry = next(row for row in root_workloads.entries()
                 if row["id"] == f"{family}_n{n}")
    cap = entry["max_batch"]
    calls = batch + 1 if stage == "warm_call" else 1
    callbacks = calls * n
    prep, inst, first = 11.0, 13.0, 17.0
    guest = 23.0 * batch if stage == "warm_call" else first
    measured = guest
    sample_ns = {"prepare": prep, "instantiate": inst, "first_call": first,
                 "warm_call": guest / batch,
                 "load_and_call": prep + inst + first}[stage]
    payload = 88 if callbacks else 0
    wire = {
        "schema_version": 2, "driver": "gc_root_stages", "case": entry["id"],
        "family": family, "input": n, "mode": "decoded", "profile": "all-phase3-core",
        "stage": stage, "batch": batch, "max_batch": cap, "instrumented": False,
        "measurement_policy": root_compare.MEASUREMENT_POLICY,
        "sample_ns": sample_ns, "prepare_ns": prep, "instantiate_ns": inst,
        "first_call_ns": first, "measured_guest_ns": measured, "reset_ns": 7.0,
        "calls_checked": calls, "total_reclaimed": calls, "reclaimed_per_call": 1,
        "live_after_reset": 0, "payload_bytes_after_reset": 0,
        "continuation_bytes_after_reset": 0, "callbacks_checked": callbacks,
        "nested_calls_checked": callbacks, "callback_live": 1 if callbacks else 0,
        "callback_reclaimed": 0, "callback_payload_min": payload,
        "callback_payload_max": payload, "shape": entry["gc_expectation"]["shape"],
        "bank_slots": entry["gc_expectation"]["bank_slots"],
        "result": f"i32:{entry['expected_result']}", "report_complete": True,
    }
    return entry, wire


class RootCompareTests(unittest.TestCase):
    def test_valid_cold_warm_and_zero_callback_proofs(self):
        # Keep synthetic work rooted in a fresh temporary directory; no absolute
        # provenance path is introduced or written by these protocol tests.
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp).resolve()
            self.assertTrue(root.is_dir())
            for family, n, stage, batch in (
                ("gc_root_snapshots_numeric_64", 17, "first_call", 1),
                ("gc_root_snapshots_reference_256", 1, "warm_call", 9),
                ("gc_root_snapshots_numeric_512", 0, "load_and_call", 1),
            ):
                entry, wire = sample(family, n, stage, batch)
                normalized = root_compare.validate_observation(
                    entry, wire, batch, expected_stage=stage)
                self.assertEqual(normalized["result"], entry["expected_result"])
                self.assertEqual(normalized["driver_result"], wire)
                self.assertEqual(root_compare.revalidate_observation(entry, normalized), normalized)

    def test_rejects_incomplete_or_forged_proofs(self):
        entry, valid = sample("gc_root_snapshots_reference_64", 17, "warm_call", 3)
        mutations = (
            ("callbacks_checked", 0), ("nested_calls_checked", 1),
            ("result", "i32:0"), ("live_after_reset", 1),
            ("reset_ns", math.inf), ("report_complete", False),
            ("callback_payload_max", 89),
        )
        for key, value in mutations:
            with self.subTest(key=key):
                wire = copy.deepcopy(valid)
                wire[key] = value
                with self.assertRaises(root_compare.RootComparisonError):
                    root_compare.validate_observation(entry, wire, 3, expected_stage="warm_call")

    def test_rejects_wrong_settings_batch_timing_and_retained_wire(self):
        entry, valid = sample("gc_root_snapshots_numeric_256", 1, "first_call")
        cases = []
        wire = copy.deepcopy(valid); wire["mode"] = "baseline"; cases.append((wire, 1, "first_call"))
        wire = copy.deepcopy(valid); wire["batch"] = 2; cases.append((wire, 1, "first_call"))
        wire = copy.deepcopy(valid); wire["sample_ns"] += 2; cases.append((wire, 1, "first_call"))
        for wire, batch, stage in cases:
            with self.subTest(wire=wire):
                with self.assertRaises(root_compare.RootComparisonError):
                    root_compare.validate_observation(entry, wire, batch, expected_stage=stage)
        with self.assertRaises(root_compare.RootComparisonError):
            root_compare.validate_observation(entry, valid, True, expected_stage="first_call")
        normalized = root_compare.validate_observation(entry, valid, 1, expected_stage="first_call")
        normalized["driver_result"]["bank_slots"] += 1
        with self.assertRaises(root_compare.RootComparisonError):
            root_compare.revalidate_observation(entry, normalized)

    def test_rejects_unknown_model_row(self):
        entry, wire = sample("gc_root_snapshots_numeric_64", 1, "first_call")
        entry = copy.deepcopy(entry)
        entry["id"] = "unknown-root_n1"
        with self.assertRaises(root_compare.RootComparisonError):
            root_compare.validate_observation(entry, wire, 1, expected_stage="first_call")


if __name__ == "__main__":
    unittest.main()
