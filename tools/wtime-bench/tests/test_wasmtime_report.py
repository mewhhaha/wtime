"""Synthetic validation tests for the offline Wasmtime comparison report."""
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

HERE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(HERE))
import wasmtime_report as report


class WasmtimeReportTests(unittest.TestCase):
    def setUp(self):
        self.reset_fixture()

    def reset_fixture(self):
        if hasattr(self, "temporary"):
            self.temporary.cleanup()
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name).resolve()
        self.manifest_path = self.inside(self.root / "manifest.json")
        self.manifest = self._write_matrix()

    def tearDown(self):
        self.temporary.cleanup()

    def inside(self, path):
        path = Path(path).resolve()
        self.assertTrue(path.is_relative_to(self.root), path)
        return path

    def _write_matrix(self):
        refs = []
        sources = {"tools/wtime-bench/run.py": "a" * 64,
                   "crates/wtime/src/lib.rs": "b" * 64,
                   "crates/wtime/Cargo.toml": "c" * 64}
        for tier, (mode, threshold) in report.TIERS.items():
            for run in report.RUNS:
                rows = []
                for workload, stage in report.GROUPS:
                    for sample in range(30):
                        base = 10 + sample / 10
                        rows.extend([
                            {"workload": workload, "stage": stage, "engine": "wtime", "sample": sample,
                             "batch": 1, "ns_per_operation": base * 2},
                            {"workload": workload, "stage": stage, "engine": "wasmtime", "sample": sample,
                             "batch": 1, "ns_per_operation": base},
                        ])
                body = {"passed": True, "source_stable": True, "wasm_correctness_checked": True,
                        "samples": 30, "seed": 1000 + run, "wasmtime_version": "49.0.2",
                        "recorded_at_utc": f"2026-01-0{run}T00:00:00Z", "rustc": "rustc 1.90.0",
                        "hardware": {"cpu_model": "Synthetic CPU"},
                        "settings": {"profile": "release", "wasmtime_features": ["std", "runtime", "cranelift"],
                                     "opcode_coverage": False, "cranelift_optimization": "speed",
                                     "fuel": False, "epoch_interruption": False, "wasmtime_cache": False,
                                     "wasmtime_parallel_compilation": False, "wtime_features": "mvp",
                                     "wtime_mode": mode, "register_entry_threshold": threshold,
                                     "preparation": "prepare boundary", "instantiation": "instance boundary",
                                     "first_call": "first call boundary", "warm_call": "warm call boundary",
                                     "load_and_call": "load call boundary"},
                        "implementation_sources": sources, "raw_samples": rows,
                        "workloads": {name: {"input": 100, "expected_result": 7,
                                              "wasm_sha256": (str(index) * 64), "wasm_bytes": 20 + index,
                                              "wat_sha256": (str(index + 1) * 64),
                                              "wasm_path": f"unused/{tier}-{run}-{name}.wasm",
                                              "wat_path": f"unused/{tier}-{run}-{name}.wat"}
                                      for index, name in enumerate((*report.WORKLOADS, "prepare_large"), 1)}}
                data = (json.dumps(body, sort_keys=True) + "\n").encode()
                path = self.inside(self.root / tier / f"run{run}.json")
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(data)
                refs.append({"tier": tier, "run": run, "path": str(path.relative_to(self.root)),
                             "sha256": hashlib.sha256(data).hexdigest(), "bytes": len(data)})
        manifest = {"reports": refs}
        self.manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
        return manifest

    def write_report(self, ref, mutate):
        path = self.inside(self.root / ref["path"])
        value = json.loads(path.read_text(encoding="utf-8"))
        mutate(value)
        data = (json.dumps(value, sort_keys=True) + "\n").encode()
        path.write_bytes(data)
        ref["sha256"] = hashlib.sha256(data).hexdigest()
        ref["bytes"] = len(data)
        self.manifest_path.write_text(json.dumps(self.manifest), encoding="utf-8")

    def test_complete_eight_report_matrix_analyzes_and_preserves_pairing(self):
        # Keep the matrix/schema check cheap; the dedicated bootstrap test below
        # exercises the exact-rank sampler with a nontrivial resample count.
        with mock.patch.object(report, "paired_interval", side_effect=lambda values, seed: {
                "median": sorted(values)[len(values)//2], "ci95": [min(values), max(values)],
                "resamples": 10, "seed": seed}):
            result = report.analyze(self.manifest_path)
        self.assertEqual(len(result["runs"]), 8)
        self.assertEqual(sum(len(run["groups"]) for run in result["runs"]), 168)
        self.assertEqual(result["runs"][0]["groups"][0]["ratio_of_engine_medians"], 2)
        self.assertIsNone(result["promotion"])
        self.assertIn("crates/wtime/src/lib.rs", result["source_identity"])

    def test_policy_missing_pair_duplicate_and_invalid_measurements_rejected(self):
        first = self.manifest["reports"][0]
        cases = [
            lambda data: data["settings"].update(wtime_mode="register"),
            lambda data: data["raw_samples"].pop(),
            lambda data: data["raw_samples"].append(dict(data["raw_samples"][0])),
            lambda data: data["raw_samples"][0].update(ns_per_operation=-1),
            lambda data: data["raw_samples"][0].update(ns_per_operation=float("nan")),
        ]
        for mutate in cases:
            with self.subTest(mutate=mutate):
                first = self.manifest["reports"][0]
                self.write_report(first, mutate)
                with mock.patch.object(report, "paired_interval", return_value={"median": 1, "ci95": [1, 1]}):
                    with self.assertRaises(ValueError):
                        report.analyze(self.manifest_path)
                # Restore a fresh isolated matrix before the next subcase.
                self.reset_fixture()

    def test_manifest_digest_and_source_identity_are_checked(self):
        original = self.manifest["reports"][0]
        original["sha256"] = "0" * 64
        self.manifest_path.write_text(json.dumps(self.manifest), encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "SHA-256"):
            report.analyze(self.manifest_path)
        self.reset_fixture()
        second = self.manifest["reports"][1]
        self.write_report(second, lambda data: data["implementation_sources"].update({"crates/wtime/src/lib.rs": "d" * 64}))
        with mock.patch.object(report, "paired_interval", return_value={"median": 1, "ci95": [1, 1]}):
            with self.assertRaisesRegex(ValueError, "source identity"):
                report.analyze(self.manifest_path)

    def test_constant_ratio_bootstrap_is_identity(self):
        result = report.paired_interval([2.5] * 30, seed=123, resamples=50)
        self.assertEqual(result["median"], 2.5)
        self.assertEqual(result["ci95"], [2.5, 2.5])
        self.assertEqual(result["resamples"], 50)


if __name__ == "__main__":
    unittest.main()
