"""Confined synthetic identity and actual-registry subset checks, no encoder/build."""
import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest
from types import SimpleNamespace
sys.dont_write_bytecode=True
HERE=Path(__file__).resolve().parent
TOOL=HERE.parent if HERE.name=='tests' else Path('/workspace/wtime/tools/wtime-bench')
sys.path.insert(0,str(TOOL))
import workloads

def load(name,path):
    spec=importlib.util.spec_from_file_location(name,path);module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module);return module
compare=load('p035_subset',TOOL/'compare.py' if HERE.name=='tests' else HERE/'payload/tools/wtime-bench/compare.py')
routes=load('p035_routes',TOOL/'profile_routes.py' if HERE.name=='tests' else HERE/'profile_routes.py')

class ToolingRouteTests(unittest.TestCase):
    def test_original_five_preserved_with_exact_three_compute_subset(self):
        rows=workloads.selection('original-mvp');before=list(rows)
        self.assertEqual(len(rows),5)
        subset=compare.resolve_compute_subset(rows,['integer_loop','branch_calls','call_overhead'])
        self.assertEqual([x['id'] for x in subset],['call_overhead','integer_loop','branch_calls'])
        self.assertEqual(rows,before)
        self.assertIs(compare.resolve_compute_subset(rows,None),rows)

    def test_wrong_duplicate_or_missing_subset_fails(self):
        rows=workloads.selection('original-mvp')
        for subset in [['call_overhead']*3,['call_overhead','integer_loop','prepare_large']]:
            with self.assertRaises(compare.ComparisonError):compare.resolve_compute_subset(rows,subset)
        with self.assertRaises(compare.ComparisonError):compare.resolve_compute_subset(rows[1:],['call_overhead','integer_loop','branch_calls'])

    def test_missing_repository_wasm_encodes_only_into_fresh_confined_output(self):
        with tempfile.TemporaryDirectory(prefix='P035-route-',dir='/tmp') as temp:
            root=Path(temp);wat=root/'model.wat';wat.write_text('(module)');encoder=root/'encoder';encoder.write_bytes(b'synthetic encoder identity')
            payload=b'synthetic encoded bytes';import hashlib
            entry={'source':{'path':'model.wat','sha256':routes.sha(wat),'wasm_sha256':hashlib.sha256(payload).hexdigest()}}
            calls=[]
            def runner(argv,**kwargs):
                output=Path(argv[-1]);self.assertEqual(output.parent,root/'fresh');output.write_bytes(payload);calls.append(argv);return SimpleNamespace(returncode=0,stderr='')
            _,wasm,proof=routes.prepare_fixture(entry,root,root/'fresh',encoder,runner=runner)
            self.assertFalse(wat.with_suffix('.wasm').exists());self.assertEqual(len(calls),1);self.assertEqual(proof['wasm_sha256'],routes.sha(wasm))
            wasm.write_bytes(b'wrong')
            with self.assertRaises(ValueError):routes.prepare_fixture(entry,root,root/'fresh',encoder,runner=runner)

if __name__=='__main__':unittest.main()
