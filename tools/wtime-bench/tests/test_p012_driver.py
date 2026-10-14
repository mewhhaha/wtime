"""P012 subprocess-test draft; keep under /tmp until P023 timing is done."""

import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

BENCH = os.environ.get("WTIME_CONTINUATION_BENCH")
ENCODER = os.environ.get("WTIME_CONFORMANCE_ENCODER", "/workspace/wtime/target/debug/wtime-conformance")
BENCH_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(BENCH_ROOT))
import continuation_workloads as continuation


@unittest.skipUnless(BENCH and Path(BENCH).is_file(), "set WTIME_CONTINUATION_BENCH to a built current-source binary")
class ContinuationDriverDraftTests(unittest.TestCase):
    def encode(self, directory, fixture):
        wat = BENCH_ROOT / "fixtures/continuations" / f"{fixture}.wat"
        wasm = Path(directory) / f"{fixture}.wasm"
        result = subprocess.run([ENCODER, "encode", str(wat), "-o", str(wasm)], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        expected = continuation._FILES
        entry = next(value for value in continuation.workload_entries() if value["source"]["path"].endswith(f"/{fixture}.wat"))
        self.assertEqual(hashlib.sha256(wat.read_bytes()).hexdigest(), entry["source"]["sha256"])
        self.assertEqual(hashlib.sha256(wasm.read_bytes()).hexdigest(), entry["source"]["wasm_sha256"])
        return wasm

    def observe(self, args):
        result = subprocess.run([BENCH, *map(str, args)], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        return json.loads(result.stdout)

    def test_numeric_depth_capture_resume_and_accounting(self):
        with tempfile.TemporaryDirectory() as temp:
            for depth, expected in ((0, 1009), (4, 1019), (32, 1537)):
                with self.subTest(depth=depth):
                    wasm = self.encode(temp, f"depth_{depth}")
                    row = self.observe(["--continuation-driver", wasm, f"continuation_depth_{depth}", "decoded", "all-phase3-core", "depth"])
                    self.assertEqual(row["payload"], ["i32:7"])
                    self.assertEqual(row["result"], [f"i32:{expected}"])
                    self.assertEqual(row["logical_frames"], depth + 1)
                    self.assertGreater(row["capture_ns"], 0)
                    self.assertGreater(row["resume_ns"], 0)
                    checkpoints = row["continuation_bytes"]
                    self.assertGreater(checkpoints["after_capture"], checkpoints["before_capture"])
                    self.assertEqual(checkpoints["after_root_transfer"], checkpoints["after_capture"])
                    self.assertEqual(checkpoints["after_drop_and_collect"], 0)

    def test_reference_root_transfer_and_guest_resume(self):
        with tempfile.TemporaryDirectory() as temp:
            wasm = self.encode(temp, "captured_reference")
            row = self.observe(["--continuation-driver", wasm, "continuation_captured_reference", "decoded", "all-phase3-core", "root"])
            self.assertEqual(row["payload"], ["i32:5"])
            self.assertEqual(row["result"], ["i32:86"])
            self.assertEqual(row["collection_with_root"]["live"], 2)
            self.assertEqual(row["continuation_bytes"]["after_drop_and_collect"], 0)

    def test_nested_bind_and_one_shot_reuse_controls(self):
        with tempfile.TemporaryDirectory() as temp:
            bound = self.encode(temp, "bound_nested")
            row = self.observe(["--continuation-driver", bound, "continuation_bound_nested", "decoded", "all-phase3-core", "bind"])
            self.assertEqual(row["result"], ["i32:0", "i32:1122"])
            self.assertEqual(row["worker_calls"], 1)
            self.assertGreater(row["bind_ns"], 0)
            self.assertGreater(row["resume_ns"], 0)
            self.assertEqual(row["bind_result"], "i32:0")
            self.assertEqual(row["continuation_bytes"]["after_collect"], 0)
            one_shot = self.encode(temp, "one_shot")
            row = self.observe(["--continuation-driver", one_shot, "continuation_one_shot", "decoded", "all-phase3-core", "oneshot"])
            self.assertEqual(row["first_result"], "i32:1")
            self.assertGreater(row["resume_once_ns"], 0)
            self.assertGreater(row["resume_reuse_ns"], 0)
            self.assertEqual(row["result"], "trap")
            self.assertEqual(row["error_kind"], "Trap")
            self.assertIn("continuation", row["error_message"].lower())
            self.assertEqual(row["continuation_bytes"]["after_collect"], 0)

    def test_cross_module_import_tag_and_fuel_budget_boundaries(self):
        with tempfile.TemporaryDirectory() as temp:
            provider = self.encode(temp, "cross_module_provider")
            consumer = self.encode(temp, "cross_module_consumer")
            row = self.observe(["--continuation-cross-module", provider, consumer, "decoded", "all-phase3-core"])
            self.assertTrue(row["exact_host_result_rejected"])
            self.assertEqual(row["exact_host_error_kind"], "Host")
            self.assertEqual(row["payload"], [])
            self.assertEqual(row["result"], ["i32:77"])
            self.assertEqual(row["continuation_bytes"]["after_drop_and_collect"], 0)

            depth0 = self.encode(temp, "depth_0")
            positive = self.observe(["--continuation-probe", depth0, "continuation_depth_0", "decoded", "all-phase3-core", "fuel-positive"])
            self.assertEqual(positive["outcome"], "suspended")
            self.assertEqual(positive["payload"], ["i32:7"])
            self.assertGreater(positive["continuation_bytes"], 0)
            self.assertEqual(positive["continuation_bytes_after_collect"], 0)
            fuel = self.observe(["--continuation-probe", depth0, "continuation_depth_0", "decoded", "all-phase3-core", "fuel"])
            self.assertEqual((fuel["error_kind"], fuel["error_message"]), ("ResourceLimit", "fuel exhausted"))
            budget = self.observe(["--continuation-probe", depth0, "continuation_depth_0", "decoded", "all-phase3-core", "budget"])
            self.assertEqual((budget["error_kind"], budget["error_message"]), ("ResourceLimit", "continuation byte budget"))
            self.assertEqual(fuel["continuation_bytes"], 0)
            self.assertEqual(budget["continuation_bytes"], 0)


if __name__ == "__main__":
    unittest.main()
