"""Unit tests for the installed P033 GC workload model and append contract."""
from __future__ import annotations

import copy
import importlib.util
import json
import os
from pathlib import Path
import unittest
from unittest import mock


TOOL_DIR = Path(os.environ.get("WTIME_BENCH_TOOL_DIR", Path(__file__).resolve().parents[1])).resolve()
MODEL_PATH = TOOL_DIR / "gc_warm_workloads.py"
REGISTRY_PATH = TOOL_DIR / "workloads.json"
spec = importlib.util.spec_from_file_location("p033_gc_warm_workloads_tested", MODEL_PATH)
if spec is None or spec.loader is None:
    raise RuntimeError(f"cannot load installed P033 model: {MODEL_PATH}")
model = importlib.util.module_from_spec(spec)
spec.loader.exec_module(model)

ORIGINAL_WORKLOAD_COUNT = 314
LEGACY_GC_SELECTION_COUNT = 45
EXPECTED_INPUTS = (1, 32, 1024, 10000, 0, 17)
EXPECTED_FAMILIES = {
    "gc_nominal_exact_casts", "gc_typed_function_refs", "gc_type_churn_64",
    "gc_type_churn_256", "gc_type_churn_1024", "gc_indirect_subtype_calls",
}


def signed32(value: int) -> int:
    value &= 0xFFFF_FFFF
    return value - 0x1_0000_0000 if value & 0x8000_0000 else value


