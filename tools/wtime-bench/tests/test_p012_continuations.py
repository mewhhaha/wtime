"""P012 fixture binding and independent continuation outcome models."""
import hashlib
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parents[1]
ROOT = HERE.parents[1]
ENCODER = ROOT / "target" / "debug" / "wtime-conformance"
sys.path.insert(0, str(HERE))
import continuation_workloads as continuation


class ContinuationWorkloadTests(unittest.TestCase):
    def test_v2_metadata_binds_fixture_sources_and_declares_ready_profile(self):
        entries = continuation.workload_entries()
        self.assertEqual(len(entries), 8)
        self.assertEqual(continuation.verify_fixture_sources(), [])
        for entry in entries:
            with self.subTest(workload=entry["id"]):
                self.assertEqual(entry["schema_version"], 2)
                self.assertEqual(entry["profile"]["status"], "ready")
                self.assertTrue(entry["stages"])
                self.assertTrue(entry["oracle"]["independent"])
                self.assertEqual(len(entry["source"]["wasm_sha256"]), 64)

    def test_depth_oracle_has_explicit_payload_result_and_frame_scaling(self):
        expected = {0: (1009, 1), 4: (1019, 5), 32: (1537, 33)}
        entries = {entry["id"]: entry for entry in continuation.workload_entries()}
        for depth, (result, frames) in expected.items():
            modeled = continuation.depth_oracle(depth)
            self.assertEqual(modeled["expected_suspension"], {
                "kind": "suspended", "payload": [{"type": "i32", "value": 7}],
            })
            self.assertEqual(modeled["expected_resume"], {
                "kind": "returned", "result": [{"type": "i32", "value": result}],
            })
            self.assertEqual(modeled["captured_logical_frames"], frames)
            self.assertEqual(entries[f"continuation_depth_{depth}"]["oracle"]["expected_resume"],
                             modeled["expected_resume"])
        with self.assertRaises(ValueError):
            continuation.depth_oracle(8)

    def test_reference_bind_one_shot_and_cross_module_probes_are_explicit(self):
        entries = {entry["id"]: entry for entry in continuation.workload_entries()}
        ref = entries["continuation_captured_reference"]
        self.assertIn("collect", ref["host_setup"]["collection"])
        self.assertEqual(ref["oracle"]["expected_resume"]["result"], [{"type": "i32", "value": 86}])
        bound = entries["continuation_bound_nested"]["oracle"]
        self.assertEqual(bound["expected_worker_calls"], 1)
        self.assertEqual(bound["expected_resume"]["result"], [
            {"type": "i32", "value": 0}, {"type": "i32", "value": 1122},
        ])
        one_shot = entries["continuation_one_shot"]["oracle"]
        self.assertEqual(one_shot["expected_first_resume"]["result"], [{"type": "i32", "value": 1}])
        self.assertEqual(one_shot["expected_second_resume"]["kind"], "trap")
        consumer = entries["continuation_cross_module_consumer"]["oracle"]
        self.assertEqual(consumer["expected_suspension"]["kind"], "suspended")
        self.assertEqual(consumer["expected_resume"]["result"], [{"type": "i32", "value": 77}])
        self.assertEqual(consumer["nominal_probe"]["incompatible_exact_host_result"], "rejected with Host error")

    def test_continuation_bytes_are_recorded_as_size_sweep_not_fixed_estimates(self):
        metadata = continuation.accounting_metadata()
        self.assertEqual(metadata["api"], "Store.continuation_bytes")
        self.assertEqual(metadata["asymptotic_driver"]["fixture_ids"], [
            "continuation_depth_0", "continuation_depth_4", "continuation_depth_32",
        ])
        self.assertIn("report measured values", metadata["asymptotic_driver"]["rule"])
        self.assertIn("zero max_continuation_bytes rejects the first capture", metadata["boundary_controls"])

    def test_existing_encoder_wasm_hashes_match_all_wat_sources(self):
        if not ENCODER.is_file():
            self.skipTest("existing wtime-conformance encoder is not available; no build requested")
        for entry in continuation.workload_entries():
            with self.subTest(workload=entry["id"]), tempfile.TemporaryDirectory() as directory:
                source = HERE / entry["source"]["path"]
                wasm = Path(directory) / (source.stem + ".wasm")
                subprocess.run([str(ENCODER), "encode", str(source), "-o", str(wasm)],
                               check=True, capture_output=True, text=True)
                self.assertEqual(hashlib.sha256(wasm.read_bytes()).hexdigest(),
                                 entry["source"]["wasm_sha256"])


if __name__ == "__main__":
    unittest.main()
