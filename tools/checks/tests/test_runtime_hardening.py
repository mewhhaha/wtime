import importlib.util
from pathlib import Path
import unittest
spec=importlib.util.spec_from_file_location('hardening',Path(__file__).parents[1]/'hardening.py');h=importlib.util.module_from_spec(spec);spec.loader.exec_module(h)
def report(arch):return {'architecture':arch,'source_sha256':'same','native_execution':True,'passed':True,'tests_executed':3,'exit_code':0}
class Hardening(unittest.TestCase):
    def test_t166_both_native_hosts(self):self.assertEqual(h.check_native([report('x86_64'),report('aarch64')]),[])
    def test_t166_cross_compile_is_not_native(self):
        arm=report('aarch64');arm['native_execution']=False;self.assertTrue(h.check_native([report('x86_64'),arm]))
    def test_t166_failed_or_zero_tests(self):
        arm=report('aarch64');arm['tests_executed']=0;self.assertTrue(h.check_native([report('x86_64'),arm]))
    def test_t166_missing_host_and_stale_source(self):
        self.assertTrue(h.check_native([report('x86_64')]))
        arm=report('aarch64');arm['source_sha256']='old';self.assertTrue(h.check_native([report('x86_64'),arm]))
