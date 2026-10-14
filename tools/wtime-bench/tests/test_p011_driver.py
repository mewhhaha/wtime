"""P011 actual-process validation draft; the parent binary is supplied by env."""

import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import sys
import unittest

BENCH = os.environ.get("WTIME_GC_EXCEPTION_BENCH")
ENCODER = os.environ.get("WTIME_CONFORMANCE_ENCODER", "/workspace/wtime/target/debug/wtime-conformance")
BENCH_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(BENCH_ROOT))
import gc_workloads


@unittest.skipUnless(BENCH and Path(BENCH).is_file(), "set WTIME_GC_EXCEPTION_BENCH to a built current-source binary")
class GcExceptionDriverDraftTests(unittest.TestCase):
    def encode(self, directory, filename):
        source = gc_workloads._path_for(filename)
        output = Path(directory) / f"{source.stem}.wasm"
        result = subprocess.run([ENCODER, "encode", str(source), "-o", str(output)],
                                capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(hashlib.sha256(output.read_bytes()).hexdigest(),
                         gc_workloads.WASM_SHA256[filename])
        return output

    def observe(self, args):
        result = subprocess.run([BENCH, *map(str, args)], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        row = json.loads(result.stdout)
        self.assertEqual(row["instrumented"], False)
        self.assertEqual(row["wasmtime"]["status"], "unsupported")
        self.assertTrue(row["wasmtime"]["reason"])
        return row

    def run_case(self, temp, filename, workload, n):
        wasm = self.encode(temp, filename)
        row = self.observe(["--gc-exception-driver", wasm, workload, f"i32:{n}",
                            "decoded", "all-phase3-core"])
        oracle = gc_workloads.oracle(workload, n)
        if isinstance(oracle, int):
            self.assertEqual(row["result"], f"i32:{oracle}")
        else:
            self.assertEqual(row["outcome"], oracle["kind"])
            self.assertEqual(row["tag"], oracle["tag"])
            self.assertEqual(row["payload"], [f"i32:{value}" for value in oracle["payload"]])
        return row

    def test_gc_checksums_collection_and_root_release(self):
        with tempfile.TemporaryDirectory() as temp:
            packed = self.run_case(temp, "packed_array.wat", "gc_packed_array_n32", 32)
            self.assertEqual(packed["result"], "i32:-96")
            self.assertEqual((packed["collection"]["live"], packed["collection"]["reclaimed"]), (0, 1))
            self.assertEqual(packed["gc_payload_bytes"]["after_collect"], 0)

            graph = self.run_case(temp, "struct_graph.wat", "gc_struct_graph_n32", 32)
            self.assertEqual(graph["result"], "i32:496")
            self.assertEqual(graph["collection"]["reclaimed"], 32)

            roots = self.run_case(temp, "root_lifecycle.wat", "gc_root_lifecycle_n32", 32)
            self.assertEqual(roots["result"], "i32:496")
            self.assertEqual(roots["root_category"], "host_value_root")
            self.assertEqual((roots["rooted_collection"]["live"], roots["rooted_collection"]["reclaimed"]), (1, 0))
            self.assertEqual((roots["released_collection"]["live"], roots["released_collection"]["reclaimed"]), (0, 1))
            self.assertGreater(roots["gc_payload_bytes"]["after_rooted_collect"], 0)
            self.assertEqual(roots["gc_payload_bytes"]["after_release"], 0)

    def test_reentrant_collect_and_descriptor_cross_module(self):
        with tempfile.TemporaryDirectory() as temp:
            host = self.run_case(temp, "host_reentry_collect.wat", "gc_host_reentry_collect_n32", 32)
            self.assertEqual(host["result"], "i32:32")
            self.assertEqual((host["host_collection"]["live"], host["host_collection"]["reclaimed"]), (1, 0))
            self.assertEqual(host["collection"]["reclaimed"], 3)
            self.assertEqual(host["gc_payload_bytes"]["after_collect"], 0)

            provider = self.encode(temp, "descriptor_provider.wat")
            consumer = self.encode(temp, "descriptor_consumer.wat")
            row = self.observe(["--gc-exception-cross-module", provider, consumer,
                                "i32:32", "decoded", "all-phase3-core"])
            self.assertEqual(row["result"], "i32:32")
            self.assertEqual(row["normalized_provider_object"], "descriptor_child_graph")
            self.assertEqual((row["collection"]["live"], row["collection"]["reclaimed"]), (0, 3))
            self.assertEqual(row["gc_payload_bytes"]["after_collect"], 0)

    def test_nominal_function_reference_and_continuation_roots(self):
        with tempfile.TemporaryDirectory() as temp:
            casts = self.run_case(temp, "nominal_exact_casts.wat", "gc_nominal_exact_casts_n32", 32)
            self.assertEqual(casts["result"], "i32:64")
            self.assertEqual(casts["collection"]["reclaimed"], 32)

            refs = self.run_case(temp, "typed_function_refs.wat", "gc_typed_function_refs_n32", 32)
            self.assertEqual(refs["result"], "i32:528")
            self.assertEqual(refs["collection"]["reclaimed"], 0)

            continuation = self.run_case(temp, "continuation_root.wat", "gc_continuation_root_n32", 32)
            self.assertEqual(continuation["result"], "i32:32")
            self.assertGreaterEqual(continuation["host_collection"]["live"], 3)
            self.assertEqual(continuation["host_collection"]["reclaimed"], 1)
            self.assertEqual(continuation["collection"]["reclaimed"], 3)
            self.assertEqual(continuation["continuation_bytes"]["after_collect"], 0)

    def test_caught_normal_host_and_uncaught_exception_outcomes(self):
        with tempfile.TemporaryDirectory() as temp:
            cases = [
                ("caught_throw.wat", "exception_caught_throw_n32", "i32:32"),
                ("normal_path.wat", "exception_normal_path_n32", "i32:32"),
                ("cross_call_catch.wat", "exception_cross_call_catch_n32", "i32:32"),
                ("host_throw_catch.wat", "exception_host_throw_catch_n32", "i32:32"),
                ("throw_ref_unwind.wat", "exception_throw_ref_unwind_n32", "i32:32"),
            ]
            for filename, workload, expected in cases:
                with self.subTest(workload=workload):
                    row = self.run_case(temp, filename, workload, 32)
                    self.assertEqual(row.get("outcome", "returned"), "returned")
                    self.assertEqual(row["result"], expected)

            uncaught = self.run_case(temp, "uncaught_throw.wat", "exception_uncaught_throw_n32", 32)
            oracle = gc_workloads.oracle("exception_uncaught_throw_n32", 32)
            self.assertEqual(uncaught["outcome"], oracle["kind"])
            self.assertEqual(uncaught["tag"], oracle["tag"])
            self.assertEqual(uncaught["payload"], [f"i32:{value}" for value in oracle["payload"]])

    def test_all_45_registered_cases_run_against_their_independent_oracle(self):
        entries = gc_workloads.entries()
        self.assertEqual(len(entries), 45)
        with tempfile.TemporaryDirectory() as temp:
            encoded = {
                filename: self.encode(temp, filename)
                for filename in gc_workloads.WASM_SHA256
            }
            for entry in entries:
                workload = entry["id"]
                n = entry["params"][0]["baseline"]
                if entry["family"] == "gc_descriptor_cross_module":
                    row = self.observe([
                        "--gc-exception-cross-module",
                        encoded["descriptor_provider.wat"],
                        encoded["descriptor_consumer.wat"],
                        f"i32:{n}", "decoded", "all-phase3-core",
                    ])
                else:
                    row = self.observe([
                        "--gc-exception-driver",
                        encoded[Path(entry["source"]["path"]).name],
                        workload, f"i32:{n}", "decoded", "all-phase3-core",
                    ])
                oracle = gc_workloads.oracle(workload, n)
                with self.subTest(workload=workload):
                    expected_case = (
                        "gc_descriptor_cross_module"
                        if entry["family"] == "gc_descriptor_cross_module"
                        else workload
                    )
                    self.assertEqual(row["case"], expected_case)
                    self.assertEqual(row["input"], f"i32:{n}")
                    self.assertEqual(row["mode"], "decoded")
                    self.assertEqual(row["profile"], "all-phase3-core")
                    if isinstance(oracle, int):
                        self.assertEqual(row.get("outcome", "returned"), "returned")
                        self.assertEqual(row["result"], f"i32:{oracle}")
                    else:
                        self.assertEqual(row["outcome"], oracle["kind"])
                        self.assertEqual(row["tag"], oracle["tag"])
                        self.assertEqual(
                            row["payload"],
                            [f"i32:{value}" for value in oracle["payload"]],
                        )


if __name__ == "__main__":
    unittest.main()
