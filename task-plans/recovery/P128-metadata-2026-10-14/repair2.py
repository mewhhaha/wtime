#!/usr/bin/env python3
"""Align the P128 contract section, then validate and refresh its snapshot metadata."""
import hashlib,json,os,stat,subprocess
from pathlib import Path
ROOT=Path('/workspace/wtime');SRC=Path('/workspace/tasks');DST=ROOT/'task-plans';ARCH=DST/'recovery/P128-metadata-2026-10-14';TASK=Path('performance/04-specialization-and-fusion/P128-minimal-scalar-workloads.md');PLAN=Path('performance/TASKS.json');MAN=ROOT/'docs/recovery/task-plan-snapshot.json'
def sha(p): return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 dpath=SRC/TASK; d=dpath.read_text()
 surplus='\n\nThe independent source-event proof uses `9 + 14*n` dynamic events, counting the implicit function-end event, across 18 rows. It records 126 source-event/fuel/reference commands: one source-event observation and six fuel probes (decoded, compact, and execution-reference at event-count minus one and exact event-count) per row.'
 if surplus in d: d=d.replace(surplus,'',1)
 dpath.write_text(d)
 (DST/TASK).write_bytes(dpath.read_bytes());os.chmod(DST/TASK,stat.S_IMODE(dpath.stat().st_mode))
 cmds=json.loads((ARCH/'argv.json').read_text())['validator_commands']; runs=[]; log=ARCH/'validator-runs.jsonl'
 for cmd in cmds:
  src=Path(cmd[-1]);cwd=SRC if src.parent==SRC else src.parent
  pre={'command':cmd,'cwd':str(cwd),'validator_source':str(src),'validator_source_sha256':sha(src),'plan_manifest_sha256':sha(src.parent/'TASKS.json')}
  with log.open('a') as f:f.write(json.dumps({'pre_run_contract_alignment':pre},sort_keys=True)+'\n');f.flush();os.fsync(f.fileno())
  p=subprocess.run(cmd,cwd=cwd,capture_output=True,text=True); r={**pre,'exit_code':p.returncode,'stdout':p.stdout,'stderr':p.stderr};runs.append(r)
  with log.open('a') as f:f.write(json.dumps({'contract_alignment_result':r},sort_keys=True)+'\n');f.flush();os.fsync(f.fileno())
 m=json.loads(MAN.read_text());m.setdefault('validator_history',[]).append(m.get('validators',[]));m['validators']=[{'command':r['command'],'exit_code':r['exit_code'],'stdout':r['stdout'],'stderr':r['stderr'],'validator_source_sha256':r['validator_source_sha256'],'plan_manifest_sha256':r['plan_manifest_sha256']} for r in runs]
 for rel in [TASK,PLAN]:
  s=SRC/rel;b=DST/rel;e=next(x for x in m['files'] if x['path']==rel.as_posix());e.update(size_bytes=s.stat().st_size,sha256=sha(s),snapshot_sha256=sha(b),byte_equal=s.read_bytes()==b.read_bytes(),mode_octal=format(stat.S_IMODE(s.stat().st_mode),'04o'))
 m['snapshot_source_byte_equality']=all((SRC/x['path']).is_file() and (SRC/x['path']).read_bytes()==(DST/x['path']).read_bytes() and sha(SRC/x['path'])==x['sha256'] for x in m['files'])
 MAN.write_text(json.dumps(m,indent=2,sort_keys=True)+'\n')
 print(json.dumps({'validators':[{'command':r['command'],'exit_code':r['exit_code'],'stdout':r['stdout'].strip(),'stderr':r['stderr'].strip()} for r in runs],'snapshot_source_byte_equality':m['snapshot_source_byte_equality']},indent=2))
 if any(r['exit_code'] for r in runs) or not m['snapshot_source_byte_equality']:raise SystemExit(1)
if __name__=='__main__':main()
