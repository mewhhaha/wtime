"""Focused P125 fixed-bank GC root model tests."""
from __future__ import annotations

import importlib.util
import json
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

TOOL_DIR = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(TOOL_DIR))
import root_workloads
import workloads


class RootWorkloadTests(unittest.TestCase):
    def test_independent_oracle_matches_closed_form_for_every_case(self):
        for family, record in root_workloads.FAMILIES.items():
            bank = record["bank"]
            for n in root_workloads.INPUTS:
                with self.subTest(family=family, n=n):
                    if record["shape"] == "numeric":
                        expected = bank * n + bank * (bank - 1) // 2 + n + 42 * n
                    else:
                        expected = bank * n + n + 42 * n
                    self.assertEqual(root_workloads.oracle(f"{family}_n{n}", n), expected)

    def test_rows_are_fixed_bank_positive_units_with_bounded_batches(self):
        rows = root_workloads.entries()
        self.assertEqual(len(rows), 24)
        self.assertEqual(len({row["id"] for row in rows}), 24)
        for row in rows:
            n = row["oracle_arguments"]["n"]
            bank = root_workloads.FAMILIES[row["family"]]["bank"]
            self.assertEqual(row["callbacks_count"], n)
            self.assertEqual(row["work_units"]["count"], bank + n)
            self.assertGreater(row["work_units"]["count"], 0)
            self.assertEqual(row["max_batch"], min(100_000, 100_000_000 // ((bank + 8) * (n + 1))))
            self.assertEqual(row["expected_outcome"], "return")
            self.assertEqual(row["gc_expectation"]["oracle_result"], row["expected_result"])
            self.assertEqual(row["gc_expectation"]["callbacks_per_call"], n)
            self.assertEqual(row["gc_expectation"]["bank_slots"], bank)
            self.assertEqual(row["gc_expectation"]["callback_live"], 1 if n else 0)

    def test_fixture_hashing_is_contained_in_fresh_temporary_root(self):
        with tempfile.TemporaryDirectory(prefix="p125-root-fixtures-") as temp:
            root = Path(temp) / "fixtures"
            root.mkdir()
            for record in root_workloads.FAMILIES.values():
                source = root_workloads.FIXTURE_DIR / record["file"]
                shutil.copyfile(source, root / record["file"])
                shutil.copyfile(source.with_suffix(".wasm"), (root / record["file"]).with_suffix(".wasm"))
            identities = root_workloads.fixture_identities(root)
            self.assertEqual(set(identities), set(root_workloads.FAMILIES))
            escaped = Path(temp) / "outside.wat"
            escaped.write_text("(module)\n")
            first = next(iter(root_workloads.FAMILIES.values()))["file"]
            (root / first).unlink()
            (root / first).symlink_to(escaped)
            with self.assertRaisesRegex(ValueError, "escapes fixture root"):
                root_workloads.fixture_identities(root)

    def test_registry_adds_only_the_new_selection(self):
        registry = workloads.load_registry()
        new_rows = [row for row in registry["workloads"] if row["id"].startswith("gc_root_snapshots_")]
        self.assertEqual([row["id"] for row in workloads.selection("gc-root-snapshots", registry)],
                         [row["id"] for row in root_workloads.entries()])
        self.assertEqual(len(new_rows), 24)
        self.assertEqual(registry["selections"]["gc-root-snapshots"], {
            "status": "ready", "workloads": [row["id"] for row in new_rows]
        })


if __name__ == "__main__":
    unittest.main()
