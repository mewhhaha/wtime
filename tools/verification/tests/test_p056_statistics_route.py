"""Contained route checks for the P056 full-portfolio statistics producer."""
from __future__ import annotations

import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import Mock, patch

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "tools"))
from verification import statistics
from verification.artifacts import VerificationError


class P056StatisticsRouteTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="p056-statistics-route-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()

    def put(self, relative: str, content: bytes) -> Path:
        path = (self.root / relative).resolve()
        self.assertTrue(path.is_relative_to(self.root), path)
        path.parent.mkdir(parents=True, exist_ok=True)
        self.assertTrue(path.parent.resolve().is_relative_to(self.root), path.parent)
        path.write_bytes(content)
        return path

    def reference(self, relative: str) -> dict:
        path = self.put(relative, ("fixture " + relative + "\n").encode())
        return {"path": str(path), "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
                "bytes": path.stat().st_size}

    def portfolio(self, *, predecessor: bool) -> Path:
        declaration = {
            "schema": "wtime-verification-portfolio-v1",
            "reports": {}, "cold_controls": {},
            "p121": self.reference("refs/p121.json"),
            "p123": self.reference("refs/p123.json"),
            "retained": self.reference("refs/retained.json"),
        }
        if predecessor:
            declaration["structural_predecessor"] = self.reference("refs/predecessor.json")
        return self.put("portfolio.json", json.dumps(declaration, sort_keys=True).encode())

    def invoke(self, task: str, portfolio: Path, gate: Mock, output: Path):
        for path in (output,):
            self.assertTrue(path.resolve().is_relative_to(self.root), path)
        with patch.object(statistics, "module", return_value=gate):
            return statistics.compute(ROOT, task, portfolio,
                                      self.reference("build.json")["path"], None, None, None, output)

    @staticmethod
    def portfolio_result(task: str) -> dict:
        return {"task": task, "analysis": {"portfolio": "evaluated-once"},
                "source_binding": {"scope": "frozen portfolio"},
                "dataset_statistics_computations": 7, "decision": "accepted"}

    def test_p056_routes_full_portfolio_to_portfolio_evaluator(self):
        declaration = self.portfolio(predecessor=True)
        gate = Mock()
        gate.evaluate_portfolio.return_value = self.portfolio_result("P056")
        record = self.invoke("P056", declaration, gate, self.root / "out/p056.json")
        gate.evaluate_portfolio.assert_called_once()
        gate.evaluate.assert_not_called()
        self.assertEqual(record["task"], "P056")
        self.assertEqual(record["portfolio_decision_computations"], 1)
        self.assertEqual(record["dataset_statistics_computations"], 7)
        self.assertIn("structural_predecessor", record["inputs"])

    def test_p056_missing_predecessor_fails_before_evaluation_or_output(self):
        declaration = self.portfolio(predecessor=False)
        gate = Mock()
        output = self.root / "out/missing-predecessor.json"
        with self.assertRaisesRegex(VerificationError, "P056 requires its immutable predecessor"):
            self.invoke("P056", declaration, gate, output)
        gate.evaluate_portfolio.assert_not_called()
        gate.evaluate.assert_not_called()
        self.assertFalse(output.exists())

    def test_wrong_task_rejects_and_p046_uses_default_portfolio_routing(self):
        declaration = self.portfolio(predecessor=False)
        wrong_gate = Mock()
        wrong_gate.evaluate_portfolio.return_value = self.portfolio_result("P046")
        output = self.root / "out/wrong-task.json"
        with self.assertRaisesRegex(VerificationError, "portfolio task differs"):
            self.invoke("P056", self.portfolio(predecessor=True), wrong_gate, output)
        wrong_gate.evaluate.assert_not_called()
        self.assertFalse(output.exists())

        p046_gate = Mock()
        p046_gate.evaluate_portfolio.return_value = self.portfolio_result("P046")
        record = self.invoke("P046", declaration, p046_gate, self.root / "out/p046.json")
        p046_gate.evaluate_portfolio.assert_called_once()
        p046_gate.evaluate.assert_not_called()
        self.assertEqual(record["task"], "P046")
        self.assertNotIn("structural_predecessor", record["inputs"])


if __name__ == "__main__":
    unittest.main()
