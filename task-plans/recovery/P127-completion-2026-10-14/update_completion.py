#!/usr/bin/env python3
"""Write P127 completion notes, sync the full task snapshot, validate, and refresh its manifest."""
import collections
import hashlib
import json
import os
import shutil
import stat
import subprocess
from pathlib import Path

ROOT=Path('/workspace/wtime'); SRC=Path('/workspace/tasks'); DST=ROOT/'task-plans'
ARCH=DST/'recovery/P127-completion-2026-10-14'; PLAN=Path('performance/TASKS.json')
TASK=Path('performance/04-specialization-and-fusion/P127-metered-unmetered-benchmark-setting.md')
MAN=ROOT/'docs/recovery/task-plan-snapshot.json'
EVIDENCE_SHA='cd1c924fb67c6236108caa38241d9dc538c4b22820d2cc7aa14c74c4372c0ab5'
EVIDENCE_PATH='/workspace/wtime/target/performance/P127/shared-finalization/attempt-0001/evidence.json'
COMPLETION_PATH='/workspace/wtime/target/performance/P127/shared-finalization/attempt-0001/completion.json'
VERIFY='/workspace/wtime/target/verification/runs/P127/verify/attempt-0001'

def sha(path):return hashlib.sha256(path.read_bytes()).hexdigest()
def regular_sources():
 paths=[]
 for p in SRC.rglob('*'):
  if p.is_symlink(): raise RuntimeError(f'source symlink rejected: {p}')
  if p.is_file():
   if not stat.S_ISREG(p.lstat().st_mode):raise RuntimeError(f'nonregular source rejected: {p}')
   paths.append(p)
 return sorted(paths)
