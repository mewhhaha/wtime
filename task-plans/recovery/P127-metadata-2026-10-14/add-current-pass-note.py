#!/usr/bin/env python3
"""Record current implementation checks separately from full P127 close, then validate."""
import collections,hashlib,json,os,stat,subprocess
from pathlib import Path
ROOT=Path('/workspace/wtime');SRC=Path('/workspace/tasks');DST=ROOT/'task-plans';ARCH=DST/'recovery/P127-metadata-2026-10-14';PLAN=Path('performance/TASKS.json');TASK=Path('performance/04-specialization-and-fusion/P127-metered-unmetered-benchmark-setting.md');MAN=ROOT/'docs/recovery/task-plan-snapshot.json'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 p=SRC/TASK;d=p.read_text();needle='## Completion evidence\n\n';addition='Current production and ad-hoc proof runs pass, but the full contract close and publication remain pending; these checks do not complete P127.\n\n'
 if needle not in d:raise RuntimeError('completion evidence heading is missing')
 if addition.strip() not in d:d=d.replace(needle,needle+addition,1)
 p.write_text(d);b=DST/TASK;b.write_bytes(d.encode());os.chmod(b,stat.S_IMODE(p.stat().st_mode))
 rs=[];log=ARCH/'validator-runs.jsonl'
 for cmd in json.loads((ARCH/'argv.json').read_text())['validator_commands']:
  script=Path(cmd[-1]);cwd=SRC if script.parent==SRC else script.parent;pre={'command':cmd,'cwd':str(cwd),'validator_source':str(script),'validator_source_sha256':sha(script),'plan_manifest_sha256':sha(script.parent/'TASKS.json')}
  with log.open('a') as f:f.write(json.dumps({'pre_run_current_pass_note':pre},sort_keys=True)+'\n');f.flush();os.fsync(f.fileno())
  q=subprocess.run(cmd,cwd=cwd,capture_output=True,text=True);r={**pre,'exit_code':q.returncode,'stdout':q.stdout,'stderr':q.stderr};rs.append(r)
  with log.open('a') as f:f.write(json.dumps({'current_pass_note_result':r},sort_keys=True)+'\n');f.flush();os.fsync(f.fileno())
 mp=ROOT/'docs/recovery/task-plan-snapshot.json';m=json.loads(mp.read_text());m.setdefault('validator_history',[]).append(m.get('validators',[]));m['validators']=[{'command':r['command'],'exit_code':r['exit_code'],'stdout':r['stdout'],'stderr':r['stderr'],'validator_source_sha256':r['validator_source_sha256'],'plan_manifest_sha256':r['plan_manifest_sha256']} for r in rs]
 for rel in [TASK,PLAN]:
  s=SRC/rel;t=DST/rel;e=next(x for x in m['files'] if x['path']==rel.as_posix());e.update(size_bytes=s.stat().st_size,sha256=sha(s),snapshot_sha256=sha(t),byte_equal=s.read_bytes()==t.read_bytes(),mode_octal=format(stat.S_IMODE(s.stat().st_mode),'04o'))
 counts={}
 for n,path in [('original',SRC/'TASKS.json'),('performance',SRC/PLAN)]:
  rows=json.loads(path.read_text())['tasks'];counts[n]={'task_total':len(rows),'status_counts':dict(sorted(collections.Counter(x.get('status','<missing>') for x in rows).items()))}
 m['task_status_counts']=counts;m['snapshot_source_byte_equality']=all((SRC/x['path']).is_file() and (SRC/x['path']).read_bytes()==(DST/x['path']).read_bytes() and sha(SRC/x['path'])==x['sha256'] for x in m['files']);mp.write_text(json.dumps(m,indent=2,sort_keys=True)+'\n')
 print(json.dumps({'validators':[{'command':r['command'],'exit_code':r['exit_code'],'stdout':r['stdout'].strip(),'stderr':r['stderr'].strip()} for r in rs],'snapshot_source_byte_equality':m['snapshot_source_byte_equality'],'counts':counts},indent=2))
 if any(r['exit_code'] for r in rs) or not m['snapshot_source_byte_equality']:raise SystemExit(1)
if __name__=='__main__':main()
