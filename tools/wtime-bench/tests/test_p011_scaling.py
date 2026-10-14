"""Mock-only fail-closed tests for the staged P011 runner draft."""
import json
import sys
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import gc_scaling as runner


class P011RunnerTests(unittest.TestCase):
    def test_single_and_cross_module_dispatch_use_the_public_driver_commands(self):
        candidate = Path("/tmp/p011-candidate")
        wasm = Path("/tmp/p011-case.wasm")
        row = {"schema_version": 2, "case": "mock", "input": "i32:32"}
        completed = mock.Mock(returncode=0, stdout=json.dumps(row), stderr="")
        with mock.patch.object(runner.subprocess, "run", return_value=completed) as call:
            got, command = runner._invoke(candidate, {"id": "mock"}, wasm, [], 32, 1.0)
        self.assertEqual(got, row)
        self.assertEqual(command, [str(candidate), "--gc-exception-driver", str(wasm),
                                   "mock", "i32:32", "decoded", "all-phase3-core"])
        provider = Path("/tmp/provider.wasm")
        with mock.patch.object(runner.subprocess, "run", return_value=completed):
            _, command = runner._invoke(candidate,
                {"id": "gc_descriptor_cross_module_n32"}, wasm,
                [(Path("descriptor_provider.wat"), provider)], 32, 1.0)
        self.assertEqual(command, [str(candidate), "--gc-exception-cross-module",
                                   str(provider), str(wasm), "i32:32",
                                   "decoded", "all-phase3-core"])

    def test_uncaught_exception_tag_and_payload_are_checked_against_oracle(self):
        entry = {"id": "exception_uncaught_throw_n32"}
        row = {"schema_version": 2, "case": entry["id"], "input": "i32:32",
               "mode": "decoded", "profile": "all-phase3-core", "instrumented": False,
               "driver": "single", "report_complete": True,
               "wasmtime": {"status": "unsupported", "reason": "not built"},
               "outcome": "exception", "tag": "wrong", "payload": ["i32:32"]}
        model = SimpleNamespace(oracle=lambda _workload, _n: {
            "kind": "exception", "tag": "number", "payload": [32]})
        with self.assertRaisesRegex(runner.MeasurementError, "exception tag differs"):
            runner._validate_row(entry, row, 32, model, "a" * 64, "b" * 64)

    def test_continuation_reclamation_includes_consumed_continuation_record(self):
        entry = {"gc_expectation": {"allocation_count": 2}}
        row = {"collection": {"live": 0, "reclaimed": 3},
               "gc_payload_bytes": {"before_invoke": 0, "after_invoke": 64,
                                    "after_collect": 0},
               "continuation_bytes": {"after_collect": 0}}
        runner._check_gc_collection(entry, row, "gc_continuation_root_n32")
        row["collection"]["reclaimed"] = 2
        with self.assertRaisesRegex(runner.MeasurementError, "plus one consumed continuation"):
            runner._check_gc_collection(entry, row, "gc_continuation_root_n32")

    def test_any_source_or_executable_mutation_invalidates_the_run(self):
        before = {"/candidate": "a" * 64, "/workloads.py": "b" * 64}
        runner._assert_stable(before, dict(before))
        after = dict(before)
        after["/workloads.py"] = "c" * 64
        with self.assertRaisesRegex(runner.MeasurementError, "identity changed"):
            runner._assert_stable(before, after)


if __name__ == "__main__":
    unittest.main()
