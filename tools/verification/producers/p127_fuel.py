"""P127 staged observation/proof runner draft, bound to the real P128 source helpers."""
from __future__ import annotations
import argparse, hashlib, importlib.util, json, math, shutil, subprocess, sys
from pathlib import Path
from typing import Any, Callable
HERE=Path(__file__).resolve()
ROOT=HERE.parents[3]
DRAFT=ROOT
BENCH=ROOT/'tools/wtime-bench'
MODEL_PATH=DRAFT/'tools/wtime-bench/p127_fuel_policy.py'
P128_PATH=BENCH/'p128_scalar_scaling.py'
P125_LOCK_REL='docs/benchmarks/performance/P125-fixture-tools.json'
P125_LOCK_SHA256='cbbec91d1a4b7f4b6379ac11c2ad37059cfefaa27480beff958c0e3eaa80ff38'
P125_ENCODER_SHA256='5123b38fd1e6b2358027778d94256202e53ff7024b3b05cd94aa5ea50d7949d4'
P125_VALIDATOR_SHA256='c543c8a9690ddf9e5df10548b87bc476a7cc7bb9cc33031030003f4e01f776ab'
sys.path.insert(0,str(BENCH))
sys.path.insert(0,str(MODEL_PATH.parent))
import p128_scalar_scaling as p128
import p127_fuel_policy as model
class P127Error(RuntimeError): pass

def sha(path:Path)->str:return hashlib.sha256(Path(path).read_bytes()).hexdigest()
def contained(root:Path,path:Path)->Path:
 root=Path(root).resolve();path=Path(path).resolve()
 if path==root or root not in path.parents:raise P127Error(f'path outside owned root: {path}')
 return path
def write_new(root:Path,path:Path,value:Any)->Path:
 path=contained(root,path);path.parent.mkdir(parents=True,exist_ok=True)
 with path.open('x',encoding='utf-8') as f:json.dump(value,f,sort_keys=True,indent=2,allow_nan=False);f.write('\n')
 return path
def _run(argv:list[str],cwd:Path,timeout:int=180):return subprocess.run(argv,cwd=cwd,text=True,capture_output=True,timeout=timeout,check=False)
def _command(root:Path,label:str,argv:list[str],cwd:Path,runner:Callable,commands:list,timeout:int=180):
 ap=write_new(root,root/'commands'/f'{label}.argv.json',{'argv':argv,'cwd':str(cwd)})
 result=runner(argv,cwd,timeout)
 stdout=contained(root,root/'commands'/f'{label}.stdout');stderr=contained(root,root/'commands'/f'{label}.stderr')
 stdout.parent.mkdir(parents=True,exist_ok=True);stdout.write_text(result.stdout or '',encoding='utf-8');stderr.write_text(result.stderr or '',encoding='utf-8')
 status=write_new(root,root/'commands'/f'{label}.status.json',{'exit_code':result.returncode})
 commands.append({'label':label,'argv':argv,'exit_code':result.returncode,'argv_path':ap.relative_to(root).as_posix(),'stdout_path':stdout.relative_to(root).as_posix(),'stderr_path':stderr.relative_to(root).as_posix(),'status_path':status.relative_to(root).as_posix(),'stdout_sha256':sha(stdout),'stderr_sha256':sha(stderr)})
 if result.returncode:raise P127Error(f'command failed {label}: {result.returncode}: {(result.stderr or '')[-1000:]}')
 return result

def _load_model():return model

def _current_sources(root:Path):
 compiled,proof=p128._source_maps(root)
 proof=dict(proof)
 proof[str(HERE.resolve().relative_to(root.resolve()))]=sha(HERE)
 proof[str(MODEL_PATH.resolve().relative_to(root.resolve()))]=sha(MODEL_PATH)
 proof[str(P128_PATH.resolve().relative_to(root.resolve()))]=sha(P128_PATH)
 return dict(compiled),dict(sorted(proof.items()))

def _typed_equal(raw:dict[str,Any],key:str,wanted:Any)->bool:
 if key not in raw:return False
 actual=raw[key]
 if wanted is None:return actual is None
 if type(wanted) is int:return type(actual) is int and actual==wanted
 if type(wanted) is bool:return type(actual) is bool and actual is wanted
 if type(wanted) is str:return type(actual) is str and actual==wanted
 return type(actual) is type(wanted) and actual==wanted

