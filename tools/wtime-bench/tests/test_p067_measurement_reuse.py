"""Failure-injection tests for the narrow P067 measurement reuse guard."""
from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "tools" / "verification" / "producers"))

import p067_retain  # noqa: E402


def completed_measure(exit_code=0, *, collector=None, command="measure"):
    collector = collector or (ROOT / "tools/verification/producers/p067_workflow.py")
    return {
        "status": "failed",
        "error": p067_retain.EXPECTED_FAILURE,
        "commands": [{
            "name": command,
            "exit_code": exit_code,
            "argv": ["python3", "-B", str(collector), "measure"],
        }],
    }


class P067MeasurementReuseGuardTests(unittest.TestCase):
    def test_accepts_only_the_known_outer_contract_guard_after_successful_collection(self):
        command = p067_retain.validate_failure(completed_measure())
        self.assertEqual(command["name"], "measure")
        self.assertEqual(command["exit_code"], 0)

    def test_rejects_semantic_failures_and_incomplete_or_malformed_collectors(self):
        bad = [
            {**completed_measure(), "error": "ValueError: oracle mismatch"},
            {**completed_measure(), "error": "RuntimeError: unrelated failure"},
            completed_measure(exit_code=1),
            completed_measure(exit_code=True),
            {**completed_measure(), "commands": []},
            {**completed_measure(), "commands": [*completed_measure()["commands"] * 2]},
        ]
        for failure in bad:
            with self.subTest(failure=failure), self.assertRaises(ValueError):
                p067_retain.validate_failure(failure)

    def test_rejects_a_different_collector_path_or_phase(self):
        wrong_path = completed_measure(collector=ROOT / "tools/verification/producers/other.py")
        wrong_phase = completed_measure(command="verify")
        for failure in (wrong_path, wrong_phase):
            with self.subTest(failure=failure), self.assertRaises(ValueError):
                p067_retain.validate_failure(failure)


if __name__ == "__main__":
    unittest.main()
