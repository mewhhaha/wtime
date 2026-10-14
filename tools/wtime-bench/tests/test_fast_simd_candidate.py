"""Synthetic candidate-lane checks for typed SIMD observations."""
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

TOOL = Path(__file__).resolve().parents[1]
sys.dont_write_bytecode = True
sys.path.insert(0, str(TOOL))

import candidate_gate
import focused_compare
import simd_scaling
import simd_workloads


class FocusedSimdCandidateTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="wtime-fast-simd-candidate-")
        self.root = Path(self.temporary.name).resolve()
        self.addCleanup(self.temporary.cleanup)
        self.entries = {row["id"]: row for row in simd_workloads.entries()}

    def inside(self, relative):
        path = (self.root / relative).resolve()
        self.assertTrue(path.is_relative_to(self.root), "synthetic path escaped fresh temp root")
        return path

    def observation(self, entry, *, wire_result=None, stage="load_and_call"):
        allowed = simd_scaling.allowed_wire_results(entry, simd_workloads)
        values = sorted(int(item.removeprefix("i32:")) for item in allowed)
        if wire_result is None:
            wire_result = values[0]
        result = values[0] if len(values) == 1 else {"allowed_results": values}
        row = {
            "result": result,
            "wire_result": wire_result,
            "ns_per_operation": 19.0,
            "batch": 1,
            "mode": "decoded",
            "settings": {
                "feature_profile": focused_compare.profile(entry),
                "features": entry["required_features"],
                "fuel": False,
                "cancellation_quantum": 1,
            },
        }
        if isinstance(entry.get("memory_expectation"), dict):
            expected = simd_workloads.expected_memory(
                entry["id"], next(entry["oracle_arguments"][key]
                                  for key in ("n", "case", "index", "vectors", "value")
                                  if key in entry["oracle_arguments"]))
            row.update({
                "memory_pages": expected["page_count"],
                "memory_page_size_log2": expected["page_size_bytes"].bit_length() - 1,
                "memory_full_sha256": expected["full_memory_sha256"],
                "memory_range": expected["range"],
                "memory_range_sha256": expected["sha256"],
                "memory_snapshot_phase": "preflight" if stage == "prepare" else "post_invoke",
                "memory_oracle": simd_scaling._memory_oracle_record(entry),
            })
        return row

    def check(self, wid, observation=None, stage="load_and_call"):
        entry = self.entries[wid]
        row = observation or self.observation(entry, stage=stage)
        candidate_gate.check_observation(entry, row, "decoded", self.root, stage=stage)

    def test_logical_input_prefers_baseline_then_value_then_zero(self):
        random_entry = self.entries["simd_integer_lanes_random_00"]
        self.assertEqual(focused_compare.input_value(random_entry), 17)
        self.assertEqual(focused_compare.input_value({"params": [{"baseline": 0, "value": 8}]}), 0)
        self.assertEqual(focused_compare.input_value({"params": [{"value": 5}]}), 5)
        self.assertEqual(focused_compare.input_value({"params": []}), 0)

    def test_randomized_multi_param_row_routes_typed_wire_input_and_logical_value(self):
        entry = self.entries["simd_integer_lanes_random_00"]
        fixture = self.inside("fixture.wasm")
        output = self.inside("evidence")
        with mock.patch.object(focused_compare.compare, "_simd_observation",
                               return_value={"result": 7}) as observe:
            result = focused_compare.observe("unused-executable", entry, fixture,
                                             "load_and_call", 1, "decoded", 0, output)
        self.assertEqual(result, {"result": 7})
        args, kwargs = observe.call_args
        self.assertEqual(args[3], simd_scaling.typed_input(entry))
        self.assertEqual(args[4], 17)
        self.assertEqual(args[5], simd_scaling.allowed_wire_results(entry, simd_workloads))
        self.assertEqual(args[6:9], ("load_and_call", 1, "decoded"))
        self.assertEqual(args[9], "simd")
        self.assertEqual(kwargs["model"], simd_workloads)

    def test_valid_strict_zero_and_relaxed_multiple_allowed_results(self):
        self.check("simd_f64_min_positive_zero")
        relaxed = "simd_relaxed_swizzle_oob_17"
        allowed = simd_scaling.allowed_wire_results(self.entries[relaxed], simd_workloads)
        self.assertGreater(len(allowed), 1)
        self.assertEqual(candidate_gate.expected(self.entries[relaxed]),
                         {"allowed_results": sorted(int(item.removeprefix("i32:")) for item in allowed)})
        for wire in allowed:
            with self.subTest(wire=wire):
                self.check(relaxed, self.observation(self.entries[relaxed],
                                                     wire_result=int(wire.removeprefix("i32:"))))

    def test_simd_observation_rejects_forged_wire_settings_and_numeric_types(self):
        wid = "simd_integer_lanes_random_00"
        mutations = {
            "wire_result": lambda row: row.update(wire_result=True),
            "profile": lambda row: row["settings"].update(feature_profile="mvp"),
            "features": lambda row: row["settings"].update(features=["simd"]),
            "fuel": lambda row: row["settings"].update(fuel=True),
            "cancellation": lambda row: row["settings"].update(cancellation_quantum=2),
            "batch_type": lambda row: row.update(batch=True),
            "batch_limit": lambda row: row.update(batch=100001),
            "timing_type": lambda row: row.update(ns_per_operation=True),
            "timing_nonfinite": lambda row: row.update(ns_per_operation=float("inf")),
        }
        for name, mutate in mutations.items():
            with self.subTest(name=name):
                row = self.observation(self.entries[wid])
                mutate(row)
                with self.assertRaises(ValueError):
                    self.check(wid, row)

    def test_simd_cold_observation_requires_batch_one(self):
        wid = "simd_integer_lanes_random_00"
        row = self.observation(self.entries[wid])
        row["batch"] = 2
        with self.assertRaisesRegex(ValueError, "cold stage is batched"):
            self.check(wid, row, stage="load_and_call")

    def test_memory_proof_is_rechecked_from_hashes_without_claiming_raw_bytes(self):
        for wid in ("simd_lane_memory_n17", "simd_lane_memory_footprint_16"):
            with self.subTest(wid=wid):
                self.check(wid)
                row = self.observation(self.entries[wid])
                self.assertNotIn("memory_hex", row)

    def test_memory_proof_rejects_each_forged_oracle_dimension(self):
        wid = "simd_lane_memory_n17"
        mutations = {
            "full_hash": lambda row: row.update(memory_full_sha256="0" * 64),
            "range_hash": lambda row: row.update(memory_range_sha256="0" * 64),
            "range": lambda row: row.update(memory_range=[1, 16]),
            "pages": lambda row: row.update(memory_pages=2),
            "snapshot_phase": lambda row: row.update(memory_snapshot_phase="preflight"),
            "oracle_record": lambda row: row["memory_oracle"].update(full_memory_sha256="0" * 64),
        }
        for name, mutate in mutations.items():
            with self.subTest(name=name):
                row = self.observation(self.entries[wid])
                mutate(row)
                with self.assertRaises(ValueError):
                    self.check(wid, row)


if __name__ == "__main__":
    unittest.main()