def _validate_response(raw:Any,expected:dict[str,Any],wire:str)->dict[str,Any]:
 if not isinstance(raw,dict):raise P127Error('observation must be an object')
 setting=model.policy(expected['fuel_policy'])
 if wire=='scalar-driver':
  n=expected['input'];result=expected['result'];exp=expected['expected']
  fields={'schema_version':2,'case':expected['workload'],'stage':expected['stage'],'input':f'i32:{n}',
          'expected':f'i32:{exp}','result':f'i32:{result}','batch':expected['batch'],'mode':expected['mode'],
          'profile':'mvp','driver':'plain','diagnostic':False,'source_fuel':setting['source_fuel'],'fuel_policy':setting['name']}
 else:
  # The existing single-case route has a numeric wire schema and no scalar-driver-only fields.
  is_prepare=expected['workload']=='prepare_large'
  fields={'schema_version':2,'case':expected['workload'],'stage':expected['stage'],
          'input':0 if is_prepare else expected['input'],'expected':0 if is_prepare else expected['expected'],
          'result':None if is_prepare else expected['result'],'batch':expected['batch'],'mode':expected['mode'],
          'source_fuel':setting['source_fuel'],'fuel_policy':setting['name']}
 if any(not _typed_equal(raw,k,v) for k,v in fields.items()):raise P127Error(f'{wire} response differs from expected identity/policy: {expected["workload"]}/{expected["stage"]}')
 duration=raw.get('ns_per_operation')
 if type(duration) not in (int,float) or not math.isfinite(duration) or duration<=0:raise P127Error('missing/invalid duration field')
 return {'workload':expected['workload'],'stage':expected['stage'],'mode':expected['mode'],'fuel_policy':setting['name'],
         'source_fuel':setting['source_fuel'],'batch':expected['batch'],'input':expected['input'],
         'expected':expected['expected'],'result':expected['result'],'timing_value_used':False}

def _live_protocol(binary:Path,mode:str,record_protocol:dict,output:Path,runner:Callable,commands:list,cwd:Path):
 proc=_command(output,f'protocol-{mode}',[str(binary),'--protocol'],cwd,runner,commands,timeout=30)
 raw_path=contained(output,output/'protocol'/f'{mode}.stdout');raw_path.parent.mkdir(parents=True,exist_ok=True);raw_path.write_text(proc.stdout or '',encoding='utf-8')
 try:response=json.loads(proc.stdout)
 except json.JSONDecodeError as e:raise P127Error(f'{mode} live protocol JSON invalid') from e
 write_new(output,output/'protocol'/f'{mode}.response.json',response)
 if response!=record_protocol:raise P127Error(f'{mode} current protocol differs from retained build record')
 if (response.get('schema_version')!=2 or response.get('engine')!='wtime' or response.get('instrumented') is not False or
     mode not in response.get('modes',[]) or 'mvp' not in response.get('features',[]) or
     response.get('source_fuel_contract')!=model.protocol_contract()):raise P127Error(f'{mode} protocol missing typed fuel contract')
 return response

def _fixture_files(root:Path,output:Path,rows:list[dict[str,Any]],lock:dict[str,Any],runner:Callable,commands:list):
 tools={}
 for name,digest in [('encoder',P125_ENCODER_SHA256),('validator',P125_VALIDATOR_SHA256)]:
  item=lock.get('tools',{}).get(name,{})
  path=Path(item.get('path','')).resolve(strict=True)
  if item.get('sha256')!=digest or sha(path)!=digest or root not in path.parents:raise P127Error(f'pinned P125 {name} identity invalid')
  tools[name]=str(path)
 receipt=lock.get('source_receipt',{});receipt_path=Path(receipt.get('path','')).resolve(strict=True)
 if root not in receipt_path.parents or sha(receipt_path)!=receipt.get('sha256'):raise P127Error('P125 source receipt invalid')
 by_key={};by_id={}
 for row in rows:
  src=row['source'];key=(src['kind'],src['path'],src['sha256'],src['wasm_sha256'])
  if key in by_key:by_id[row['id']]=by_key[key];continue
  if src['kind']=='wat_file':
   source=(root/'tools/wtime-bench'/src['path']).resolve(strict=True)
   if root not in source.parents or sha(source)!=src['sha256']:raise P127Error(f'fixture source changed: {src["path"]}')
   staged=contained(output,output/'fixtures'/Path(src['path']).name);staged.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(source,staged)
  else:
   generator_source=root/'tools/wtime-bench/workloads.py'
   helper=contained(output,output/'executed/generate_prepare_large.py');helper.parent.mkdir(parents=True,exist_ok=True)
   shutil.copyfile(generator_source,contained(output,output/'executed/workloads.py'))
   helper.write_text("from pathlib import Path\nimport sys\nsys.path.insert(0,sys.argv[1])\nimport workloads\nPath(sys.argv[2]).write_text(workloads.generate_prepare_large()+'\\n',encoding='utf-8')\n",encoding='utf-8')
   staged=contained(output,output/'fixtures/prepare_large.wat');staged.parent.mkdir(parents=True,exist_ok=True)
   _command(output,'generate-prepare-large',[sys.executable,'-B',str(helper),str(generator_source.parent),str(staged)],root,runner,commands)
   if sha(staged)!=src['sha256']:raise P127Error('generated prepare_large WAT hash mismatch')
  fixture_id=Path(src['path'].split(':')[0]).stem if src['kind']=='wat_file' else 'prepare_large'
  wasm=contained(output,output/'fixtures'/f'{fixture_id}.wasm')
  _command(output,'encode-'+fixture_id,[tools['encoder'],'encode',str(staged),'-o',str(wasm)],root,runner,commands)
  if not wasm.is_file() or sha(wasm)!=src['wasm_sha256']:raise P127Error(f'encoded fixture hash mismatch: {fixture_id}')
  _command(output,'validate-'+fixture_id,[tools['validator'],'--profile','mvp','check',str(wasm)],root,runner,commands)
  value={'wat':staged,'wasm':wasm,'source_sha256':src['sha256'],'wasm_sha256':src['wasm_sha256']};by_key[key]=value;by_id[row['id']]=value
 return by_id


