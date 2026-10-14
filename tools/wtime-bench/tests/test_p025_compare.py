"""Mocked validation tests for P025 staged memory comparison; no Wasm process is run."""
from __future__ import annotations

import hashlib
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

HERE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(HERE))
import compare
import gates
import memory_scaling


PAGE = 65_536
IMAGE = bytes([1]) + bytes(PAGE - 1)
IMAGE_SHA = hashlib.sha256(IMAGE).hexdigest()
RANGE_SHA = hashlib.sha256(b"\x01").hexdigest()


def _memory_entry(family="memory_widths_predictable", stages=None):
    return {
        "id": family, "family": family,
        "stages": compare.STAGES.copy() if stages is None and family in compare.MEMORY_STAGED_FAMILIES
        else (["load_and_call"] if stages is None else stages),
        "source": {"sha256": "a" * 64, "wasm_sha256": "b" * 64,
                   "path": "fixtures/memory/widths.wat"},
        "required_features": ["mvp"],
        "params": [{"name": "accesses", "baseline": 1}],
        "setup_policy": {"timed": False},
    }


def _expected():
    return {"memory_index": 0, "initial_pages": 1, "range": [0, 1],
            "bytes_hex": "01", "sha256": RANGE_SHA,
            "full_memory_sha256": IMAGE_SHA, "result": 3}


def _driver_row(entry, *, stage="warm_call", batch=4, image=IMAGE, ns=10.0,
                outcome="i32:3", mode="decoded", profile="mvp", driver="plain"):
    return {"schema_version": 2, "case": entry["id"], "stage": stage,
            "input": "i32:1", "expected": "i32:3", "result": outcome,
            "mode": mode, "profile": profile, "driver": driver,
            "batch": batch, "ns_per_operation": ns,
            "memory_pages": 1, "memory_page_size_log2": 16,
            "memory_hex": image.hex(), "exported_memory_index": 0,
            "trap_kind": None, "trap_message": None}


