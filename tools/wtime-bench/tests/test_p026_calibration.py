import importlib.util
import math
from pathlib import Path
import unittest
import sys
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from unittest import mock

P = Path(__file__).resolve().parents[1] / 'compare.py'
spec = importlib.util.spec_from_file_location('calibration_compare', P)
compare = importlib.util.module_from_spec(spec)
spec.loader.exec_module(compare)


class CalibrationTests(unittest.TestCase):
    def call(self, **kwargs):
        defaults = dict(exe='exe', fixture='fixture', entry={'id':'case'},
                        input_value='i32:1', raw_value=1, allowed=['i32:1'],
                        mode='decoded', profile='simd', cpu=4, model=object())
        defaults.update(kwargs)
        return compare._calibrate_simd(**defaults)

    def test_grows_requested_batch_until_measured_target(self):
        seen=[]
        def observe(*args, **kwargs):
            request=args[7]
            seen.append(request)
            if request == 0:
                return {'batch':1,'ns_per_operation':1_000_000.0}
            return {'batch':request,'ns_per_operation':1_000_000.0}
        with mock.patch.object(compare, '_simd_observation', side_effect=observe):
            row=self.call()
        self.assertEqual(seen, [0, 22])
        self.assertGreaterEqual(row['batch']*row['ns_per_operation'], 20_000_000)

    def test_fails_explicitly_at_batch_ceiling(self):
        def observe(*args, **kwargs):
            request=args[7]
            return {'batch':1 if request == 0 else request, 'ns_per_operation':1.0}
        with mock.patch.object(compare, '_simd_observation', side_effect=observe):
            with self.assertRaisesRegex(compare.ComparisonError, 'did not reach'):
                self.call(max_batch=10, max_attempts=8)

    def test_initial_batch_is_used_when_target_already_met(self):
        expected={'batch':20,'ns_per_operation':1_000_000.0,'result':1}
        def observe(*args, **kwargs):
            self.assertEqual(args[7], 0)
            return expected
        with mock.patch.object(compare, '_simd_observation', side_effect=observe):
            row=self.call()
        self.assertIs(row, expected)
        self.assertTrue(math.isfinite(row['ns_per_operation']))
        self.assertGreater(row['ns_per_operation'], 0)

    def test_rejects_response_batch_that_ignores_explicit_request(self):
        def observe(*args, **kwargs):
            request=args[7]
            return {'batch':1 if request == 0 else request-1, 'ns_per_operation':1.0}
        with mock.patch.object(compare, '_simd_observation', side_effect=observe):
            with self.assertRaisesRegex(compare.ComparisonError, 'differs from request'):
                self.call(max_batch=10, max_attempts=3)


    def test_invalid_timing_is_rejected_before_calibration_can_accept_it(self):
        entry={'id':'case','required_features':['simd']}
        for timing in [0.0, -1.0, float('nan'), float('inf')]:
            wire={'schema_version':2,'case':'case','stage':'warm_call','input':'i32:1',
                  'expected':'i32:1','result':'i32:1','mode':'decoded','profile':'simd',
                  'batch':1,'ns_per_operation':timing}
            with self.subTest(timing=timing), mock.patch.object(compare,'process',return_value=wire):
                with self.assertRaisesRegex(compare.ComparisonError,'timing'):
                    self.call(entry=entry)


if __name__ == '__main__':
    unittest.main()
