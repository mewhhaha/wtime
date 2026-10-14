"""Fail-closed checks for the P010 SIMD observation runner."""
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import simd_scaling as runner


class P010RunnerValidationTests(unittest.TestCase):
    def setUp(self):
        self.entry = {"id": "simd_case", "memory_expectation": None}
        self.executable = Path("/tmp/candidate")
        self.wasm = Path("/tmp/case.wasm")

    def protocol_row(self, **overrides):
        row = {"schema_version": 2, "case": "simd_case", "stage": "first_call",
               "input": "i32:17", "expected": "i32:7", "result": "i32:7",
               "batch": 1, "ns_per_operation": 123.5, "mode": "decoded",
               "profile": "simd"}
        row.update(overrides)
        return mock.Mock(returncode=0, stdout=json.dumps(row), stderr="")

    def call(self):
        return runner._call_candidate(self.executable, self.wasm, self.entry, "i32:17",
                                      ["i32:7"], "first_call", "simd", 1.0)

    def test_protocol_identity_and_input_mismatch_fail_closed(self):
        with mock.patch.object(runner.subprocess, "run",
                               return_value=self.protocol_row(input="i32:18")):
            with self.assertRaisesRegex(runner.MeasurementError, "input/settings mismatch"):
                self.call()
        with mock.patch.object(runner.subprocess, "run",
                               return_value=self.protocol_row(case="other_case")):
            with self.assertRaisesRegex(runner.MeasurementError, "protocol identity mismatch"):
                self.call()

    def test_zero_or_nonfinite_timing_is_rejected(self):
        for value in (0.0, float("nan"), float("inf")):
            with self.subTest(value=value), mock.patch.object(
                    runner.subprocess, "run", return_value=self.protocol_row(ns_per_operation=value)):
                with self.assertRaisesRegex(runner.MeasurementError, "timing must be finite and positive"):
                    self.call()

    def test_result_outside_oracle_allowed_set_is_rejected(self):
        with mock.patch.object(runner.subprocess, "run",
                               return_value=self.protocol_row(result="i32:8")):
            with self.assertRaisesRegex(runner.MeasurementError, "outside independent oracle set"):
                self.call()

    def test_source_and_binary_mutation_is_rejected(self):
        before = {"/candidate": "a" * 64, "/model.py": "b" * 64}
        unchanged = dict(before)
        runner._assert_stable_sources(before, unchanged)
        with self.assertRaisesRegex(runner.MeasurementError, "identity changed"):
            runner._assert_stable_sources(before, {**before, "/candidate": "c" * 64})
        with self.assertRaisesRegex(runner.MeasurementError, "identity changed"):
            runner._assert_stable_sources(before, {"/model.py": "b" * 64})

    def test_profiles_enable_combined_simd_saturating_and_relaxed_features(self):
        self.assertEqual(runner.execution_profile({"required_features": ["mvp", "simd"]}), "simd")
        self.assertEqual(runner.execution_profile(
            {"required_features": ["mvp", "simd", "saturating_float_to_int"]}), "simd")
        self.assertEqual(runner.execution_profile(
            {"required_features": ["mvp", "saturating_float_to_int"]}), "mvp-saturating")
        self.assertEqual(runner.execution_profile(
            {"required_features": ["mvp", "simd", "relaxed_simd"]}), "simd-relaxed")

    def test_source_discovery_binds_root_runtime_and_all_benchmark_rust_leaves(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for relative in (
                "Cargo.toml", "Cargo.lock", "tools/wtime-bench/Cargo.toml",
                "tools/wtime-bench/Cargo.lock", "crates/runtime/src/lib.rs",
                "tools/wtime-bench/src/main.rs", "tools/wtime-bench/tests/protocol.rs",
            ):
                path = root / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("source identity fixture\n")
            paths = runner._repository_source_inputs(root)
            names = {str(path.relative_to(root)) for path in paths}
            self.assertTrue({
                "Cargo.toml", "Cargo.lock", "tools/wtime-bench/Cargo.toml",
                "tools/wtime-bench/Cargo.lock", "crates/runtime/src/lib.rs",
                "tools/wtime-bench/src/main.rs", "tools/wtime-bench/tests/protocol.rs",
            }.issubset(names))

    def test_current_model_encodes_all_20_fixtures_with_pinned_hashes(self):
        import os
        import simd_workloads as model
        encoder = os.environ.get("WTIME_CONFORMANCE_ENCODER")
        self.assertTrue(encoder and Path(encoder).is_file(), "set WTIME_CONFORMANCE_ENCODER")
        entries = model.entries()
        self.assertEqual(len(entries), 114)
        with tempfile.TemporaryDirectory() as directory:
            model.encode_fixtures(encoder, directory)
            fixtures = {entry["source"]["path"]: runner.fixture_for(
                entry, Path(model.__file__), Path(directory)) for entry in entries}
            self.assertEqual(len(fixtures), 20)


if __name__ == "__main__":
    unittest.main()