def main():
 pre=[
  (SRC/PLAN,ARCH/'pre-edit/source/performance/TASKS.json'),
  (SRC/TASK,ARCH/'pre-edit/source/performance/04-specialization-and-fusion/P127-metered-unmetered-benchmark-setting.md'),
  (DST/PLAN,ARCH/'pre-edit/snapshot/performance/TASKS.json'),
  (DST/TASK,ARCH/'pre-edit/snapshot/performance/04-specialization-and-fusion/P127-metered-unmetered-benchmark-setting.md'),
  (MAN,ARCH/'pre-edit/source/docs/recovery/task-plan-snapshot.json')]
 for a,b in pre:
  if not a.is_file() or not b.is_file() or a.read_bytes()!=b.read_bytes():raise RuntimeError(f'pre-edit archive mismatch: {a}')

 completion={
  'status':'done','outcome':'contract_only','attempt':'attempt-0001',
  'completion_record':COMPLETION_PATH,'evidence_path':EVIDENCE_PATH,'evidence_sha256':EVIDENCE_SHA,'evidence_bytes':1468,
  'verification_run':VERIFY,
  'verification_counts':{
   'named_checks':38,'python_checks':29,'driver_rust_checks':5,'private_rust_checks':4,'failed':0,'ignored':0,
   'staged_observations':184,'staged_total_commands':195,'by_mode':{'decoded':92,'compact':92},
   'by_policy':{'unmetered':92,'metered':92},'observation_commands':184,
   'warm_batch':3,'registry_rows':459,'selection_rows':10,
   'registry_sha256':'8917f14d06b2dd69d1abb1885d8186cdc11191924aef8ee073e356cec5cfd668',
   'production_instrumented':False,'runtime_core_source_files_unchanged':113,
  },
  'independent_fuel_proof':{
   'scalar_rows':9,'modes':['decoded','compact','execution-reference'],'policies':['unmetered','metered'],
   'repeats_per_tuple':3,'scalar_guest_calls':162,'reference_source_event_probes':9,
   'low_fuel_prefixes':24,'cursor_and_remaining_budget_windows':12,
   'continuation_transition_tuples':27,'continuation_guest_calls':54,
   'accounting_merge_tuples':5,'error_priority_tuples':3,
   'private_diagnostic_instrumented':True,'diagnostic_test_only_append':True,
   'runtime_core_unchanged':True,'timing_evidence':False,
  },
  'prerequisites':{'current_source_cases':12,'receipt':'/workspace/wtime/target/verification/runs/P050/verify/attempt-0002/receipt.json','p050_case_count':354},
  'initial_workspace_compile_failure':{
   'path':'/workspace/wtime/target/performance/P127/independent-fuel-proof/attempt-0001/diagnostic-tests.stderr',
   'detail':'The copied private diagnostic workspace lacked required P113 integration-control and prepare_large Wasm fixtures; compilation stopped before any guest execution.',
   'retry_path':'/workspace/wtime/target/performance/P127/independent-fuel-proof/attempt-0002/diagnostic-tests.stderr',
   'retry_result':'The fixtures were staged into the private workspace and the diagnostic test compilation/execution passed.',
  },
  'limitations':['contract_only; no timing statistics or speed claims','No candidate was promoted','P053 owns later accounting optimization, paired calibration, and performance decisions'],
 }
 # Verify authoritative publisher evidence before writing any completion note.
 taskdata=json.loads((SRC/PLAN).read_text(encoding='utf-8'))
 task=next(t for t in taskdata['tasks'] if t['id']=='P127')
 expected=[{'path':EVIDENCE_PATH,'sha256':EVIDENCE_SHA,'bytes':1468}]
 if task.get('status')!='done' or task.get('outcome')!='contract_only' or task.get('evidence')!=expected:
  raise RuntimeError('publisher status/evidence differs from canonical completion record')
 if sha(Path(EVIDENCE_PATH))!=EVIDENCE_SHA:raise RuntimeError('published evidence hash mismatch')
 if taskdata['tasks'][0].get('id') is None:raise RuntimeError('unexpected task manifest')
 task['completion_notes']=completion
 (SRC/PLAN).write_text(json.dumps(taskdata,indent=2,ensure_ascii=False)+'\n',encoding='utf-8')

 md=SRC/TASK;doc=md.read_text(encoding='utf-8')
 start=doc.index('## Completion evidence\n');end=doc.index('\n## Executed evidence',start)
 note=f'''## Completion evidence

P127 completed as `done` / `contract_only` in verification attempt 0001. The canonical completion record is `{COMPLETION_PATH}`. Its generated evidence file is `{EVIDENCE_PATH}` (SHA256 `{EVIDENCE_SHA}`, 1,468 bytes). The publisher's `Executed evidence` block below remains intact. No candidate was promoted and the result contains no timing statistics or speed claim.

The full verification executed 38 named checks: 29 Python checks, five driver Rust checks, and four private Rust checks, with zero failures and zero ignored checks. The staged lane recorded 184 real observations in 195 total commands; the other 11 commands were preparation/protocol work. Decoded and Compact each contributed 92 observations, and unmetered and metered each contributed 92. It covered the ten fixed selections, all 459 registry rows at unchanged SHA256 `8917f14d06b2dd69d1abb1885d8186cdc11191924aef8ee073e356cec5cfd668`, and warm batch three. Both production binaries were uninstrumented, and 113 current runtime-core source files were unchanged.

The independent fuel proof made 162 guest calls across nine scalar rows, three modes (`decoded`, `compact`, `execution-reference`), two source-fuel policies, and three repeats; it also ran nine reference source-event probes. Separate diagnostics covered 24 low-fuel prefixes, 12 cursor/remaining-budget windows, 27 continuation tuples across 54 guest calls, five accounting-merge tuples, and three error-priority tuples. Its copied runtime accounting code was a test-only diagnostic; no runtime/core source was edited. The production policy was `--source-fuel none` or `--source-fuel 50000000`; omitting the flag preserved the prior wire behavior. `None` and `Some(50_000_000)` were checked under the runtime-only, `compact-executor`, and `execution-reference` feature configurations, with the instrumented private fuel lane reserved for semantic diagnostics.

The initial private-workspace diagnostic compile stopped before guest execution because the copied workspace lacked P113 integration-control and `prepare_large` Wasm fixtures. The failure is retained at `/workspace/wtime/target/performance/P127/independent-fuel-proof/attempt-0001/diagnostic-tests.stderr`; after staging those fixture inputs, retry attempt 0002 passed at `/workspace/wtime/target/performance/P127/independent-fuel-proof/attempt-0002/diagnostic-tests.stderr`. The 12 current source prerequisites, including P020, P018, and T145, were inherited from the 354-case P050 receipt at `/workspace/wtime/target/verification/runs/P050/verify/attempt-0002/receipt.json`.

P053 owns later accounting optimization, paired calibration, and performance decisions. P127's completed result validates the fuel-policy protocol and semantics only; it makes no speed or adoption claim.'''
 md.write_text(doc[:start]+note+doc[end:],encoding='utf-8')

 # Synchronize exactly the authoritative source inventory (321 ordinary files), preserving bytes and modes.
 source_files=regular_sources()
 if len(source_files)!=321:raise RuntimeError(f'expected 321 source files, found {len(source_files)}')
 for src in source_files:
  rel=src.relative_to(SRC);dst=DST/rel;dst.parent.mkdir(parents=True,exist_ok=True)
  data=src.read_bytes()
  if not dst.exists() or dst.read_bytes()!=data:dst.write_bytes(data)
  os.chmod(dst,stat.S_IMODE(src.stat().st_mode))

 # Run both original validators. Log command, validator SHA, and current task-manifest SHA before execution.
 commands=json.loads((ARCH/'argv.json').read_text(encoding='utf-8'))['validator_commands'];runs=[];log=ARCH/'validator-runs.jsonl'
 for command in commands:
  source=Path(command[-1]);cwd=SRC if source.parent==SRC else source.parent
  record={'command':command,'cwd':str(cwd),'validator_source':str(source),'validator_source_sha256':sha(source),'plan_manifest_sha256':sha(source.parent/'TASKS.json')}
  with log.open('a',encoding='utf-8') as f:f.write(json.dumps({'pre_run':record},sort_keys=True)+'\n');f.flush();os.fsync(f.fileno())
  result=subprocess.run(command,cwd=cwd,capture_output=True,text=True)
  item={**record,'exit_code':result.returncode,'stdout':result.stdout,'stderr':result.stderr};runs.append(item)
  with log.open('a',encoding='utf-8') as f:f.write(json.dumps({'result':item},sort_keys=True)+'\n');f.flush();os.fsync(f.fileno())

 snapshot=json.loads(MAN.read_text(encoding='utf-8'))
 snapshot.setdefault('validator_history',[]).append(snapshot.get('validators',[]))
 snapshot['validators']=[{'command':r['command'],'exit_code':r['exit_code'],'stdout':r['stdout'],'stderr':r['stderr'],'validator_source_sha256':r['validator_source_sha256'],'plan_manifest_sha256':r['plan_manifest_sha256']} for r in runs]
 status_counts={}
 for label,path in [('original',SRC/'TASKS.json'),('performance',SRC/PLAN)]:
  tasks=json.loads(path.read_text(encoding='utf-8'))['tasks']
  status_counts[label]={'task_total':len(tasks),'status_counts':dict(sorted(collections.Counter(t.get('status','<missing>') for t in tasks).items()))}
 snapshot['task_status_counts']=status_counts
 copied=[]
 for src in source_files:
  rel=src.relative_to(SRC);dst=DST/rel
  if src.read_bytes()!=dst.read_bytes() or stat.S_IMODE(src.stat().st_mode)!=stat.S_IMODE(dst.stat().st_mode):copied.append(rel.as_posix())
 by_path={x['path']:x for x in snapshot['files']}
 if len(by_path)!=321:raise RuntimeError(f'snapshot manifest records {len(by_path)} entries, expected 321')
 for src in source_files:
  rel=src.relative_to(SRC).as_posix();dst=DST/rel;entry=by_path.get(rel)
  if entry is None:raise RuntimeError(f'missing snapshot manifest path {rel}')
  entry.update(size_bytes=src.stat().st_size,sha256=sha(src),snapshot_sha256=sha(dst),byte_equal=src.read_bytes()==dst.read_bytes(),mode_octal=format(stat.S_IMODE(src.stat().st_mode),'04o'))
 snapshot['snapshot_source_byte_equality']=not copied and all(e['byte_equal'] for e in snapshot['files'])
 MAN.write_text(json.dumps(snapshot,indent=2,sort_keys=True)+'\n',encoding='utf-8')
 print(json.dumps({'status':task['status'],'outcome':task['outcome'],'evidence_sha256':EVIDENCE_SHA,'validators':[{'command':r['command'],'exit_code':r['exit_code'],'stdout':r['stdout'].strip(),'stderr':r['stderr'].strip()} for r in runs],'source_file_count':len(source_files),'snapshot_file_count':len(snapshot['files']),'byte_mode_equal':snapshot['snapshot_source_byte_equality'],'status_counts':status_counts},indent=2))
 if any(r['exit_code'] for r in runs) or copied or not snapshot['snapshot_source_byte_equality']:raise SystemExit(1)

if __name__=='__main__':main()
