"""Mocked fail-closed tests for the P015 integrity runner; source-bound integrity checks."""
from __future__ import annotations

import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import unittest
from unittest.mock import patch


SPEC = importlib.util.spec_from_file_location(
    "p015_scaling", Path(__file__).resolve().parents[1] / "representative_scaling.py")
assert SPEC is not None and SPEC.loader is not None
runner = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(runner)


class FakeModel:
    image = bytes([1, 2, 3, 4]) + bytes(65_532)
    output = image[:4]

    @staticmethod
    def oracle(_workload, _size):
        return {"result": 7, "output_sha256": hashlib.sha256(FakeModel.output).hexdigest(),
                "output_bytes": 4}

    @staticmethod
    def expected_memory_image(_workload, _size):
        return FakeModel.image


def make_entry():
    oracle = FakeModel.oracle("sample", 4)
    return {
        "id": "representative_sample_small", "params": [{"baseline": 4}],
        "expected_result": 7, "expected_output_sha256": oracle["output_sha256"],
        "expected_full_memory_sha256": hashlib.sha256(FakeModel.image).hexdigest(),
        "memory_page_count": 1, "output_range": [0, 4], "output_bytes": 4,
        "setup_policy": {"timed": False, "frequency": "before_each_timed_run"},
    }


def make_row(entry, *, mode="wasmtime", result="i32:7", image=None):
    expected = FakeModel.oracle(entry["id"], 4)
    image = FakeModel.image if image is None else image
    return {
        "schema_version": 2, "case": entry["id"], "stage": runner.STAGE,
        "input": "4", "expected": "i32:7", "result": result,
        "mode": mode, "profile": "mvp", "expected_output_sha256": expected["output_sha256"],
        "batch": 1, "ns_per_operation": 12.5,
        "memory_pages": 1, "memory_page_size_log2": 16,
        "memory_hex": image.hex(),
    }


class RepresentativeRunnerTests(unittest.TestCase):
    def test_full_memory_image_mismatch_fails_even_if_output_slice_matches(self):
        entry = make_entry()
        changed = bytearray(FakeModel.image)
        changed[500] = 1
        row = make_row(entry, image=bytes(changed))
        with self.assertRaisesRegex(runner.RepresentativeError, "full memory image"):
            runner._validate_observation(row, entry, FakeModel, "wasmtime", "mvp")

    def test_semantic_result_mismatch_is_not_reported_as_unsupported(self):
        entry = make_entry()
        row = make_row(entry, result="i32:8")
        with self.assertRaisesRegex(runner.RepresentativeError, "result"):
            runner._validate_observation(row, entry, FakeModel, "wasmtime", "mvp")

    def test_source_or_binary_change_invalidates_snapshot(self):
        before = {"/frozen/source.rs": "a" * 64, "/candidate": "b" * 64}
        changed = dict(before, **{"/candidate": "c" * 64})
        with self.assertRaisesRegex(runner.RepresentativeError, "identity changed"):
            runner.assert_source_stable(before, changed)

    def test_supported_wasmtime_sample_is_validated_and_retained(self):
        entry = make_entry()
        response = make_row(entry)
        fake_process = subprocess.CompletedProcess(
            args=["wasmtime"], returncode=0, stdout=json.dumps(response), stderr="")
        with patch.object(runner, "_run", return_value=fake_process):
            result = runner._invoke(Path("/mock/wasmtime"), "wasmtime", Path("/fixture.wasm"),
                                    entry, FakeModel, 1.0, wasmtime=True)
        self.assertEqual(result["status"], "ok")
        self.assertEqual(result["result"], 7)
        self.assertEqual(result["raw"]["mode"], "wasmtime")

    def test_unsupported_requires_explicit_compile_failure_evidence(self):
        self.assertTrue(runner._compile_unsupported({
            "status": "unsupported", "failure_stage": "compile", "reason": "feature rejected"}))
        self.assertFalse(runner._compile_unsupported({
            "status": "unsupported", "failure_stage": "execute", "reason": "result mismatch"}))
        self.assertFalse(runner._compile_unsupported({
            "status": "unsupported", "failure_stage": "compile", "reason": ""}))


if __name__ == "__main__":
    unittest.main()
