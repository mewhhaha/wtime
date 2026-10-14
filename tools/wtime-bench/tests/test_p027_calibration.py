import importlib.util
from pathlib import Path
import sys
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import unittest
from unittest import mock

p=Path(__file__).resolve().parents[1]/'compare.py'
spec=importlib.util.spec_from_file_location('memory_calibration_compare',p)
compare=importlib.util.module_from_spec(spec); spec.loader.exec_module(compare)

class MemoryCalibrationTests(unittest.TestCase):
    def call(self, **kwargs):
        values=dict(exe='exe',fixture={'id':'memory_case','memory':True},mode='decoded',cpu=4)
        values.update(kwargs)
        return compare._calibrate_memory(**values)

    def test_grows_from_automatic_request_to_target(self):
        seen=[]
        def observe(exe,fixture,stage,batch,mode,cpu):
            seen.append(batch)
            return {'batch':1 if batch==0 else batch,'ns_per_operation':1_000_000.0}
        with mock.patch.object(compare,'_observe',side_effect=observe):
            row=self.call()
        self.assertEqual(seen,[0,22])
        self.assertGreaterEqual(row['batch']*row['ns_per_operation'],20_000_000)

    def test_ceiling_failure_is_explicit(self):
        def observe(exe,fixture,stage,batch,mode,cpu):
            return {'batch':1 if batch==0 else batch,'ns_per_operation':1.0}
        with mock.patch.object(compare,'_observe',side_effect=observe):
            with self.assertRaisesRegex(compare.ComparisonError,'did not reach'):
                self.call(max_batch=10)

    def test_invalid_nonpositive_or_nonfinite_timing_is_rejected(self):
        for timing in (0.0,-1.0,float('nan'),float('inf')):
            with self.subTest(timing=timing):
                with mock.patch.object(compare,'_observe',return_value={'batch':1,'ns_per_operation':timing}):
                    with self.assertRaisesRegex(compare.ComparisonError,'finite and positive'):
                        self.call()

    def test_explicit_request_must_be_echoed(self):
        def observe(exe,fixture,stage,batch,mode,cpu):
            return {'batch':1 if batch==0 else batch-1,'ns_per_operation':1_000_000.0}
        with mock.patch.object(compare,'_observe',side_effect=observe):
            with self.assertRaisesRegex(compare.ComparisonError,'differs from request'):
                self.call(max_attempts=3)

    def test_common_memory_batch_uses_larger_side_calibration(self):
        rows=[{'batch':20,'ns_per_operation':1_000_000.0},
              {'batch':35,'ns_per_operation':1_000_000.0}]
        self.assertEqual(compare._common_calibrated_batch({'memory':True},rows),35)

if __name__=='__main__': unittest.main()
