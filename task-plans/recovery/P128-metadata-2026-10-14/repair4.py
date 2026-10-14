#!/usr/bin/env python3
"""Finish wording alignment, source validators, and snapshot manifest refresh."""
import hashlib,json,os,stat,subprocess
from pathlib import Path
ROOT=Path('/workspace/wtime');SRC=Path('/workspace/tasks');DST=ROOT/'task-plans';ARCH=DST/'recovery/P128-metadata-2026-10-14';TASK=Path('performance/04-specialization-and-fusion/P128-minimal-scalar-workloads.md');PLAN=Path('performance/TASKS.json');MAN=ROOT/'docs/recovery/task-plan-snapshot.json'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 p=SRC/TASK;text=p.read_text();text=text.replace('Separately, normalize every workload','Normalize every workload',1).replace('This remains exactly **180 real observations/driver commands**','This is exactly **180 real observations/driver commands**',1);p.write_text(text);dst=DST/TASK;dst.write_text(text);os.chmod(dst,stat.S_IMODE(p.stat().st_mode))
 cmds=json.loads((ARCH/'argv.json').read_text())['validator_commands'];rs=[];log=ARCH/'validator-runs.jsonl'
 for c in cmds:
  src=Path(c[-1]);cwd=SRC if src.parent==SRC else src.parent;pre={'command':c,'cwd':str(cwd),'validator_source':str(src),'validator_source_sha256':sha(src),'plan_manifest_sha256':sha(src.parent/'TASKS.json')}
  with log.open('a') as f:f.write(json.dumps({'pre_run_final_wording':pre},sort_keys=True)+'\n');f.flush();os.fsync(f.fileno())
  q=subprocess.run(c,cwd=cwd,capture_output=True,text=True);r={**pre,'exit_code':q.returncode,'stdout':q.stdout,'stderr':q.stderr};rs.append(r)
  with log.open('a') as f:f.write(json.dumps({'final_wording_result':r},sort_keys=True)+'\n');f.flush();os.fsync(f.fileno())
 m=json.loads(MAN.read_text());m.setdefault('validator_history',[]).append(m.get('validators',[]));m['validators']=[{'command':r['command'],'exit_code':r['exit_code'],'stdout':r['stdout'],'stderr':r['stderr'],'validator_source_sha256':r['validator_source_sha256'],'plan_manifest_sha256':r['plan_manifest_sha256']} for r in rs]
 for rel in [TASK,PLAN]:
  s=SRC/rel;b=DST/rel;e=next(z for z in m['files'] if z['path']==rel.as_posix());e.update(size_bytes=s.stat().st_size,sha256=sha(s),snapshot_sha256=sha(b),byte_equal=s.read_bytes()==b.read_bytes(),mode_octal=format(stat.S_IMODE(s.stat().st_mode),'04o'))
 m['snapshot_source_byte_equality']=all((SRC/x['path']).is_file() and (SRC/x['path']).read_bytes()==(DST/x['path']).read_bytes() and sha(SRC/x['path'])==x['sha256'] for x in m['files']);MAN.write_text(json.dumps(m,indent=2,sort_keys=True)+'\n')
 print(json.dumps({'validators':[{'command':r['command'],'exit_code':r['exit_code'],'stdout':r['stdout'].strip(),'stderr':r['stderr'].strip()} for r in rs],'snapshot_source_byte_equality':m['snapshot_source_byte_equality']},indent=2))
 if any(r['exit_code'] for r in rs) or not m['snapshot_source_byte_equality']:raise SystemExit(1)
if __name__=='__main__':main()