def _validate_matrix(plan:list[dict[str,Any]],observations:list[dict[str,Any]]):
 expected={(x['workload'],x['stage'],x['mode'],x['fuel_policy']) for x in plan}
 actual=[(x.get('workload'),x.get('stage'),x.get('mode'),x.get('fuel_policy')) for x in observations]
 if len(plan)!=184 or len(actual)!=184 or len(set(actual))!=184 or set(actual)!=expected:
  raise P127Error('matrix missing, duplicate, or unexpected')
 return True

def _driver_argv(binary:Path,fixture:Path,row:dict,mode:str,setting:dict)->tuple[list[str],str]:
 wid=row.get('workload',row.get('id'));stage=row['stage'];batch=row['batch'];fuel=model.cli_value(setting)
 if wid in ('call_overhead','prepare_large'):
  input_value=0 if wid=='prepare_large' else row['input'];expected=0 if wid=='prepare_large' else row['expected']
  return [str(binary),'--single-case',str(fixture),wid,str(input_value),str(expected),stage,str(batch),mode,'--source-fuel',fuel],'single-case'
 return [str(binary),'--scalar-driver',str(fixture),wid,str(row['input']),str(row['expected']),stage,str(batch),mode,'mvp','plain','--source-fuel',fuel],'scalar-driver'

