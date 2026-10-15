"""Contained synthetic tests for the P111 draft smoke/provenance API."""
from __future__ import annotations

import hashlib
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import sys
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import performance_ci as ci


def write_inside(root: Path, relative: str, raw: bytes) -> Path:
    root = root.resolve()
    path = (root / relative).resolve()
    if not path.is_relative_to(root):
        raise AssertionError("synthetic write escaped its fresh root")
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(raw)
    return path


class P111PerformanceCITests(unittest.TestCase):
    def fixture_root(self, root: Path):
        oracle_src = b"def oracle(case, n):\n    return {'call_overhead': 101, 'integer_loop': 7, 'branch_calls': -3}[case]\n"
        oracle_path = write_inside(root, "tools/wtime-bench/run.py", oracle_src)
        specs = {}
        for case in ("call_overhead", "integer_loop", "branch_calls"):
            wat = write_inside(root, f"tools/wtime-bench/fixtures/{case}.wat", (case + " wat").encode())
            wasm = write_inside(root, f"tools/wtime-bench/fixtures/{case}.wasm", (case + " wasm").encode())
            specs[case] = {"input": {"call_overhead": 100, "integer_loop": 10000,
                                      "branch_calls": 5000}[case],
                           "wat": wat.relative_to(root).as_posix(),
                           "wat_sha256": ci.file_sha(wat),
                           "wasm": wasm.relative_to(root).as_posix(),
                           "wasm_sha256": ci.file_sha(wasm)}
        for relative, data in (("Cargo.toml", b"workspace"), ("Cargo.lock", b"lock"),
                               ("rust-toolchain.toml", b"channel = 'stable'"),
                               ("crates/a/Cargo.toml", b"[package]"),
                               ("crates/a/src/lib.rs", b"pub fn f() {}")):
            write_inside(root, relative, data)
        binary = write_inside(root, "target/debug/wtime-cli", b"synthetic executable payload")
        binding = {"schema": ci.BUILD_SCHEMA, "build_argv": ci.BUILD_ARGV, "build_exit_code": 0,
                   "binary": ci.normal(binary), "core_sources": ci.current_core_sources(root)}
        binding_path = write_inside(root, "target/performance/P111/build.json",
                                    json.dumps(binding, sort_keys=True).encode())
        return oracle_path, specs, binary, binding_path

    def test_smoke_runs_three_fixed_cases_and_checks_exact_oracle_results(self):
        with tempfile.TemporaryDirectory(prefix="p111-smoke-") as temp:
            root = Path(temp).resolve()
            oracle_path, specs, binary, binding = self.fixture_root(root)
            with patch.object(ci, "ROOT", root), patch.object(ci, "FIXTURES", specs), \
                 patch.object(ci, "ORACLE_SHA256", ci.file_sha(oracle_path)):
                calls = []

                def fake_run(argv, **kwargs):
                    calls.append(argv)
                    expected = {"call_overhead": 101, "integer_loop": 7, "branch_calls": -3}
                    case = Path(argv[4]).stem
                    return SimpleNamespace(returncode=0, stdout=f"I32({expected[case]})\n", stderr="")

                output = root / "target/performance/P111/smoke.json"
                report = ci.run_smoke(binary, binding, output, root=root, run=fake_run)
                checked = ci.validate_smoke_report(report, root)
                self.assertEqual(checked["invocations"], 3)
                self.assertEqual(len(calls), 3)
                self.assertTrue(output.is_relative_to(root))
                self.assertEqual(json.loads(output.read_text())["nonzero_invocations"], 3)

    def test_smoke_rejects_zero_work_bad_stdout_and_stale_build_inputs(self):
        with tempfile.TemporaryDirectory(prefix="p111-invalid-") as temp:
            root = Path(temp).resolve()
            oracle_path, specs, binary, binding = self.fixture_root(root)
            with patch.object(ci, "ROOT", root), patch.object(ci, "FIXTURES", specs), \
                 patch.object(ci, "ORACLE_SHA256", ci.file_sha(oracle_path)):
                empty = root / "target/performance/P111/empty.json"
                with self.assertRaises(ci.PerformanceCIError):
                    ci.run_smoke(binary, binding, empty, root=root,
                                 run=lambda *a, **k: SimpleNamespace(returncode=0, stdout="", stderr=""))
                wrong = root / "target/performance/P111/wrong.json"
                with self.assertRaises(ci.PerformanceCIError):
                    ci.run_smoke(binary, binding, wrong, root=root,
                                 run=lambda *a, **k: SimpleNamespace(returncode=0, stdout="I32(0)\n", stderr=""))
                write_inside(root, "crates/a/src/lib.rs", b"changed core")
                stale = root / "target/performance/P111/stale.json"
                with self.assertRaises(ci.PerformanceCIError):
                    ci.run_smoke(binary, binding, stale, root=root,
                                 run=lambda *a, **k: self.fail("stale source must fail before invoking CLI"))

    def test_smoke_rejects_fixture_hash_tampering_and_malformed_value_shape(self):
        with tempfile.TemporaryDirectory(prefix="p111-fixture-") as temp:
            root = Path(temp).resolve()
            oracle_path, specs, binary, binding = self.fixture_root(root)
            with patch.object(ci, "ROOT", root), patch.object(ci, "FIXTURES", specs), \
                 patch.object(ci, "ORACLE_SHA256", ci.file_sha(oracle_path)):
                corrupted = root / specs["integer_loop"]["wasm"]
                corrupted.write_bytes(b"corrupt")
                with self.assertRaises(ci.PerformanceCIError):
                    ci.run_smoke(binary, binding, root / "target/out.json", root=root,
                                 run=lambda *a, **k: self.fail("fixture validation must precede execution"))
            for value in ("101\n", "I64(101)\n", "I32(101)\nextra\n", ""):
                with self.subTest(value=value), self.assertRaises(ci.PerformanceCIError):
                    ci.parse_value_i32(value)

    def _lane_fixture(self, root: Path, *, measured_config: bool = True,
                      runner_matches: bool = True, hardware_matches: bool = True):
        root = root.resolve()
        config_path = write_inside(root, "config/reference.json", b"placeholder")
        hardware = {"cpu_model": "Reference CPU", "selected_cpu": 2,
                    "cpu_quota": "100000 100000"}
        config = {"schema": ci.REFERENCE_SCHEMA, "runner_id": "dedicated-runner-17",
                  "architecture": "x86_64", "exclusive_runner": True,
                  "hardware": hardware}
        config_path.write_text(json.dumps(config, sort_keys=True))
        report_hardware = dict(hardware)
        if not hardware_matches:
            report_hardware["cpu_model"] = "different CPU"
        report_path = write_inside(
            root, "reports/measurement.json",
            json.dumps({"metadata": {"hardware": report_hardware}}).encode())
        done_path = write_inside(root, "reports/completion.json", b"completion")
        stats_path = write_inside(root, "reports/statistics.json", b"statistics")
        relative_config = config_path.relative_to(root).as_posix()
        files = {relative_config: ci.file_sha(config_path)} if measured_config else {}
        runner_digest = ci.digest_bytes(config["runner_id"].encode())
        if not runner_matches:
            runner_digest = "0" * 64
        measure = {"inputs": {
            "components": {"reference_config": {"files": files}},
            "environment": {"variables": {"WTIME_REFERENCE_RUNNER": runner_digest}},
        }}
        bound = {"completion": ci.normal(done_path), "statistics": ci.normal(stats_path),
                 "decision": "adopted", "receipts": {"measure": measure},
                 "value": {"inputs": {"report": ci.normal(report_path)}}}
        return config_path, done_path, stats_path, bound

    def test_shared_worker_trend_never_promotes_a_stored_adopted_decision(self):
        with tempfile.TemporaryDirectory(prefix="p111-trend-") as temp:
            root = Path(temp).resolve()
            _, completion, statistics, bound = self._lane_fixture(root)
            with patch.object(ci, "finalized_statistics", return_value=bound) as read_stats:
                result = ci.trend_lane(statistics, completion, root)
            read_stats.assert_called_once_with(completion, statistics, root)
            self.assertEqual(result["status"], "passed")
            self.assertEqual(result["decision"], "inconclusive")
            self.assertEqual(result["recorded_decision"], "adopted")
            self.assertIs(result["promotion_allowed"], False)
            self.assertIs(result["statistics_recomputed"], False)

    def test_reference_requires_config_hash_and_measured_runner_identity(self):
        with tempfile.TemporaryDirectory(prefix="p111-reference-bind-") as temp:
            root = Path(temp).resolve()
            config, completion, statistics, unbound = self._lane_fixture(
                root, measured_config=False, runner_matches=True)
            with patch.object(ci, "finalized_statistics", return_value=unbound):
                with self.assertRaises(ci.PerformanceCIError):
                    ci.reference_lane(config, statistics, completion, root)

            config, completion, statistics, wrong_runner = self._lane_fixture(
                root, measured_config=True, runner_matches=False)
            with patch.object(ci, "finalized_statistics", return_value=wrong_runner):
                with self.assertRaises(ci.PerformanceCIError):
                    ci.reference_lane(config, statistics, completion, root)

    def test_bound_reference_carries_stored_decision_without_recomputing_statistics(self):
        with tempfile.TemporaryDirectory(prefix="p111-reference-good-") as temp:
            root = Path(temp).resolve()
            config, completion, statistics, bound = self._lane_fixture(root)
            with patch.object(ci, "finalized_statistics", return_value=bound) as read_stats:
                result = ci.reference_lane(config, statistics, completion, root)
            read_stats.assert_called_once_with(completion, statistics, root)
            self.assertEqual(result["status"], "passed")
            self.assertEqual(result["decision"], "adopted")
            self.assertIs(result["statistics_recomputed"], False)
            self.assertIs(result["promotion_allowed"], True)
            self.assertIs(result["timing_evidence"], True)

    def test_reference_rejects_mismatched_measured_hardware(self):
        with tempfile.TemporaryDirectory(prefix="p111-reference-hardware-") as temp:
            root = Path(temp).resolve()
            config, completion, statistics, bound = self._lane_fixture(
                root, measured_config=True, runner_matches=True, hardware_matches=False)
            with patch.object(ci, "finalized_statistics", return_value=bound):
                with self.assertRaises(ci.PerformanceCIError):
                    ci.reference_lane(config, statistics, completion, root)

    def test_finalized_statistics_rejects_synthetic_evidence_before_loading_contract(self):
        import sys
        sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
        from verification.artifacts import normal as artifact_normal
        with tempfile.TemporaryDirectory(prefix="p111-synthetic-evidence-") as temp:
            root = Path(temp).resolve()
            evidence_path = write_inside(root, "evidence.json", json.dumps({
                "schema": "wtime-verification-evidence-v1", "task": "P111",
                "status": "passed", "passed": True, "synthetic": True,
            }).encode())
            completion_path = write_inside(root, "completion.json", json.dumps({
                "task": "P111", "status": "done", "outcome": "adopted",
                "evidence": [artifact_normal(evidence_path)],
            }).encode())
            statistics_path = write_inside(root, "statistics.json", b"unused")
            with patch("verification.contracts.load_spec",
                       side_effect=AssertionError("synthetic evidence must be rejected first")):
                with self.assertRaises(ci.PerformanceCIError):
                    ci.finalized_statistics(completion_path, statistics_path, root)

    def test_missing_stable_reference_configuration_is_explicitly_unrun(self):
        report = ci.reference_lane()
        self.assertEqual(report["status"], "unrun")
        self.assertEqual(report["decision"], "unrun")
        self.assertEqual(report["reason"], "reference_configuration_absent")
        self.assertIs(report["timing_evidence"], False)


if __name__ == "__main__":
    unittest.main()
