"""Synthetic checks for retaining separate P094 portable cost-run inventories."""
from __future__ import annotations

import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(ROOT / "tools"), str(ROOT / "tools" / "verification" / "producers")]
from verification.artifacts import VerificationError  # noqa: E402
import p094  # noqa: E402


def inventory_document():
    return {
        "analysis": {"bootstrap_resamples": 10_000},
        "per_run": [
            {"run": 1, "pairs": 30, "stage": "warm_call",
             "candidate_over_baseline": {"median": 0.80},
             "median_ratio_95ci": {"lower": 0.75, "upper": 0.85}},
            {"run": 1, "pairs": 32, "stage": "first_call",
             "candidate_over_baseline": {"median": 0.90},
             "median_ratio_95ci": {"lower": 0.86, "upper": 0.94}},
            {"run": 2, "pairs": 31, "stage": "warm_call",
             "candidate_over_baseline": {"median": 1.10},
             "median_ratio_95ci": {"lower": 1.04, "upper": 1.16}},
            {"run": 2, "pairs": 35, "stage": "load_and_call",
             "candidate_over_baseline": {"median": 1.20},
             "median_ratio_95ci": {"lower": 1.12, "upper": 1.28}},
        ],
    }


def write_reference(root: Path, name: str, value: dict) -> dict:
    path = root / name
    path.write_text(json.dumps(value, allow_nan=True), encoding="utf-8")
    return {"path": str(path.resolve()), "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
            "bytes": path.stat().st_size}


class P094CostInventoryTests(unittest.TestCase):
    def test_two_complete_runs_remain_separate_and_retain_statistics_references(self):
        with tempfile.TemporaryDirectory(prefix="p094-cost-inventory-", dir="/tmp") as temporary:
            root = Path(temporary).resolve()
            reference = write_reference(root, "portable-stats.json", inventory_document())
            self.assertTrue(Path(reference["path"]).is_relative_to(root))
            result = p094.cost_inventory({"fixture-family": reference})["fixture-family"]

            self.assertEqual(result["statistics"], reference)
            self.assertEqual(set(result["runs"]), {1, 2})
            self.assertEqual(result["runs"][1], {
                "groups": 2, "warm_groups": 1, "cold_groups": 1,
                "ratio_minimum": 0.8, "ratio_maximum": 0.9,
            })
            self.assertEqual(result["runs"][2], {
                "groups": 2, "warm_groups": 1, "cold_groups": 1,
                "ratio_minimum": 1.1, "ratio_maximum": 1.2,
            })
            self.assertNotIn("aggregate", result)
            self.assertIn("no aggregate speedup", result["scope"])

    def test_incomplete_and_boolean_pair_run_or_bootstrap_counts_fail(self):
        mutations = []

        missing_run = inventory_document()
        missing_run["per_run"] = [row for row in missing_run["per_run"] if row["run"] == 1]
        mutations.append(missing_run)

        bad_run = inventory_document()
        bad_run["per_run"][0]["run"] = True
        mutations.append(bad_run)

        bad_pairs = inventory_document()
        bad_pairs["per_run"][0]["pairs"] = 29
        mutations.append(bad_pairs)

        boolean_pairs = inventory_document()
        boolean_pairs["per_run"][0]["pairs"] = True
        mutations.append(boolean_pairs)

        boolean_bootstrap = inventory_document()
        boolean_bootstrap["analysis"]["bootstrap_resamples"] = True
        mutations.append(boolean_bootstrap)

        insufficient_bootstrap = inventory_document()
        insufficient_bootstrap["analysis"]["bootstrap_resamples"] = 9_999
        mutations.append(insufficient_bootstrap)

        with tempfile.TemporaryDirectory(prefix="p094-cost-counts-", dir="/tmp") as temporary:
            root = Path(temporary).resolve()
            for index, document in enumerate(mutations):
                with self.subTest(index=index):
                    reference = write_reference(root, f"invalid-{index}.json", document)
                    self.assertTrue(Path(reference["path"]).is_relative_to(root))
                    with self.assertRaises((VerificationError, ValueError)):
                        p094.cost_inventory({"fixture-family": reference})

    def test_nonfinite_nonpositive_or_reversed_ratio_intervals_fail(self):
        mutations = []

        for invalid in (0, -0.5, float("inf")):
            document = inventory_document()
            document["per_run"][0]["candidate_over_baseline"]["median"] = invalid
            mutations.append(document)

        nonpositive_lower = inventory_document()
        nonpositive_lower["per_run"][0]["median_ratio_95ci"]["lower"] = 0
        mutations.append(nonpositive_lower)

        nonfinite_upper = inventory_document()
        nonfinite_upper["per_run"][0]["median_ratio_95ci"]["upper"] = float("nan")
        mutations.append(nonfinite_upper)

        reversed_interval = inventory_document()
        reversed_interval["per_run"][0]["median_ratio_95ci"] = {"lower": 1.2, "upper": 1.1}
        mutations.append(reversed_interval)

        with tempfile.TemporaryDirectory(prefix="p094-cost-ratios-", dir="/tmp") as temporary:
            root = Path(temporary).resolve()
            for index, document in enumerate(mutations):
                with self.subTest(index=index):
                    reference = write_reference(root, f"invalid-ratio-{index}.json", document)
                    self.assertTrue(Path(reference["path"]).is_relative_to(root))
                    with self.assertRaises((VerificationError, ValueError)):
                        p094.cost_inventory({"fixture-family": reference})


if __name__ == "__main__":
    unittest.main()
