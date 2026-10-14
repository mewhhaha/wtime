"""Focused source, promotion, budget, and restoration tests for P005."""
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

ROOT = Path(__file__).resolve().parents[3]
BENCH = ROOT / "tools/wtime-bench"
sys.path.insert(0, str(BENCH))
sys.path.insert(0, str(ROOT / "tools/checks"))
import gates
import build_budget


def load_report():
    pairs = []
    for index in range(30):
        pairs.append({
            "pair_id": str(index), "workload": "integer_loop", "stage": "warm_call",
            "input": 64, "wasm_sha256": "a" * 64, "expected": 7,
            "order": ["baseline", "candidate"],
            "baseline": {"ns_per_operation": 100.0, "batch": 10, "result": 7, "mode": "decoded"},
            "candidate": {"ns_per_operation": 90.0, "batch": 10, "result": 7, "mode": "decoded"},
        })
    return {
        "schema_version": 2, "passed": True, "source_stable": True, "selection": "scalar",
        "metadata": {"settings": {"candidate_mode": "decoded", "baseline_mode": "decoded",
                                    "instrumented": False}, "instrumented": False,
                     "candidate_sources": {"crates/a.rs": "1" * 64}},
        "runs": [{"seed": 3, "pairs": pairs}, {"seed": 4, "pairs": pairs}],
    }


def row(run, workload, stage, median, lower=None, upper=None):
    lower = median if lower is None else lower
    upper = median if upper is None else upper
    return {"run": run, "workload": workload, "stage": stage,
            "candidate_over_baseline": {"median": median},
            "median_ratio_95ci": {"lower": lower, "upper": upper}}


