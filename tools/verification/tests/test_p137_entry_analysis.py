"""Contained integrity tests for P137's non-promoting analysis envelope."""
from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

SOURCE = Path(__file__).resolve().parents[1] / "producers/p137.py"
spec = importlib.util.spec_from_file_location("p137_producer_tests", SOURCE)
p137 = importlib.util.module_from_spec(spec)
assert spec and spec.loader
spec.loader.exec_module(p137)


class P137AnalysisEnvelopeTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory(prefix="p137-analysis-")
        self.root = Path(self.temp.name).resolve()

    def tearDown(self) -> None:
        self.temp.cleanup()

    def result(self) -> dict:
        runs = []
        for run_num, seed in enumerate((136001, 136002)):
            rows = []
            for calls in (1, 10, 100, 10000):
                for strategy, threshold in (("eager", 0), ("adaptive", 3)):
                    row_id = f"p135_counter-{calls}-{strategy}-{threshold}"
                    rows.append({
                        "row_id": row_id,
                        "settings": {"row_id": row_id, "case": "p135_counter", "calls": calls,
                                     "strategy": strategy, "threshold": threshold},
                        "pairs": 2, "raw_unit": "ns", "metric": "entry_total",
                        "candidate_over_baseline": None, "ci95": None, "bootstrap_resamples": 0,
                    })
            runs.append({"run_id": run_num, "seed": seed, "rows": rows})
        summary = {"median": 10, "mad": 0, "p25": 10, "p75": 10, "min": 10,
                   "max": 10, "p99": 10, "count": 2, "unit": "ns"}
        for run in runs:
            for row in run["rows"]:
                row["bootstrap_seed"] = 123
                row["baseline"] = {field: dict(summary) for field in
                                   ("total_ns", "prepare_ns", "instantiate_ns", "invoke_ns")}
                row["candidate"] = {field: dict(summary) for field in
                                    ("total_ns", "prepare_ns", "instantiate_ns", "invoke_ns")}
        return {"schema": "wtime-register-entry-analysis-v1", "status": "passed",
                "metric": "entry_total", "raw_unit": "ns", "timing_evidence": False,
                "gate_eligible": False, "decision": None, "threshold_selection": None,
                "scaled_by_calls": False, "runs": runs}

    def test_accepts_complete_smoke_matrix_without_promotion(self) -> None:
        result = self.result()
        p137.validate_smoke_analysis(result)
        self.assertEqual(sum(row["pairs"] for run in result["runs"] for row in run["rows"]), 32)

    def test_rejects_promoting_envelope_or_computed_uncertainty(self) -> None:
        result = self.result()
        result["gate_eligible"] = True
        with self.assertRaisesRegex(p137.P137Error, "invalid or promoting"):
            p137.validate_smoke_analysis(result)
        result = self.result()
        result["runs"][0]["rows"][0]["ci95"] = [0.1, 0.2]
        with self.assertRaisesRegex(p137.P137Error, "claims uncertainty"):
            p137.validate_smoke_analysis(result)

    def test_rejects_incomplete_duplicate_and_foreign_setting_rows(self) -> None:
        result = self.result()
        result["runs"][0]["rows"].pop()
        with self.assertRaisesRegex(p137.P137Error, "matrix is incomplete"):
            p137.validate_smoke_analysis(result)
        result = self.result()
        result["runs"][0]["rows"][1] = dict(result["runs"][0]["rows"][0])
        with self.assertRaisesRegex(p137.P137Error, "duplicated"):
            p137.validate_smoke_analysis(result)
        result = self.result()
        result["runs"][1]["rows"][0]["settings"]["threshold"] = 99
        with self.assertRaisesRegex(p137.P137Error, "settings differ"):
            p137.validate_smoke_analysis(result)

    def test_rejects_bool_counts_and_duplicate_run_identity(self) -> None:
        result = self.result()
        result["runs"][0]["rows"][0]["pairs"] = True
        with self.assertRaisesRegex(p137.P137Error, "claims uncertainty"):
            p137.validate_smoke_analysis(result)
        result = self.result()
        result["runs"][1]["seed"] = result["runs"][0]["seed"]
        with self.assertRaisesRegex(p137.P137Error, "run identity"):
            p137.validate_smoke_analysis(result)

    def test_synthetic_files_and_resolved_paths_are_contained(self) -> None:
        report_path = self.root / "analysis/report.json"
        report_path.parent.mkdir(parents=True)
        report_path.write_text("{}", encoding="utf-8")
        self.assertTrue(report_path.resolve().is_relative_to(self.root))
        self.assertEqual(p137.ref(report_path)["path"], str(report_path.resolve()))
        self.assertTrue(report_path.resolve().is_relative_to(self.root))

    def test_public_checker_accepts_valid_contained_receipt_and_rejects_stale_hash(self) -> None:
        analysis = self.result()
        analysis["method"] = "paired-log-median-rank-bootstrap-v1"
        analysis_path = self.root / "analysis/report.json"
        analysis_path.parent.mkdir(parents=True)
        analysis_path.write_text(json.dumps(analysis), encoding="utf-8")
        proof = {"build_source_inputs_sha256": "b" * 64}
        refs = {"proof": {"path": "contained-synthetic-proof"}}
        source_map = {"crates/example.rs": "a" * 64}
        receipt = {
            "schema": "wtime-p137-verification-v1", "status": "passed", "outcome": "contract_only",
            "decision": None, "timing_evidence": False, "gate_eligible": False,
            "candidate_promoted": False, "guest_or_build_launched": False,
            "p136": refs, "p136_proof_identity_sha256": p137.canonical_sha(proof),
            "current_source_map_sha256": p137.canonical_sha(source_map),
            "current_source_file_count": len(source_map), "analysis": p137.ref(analysis_path),
            "analysis_sha256": p137.sha(analysis_path), "counts": {"runs": 2, "rows": 8, "pairs": 32},
            "analysis_method": analysis["method"], "bootstrap_resamples": 0,
            "build_budgets_reused": True, "build_budget_source": proof["build_source_inputs_sha256"],
        }
        receipt_path = self.root / "receipt.json"
        receipt_path.write_text(json.dumps(receipt), encoding="utf-8")
        self.assertTrue(receipt_path.resolve().is_relative_to(self.root))
        analysis_dir = analysis_path.parent
        analysis["method"] = "paired-log-median-rank-bootstrap-v1"
        names = ("entry_batch_analyze.py", "analyze.py", "bootstrap.py")
        analysis["analysis_source_refs"] = {name: p137.pair_ref(p137.ROOT / "tools/wtime-bench" / name)
                                            for name in names}
        copied = {}
        for name in names:
            target = analysis_dir / "helpers" / name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes((p137.ROOT / "tools/wtime-bench" / name).read_bytes())
            copied[name] = p137.pair_ref(target)
        analysis["analysis_helpers"] = copied
        source_map = {"crates/example.rs": "a" * 64}
        proof = {"build_source_inputs_sha256": "b" * 64}
        refs = {"proof": {"path": "contained-synthetic-proof"}}
        analysis_path.write_text(json.dumps(analysis), encoding="utf-8")
        comparison_path = self.root / "collection/report.json"
        comparison_path.parent.mkdir()
        comparison_path.write_text("{}", encoding="utf-8")
        analysis["comparison"] = p137.pair_ref(comparison_path)
        receipt["analysis"] = p137.ref(analysis_path)
        receipt["analysis_sha256"] = p137.sha(analysis_path)
        receipt["p136"] = refs
        receipt["p136_proof_identity_sha256"] = p137.canonical_sha(proof)
        receipt["current_source_map_sha256"] = p137.canonical_sha(source_map)
        receipt["current_source_file_count"] = len(source_map)
        receipt["analysis_method"] = analysis["method"]
        receipt["build_budget_source"] = proof["build_source_inputs_sha256"]
        analysis_path.write_text(json.dumps(analysis), encoding="utf-8")
        receipt["analysis_sha256"] = p137.sha(analysis_path)
        comparable = {key: value for key, value in analysis.items()
                      if key not in ("analysis_helpers", "comparison")}

        class FakeAnalyzer:
            @staticmethod
            def analyze(_report, _collection, allow_smoke=False):
                assert allow_smoke is True
                return comparable

        with patch.object(p137, "_verify_p136", return_value=(proof, refs)), \
             patch.object(p137, "p136_sources", return_value=source_map), \
             patch.object(p137, "analyzer_module", return_value=FakeAnalyzer), \
             patch.object(p137, "P136_REPORT", comparison_path), \
             patch.object(p137, "P136_COLLECTION", comparison_path.parent):
            p137.validate_smoke_analysis(analysis)
            analysis_path.write_text(json.dumps(analysis), encoding="utf-8")
            receipt["analysis"] = p137.ref(analysis_path)
            receipt["analysis_sha256"] = p137.sha(analysis_path)
            receipt_path.write_text(json.dumps(receipt), encoding="utf-8")
            self.assertEqual(p137.verify_evidence(receipt_path), receipt)
            receipt["analysis_sha256"] = "0" * 64
            receipt_path.write_text(json.dumps(receipt), encoding="utf-8")
            with self.assertRaisesRegex(p137.P137Error, "content hash"):
                p137.verify_evidence(receipt_path)


if __name__ == "__main__":
    unittest.main()
