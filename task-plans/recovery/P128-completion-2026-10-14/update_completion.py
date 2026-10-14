#!/usr/bin/env python3
"""Add the P128 completion record, mirror publisher plan files, validate, and refresh snapshot hashes."""
import collections
import hashlib
import json
import os
import stat
import subprocess
from pathlib import Path

ROOT = Path('/workspace/wtime')
SRC = Path('/workspace/tasks')
DST = ROOT / 'task-plans'
ARCHIVE = DST / 'recovery/P128-completion-2026-10-14'
TASK = Path('performance/04-specialization-and-fusion/P128-minimal-scalar-workloads.md')
PLAN = Path('performance/TASKS.json')
INDEX = Path('performance/INDEX.md')
README = Path('performance/README.md')
MANIFEST = ROOT / 'docs/recovery/task-plan-snapshot.json'
EVIDENCE = 'abe68e19068eb70dfbc771c9e063e270ebf8743b514313500aecf69e4d5ddedf'
REGISTRY = '8917f14d06b2dd69d1abb1885d8186cdc11191924aef8ee073e356cec5cfd668'

def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def main():
    # Confirm the pre-edit inputs were archived before changing any plan files.
    checks = [
        (SRC / PLAN, ARCHIVE / 'pre-edit/source/performance/TASKS.json'),
        (SRC / INDEX, ARCHIVE / 'pre-edit/source/performance/INDEX.md'),
        (SRC / README, ARCHIVE / 'pre-edit/source/performance/README.md'),
        (SRC / TASK, ARCHIVE / 'pre-edit/source/performance/04-specialization-and-fusion/P128-minimal-scalar-workloads.md'),
        (DST / PLAN, ARCHIVE / 'pre-edit/snapshot/performance/TASKS.json'),
        (DST / INDEX, ARCHIVE / 'pre-edit/snapshot/performance/INDEX.md'),
        (DST / README, ARCHIVE / 'pre-edit/snapshot/performance/README.md'),
        (DST / TASK, ARCHIVE / 'pre-edit/snapshot/performance/04-specialization-and-fusion/P128-minimal-scalar-workloads.md'),
        (MANIFEST, ARCHIVE / 'pre-edit/source/docs/recovery/task-plan-snapshot.json'),
    ]
    for live, saved in checks:
        if not live.is_file() or not saved.is_file() or live.read_bytes() != saved.read_bytes():
            raise RuntimeError(f'pre-edit archive mismatch: {live}')

    # Keep the publisher's status/evidence as the single authoritative task entry.
    plan = json.loads((SRC / PLAN).read_text(encoding='utf-8'))
    task = next(x for x in plan['tasks'] if x['id'] == 'P128')
    if task.get('status') != 'done' or task.get('outcome') != 'contract_only':
        raise RuntimeError('published P128 done/contract_only status is missing')
    if task.get('evidence') != [{
        'path': '/workspace/wtime/target/performance/P128/shared-finalization/attempt-0002/evidence.json',
        'sha256': EVIDENCE, 'bytes': 1518,
    }]:
        raise RuntimeError('P128 evidence entry differs from the reviewed publication')
    if task.get('fixture_cases') is None or len(task['fixture_cases']) != 21:
        raise RuntimeError('published P128 21-case metadata is missing')
    if any(x.get('id') == 'P050' and x.get('status') != 'done' for x in plan['tasks']):
        raise RuntimeError('P050 is no longer done')

    p = SRC / TASK
    doc = p.read_text(encoding='utf-8')
    start = doc.index('## Completion evidence\n')
    end = doc.index('\n## Executed evidence', start)
    completion = f'''## Completion evidence

P128 completed as `done` / `contract_only` in verification attempt 0002. The reviewed completion record is [`completion.json`](/workspace/wtime/target/performance/P128/shared-finalization/attempt-0002/completion.json); its evidence file is [`evidence.json`](/workspace/wtime/target/performance/P128/shared-finalization/attempt-0002/evidence.json), SHA256 `{EVIDENCE}` (1,518 bytes). No candidate was promoted, and the result makes no speed claim.

The final verification executed all 21 named checks: nine model/fixture tests, five proof-mock tests, and seven execution-identity tests, with zero failures or ignored checks. Attempt 0001 stopped during preflight after 14 checks and before building because `tools/wtime-bench/p128_scalar_scaling.py` lacked an execution-identity classification. After the model/runner classification and seven identity checks were registered, attempt 0002 completed the verification lane.

The staged proof recorded 180 actual observation commands: 90 decoded and 90 compact, with 36 observations at each of the five stages. Eight separate preparation/protocol commands bring the total to 188 commands. The MVP profile used source-current, uninstrumented runtime-only binaries; Decoded had `runtime_features=[]` and Compact had `runtime_features=["compact-executor"]`. Warm calls used batch size three.

The independent source/fuel proof covered 18 rows and 126 diagnostic commands. Exact dynamic source events, including the implicit function-end instruction, follow `9 + 14*n`; exact fuel boundaries and execution-reference results agreed. The earlier `11 + 14*n` draft model remains in the archived historical evidence and was not used as the accepted formula. The proof inherited seven current P047 cases from the full 354-case P050 verification lane at [`receipt.json`](/workspace/wtime/target/verification/runs/P050/verify/attempt-0002/receipt.json).

The shared registry retained all 459 rows and all 12 selection memberships. Its SHA256 remained `{REGISTRY}`. P050 stayed done with its existing evidence entry.'''
    p.write_text(doc[:start] + completion + doc[end:], encoding='utf-8')

    # Mirror publisher-updated shared files and the P128 note into task-plans.
    for rel in (TASK, PLAN, INDEX, README):
        source, target = SRC / rel, DST / rel
        if not source.is_file():
            raise RuntimeError(f'missing source plan file: {source}')
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(source.read_bytes())
        os.chmod(target, stat.S_IMODE(source.stat().st_mode))

    # Run source validators, recording command and current validator/plan SHAs before execution.
    commands = json.loads((ARCHIVE / 'argv.json').read_text(encoding='utf-8'))['validator_commands']
    log = ARCHIVE / 'validator-runs.jsonl'
    runs = []
    for command in commands:
        source = Path(command[-1])
        cwd = SRC if source.parent == SRC else source.parent
        record = {
            'command': command, 'cwd': str(cwd), 'validator_source': str(source),
            'validator_source_sha256': sha(source), 'plan_manifest_sha256': sha(source.parent / 'TASKS.json'),
        }
        with log.open('a', encoding='utf-8') as f:
            f.write(json.dumps({'pre_run': record}, sort_keys=True) + '\n'); f.flush(); os.fsync(f.fileno())
        proc = subprocess.run(command, cwd=cwd, capture_output=True, text=True)
        result = {**record, 'exit_code': proc.returncode, 'stdout': proc.stdout, 'stderr': proc.stderr}
        runs.append(result)
        with log.open('a', encoding='utf-8') as f:
            f.write(json.dumps({'result': result}, sort_keys=True) + '\n'); f.flush(); os.fsync(f.fileno())

    manifest = json.loads(MANIFEST.read_text(encoding='utf-8'))
    manifest.setdefault('validator_history', []).append(manifest.get('validators', []))
    manifest['validators'] = [{
        'command': r['command'], 'exit_code': r['exit_code'], 'stdout': r['stdout'], 'stderr': r['stderr'],
        'validator_source_sha256': r['validator_source_sha256'], 'plan_manifest_sha256': r['plan_manifest_sha256'],
    } for r in runs]
    status_counts = {}
    for label, path in [('original', SRC / 'TASKS.json'), ('performance', SRC / PLAN)]:
        entries = json.loads(path.read_text(encoding='utf-8'))['tasks']
        status_counts[label] = {
            'task_total': len(entries),
            'status_counts': dict(sorted(collections.Counter(x.get('status', '<missing>') for x in entries).items())),
        }
    manifest['task_status_counts'] = status_counts
    for rel in (TASK, PLAN, INDEX, README):
        source, target = SRC / rel, DST / rel
        if source.read_bytes() != target.read_bytes():
            raise RuntimeError(f'snapshot/source mismatch for {rel}')
        mode = stat.S_IMODE(source.stat().st_mode)
        item = next((x for x in manifest['files'] if x['path'] == rel.as_posix()), None)
        if item is None:
            raise RuntimeError(f'missing snapshot manifest row: {rel}')
        item.update(size_bytes=source.stat().st_size, sha256=sha(source), snapshot_sha256=sha(target),
                    byte_equal=True, mode_octal=format(mode, '04o'))
    manifest['snapshot_source_byte_equality'] = all(
        (SRC / item['path']).is_file()
        and (SRC / item['path']).read_bytes() == (DST / item['path']).read_bytes()
        and sha(SRC / item['path']) == item['sha256']
        for item in manifest['files']
    )
    MANIFEST.write_text(json.dumps(manifest, indent=2, sort_keys=True) + '\n', encoding='utf-8')
    print(json.dumps({
        'validators': [{'command': r['command'], 'exit_code': r['exit_code'],
                        'stdout': r['stdout'].strip(), 'stderr': r['stderr'].strip()} for r in runs],
        'task_status_counts': status_counts, 'snapshot_source_byte_equality': manifest['snapshot_source_byte_equality'],
        'P128_status': task['status'], 'P128_outcome': task['outcome'],
    }, indent=2))
    if any(r['exit_code'] for r in runs) or not manifest['snapshot_source_byte_equality']:
        raise SystemExit(1)

if __name__ == '__main__':
    main()
