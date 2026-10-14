"""Revalidate current source-bound P047 cases and P128 source/fuel observations."""
from pathlib import Path
import argparse,hashlib,json,sys
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'tools/wtime-bench'))
import focused_compare,scalar_p128_workloads as model,selection_manifest,semantic_receipt
def sha(path):return hashlib.sha256(Path(path).read_bytes()).hexdigest()
def need(ok,msg):
 if not ok:raise ValueError(msg)
def run(semantic_path,source_path,output):
 source_path=Path(source_path).resolve(strict=True);semantic_path=Path(semantic_path).resolve(strict=True)
 core={name:sha(ROOT/name) for name in selection_manifest._current_core_paths(ROOT/'tools/wtime-bench/workloads.json')}
 before=sha(source_path);d=json.loads(source_path.read_text())
 need(d.get('status')=='passed' and d.get('source_stable') is True and d.get('runtime_sources')==core,'source proof is not current')
 need(d.get('observation_commands')==126 and len(d.get('rows',[]))==18,'source proof matrix differs')
 expected={(family.removeprefix('p128_'),n) for family in model.FIXTURES for n in model.FIXED_INPUTS};seen=set()
 for row in d['rows']:
  key=(row['fixture'],row['n']);need(key in expected and key not in seen,'duplicate/unexpected source proof row');seen.add(key)
  events=model.source_event_count(row['n']);result=model.oracle('p128_'+row['fixture']+'_n'+str(row['n']),row['n'])
  need(row['source_events']==events and row['result']==result,'source/result model differs from actual proof')
  checks=row.get('checks',[]);need(len(checks)==6,'missing fuel probes')
  wanted={(mode,budget) for mode in ['decoded','compact','reference'] for budget in [events-1,events]};actual=set()
  for check in checks:
   k=(check['mode'],check['budget']);need(k in wanted and k not in actual,'wrong/duplicate mode/budget');actual.add(k)
   need(check['n']==row['n'] and check['status']==('returned' if check['budget']==events else 'fuel exhausted'),'fuel boundary differs')
   need(check['result']==(result if check['budget']==events else None),'fuel result differs')
  need(actual==wanted,'missing fuel boundary tuple')
 need(seen==expected,'missing source proof row')
 for name,ref in d['libraries'].items():need(sha(ref['path'])==ref['sha256'],'changed diagnostic library '+name)
 need(sha(d['probe']['path'])==d['probe']['sha256'],'changed diagnostic probe')
 need(sha(source_path.parent/'probe.rs')==d['probe']['source_sha256'],'changed diagnostic source')
 parent=json.loads(semantic_path.read_text());names=set(parent['tests']['compact-semantics']['names']);short={n.rsplit('::',1)[-1] for n in names}
 # Select actual prefixed P047 names from the full passing parent lane.
 required=sorted(n for n in short if n.startswith('p047_'));need(len(required)==7,'missing seven P047 cases')
 value=semantic_receipt.validate(semantic_path,{'task':'P050','semantic_cases':required},focused_compare.sources(),ROOT)
 need(before==sha(source_path),'proof changed during validation')
 need(core=={name:sha(ROOT/name) for name in selection_manifest._current_core_paths(ROOT/'tools/wtime-bench/workloads.json')},'core changed during validation')
 record={'status':'passed','runtime_core_unchanged':True,'semantic_prerequisite':value,'p047_case_count':7,'source_event_rows':18,'source_event_commands':126,'source_event_formula':'9 + 14*n','source_events_match':True,'exact_fuel_boundaries_match':True,'source_proof':{'path':str(source_path),'sha256':before},'semantic_receipt':{'path':str(semantic_path),'sha256':sha(semantic_path)},'runtime_sources':core,'performance_candidate':False}
 output=Path(output);output.parent.mkdir(parents=True,exist_ok=True)
 with output.open('x') as f:json.dump(record,f,indent=2);f.write('\n')
 return record
if __name__=='__main__':
 p=argparse.ArgumentParser();p.add_argument('--semantic-receipt',type=Path,required=True);p.add_argument('--source-proof',type=Path,required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args()
 try:r=run(a.semantic_receipt,a.source_proof,a.output)
 except (OSError,ValueError,KeyError) as e:print(str(e),file=sys.stderr);raise SystemExit(1)
 print(json.dumps({'status':r['status'],'p047_case_count':7,'source_event_rows':18}))
