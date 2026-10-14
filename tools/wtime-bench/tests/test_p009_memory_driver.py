"""Subprocess checks for the public-API P009 memory observation driver.

Set WTIME_BENCH_MEMORY_DRIVER to a built current-source benchmark binary. The
tests still validate source and fixture hashes before accepting observations.
"""

import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parents[1]
ROOT = HERE.parents[1]
BENCH = os.environ.get("WTIME_BENCH_MEMORY_DRIVER")
ENCODER = os.environ.get(
    "WTIME_CONFORMANCE_ENCODER", str(ROOT / "target/debug/wtime-conformance")
)
sys.path.insert(0, str(HERE))
import memory_workloads


@unittest.skipUnless(
    BENCH and Path(BENCH).is_file(),
    "set WTIME_BENCH_MEMORY_DRIVER to a built current-source protocol binary",
)
class MemoryDriverSubprocessTests(unittest.TestCase):
    def run_case(self, temp, workload, fixture, input_value, expected, profile, driver):
        wat = HERE / "fixtures/memory" / f"{fixture}.wat"
        wasm = Path(temp) / f"{fixture}.wasm"
        encoded = subprocess.run(
            [ENCODER, "encode", str(wat), "-o", str(wasm)],
            cwd=ROOT,
            text=True,
            capture_output=True,
        )
        self.assertEqual(encoded.returncode, 0, encoded.stderr)
        self.assertEqual(
            hashlib.sha256(wasm.read_bytes()).hexdigest(),
            memory_workloads.WASM_SHA256[f"{fixture}.wat"],
        )
        expected_outcome = "trap" if expected == "trap" else f"i32:{expected}"
        result = subprocess.run(
            [
                BENCH,
                "--memory-driver",
                str(wasm),
                workload,
                f"i32:{input_value}",
                expected_outcome,
                "decoded",
                profile,
                driver,
            ],
            cwd=ROOT,
            text=True,
            capture_output=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        row = json.loads(result.stdout)
        oracle = memory_workloads.expected_memory(workload, input_value)
        self.assertEqual(row["schema_version"], 2)
        self.assertEqual(row["case"], workload)
        self.assertEqual(row["stage"], "load_and_call")
        self.assertEqual(row["input"], f"i32:{input_value}")
        self.assertEqual(row["expected"], expected_outcome)
        self.assertEqual(row["result"], expected_outcome)
        self.assertEqual(row["batch"], 1)
        self.assertGreater(row["ns_per_operation"], 0)
        self.assertEqual(row["driver"], driver)
        memory = bytes.fromhex(row["memory_hex"])
        page_bytes = 1 << row["memory_page_size_log2"]
        self.assertLessEqual(len(memory), 17 * memory_workloads.PAGE)
        self.assertEqual(len(memory), row["memory_pages"] * page_bytes)
        self.assertEqual(
            hashlib.sha256(memory).hexdigest(), oracle["full_memory_sha256"]
        )
        if "bytes_hex" in oracle:
            start, end = oracle.get(
                "range", (0, len(bytes.fromhex(oracle["bytes_hex"])))
            )
            self.assertEqual(memory[start:end], bytes.fromhex(oracle["bytes_hex"]))
        return row, oracle, memory

    def test_memory_driver_subprocess_observations(self):
        with tempfile.TemporaryDirectory() as temp:
            with self.subTest(case="growth"):
                row, oracle, memory = self.run_case(
                    temp,
                    "memory_growth_success_one",
                    "growth",
                    1,
                    memory_workloads.oracle("memory_growth_success_one", 1),
                    "mvp",
                    "plain",
                )
                self.assertEqual(row["memory_pages"], oracle["pages_after"])
                self.assertEqual(memory, bytes(2 * memory_workloads.PAGE))
            with self.subTest(case="oob_order"):
                row, oracle, memory = self.run_case(
                    temp,
                    "memory_oob_copy_order",
                    "oob_order",
                    0,
                    "trap",
                    "all-phase3-core",
                    "plain",
                )
                self.assertIn("MemoryOutOfBounds", row["trap_kind"])
                self.assertEqual(memory[8:10], b"\xaa\0")
                self.assertEqual(memory[:4], b"\x01\x02\x03\x04")
            with self.subTest(case="aliased_copy"):
                row, oracle, memory = self.run_case(
                    temp,
                    "memory_copy_aliased_64",
                    "bulk_copy_aliased",
                    64,
                    memory_workloads.oracle("memory_copy_aliased_64", 64),
                    "all-phase3-core",
                    "alias",
                )
                self.assertTrue(oracle["alias_memories"])
                self.assertEqual(row["memory_pages"], 17)
                self.assertEqual(
                    memory[:65], bytes.fromhex(oracle["bytes_hex"])
                )
            with self.subTest(case="custom_page_one_byte"):
                row, oracle, memory = self.run_case(
                    temp,
                    "memory_custom_page_one_byte",
                    "custom_page_one_byte",
                    255,
                    memory_workloads.oracle("memory_custom_page_one_byte", 255),
                    "all-phase3-core",
                    "plain",
                )
                self.assertEqual(row["memory_pages"], 256)
                self.assertEqual(row["memory_page_size_log2"], 0)
                self.assertEqual(len(memory), 256)
                self.assertEqual(memory[-1], 0x80)
            with self.subTest(case="multi_memory_second_export"):
                row, oracle, memory = self.run_case(
                    temp,
                    "memory_multi_memory_second_export",
                    "multi_memory_second",
                    0,
                    memory_workloads.oracle("memory_multi_memory_second_export", 0),
                    "all-phase3-core",
                    "multi",
                )
                self.assertEqual(row["memory_count"], 2)
                self.assertEqual(row["exported_memory_index"], 1)
                self.assertEqual(len(row["all_memory_hex"]), 2)
                all_memories = [bytes.fromhex(item) for item in row["all_memory_hex"]]
                self.assertEqual(
                    [hashlib.sha256(item).hexdigest() for item in all_memories],
                    oracle["all_memory_sha256"],
                )
                self.assertEqual(memory, all_memories[1])
                self.assertEqual(all_memories[0][:2], b"\x11\x33")
                self.assertEqual(all_memories[1][:2], b"\x22\x44")


if __name__ == "__main__":
    unittest.main()