class P005GateTests(unittest.TestCase):
    def test_stale_candidate_source_identity_is_rejected_before_analysis(self):
        report = load_report()
        with mock.patch.object(gates, "_current_source_identity", return_value={"crates/a.rs": "2" * 64}), \
                mock.patch.object(gates.analyze, "analyze") as analyze_report:
            with self.assertRaisesRegex(gates.GateError, "source identity is stale"):
                gates._check_report(report, "scalar")
            analyze_report.assert_not_called()

    def test_current_report_is_analyzed_and_warm_family_requires_both_runs(self):
        report = load_report()
        source_map = report["metadata"]["candidate_sources"]
        with mock.patch.object(gates, "_current_source_identity", return_value=source_map):
            _, result = gates._check_report(report, "scalar")
        legacy_selection = {name: {"prepare", "instantiate", "first_call", "warm_call", "load_and_call"}
                            for name in ("call_overhead", "integer_loop", "branch_calls")}
        with mock.patch.object(gates, "_selection_stages", return_value=legacy_selection):
            decision, reasons, checks = gates._comparison_decision(
                {"per_run": [row(run, workload, stage, 0.90 if stage == "warm_call" else 1.0,
                                  0.90 if stage == "warm_call" else 0.98,
                                  0.95 if stage == "warm_call" else 1.02)
                              for run in (1, 2)
                              for workload in legacy_selection
                              for stage in legacy_selection[workload]]},
                "scalar",
            )
        self.assertEqual(result["per_run"][0]["decision"], "supported_improvement")
        self.assertEqual(decision, "adopted")
        self.assertEqual(reasons, [])
        self.assertEqual(checks["primary_workloads"], ["branch_calls", "call_overhead", "integer_loop"])
        missing = {"per_run": [entry for entry in [row(run, workload, stage, 0.9)
                    for run in (1, 2) for workload in ("call_overhead", "integer_loop", "branch_calls")
                    for stage in ("prepare", "instantiate", "first_call", "warm_call", "load_and_call")]
                    if not (entry["run"] == 2 and entry["workload"] == "branch_calls" and entry["stage"] == "warm_call")]}
        with mock.patch.object(gates, "_selection_stages", return_value=legacy_selection):
            with self.assertRaisesRegex(gates.GateError, "missing primary warm"):
                gates._comparison_decision(missing, "scalar")

    def test_inconclusive_interval_does_not_promote_and_hard_budget_rejects(self):
        entries = []
        for run in (1, 2):
            for workload in ("call_overhead", "integer_loop", "branch_calls"):
                for stage in ("prepare", "instantiate", "first_call", "warm_call", "load_and_call"):
                    if stage == "warm_call" and workload == "integer_loop" and run == 2:
                        entries.append(row(run, workload, stage, 0.93, 0.90, 1.01))
                    else:
                        entries.append(row(run, workload, stage, 0.90 if stage == "warm_call" else 1.0,
                                           0.90 if stage == "warm_call" else 0.99,
                                           0.95 if stage == "warm_call" else 1.02))
        legacy_selection = {name: {"prepare", "instantiate", "first_call", "warm_call", "load_and_call"}
                            for name in ("call_overhead", "integer_loop", "branch_calls")}
        with mock.patch.object(gates, "_selection_stages", return_value=legacy_selection):
            decision, _, _ = gates._comparison_decision({"per_run": entries}, "scalar")
        self.assertEqual(decision, "inconclusive")
        bad = {"source_stable": True, "default_dependency_closure_pass": True,
               "budgets": gates.HARD_LIMITS, "clean_seconds": 61.0, "leaf_seconds": 1.0,
               "stripped_bytes": 1000}
        self.assertFalse(build_budget.check_budget(bad))

    def test_build_identity_and_dependency_closure_are_mandatory(self):
        inputs = {"Cargo.toml": "a" * 64}
        report = {"mode": "default", "features": [], "source_stable": True,
                  "source_sha256": "current", "source_inputs": inputs,
                  "source_inputs_after": inputs, "source_sha256_after": "current",
                  "default_dependency_closure_pass": False, "budget_pass": True,
                  "budgets": gates.HARD_LIMITS, "clean_seconds": 1.0, "leaf_seconds": 1.0,
                  "stripped_bytes": 1000}
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "build.json"
            path.write_text(json.dumps(report))
            with mock.patch.object(gates.build_budget, "cargo_metadata", return_value={"packages": []}), \
                    mock.patch.object(gates.build_budget, "source_inputs", return_value=inputs), \
                    mock.patch.object(gates.build_budget, "hash_inputs", return_value="current"), \
                    mock.patch.object(gates, "_current_rust_sources", return_value={"crates/example.rs": ""}), \
                    mock.patch("dependency_budget.violations", return_value=[]):
                with self.assertRaisesRegex(gates.GateError, "does not prove the default dependency"):
                    gates._validate_build(path, expected_mode="default", label="default build report")
                report["default_dependency_closure_pass"] = True
                path.write_text(json.dumps(report))
                self.assertEqual(gates._validate_build(path, expected_mode="default",
                                                       label="default build report")["mode"], "default")
                report["source_sha256"] = "stale"
                path.write_text(json.dumps(report))
                with self.assertRaisesRegex(gates.GateError, "source hash is stale"):
                    gates._validate_build(path, expected_mode="default", label="default build report")

    def test_feature_spec_is_strict_and_failed_leaf_build_restores_file(self):
        metadata = {"workspace_members": ["id"], "packages": [{
            "id": "id", "name": "wtime-runtime", "features": {"extra": []}}]}
        self.assertEqual(build_budget.validate_feature_spec("wtime-runtime/extra", metadata),
                         ("wtime-runtime", "extra"))
        for invalid in ("extra", "wtime-runtime/missing", "unknown/extra", "wtime-runtime/"):
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                build_budget.validate_feature_spec(invalid, metadata)
        with tempfile.TemporaryDirectory() as directory:
            leaf = Path(directory) / "main.rs"
            original = b"fn main() {}\n"
            leaf.write_bytes(original)
            before = leaf.stat()
            with mock.patch.object(build_budget, "run", side_effect=RuntimeError("injected build failure")):
                with self.assertRaisesRegex(RuntimeError, "injected build failure"):
                    build_budget._runtime_leaf(["cargo", "build"], {}, leaf)
            self.assertEqual(leaf.read_bytes(), original)
            after = leaf.stat()
            self.assertEqual(after.st_mtime_ns, before.st_mtime_ns)


if __name__ == "__main__":
    unittest.main()
