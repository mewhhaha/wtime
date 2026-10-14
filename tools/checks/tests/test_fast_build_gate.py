import importlib.util
from pathlib import Path
import unittest
spec=importlib.util.spec_from_file_location('budget',Path(__file__).parents[1]/'build_budget.py');b=importlib.util.module_from_spec(spec);spec.loader.exec_module(b)
class FastBuild(unittest.TestCase):
    def test_t167_case_03_failed_budget(self):
        self.assertFalse(b.check_budget({'budgets':{'clean_seconds':60},'clean_seconds':61}))
    def test_t167_missing_measurement_fails(self):self.assertFalse(b.check_budget({'budgets':{'leaf_seconds':5}}))
    def test_t167_within_budget(self):self.assertTrue(b.check_budget({'budgets':{'clean_seconds':60,'leaf_seconds':5},'clean_seconds':3,'leaf_seconds':1,'source_stable':True,'default_dependency_closure_pass':True}))
    def test_t167_concurrent_edit_fails(self):self.assertFalse(b.check_budget({'budgets':{'clean_seconds':60},'clean_seconds':3,'source_stable':False}))
