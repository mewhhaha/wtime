"""Contained synthetic tests for P127 wire validation and P128 record binding."""
from __future__ import annotations
import hashlib, importlib.util, json, shutil, tempfile, unittest
from pathlib import Path
from unittest.mock import patch
DRAFT=Path(__file__).resolve().parents[3]
PRODUCER=DRAFT/'tools/verification/producers/p127_fuel.py'
spec=importlib.util.spec_from_file_location('p127_producer_v2_tests',PRODUCER);producer=importlib.util.module_from_spec(spec);spec.loader.exec_module(producer)
model=producer.model
REAL=Path(__file__).resolve().parents[3]
def digest(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def put(root,path,data):
 p=producer.contained(root,root/path);p.parent.mkdir(parents=True,exist_ok=True);p.write_bytes(data);return p
def source_map(root):
 compiled={}
 for crate in ('wtime-runtime','wtime-types','wtime-decode','wtime-validate'):
  rel=f'crates/{crate}/src/lib.rs';compiled[rel]=digest(put(root,Path(rel),f'// {crate}\n'.encode()))
 return compiled
def record(root,compiled,mode='decoded'):
 binary=put(root,Path('target/bin-'+mode),b'synthetic runtime binary')
 protocol={'schema_version':2,'engine':'wtime','modes':['decoded','compact'],'features':['mvp'],'instrumented':False,'source_fuel_contract':model.protocol_contract()}
 protocol_file=put(root,Path('target/protocol-'+mode+'.json'),json.dumps(protocol).encode())
 runtime={k:v for k,v in compiled.items() if k.startswith('crates/wtime-runtime/src/')}
 core={k:v for k,v in compiled.items() if k.endswith('.rs') and k.startswith(('crates/wtime-runtime/src/','crates/wtime-types/src/','crates/wtime-decode/src/','crates/wtime-validate/src/'))}
 features=[] if mode=='decoded' else ['compact-executor']
 cargo=['runtime-only']+features
 value={'schema':'wtime-runtime-only-production-v1','phase':'build','mode':mode,'exit_code':0,'source_stable':True,'instrumented':False,
 'runtime_features':features,'feature_flags':features,'harness_features':['runtime-only'],'cargo_features':cargo,
 'command':['cargo','build','--locked','--offline','--no-default-features','--features',','.join(cargo)],
 'source_identities':compiled,'runtime_source_map':runtime,'core_source_map':core,'binary_path':str(binary),'binary_sha256':digest(binary),
 'protocol':protocol,'protocol_exit':0,'protocol_sha256':digest(protocol_file),'protocol_command':[str(binary),'--protocol'],'protocol_stdout':str(protocol_file)}
 return put(root,Path('target/record-'+mode+'.json'),json.dumps(value).encode()),value
class Done:
 returncode=0;stdout='';stderr=''
class P127ProducerV2Tests(unittest.TestCase):
 def test_actual_scalar_driver_wire_requires_typed_strings_and_policy_keys(self):
  row={'workload':'scalar_accumulator_n17','stage':'first_call','mode':'decoded','fuel_policy':'metered','source_fuel':50_000_000,'batch':1,'input':17,'expected':136,'result':136}
  raw={'schema_version':2,'case':row['workload'],'stage':row['stage'],'input':'i32:17','expected':'i32:136','result':'i32:136','batch':1,'ns_per_operation':1.0,'mode':'decoded','profile':'mvp','driver':'plain','diagnostic':False,'source_fuel':50_000_000,'fuel_policy':'metered'}
  self.assertEqual(producer._validate_response(raw,row,'scalar-driver')['result'],136)
  for changed in ({'input':17},{'source_fuel':True},{'fuel_policy':True},{'diagnostic':True},{'result':'i32:137'}):
   with self.subTest(changed=changed),self.assertRaises(producer.P127Error):producer._validate_response({**raw,**changed},row,'scalar-driver')
  for missing in ('source_fuel','fuel_policy','diagnostic','expected'):
   with self.subTest(missing=missing),self.assertRaises(producer.P127Error):producer._validate_response({k:v for k,v in raw.items() if k!=missing},row,'scalar-driver')
 def test_single_case_route_and_wire_cover_call_overhead_and_prepare_large(self):
  with tempfile.TemporaryDirectory() as td:
   root=Path(td).resolve();binary=put(root,Path('bin'),b'b');fixture=put(root,Path('fixture.wasm'),b'w')
   call=model.entries(REAL/'tools/wtime-bench/workloads.json')[-2]
   callrow={'workload':'call_overhead','stage':'warm_call','mode':'compact','fuel_policy':'unmetered','source_fuel':None,'batch':3,'input':100,'expected':101,'result':101}
   argv,wire=producer._driver_argv(binary,fixture,callrow,'compact',model.policy('unmetered'))
   self.assertEqual(wire,'single-case');self.assertEqual(argv[1],'--single-case');self.assertEqual(argv[4:8],['100','101','warm_call','3']);self.assertEqual(argv[-2:],['--source-fuel','none'])
   raw={'schema_version':2,'case':'call_overhead','stage':'warm_call','input':100,'expected':101,'result':101,'batch':3,'ns_per_operation':2.0,'mode':'compact','source_fuel':None,'fuel_policy':'unmetered'}
   producer._validate_response(raw,callrow,'single-case')
   prep={'workload':'prepare_large','stage':'prepare','mode':'decoded','fuel_policy':'metered','source_fuel':50_000_000,'batch':1,'input':None,'expected':None,'result':None}
   argv,wire=producer._driver_argv(binary,fixture,prep,'decoded',model.policy('metered'))
   self.assertEqual(wire,'single-case');self.assertEqual(argv[4:8],['0','0','prepare','1'])
   prepraw={'schema_version':2,'case':'prepare_large','stage':'prepare','input':0,'expected':0,'result':None,'batch':1,'ns_per_operation':1.0,'mode':'decoded','source_fuel':50_000_000,'fuel_policy':'metered'}
   got=producer._validate_response(prepraw,prep,'single-case');self.assertIsNone(got['result'])
   scalar={'workload':'scalar_accumulator_n17','stage':'warm_call','mode':'decoded','fuel_policy':'metered','input':17,'expected':136,'result':136,'batch':3}
   argv,wire=producer._driver_argv(binary,fixture,scalar,'decoded',model.policy('metered'))
   self.assertEqual(wire,'scalar-driver');self.assertEqual(argv[1],'--scalar-driver');self.assertTrue(Path(argv[2]).is_absolute())
 def test_full_184_policy_mode_stage_matrix_accepts_only_exact_tuple_set(self):
  rows=model.entries(REAL/'tools/wtime-bench/workloads.json');plan=model.matrix(rows)
  fake=[{k:r[k] for k in ('workload','stage','mode','fuel_policy')} for r in plan]
  self.assertTrue(producer._validate_matrix(plan,fake))
  for invalid in (fake[:-1],fake+[fake[0]],[*fake[:-1],{**fake[-1],'fuel_policy':'unmetered'}]):
   with self.subTest(length=len(invalid)),self.assertRaises(producer.P127Error):producer._validate_matrix(plan,invalid)
 def test_truthful_p128_record_shape_and_core_record_protocol_staleness_fail_closed(self):
  with tempfile.TemporaryDirectory() as td:
   root=Path(td).resolve();compiled=source_map(root);path,good=record(root,compiled)
   checked,binary,_=producer.p128._verify_record(root,path,'decoded',compiled)
   self.assertEqual(checked['schema'],'wtime-runtime-only-production-v1');self.assertEqual(binary,Path(good['binary_path']))
   variants=[]
   core={**good,'core_source_map':{**good['core_source_map'],'crates/wtime-runtime/src/lib.rs':'f'*64}};variants.append(core)
   stale={**good,'source_stable':False};variants.append(stale)
   badproto={**good,'protocol_sha256':'0'*64};variants.append(badproto)
   badrecord={**good,'runtime_features':['compact-executor']};variants.append(badrecord)
   for i,value in enumerate(variants):
    p=put(root,Path(f'target/bad-{i}.json'),json.dumps(value).encode())
    with self.subTest(i=i),self.assertRaises(producer.p128.P128Error):producer.p128._verify_record(root,p,'decoded',compiled)
 def test_protocol_mutation_is_rejected_and_protocol_logs_are_retained(self):
  with tempfile.TemporaryDirectory() as td:
   root=Path(td).resolve();binary=put(root,Path('bin'),b'binary');out=root/'attempt';out.mkdir();commands=[]
   expected={'schema_version':2,'engine':'wtime','modes':['decoded'],'features':['mvp'],'instrumented':False,'source_fuel_contract':model.protocol_contract()}
   class Protocol(Done):stdout=json.dumps({**expected,'features':['stable-core']})
   with self.assertRaises(producer.P127Error):producer._live_protocol(binary,'decoded',expected,out,lambda *a:Protocol(),commands,root)
   self.assertEqual(len(commands),1)
   self.assertTrue((out/commands[0]['stdout_path']).is_file())
 def test_source_maps_use_p128_closure_and_record_protocol_mutations_abort_pre_guest(self):
  compiled,proof=producer._current_sources(REAL)
  self.assertTrue(any(x.startswith('crates/wtime-runtime/src/') for x in compiled))
  self.assertIn('crates/wtime-types/src/lib.rs',compiled)
  self.assertIn('tools/wtime-bench/p128_scalar_scaling.py',proof)
  self.assertIn(str(PRODUCER.relative_to(REAL)),proof)
  mutations=('core','record','protocol','retained-protocol','runtime-source')
  for mutation in mutations:
   with self.subTest(mutation=mutation),tempfile.TemporaryDirectory() as td:
    root=Path(td).resolve();registry=root/'tools/wtime-bench/workloads.json';registry.parent.mkdir(parents=True);shutil.copyfile(REAL/'tools/wtime-bench/workloads.json',registry)
    lock=root/'docs/benchmarks/performance/P125-fixture-tools.json';lock.parent.mkdir(parents=True);shutil.copyfile(REAL/'docs/benchmarks/performance/P125-fixture-tools.json',lock)
    compiled=source_map(root);decoded,doc=record(root,compiled);compact,_=record(root,compiled,'compact')
    if mutation=='core':doc['core_source_map']['crates/wtime-runtime/src/lib.rs']='0'*64
    elif mutation=='record':doc['schema']='wrong-schema'
    elif mutation=='protocol':doc['protocol']['source_fuel_contract']={'schema_version':1,'accepted':[]}
    elif mutation=='retained-protocol':
     protocol_path=Path(doc['protocol_stdout']);protocol_path.write_text('{"stale":true}')
    elif mutation=='runtime-source':doc['runtime_source_map']['crates/wtime-runtime/src/lib.rs']='f'*64
    bad=put(root,Path('target/record-decoded-mutated.json'),json.dumps(doc).encode())
    calls=[]
    with self.assertRaises((producer.P127Error,producer.p128.P128Error)):
     producer.run(bad,compact,registry,lock,root/'attempt',root=root,source_reader=lambda _: (compiled,compiled),command_runner=lambda *a:(calls.append(a) or Done()))
    self.assertEqual(calls,[])
 def test_raw_command_logs_exist_before_response_mismatch_and_escape_is_blocked(self):
  with tempfile.TemporaryDirectory() as td:
   root=Path(td).resolve();out=root/'attempt';out.mkdir();commands=[]
   class Raw(Done):stdout='{"case":"wrong"}';stderr='retained'
   producer._command(out,'guest',['binary','--single-case','--source-fuel','none'],root,lambda *a:Raw(),commands)
   log=out/commands[0]['stdout_path'];self.assertTrue(log.is_file());self.assertEqual(log.read_text(),Raw.stdout)
   row={'workload':'call_overhead','stage':'prepare','mode':'decoded','fuel_policy':'unmetered','source_fuel':None,'batch':1,'input':100,'expected':101,'result':101}
   with self.assertRaises(producer.P127Error):producer._validate_response(json.loads(log.read_text()),row,'single-case')
   outside=root.parent/(root.name+'-p127-escape-output')
   if outside.exists():self.fail('escape fixture path unexpectedly preexists')
   with self.assertRaises(producer.P127Error):producer.run(root/'none',root/'none2',root/'registry',root/'lock',outside,root=root)
   self.assertFalse(outside.exists())
 def test_mock_prepare_large_fixture_is_generated_encoded_and_mvp_validated_inside_attempt(self):
  with tempfile.TemporaryDirectory() as td:
   root=Path(td).resolve();out=root/'attempt';out.mkdir();bench=root/'tools/wtime-bench';bench.mkdir(parents=True)
   shutil.copyfile(REAL/'tools/wtime-bench/workloads.py',bench/'workloads.py')
   receipt=put(root,Path('target/receipt.json'),b'synthetic receipt')
   tool=put(root,Path('target/tool'),b'synthetic pinned tool')
   toolhash=digest(tool);lock={'source_receipt':{'path':str(receipt),'sha256':digest(receipt)},'tools':{'encoder':{'path':str(tool),'sha256':toolhash},'validator':{'path':str(tool),'sha256':toolhash}}}
   rows=[r for r in model.entries(REAL/'tools/wtime-bench/workloads.json') if r['id']=='prepare_large']
   wasm_source=REAL/'target/performance/P033-before-corrected/fixtures/prepare_large.wasm'
   self.assertEqual(digest(wasm_source),rows[0]['source']['wasm_sha256'])
   calls=[]
   def mock(argv,cwd,timeout=180):
    calls.append(argv)
    if any(str(x).endswith('generate_prepare_large.py') for x in argv):
     # Execute only the deterministic generator helper, never a runtime/guest.
     import subprocess
     return subprocess.run(argv,cwd=cwd,text=True,capture_output=True,check=False)
    if 'encode' in argv:
     dest=Path(argv[-1]);producer.contained(out,dest);shutil.copyfile(wasm_source,dest)
    return Done()
   commands=[]
   with patch.object(producer,'P125_ENCODER_SHA256',toolhash),patch.object(producer,'P125_VALIDATOR_SHA256',toolhash):
    values=producer._fixture_files(root,out,rows,lock,mock,commands)
   item=values['prepare_large'];self.assertTrue(item['wasm'].is_file());self.assertIn(out,item['wasm'].parents)
   labels=[c['label'] for c in commands]
   self.assertIn('encode-prepare_large',labels);self.assertIn('validate-prepare_large',labels);self.assertIn('generate-prepare-large',labels)
   self.assertTrue(any(x[1:4]==['--profile','mvp','check'] for x in calls))
if __name__=='__main__':unittest.main()
