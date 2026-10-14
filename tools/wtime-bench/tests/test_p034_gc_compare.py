"""Focused P034 comparator contract tests; all generated files stay in TemporaryDirectory."""
from __future__ import annotations

import copy
import importlib.util
import json
import sys
import tempfile
import types
import unittest
from unittest.mock import patch
from pathlib import Path

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent
STAGE = HERE.parent if HERE.name == "tests" else HERE
ROOT = STAGE.parent.parent
MODEL_DIR = STAGE
sys.path.insert(0, str(STAGE))
CONTRACT_FILE = ROOT / "docs/benchmarks/performance/P034-contract.json"
MEASUREMENT_FILE = ROOT / "docs/benchmarks/performance/P034-measurement-contract.json"


def load_module(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    assert spec is not None and spec.loader is not None
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


gc_compare = load_module("p034_test_gc_compare", STAGE / "gc_compare.py")
layout_model = load_module("p034_test_gc_layout_workloads", MODEL_DIR / "gc_layout_workloads.py")
REGISTRY = json.loads((MODEL_DIR / "workloads.json").read_text())
CONTRACT = json.loads(CONTRACT_FILE.read_text())
MEASUREMENT = json.loads(MEASUREMENT_FILE.read_text())


def selected_predecessor_rows():
    by_id = {row["id"]: row for row in REGISTRY["workloads"]}
    control_ids = REGISTRY["selections"]["gc-exceptions"]["workloads"]
    old = [by_id[item] for item in control_ids[:45]]
    warm = [by_id[item] for item in control_ids[45:]]
    return old, warm


class RowsModel:
    def __init__(self, rows):
        self._rows = rows

    def entries(self):
        return copy.deepcopy(self._rows)


def valid_contract_result(registry=None, measurement=None):
    old, warm = selected_predecessor_rows()
    layouts = layout_model.entries()
    chosen_registry = copy.deepcopy(REGISTRY if registry is None else registry)
    return gc_compare.validate_layout_contract(
        copy.deepcopy(CONTRACT), old, warm, layouts,
        list(chosen_registry["selections"]["gc-layouts"]["workloads"]),
        layout_model, chosen_registry,
        copy.deepcopy(MEASUREMENT if measurement is None else measurement),
    )


class P034GcCompareTests(unittest.TestCase):
    def test_exact_selection_partition_and_matrix(self):
        old, warm = selected_predecessor_rows()
        layouts = gc_compare.selected_layout_model_rows(REGISTRY, old, warm, layout_model)
        ids = REGISTRY["selections"]["gc-layouts"]["workloads"]
        result = valid_contract_result()
        self.assertEqual(len(ids), 117)
        self.assertEqual(result["primary_workload_count"], 24)
        self.assertEqual(result["protected_workload_count"], 93)
        self.assertEqual(result["groups_per_run"], 405)
        self.assertEqual(result["groups_total"], 810)
        self.assertEqual(result["pairs_total"], 24300)
        self.assertEqual(result["warm_calibrations_total"], 144)
        self.assertEqual(result["correctness_probes_total"], 234)
        self.assertEqual(len(layouts), 36)

    def test_changed_350_row_predecessor_is_rejected(self):
        mutated = copy.deepcopy(REGISTRY)
        mutated["workloads"][12]["description"] = "modified predecessor row"
        with self.assertRaisesRegex(gc_compare.GcComparisonError, "frozen predecessor registry proof"):
            valid_contract_result(registry=mutated)

    def test_rich_measurement_contract_mismatch_is_rejected(self):
        mutated = copy.deepcopy(MEASUREMENT)
        mutated["groups_total"] += 1
        with self.assertRaisesRegex(gc_compare.GcComparisonError, "measurement contract"):
            valid_contract_result(measurement=mutated)

    def test_gc_exceptions_control_stays_exact_81_rows(self):
        old, warm = selected_predecessor_rows()
        actual_old, actual_warm = gc_compare.selected_model_rows(
            copy.deepcopy(REGISTRY), RowsModel(old), RowsModel(warm))
        self.assertEqual(len(actual_old), 45)
        self.assertEqual(len(actual_warm), 36)
        self.assertEqual(REGISTRY["selections"]["gc-exceptions"]["workloads"],
                         [row["id"] for row in actual_old + actual_warm])

    def test_archive_contract_byte_mismatch_is_rejected_before_output(self):
        with tempfile.TemporaryDirectory(prefix="p034-contract-guard-") as temp:
            temp_root = Path(temp)
            frozen = temp_root / "archive"
            live = temp_root / "live"
            manifest_files = {}
            required = [
                "source/tools/wtime-bench/workloads.json",
                "source/tools/wtime-bench/gc_workloads.py",
                "source/tools/wtime-bench/gc_warm_workloads.py",
                "source/tools/wtime-bench/gc_scaling.py",
                "source/tools/wtime-bench/gc_layout_workloads.py",
                "source/docs/benchmarks/performance/P034-contract.json",
                "source/docs/benchmarks/performance/P034-measurement-contract.json",
            ]
            source_bytes = {
                "docs/benchmarks/performance/P034-contract.json":
                    CONTRACT_FILE.read_bytes(),
                "docs/benchmarks/performance/P034-measurement-contract.json":
                    MEASUREMENT_FILE.read_bytes(),
            }
            for rel in required:
                logical = rel.removeprefix("source/")
                data = source_bytes.get(logical, ("synthetic source: " + logical).encode())
                (live / logical).parent.mkdir(parents=True, exist_ok=True)
                (live / logical).write_bytes(data)
                archive_path = frozen / rel
                archive_path.parent.mkdir(parents=True, exist_ok=True)
                if rel.endswith("P034-measurement-contract.json"):
                    archive_path.write_bytes(data + b"\ncorrupt")
                else:
                    archive_path.write_bytes(data)
                import hashlib
                manifest_files[rel] = {
                    "role": "source", "source_path": logical,
                    "sha256": hashlib.sha256(data).hexdigest(),
                }

            manifest = {"files": manifest_files}
            baseline = types.ModuleType("baseline")
            baseline.verify_archive = lambda _path: manifest
            checkpoint = types.ModuleType("checkpoint")
            checkpoint.Journal = type("Journal", (), {"preflight": staticmethod(lambda _path: None)})
            compare = types.ModuleType("compare")
            compare._read_build_growth_explanation = lambda _path: None
            compare._baseline_identity = lambda *_args: {
                "kind": "current-source-archive", "mode": "decoded", "frozen": frozen,
            }
            compare._verify_scalar_harness_sources = lambda *_args: None
            overrides = {
                "baseline": baseline,
                "checkpoint": checkpoint,
                "compare": compare,
                "gc_scaling": types.ModuleType("gc_scaling"),
                "gc_workloads": types.ModuleType("gc_workloads"),
                "gc_warm_workloads": types.ModuleType("gc_warm_workloads"),
                "workloads": types.ModuleType("workloads"),
                "gc_layout_workloads": layout_model,
            }
            patcher = patch.dict(sys.modules, overrides)
            patcher.start()
            prior = (gc_compare.ROOT, gc_compare.LAYOUT_CONTRACT_PATH,
                     gc_compare.LAYOUT_MEASUREMENT_CONTRACT_PATH)
            gc_compare.ROOT = live
            gc_compare.LAYOUT_CONTRACT_PATH = live / "docs/benchmarks/performance/P034-contract.json"
            gc_compare.LAYOUT_MEASUREMENT_CONTRACT_PATH = live / "docs/benchmarks/performance/P034-measurement-contract.json"
            output = temp_root / "output-must-not-be-created"
            args = types.SimpleNamespace(selection="gc-layouts", pairs=30, seed=9,
                                         output=str(output), baseline=str(temp_root / "archive"),
                                         candidate_mode="decoded", build_growth_explanation_file=None)
            try:
                with self.assertRaisesRegex(gc_compare.GcComparisonError, "P034-measurement-contract.json"):
                    gc_compare.run(args, None)
                self.assertFalse(output.exists())
            finally:
                (gc_compare.ROOT, gc_compare.LAYOUT_CONTRACT_PATH,
                 gc_compare.LAYOUT_MEASUREMENT_CONTRACT_PATH) = prior
                patcher.stop()


if __name__ == "__main__":
    unittest.main()
