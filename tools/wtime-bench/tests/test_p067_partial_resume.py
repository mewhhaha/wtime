"""Reject unrelated failures rather than treating them as reusable sampling."""
from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'tools/verification/producers'))
import p067_resume


class PartialResumeTests(unittest.TestCase):
    def failure(self):
        return {'status': 'failed', 'error': 'VerificationError: command failed: measure',
                'commands': [{'name': 'measure', 'exit_code': 1}]}

    def test_only_the_recorded_pre_sampling_metadata_failure_is_eligible(self):
        result = p067_resume.validate_failure(self.failure(), {'exit_code': 1}, p067_resume.FAILED_MESSAGE)
        self.assertEqual(result['name'], 'measure')

    def test_semantic_failures_wrong_commands_and_duplicate_results_are_rejected(self):
        failed = self.failure()
        for value in ({**failed, 'status': 'passed'}, {**failed, 'error': 'oracle mismatch'},
                      {**failed, 'commands': []}, {**failed, 'commands': failed['commands'] * 2},
                      {**failed, 'commands': [{'name': 'verify', 'exit_code': 1}]}):
            with self.subTest(value=value), self.assertRaises(ValueError):
                p067_resume.validate_failure(value, {'exit_code': 1}, p067_resume.FAILED_MESSAGE)

    def test_boolean_exit_status_or_any_different_collector_error_is_rejected(self):
        for code in (0, True, None, '1'):
            with self.subTest(code=code), self.assertRaises(ValueError):
                p067_resume.validate_failure(self.failure(), {'exit_code': code}, p067_resume.FAILED_MESSAGE)
        with self.assertRaises(ValueError):
            p067_resume.validate_failure(self.failure(), {'exit_code': 1}, 'comparison failed: bad result\n')


if __name__ == '__main__':
    unittest.main()
