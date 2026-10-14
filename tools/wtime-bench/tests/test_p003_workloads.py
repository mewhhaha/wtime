import copy
import hashlib
import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(HERE))

import continuation_workloads
import scalar_workloads
import workloads


class WorkloadRegistryTests(unittest.TestCase):
    def test_explicit_selections_and_original_fixture_hashes(self):
        registry = workloads.load_registry()
        self.assertEqual([w["id"] for w in workloads.selection("original-mvp", registry)], [
            "call_overhead", "integer_loop", "memory_fill_scan", "branch_calls", "prepare_large"
        ])
        legacy_scalar_ids = ["call_overhead", "integer_loop", "branch_calls"]
        scalar_model = scalar_workloads.entries()
        self.assertEqual(
            [w["id"] for w in workloads.selection("scalar", registry)],
            legacy_scalar_ids + [entry["id"] for entry in scalar_model],
        )
        self.assertEqual(workloads.selection("scalar", registry)[3:], scalar_model)
        continuation_model = [
            entry for entry in continuation_workloads.workload_entries()
            if entry["id"] != "continuation_cross_module_provider"
        ]
        self.assertEqual(registry["selections"]["continuations"]["status"], "ready")
        self.assertEqual(workloads.selection("continuations", registry), continuation_model)
        for name, declared in registry["selections"].items():
            if declared["status"] == "ready":
                with self.subTest(selection=name):
                    selected = workloads.selection(name, registry)
                    self.assertTrue(selected)
                    self.assertEqual([entry["id"] for entry in selected], declared["workloads"])
        for entry in registry["workloads"]:
            source = entry["source"]
            if source["kind"] == "wat_file":
                data = (HERE / source["path"]).read_bytes()
            else:
                data = (workloads.generate_prepare_large() + "\n").encode()
            self.assertEqual(hashlib.sha256(data).hexdigest(), source["sha256"], entry["id"])

    def test_pending_selection_still_cannot_launch(self):
        registry = copy.deepcopy(workloads.load_registry())
        registry["selections"]["continuations"] = {
            "status": "pending", "owner_task": "synthetic-readiness-boundary", "workloads": []
        }
        with self.assertRaises(workloads.EmptySelectionError):
            workloads.selection("continuations", registry)

    def test_float_and_v128_bits_round_trip_little_endian(self):
        f32 = workloads.encode_value("f32_bits", 0x80000000)
        f64 = workloads.encode_value("f64_bits", 0x7FF8000000000042)
        raw = bytes(range(16))
        vector = workloads.encode_value("v128_bits", raw)
        self.assertEqual(workloads.decode_value(f32), 0x80000000)
        self.assertEqual(workloads.decode_value(f64), 0x7FF8000000000042)
        self.assertEqual(vector["value"], "000102030405060708090a0b0c0d0e0f")
        self.assertEqual(workloads.decode_value(vector), raw)
        self.assertEqual(workloads.decode_value(workloads.encode_value("reference", "struct")), "struct")
        with self.assertRaises(workloads.WorkloadError):
            workloads.encode_value("f32_bits", 1 << 32)
        with self.assertRaises(workloads.WorkloadError):
            workloads.encode_value("v128_bits", bytes(15))

    def test_duplicate_ids_oracle_and_reset_conflicts_are_rejected(self):
        registry = workloads.load_registry()
        duplicate = copy.deepcopy(registry)
        duplicate["workloads"].append(copy.deepcopy(duplicate["workloads"][0]))
        with self.assertRaisesRegex(workloads.WorkloadError, "duplicate workload id"):
            workloads.validate_registry(duplicate)

        missing_oracle = copy.deepcopy(registry)
        del missing_oracle["workloads"][0]["oracle"]
        with self.assertRaisesRegex(workloads.WorkloadError, "independent registered oracle"):
            workloads.validate_registry(missing_oracle)

        bad_reset = copy.deepcopy(registry)
        bad_reset["workloads"][0]["reset_policy"] = "fresh_instance_per_invocation"
        with self.assertRaisesRegex(workloads.WorkloadError, "per-invocation reset conflicts"):
            workloads.validate_registry(bad_reset)


if __name__ == "__main__":
    unittest.main()
