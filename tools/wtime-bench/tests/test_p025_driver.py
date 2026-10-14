"""Actual staged P025 memory-driver subprocess checks.

Set WTIME_BENCH_MEMORY_DRIVER to the built default benchmark binary. Fixture
encoding uses the existing conformance encoder and validates the pinned Wasm
hash before any driver call.
"""

import copy
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
ENCODER = Path(os.environ.get(
    "WTIME_CONFORMANCE_ENCODER", str(ROOT / "target/debug/wtime-conformance")
))
sys.path.insert(0, str(HERE))

import memory_scaling
import memory_workloads


STAGES = ("prepare", "instantiate", "first_call", "warm_call", "load_and_call")
REPEATABLE = (
    ("memory_widths_predictable", 64, "widths", "mvp"),
    ("memory_widths_shuffled", 64, "widths_shuffled", "mvp"),
    ("memory_widths_read_scan", 1024, "read_scan", "mvp"),
)


@unittest.skipUnless(
    BENCH and Path(BENCH).is_file() and ENCODER.is_file(),
    "set WTIME_BENCH_MEMORY_DRIVER and provide WTIME_CONFORMANCE_ENCODER",
)
class StagedMemoryDriverSubprocessTests(unittest.TestCase):
    def encode(self, output_dir: Path, fixture: str) -> Path:
        wat = HERE / "fixtures" / "memory" / f"{fixture}.wat"
        wasm = output_dir / f"{fixture}.wasm"
        encoded = subprocess.run(
            [str(ENCODER), "encode", str(wat), "-o", str(wasm)],
            cwd=ROOT,
            text=True,
            capture_output=True,
            timeout=60,
        )
        self.assertEqual(encoded.returncode, 0, encoded.stderr)
        digest = hashlib.sha256(wasm.read_bytes()).hexdigest()
        self.assertEqual(digest, memory_workloads.WASM_SHA256[f"{fixture}.wat"])
        return wasm

    def observe(self, wasm: Path, workload: str, value: int, expected: dict,
                stage: str, batch: int, profile: str = "mvp") -> dict:
        outcome = memory_scaling._expected_outcome(expected)
        completed = subprocess.run(
            [
                str(BENCH), "--memory-staged", str(wasm), workload,
                f"i32:{value}", outcome, stage, str(batch), "decoded", profile,
                "plain",
            ],
            cwd=ROOT,
            text=True,
            capture_output=True,
            timeout=180,
        )
        self.assertEqual(completed.returncode, 0, completed.stderr)
        try:
            row = json.loads(completed.stdout)
        except json.JSONDecodeError as exc:
            self.fail(f"staged driver returned invalid JSON: {exc}: {completed.stdout[:500]!r}")
        memory_scaling._validate_observation(
            row,
            {"id": workload},
            value,
            expected,
            "decoded",
            profile,
            "plain",
            stage=stage,
            expected_batch=None if stage == "warm_call" and batch == 0 else batch,
        )
        self.assertGreater(row["ns_per_operation"], 0)
        if stage == "warm_call" and batch == 0:
            self.assertGreaterEqual(row["batch"], 1)
            self.assertLessEqual(row["batch"], 100_000)
        return row

    def _check_all_stages(self, workload: str, value: int, fixture: str,
                          profile: str = "mvp") -> None:
        expected = memory_workloads.expected_memory(workload, value)
        self.assertEqual(expected.get("result"), memory_workloads.oracle(workload, value))
        with tempfile.TemporaryDirectory(prefix="p025-memory-driver-") as temp:
            wasm = self.encode(Path(temp), fixture)
            for stage in STAGES:
                with self.subTest(workload=workload, stage=stage):
                    requested_batch = 4 if stage == "warm_call" else 1
                    row = self.observe(wasm, workload, value, expected, stage,
                                       requested_batch, profile)
                    self.assertEqual(row["batch"], requested_batch)
                    if stage == "warm_call":
                        self.assertEqual(
                            hashlib.sha256(bytes.fromhex(row["memory_hex"])).hexdigest(),
                            expected["full_memory_sha256"],
                        )

    def test_predictable_widths_full_memory_matches_in_every_stage(self):
        self._check_all_stages(*REPEATABLE[0])

    def test_shuffled_widths_full_memory_matches_in_every_stage(self):
        self._check_all_stages(*REPEATABLE[1])

    def test_read_scan_full_memory_matches_in_every_stage(self):
        self._check_all_stages(*REPEATABLE[2])

    def test_calibrated_warm_batch_is_bounded_and_memory_checked(self):
        workload, value, fixture, profile = REPEATABLE[2]
        expected = memory_workloads.expected_memory(workload, value)
        with tempfile.TemporaryDirectory(prefix="p025-memory-calibration-") as temp:
            wasm = self.encode(Path(temp), fixture)
            row = self.observe(wasm, workload, value, expected, "warm_call", 0, profile)
            self.assertGreaterEqual(row["batch"], 1)
            self.assertLessEqual(row["batch"], 100_000)

    def test_growth_and_trapping_fixtures_reject_warm_repetition(self):
        for workload, value, fixture, profile in (
            ("memory_growth_success_one", 1, "growth", "mvp"),
            ("memory_oob_copy_order", 0, "oob_order", "all-phase3-core"),
        ):
            entry = next(item for item in memory_workloads.entries() if item["id"] == workload)
            expected = memory_workloads.expected_memory(workload, value)
            outcome = memory_scaling._expected_outcome(expected)
            with self.subTest(workload=workload), tempfile.TemporaryDirectory() as temp:
                wasm = self.encode(Path(temp), fixture)
                result = subprocess.run(
                    [str(BENCH), "--memory-staged", str(wasm), workload,
                     f"i32:{value}", outcome, "warm_call", "4", "decoded",
                     profile, "plain"],
                    cwd=ROOT,
                    text=True,
                    capture_output=True,
                    timeout=60,
                )
                self.assertNotEqual(result.returncode, 0)
                self.assertTrue(result.stderr or result.stdout)

    def test_validator_rejects_nonunit_batch_for_cold_stage(self):
        workload, value, fixture, profile = REPEATABLE[0]
        expected = memory_workloads.expected_memory(workload, value)
        with tempfile.TemporaryDirectory(prefix="p025-memory-cold-batch-") as temp:
            wasm = self.encode(Path(temp), fixture)
            row = self.observe(wasm, workload, value, expected, "first_call", 1, profile)
            invalid = copy.deepcopy(row)
            invalid["batch"] = 4
            with self.assertRaisesRegex(memory_scaling.BaselineError, "batch"):
                memory_scaling._validate_observation(
                    invalid,
                    {"id": workload},
                    value,
                    expected,
                    "decoded",
                    profile,
                    "plain",
                    stage="first_call",
                    expected_batch=None,
                )


if __name__ == "__main__":
    unittest.main()
