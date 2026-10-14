import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest
HERE=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(HERE))
import compare
class CompareTests(unittest.TestCase):
    def good(self):
        return dict(schema_version=2,case='call_overhead',stage='warm_call',input=100,
                    expected=101,result=101,batch=4,ns_per_operation=123.0,mode='decoded')
    def validate(self,data):
        return compare.validate_observation(data,'call_overhead','warm_call',100,101,4,'decoded')
    def test_p002_missing_wrong_oracle_and_nonfinite_fail(self):
        for delta in [{'result':0},{'batch':3},{'ns_per_operation':float('nan')},{'mode':'register'}]:
            with self.subTest(delta=delta),self.assertRaises(compare.ComparisonError):
                self.validate(self.good()|delta)
        self.assertEqual(self.validate(self.good())['batch'],4)
    def test_p002_process_crash_and_missing_response_fail(self):
        cpu=min(compare.os.sched_getaffinity(0))
        with self.assertRaises(compare.ComparisonError): compare.process(sys.executable,['-c','raise SystemExit(3)'],cpu)
        with self.assertRaises(compare.ComparisonError): compare.process(sys.executable,['-c','print("garbage")'],cpu)
    def test_p002_order_is_reproducible_and_contains_both_orders(self):
        orders=compare.pair_order(20261011,30)
        self.assertEqual(orders,compare.pair_order(20261011,30))
        self.assertIn(['baseline','candidate'],orders)
        self.assertIn(['candidate','baseline'],orders)
        self.assertEqual(30,len(orders))
