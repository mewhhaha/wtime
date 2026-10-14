"""P122 mode forwarding and staged representative protocol checks.

Synthetic files live in a fresh temporary repository-shaped root.  Set
WTIME_P122_BENCH_BIN to a release binary built with compact-executor to run the
actual-driver probe as well as the source and wire checks.
"""
from __future__ import annotations

import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True

BENCH = Path(__file__).resolve().parents[1]


def load(path: Path, name: str):
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise AssertionError(f"cannot load {path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


class P122RepresentativeProtocolTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="p122-representative-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.tool = self.root / "tools" / "wtime-bench"
        self.fixtures = self.tool / "fixtures" / "representative"
        self.fixtures.mkdir(parents=True)
        for name in ("representative_compare.py", "representative_scaling.py",
                     "representative_workloads.py"):
            source = BENCH / name
            target = self.tool / name
            shutil.copyfile(source, target)
            self.assert_inside_root(target)
        for source in (BENCH / "fixtures" / "representative").glob("*"):
            if source.suffix in (".wat", ".wasm"):
                target = self.fixtures / source.name
                shutil.copyfile(source, target)
                self.assert_inside_root(target)
        self.compare = load(self.tool / "representative_compare.py", "p122_compare")
        self.scaling = load(self.tool / "representative_scaling.py", "p122_scaling")
        self.model = load(self.tool / "representative_workloads.py", "p122_model")
        self.entry = self.model.entries()[0]
        self.size = self.entry["params"][0]["baseline"]
        self.answer = self.model.oracle(self.entry["id"], self.size)
        self.image = self.model.expected_memory_image(self.entry["id"], self.size)

    def assert_inside_root(self, path: Path):
        path.resolve().relative_to(self.root)

    def wire(self, *, mode="compact", stage="run_after_setup", instrumented=False):
        return {
            "schema_version": 2, "case": self.entry["id"], "stage": stage,
            "input": str(self.size), "expected": f"i32:{self.answer['result']}",
            "result": f"i32:{self.answer['result']}", "batch": 3,
            "calls_checked": 3, "report_complete": True,
            "setup_reset_before_each_call": True, "setup_excluded_from_timing": True,
            "instrumented": instrumented, "measured_guest_ns": 1200.0,
            "ns_per_operation": 400.0, "mode": mode, "profile": "mvp",
            "memory_pages": 1, "memory_page_size_log2": 16,
            "memory_hex": self.image.hex(),
            "expected_output_sha256": self.answer["output_sha256"],
        }

    def validate(self, wire, expected_mode="compact", diagnostic=False):
        return self.compare.validate_observation(
            self.entry, wire, self.model, self.scaling, 3,
            expected_mode=expected_mode, diagnostic=diagnostic,
        )

    def test_p122_compact_staged_wire_is_accepted_with_full_memory_proof(self):
        observed = self.wire()
        proof = self.validate(observed)
        self.assertEqual(proof["validated_memory_proof"]["full_memory_sha256"],
                         self.entry["expected_full_memory_sha256"])
        self.assertIs(proof["raw_wire_observation"], observed)
        # The old P015 call shape still defaults to Decoded for callers that
        # validate a legacy single observation.
        decoded = self.wire(mode="decoded")
        self.assertEqual(self.compare.validate_observation(
            self.entry, decoded, self.model, self.scaling, 3
        )["mode"], "decoded")

    def test_p122_rejects_wrong_mode_stage_and_instrumentation(self):
        for bad in (self.wire(mode="decoded"),
                    self.wire(stage="prepare"),
                    self.wire(instrumented=True)):
            with self.subTest(mode=bad["mode"], stage=bad["stage"],
                              instrumented=bad["instrumented"]):
                with self.assertRaises((self.compare.RepresentativeComparisonError,
                                        self.scaling.RepresentativeError)):
                    self.validate(bad)

    def test_p122_host_timer_boundary_and_production_feature_guard(self):
        driver = (BENCH / "src" / "representative_driver.rs").read_text()
        self.assertIn('matches!(mode.as_str(), "decoded" | "compact")', driver)
        self.assertIn('!cfg!(feature = "performance-counters") && !cfg!(feature = "execution-reference")',
                      driver)
        start = driver.index("let start = Instant::now();", driver.index("fn execute_staged("))
        reset = driver.index("invoke_prepare(&mut store, prepare, size);", driver.index("fn execute_staged("))
        invoke = driver.index("let current = invoke_i32(&mut store, run, size);", start)
        elapsed = driver.index("guest_ns += start.elapsed().as_nanos()", invoke)
        oracle = driver.index('assert_eq!(current, expected', elapsed)
        snapshot = driver.index("final_snapshot = Some(snapshot(&store, instance));", oracle)
        self.assertLess(reset, start)
        self.assertLess(start, invoke)
        self.assertLess(invoke, elapsed)
        self.assertLess(elapsed, oracle)
        self.assertLess(oracle, snapshot)

    def test_p122_real_compact_driver_stdout_confirms_mode_and_call_count(self):
        production_value = os.environ.get("WTIME_P122_BENCH_BIN", str(BENCH.parents[1] / "target/performance/P122-production-target/release/wtime-bench"))
        diagnostic_value = os.environ.get("WTIME_P122_DIAG_BIN", str(BENCH.parents[1] / "target/performance/P122-diagnostic-target/release/wtime-bench"))
        self.assertTrue(production_value, "WTIME_P122_BENCH_BIN is required for the actual-binary probe")
        self.assertTrue(diagnostic_value, "WTIME_P122_DIAG_BIN is required for the actual-binary probe")
        production = Path(production_value).resolve()
        diagnostic = Path(diagnostic_value).resolve()
        self.assertTrue(production.is_file(), f"production benchmark executable missing: {production}")
        self.assertTrue(diagnostic.is_file(), f"diagnostic benchmark executable missing: {diagnostic}")

        def capabilities(executable, *, instrumented):
            response = subprocess.run([str(executable), "--protocol"], check=True,
                                      text=True, capture_output=True)
            value = json.loads(response.stdout)
            self.assertEqual(value["schema_version"], 2)
            self.assertIs(value["instrumented"], instrumented)
            self.assertIn("compact", value["modes"])
            self.assertIn("mvp", value["features"])
            return value

        capabilities(production, instrumented=False)
        capabilities(diagnostic, instrumented=True)

        # All nine rows use fixtures copied into this test's fresh temporary root.
        for entry in self.model.entries():
            with self.subTest(case=entry["id"]):
                size = entry["params"][0]["baseline"]
                oracle = self.model.oracle(entry["id"], size)
                wasm = self.fixtures / Path(entry["source"]["path"]).with_suffix(".wasm").name
                self.assertTrue(wasm.is_file(), f"frozen fixture missing from temporary root: {wasm}")

                output = subprocess.run([
                    str(production), "--representative-staged", str(wasm), entry["id"],
                    str(size), f"i32:{oracle['result']}", oracle["output_sha256"],
                    "run_after_setup", "3", "compact", "mvp",
                ], check=True, text=True, capture_output=True)
                actual = json.loads(output.stdout)
                proof = self.compare.validate_observation(
                    entry, actual, self.model, self.scaling, 3, expected_mode="compact",
                )
                self.assertEqual(actual["stage"], "run_after_setup")
                self.assertEqual(actual["calls_checked"], 3)
                self.assertIs(actual["instrumented"], False)
                self.assertEqual(proof["validated_memory_proof"]["full_memory_sha256"],
                                 entry["expected_full_memory_sha256"])
                self.assertEqual(actual["expected_output_sha256"], oracle["output_sha256"])

                output = subprocess.run([
                    str(diagnostic), "--representative-staged-diagnostic", str(wasm), entry["id"],
                    str(size), f"i32:{oracle['result']}", oracle["output_sha256"],
                    "run_after_setup", "1", "compact", "mvp",
                ], check=True, text=True, capture_output=True)
                actual = json.loads(output.stdout)
                proof = self.compare.validate_observation(
                    entry, actual, self.model, self.scaling, 1,
                    expected_mode="compact", diagnostic=True,
                )
                self.assertEqual(actual["stage"], "run_after_setup")
                self.assertEqual(actual["calls_checked"], 1)
                self.assertIs(actual["instrumented"], True)
                self.assertTrue(actual["diagnostic"])
                self.assertEqual(proof["validated_memory_proof"]["full_memory_sha256"],
                                 entry["expected_full_memory_sha256"])
                self.assertEqual(actual["expected_output_sha256"], oracle["output_sha256"])
                counters = actual["counters"]
                compact_steps = counters["compact_steps"]
                source_dispatches = counters["source_dispatches"]
                self.assertGreater(compact_steps, 0)
                self.assertGreaterEqual(source_dispatches, compact_steps)
                # Diagnostic elapsed values are consumed only for protocol validity;
                # they do not yield ns/op rows or production timing claims.
                self.assertNotIn("ns_per_operation", actual)


if __name__ == "__main__":
    unittest.main()
