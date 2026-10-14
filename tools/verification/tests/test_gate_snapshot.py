#!/usr/bin/env python3
"""Isolated tests for gate evaluation's single shared build snapshot."""
from __future__ import annotations

import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "tools/checks"))
import dependency_budget  # noqa: E402
GATES_PATH = ROOT / "tools/wtime-bench/gates.py"
spec = importlib.util.spec_from_file_location("gates_snapshot_tested", GATES_PATH)
gates = importlib.util.module_from_spec(spec)
assert spec and spec.loader
sys.modules[spec.name] = gates
spec.loader.exec_module(gates)


def write_json(path: Path, value: dict) -> Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, sort_keys=True) + "\n")
    return path


def build_record(source_inputs: dict[str, str], mode: str, features: list[str]) -> dict:
    source_hash = gates.build_budget.hash_inputs(source_inputs)
    budgets = dict(gates.HARD_LIMITS)
    values = {name: 1.0 for name in gates.HARD_LIMITS}
    values["stripped_bytes"] = 1
    return {
        "source_stable": True,
        "source_sha256": source_hash,
        "source_sha256_after": source_hash,
        "source_inputs": source_inputs,
        "source_inputs_after": source_inputs,
        "mode": mode,
        "features": features,
        "default_dependency_closure_pass": True,
        "budgets": budgets,
        "budget_pass": True,
        **values,
    }


class GateSnapshotTests(unittest.TestCase):
    def _evaluate_fixture(self, root: Path, *, source_inputs: dict[str, str],
                          source_after: dict[str, str] | None = None,
                          benchmark_after: dict[str, str] | None = None,
                          rust_after: dict[str, str] | None = None) -> dict:
        selection = "synthetic-selection"
        benchmark_sources = {"tools/wtime-bench/run.py": "a" * 64}
        metadata = {
            "settings": {"candidate_mode": "compact"},
            "candidate_build_features": ["wtime-runtime/compact-executor"],
            "candidate_sources": benchmark_sources,
        }
        report = {"passed": True, "source_stable": True, "selection": selection,
                  "metadata": metadata}
        report_path = write_json(root / "comparison/report.json", report)
        default_path = write_json(root / "default.json", build_record(source_inputs, "default", []))
        optional_path = write_json(root / "optional.json", build_record(
            source_inputs, "optional-feature", ["wtime-runtime/compact-executor"]))
        baseline_path = write_json(root / "baseline.json", {
            **build_record(source_inputs, "default", []),
        })
        with patch.object(gates, "_task", return_value={
                "measurement_selection": selection, "execution_mode": "compact", "kind": "gate"}), \
             patch.object(gates.analyze, "analyze", return_value={}), \
             patch.object(gates, "_current_source_identity", side_effect=[
                 benchmark_sources, benchmark_after if benchmark_after is not None else benchmark_sources]), \
             patch.object(gates, "_comparison_decision", return_value=("adopted", [], {})), \
             patch.object(gates.build_budget, "cargo_metadata", return_value={"packages": []}) as cargo_metadata, \
             patch.object(gates.build_budget, "source_inputs",
                          side_effect=[source_inputs, source_after if source_after is not None else source_inputs]) as input_map, \
             patch.object(gates, "_current_rust_sources", side_effect=[
                 {"crates/example.rs": ""}, rust_after if rust_after is not None else {"crates/example.rs": ""}]), \
             patch.object(gates, "BASELINE_BUILD_PATH", baseline_path), \
             patch.object(dependency_budget, "violations", return_value=[]) as dependency_scan:
            result = gates.evaluate("P900", report_path.parent, default_path, optional_path)
        self.assertEqual(cargo_metadata.call_count, 1)
        self.assertEqual(input_map.call_count, 2)
        dependency_scan.assert_called_once_with({"packages": []}, {"crates/example.rs": ""})
        return result

    def test_evaluate_reuses_one_snapshot_for_both_build_reports_and_dependencies(self):
        with tempfile.TemporaryDirectory(prefix="gate-snapshot-shared-", dir="/tmp") as td:
            root = Path(td).resolve()
            result = self._evaluate_fixture(root, source_inputs={"Cargo.toml": "1" * 64})
            self.assertEqual(result["decision"], "contract_only")

    def test_changed_build_inputs_after_snapshot_fail_closed(self):
        with tempfile.TemporaryDirectory(prefix="gate-snapshot-drift-", dir="/tmp") as td:
            root = Path(td).resolve()
            initial = {"Cargo.toml": "1" * 64}
            changed = {"Cargo.toml": "2" * 64}
            with self.assertRaisesRegex(gates.GateError, "changed during gate evaluation"):
                self._evaluate_fixture(root, source_inputs=initial, source_after=changed)

    def test_changed_benchmark_sources_after_snapshot_fail_closed(self):
        with tempfile.TemporaryDirectory(prefix="gate-snapshot-harness-drift-", dir="/tmp") as td:
            root = Path(td).resolve()
            with self.assertRaisesRegex(gates.GateError, "benchmark source inputs changed"):
                self._evaluate_fixture(root, source_inputs={"Cargo.toml": "1" * 64},
                                       benchmark_after={"tools/wtime-bench/run.py": "b" * 64})

    def test_changed_rust_dependency_scan_sources_after_snapshot_fail_closed(self):
        with tempfile.TemporaryDirectory(prefix="gate-snapshot-rust-drift-", dir="/tmp") as td:
            root = Path(td).resolve()
            with self.assertRaisesRegex(gates.GateError, "Rust dependency/unsafe scan sources changed"):
                self._evaluate_fixture(root, source_inputs={"Cargo.toml": "1" * 64},
                                       rust_after={"crates/example.rs": "changed"})

    def test_stale_build_map_and_dependency_violations_are_rejected(self):
        with tempfile.TemporaryDirectory(prefix="gate-snapshot-stale-", dir="/tmp") as td:
            root = Path(td).resolve()
            source_inputs = {"Cargo.toml": "1" * 64}
            snapshot = gates._BuildSnapshot({"packages": []}, source_inputs,
                                             gates.build_budget.hash_inputs(source_inputs), {}, [])
            path = write_json(root / "stale.json", build_record({"Cargo.toml": "2" * 64}, "default", []))
            with self.assertRaisesRegex(gates.GateError, "source hash is stale"):
                gates._validate_build(path, expected_mode="default", label="stale", snapshot=snapshot)
            snapshot.dependency_violations = ["synthetic external dependency"]
            with self.assertRaisesRegex(gates.GateError, "dependency/unsafe gate failed"):
                gates._check_default_dependencies(snapshot)


if __name__ == "__main__":
    unittest.main()
