import importlib.util
import json
from pathlib import Path
import random
import sys
import tempfile
import unittest

MODULE=Path(__file__).resolve().parents[1]/'fuzz_decode.py'
spec=importlib.util.spec_from_file_location('fuzz_decode',MODULE)
fuzz=importlib.util.module_from_spec(spec);spec.loader.exec_module(fuzz)

class FuzzHarnessTests(unittest.TestCase):
    def test_t164_case_01_known_panic(self):
        result=fuzz.run_process([sys.executable,'-c','print(\'{"status":"panic"}\')'])
        self.assertIn(result['status'],fuzz.BAD)
        with tempfile.TemporaryDirectory() as d:
            record=fuzz.save_failure(d,b'ab!cd',{'seed':17,'result':result},lambda b:b'!'in b)
            self.assertEqual(Path(record['input']).read_bytes(),b'!')
            self.assertEqual(json.loads(Path(record['input']).with_suffix('.json').read_text())['seed'],17)
    def test_t164_case_02_timeout_retained(self):
        result=fuzz.run_process([sys.executable,'-c','import time;time.sleep(1)'],timeout=.01)
        self.assertEqual(result['status'],'timeout')
        with tempfile.TemporaryDirectory() as d:
            record=fuzz.save_failure(d,b'input',{'result':result})
            self.assertEqual(json.loads(Path(record['input']).with_suffix('.json').read_text())['result']['status'],'timeout')
    def test_t164_case_03_oracle_unsupported(self):
        result=fuzz.run_oracle([sys.executable,'-c','import sys;print("unsupported feature",file=sys.stderr);sys.exit(1)'],'ignored')
        self.assertEqual(result['status'],'unsupported')
    def test_t164_deterministic_mutations(self):
        first=random.Random(164);second=random.Random(164)
        self.assertEqual([fuzz.mutate(b'abcdef',first)for _ in range(100)],[fuzz.mutate(b'abcdef',second)for _ in range(100)])
    def test_t164_signal_not_invalid(self):
        result=fuzz.run_process([sys.executable,'-c','import os,signal;os.kill(os.getpid(),signal.SIGABRT)'])
        self.assertEqual(result['status'],'signal')

if __name__=='__main__':unittest.main()
