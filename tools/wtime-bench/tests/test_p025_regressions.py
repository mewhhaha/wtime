"""Proposed fast regressions for the P025 compare CLI and memory metadata fixes."""
from __future__ import annotations

import os
from pathlib import Path
import sys
import unittest
from unittest import mock

TOOL_DIR = Path(os.environ["WTIME_BENCH_TOOL_DIR"]).resolve() if os.environ.get("WTIME_BENCH_TOOL_DIR") \
    else Path(__file__).resolve().parents[1]
sys.path.insert(0, str(TOOL_DIR))
import compare
import memory_workloads
import workloads


class P025CompareRegressionTests(unittest.TestCase):
    def test_cli_accepts_build_growth_explanation_file(self):
        explanation = Path("/tmp/predeclared-build-growth.md")
        argv = ["compare.py", "--baseline", "/tmp/baseline", "--candidate", "/tmp/candidate",
                "--selection", "memory", "--output", "/tmp/report", "--build-growth-explanation-file",
                str(explanation)]
        with mock.patch.object(sys, "argv", argv), mock.patch.object(compare, "run", return_value={}) as run:
            self.assertEqual(compare.main(), 0)
        self.assertEqual(run.call_args.args[0].build_growth_explanation_file, explanation)

    def test_registry_selection_includes_all_memory_model_rows_including_memory64(self):
        selected = workloads.selection("memory")
        memory_rows = [row for row in selected if row["id"] != "memory_fill_scan"
                       and row.get("oracle", {}).get("module") == "memory_workloads.py"]
        modeled = memory_workloads.entries()
        self.assertIn("memory64_i32_extended", {row["id"] for row in memory_rows})
        compare._validate_memory_model_rows(memory_rows, modeled)
        without_memory64 = [row for row in memory_rows if row["id"] != "memory64_i32_extended"]
        with self.assertRaisesRegex(compare.ComparisonError, "complete memory_workloads model"):
            compare._validate_memory_model_rows(without_memory64, modeled)

    def test_memory_wire_outcome_uses_scaling_oracle_without_module_hook(self):
        memory64_result = memory_workloads.expected_memory("memory64_i32_extended", 65535)
        boundary_trap = memory_workloads.expected_memory("memory64_i32_extended", memory_workloads.PAGE)
        opaque_module = object()  # memory_workloads intentionally has no _expected_outcome helper.
        self.assertEqual(compare._memory_wire_outcome(memory64_result, opaque_module), -128)
        self.assertEqual(compare._memory_wire_outcome(boundary_trap, opaque_module), "trap")


if __name__ == "__main__":
    unittest.main()
