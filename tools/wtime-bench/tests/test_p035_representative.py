"""Pure-data P035 representative inventory and observation checks.

Run with WTIME_BENCH_TOOL_DIR=/workspace/wtime/tools/wtime-bench.  All copied
fixtures and generated observation data live under TemporaryDirectory; this
test does not run the encoder, a benchmark executable, or Wasmtime.
"""
from __future__ import annotations

import hashlib
import importlib.util
import math
import os
from pathlib import Path
import shutil
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True

BENCH = Path(os.environ.get("WTIME_BENCH_TOOL_DIR", Path(__file__).resolve().parents[1]))
COMPARE = Path(os.environ.get("WTIME_P035_COMPARE_PATH", BENCH / "representative_compare.py"))
if not COMPARE.is_file():
    COMPARE = Path(__file__).resolve().parent.parent / "P035-command-review" / "representative_compare.py"


def load(path: Path, name: str):
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise AssertionError(f"cannot load {path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


def load_compare():
    if not COMPARE.is_file():
        raise AssertionError(f"production representative validator is missing: {COMPARE}")
    name = "p035_production_representative_compare"
    if name in sys.modules:
        return sys.modules[name]
    return load(COMPARE, name)


def validate_wire(wire: dict, entry: dict, model, scaling, batch: int, *, diagnostic=False):
    """Use the production validator; check only fields it does not currently cover."""
    compare = load_compare()
    proof = compare.validate_observation(entry, wire, model, scaling, batch, diagnostic=diagnostic)
    if wire.get("setup_reset_before_each_call") is not True:
        raise compare.RepresentativeComparisonError("setup reset policy is missing")
    if diagnostic:
        elapsed = wire.get("diagnostic_elapsed_ns")
        if (wire.get("diagnostic") is not True or not isinstance(wire.get("counters"), dict)
                or isinstance(elapsed, bool) or not isinstance(elapsed, (int, float))
                or not math.isfinite(elapsed) or elapsed <= 0
                or not math.isclose(elapsed, wire["measured_guest_ns"], rel_tol=1e-9, abs_tol=1e-6)):
            raise compare.RepresentativeComparisonError("diagnostic timing/counter payload is malformed")
    elif "diagnostic_elapsed_ns" in wire or "counters" in wire:
        raise compare.RepresentativeComparisonError("diagnostic fields leaked into uninstrumented timing")
    return proof


class RepresentativeReviewTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="p035-representative-review-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        tool = self.root / "tools" / "wtime-bench"
        (tool / "fixtures" / "representative").mkdir(parents=True)
        shutil.copy2(BENCH / "representative_workloads.py", tool / "representative_workloads.py")
        for path in (BENCH / "fixtures" / "representative").glob("*.wat"):
            shutil.copy2(path, tool / "fixtures" / "representative" / path.name)
        self.model = load(tool / "representative_workloads.py", "p035_review_model")

    def _registry(self):
        rows = self.model.entries()
        return {"schema_version": 2, "workloads": rows,
                "selections": {"representative": {"status": "ready",
                    "workloads": [row["id"] for row in rows]}}}

    def test_inventory_is_exactly_nine_modeled_inputs(self):
        rows = self.model.entries()
        ids = [row["id"] for row in rows]
        self.assertEqual(len(ids), 9)
        self.assertEqual(len(set(ids)), 9)
        self.assertEqual({row["family"] for row in rows}, set(self.model.KERNELS))
        self.assertEqual({row["validation_role"] for row in rows}, {"small", "medium", "held_out"})
        self.assertTrue(all(row["oracle"]["independent"] for row in rows))
        # The staged compare must select by oracle module/family, not scalar-driver IDs.
        self.assertTrue(all(row["oracle"]["module"] == "representative_workloads.py" for row in rows))
        registry = self._registry()
        self.assertEqual(registry["selections"]["representative"]["workloads"], ids)

    def test_frozen_fixture_and_oracle_provenance(self):
        provenance = self.model.provenance()["kernels"]
        for family, record in provenance.items():
            source = self.root / "tools" / "wtime-bench" / record["source"]
            self.assertEqual(hashlib.sha256(source.read_bytes()).hexdigest(), record["source_sha256"])
            self.assertEqual(record["license"], "CC0-1.0")
            self.assertEqual(record["external_source_or_download"], False)
            self.assertEqual(len(record["wasm_sha256"]), 64)
            self.assertEqual(record["encoder_sha256"], self.model.ENCODER_SHA256)
            for size in self.model.KERNELS[family]["cases"].values():
                answer = self.model.oracle(family, size)
                image = self.model.expected_memory_image(family, size)
                self.assertEqual(len(image), 65_536)
                self.assertEqual(hashlib.sha256(image[4096:4096 + answer["output_bytes"]]).hexdigest(),
                                 answer["output_sha256"])

    def test_staged_contract_checks_every_call_and_full_independent_image(self):
        runner = load(BENCH / "representative_scaling.py", "p035_review_runner")
        compare = load_compare()
        model = self.model
        entry = model.entries()[0]
        size = entry["params"][0]["baseline"]
        expected = model.oracle(entry["id"], size)
        image = model.expected_memory_image(entry["id"], size)
        row = {
            "schema_version": 2, "case": entry["id"], "stage": "run_after_setup",
            "input": str(size), "expected": f"i32:{expected['result']}",
            "result": f"i32:{expected['result']}", "batch": 7, "calls_checked": 7,
            "report_complete": True, "setup_reset_before_each_call": True,
            "setup_excluded_from_timing": True, "instrumented": False,
            "measured_guest_ns": 864.5,
            "ns_per_operation": 123.5, "mode": "decoded", "profile": "mvp",
            "memory_pages": 1, "memory_page_size_log2": 16,
            "memory_hex": image.hex(), "expected_output_sha256": expected["output_sha256"],
        }
        result = validate_wire(row, entry, model, runner, 7)
        self.assertEqual(result["validated_memory_proof"]["full_memory_sha256"],
                         entry["expected_full_memory_sha256"])
        self.assertEqual(result["raw_wire_observation"], row)
        # The native JSON rounds average nanoseconds to three decimal places.
        rounded = dict(row, measured_guest_ns=1000.0, ns_per_operation=142.857)
        validate_wire(rounded, entry, model, runner, 7)
        with self.assertRaises(compare.RepresentativeComparisonError):
            validate_wire(dict(rounded, ns_per_operation=142.85), entry, model, runner, 7)
        for field, wrong in (("calls_checked", 6), ("report_complete", False),
                             ("setup_reset_before_each_call", False),
                             ("setup_excluded_from_timing", False),
                             ("instrumented", True), ("measured_guest_ns", 100.0),
                             ("ns_per_operation", 1.0)):
            bad = dict(row)
            bad[field] = wrong
            with self.assertRaises(compare.RepresentativeComparisonError):
                validate_wire(bad, entry, model, runner, 7)
        corrupted = dict(row)
        corrupted["memory_hex"] = (b"\xff" + image[1:]).hex()
        with self.assertRaises((compare.RepresentativeComparisonError,
                                runner.RepresentativeError)):
            validate_wire(corrupted, entry, model, runner, 7)
        with self.assertRaises(compare.RepresentativeComparisonError):
            validate_wire(row, entry, model, runner, 0)

    def test_counter_diagnostic_checks_full_memory_and_elapsed_path(self):
        runner = load(BENCH / "representative_scaling.py", "p035_review_diagnostic_runner")
        compare = load_compare()
        entry = self.model.entries()[0]
        size = entry["params"][0]["baseline"]
        expected = self.model.oracle(entry["id"], size)
        image = self.model.expected_memory_image(entry["id"], size)
        row = {
            "schema_version": 2, "diagnostic": True, "instrumented": True,
            "case": entry["id"], "stage": "run_after_setup", "input": str(size),
            "expected": f"i32:{expected['result']}", "result": f"i32:{expected['result']}",
            "batch": 1, "calls_checked": 1, "report_complete": True,
            "setup_reset_before_each_call": True, "setup_excluded_from_timing": True,
            "measured_guest_ns": 234.0, "diagnostic_elapsed_ns": 234.0,
            "ns_per_operation": 234.0, "mode": "decoded", "profile": "mvp",
            "memory_pages": 1, "memory_page_size_log2": 16, "memory_hex": image.hex(),
            "expected_output_sha256": expected["output_sha256"], "counters": {"instruction": 1},
        }
        result = validate_wire(row, entry, self.model, runner, 1, diagnostic=True)
        self.assertEqual(result["raw_wire_observation"], row)
        for field, wrong in (("diagnostic", False), ("diagnostic_elapsed_ns", 0.0),
                             ("counters", "not-an-object")):
            bad = dict(row)
            bad[field] = wrong
            with self.assertRaises(compare.RepresentativeComparisonError):
                validate_wire(bad, entry, self.model, runner, 1, diagnostic=True)
        with self.assertRaises(compare.RepresentativeComparisonError):
            validate_wire(row, entry, self.model, runner, 0, diagnostic=True)

    def test_pair_totals_are_derived_from_declared_inventory_and_config(self):
        # This deliberately avoids freezing a guessed sample count into the test.
        workloads = self.model.entries()
        runs = 2
        pairs_per_group = 30
        stages = {stage for row in workloads for stage in row["stages"]}
        groups = len(workloads) * len(stages)
        self.assertEqual(groups, 9)
        self.assertEqual(groups * runs * pairs_per_group, 540)
        self.assertEqual(groups * runs * pairs_per_group * 2, 1080)


if __name__ == "__main__":
    unittest.main()
