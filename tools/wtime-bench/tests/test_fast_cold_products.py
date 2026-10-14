"""Safety and accounting checks for cold preparation product aliases."""
from pathlib import Path
import sys
import tempfile
import unittest

BENCH = Path(__file__).resolve().parents[1]
sys.dont_write_bytecode = True
sys.path.insert(0, str(BENCH))

import cold_products


class FastColdProductsTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="fast-cold-products-")
        self.root = Path(self.temporary.name).resolve()
        self.addCleanup(self.temporary.cleanup)

    def inside(self, relative: str) -> Path:
        path = (self.root / relative).resolve()
        self.assertTrue(path.is_relative_to(self.root), "synthetic output escaped fresh temp root")
        return path

    def product(self, workload, **changes):
        row = {
            "workload": workload,
            "stage": "prepare",
            "wasm_sha256": "a" * 64,
            "profile": "baseline",
            "runtime_settings": {"mode": "interpreted", "fuel": 1000},
            "timer": {"driver": "cold-runner", "source_sha256": "b" * 64},
            "setup": {"recipe": "compile-and-instantiate", "version": 1},
        }
        row.update(changes)
        return row

    def test_identical_preparation_products_alias_to_one_lexical_sample(self):
        products = [self.product("zeta"), self.product("alpha"), self.product("middle")]
        result = cold_products.plan(products)

        self.assertEqual(result["logical_groups"], 3)
        self.assertEqual(result["unique_groups"], 1)
        self.assertEqual(result["aliases"], 2)
        self.assertEqual(result["groups"][0]["sample_workload"], "alpha")
        self.assertEqual(result["groups"][0]["members"], ["alpha", "middle", "zeta"])
        self.assertEqual(result["uncertainty"], cold_products.UNCERTAINTY)

    def test_each_product_identity_change_splits_preparation_group(self):
        base = self.product("base")
        cases = [
            self.product("profile", profile="optimized"),
            self.product("fuel", runtime_settings={"mode": "interpreted", "fuel": 1001}),
            self.product("timer", timer={"driver": "other", "source_sha256": "b" * 64}),
            self.product("setup", setup={"recipe": "different", "version": 1}),
            self.product("wasm", wasm_sha256="c" * 64),
        ]
        result = cold_products.plan([base, *cases])

        self.assertEqual(result["unique_groups"], len(cases) + 1)
        self.assertEqual(result["aliases"], 0)

    def test_input_sensitive_stages_never_alias_even_with_identical_products(self):
        stages = ("first_call", "warm_call", "load_and_call", "instantiate")
        products = [self.product(f"{stage}-{workload}", stage=stage)
                    for stage in stages for workload in ("input-a", "input-b")]
        result = cold_products.plan(products)

        self.assertEqual(result["unique_groups"], 8)
        self.assertTrue(all(len(group["members"]) == 1 for group in result["groups"]))

    def test_verify_rejects_tampered_alias_membership(self):
        products = [self.product("a"), self.product("b")]
        record = cold_products.plan(products)
        altered = {**record, "groups": [dict(record["groups"][0], members=["a"])]}

        with self.assertRaisesRegex(ValueError, "differs from recomputed"):
            cold_products.verify(altered, products)
        with self.assertRaisesRegex(ValueError, "differs from recomputed"):
            cold_products.verify({**record, "logical_groups": True}, products)

    def test_summarize_counts_shared_pairs_once_and_weights_logical_references(self):
        products = [self.product("a"), self.product("b"), self.product("solo", setup={"recipe": "solo"})]
        record = cold_products.plan(products)
        counts = {f"{group['sample_workload']}\0{group['stage']}": 5
                  for group in record["groups"]}

        result = cold_products.summarize(record, counts)

        self.assertEqual(result["unique_pair_count"], 10)
        self.assertEqual(result["logical_pair_references"], 15)
        self.assertEqual(result["extra_samples"], 0)
        self.assertIn("no extra samples", result["sampling_note"])

    def test_summarize_requires_exact_canonical_positive_sample_counts(self):
        record = cold_products.plan([self.product("a"), self.product("b")])
        good = {f"{record['groups'][0]['sample_workload']}\0prepare": 2}

        with self.assertRaisesRegex(ValueError, "exactly match"):
            cold_products.summarize(record, {**good, "b\0prepare": 2})
        with self.assertRaisesRegex(ValueError, "positive integers"):
            cold_products.summarize(record, {next(iter(good)): 0})

    def test_rejects_duplicate_and_malformed_products(self):
        with self.assertRaisesRegex(ValueError, "duplicate"):
            cold_products.plan([self.product("same"), self.product("same")])
        with self.assertRaisesRegex(ValueError, "64 hexadecimal"):
            cold_products.plan([self.product("bad", wasm_sha256="not-a-digest")])


if __name__ == "__main__":
    unittest.main()
