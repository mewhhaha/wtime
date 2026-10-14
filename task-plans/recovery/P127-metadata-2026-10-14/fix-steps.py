#!/usr/bin/env python3
"""Align P127 manifest recipes to the expanded task note, rerun validators, refresh snapshot."""
import collections,hashlib,json,os,stat,subprocess
from pathlib import Path
ROOT=Path('/workspace/wtime');SRC=Path('/workspace/tasks');DST=ROOT/'task-plans';ARCH=DST/'recovery/P127-metadata-2026-10-14';PLAN=Path('performance/TASKS.json');MAN=ROOT/'docs/recovery/task-plan-snapshot.json'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 planp=SRC/PLAN;d=json.loads(planp.read_text());t=next(x for x in d['tasks'] if x['id']=='P127')
 t['implementation_steps'][2]='Keep execution mode, feature profile, fuel policy, and stage as separate typed fields. For `prepare`, record the selected setting even though no guest call occurs. For warm calls, bind the single selected policy and fixed batch of three to the observation; do not calibrate in P127. Keep production binaries uninstrumented: runtime-only for Decoded and `compact-executor` for Compact. Unit driver checks use runtime-only, `compact-executor`, and `execution-reference` feature configurations. The separate private fuel proof uses instrumented `compact-executor` plus `execution-reference` only for semantic diagnostics, never as a production measurement binary.'
 t['implementation_steps'][3]='Add a separate proof producer that checks every expected row/stage/mode/policy tuple, invokes the real scalar driver, archives command/stdout/stderr before oracle validation, and rejects missing, duplicate, mixed-setting, instrumented, or source-drifted records. Its full close collects all 184 observations and the independent fuel proof without running statistics. The independent proof covers nine scalar rows × three modes (`decoded`, `compact`, `execution-reference`) × two policies × three repeats (162 guest calls) plus nine reference source-event probes; separate diagnostics retain 24 low-fuel prefixes, 12 cursor windows, 27 continuation tuples/54 guest calls, and five merge/three priority unit tuples.'
 planp.write_text(json.dumps(d,indent=2,ensure_ascii=False)+'\n')
 a=SRC/PLAN;b=DST/PLAN;b.write_bytes(a.read_bytes());os.chmod(b,stat.S_IMODE(a.stat().st_mode))
 cmds=json.loads((ARCH/'argv.json').read_text())['validator_commands'];runs=[];log=ARCH/'validator-runs.jsonl'
 for cmd in cmds:
  script=Path(cmd[-1]);cwd=SRC if script.parent==SRC else script.parent;pre={'command':cmd,'cwd':str(cwd),'validator_source':str(script),'validator_source_sha256':sha(script),'plan_manifest_sha256':sha(script.parent/'TASKS.json')}
  with log.open('a') as f:f.write(json.dumps({'pre_run_recipe_alignment':pre},sort_keys=True)+'\n');f.flush();os.fsync(f.fileno())
  p=subprocess.run(cmd,cwd=cwd,capture_output=True,text=True);r={**pre,'exit_code':p.returncode,'stdout':p.stdout,'stderr':p.stderr};runs.append(r)
  with log.open('a') as f:f.write(json.dumps({'recipe_alignment_result':r},sort_keys=True)+'\n');f.flush();os.fsync(f.fileno())
 mpath=ROOT/'docs/recovery/task-plan-snapshot.json';m=json.loads(mpath.read_text());m.setdefault('validator_history',[]).append(m.get('validators',[]));m['validators']=[{'command':r['command'],'exit_code':r['exit_code'],'stdout':r['stdout'],'stderr':r['stderr'],'validator_source_sha256':r['validator_source_sha256'],'plan_manifest_sha256':r['plan_manifest_sha256']} for r in runs]
 counts={}
 for label,pp in [('original',SRC/'TASKS.json'),('performance',SRC/PLAN)]:
  rows=json.loads(pp.read_text())['tasks'];counts[label]={'task_total':len(rows),'status_counts':dict(sorted(collections.Counter(x.get('status','<missing>') for x in rows).items()))}
 m['task_status_counts']=counts
 for rel in [Path('performance/04-specialization-and-fusion/P127-metered-unmetered-benchmark-setting.md'),PLAN]:
  s=SRC/rel;b=DST/rel
  row=next(x for x in m['files'] if x['path']==rel.as_posix());row.update(size_bytes=s.stat().st_size,sha256=sha(s),snapshot_sha256=sha(b),byte_equal=s.read_bytes()==b.read_bytes(),mode_octal=format(stat.S_IMODE(s.stat().st_mode),'04o'))
 m['snapshot_source_byte_equality']=all((SRC/x['path']).is_file() and (SRC/x['path']).read_bytes()==(DST/x['path']).read_bytes() and sha(SRC/x['path'])==x['sha256'] for x in m['files']);mpath.write_text(json.dumps(m,indent=2,sort_keys=True)+'\n')
 print(json.dumps({'validators':[{'command':x['command'],'exit_code':x['exit_code'],'stdout':x['stdout'].strip(),'stderr':x['stderr'].strip()} for x in runs],'counts':counts,'byte_equality':m['snapshot_source_byte_equality']},indent=2))
 if any(x['exit_code'] for x in runs) or not m['snapshot_source_byte_equality']:raise SystemExit(1)
if __name__=='__main__':main()
