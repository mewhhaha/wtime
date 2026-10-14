from __future__ import annotations
import importlib.util, json, tempfile, unittest
from pathlib import Path
MODEL_PATH=Path(__file__).resolve().parents[1]/'p127_fuel_policy.py'
ROOT=Path(__file__).resolve().parents[3]
spec=importlib.util.spec_from_file_location('p127_model_tests',MODEL_PATH);model=importlib.util.module_from_spec(spec);spec.loader.exec_module(model)
class P127PolicyModelTests(unittest.TestCase):
 def test_exact_typed_policy_and_cli_contract(self):
  self.assertEqual(model.policy(None),{'name':'unmetered','source_fuel':None})
  self.assertEqual(model.policy(50_000_000),{'name':'metered','source_fuel':50_000_000})
  self.assertEqual(model.parse_cli('none'),model.policy('unmetered'))
  self.assertEqual(model.parse_cli('50000000'),model.policy('metered'))
  self.assertEqual(model.cli_value(model.policy('metered')),'50000000')
  for bad in (True,False,-1,0,50_000_000.0,50_000_001,'50000001','unmetered-ish'):
   with self.subTest(bad=bad),self.assertRaises(ValueError):model.policy(bad)
 def test_protocol_contract_encodes_null_vs_exact_budget(self):
  c=model.protocol_contract();self.assertEqual(c,{'schema_version':1,'accepted':[{'name':'unmetered','source_fuel':None},{'name':'metered','source_fuel':50_000_000}]})
 def test_fixed_matrix_is_46_rows_times_four_policies_modes(self):
  root=ROOT
  rows=model.entries(root/'tools/wtime-bench/workloads.json')
  self.assertEqual([r['id'] for r in rows],list(model.ROW_IDS))
  self.assertEqual(sum(len(r['stages']) for r in rows),46)
  plan=model.matrix(rows);self.assertEqual(len(plan),184)
  self.assertEqual(sum(x['workload']=='prepare_large' for x in plan),4)
  self.assertTrue(all(x['stage']=='prepare' and x['input'] is None and x['expected'] is None and x['result'] is None for x in plan if x['workload']=='prepare_large'))
  self.assertEqual(sum(x['stage']=='warm_call' and x['batch']==3 for x in plan),36)
  self.assertTrue(all(x['batch']==1 for x in plan if x['stage']!='warm_call'))
 def test_oracles_are_independent_and_prepare_has_null(self):
  root=ROOT;rows=model.entries(root/'tools/wtime-bench/workloads.json');by={r['id']:r for r in rows}
  self.assertEqual(by['scalar_accumulator_n17']['expected'],136)
  self.assertEqual(by['call_overhead']['expected'],101)
  self.assertEqual(by['scalar_dependent_n0']['expected'],1)
  self.assertEqual(by['prepare_large']['expected'],None)
  prep=next(x for x in model.matrix(rows) if x['workload']=='prepare_large')
  self.assertIsNone(prep['expected']);self.assertIsNone(prep['result'])
 def test_dependent_oracle_matches_literal_recurrence_in_admitted_wat(self):
  root=ROOT;wat=(root/'tools/wtime-bench/fixtures/scalar/dependent.wat').read_text()
  self.assertIn('i32.mul (local.get $acc) (i32.const 1664525)',wat)
  self.assertIn('(i32.const 1013904223)',wat)
  for n in model.INPUTS:
   value=1
   for _ in range(n):value=(value*1664525+1013904223)&0xffffffff
   signed=value-(1<<32) if value&(1<<31) else value
   self.assertEqual(model.expected_result({'id':f'scalar_dependent_n{n}'}),signed)
 def test_registry_drift_rejects_without_mutating_source(self):
  with tempfile.TemporaryDirectory() as td:
   root=Path(td).resolve();p=root/'workloads.json';p.write_text('{}')
   with self.assertRaisesRegex(ValueError,'registry hash changed'):model.entries(p)
 def test_cli_value_requires_exact_well_typed_policy_dict(self):
  self.assertEqual(model.cli_value({'name':'unmetered','source_fuel':None}),'none')
  self.assertEqual(model.cli_value({'name':'metered','source_fuel':50_000_000}),'50000000')
  invalid=(None,{}, {'name':'metered'}, {'name':'metered','source_fuel':None},
           {'name':'unmetered','source_fuel':0}, {'name':'unmetered','source_fuel':False},
           {'name':'metered','source_fuel':True}, {'name':'metered','source_fuel':50_000_000.0},
           {'name':'metered','source_fuel':50_000_000,'extra':1},
           {'name':True,'source_fuel':50_000_000})
  for setting in invalid:
   with self.subTest(setting=setting),self.assertRaises(ValueError):model.cli_value(setting)

 def test_validate_wire_setting_requires_presence_and_exact_json_types(self):
  cases=((model.policy('unmetered'),{'source_fuel':None,'fuel_policy':'unmetered'}),
         (model.policy('metered'),{'source_fuel':50_000_000,'fuel_policy':'metered'}))
  for setting,wire in cases:model.validate_wire_setting(wire,setting)
  setting=model.policy('metered')
  invalid=({}, {'source_fuel':50_000_000}, {'fuel_policy':'metered'},
           {'source_fuel':True,'fuel_policy':'metered'},
           {'source_fuel':50_000_000.0,'fuel_policy':'metered'},
           {'source_fuel':50_000_000,'fuel_policy':True},
           {'source_fuel':None,'fuel_policy':'metered'},
           {'source_fuel':50_000_000,'fuel_policy':'unmetered'})
  for wire in invalid:
   with self.subTest(wire=wire),self.assertRaises(ValueError):model.validate_wire_setting(wire,setting)

if __name__=='__main__':unittest.main()
