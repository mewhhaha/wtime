#!/usr/bin/env python3
"""Repair P128 plan consistency after the first original validator pass."""
import hashlib, json, os, stat, subprocess
from pathlib import Path
ROOT=Path('/workspace/wtime'); SRC=Path('/workspace/tasks'); DST=ROOT/'task-plans'
ARCHIVE=DST/'recovery/P128-metadata-2026-10-14'; TASK=Path('performance/04-specialization-and-fusion/P128-minimal-scalar-workloads.md'); PLAN=Path('performance/TASKS.json'); MANIFEST=ROOT/'docs/recovery/task-plan-snapshot.json'
def sha(p): return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
    planp=SRC/PLAN; plan=json.loads(planp.read_text()); t=next(x for x in plan['tasks'] if x['id']=='P128')
    docp=SRC/TASK; doc=docp.read_text()
    additions=[
      'tools/wtime-bench/execution_identity.py',
      'tools/wtime-bench/tests/test_fast_execution_identity.py',
      'tools/verification/producers/p128_evidence.py',
      'tools/verification/tests/data/admitted_wasm/p128_identity_copy.wasm',
      'tools/verification/tests/data/admitted_wasm/p128_literal_wrap_add.wasm',
      'tools/verification/tests/data/admitted_wasm/p128_literal_bitwise.wasm']
    if any(f'- `{x}`' not in doc for x in additions):
        marker='- `docs/benchmarks/performance/P128-verification.json`\n'
        if marker not in doc: raise RuntimeError('ownership marker missing')
        doc=doc.replace(marker, marker + ''.join(f'- `{x}`\n' for x in additions), 1)
    # The contract text must be identical after whitespace normalization.
    target=t['input_output_contract']
    first, sep, rest=target.partition('\n\n')
    if not sep: raise RuntimeError('contract paragraph boundary missing')
    needle='producing 18 model rows, never arbitrary ranges.'
    addition=' Exact dynamic source events, including the implicit function end, follow `9 + 14*n`; the source proof requires 18 rows and 126 source-event/fuel/reference commands (one event command plus six fuel probes per row).'
    if addition.strip() not in first:
        if needle not in first: raise RuntimeError('contract insertion point missing')
        first=first.replace(needle, needle+addition, 1)
    doc=doc.replace('The loop count aliases are fixed at `n=0,1,17,32,1024,10000`, producing 18 model rows, never arbitrary ranges. Each fixture',
                    'The loop count aliases are fixed at `n=0,1,17,32,1024,10000`, producing 18 model rows, never arbitrary ranges.'+addition+' Each fixture',1)
    # Restore heading lost by the prior bounded section rewrite.
    doc=doc.replace('\n\nUse development groups', '\n\n## Verification\n\nUse development groups', 1)
    docp.write_text(doc,encoding='utf-8')
    t['input_output_contract']=first+'\n\n'+rest
    t['implementation_steps'][3]='Add selection and verification contracts that bind the 18 fixture model rows, frozen WAT/Wasm hashes, exact stage list, oracle implementation, executable/source identities, mode, stage, and warm batch. The proof producer archives raw output and command records before oracle checks, independently checks result and modulo arithmetic, validates the 18-row source-event/fuel proof and its 126 source-event/fuel/reference commands, and requires every one of the 180 staged observations.'
    planp.write_text(json.dumps(plan,indent=2,ensure_ascii=False)+'\n',encoding='utf-8')
    for rel in (TASK,PLAN):
        s=SRC/rel; d=DST/rel; d.write_bytes(s.read_bytes()); os.chmod(d,stat.S_IMODE(s.stat().st_mode))
    argv=json.loads((ARCHIVE/'argv.json').read_text())['validator_commands']; runs=[]
    log=ARCHIVE/'validator-runs.jsonl'
    for cmd in argv:
        script=Path(cmd[-1]); cwd=SRC if script.parent==SRC else script.parent
        pre={'command':cmd,'cwd':str(cwd),'validator_source':str(script),'validator_source_sha256':sha(script),'plan_manifest_sha256':sha(script.parent/'TASKS.json')}
        with log.open('a') as f: f.write(json.dumps({'pre_run_repair':pre},sort_keys=True)+'\n'); f.flush(); os.fsync(f.fileno())
        p=subprocess.run(cmd,cwd=cwd,capture_output=True,text=True)
        r={**pre,'exit_code':p.returncode,'stdout':p.stdout,'stderr':p.stderr}; runs.append(r)
        with log.open('a') as f: f.write(json.dumps({'repair_result':r},sort_keys=True)+'\n'); f.flush(); os.fsync(f.fileno())
    m=json.loads(MANIFEST.read_text()); m.setdefault('validator_history',[]).append(m.get('validators',[]))
    m['validators']=[{'command':r['command'],'exit_code':r['exit_code'],'stdout':r['stdout'],'stderr':r['stderr'],'validator_source_sha256':r['validator_source_sha256'],'plan_manifest_sha256':r['plan_manifest_sha256']} for r in runs]
    for rel in (TASK,PLAN):
        s=SRC/rel; d=DST/rel
        entry=next(x for x in m['files'] if x['path']==rel.as_posix())
        entry.update(size_bytes=s.stat().st_size,sha256=sha(s),snapshot_sha256=sha(d),byte_equal=s.read_bytes()==d.read_bytes(),mode_octal=format(stat.S_IMODE(s.stat().st_mode),'04o'))
    m['snapshot_source_byte_equality']=all((SRC/x['path']).is_file() and (SRC/x['path']).read_bytes()==(DST/x['path']).read_bytes() and sha(SRC/x['path'])==x['sha256'] for x in m['files'])
    MANIFEST.write_text(json.dumps(m,indent=2,sort_keys=True)+'\n')
    print(json.dumps({'runs':[{'command':r['command'],'exit_code':r['exit_code'],'stdout':r['stdout'].strip(),'stderr':r['stderr'].strip()} for r in runs],'byte_equality':m['snapshot_source_byte_equality']},indent=2))
    if any(x['exit_code'] for x in runs) or not m['snapshot_source_byte_equality']: raise SystemExit(1)
if __name__=='__main__': main()
