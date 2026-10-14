from __future__ import annotations
import importlib.util, json, tempfile, unittest
from pathlib import Path
from unittest.mock import patch

ROOT=Path(__file__).resolve().parents[3]
BENCH=ROOT/'tools/wtime-bench'
spec=importlib.util.spec_from_file_location('p127_comparison_model_tests',BENCH/'p127_fuel_policy.py')
policy=importlib.util.module_from_spec(spec);assert spec and spec.loader;spec.loader.exec_module(policy)
sys_path=str(BENCH)
import sys
if sys_path not in sys.path:sys.path.insert(0,sys_path)
import compare, focused_compare


def output_under(root:Path)->Path:
 root=root.resolve();out=(root/'output').resolve()
 if root not in out.parents:raise AssertionError(f'output escaped fresh test root: {out}')
 out.mkdir()
 return out


def single_wire(case,stage,input_value,expected,mode,setting=None):
 data={'schema_version':2,'case':case,'stage':stage,'input':input_value,'expected':expected,
       'result':None if case=='prepare_large' else expected,'batch':3 if stage=='warm_call' else 1,
       'ns_per_operation':1.25,'mode':mode}
 if setting is not None:data.update({'source_fuel':setting['source_fuel'],'fuel_policy':setting['name']})
 return data


def scalar_wire(entry,input_value,expected,stage,mode,setting=None):
 data={'schema_version':2,'case':entry['id'],'stage':stage,'input':f'i32:{input_value}',
       'expected':f'i32:{expected}','result':f'i32:{expected}','batch':3 if stage=='warm_call' else 1,
       'ns_per_operation':2.5,'mode':mode,'profile':entry['driver']['profile'],
       'driver':entry['driver']['kind'],'diagnostic':False}
 if setting is not None:data.update({'source_fuel':setting['source_fuel'],'fuel_policy':setting['name']})
 return data


