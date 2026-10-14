import hashlib
import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(HERE))

import scalar_workloads


class ScalarWorkloadTests(unittest.TestCase):
    def test_entries_cover_scaling_and_held_out_inputs(self):
        entries = scalar_workloads.entries()
        expected_count = (
            (len(scalar_workloads.FAMILIES) - 1) * len(scalar_workloads.INPUTS)
            + len(scalar_workloads.RECURSION_INPUTS)
        )
        self.assertEqual(len(entries), expected_count)
        self.assertEqual({entry["family"] for entry in entries}, set(scalar_workloads.FAMILIES))
        self.assertEqual(
            [entry["params"][0]["baseline"] for entry in entries[:6]], [1, 32, 1024, 10000, 0, 17]
        )
        for entry in entries:
            source = HERE / entry["source"]["path"]
            self.assertEqual(hashlib.sha256(source.read_bytes()).hexdigest(),
                             entry["source"]["sha256"])
            self.assertEqual(entry["result"]["type"], "i32")
            self.assertEqual(entry["params"][0]["type"], "i32")
            filename = Path(entry["source"]["path"]).name
            self.assertEqual(entry["source"]["encoding_status"], "encoded")
            self.assertEqual(entry["source"]["wasm_sha256"],
                             scalar_workloads.WASM_SHA256[filename])
        tail = next(entry for entry in entries if entry["family"] == "scalar_tail_calls")
        self.assertIn("tail_call", tail["required_features"])
        self.assertEqual(tail["comparators"]["wasmtime"]["status"], "probe_required")
        held = [entry for entry in entries if entry["validation_role"] == "held_out"]
        self.assertTrue(held)

    def test_wrapping_and_hand_computed_oracles(self):
        self.assertEqual(scalar_workloads.oracle("scalar_accumulator", 0), 0)
        self.assertEqual(scalar_workloads.oracle("scalar_accumulator_n17", 17), 136)
        self.assertEqual(scalar_workloads.oracle("scalar_dependent", 0), 1)
        self.assertEqual(scalar_workloads.oracle("scalar_dependent", 1), 1015568748)
        self.assertEqual(scalar_workloads.oracle("scalar_direct_calls", 1), 4)
        self.assertEqual(scalar_workloads.oracle("scalar_indirect_calls", 2), 13)
        self.assertEqual(scalar_workloads.oracle("scalar_tail_calls", 2), 13)
        self.assertEqual(scalar_workloads.oracle("scalar_br_table", 5), 25)
        self.assertEqual(scalar_workloads.oracle("scalar_multivalue_branch", 0), 1)
        self.assertEqual(scalar_workloads.oracle("scalar_multivalue_branch", 2), 4)
        self.assertEqual(scalar_workloads.oracle("scalar_ordinary_recursion", 2), 13)
        self.assertEqual(scalar_workloads.oracle("scalar_call_ref", 2), 13)
        self.assertEqual(scalar_workloads.oracle("scalar_host_call", 17), 17)
        self.assertEqual(scalar_workloads.table_mutation_probe(1), (2, 3))

    def test_branch_oracle_matches_independent_reference_loop(self):
        state = 1
        for _ in range(17):
            state = (state * 1664525 + 1013904223) & 0xFFFF_FFFF
            if state & 0x8000_0000:
                state ^= 1511506142
            else:
                state = (state + 265443576) & 0xFFFF_FFFF
        expected = state - 0x1_0000_0000 if state & 0x8000_0000 else state
        self.assertEqual(scalar_workloads.oracle("scalar_branch_entropy", 17), expected)

    def test_call_shapes_and_api_availability_are_labeled_honestly(self):
        for filename, token in [
            ("direct_calls.wat", "call $step"),
            ("indirect_calls.wat", "call_indirect"),
            ("tail_calls.wat", "return_call $walk"),
            ("call_ref.wat", "call_ref $unary"),
            ("multivalue_branch.wat", "(result i32 i32)"),
            ("ordinary_recursion.wat", "(call $walk"),
            ("table_controls.wat", '"probe_null"'),
            ("host_call.wat", '(import "host" "tick"'),
            ("br_table.wat", "br_table"),
        ]:
            text = (HERE / "fixtures" / "scalar" / filename).read_text()
            self.assertIn(token, text)
            self.assertIn('(func (export "run") (param $n i32) (result i32)', text)
        for entry in scalar_workloads.entries():
            api = entry["api_comparison"]
            self.assertEqual(api["dynamic_values"]["status"], "available")
            self.assertEqual(api["typed_guest_call"]["status"], "unavailable")
            self.assertIn("not a typed guest-call API", api["typed_guest_call"]["reason"])
        table = next(entry for entry in scalar_workloads.entries()
                     if entry["family"] == "scalar_table_mutation")
        self.assertEqual(table["measurement_status"], "driver_setup_required")
        self.assertEqual(len(table["driver_requirements"]["correctness_probes"]), 4)
        host = next(entry for entry in scalar_workloads.entries()
                    if entry["family"] == "scalar_host_call")
        self.assertEqual(host["measurement_status"], "host_setup_required")
        recursion = next(entry for entry in scalar_workloads.entries()
                         if entry["family"] == "scalar_ordinary_recursion")
        self.assertEqual(recursion["driver_requirements"]["timed_input_max"], 32)


if __name__ == "__main__":
    unittest.main()
