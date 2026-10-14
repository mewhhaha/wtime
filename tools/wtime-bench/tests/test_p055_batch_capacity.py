from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

BENCH = Path(__file__).resolve().parents[1]
if str(BENCH) not in sys.path:
    sys.path.insert(0, str(BENCH))

import compare
import focused_compare


def single_wire(case, stage, input_value, expected, batch, mode, ns=2.5):
    return {
        'schema_version': 2, 'case': case, 'stage': stage,
        'input': input_value, 'expected': expected,
        'result': None if case == 'prepare_large' else expected,
        'batch': batch, 'ns_per_operation': ns, 'mode': mode,
    }


def scalar_wire(entry, batch):
    return {
        'schema_version': 2, 'case': entry['id'], 'stage': 'warm_call',
        'input': 'i32:17', 'expected': 'i32:136', 'result': 'i32:136',
        'batch': batch, 'ns_per_operation': 2.5, 'mode': 'decoded',
        'profile': 'mvp', 'driver': 'plain', 'diagnostic': False,
    }


class P055BatchCapacityTests(unittest.TestCase):
    def test_run_single_case_calibration_can_exceed_100k_and_reaches_target_on_both_sides(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td).resolve()
            fixture = root / 'synthetic-call-overhead.wasm'
            calls = []

            def observe(side, batch):
                calls.append((side, batch))
                return {'batch': batch, 'ns_per_operation': 25 if side == 'baseline' else 24}

            entry = {'id': 'call_overhead', 'oracle': {'module': 'run.py'}}
            batch, audit = focused_compare.calibrate(entry, observe)
            final = audit[-1]['observations']
            self.assertGreater(batch, 100_000)
            self.assertLessEqual(batch, 1_000_000)
            self.assertEqual([final[side]['batch'] for side in ('baseline', 'candidate')], [batch, batch])
            self.assertTrue(all(final[side]['ns_per_operation'] * batch >= 20_000_000
                                for side in ('baseline', 'candidate')))
            self.assertEqual(calls[-2:], [('baseline', batch), ('candidate', batch)])
            self.assertTrue(fixture.is_relative_to(root))

    def test_single_case_validation_accepts_one_million_and_reports_overflow_details(self):
        accepted = single_wire('call_overhead', 'warm_call', 100, 101,
                               1_000_000, 'decoded', ns=25.0)
        self.assertIs(compare.validate_observation(accepted, 'call_overhead', 'warm_call',
                                                   100, 101, 1_000_000, 'decoded'), accepted)
        rejected = single_wire('call_overhead', 'warm_call', 100, 101,
                               1_000_001, 'decoded', ns=25.0)
        with self.assertRaisesRegex(compare.ComparisonError,
                                    r'call_overhead/warm_call: single-case batch 1000001.*25000025'):
            compare.validate_observation(rejected, 'call_overhead', 'warm_call',
                                         100, 101, 0, 'decoded')

    def test_single_case_cap_failure_names_workload_batch_and_both_side_totals(self):
        def observe(_side, batch):
            return {'batch': batch, 'ns_per_operation': 1.0}

        with self.assertRaisesRegex(
                ValueError,
                r'call_overhead: calibration reached batch cap 1000000; final batch 1000000, '
                r'side totals below 20000000ns: baseline=1000000ns.*candidate=1000000ns'):
            focused_compare.calibrate(
                {'id': 'call_overhead', 'oracle': {'module': 'run.py'}}, observe)

    def test_scalar_and_simd_calibration_caps_remain_100k(self):
        def observe(_side, batch):
            return {'batch': batch, 'ns_per_operation': 1.0}

        for module, workload in (('scalar_workloads.py', 'scalar_accumulator_n17'),
                                 ('simd_workloads.py', 'simd_integer_lanes')):
            with self.subTest(module=module), self.assertRaisesRegex(
                    ValueError, rf'{workload}: calibration reached batch cap 100000; final batch 100000'):
                focused_compare.calibrate({'id': workload, 'oracle': {'module': module}}, observe)

    def test_typed_scalar_validation_keeps_100k_and_legacy_single_case_wire_unchanged(self):
        entry = {'id': 'scalar_accumulator_n17',
                 'driver': {'profile': 'mvp', 'kind': 'plain'}}
        with tempfile.TemporaryDirectory() as td:
            root = Path(td).resolve()
            scalar_fixture = root / 'scalar.wasm'
            with patch.object(compare, 'process', return_value=scalar_wire(entry, 100_001)):
                with self.assertRaisesRegex(compare.ComparisonError, 'invalid scalar batch'):
                    compare.scalar_observation('candidate', str(scalar_fixture), entry,
                                               17, 136, 'warm_call', 0, 'decoded', 0)

            fixture = root / 'legacy.wasm'
            wire = single_wire('call_overhead', 'warm_call', 100, 101, 3, 'decoded')
            calls = []

            def process(executable, argv, cpu):
                calls.append((executable, list(argv), cpu))
                return wire

            with patch.object(compare, 'process', side_effect=process):
                observed = compare.observation('baseline', str(fixture), 'call_overhead',
                                               100, 101, 'warm_call', 3, 'decoded', 0)
            self.assertEqual(calls[0][1], ['--single-case', str(fixture), 'call_overhead',
                                           100, 101, 'warm_call', 3, 'decoded'])
            self.assertEqual(observed, wire)
            self.assertEqual(set(observed), {'schema_version', 'case', 'stage', 'input',
                                             'expected', 'result', 'batch',
                                             'ns_per_operation', 'mode'})
            self.assertTrue(fixture.is_relative_to(root))


if __name__ == '__main__':
    unittest.main()