class P025MemoryComparisonTests(unittest.TestCase):
    def test_gate_source_identity_includes_memory_tools_only_for_memory_selection(self):
        report = {
            "passed": True,
            "source_stable": True,
            "selection": "memory",
            "metadata": {"candidate_sources": {"memory-source.py": "abc"}},
        }
        with mock.patch.object(gates.compare, "sources", return_value={"memory-source.py": "abc"}) as sources, \
                mock.patch.object(gates.analyze, "analyze", return_value={}) as analyze_report:
            gates._check_report(report, "memory")
        sources.assert_called_once_with(include_memory=True, include_simd=False)
        analyze_report.assert_called_once_with(report)

        report["selection"] = "scalar"
        with mock.patch.object(gates.compare, "sources", return_value={"scalar-source.py": "def"}) as sources, \
                mock.patch.object(gates.analyze, "analyze", return_value={}) as analyze_report:
            report["metadata"]["candidate_sources"] = {"scalar-source.py": "def"}
            gates._check_report(report, "scalar")
        sources.assert_called_once_with(include_memory=False, include_simd=False)
        analyze_report.assert_called_once_with(report)

    def test_memory_validator_accepts_dynamic_stage_and_requires_exact_batch(self):
        entry = _memory_entry()
        row = _driver_row(entry)
        verified = memory_scaling._validate_observation(
            row, entry, 1, _expected(), "decoded", "mvp", "plain",
            stage="warm_call", expected_batch=4)
        self.assertEqual(verified["batch"], 4)
        self.assertEqual(verified["full_memory_sha256"], IMAGE_SHA)
        with self.assertRaisesRegex(memory_scaling.BaselineError, "batch mismatch"):
            memory_scaling._validate_observation(
                dict(row, batch=5), entry, 1, _expected(), "decoded", "mvp", "plain",
                stage="warm_call", expected_batch=4)
        with self.assertRaisesRegex(memory_scaling.BaselineError, "outcome/identity"):
            memory_scaling._validate_observation(
                row, entry, 1, _expected(), "decoded", "mvp", "plain",
                stage="first_call", expected_batch=4)

    def test_memory_full_image_and_range_model_hash_are_both_required(self):
        entry = _memory_entry()
        changed = bytearray(IMAGE)
        changed[-1] = 0x80
        with self.assertRaisesRegex(memory_scaling.BaselineError, "full-memory hash mismatch"):
            memory_scaling._validate_observation(
                _driver_row(entry, image=bytes(changed)), entry, 1, _expected(),
                "decoded", "mvp", "plain", stage="warm_call", expected_batch=4)
        wrong_hash = dict(_expected(), sha256="0" * 64)
        with self.assertRaisesRegex(memory_scaling.BaselineError, "range digest mismatch"):
            memory_scaling._validate_observation(
                _driver_row(entry), entry, 1, wrong_hash, "decoded", "mvp", "plain",
                stage="warm_call", expected_batch=4)

    def test_memory_wire_rows_are_normalized_and_never_persist_memory_hex(self):
        entry = _memory_entry()
        fixture = {"path": "/mock/widths.wasm", "entry": entry,
                   "expected_memory": _expected(), "memory_module": memory_scaling,
                   "expected": 3}
        with mock.patch.object(compare, "process", return_value=_driver_row(entry)) as process:
            row = compare.memory_observation(
                Path("/mock/engine"), fixture["path"], entry, 1,
                fixture["expected_memory"], "warm_call", 4, "decoded", 0,
                memory_module=_oracle_module())
        args = process.call_args.args[1]
        self.assertEqual(args[0], "--memory-staged")
        self.assertEqual(len(args), 10)
        self.assertEqual(row["input"], 1)
        self.assertEqual(row["expected"], 3)
        self.assertEqual(row["result"], 3)
        self.assertEqual(row["batch"], 4)
        self.assertNotIn("memory_hex", row["wire_observation"])

    def test_short_memory_auto_estimate_is_recalibrated_before_timing(self):
        entry = _memory_entry()
        fixture = {"path": "/mock/widths.wasm", "entry": entry,
                   "expected_memory": _expected(), "memory_module": _oracle_module(),
                   "expected": 3}
        enough = _driver_row(entry, batch=2, ns=10_000_000.0)
        with mock.patch.object(compare, "process", return_value=enough):
            row = compare.memory_observation(
                Path("/mock/engine"), fixture["path"], entry, 1,
                fixture["expected_memory"], "warm_call", 0, "decoded", 0,
                memory_module=fixture["memory_module"])
        self.assertEqual(row["batch"], 2)
        too_short = _driver_row(entry, batch=2, ns=9_999_999.0)
        with mock.patch.object(compare, "process", return_value=too_short):
            estimate = compare.memory_observation(
                Path("/mock/engine"), fixture["path"], entry, 1,
                fixture["expected_memory"], "warm_call", 0, "decoded", 0,
                memory_module=fixture["memory_module"])
        self.assertLess(estimate["batch"] * estimate["ns_per_operation"], 20_000_000)
        fixture.update(id=entry["id"], input=1, memory=True)
        requests = []
        def process(_exe, arguments, _cpu):
            requested = int(arguments[6])
            requests.append(requested)
            return too_short if requested == 0 else _driver_row(entry, batch=requested, ns=10_000_000.0)
        with mock.patch.object(compare, "process", side_effect=process):
            confirmed = compare._calibrate_memory(Path("/mock/engine"), fixture, "decoded", 0)
        self.assertEqual(requests, [0, 3])
        self.assertGreaterEqual(confirmed["batch"] * confirmed["ns_per_operation"], 20_000_000)

    def test_stage_contract_and_full_model_rows_fail_closed(self):
        entry = _memory_entry()
        compare._validate_memory_stage_contract(entry)
        compare._validate_memory_stage_contract(
            _memory_entry("memory_copy_forward_64", ["load_and_call"]))
        with self.assertRaisesRegex(compare.ComparisonError, "stage contract"):
            compare._validate_memory_stage_contract(_memory_entry(stages=["load_and_call"]))
        with self.assertRaisesRegex(compare.ComparisonError, "stage contract"):
            compare._validate_memory_stage_contract(
                _memory_entry("memory_copy_forward_64", compare.STAGES.copy()))
        compare._validate_memory_model_rows([entry], [dict(entry)])
        changed = dict(entry, expected_result=999)
        with self.assertRaisesRegex(compare.ComparisonError, "full memory registry row"):
            compare._validate_memory_model_rows([entry], [changed])

    def test_trap_outcome_normalizes_without_losing_timing_or_trap_metadata(self):
        entry = _memory_entry("memory_oob_copy_order", ["load_and_call"])
        expected = {"memory_index": 0, "initial_pages": 1,
                    "range": [0, PAGE], "bytes_hex": IMAGE.hex(),
                    "sha256": IMAGE_SHA, "full_memory_sha256": IMAGE_SHA,
                    "trap": "out_of_bounds", "preserved_prior_store": [8, 170]}
        row = _driver_row(entry, stage="load_and_call", batch=1, image=IMAGE,
                          outcome="trap")
        row["expected"] = "trap"
        row["trap_kind"] = "Some(MemoryOutOfBounds)"
        row["trap_message"] = "out of bounds"
        fixture = {"path": "/mock/oob.wasm", "entry": entry,
                   "expected_memory": expected, "expected": "trap"}
        with mock.patch.object(compare, "process", return_value=row):
            normalized = compare.memory_observation(
                Path("/mock/engine"), fixture["path"], entry, 1, expected,
                "load_and_call", 1, "decoded", 0, memory_module=_oracle_module())
        self.assertEqual(normalized["expected"], "trap")
        self.assertEqual(normalized["result"], "trap")
        self.assertEqual(normalized["ns_per_operation"], 10.0)
        self.assertEqual(normalized["memory"]["trap_kind"], "Some(MemoryOutOfBounds)")

    def test_build_growth_explanation_is_small_utf8_bound_and_source_bound(self):
        with tempfile.TemporaryDirectory() as directory:
            source=Path(directory)/'growth.md'
            source.write_text('Predeclared cumulative build-growth explanation.\n',encoding='utf-8')
            record=compare._read_build_growth_explanation(source)
            self.assertIn('cumulative build-growth',record['metadata']['relative_build_growth_explanation'])
            source_identity=record['metadata']['relative_build_growth_explanation_source']
            self.assertEqual(source_identity['sha256'],hashlib.sha256(source.read_bytes()).hexdigest())
            self.assertEqual(source_identity['path'],str(source.resolve()))
            compare._verify_build_growth_explanation(record)
            source.write_text('changed after measurement started\n',encoding='utf-8')
            with self.assertRaisesRegex(compare.ComparisonError,'changed during measurement'):
                compare._verify_build_growth_explanation(record)

            source.write_bytes(b'\xff')
            with self.assertRaisesRegex(compare.ComparisonError,'UTF-8'):
                compare._read_build_growth_explanation(source)
            source.write_bytes(b' '*16385)
            with self.assertRaisesRegex(compare.ComparisonError,'16 KiB'):
                compare._read_build_growth_explanation(source)
            source.write_bytes(b' \n\t')
            with self.assertRaisesRegex(compare.ComparisonError,'non-whitespace'):
                compare._read_build_growth_explanation(source)

    def test_common_memory_batch_uses_max_while_scalar_keeps_min(self):
        calibrations=[{'batch':64},{'batch':256}]
        self.assertEqual(compare._common_calibrated_batch({'memory':True},calibrations),256)
        self.assertEqual(compare._common_calibrated_batch({'scalar':True},calibrations),64)
        with self.assertRaisesRegex(compare.ComparisonError,'invalid batch'):
            compare._common_calibrated_batch({'memory':True},[{'batch':0},{'batch':64}])


def _oracle_module():
    class Oracle:
        @staticmethod
        def _expected_outcome(value):
            if value.get("trap") in {"trap", "out_of_bounds"}:
                return "trap"
            return f"i32:{value['result']}"

        @staticmethod
        def _profile(_entry):
            return "mvp"

        @staticmethod
        def _driver(_entry):
            return "plain"

    return Oracle


if __name__ == "__main__":
    unittest.main()
