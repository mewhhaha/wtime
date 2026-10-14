"""Compile the actual protocol tests in a five-local-package Cargo workspace."""
from pathlib import Path
import argparse,hashlib,json,os,shutil,subprocess,sys
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'tools/wtime-bench'))
import focused_compare
FEATURES=['runtime-only','compact-executor','execution-reference']
NAMES=['p127_omitted_and_explicit_unmetered_preserve_distinct_wire_identity','p127_fixed_metered_policy_sets_fuel_without_changing_other_limits','p127_malformed_or_unsupported_fuel_arguments_are_rejected','p127_scalar_builder_uses_selected_fuel_at_guest_source_boundary','p127_single_case_builder_uses_selected_fuel_at_guest_source_boundary']
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def write(p,v):
 p.parent.mkdir(parents=True,exist_ok=True)
 with p.open('x') as f:json.dump(v,f,indent=2,sort_keys=True);f.write('\n')
def run(output):
 output=Path(output).resolve()
 if ROOT.resolve() not in output.parents:raise ValueError('output outside repository')
 output.mkdir(parents=True,exist_ok=False);shutil.copy2(__file__,output/'helper.executed.py')
 before=focused_compare.sources()
 write(output/'argv.json',{'argv':sys.argv,'sources':before})
 manifest='''[package]
name="wtime-p127-driver-tests"
version="0.1.0"
edition="2024"
publish=false
[workspace]
[features]
default=[]
runtime-only=[]
compact-executor=["wtime-runtime/compact-executor"]
execution-reference=["wtime-runtime/execution-reference"]
[dependencies]
'''
 for name in ['wtime-runtime','wtime-types']:manifest+=name+'={path='+json.dumps(str(ROOT/'crates'/name))+'}\n'
 (output/'Cargo.toml').write_text(manifest)
 source='#![forbid(unsafe_code)]\n#[path='+json.dumps(str(ROOT/'tools/wtime-bench/src/protocol.rs'))+']\nmod protocol;\nfn main() {}\n'
 (output/'src').mkdir();(output/'src/main.rs').write_text(source)
 for raw,digest in before.items():
  p=ROOT/raw;dest=output/'sources'/raw;dest.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(p,dest)
  if sha(dest)!=digest:raise ValueError('source copy mismatch: '+raw)
 env=dict(os.environ);env['CARGO_TARGET_DIR']=str(ROOT/'tools/wtime-bench/target')
 commands=[]
 def invoke(name,argv):
  write(output/(name+'.argv.json'),{'argv':argv,'cwd':str(output),'cargo_target_dir':env['CARGO_TARGET_DIR']})
  p=subprocess.run(argv,cwd=output,env=env,capture_output=True,text=True)
  (output/(name+'.stdout')).write_text(p.stdout);(output/(name+'.stderr')).write_text(p.stderr)
  row={'name':name,'exit_code':p.returncode,'stdout_sha256':sha(output/(name+'.stdout')),'stderr_sha256':sha(output/(name+'.stderr'))};write(output/(name+'.result.json'),row);commands.append(row)
  if p.returncode:raise ValueError(name+' failed; retained logs: '+str(output))
  return p
 invoke('lock',['cargo','generate-lockfile','--offline'])
 p=invoke('tests',['cargo','test','--locked','--offline','--features',','.join(FEATURES),'p127_','--','--nocapture'])
 sys.path.insert(0,str(ROOT/'tools'))
 from verification.runner import test_results
 result=test_results(p.stdout+'\n'+p.stderr,{'required_names':NAMES,'minimum_passed':5,'features':FEATURES})
 if focused_compare.sources()!=before:raise ValueError('sources changed during driver tests')
 write(output/'report.json',{'status':'passed','source_stable':True,'features':FEATURES,'tests':result,'commands':commands,'sources':before,'instrumented':True,'timing_evidence':False,'workspace_manifest_sha256':sha(output/'Cargo.toml'),'workspace_lock_sha256':sha(output/'Cargo.lock')})
 print(p.stdout,end='');print(json.dumps({'status':'passed','named_driver_tests':5}))
if __name__=='__main__':
 p=argparse.ArgumentParser();p.add_argument('--output',type=Path,required=True);a=p.parse_args()
 try:run(a.output)
 except (OSError,ValueError,KeyError) as e:print(str(e),file=sys.stderr);raise SystemExit(1)
