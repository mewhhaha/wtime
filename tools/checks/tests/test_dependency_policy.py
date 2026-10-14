import importlib.util
from pathlib import Path
import unittest
spec = importlib.util.spec_from_file_location('budget', Path(__file__).parents[1]/'dependency_budget.py')
budget = importlib.util.module_from_spec(spec)
spec.loader.exec_module(budget)

def metadata(include_external=False, tool=False):
    return {'packages': [{'id':'core','name':'wtime-runtime','source':None}, {'id':'ext','name':'wast','source':'registry'}], 'resolve': {'nodes': [{'id':'core','deps':([{'pkg':'ext','dep_kinds':[{'kind': 'dev' if tool else None}]}] if include_external else [])}, {'id':'ext','deps':[]}]}}

class Policy(unittest.TestCase):
    def test_t010_external_core_edge(self):
        self.assertTrue(budget.violations(metadata(True), {}))
    def test_t010_tool_only(self):
        self.assertEqual(budget.violations(metadata(True, True), {}), [])
    def test_t010_unsafe_inventory(self):
        self.assertTrue(budget.violations(metadata(), {'x.rs':'unsafe { x() }'}))
    def test_t010_safe_core(self):
        self.assertEqual(budget.violations(metadata(), {'x.rs':'fn x() {}'}), [])

    def test_t010_local_external_edge(self):
        data=metadata(True);data['packages'][1]['source']=None
        self.assertTrue(budget.violations(data, {}))
    def test_t010_multiline_unsafe(self):
        self.assertTrue(budget.violations(metadata(), {'x.rs':'unsafe\n{ x() }'}))
