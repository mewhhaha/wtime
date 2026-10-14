"""Synthetic shared-batch capacity checks for P056 legacy controls."""
from __future__ import annotations

from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

BENCH = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(BENCH))
import compare

SIDES = [('baseline', 'baseline.exe', 'decoded'),
         ('candidate', 'candidate.exe', 'portable')]
LEGACY_IDS = ('call_overhead', 'integer_loop', 'branch_calls')


class P056CalibrationCapacityTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='p056-calibration-capacity-')
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        fixture_path = (self.root / 'synthetic-fixture.wasm').resolve()
        self.assertTrue(fixture_path.is_relative_to(self.root), fixture_path)
        fixture_path.write_bytes(b'synthetic fixture bytes\n')

    @staticmethod
    def fixture(workload):
        return {'id': workload, 'scalar': False, 'memory': False,
                'input': 100, 'expected': 101}

    def test_all_explicit_legacy_controls_can_confirm_above_100k(self):
        for workload in LEGACY_IDS:
            with self.subTest(workload=workload):
                calls = []

                def observe(exe, fixture, stage, requested, mode, cpu):
                    calls.append((exe, fixture['id'], stage, requested, mode))
                    batch = 100_001 if requested == 0 else requested
                    return {'batch': batch, 'ns_per_operation': 200.0}

                with patch.object(compare, '_observe', side_effect=observe):
                    result = compare._calibrate_scalar_shared_batch(
                        self.fixture(workload), SIDES, 3)
                self.assertEqual(result['max_batch'], 1_000_000)
                self.assertEqual(result['batch'], 100_001)
                self.assertEqual([call[3] for call in calls],
                                 [0, 0, 100_001, 100_001])
                self.assertTrue(all(side['calibrated_total_ns'] >= 20_000_000
                                    for side in result['sides']))

    def test_legacy_control_still_fails_at_one_million_hard_cap(self):
        calls = []

        def observe(exe, fixture, stage, requested, mode, cpu):
            calls.append(requested)
            return {'batch': 1_000_000 if requested == 0 else requested,
                    'ns_per_operation': 1.0}

        with patch.object(compare, '_observe', side_effect=observe):
            with self.assertRaisesRegex(compare.ComparisonError,
                    'did not reach 20000000ns on both sides within 1000000 calls'):
                compare._calibrate_scalar_shared_batch(
                    self.fixture('call_overhead'), SIDES, 3)
        self.assertEqual(calls, [0, 0, 1_000_000, 1_000_000])

    def test_typed_scalar_calibration_grows_above_100k_and_confirms_both_sides(self):
        fixture = {'id': 'scalar_test', 'scalar': True, 'memory': False}
        calls = []

        def observe(exe, got_fixture, stage, requested, mode, cpu):
            calls.append((exe, requested))
            batch = 100_000 if requested == 0 else requested
            return {'batch': batch, 'ns_per_operation': 205.0 if exe == 'baseline.exe' else 193.0}

        with patch.object(compare, '_observe', side_effect=observe):
            result = compare._calibrate_scalar_shared_batch(fixture, SIDES, 3)
            self.assertEqual(result['max_batch'], 1_000_000)
            self.assertEqual(result['batch'], 113_990)
            self.assertEqual([call[1] for call in calls],
                             [0, 0, 100_000, 100_000, 113_990, 113_990])
            self.assertTrue(all(side['calibrated_total_ns'] >= 20_000_000
                                for side in result['sides']))
            with self.assertRaisesRegex(compare.ComparisonError, 'invalid scalar calibration limits'):
                compare._calibrate_scalar_shared_batch(
                    fixture, SIDES, 3, max_batch=1_000_001)

    def test_common_batch_limit_accepts_typed_scalar_and_rejects_memory_above_100k(self):
        self.assertEqual(compare._common_calibrated_batch(
            self.fixture('branch_calls'), [{'batch': 150_000}, {'batch': 180_000}]), 150_000)
        self.assertEqual(compare._common_calibrated_batch(
            {'id': 'typed', 'scalar': True, 'memory': False}, [{'batch': 150_000}]), 150_000)
        with self.assertRaisesRegex(compare.ComparisonError, 'invalid batch'):
            compare._common_calibrated_batch(
                {'id': 'memory_case', 'scalar': False, 'memory': True}, [{'batch': 100_001}])


if __name__ == '__main__':
    unittest.main()
