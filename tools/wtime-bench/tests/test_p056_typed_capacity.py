"""Synthetic typed SIMD batch-capacity checks for P056."""
from __future__ import annotations

from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

BENCH = Path(__file__).resolve().parents[1]
if str(BENCH) not in sys.path:
    sys.path.insert(0, str(BENCH))

import compare
import simd_scaling
import simd_workloads


def model_entry(workload_id):
    return next(entry for entry in simd_workloads.entries()
                if entry['id'] == workload_id)


class P056TypedCapacityTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='p056-typed-capacity-')
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.fixture = (self.root / 'synthetic-simd.wasm').resolve()
        self.assertTrue(self.fixture.is_relative_to(self.root), self.fixture)
        self.fixture.write_bytes(b'synthetic SIMD fixture\n')
        self.typed = model_entry('simd_integer_lanes_n0')
        self.memory = model_entry('simd_lane_memory_n0')
        self.typed_input = simd_scaling.typed_input(self.typed)
        self.memory_input = simd_scaling.typed_input(self.memory)
        self.typed_allowed = simd_scaling.allowed_wire_results(self.typed, simd_workloads)
        self.memory_allowed = simd_scaling.allowed_wire_results(self.memory, simd_workloads)
        self.assertEqual(simd_scaling.batch_limit(self.typed), 1_000_000)
        self.assertEqual(simd_scaling.batch_limit(self.memory), 100_000)

    def process_response(self, argv, *, batch=None, ns=150.0):
        command, _fixture, case, value, expected, stage, requested, mode, profile = argv
        return {
            'schema_version': 2, 'case': case, 'stage': stage, 'input': value,
            'expected': expected, 'result': expected.split('|')[0],
            'batch': int(requested) if batch is None else batch,
            'ns_per_operation': ns, 'mode': mode, 'profile': profile,
        }

    def test_typed_simd_calibration_grows_above_100k_and_reaches_twenty_ms(self):
        calls = []

        def observe(_exe, _fixture, _entry, _input, _raw, _allowed,
                    stage, batch, mode, profile, cpu, *, model, expected_batch=None):
            calls.append((stage, batch))
            actual = 100_000 if batch == 0 else batch
            return {'batch': actual, 'ns_per_operation': 150.0}

        with patch.object(compare, '_simd_observation', side_effect=observe):
            row = compare._calibrate_simd(
                'candidate', str(self.fixture), self.typed, self.typed_input, 0,
                self.typed_allowed, 'decoded',
                simd_scaling.execution_profile(self.typed), 0, model=simd_workloads)
        self.assertEqual(row['batch'], 146_667)
        self.assertGreater(row['batch'], 100_000)
        self.assertGreaterEqual(row['batch'] * row['ns_per_operation'], 20_000_000)
        self.assertEqual(calls, [('warm_call', 0), ('warm_call', 146_667)])

    def test_typed_simd_observation_accepts_requested_100001_and_rejects_overflow(self):
        expected = '|'.join(self.typed_allowed)
        with patch.object(compare, 'process',
                          side_effect=lambda _exe, argv, _cpu:
                          self.process_response(argv, batch=100_001)):
            observed = compare._simd_observation(
                'candidate', str(self.fixture), self.typed, self.typed_input, 0,
                self.typed_allowed, 'warm_call', 100_001, 'decoded',
                simd_scaling.execution_profile(self.typed), 0,
                model=simd_workloads, expected_batch=100_001)
        self.assertEqual(observed['batch'], 100_001)
        self.assertEqual(observed['wire_result'],
                         int(expected.split('|')[0].removeprefix('i32:')))

        with patch.object(compare, 'process',
                          side_effect=lambda _exe, argv, _cpu:
                          self.process_response(argv, batch=1_000_001)):
            with self.assertRaisesRegex(compare.ComparisonError, 'batch must be in 1..1000000'):
                compare._simd_observation(
                    'candidate', str(self.fixture), self.typed, self.typed_input, 0,
                    self.typed_allowed, 'warm_call', 1_000_001, 'decoded',
                    simd_scaling.execution_profile(self.typed), 0,
                    model=simd_workloads)

    def test_memory_simd_keeps_100k_cap_and_fails_when_timing_is_too_short(self):
        calls = []

        def observe(_exe, _fixture, _entry, _input, _raw, _allowed,
                    _stage, requested, _mode, _profile, _cpu, *, model,
                    expected_batch=None):
            calls.append(requested)
            return {'batch': 100_000 if requested == 0 else requested,
                    'ns_per_operation': 1.0}

        with patch.object(compare, '_simd_observation', side_effect=observe):
            with self.assertRaisesRegex(
                    compare.ComparisonError,
                    r'did not reach 20000000ns.*within 100000 calls'):
                compare._calibrate_simd(
                    'candidate', str(self.fixture), self.memory, self.memory_input, 0,
                    self.memory_allowed, 'decoded',
                    simd_scaling.execution_profile(self.memory), 0,
                    model=simd_workloads)
        self.assertEqual(calls, [0])

    def test_simd_observation_rejects_requested_batch_mismatch_and_nonfinite_timing(self):
        common = (str(self.fixture), self.typed, self.typed_input, 0,
                  self.typed_allowed, 'warm_call', 'decoded',
                  simd_scaling.execution_profile(self.typed), 0)
        with patch.object(compare, 'process',
                          side_effect=lambda _exe, argv, _cpu:
                          self.process_response(argv, batch=100_002)):
            with self.assertRaisesRegex(compare.ComparisonError, 'paired batch mismatch'):
                compare._simd_observation(
                    'candidate', *common[:6], 100_001, *common[6:],
                    model=simd_workloads, expected_batch=100_001)

        with patch.object(compare, 'process',
                          side_effect=lambda _exe, argv, _cpu:
                          self.process_response(argv, batch=100_001, ns=float('inf'))):
            with self.assertRaisesRegex(compare.ComparisonError,
                                        'timing must be finite and positive'):
                compare._simd_observation(
                    'candidate', *common[:6], 100_001, *common[6:],
                    model=simd_workloads, expected_batch=100_001)


if __name__ == '__main__':
    unittest.main()
