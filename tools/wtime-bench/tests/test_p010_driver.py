"""Actual-process checks for the typed SIMD protocol and memory snapshots.

Set WTIME_BENCH_SIMD_DRIVER to a built current-source benchmark executable.
The existing conformance encoder is used to bind each run to the recorded Wasm
fixture bytes; these checks do not build the benchmark.
"""

from __future__ import annotations

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
BENCH = os.environ.get("WTIME_BENCH_SIMD_DRIVER")
ENCODER = os.environ.get(
    "WTIME_CONFORMANCE_ENCODER", str(ROOT / "target/debug/wtime-conformance")
)
sys.path.insert(0, str(HERE))

import simd_workloads
import simd_scaling


@unittest.skipUnless(
    BENCH and Path(BENCH).is_file(),
    "set WTIME_BENCH_SIMD_DRIVER to a built current-source protocol binary",
)
class SimdDriverSubprocessTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temp = tempfile.TemporaryDirectory()
        cls.wasm_paths = {
            name: Path(cls.temp.name) / (Path(name).stem + ".wasm")
            for name in simd_workloads.WASM_SHA256
        }
        actual_hashes = simd_workloads.encode_fixtures(ENCODER, cls.temp.name)
        cls.assert_hashes = actual_hashes
        for name, expected in simd_workloads.WASM_SHA256.items():
            if expected is not None:
                if actual_hashes[name] != expected:
                    raise AssertionError(f"encoded Wasm hash changed for {name}")
        cls.entries = {entry["id"]: entry for entry in simd_workloads.entries()}

    @classmethod
    def tearDownClass(cls):
        cls.temp.cleanup()

    def typed_input(self, workload_id: str) -> str:
        entry = self.entries[workload_id]
        values = []
        for param in entry["params"]:
            value = param.get("value", param.get("baseline", param.get("min")))
            if value is None:
                raise AssertionError(f"no fixed typed input for {workload_id}: {param}")
            values.append(f'{param["type"]}:{value}')
        return ";".join(values) if values else "-"

    def profile(self, workload_id: str) -> str:
        features = set(self.entries[workload_id]["required_features"])
        if "relaxed_simd" in features:
            return "simd-relaxed"
        if "simd" in features:
            return "simd"
        if "saturating_float_to_int" in features:
            return "mvp-saturating"
        return "mvp"

    def call(self, workload_id: str, *, stage="first_call", profile=None,
             flag="--typed-case", expected=None, input_value=None):
        entry = self.entries[workload_id]
        if expected is None:
            if "expected_results" in entry:
                expected = "|".join(f'i32:{value}' for value in entry["expected_results"])
            else:
                oracle_args = entry.get("oracle_arguments", {})
                oracle_value = oracle_args.get("n", oracle_args.get("vectors"))
                if oracle_value is None:
                    for param in entry["params"]:
                        oracle_value = param.get(
                            "value", param.get("baseline", param.get("min"))
                        )
                        if oracle_value is not None:
                            break
                expected = f"i32:{simd_workloads.oracle(workload_id, oracle_value)}"
        if input_value is None:
            input_value = self.typed_input(workload_id)
        fixture = Path(entry["source"]["path"]).name
        command = [
            BENCH,
            flag,
            str(self.wasm_paths[fixture]),
            workload_id,
            input_value,
            expected,
            stage,
            "1",
            "decoded",
            profile or self.profile(workload_id),
        ]
        return subprocess.run(command, cwd=ROOT, text=True, capture_output=True)

    def assert_row(self, result, workload_id, stage, input_value, expected,
                   profile=None):
        self.assertEqual(result.returncode, 0, result.stderr)
        row = json.loads(result.stdout)
        self.assertEqual(row["schema_version"], 2)
        self.assertEqual(row["case"], workload_id)
        self.assertEqual(row["stage"], stage)
        self.assertEqual(row["input"], input_value)
        self.assertEqual(row["expected"], expected)
        if "|" in expected:
            self.assertIn(row["result"], expected.split("|"))
        else:
            self.assertEqual(row["result"], expected)
        self.assertEqual(row["batch"], 1)
        self.assertGreater(row["ns_per_operation"], 0)
        self.assertEqual(row["mode"], "decoded")
        self.assertEqual(row["profile"], profile or self.profile(workload_id))
        return row

    def encode_hashes_match_registry(self):
        """Keep the run tied to the exact sources in the registry."""
        for entry in self.entries.values():
            source = HERE / entry["source"]["path"]
            self.assertEqual(
                hashlib.sha256(source.read_bytes()).hexdigest(),
                entry["source"]["sha256"],
            )
            self.assertEqual(
                self.assert_hashes[Path(source).name], entry["source"]["wasm_sha256"]
            )

    def test_seeded_vector_and_scalar_inputs_match_independent_checksums(self):
        self.encode_hashes_match_registry()
        for suffix, _lanes in simd_workloads.randomized_lane_inputs():
            vector_id = f"simd_integer_lanes_random_{suffix}"
            scalar_id = f"simd_scalar_integer_lanes_random_{suffix}"
            expected_value = simd_workloads.oracle(vector_id)
            self.assertEqual(expected_value, simd_workloads.oracle(scalar_id))
            for workload_id in (vector_id, scalar_id):
                with self.subTest(workload=workload_id):
                    encoded = f"i32:{expected_value}"
                    row = self.assert_row(
                        self.call(workload_id), workload_id, "first_call",
                        self.typed_input(workload_id), encoded, self.profile(workload_id),
                    )
                    self.assertEqual(row["result"], encoded)

    def test_relaxed_oob_swizzle_accepts_only_recorded_alternatives(self):
        workload_id = "simd_relaxed_swizzle_oob_17"
        accepted = simd_workloads.allowed_results(workload_id, 17)
        expected = "|".join(f"i32:{value}" for value in sorted(accepted))
        row = self.assert_row(
            self.call(workload_id, flag="--typed-allowed-case"), workload_id,
            "first_call", self.typed_input(workload_id), expected,
        )
        self.assertIn(row["result"], {f"i32:{value}" for value in accepted})
        rejected = self.call(
            workload_id, flag="--typed-allowed-case", expected="i32:999"
        )
        self.assertNotEqual(rejected.returncode, 0, "a false relaxed oracle was accepted")

    def test_minimal_profiles_reject_missing_simd_and_relaxed_features(self):
        simd = self.call("simd_shuffle_n17", profile="mvp-saturating")
        self.assertNotEqual(simd.returncode, 0)
        self.assertIn("DisabledFeature", simd.stderr + simd.stdout)

        relaxed = self.call(
            "simd_relaxed_swizzle_oob_17",
            profile="simd",
            flag="--typed-allowed-case",
        )
        self.assertNotEqual(relaxed.returncode, 0)
        self.assertIn("DisabledFeature", relaxed.stderr + relaxed.stdout)

    def test_all_stages_keep_the_16_byte_memory_oracle(self):
        workload_id = "simd_lane_memory_footprint_16"
        entry = self.entries[workload_id]
        n = entry["params"][0]["value"]
        expected_value = simd_workloads.oracle(workload_id, n)
        oracle = simd_workloads.expected_memory(workload_id, n)
        expected = f"i32:{expected_value}"
        for stage in simd_workloads.STAGES:
            with self.subTest(stage=stage):
                row = self.assert_row(
                    self.call(workload_id, stage=stage, flag="--simd-memory-case"),
                    workload_id, stage, f"i32:{n}", expected,
                )
                memory = bytes.fromhex(row["memory_hex"])
                self.assertEqual(row["memory_pages"], oracle["page_count"])
                self.assertEqual(row["memory_page_size_log2"], 16)
                self.assertEqual(len(memory), 65536)
                self.assertEqual(
                    hashlib.sha256(memory).hexdigest(), oracle["full_memory_sha256"]
                )
                start, end = oracle["range"]
                self.assertEqual(
                    hashlib.sha256(memory[start:end]).hexdigest(), oracle["sha256"]
                )
                self.assertEqual(memory[start:end], bytes.fromhex("02000000040000000600000008000000"))
                # The prepare row intentionally reports the untimed preflight image.
                if stage == "prepare":
                    self.assertEqual(row["result"], expected)

    def test_full_64kib_memory_footprint_matches_full_image_digest(self):
        workload_id = "simd_lane_memory_footprint_65536"
        entry = self.entries[workload_id]
        n = entry["params"][0]["value"]
        oracle = simd_workloads.expected_memory(workload_id, n)
        expected = f'i32:{simd_workloads.oracle(workload_id, n)}'
        row = self.assert_row(
            self.call(workload_id, stage="load_and_call", flag="--simd-memory-case"),
            workload_id, "load_and_call", f"i32:{n}", expected,
        )
        memory = bytes.fromhex(row["memory_hex"])
        self.assertEqual(len(memory), 65536)
        self.assertEqual(hashlib.sha256(memory).hexdigest(), oracle["full_memory_sha256"])
        self.assertEqual(hashlib.sha256(memory[:65536]).hexdigest(), oracle["sha256"])

    def test_nan_signed_zero_and_f64_bit_input_controls(self):
        cases = (
            "simd_float_min_zero",
            "simd_float_max_zero",
            "simd_float_nan_classify",
            "simd_scalar_float_nan_classify",
            "simd_f64_min_negative_zero",
            "simd_f64_min_positive_zero",
            "simd_f64_min_nan",
            "simd_typed_f64_neg_1_75",
            "simd_typed_f64_pos_3_5",
        )
        for workload_id in cases:
            with self.subTest(workload=workload_id):
                value = simd_workloads.oracle(workload_id)
                expected = f"i32:{value}"
                input_value = self.typed_input(workload_id)
                if "f64_bits:" in input_value:
                    self.assertRegex(input_value, r"(?:^|;)f64_bits:[0-9]+(?:;|$)")
                row = self.assert_row(
                    self.call(workload_id), workload_id, "first_call", input_value,
                    expected,
                )
                self.assertEqual(row["result"], expected)

    def test_one_actual_call_per_fixture_uses_runner_profile_and_oracle(self):
        by_fixture = {}
        for entry in self.entries.values():
            by_fixture.setdefault(Path(entry["source"]["path"]).name, entry)
        self.assertEqual(len(by_fixture), len(simd_workloads.WASM_SHA256))

        for fixture, entry in by_fixture.items():
            workload_id = entry["id"]
            profile = simd_scaling.execution_profile(entry)
            input_value = simd_scaling.typed_input(entry)
            allowed = simd_scaling.allowed_wire_results(entry, simd_workloads)
            flag = (
                "--simd-memory-case" if "memory_expectation" in entry
                else "--typed-allowed-case" if len(allowed) > 1
                else "--typed-case"
            )
            expected = "|".join(allowed)
            with self.subTest(fixture=fixture, workload=workload_id, profile=profile):
                row = self.assert_row(
                    self.call(
                        workload_id, stage="first_call", profile=profile,
                        flag=flag, expected=expected, input_value=input_value,
                    ),
                    workload_id, "first_call", input_value, expected, profile,
                )
                self.assertIn(row["result"], allowed)


if __name__ == "__main__":
    unittest.main()
