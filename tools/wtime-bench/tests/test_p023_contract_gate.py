"""Contract-aware family promotion rules for P023."""
import json
import math
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

ROOT = Path(__file__).resolve().parents[3]
BENCH = ROOT / "tools/wtime-bench"
sys.path.insert(0, str(BENCH))
import gates
import analyze


STAGES = ("prepare", "instantiate", "first_call", "warm_call", "load_and_call")
WORKLOADS = {"primary_a": {"input": "1", "ratio": 0.90},
             "primary_b": {"input": "1", "ratio": 0.96},
             "protected": {"input": "1", "ratio": 1.04}}


def report_for(protected_ratio=1.04, noisy=False):
    report = {"schema_version": 2, "passed": True, "source_stable": True,
              "selection": "contract_test", "metadata": {"settings": {"instrumented": False}},
              "runs": []}
    for run_num, seed in ((1, 901), (2, 902)):
        pairs = []
        index = 0
        for workload, config in WORKLOADS.items():
            ratio = protected_ratio if workload == "protected" else config["ratio"]
            for stage in STAGES:
                for rep in range(30):
                    # Warm variation gives the bootstrap real paired observations.
                    factor = (1.0 + (rep % 5) * 0.001) if noisy and stage == "warm_call" else 1.0
                    pairs.append({
                        "pair_id": f"{run_num}-{index}", "workload": workload, "stage": stage,
                        "input": config["input"], "wasm_sha256": ("a" if workload == "primary_a" else
                            "b" if workload == "primary_b" else "c") * 64,
                        "expected": 7, "order": ["baseline", "candidate"],
                        "baseline": {"ns_per_operation": 100.0, "batch": 10, "result": 7, "mode": "decoded"},
                        "candidate": {"ns_per_operation": 100.0 * ratio * factor,
                                      "batch": 10, "result": 7, "mode": "decoded"},
                    })
                    index += 1
        report["runs"].append({"seed": seed, "pairs": pairs})
    return report


def add_second_input(report, *, omit_stage=None):
    for run in report["runs"]:
        copies = [pair for pair in run["pairs"]
                  if pair["workload"] == "primary_a" and pair["input"] == "1"
                  and pair["stage"] != omit_stage]
        for copy in copies:
            item = json.loads(json.dumps(copy))
            item["pair_id"] = "second-" + item["pair_id"]
            item["input"] = "2"
            item["candidate"]["ns_per_operation"] = item["baseline"]["ns_per_operation"] * 0.99
            run["pairs"].append(item)


CONTRACT = {"schema_version": 1, "selection": "contract_test",
            "primary_workloads": ["primary_a", "primary_b"],
            "protected_workloads": ["protected"]}
SELECTION = {workload: set(STAGES) for workload in WORKLOADS}


