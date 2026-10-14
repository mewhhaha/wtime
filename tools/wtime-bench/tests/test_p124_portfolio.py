"""Synthetic integrity cases for the P046 matrix and P121 producer inputs."""
import copy
import sys
import unittest
from pathlib import Path

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import p046_gate as gate
import workloads


def _report(family, baseline):
    rows = workloads.selection(family)
    settings = {"instrumented": False, "profile": "release",
                "baseline_mode": "decoded", "candidate_mode": "compact"}
    if family == "representative":
        settings.update(representative_stage="run_after_setup",
                        representative_reset="setup-before-run",
                        representative_measurement_policy="guest-interval-only")
    metadata = {"settings": settings,
                "baseline_archive_kind": "current-source-archive" if baseline == "decoded"
                else "frozen-typed-adapter",
                "baseline_archive_path": f"/synthetic/{baseline}/{family}",
                "p001_adapter_path": f"/synthetic/p001/{family}",
                "baseline_manifest_sha256": "1" * 64,
                "baseline_binary_sha256": ("2" if baseline == "decoded" else "3") * 64,
                "candidate_binary_sha256": "4" * 64,
                "candidate_sources": {"crates/example/src/lib.rs": "5" * 64},
                "hardware": {"selected_cpu": 2, "cpu_model": "synthetic-cpu",
                             "os": "synthetic-os", "cpu_quota": "1 100000"}}
    runs = []
    for run_id in (0, 1):
        pairs = []
        for row in rows:
            source = row["source"]
            params = row.get("oracle_arguments", {})
            if params:
                input_value = str(next(iter(params.values())))
            else:
                row_params = row.get("params", [])
                input_value = str(row_params[0]["baseline"] if row_params else 0)
            expected = row.get("expected_result", row.get("expected", 0))
            for stage in row["stages"]:
                for sample in range(30):
                    pairs.append({
                        "pair_id": f"{run_id}:{row['id']}:{stage}:{sample}",
                        "workload": row["id"], "stage": stage, "input": input_value,
                        "expected": expected, "wat_sha256": source["sha256"],
                        "wasm_sha256": source["wasm_sha256"],
                        "order": ["baseline", "candidate"],
                        "baseline": {"mode": "decoded", "ns_per_operation": 10.0,
                                     "batch": 1, "result": copy.deepcopy(expected)},
                        "candidate": {"mode": "compact", "ns_per_operation": 9.0,
                                      "batch": 1, "result": copy.deepcopy(expected)},
                    })
        runs.append({"run_id": run_id, "seed": 900 + run_id, "pairs": pairs})
    return {"schema_version": 2, "passed": True, "source_stable": True,
            "selection": family, "metadata": metadata, "runs": runs}


class P124PortfolioMatrixTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        # Build once from the complete current workload registry.
        cls.reports = {"structural": {family: _report(family, "decoded")
                                       for family in gate.FAMILIES},
                       "p001": {family: _report(family, "p001")
                                for family in gate.FAMILIES}}

    def test_p124_full_four_family_two_baseline_matrix_counts_exactly(self):
        result = gate.validate_matrix(self.reports, check_sources=False)
        expected_total = sum(len(gate._expected_groups(family)) * gate.MIN_PAIRS * 2
                             for family in gate.FAMILIES)
        self.assertEqual(expected_total, 71_100)
        self.assertEqual(result["decoded_pairs_total"], expected_total)
        self.assertEqual(result["p001_pairs_total"], expected_total)
        self.assertEqual(set(result["counts"]["decoded"]), set(gate.FAMILIES))
        self.assertEqual(set(result["counts"]["p001"]), set(gate.FAMILIES))
        self.assertEqual(result["counts"]["decoded"]["memory"]["groups_per_run"], 181)

    def test_p124_missing_family_workload_stage_run_or_pair_fails_closed(self):
        original = self.reports["structural"]["scalar"]
        self.reports["structural"].pop("scalar")
        with self.assertRaises(gate.PortfolioError):
            gate.validate_matrix(self.reports, check_sources=False)
        self.reports["structural"]["scalar"] = original

        report = original
        second = report["runs"][1]
        removed = second["pairs"].pop()
        with self.assertRaises(gate.PortfolioError):
            gate.validate_matrix(self.reports, check_sources=False)
        second["pairs"].append(removed)

        run = report["runs"].pop()
        with self.assertRaises(gate.PortfolioError):
            gate.validate_matrix(self.reports, check_sources=False)
        report["runs"].append(run)

        pair = report["runs"][0]["pairs"].pop()
        report["runs"][0]["pairs"].append({**pair, "stage": "missing-stage"})
        with self.assertRaises(gate.PortfolioError):
            gate.validate_matrix(self.reports, check_sources=False)
        report["runs"][0]["pairs"].pop()
        report["runs"][0]["pairs"].append(pair)

    def test_p124_representative_rejects_wrong_mode_stage_and_instrumentation(self):
        report = self.reports["structural"]["representative"]
        settings = report["metadata"]["settings"]
        settings["candidate_mode"] = "decoded"
        with self.assertRaises(gate.PortfolioError):
            gate.validate_matrix(self.reports, check_sources=False)
        settings["candidate_mode"] = "compact"
        settings["representative_stage"] = "warm_call"
        with self.assertRaises(gate.PortfolioError):
            gate.validate_matrix(self.reports, check_sources=False)
        settings["representative_stage"] = "run_after_setup"
        settings["instrumented"] = True
        with self.assertRaises(gate.PortfolioError):
            gate.validate_matrix(self.reports, check_sources=False)
        settings["instrumented"] = False

    def test_p124_hard_matrix_identity_requires_same_candidate_and_cpu(self):
        report = self.reports["p001"]["simd"]
        report["metadata"]["candidate_binary_sha256"] = "a" * 64
        with self.assertRaises(gate.PortfolioError):
            gate.validate_matrix(self.reports, check_sources=False)
        report["metadata"]["candidate_binary_sha256"] = "4" * 64
        report["metadata"]["hardware"]["selected_cpu"] = 3
        with self.assertRaises(gate.PortfolioError):
            gate.validate_matrix(self.reports, check_sources=False)
        report["metadata"]["hardware"]["selected_cpu"] = 2

    def test_p124_failed_or_zero_work_p121_evidence_blocks_portfolio(self):
        good = {"status": "passed", "task": "P121", "timing_evidence": False,
                "reports": [{"suite": name, "selected": count, "executed": count}
                            for name, count in (("mvp", 100), ("core3", 200),
                                                ("phase3", 300), ("interactions", 400))],
                "totals": {"selected": 145760, "executed": 145760,
                           "total_dispatched_source_events": 200,
                           "optimized_compact_source_events": 100,
                           "decoded_fallback_source_events": 100,
                           "preparation": {"accepted": 10, "declined": 2,
                                           "published_artifacts": 5,
                                           "unpublished_artifacts": 1,
                                           "attempted_work_known_sum": 8,
                                           "attempted_work_unknown_artifacts": 1,
                                           "artifact_retained_budget_bytes": 4096,
                                           "retained_budget_basis": "conservative capacity and headers"}}}
        self.assertEqual(gate.validate_p121(good, check_sources=False)["status"], "passed")
        bad = copy.deepcopy(good)
        bad["totals"]["optimized_compact_source_events"] = 0
        with self.assertRaises(gate.PortfolioError):
            gate.validate_p121(bad, check_sources=False)
        bad = copy.deepcopy(good)
        bad["reports"][0]["executed"] -= 1
        with self.assertRaises(gate.PortfolioError):
            gate.validate_p121(bad, check_sources=False)


if __name__ == "__main__":
    unittest.main()
