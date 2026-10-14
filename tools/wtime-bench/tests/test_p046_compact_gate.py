"""Composite P046 orchestration tests with isolated synthetic records."""
import hashlib
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import p046_gate as gate


class P046PortfolioEvaluationTests(unittest.TestCase):
    def test_p046_p121_numeric_aggregate_uses_verified_suite_accounting(self):
        suites = (("mvp", 18480), ("core3", 62968), ("phase3", 64297), ("interactions", 15))
        value = {"status":"passed", "task":"P121", "timing_evidence":False,
                 "reports":[{"suite":name, "selected":count, "executed":count,
                     "preparation_summary":{"artifact_retained_budget_bytes":1024,
                         "artifact_retained_budget_bytes_basis":"conservative capacity plus headers"}}
                     for name,count in suites],
                 "totals":{"selected":145760, "executed":145760,
                     "total_dispatched_source_events":20, "optimized_compact_source_events":10,
                     "decoded_fallback_source_events":10,
                     "preparation":{"accepted":4, "declined":1, "published_artifacts":4,
                         "unpublished_artifacts":1, "attempted_work_known_sum":20,
                         "attempted_work_unknown_artifacts":1, "artifact_retained_budget_bytes":4096}}}
        self.assertEqual(gate.validate_p121(value, check_sources=False)['retained_budget_bytes'], 4096)
        value['totals']['preparation']['artifact_retained_budget_bytes'] += 1
        with self.assertRaisesRegex(gate.PortfolioError, 'aggregate retained bytes'):
            gate.validate_p121(value, check_sources=False)
        value['totals']['preparation']['artifact_retained_budget_bytes'] -= 1
        value['reports'][0]['preparation_summary']['artifact_retained_budget_bytes_basis'] = 'physical estimate'
        with self.assertRaises(gate.PortfolioError):
            gate.validate_p121(value, check_sources=False)

    def test_p046_direct_cli_imports_validate_capability_producer(self):
        # A direct tools/verify.py invocation has no repository root in
        # sys.path. Exercise that environment from an unrelated directory.
        bench = Path(__file__).resolve().parents[1]
        script = '''
import sys
sys.path.insert(0, sys.argv[1])
import p046_gate
try:
    p046_gate._load_p123_support(
        {"schema": "p123-capabilities-v1", "status": "passed"},
        check_sources=False)
except p046_gate.PortfolioError as error:
    assert "invalid capability timing_evidence" in str(error), str(error)
else:
    raise AssertionError("invalid capability receipt was accepted")
'''
        result = subprocess.run([sys.executable, '-I', '-B', '-c', script, str(bench)],
                                cwd=self.root, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="p046-synthetic-")
        self.root = Path(self.temp.name).resolve()

    def tearDown(self):
        self.temp.cleanup()

    def artifact(self, relative, content):
        path = (self.root / relative).resolve()
        self.assertTrue(path.is_relative_to(self.root), "synthetic output escaped fresh temp root")
        path.parent.mkdir(parents=True, exist_ok=True)
        raw = content if isinstance(content, bytes) else (json.dumps(content, sort_keys=True) + "\n").encode()
        path.write_bytes(raw)
        return {"path": str(path), "sha256": hashlib.sha256(raw).hexdigest(), "bytes": len(raw)}

    def inputs(self):
        reports = {"structural": {}, "p001": {}}
        for baseline in reports:
            for family in gate.FAMILIES:
                reports[baseline][family] = {"selection": family, "synthetic_identity": baseline + family}
        p123 = self.artifact("p123.json", {"task": "P123", "status": "passed"})
        p121 = self.artifact("p121.json", {"task": "P121", "status": "passed"})
        portfolio = {"schema": "p046-portfolio-v1", "reports": reports,
                     "p123": p123, "p121": p121,
                     "structural_build_reports": {
                         "default": self.artifact("default-build.json", {"synthetic": True}),
                         "compact": self.artifact("compact-build.json", {"synthetic": True})},
                     "retained": self.artifact("retained.json", {"synthetic": True}),
                     "frozen_build_report": str(self.artifact("frozen-build.json", {}).get("path")),
                     "build_growth_explanation": "synthetic policy test"}
        portfolio_ref = self.artifact("portfolio.json", portfolio)
        default_build = Path(portfolio["structural_build_reports"]["default"]["path"])
        compact_build = Path(portfolio["structural_build_reports"]["compact"]["path"])
        return portfolio_ref, default_build, compact_build

    def test_p046_distinct_report_sets_are_analyzed_once_and_decision_once(self):
        portfolio_ref, default_build, compact_build = self.inputs()
        matrix = {"candidate_binary_sha256": "a" * 64, "candidate_sources": {"crates/a.rs": "b" * 64},
                  "p001_pairs_total": 240, "reports": {}}
        p123_support = {family: True for family in gate.FAMILIES}
        for baseline in ("decoded", "p001"):
            for family in gate.FAMILIES:
                matrix["reports"][(baseline, family)] = {"candidate_sources": matrix["candidate_sources"]}
        analyzer = mock.Mock(return_value={"per_run": [{"stage": "warm_call",
                                                            "decision": "supported_improvement"}]})
        with mock.patch.object(gate, "_load_p123_support", return_value=({}, p123_support)), \
             mock.patch.object(gate, "validate_matrix", return_value=matrix), \
             mock.patch.object(gate, "validate_p121", return_value={"status": "passed",
                                                                    "optimized_source_events": 1}), \
             mock.patch.object(gate, "_build_decision", return_value={"default_budget_pass": True,
                    "compact_budget_pass": True, "dependency_violations": [],
                    "relative_growth_review":False,"relative_growth_explanation_missing":False}), \
             mock.patch.object(gate, "_retained_gate", return_value={"status": "passed",
                    "relative_status":"within_budget","explanation_missing":False}), \
             mock.patch.object(gate, "_analysis_policy", return_value=("adopted", [], {})):
            result = gate.evaluate_portfolio(Path(portfolio_ref["path"]), default_build, compact_build,
                                             analyzer_fn=analyzer, check_sources=False)
        self.assertEqual(analyzer.call_count, 8)
        self.assertEqual(result["dataset_statistics_computations"], 8)
        self.assertEqual(result["portfolio_decision_computations"], 1)
        self.assertEqual(result["decision"], "adopted")

    def test_p046_hard_build_failure_overrides_speed_gain(self):
        portfolio_ref, default_build, compact_build = self.inputs()
        matrix = {"candidate_binary_sha256": "a" * 64, "candidate_sources": {"crates/a.rs": "b" * 64},
                  "p001_pairs_total": 240, "reports": {
                      (baseline, family): {"candidate_sources": {"crates/a.rs": "b" * 64}}
                      for baseline in ("decoded", "p001") for family in gate.FAMILIES}}
        with mock.patch.object(gate, "_load_p123_support", return_value=({}, {f: True for f in gate.FAMILIES})), \
             mock.patch.object(gate, "validate_matrix", return_value=matrix), \
             mock.patch.object(gate, "validate_p121", return_value={"status": "passed",
                                                                    "optimized_source_events": 1}), \
             mock.patch.object(gate, "_build_decision", return_value={"default_budget_pass": False,
                    "compact_budget_pass": True, "dependency_violations": [],
                    "relative_growth_review":False,"relative_growth_explanation_missing":False}), \
             mock.patch.object(gate, "_retained_gate", return_value={"status": "passed",
                    "relative_status":"within_budget","explanation_missing":False}), \
             mock.patch.object(gate, "_analysis_policy", return_value=("adopted", [], {})), \
             mock.patch.object(gate.analyze, "analyze", return_value={"per_run": [{
                 "stage": "warm_call", "decision": "supported_improvement"}]}):
            result = gate.evaluate_portfolio(Path(portfolio_ref["path"]), default_build, compact_build,
                                             analyzer_fn=lambda report:{'per_run':[{'stage':'warm_call','decision':'supported_improvement'}]},
                                             check_sources=False)
        self.assertEqual(result["decision"], "rejected")
        self.assertTrue(any("hard build" in reason for reason in result["reasons"]))


if __name__ == "__main__":
    unittest.main()
