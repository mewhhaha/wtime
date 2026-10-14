import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

TOOLS=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(TOOLS))
spec=importlib.util.spec_from_file_location('test_evidence',TOOLS/'test_evidence.py');gate=importlib.util.module_from_spec(spec);spec.loader.exec_module(gate)
class Evidence(unittest.TestCase):
    def check(self,code,output,hashes=('a','a')):
        with tempfile.TemporaryDirectory() as directory:
            path=Path(directory)/'gate.json'
            with patch.object(gate.subprocess,'run',return_value=subprocess.CompletedProcess(['cargo'],code,stdout=output)),patch.object(gate.hardening,'source_hash',side_effect=hashes):
                passed=gate.collect(['cargo'],path)
            return passed,json.loads(path.read_text())
    def test_empty_gate_fails(self):self.assertFalse(self.check(0,'test result: ok. 0 passed;')[0])
    def test_failed_command_preserves_exit(self):
        passed,record=self.check(101,'test result: ok. 3 passed;');self.assertFalse(passed);self.assertEqual(record['exit_code'],101)
    def test_stale_gate_fails(self):self.assertFalse(self.check(0,'test result: ok. 3 passed;',('a','b'))[0])
    def test_positive_counts_retained(self):
        passed,record=self.check(0,'test result: ok. 3 passed;\ntest result: ok. 2 passed;');self.assertTrue(passed);self.assertEqual(record['executed'],5)
if __name__=='__main__':unittest.main()