def run(decoded_record:Path,compact_record:Path,registry:Path,lock:Path,output:Path,*,root:Path=ROOT,
        source_reader:Callable|None=None,command_runner:Callable|None=None)->dict[str,Any]:
 root=Path(root).resolve(strict=True);output=Path(output).resolve()
 if root not in output.parents:raise P127Error('output must be a strict child of the workspace')
 output.mkdir(parents=True,exist_ok=False);runner=command_runner or _run
 if sha(registry)!=model.REGISTRY_SHA256:raise P127Error('frozen registry hash changed')
 rows=model.entries(registry);plan=model.matrix(rows)
 if len(plan)!=184:raise P127Error('frozen P127 matrix is not 184 observations')
 if sha(lock)!=P125_LOCK_SHA256:raise P127Error('P125 fixture-tools lock hash changed')
 lock_data=json.loads(Path(lock).read_text());commands=[]
 source_reader=source_reader or _current_sources
 compiled,proof=source_reader(root);compiled=dict(compiled);proof=dict(proof)
 # P128's helpers strictly validate the real runtime-only record schema, source maps, features, retained protocol bytes and binaries.
 records={}
 for mode,path in [('decoded',decoded_record),('compact',compact_record)]:
  record,binary,record_sha=p128._verify_record(root,path,mode,compiled)
  if record.get('protocol',{}).get('source_fuel_contract')!=model.protocol_contract():raise P127Error(f'{mode} record protocol lacks P127 fuel contract')
  records[mode]=(record,binary,Path(path).resolve(strict=True),record_sha,sha(binary))
 if records['decoded'][1]==records['compact'][1]:raise P127Error('decoded and compact binaries must be distinct')
 # Archive the entire current P128 compiled/proof input closure before commands.
 p128._archive_sources(root,output,proof)
 for name,path in [('registry',registry),('P125-fixture-tools.json',lock)]:
  dest=contained(output,output/'executed'/name);dest.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(path,dest)
  if sha(dest)!=sha(path):raise P127Error(f'{name} archive hash mismatch')
 write_new(output,output/'executed/source-maps.json',{'compiled':compiled,'proof':proof})
 write_new(output,output/'executed/producer.argv.json',{'argv':sys.argv,'cwd':str(Path.cwd())})
 for mode,(record,binary,record_path,record_sha,binary_sha) in records.items():
  archived=contained(output,output/'executed/production-records'/f'{mode}.json');archived.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(record_path,archived)
  if sha(archived)!=record_sha:raise P127Error(f'{mode} record changed while archiving')
  _live_protocol(binary,mode,record['protocol'],output,runner,commands,root)
 fixtures=_fixture_files(root,output,rows,lock_data,runner,commands)
 observations=[]
 for mode,(record,binary,record_path,record_sha,binary_sha) in records.items():
  for item in plan:
   if item['mode']!=mode:continue
   row={**item,'expected':item['expected'],'result':item['result']}
   argv,wire=_driver_argv(binary,fixtures[item['workload']]['wasm'],row,mode,model.policy(item['fuel_policy']))
   label=f"{mode}.{item['workload']}.{item['stage']}.{item['fuel_policy']}"
   proc=_command(output,label,argv,root,runner,commands)
   stdout_path=contained(output,output/'observations'/f'{label}.stdout');stdout_path.parent.mkdir(parents=True,exist_ok=True);stdout_path.write_text(proc.stdout or '',encoding='utf-8')
   json_path=contained(output,output/'observations'/f'{label}.json')
   try:raw=json.loads(proc.stdout)
   except json.JSONDecodeError as e:raise P127Error(f'bad JSON retained at {stdout_path}') from e
   write_new(output,json_path,raw)
   proof_row=_validate_response(raw,row,wire)
   proof_row.update({'wire':wire,'argv_path':f'commands/{label}.argv.json','raw_stdout_path':stdout_path.relative_to(output).as_posix(),'raw_stdout_sha256':sha(stdout_path),'raw_json_path':json_path.relative_to(output).as_posix(),'raw_json_sha256':sha(json_path)})
   observations.append(proof_row)
 # Validate the complete input closure again after the proof.
 if source_reader(root)!=(compiled,proof):raise P127Error('current compiled/proof source maps changed during proof')
 if sha(registry)!=model.REGISTRY_SHA256 or sha(lock)!=P125_LOCK_SHA256:raise P127Error('registry or P125 lock changed during proof')
 for mode,(_,binary,record_path,record_sha,binary_sha) in records.items():
  if sha(record_path)!=record_sha or sha(binary)!=binary_sha:raise P127Error(f'{mode} production input changed during proof')
 _validate_matrix(plan,observations)
 report={'schema':'wtime-p127-fuel-foundation-v1','status':'passed','source_stable':True,'instrumented':False,'performance_candidate':False,'timing_analysis':False,'speed_decision':None,
 'registry_sha256':sha(registry),'registry_rows':459,'selection_ids':list(model.ROW_IDS),'selection_count':10,
 'compiled_sources':compiled,'proof_sources':proof,'policies':model.protocol_contract()['accepted'],'modes':list(model.MODES),
 'matrix_rows':184,'driver_observations':observations,'commands':commands,'fixtures':{k:{a:(str(vv.relative_to(output)) if isinstance(vv,Path) else vv) for a,vv in v.items()} for k,v in fixtures.items()},
 'warm_batch':3,'counts':{'observations':184,'observation_commands':184,'commands_total':len(commands),'by_mode':{m:sum(x['mode']==m for x in observations) for m in model.MODES},'by_policy':{p[0]:sum(x['fuel_policy']==p[0] for x in observations) for p in model.POLICIES}},
 'limitations':['The fixed source_fuel_contract and every observation bind None or Some(50_000_000).','No timing analysis, paired comparison, calibration or speed claim.']}
 write_new(output,output/'report.json',report);return report

def main(argv=None):
 p=argparse.ArgumentParser();p.add_argument('--decoded-record',type=Path,required=True);p.add_argument('--compact-record',type=Path,required=True);p.add_argument('--registry',type=Path,required=True);p.add_argument('--lock',type=Path,required=True);p.add_argument('--output',type=Path,required=True);p.add_argument('--root',type=Path,default=ROOT);a=p.parse_args(argv)
 try:r=run(a.decoded_record,a.compact_record,a.registry,a.lock,a.output,root=a.root)
 except (OSError,ValueError,KeyError,P127Error,p128.P128Error) as e:print(f'P127 proof failed: {e}',file=sys.stderr);return 1
 print(f"P127 proof passed: {r['counts']['observations']} observations; timing analysis disabled");return 0
if __name__=='__main__':raise SystemExit(main())
