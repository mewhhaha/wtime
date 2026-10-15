"""A completed collector may be reused only for the known contract guard failure."""
import copy
from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'tools/verification/producers'))
import p080_retain


def failed_phase():
    return {'status': 'failed', 'error': p080_retain.EXPECTED_FAILURE,
            'commands': [{'name': 'specialist-comparisons', 'exit_code': 0,
                'argv': [sys.executable, '-B', str(ROOT / 'tools/verification/producers/p080_workflow.py'), 'measure']}]}


class ReuseTests(unittest.TestCase):
    def test_completed_collector_and_exact_contract_guard_are_required(self):
        original = failed_phase()
        self.assertEqual(p080_retain.validate_failure(original), original['commands'][0])
        for error in ('semantic oracle mismatch', 'command failed: specialist-comparisons', ''):
            value = copy.deepcopy(original)
            value['error'] = error
            with self.assertRaises(ValueError):
                p080_retain.validate_failure(value)

    def test_failed_missing_duplicate_or_boolean_exit_status_cannot_be_reused(self):
        for commands in ([], failed_phase()['commands'] * 2):
            value = failed_phase()
            value['commands'] = commands
            with self.assertRaises(ValueError):
                p080_retain.validate_failure(value)
        for status in (1, -9, False, None):
            value = failed_phase()
            value['commands'][0]['exit_code'] = status
            with self.assertRaises(ValueError):
                p080_retain.validate_failure(value)

    def test_different_collector_or_phase_is_rejected(self):
        for index, replacement in ((2, '/tmp/another.py'), (3, 'analyze')):
            value = failed_phase()
            value['commands'][0]['argv'][index] = replacement
            with self.assertRaises(ValueError):
                p080_retain.validate_failure(value)


if __name__ == '__main__':
    unittest.main()
