import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
TOOLS=Path(__file__).resolve().parents[1];sys.path.insert(0,str(TOOLS))
spec=importlib.util.spec_from_file_location('allocation_failure',TOOLS/'allocation_failure.py');gate=importlib.util.module_from_spec(spec);spec.loader.exec_module(gate)
class AllocationFailure(unittest.TestCase):
    def run_gate(self,result):
        with tempfile.TemporaryDirectory() as directory:
            worker=Path(directory)/'worker';worker.write_bytes(b'worker');report=Path(directory)/'report.json'
            with patch.object(gate.subprocess,'run',return_value=result),patch.object(gate.hardening,'source_hash',return_value='same'):
                passed=gate.collect(worker,report)
            return passed,json.loads(report.read_text())
    def test_allocator_failure_requires_recovery(self):
        result=subprocess.CompletedProcess([],0,stdout='{"allocator_failed":true,"recovery_passed":false,"executed":1}',stderr='')
        self.assertFalse(self.run_gate(result)[0])
    def test_process_fault_is_not_allocator_success(self):
        result=subprocess.CompletedProcess([],-6,stdout='{"allocator_failed":true,"recovery_passed":true,"executed":1}',stderr='')
        passed,record=self.run_gate(result);self.assertFalse(passed);self.assertEqual(record['exit_code'],-6)
    def test_successful_failure_and_recovery_retained(self):
        result=subprocess.CompletedProcess([],0,stdout='{"allocator_failed":true,"recovery_passed":true,"executed":1}',stderr='')
        passed,record=self.run_gate(result);self.assertTrue(passed);self.assertEqual(record['requested_allocation_bytes'],512*1024*1024)
if __name__=='__main__':unittest.main()
