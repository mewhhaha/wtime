"""Focused P071 model and fixture-contract tests; no guest execution."""
from __future__ import annotations
import re
import shutil
import sys
import tempfile
import unittest
from pathlib import Path
TOOL_DIR = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(TOOL_DIR))
import gc_collect_workloads as gcw

class GcCollectWorkloadTests(unittest.TestCase):
    def test_oracle_is_exact_for_all_nine_shapes(self):
        rows = gcw.entries()
        self.assertEqual(len(rows), 9)
        self.assertEqual(len({row["id"] for row in rows}), 9)
        for row in rows:
            n = row["oracle_arguments"]["repetitions"]
            count = gcw.FAMILIES[row["family"]]["count"]
            objects = count + 1 if gcw.FAMILIES[row["family"]]["shape"] == "wide" else count
            expected = count * (count - 1) // 2 + n * objects
            self.assertEqual(row["expected_result"], expected)
            self.assertEqual(gcw.oracle(row["id"], n), expected)

    def test_rows_bind_positive_allocation_and_stage_contract(self):
        for row in gcw.entries():
            count = gcw.FAMILIES[row["family"]]["count"]
            n = row["oracle_arguments"]["repetitions"]
            objects = count + 1 if gcw.FAMILIES[row["family"]]["shape"] == "wide" else count
            self.assertEqual(row["gc_expectation"]["allocation_count"], objects)
            self.assertEqual(row["callbacks_count"], n)
            self.assertEqual(row["gc_expectation"]["callback_live"], objects)
            self.assertEqual(row["gc_expectation"]["live_after_clear_and_collect"], 0)
            self.assertEqual(row["gc_expectation"]["allocations_in_run"], 0)
            self.assertEqual(row["stages"], ["prepare", "instantiate", "first_call", "warm_call", "load_and_call"])
            self.assertEqual(row["reset_policy"], "fresh_instance_for_cold_stages_reused_instance_for_warm_call")
            self.assertEqual(row["warm_state_policy"], "one initialized graph retained until final clear and collection")
            self.assertEqual(row["max_batch"], min(100_000, 100_000_000 // ((count + 1) * (n + 1))))
            self.assertGreater(row["work_units"]["count"], 0)

    def test_wide_and_chain_fixture_shapes_are_distinct_and_fixed(self):
        for family in ("gc_collect_wide_32", "gc_collect_wide_1024"):
            record = gcw.FAMILIES[family]
            wat = (gcw.FIXTURE_DIR / record["file"]).read_text()
            self.assertIn(f"array.new_fixed $roots_type {record['count']}", wat)
            values = [int(value) for value in re.findall(r"\(struct\.new \$leaf \(i32\.const (\d+)\)\)", wat)]
            self.assertEqual(values, list(range(record["count"])))
            self.assertIn("(array.get $roots_type", wat)
            run_body = wat.split('(func (export "run")', 1)[1].split('(func (export "clear")', 1)[0]
            self.assertNotIn("struct.new", run_body)
            self.assertNotIn("array.new", run_body)
            self.assertIn('(func (export "init")', wat)
            self.assertIn('(func (export "clear")', wat)
        wat = (gcw.FIXTURE_DIR / gcw.FAMILIES["gc_collect_chain_256"]["file"]).read_text()
        self.assertIn("(type $node (struct (field i32) (field (ref null $node))))", wat)
        self.assertIn("(loop $build", wat)
        self.assertIn("(loop $scan", wat)
        run_body = wat.split('(func (export "run")', 1)[1].split('(func (export "clear")', 1)[0]
        self.assertNotIn("struct.new", run_body)

    def test_fixture_hashes_match_and_escape_is_rejected(self):
        self.assertEqual(set(gcw.fixture_identities()), set(gcw.FAMILIES))
        with tempfile.TemporaryDirectory(prefix="p071-fixtures-") as temp:
            root = Path(temp) / "fixtures"
            root.mkdir()
            for record in gcw.FAMILIES.values():
                src = gcw.FIXTURE_DIR / record["file"]
                shutil.copyfile(src, root / record["file"])
                shutil.copyfile(src.with_suffix(".wasm"), (root / record["file"]).with_suffix(".wasm"))
            outside = Path(temp) / "outside.wat"
            outside.write_text("(module)\n")
            name = next(iter(gcw.FAMILIES.values()))["file"]
            (root / name).unlink()
            (root / name).symlink_to(outside)
            with self.assertRaisesRegex(ValueError, "escapes fixture root"):
                gcw.fixture_identities(root)
            (root / name).unlink()
            shutil.copyfile(gcw.FIXTURE_DIR / name, root / name)
            wasm_path = (root / name).with_suffix(".wasm")
            wasm_path.unlink()
            outside_wasm = Path(temp) / "outside.wasm"
            outside_wasm.write_bytes((gcw.FIXTURE_DIR / name).with_suffix(".wasm").read_bytes())
            wasm_path.symlink_to(outside_wasm)
            with self.assertRaisesRegex(ValueError, "escapes fixture root"):
                gcw.fixture_identities(root)

    def test_oracle_rejects_unknown_or_mismatched_input(self):
        with self.assertRaises(ValueError): gcw.oracle("gc_collect_wide_32_n0", 0)
        with self.assertRaises(ValueError): gcw.oracle("gc_collect_wide_32_n17", 32)
        with self.assertRaises(ValueError): gcw.oracle("unknown_n1", 1)

if __name__ == "__main__":
    unittest.main()
