import hashlib
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(HERE))

import gc_workloads
import workloads


class GcExceptionWorkloadTests(unittest.TestCase):
    def test_schema_hashes_and_driver_requirements_are_explicit(self):
        entries = gc_workloads.entries()
        ids = {item["id"] for item in entries}
        self.assertIn("gc_packed_array_n32", ids)
        self.assertIn("gc_struct_graph_n1024", ids)
        self.assertIn("gc_nominal_exact_casts_n32", ids)
        self.assertIn("gc_typed_function_refs_n32", ids)
        self.assertIn("gc_descriptor_cross_module_n32", ids)
        self.assertIn("gc_host_reentry_collect_n32", ids)
        self.assertIn("gc_continuation_root_n32", ids)
        self.assertIn("exception_normal_path_n32", ids)
        self.assertIn("exception_caught_throw_n32", ids)
        self.assertIn("exception_uncaught_throw_n32", ids)
        for item in entries:
            source = HERE / item["source"]["path"]
            self.assertEqual(hashlib.sha256(source.read_bytes()).hexdigest(),
                             item["source"]["sha256"], item["id"])
            self.assertEqual(len(item["source"]["wasm_sha256"]), 64)
            self.assertEqual(item["reset_policy"], "fresh_instance_per_invocation")
            self.assertNotIn("warm_call", item["stages"])
            self.assertEqual(item["comparators"]["wasmtime"]["status"], "unsupported")
            if item["family"].startswith("gc_"):
                self.assertEqual(item["collector_stage"]["name"], "collector_stage")
        cross = next(item for item in entries if item["id"] == "gc_descriptor_cross_module_n32")
        self.assertEqual(cross["setup_modules"][0]["module_name"], "provider")
        self.assertTrue(cross["requires_host_driver"])
        self.assertEqual(cross["state_expectation"]["result"], 32)
        host_gc = next(item for item in entries if item["id"] == "gc_host_reentry_collect_n32")
        self.assertEqual(host_gc["collector_stage"]["name"], "collector_stage")
        self.assertEqual(host_gc["collector_stage"]["expected"]["live"], 1)
        continuation = next(item for item in entries if item["id"] == "gc_continuation_root_n32")
        self.assertIn("stack_switching", continuation["required_features"])
        self.assertEqual(continuation["collector_stage"]["expected"]["live_min"], 3)
        root = next(item for item in entries if item["id"] == "gc_root_lifecycle_n32")
        self.assertEqual(root["collector_stage"]["sequence"][-1]["expected"],
                         {"live": 0, "reclaimed": 1})
        workloads.validate_registry({
            "schema_version": 2,
            "workloads": entries,
            "selections": {"gc-exceptions": {"status": "ready", "workloads": sorted(ids)}},
        })

    def test_array_graph_cast_and_typed_call_oracles(self):
        n = 32
        self.assertEqual(gc_workloads.oracle("gc_packed_array_n32", n), sum([-3] * n))
        self.assertEqual(gc_workloads.oracle("gc_struct_graph_n32", n), sum(range(n)))
        # Proper subtype passes both nominal types and fails exact-base cast.
        self.assertEqual(gc_workloads.oracle("gc_nominal_exact_casts_n32", n), 32 * 2)
        self.assertEqual(gc_workloads.oracle("gc_typed_function_refs_n32", n), sum(i + 1 for i in range(n)))
        self.assertEqual(gc_workloads.oracle("gc_descriptor_graph_n32", n), n)
        self.assertEqual(gc_workloads.oracle("gc_descriptor_cross_module_n32", n), n)

    def test_exception_outcomes_are_normalized_across_normal_caught_and_uncaught_paths(self):
        n = 123
        self.assertEqual(gc_workloads.oracle("exception_normal_path_n123", n), n)
        self.assertEqual(gc_workloads.oracle("exception_caught_throw_n123", n), n)
        self.assertEqual(gc_workloads.oracle("exception_cross_call_catch_n123", n), n)
        self.assertEqual(gc_workloads.oracle("exception_host_throw_catch_n123", n), n)
        self.assertEqual(gc_workloads.oracle("exception_throw_ref_unwind_n123", n), n)
        self.assertEqual(gc_workloads.oracle("exception_uncaught_throw_n123", n),
                         {"kind": "exception", "tag": "number", "payload": [n]})
        uncaught = next(item for item in gc_workloads.entries()
                        if item["id"] == "exception_uncaught_throw_n1")
        self.assertEqual(uncaught["result"]["type"], "none")
        self.assertEqual(uncaught["exception_tag"], "number")
        host_throw = next(item for item in gc_workloads.entries()
                          if item["id"] == "exception_host_throw_catch_n32")
        self.assertTrue(host_throw["requires_host_driver"])
        self.assertEqual(host_throw["host_imports"][1]["outcome"]["payload"], [32])

    def test_wat_files_are_bound_to_existing_encoder_output(self):
        encoder = Path("/workspace/wtime/target/debug/wtime-conformance")
        if not encoder.is_file():
            self.skipTest("existing conformance encoder is unavailable")
        with tempfile.TemporaryDirectory() as tempdir:
            hashes = gc_workloads.encode_fixtures(encoder, tempdir)
            self.assertEqual(hashes, gc_workloads.WASM_SHA256)
            for filename, digest in hashes.items():
                wasm = Path(tempdir, Path(filename).with_suffix(".wasm"))
                self.assertEqual(hashlib.sha256(wasm.read_bytes()).hexdigest(), digest)


if __name__ == "__main__":
    unittest.main()