class P127ComparisonTests(unittest.TestCase):
 def test_legacy_single_case_and_scalar_argv_remain_byte_for_byte_unchanged(self):
  with tempfile.TemporaryDirectory() as td:
   root=Path(td).resolve();out=output_under(root);calls=[]
   def fake(exe,args,cpu):
    calls.append((exe,list(args),cpu))
    if args[0]=='--single-case':return single_wire(args[2],args[5],int(args[3]),int(args[4]),args[7])
    entry={'id':args[2],'driver':{'profile':args[8],'kind':args[9]}}
    return scalar_wire(entry,int(args[3]),int(args[4]),args[5],args[7])
   with patch.object(compare,'process',side_effect=fake):
    a=compare.observation('base',str(root/'fixture.wasm'),'call_overhead',100,101,'warm_call',3,'decoded',2)
    entry={'id':'scalar_accumulator_n17','driver':{'profile':'mvp','kind':'plain'}}
    b=compare.scalar_observation('candidate',str(root/'fixture.wasm'),entry,17,136,'warm_call',3,'compact',2)
   self.assertEqual(calls[0][1],['--single-case',str(root/'fixture.wasm'),'call_overhead',100,101,'warm_call',3,'decoded'])
   self.assertEqual(calls[1][1],['--scalar-driver',str(root/'fixture.wasm'),'scalar_accumulator_n17',17,136,'warm_call',3,'compact','mvp','plain'])
   self.assertNotIn('source_fuel',a);self.assertNotIn('fuel_policy',b);self.assertEqual(a['result'],101);self.assertEqual(b['result'],136)
   self.assertIn(root, out.parents)

 def test_explicit_metered_and_unmetered_policies_route_through_both_compare_helpers(self):
  with tempfile.TemporaryDirectory() as td:
   root=Path(td).resolve();out=output_under(root);calls=[]
   entry={'id':'scalar_dependent_n17','driver':{'profile':'mvp','kind':'plain'}}
   def fake(exe,args,cpu):
    calls.append((exe,list(args),cpu))
    if args[0]=='--single-case':
     setting=policy.parse_cli(args[-1]);return single_wire(args[2],args[5],int(args[3]),int(args[4]),args[7],setting)
    setting=policy.parse_cli(args[-1]);return scalar_wire(entry,int(args[3]),-123,'warm_call',args[7],setting)
   with patch.object(compare,'process',side_effect=fake):
    generic=compare.observation('base',str(root/'fixture.wasm'),'call_overhead',100,101,'warm_call',3,'decoded',1,
                                fuel_setting=policy.policy('metered'))
    scalar=compare.scalar_observation('candidate',str(root/'fixture.wasm'),entry,17,-123,'warm_call',3,'compact',1,
                                     fuel_setting=policy.policy('unmetered'))
   self.assertEqual(calls[0][1][-2:],['--source-fuel','50000000'])
   self.assertEqual(calls[1][1][-2:],['--source-fuel','none'])
   self.assertEqual(generic['fuel_policy'],'metered');self.assertEqual(scalar['fuel_policy'],'unmetered')
   self.assertIn(root,out.parents)

 def test_compare_routes_reject_missing_wrong_float_and_boolean_wire_budget(self):
  with tempfile.TemporaryDirectory() as td:
   root=Path(td).resolve();out=output_under(root);entry={'id':'scalar_accumulator_n17','driver':{'profile':'mvp','kind':'plain'}}
   metered=policy.policy('metered')
   for mutation in ('missing-budget','missing-name','float','bool','wrong'):
    def fake(exe,args,cpu,mutation=mutation):
     data=scalar_wire(entry,17,136,'first_call','decoded',metered)
     if mutation=='missing-budget':data.pop('source_fuel')
     elif mutation=='missing-name':data.pop('fuel_policy')
     elif mutation=='float':data['source_fuel']=50_000_000.0
     elif mutation=='bool':data['source_fuel']=True
     elif mutation=='wrong':data['fuel_policy']='unmetered'
     return data
    with patch.object(compare,'process',side_effect=fake),self.subTest(mutation=mutation),self.assertRaises(ValueError):
     compare.scalar_observation('runtime',str(root/'fixture.wasm'),entry,17,136,'first_call',1,'decoded',0,fuel_setting=metered)
   self.assertIn(root,out.parents)

 def test_focused_observe_shares_same_setting_for_simulated_paired_sides(self):
  with tempfile.TemporaryDirectory() as td:
   root=Path(td).resolve();out=output_under(root);entry={'id':'scalar_accumulator_n17','family':'scalar_accumulator',
      'oracle':{'module':'scalar_workloads.py'},'params':[{'name':'n','type':'i32','baseline':17}],
      'driver':{'profile':'mvp','kind':'plain'}}
   setting=policy.policy('metered');calls=[]
   def fake(exe,args,cpu):
    calls.append((exe,list(args)))
    parsed=policy.parse_cli(args[-1])
    return scalar_wire(entry,17,136,args[5],args[7],parsed)
   with patch.object(compare,'process',side_effect=fake):
    results=[focused_compare.observe(side,entry,str(root/'fixture.wasm'),'warm_call',3,'decoded',0,out,
              fuel_setting=setting) for side in ('baseline','candidate')]
   self.assertEqual([x[1][-2:] for x in calls],[['--source-fuel','50000000']]*2)
   self.assertEqual([x['fuel_policy'] for x in results],['metered','metered'])
   self.assertEqual([x['result'] for x in results],[136,136])
   self.assertTrue(all(Path(c[1][1]).is_absolute() and root in Path(c[1][1]).parents for c in calls));self.assertIn(root,out.parents)

 def test_focused_prepare_large_is_numeric_single_case_and_returns_null(self):
  with tempfile.TemporaryDirectory() as td:
   root=Path(td).resolve();out=output_under(root);entry={'id':'prepare_large','oracle':{'module':'run.py'},'params':[],'source':{}}
   calls=[];setting=policy.policy('unmetered')
   def fake(exe,args,cpu):
    calls.append(list(args));parsed=policy.parse_cli(args[-1])
    return single_wire('prepare_large','prepare',0,0,'compact',parsed)
   with patch.object(compare,'process',side_effect=fake):
    result=focused_compare.observe('candidate',entry,str(root/'prepare_large.wasm'),'prepare',1,'compact',0,out,fuel_setting=setting)
   self.assertEqual(calls[0][0],'--single-case');self.assertEqual(calls[0][3:7],[0,0,'prepare',1])
   self.assertEqual(calls[0][-2:],['--source-fuel','none']);self.assertIsNone(result['result']);self.assertIn(root,out.parents)

 def test_unsupported_focused_family_fails_before_process_command(self):
  with tempfile.TemporaryDirectory() as td:
   root=Path(td).resolve();out=output_under(root);entry={'id':'memory_probe','oracle':{'module':'memory_workloads.py'},'params':[]}
   calls=[]
   with patch.object(compare,'process',side_effect=lambda *args:(calls.append(args) or {})),self.assertRaises(ValueError):
    focused_compare.observe('candidate',entry,'/fixture.wasm','prepare',1,'decoded',0,out,fuel_setting=policy.policy('metered'))
   self.assertEqual(calls,[]);self.assertIn(root,out.parents)

if __name__=='__main__':unittest.main()
