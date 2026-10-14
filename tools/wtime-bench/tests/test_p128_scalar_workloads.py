"""Model, fixture, and frozen-registry checks for the P128 foundation."""
from __future__ import annotations

import hashlib
import importlib.util
import json
from pathlib import Path
import re
import shutil
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[3]
BENCH = ROOT / "tools/wtime-bench"
MODEL_PATH = BENCH / "scalar_p128_workloads.py"
MODEL_SPEC = importlib.util.spec_from_file_location("scalar_p128_workloads_test", MODEL_PATH)
MODEL = importlib.util.module_from_spec(MODEL_SPEC)
assert MODEL_SPEC is not None and MODEL_SPEC.loader is not None
MODEL_SPEC.loader.exec_module(MODEL)
REGISTRY = BENCH / "workloads.json"
ADMITTED_WASM = ROOT / "tools/verification/tests/data/admitted_wasm"
REGISTRY_SHA256 = "8917f14d06b2dd69d1abb1885d8186cdc11191924aef8ee073e356cec5cfd668"


def sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


class P128ScalarWorkloadTests(unittest.TestCase):
    def test_fixed_alias_grid_is_exact_and_registry_stays_459_rows(self):
        before = REGISTRY.read_bytes()
        self.assertEqual(hashlib.sha256(before).hexdigest(), REGISTRY_SHA256)
        registry = json.loads(before)
        self.assertEqual(len(registry["workloads"]), 459)
        selections_before = {
            name: tuple(selection["workloads"])
            for name, selection in registry["selections"].items()
        }
        rows = MODEL.entries()
        expected = {f"{family}_n{n}" for family in MODEL.FIXTURES for n in MODEL.FIXED_INPUTS}
        self.assertEqual(len(rows), 18)
        self.assertEqual({row["id"] for row in rows}, expected)
        self.assertFalse(expected.intersection(row["id"] for row in registry["workloads"]))
        self.assertEqual(hashlib.sha256(REGISTRY.read_bytes()).hexdigest(), REGISTRY_SHA256)
        self.assertEqual({name: tuple(v["workloads"]) for name, v in registry["selections"].items()},
                         selections_before)

    def test_identity_copy_fixture_has_an_independent_n_oracle(self):
        source = (BENCH / "fixtures/scalar/p128_identity_copy.wat").read_text()
        self.assertIn("local.get $acc\n        local.set $acc", source)
        self.assertIn("i32.const 1\n        local.get $acc\n        i32.add", source)
        for n in MODEL.FIXED_INPUTS:
            with self.subTest(n=n):
                self.assertEqual(MODEL.oracle(f"p128_identity_copy_n{n}", n), n)

    def test_literal_add_wraps_as_i32_without_float_or_trapping_ops(self):
        source = (BENCH / "fixtures/scalar/p128_literal_wrap_add.wat").read_text()
        self.assertIn("i32.const 2147483647\n        i32.const 1\n        i32.add", source)
        self.assertEqual(tuple(MODEL.oracle(f"p128_literal_wrap_add_n{n}", n)
                               for n in MODEL.FIXED_INPUTS),
                         (0, -2147483648, -2147483648, 0, 0, 0))

    def test_literal_bitwise_fixture_matches_independent_mask_oracle(self):
        source = (BENCH / "fixtures/scalar/p128_literal_bitwise.wat").read_text()
        self.assertIn("i32.const 324508639\n        i32.const -1\n        i32.and", source)
        expected = tuple(MODEL._signed_i32((n * 0x13579BDF) & 0xFFFFFFFF)
                         for n in MODEL.FIXED_INPUTS)
        actual = tuple(MODEL.oracle(f"p128_literal_bitwise_n{n}", n)
                       for n in MODEL.FIXED_INPUTS)
        self.assertEqual(actual, expected)

    def test_fixture_work_count_is_exactly_n_and_invalid_aliases_fail(self):
        rows = {row["id"]: row for row in MODEL.entries()}
        expected_events = {0: 9, 1: 23, 17: 247, 32: 457, 1024: 14345, 10000: 140009}
        for workload_id, row in rows.items():
            n = row["oracle_arguments"]["n"]
            self.assertEqual(row["work_units"]["count"], n)
            self.assertEqual(row["source_event_model"]["dynamic"], expected_events[n])
        for alias, n in (
            ("p128_identity_copy_n2", 2),
            ("p128_identity_copy_n01", 1),
            ("p128_identity_copy_n1_extra", 1),
            ("p128_unknown_n1", 1),
            ("p128_literal_wrap_add_n1", 17),
            ("p128_identity_copy_n-1", -1),
        ):
            with self.subTest(alias=alias, n=n), self.assertRaises(ValueError):
                MODEL.oracle(alias, n)
        with self.assertRaises(ValueError):
            MODEL.oracle("p128_identity_copy_n1", True)
        with self.assertRaises(ValueError):
            MODEL.source_event_count(2)

    def test_wat_opcode_shape_rejects_multiply_divide_float_memory_or_calls(self):
        forbidden = re.compile(
            r"\b(?:i32\.mul|i32\.div_[su]|f32\.|f64\.|memory\.|call(?:_ref|_indirect)?|ref\.)"
        )
        for family, metadata in MODEL.FIXTURES.items():
            source = (BENCH / "fixtures/scalar" / metadata["file"]).read_text()
            self.assertIsNone(forbidden.search(source), family)

    def test_admitted_wat_and_wasm_bytes_match_frozen_hashes(self):
        for family, metadata in MODEL.FIXTURES.items():
            wat = BENCH / "fixtures/scalar" / metadata["file"]
            wasm = ADMITTED_WASM / wat.with_suffix(".wasm").name
            with self.subTest(family=family):
                self.assertEqual(sha(wat), metadata["wat_sha256"])
                self.assertEqual(sha(wasm), metadata["wasm_sha256"])

    def test_rows_bind_profile_modes_stages_oracle_and_mvp_work_units(self):
        rows = MODEL.entries()
        self.assertEqual(tuple(MODEL.STAGES),
                         ("prepare", "instantiate", "first_call", "warm_call", "load_and_call"))
        self.assertEqual(tuple(MODEL.MODES), ("decoded", "compact"))
        for row in rows:
            self.assertEqual(row["profile"], "mvp")
            self.assertEqual(row["required_features"], ["mvp"])
            self.assertEqual(row["stages"], list(MODEL.STAGES))
            self.assertEqual(row["modes"], list(MODEL.MODES))
            self.assertTrue(row["oracle"]["independent"])
            self.assertEqual(row["result"]["type"], "i32")

    def test_frozen_registry_rows_and_selections_survive_model_generation(self):
        before = REGISTRY.read_bytes()
        frozen = json.loads(before)
        with tempfile.TemporaryDirectory(prefix="p128-frozen-registry-") as temp:
            root = Path(temp).resolve()
            copied = root / "workloads.json"
            shutil.copyfile(REGISTRY, copied)
            rows = MODEL.entries()
            self.assertEqual(len(rows), 18)
            self.assertEqual(copied.resolve().parent, root)
            after = copied.read_bytes()
            self.assertEqual(after, before)
            self.assertEqual(json.loads(after)["workloads"], frozen["workloads"])
            self.assertEqual(json.loads(after)["selections"], frozen["selections"])
        self.assertEqual(REGISTRY.read_bytes(), before)


if __name__ == "__main__":
    unittest.main()