class P023ContractGateTests(unittest.TestCase):
    def test_contract_requires_full_unique_disjoint_selection_union(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "contract.json"
            with mock.patch.object(gates, "_selection_stages", return_value=SELECTION):
                path.write_text(json.dumps(CONTRACT))
                parsed, digest = gates._load_contract(path, "contract_test")
                self.assertEqual(parsed, CONTRACT)
                self.assertEqual(len(digest), 64)
                for bad in (
                    {**CONTRACT, "primary_workloads": ["primary_a"]},
                    {**CONTRACT, "primary_workloads": ["unknown", "primary_b"]},
                    {**CONTRACT, "primary_workloads": ["primary_a", "primary_a", "primary_b"]},
                    {**CONTRACT, "protected_workloads": ["primary_a"]},
                ):
                    path.write_text(json.dumps(bad))
                    with self.subTest(bad=bad), self.assertRaises(gates.GateError):
                        gates._load_contract(path, "contract_test")

    def test_historical_p023_contract_and_current_p042_contract_match_their_scopes(self):
        p023_path = ROOT / "docs/benchmarks/performance/P023-contract.json"
        p023 = gates._read_json(p023_path, "historical P023 promotion contract")
        self.assertEqual(gates._file_sha(p023_path),
                         "fdbae8d5ad595168fd27f73d062d7196be183ed22969bef83a39cd56a336eb62")
        p023_ids = set(p023["primary_workloads"]) | set(p023["protected_workloads"])
        self.assertEqual(len(p023["primary_workloads"]), 52)
        self.assertEqual(len(p023["protected_workloads"]), 27)
        p023_task_contract = gates._task("P023")["promotion_contract"]
        self.assertIs(p023_task_contract["declared_before_runtime_edit"], True)
        self.assertEqual((p023_task_contract["primary_count"], p023_task_contract["protected_count"]), (52, 27))
        current_scalar = set(gates._selection_stages("scalar"))
        self.assertTrue(p023_ids <= current_scalar)
        self.assertTrue(current_scalar - p023_ids, "later scalar rows should remain outside the historical P023 contract")

        p042_path = ROOT / "docs/benchmarks/performance/P042-contract.json"
        p042, digest = gates._load_contract(p042_path, "scalar-p042-cache")
        self.assertEqual(digest, "963b853f6333d6e10ad6d47f7e06a7e910898df834a13db4d16af63e283d219f")
        self.assertEqual(len(p042["primary_workloads"]), 30)
        self.assertEqual(len(p042["protected_workloads"]), 80)
        self.assertEqual(set(p042["primary_workloads"]) | set(p042["protected_workloads"]),
                         set(gates._selection_stages("scalar-p042-cache")))

    def test_family_weights_workloads_equally_and_bootstrap_is_repeatable(self):
        report = report_for(noisy=True)
        digest = "d" * 64
        first = gates._family_bootstrap(report, {"primary_a", "primary_b"}, 1, digest)
        again = gates._family_bootstrap(report, {"primary_a", "primary_b"}, 1, digest)
        self.assertAlmostEqual(first["median_ratio"], math.sqrt(0.90 * 0.96) * 1.002, places=5)
        self.assertEqual(first, again)
        self.assertEqual(first["bootstrap_resamples"], 10_000)
        self.assertEqual(first["input_fixture_groups"], 2)

    def test_multiple_inputs_are_median_weighted_and_pair_order_is_irrelevant(self):
        report = report_for(noisy=True)
        add_second_input(report)
        primary = {"primary_a", "primary_b"}
        digest = "f" * 64
        result = gates._family_bootstrap(report, primary, 1, digest)
        expected_a = math.sqrt(0.90 * 1.002 * 0.99)  # median of input-level log medians
        self.assertAlmostEqual(result["median_ratio"], math.sqrt(expected_a * 0.96 * 1.002), places=5)
        self.assertEqual(result["input_fixture_groups"], 3)
        reordered = json.loads(json.dumps(report))
        for run in reordered["runs"]:
            run["pairs"].reverse()
        self.assertEqual(result, gates._family_bootstrap(reordered, primary, 1, digest))

    def test_contract_promotion_uses_family_and_keeps_protected_regression_guard(self):
        report = report_for()
        summary = analyze.analyze(report)
        with mock.patch.object(gates, "_selection_stages", return_value=SELECTION):
            decision, reasons, checks = gates._comparison_decision(
                summary, "contract_test", report=report, contract=CONTRACT, contract_hash="d" * 64)
        self.assertEqual(decision, "adopted")
        self.assertEqual(reasons, [])
        self.assertEqual(checks["primary_workloads"], ["primary_a", "primary_b"])
        family = next(item["primary_family"] for item in checks["checks"] if "primary_family" in item)
        self.assertAlmostEqual(family[0]["median_ratio"],
                               math.sqrt(0.90 * 0.96), places=5)

        bad_report = report_for(protected_ratio=1.06)
        with mock.patch.object(gates, "_selection_stages", return_value=SELECTION):
            decision, reasons, _ = gates._comparison_decision(
                analyze.analyze(bad_report), "contract_test", report=bad_report,
                contract=CONTRACT, contract_hash="d" * 64)
        self.assertEqual(decision, "rejected")
        self.assertTrue(any("protected" in reason and "exceeds 1.05" in reason for reason in reasons))

    def test_each_warm_input_fixture_requires_all_selected_stages(self):
        report = report_for()
        add_second_input(report, omit_stage="instantiate")
        summary = analyze.analyze(report)
        with mock.patch.object(gates, "_selection_stages", return_value=SELECTION):
            with self.assertRaisesRegex(gates.GateError, "missing instantiate input/fixture groups"):
                gates._comparison_decision(summary, "contract_test", report=report,
                                           contract=CONTRACT, contract_hash="e" * 64)


if __name__ == "__main__":
    unittest.main()