def independent_result(family: str, n: int) -> int:
    if family == "gc_nominal_exact_casts":
        return signed32(2 * n)
    if family == "gc_typed_function_refs":
        return signed32(n * (n + 1) // 2)
    if family == "gc_indirect_subtype_calls":
        return signed32(128 * n)
    width = int(family.rsplit("_", 1)[1])
    quotient, remainder = divmod(n, width)
    return signed32(quotient * width * (width - 1) // 2 + remainder * (remainder - 1) // 2)


def reconstruct_pre_p033_registry(current: dict) -> dict:
    """Recover the pre-append logical prefix without reading historical artifacts."""
    base = copy.deepcopy(current)
    base["workloads"] = copy.deepcopy(current["workloads"][:ORIGINAL_WORKLOAD_COUNT])
    base_ids = {row["id"] for row in base["workloads"]}
    for selection_name, selection in base["selections"].items():
        current_ids = current["selections"][selection_name]["workloads"]
        if selection_name == "gc-exceptions":
            selection["workloads"] = copy.deepcopy(current_ids[:LEGACY_GC_SELECTION_COUNT])
        else:
            selection["workloads"] = [wid for wid in current_ids if wid in base_ids]
    return base


class P033GcModelTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.registry = json.loads(REGISTRY_PATH.read_text())

    def test_all_six_fixture_wat_and_wasm_hashes_are_present_and_bound(self):
        identities = model.fixture_identities()
        self.assertEqual(set(identities), set(model.FAMILIES))
        self.assertEqual(len(identities), 6)
        for family, record in model.FAMILIES.items():
            self.assertRegex(record["wat_sha256"], r"^[0-9a-f]{64}$")
            self.assertRegex(record["wasm_sha256"], r"^[0-9a-f]{64}$")
            self.assertEqual(identities[family]["wat_sha256"], record["wat_sha256"])
            self.assertEqual(identities[family]["wasm_sha256"], record["wasm_sha256"])

    def test_exact_36_rows_cover_six_families_and_independent_oracles(self):
        rows = model.entries()
        self.assertEqual(len(rows), 36)
        self.assertEqual({row["family"] for row in rows}, EXPECTED_FAMILIES)
        expected_ids = [f"{family}_n{n}" for family in model.FAMILIES for n in EXPECTED_INPUTS]
        self.assertEqual([row["id"] for row in rows], expected_ids)
        for row in rows:
            n = row["oracle_arguments"]["n"]
            self.assertEqual(row["params"], [{"name": "n", "type": "i32", "min": n,
                                               "max": n, "baseline": n}])
            self.assertEqual(row["result"], {"type": "i32"})
            self.assertEqual(row["oracle"], {"kind": "python", "module": "gc_warm_workloads.py",
                                              "function": "oracle", "independent": True})
            self.assertEqual(model.oracle(row["id"], n), independent_result(row["family"], n))
            self.assertEqual(row["gc_expectation"]["oracle_result"], independent_result(row["family"], n))
            expected_allocations = n if row["family"] == "gc_nominal_exact_casts" else 0
            self.assertEqual(row["gc_expectation"]["allocation_count"], expected_allocations)
            self.assertEqual(row["gc_expectation"]["live_after_reset"], 0)
            self.assertEqual(row["gc_expectation"]["payload_bytes_after_reset"], 0)
            self.assertEqual(row["gc_expectation"]["continuation_bytes_after_reset"], 0)
            self.assertEqual(row["gc_expectation"]["reset_in_timing"], False)
            self.assertEqual(row["source"]["encoding_status"], "encoded")
            self.assertEqual(row["source"]["wasm_sha256"], model.FAMILIES[
                next(key for key in model.FAMILIES if row["id"].startswith(key + "_n"))
            ]["wasm_sha256"])

    def test_inputs_ids_and_signed_oracle_ranges_reject_invalid_values(self):
        for invalid in (True, False, -1, 10001, 1.0, "1", None):
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                model._count(invalid)
        for bad_id, n in (
            ("gc_type_churn_64_n1", 32),
            ("gc_type_churn_64_n01", 1),
            ("gc_type_churn_64_n10001", 10001),
            ("gc_unknown_n1", 1),
        ):
            with self.subTest(bad_id=bad_id, n=n), self.assertRaises(ValueError):
                model.oracle(bad_id, n)
        for family in model.FAMILIES:
            for n in EXPECTED_INPUTS:
                value = model.oracle(f"{family}_n{n}", n)
                self.assertGreaterEqual(value, -(1 << 31))
                self.assertLessEqual(value, (1 << 31) - 1)

    def test_append_preserves_reconstructed_prefix_selections_and_input(self):
        before = reconstruct_pre_p033_registry(self.registry)
        before_saved = copy.deepcopy(before)
        old_workloads = copy.deepcopy(before["workloads"])
        old_selections = copy.deepcopy(before["selections"])
        added = model.entries()
        updated = model.append_to_registry(before)
        self.assertEqual(before, before_saved, "append helper must not mutate its input")
        self.assertEqual(updated["workloads"][:ORIGINAL_WORKLOAD_COUNT], old_workloads)
        self.assertEqual(updated["workloads"][ORIGINAL_WORKLOAD_COUNT:], added)
        self.assertEqual(len(updated["workloads"]), ORIGINAL_WORKLOAD_COUNT + 36)
        gc_ids = old_selections["gc-exceptions"]["workloads"]
        added_ids = [row["id"] for row in added]
        self.assertEqual(updated["selections"]["gc-exceptions"]["workloads"], gc_ids + added_ids)
        self.assertEqual(len(updated["selections"]["gc-exceptions"]["workloads"]), 81)
        for name, selection in old_selections.items():
            if name != "gc-exceptions":
                self.assertEqual(updated["selections"][name], selection)
        self.assertEqual(updated["selections"]["gc-exceptions"]["status"], "ready")
        with self.assertRaises(ValueError):
            model.append_to_registry(updated)

    def test_append_rejects_bad_hash_and_missing_wasm_without_touching_fixtures(self):
        families = copy.deepcopy(model.FAMILIES)
        family_name = next(iter(families))
        families[family_name]["wat_sha256"] = "0" * 64
        with mock.patch.object(model, "FAMILIES", families):
            with self.assertRaisesRegex(ValueError, "WAT identity mismatch"):
                model.entries()

        families = copy.deepcopy(model.FAMILIES)
        families[family_name]["wasm_sha256"] = None
        with mock.patch.object(model, "FAMILIES", families):
            with self.assertRaisesRegex(ValueError, "Wasm SHA-256 is missing"):
                model.entries()


if __name__ == "__main__":
    unittest.main()
