from __future__ import annotations

import copy
import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parents[1]
MODEL_ROOT = (Path(__file__).resolve().parents[2]
              / "P071-foundation-0001/source/tools/wtime-bench")
sys.path.insert(0, str(MODEL_ROOT))
sys.path.insert(0, str(HERE))

import gc_collect_compare
import gc_collect_workloads


def make_wire(entry: dict, stage: str = "warm_call", batch: int = 3) -> dict:
    repetitions = entry["params"][0]["baseline"]
    record = gc_collect_workloads.FAMILIES[entry["family"]]
    nodes = record["count"]
    live = nodes + 1 if record["shape"] == "wide" else nodes
    calls = batch + 1 if stage == "warm_call" else 1
    prepare_ns = 11.0
    instantiate_ns = 23.0
    first_call_ns = 7.0
    if stage == "warm_call":
        sample_ns = 9.5
        measured_guest_ns = sample_ns * batch
    else:
        measured_guest_ns = first_call_ns
        sample_ns = {
            "prepare": prepare_ns,
            "instantiate": instantiate_ns,
            "first_call": first_call_ns,
            "load_and_call": prepare_ns + instantiate_ns + first_call_ns,
        }[stage]
    return {
        "schema_version": 2,
        "driver": gc_collect_compare.DRIVER,
        "case": entry["id"],
        "family": entry["family"],
        "shape": record["shape"],
        "nodes": nodes,
        "live_objects": live,
        "input": repetitions,
        "mode": gc_collect_compare.MODE,
        "profile": gc_collect_compare.PROFILE,
        "stage": stage,
        "batch": batch,
        "max_batch": gc_collect_workloads.max_batch(nodes, repetitions),
        "instrumented": False,
        "measurement_policy": gc_collect_compare.MEASUREMENT_POLICY,
        "report_complete": True,
        "sample_ns": sample_ns,
        "prepare_ns": prepare_ns,
        "instantiate_ns": instantiate_ns,
        "first_call_ns": first_call_ns,
        "measured_guest_ns": measured_guest_ns,
        "cleanup_ns": 31.0,
        "calls_checked": calls,
        "callbacks_checked": calls * repetitions,
        "callback_live": live,
        "callback_reclaimed": 0,
        "callback_payload_min": 4096,
        "callback_payload_max": 4096,
        "final_live": 0,
        "final_reclaimed": live,
        "final_payload_bytes": 0,
        "final_continuation_bytes": 0,
        "result": f"i32:{gc_collect_workloads.oracle(entry['id'], repetitions)}",
    }


class GcCollectComparisonTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.rows = gc_collect_workloads.entries()
        cls.by_id = {row["id"]: row for row in cls.rows}

    def test_all_nine_model_rows_validate_exact_timer_stages(self):
        self.assertEqual(len(self.rows), 9)
        for row in self.rows:
            with self.subTest(row=row["id"]):
                for stage in gc_collect_workloads.STAGES:
                    batch = 3 if stage == "warm_call" else 1
                    normalized = gc_collect_compare.validate_observation(
                        row, make_wire(row, stage, batch), batch,
                        expected_mode="decoded", expected_stage=stage)
                    self.assertEqual(normalized["result"], row["expected_result"])
                    self.assertEqual(normalized["ns_per_operation"], make_wire(row, stage, batch)["sample_ns"])

    def test_rejects_wrong_result_identity_and_unknown_wire_fields(self):
        row = self.by_id["gc_collect_wide_32_n32"]
        base = make_wire(row)
        for field, value in (("result", "i32:0"), ("case", "gc_collect_chain_256_n17"),
                             ("shape", "chain"), ("profile", "mvp"),
                             ("unexpected", 1)):
            tampered = copy.deepcopy(base)
            tampered[field] = value
            with self.subTest(field=field), self.assertRaises(gc_collect_compare.GcCollectComparisonError):
                gc_collect_compare.validate_observation(row, tampered, 3)

    def test_rejects_callback_count_liveness_payload_and_final_cleanup_drift(self):
        row = self.by_id["gc_collect_chain_256_n17"]
        base = make_wire(row)
        for field, value in (("calls_checked", 3), ("callbacks_checked", 1),
                             ("callback_live", 1), ("callback_reclaimed", 1),
                             ("callback_payload_min", 0), ("callback_payload_max", 4097),
                             ("final_live", 1), ("final_reclaimed", 0),
                             ("final_payload_bytes", 1), ("final_continuation_bytes", 1)):
            tampered = copy.deepcopy(base)
            tampered[field] = value
            with self.subTest(field=field), self.assertRaises(gc_collect_compare.GcCollectComparisonError):
                gc_collect_compare.validate_observation(row, tampered, 3)

    def test_rejects_bool_integer_substitutions_and_nonfinite_times(self):
        row = self.by_id["gc_collect_wide_32_n1"]
        for field in ("schema_version", "nodes", "input", "batch", "calls_checked"):
            tampered = make_wire(row, "first_call", 1)
            tampered[field] = True
            with self.subTest(field=field), self.assertRaises(gc_collect_compare.GcCollectComparisonError):
                gc_collect_compare.validate_observation(row, tampered, 1)
        tampered_entry = copy.deepcopy(row)
        tampered_entry["params"][0]["baseline"] = True
        with self.assertRaises(gc_collect_compare.GcCollectComparisonError):
            gc_collect_compare.validate_observation(tampered_entry, make_wire(row, "first_call", 1), 1)
        for field in ("sample_ns", "prepare_ns", "measured_guest_ns", "cleanup_ns"):
            tampered = make_wire(row, "first_call", 1)
            tampered[field] = float("nan")
            with self.subTest(field=field), self.assertRaises(gc_collect_compare.GcCollectComparisonError):
                gc_collect_compare.validate_observation(row, tampered, 1)

    def test_rejects_wrong_requested_stage_batch_mode_and_cap(self):
        row = self.by_id["gc_collect_wide_1024_n32"]
        wire = make_wire(row, "warm_call", 3)
        with self.assertRaises(gc_collect_compare.GcCollectComparisonError):
            gc_collect_compare.validate_observation(row, wire, 2, expected_stage="warm_call")
        with self.assertRaises(gc_collect_compare.GcCollectComparisonError):
            gc_collect_compare.validate_observation(row, wire, 3, expected_stage="first_call")
        with self.assertRaises(gc_collect_compare.GcCollectComparisonError):
            gc_collect_compare.validate_observation(row, make_wire(row, "first_call", 1), 1,
                                                    expected_mode="register")
        over_cap = make_wire(row, "warm_call", gc_collect_workloads.max_batch(1024, 32) + 1)
        with self.assertRaises(gc_collect_compare.GcCollectComparisonError):
            gc_collect_compare.validate_observation(row, over_cap, over_cap["batch"])

    def test_revalidation_rejects_normalized_and_raw_wire_tampering(self):
        row = self.by_id["gc_collect_wide_32_n32"]
        normalized = gc_collect_compare.validate_observation(row, make_wire(row), 3)
        accepted = gc_collect_compare.revalidate_observation(row, normalized, expected_batch=3)
        self.assertEqual(accepted, normalized)
        for field, value in (("result", 0), ("ns_per_operation", 1.0), ("mode", "register")):
            tampered = copy.deepcopy(normalized)
            tampered[field] = value
            with self.subTest(normalized_field=field), self.assertRaises(gc_collect_compare.GcCollectComparisonError):
                gc_collect_compare.revalidate_observation(row, tampered, expected_batch=3)
        tampered = copy.deepcopy(normalized)
        tampered["driver_result"]["final_reclaimed"] -= 1
        with self.assertRaises(gc_collect_compare.GcCollectComparisonError):
            gc_collect_compare.revalidate_observation(row, tampered, expected_batch=3)


if __name__ == "__main__":
    unittest.main()
