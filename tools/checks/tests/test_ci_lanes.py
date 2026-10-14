import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[3]

class Lanes(unittest.TestCase):
    def invoke(self, lane, code):
        with tempfile.TemporaryDirectory() as directory:
            cargo = Path(directory) / 'cargo'
            cargo.write_text('#!/bin/sh\nprintf "%s\\n" "$*"\nexit ' + str(code) + '\n')
            cargo.chmod(0o755)
            return subprocess.run(['bash', str(ROOT/'tools/checks/ci.sh'), lane], env={**os.environ, 'PATH': directory+':'+os.environ['PATH'], 'SUITE':'i32'}, capture_output=True, text=True)
    def test_t014_case_01(self):
        result=self.invoke('core',0)
        self.assertEqual(result.returncode,0)
        self.assertIn('--exclude wtime-conformance',result.stdout)
    def test_t014_case_02(self):
        self.assertEqual(self.invoke('feature',17).returncode,17)
    def test_t014_allocation_build_failure(self):
        self.assertEqual(self.invoke('allocation',17).returncode,17)
    def test_t014_case_03(self):
        self.assertEqual(self.invoke('unknown',0).returncode,2)
        self.assertEqual(self.invoke('oracle',0).returncode,77)

if __name__ == '__main__': unittest.main()
