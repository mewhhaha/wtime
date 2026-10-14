import json
from pathlib import Path
import subprocess
import tempfile
import unittest
ROOT=Path(__file__).resolve().parents[3]
EXE=ROOT/'tools/wtime-bench/target/release/wtime-bench'
ENCODER=ROOT/'target/debug/wtime-conformance'
class P003Protocol(unittest.TestCase):
    def encode(self,dir,text):
        wat=Path(dir)/'case.wat'; wasm=Path(dir)/'case.wasm';wat.write_text(text)
        subprocess.run([str(ENCODER),'encode',str(wat),'-o',str(wasm)],check=True,capture_output=True)
        return wasm
    def typed(self,wasm,input_value,expected,profile='mvp'):
        r=subprocess.run([str(EXE),'--typed-case',str(wasm),'typed',input_value,expected,'first_call','1','decoded',profile],check=True,capture_output=True,text=True)
        return json.loads(r.stdout)
    def test_p003_real_float_and_i64_bit_patterns(self):
        with tempfile.TemporaryDirectory() as d:
            wasm=self.encode(d,'(module (func (export "run") (param i64 f32 f64) (result i64 f32 f64) local.get 0 local.get 1 local.get 2))')
            values='i64:-9223372036854775808;f32_bits:2147483648;f64_bits:9221120237041090626'
            r=self.typed(wasm,values,values,'all-phase3-core');self.assertEqual(r['result'],values)
    def test_p003_real_v128_little_endian(self):
        with tempfile.TemporaryDirectory() as d:
            wasm=self.encode(d,'(module (func (export "run") (param v128) (result v128) local.get 0))')
            values='v128_bits:000102030405060708090a0b0c0d0e0f'
            self.assertEqual(self.typed(wasm,values,values,'all-phase3-core')['result'],values)
    def test_p003_capability_is_executed_and_unsupported_has_no_timing(self):
        with tempfile.TemporaryDirectory() as d:
            wasm=self.encode(d,'(module (func (export "run") (param i32) (result i32) local.get 0 i32.const 1 i32.add))')
            command=[str(EXE),'--capability',str(wasm),'i32:100','i32:101']
            supported=json.loads(subprocess.check_output(command+['mvp'],text=True))
            self.assertEqual(supported['status'],'supported');self.assertTrue(supported['compile_execute_checked'])
            unsupported=json.loads(subprocess.check_output(command+['stack_switching'],text=True))
            self.assertEqual(unsupported['status'],'unsupported');self.assertIn('reason',unsupported)
            self.assertNotIn('ns_per_operation',unsupported)
            bad=subprocess.run([str(EXE),'--capability',str(wasm),'i32:100','i32:999','mvp'],capture_output=True)
            self.assertNotEqual(bad.returncode,0)
    def test_p003_null_reference_categories_and_wrong_oracle(self):
        with tempfile.TemporaryDirectory() as d:
            wasm=self.encode(d,'(module (func (export "run") (param externref) (result externref) local.get 0))')
            self.assertEqual(self.typed(wasm,'reference:null','reference:null','all-phase3-core')['result'],'reference:null')
            r=subprocess.run([str(EXE),'--typed-case',str(wasm),'typed','reference:null','reference:extern','first_call','1','decoded','all-phase3-core'],capture_output=True)
            self.assertNotEqual(r.returncode,0)
