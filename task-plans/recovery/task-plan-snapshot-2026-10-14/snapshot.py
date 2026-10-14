#!/usr/bin/env python3
"""Copy the ordinary task plan files byte-for-byte and record provenance."""
import collections
import hashlib
import json
import os
import stat
import subprocess
import sys
from pathlib import Path

SRC = Path('/workspace/tasks')
DST = Path('/workspace/wtime/task-plans')
ARCHIVE = DST / 'recovery/task-plan-snapshot-2026-10-14'
LOG = ARCHIVE / 'validator-runs.jsonl'
MANIFEST = Path('/workspace/wtime/docs/recovery/task-plan-snapshot.json')
EXCLUDED_DIRS = {'__pycache__', '.pytest_cache', '.mypy_cache', '.ruff_cache'}
EXCLUDED_SUFFIXES = {'.pyc', '.pyo'}

def sha(path):
    h = hashlib.sha256()
    with path.open('rb') as f:
        for block in iter(lambda: f.read(1024 * 1024), b''):
            h.update(block)
    return h.hexdigest()

def source_entries():
    files, excluded = [], []
    for root, dirs, names in os.walk(SRC, followlinks=False):
        rootp = Path(root)
        for name in list(dirs):
            path = rootp / name
            if path.is_symlink():
                raise RuntimeError(f'source symlink rejected: {path}')
            if name in EXCLUDED_DIRS:
                excluded.append(path.relative_to(SRC).as_posix() + '/')
                dirs.remove(name)
            elif not stat.S_ISDIR(path.lstat().st_mode):
                raise RuntimeError(f'unknown source entry rejected: {path}')
        for name in names:
            path = rootp / name
            mode = path.lstat().st_mode
            rel = path.relative_to(SRC).as_posix()
            if stat.S_ISLNK(mode):
                raise RuntimeError(f'source symlink rejected: {path}')
            if name.endswith(tuple(EXCLUDED_SUFFIXES)):
                excluded.append(rel)
                continue
            if not stat.S_ISREG(mode):
                raise RuntimeError(f'unknown source entry rejected: {path}')
            files.append((rel, path))
    return sorted(files), sorted(excluded)

def main():
    argv = json.loads((ARCHIVE / 'argv.json').read_text())
    files, excluded = source_entries()
    if DST.exists():
        expected = {rel for rel, _ in files}
        allowed_extra = {ARCHIVE.relative_to(DST).as_posix()}
        for existing in DST.rglob('*'):
            rel = existing.relative_to(DST).as_posix()
            if existing.is_symlink():
                raise RuntimeError(f'destination symlink rejected: {existing}')
            if existing.is_file() and rel not in expected and not any(rel.startswith(x + '/') for x in allowed_extra):
                raise RuntimeError(f'destination-existing discrepancy rejected: {rel}')
            if existing.is_file() and rel in expected:
                source = dict(files)[rel]
                if existing.read_bytes() != source.read_bytes():
                    raise RuntimeError(f'destination-existing content discrepancy rejected: {rel}')
    copied = []
    for rel, source in files:
        dest = DST / rel
        dest.parent.mkdir(parents=True, exist_ok=True)
        if dest.is_symlink() or (dest.exists() and not dest.is_file()):
            raise RuntimeError(f'destination-existing discrepancy rejected: {dest}')
        data = source.read_bytes()
        if dest.exists():
            if dest.read_bytes() != data:
                raise RuntimeError(f'destination-existing content discrepancy rejected: {dest}')
        else:
            dest.write_bytes(data)
        os.chmod(dest, stat.S_IMODE(source.stat().st_mode))
        copied.append(rel)

    # Run validators from the original source paths. Log provenance durably before each run.
    runs = []
    for command in argv['validator_commands']:
        source = Path(command[-1])
        record = {
            'command': command,
            'cwd': str(SRC if source.parent == SRC else source.parent),
            'validator_source': str(source),
            'validator_source_sha256': sha(source),
            'plan_manifest_sha256': sha(source.parent / 'TASKS.json'),
            'validator_source_current_sha256': sha(source),
        }
        with LOG.open('a', encoding='utf-8') as log:
            log.write(json.dumps(record, sort_keys=True) + '\n')
            log.flush()
            os.fsync(log.fileno())
        proc = subprocess.run(command, cwd=record['cwd'], capture_output=True, text=True)
        result = {**record, 'exit_code': proc.returncode, 'stdout': proc.stdout, 'stderr': proc.stderr}
        runs.append(result)
        with LOG.open('a', encoding='utf-8') as log:
            log.write(json.dumps({'result_for': command, 'exit_code': proc.returncode, 'stdout': proc.stdout, 'stderr': proc.stderr}, sort_keys=True) + '\n')
            log.flush()
            os.fsync(log.fileno())

    plan_counts = {}
    for label, path in [('original', SRC / 'TASKS.json'), ('performance', SRC / 'performance/TASKS.json')]:
        tasks = json.loads(path.read_text())['tasks']
        plan_counts[label] = {'task_total': len(tasks), 'status_counts': dict(sorted(collections.Counter(t.get('status', '<missing>') for t in tasks).items()))}

    records, equality = [], True
    for rel, source in files:
        dest = DST / rel
        same = source.read_bytes() == dest.read_bytes()
        equality = equality and same
        records.append({'path': rel, 'size_bytes': source.stat().st_size, 'sha256': sha(source), 'snapshot_sha256': sha(dest), 'byte_equal': same, 'mode_octal': format(stat.S_IMODE(source.stat().st_mode), '04o')})
    manifest = {
        'schema_version': 1,
        'source_root': str(SRC),
        'snapshot_root': str(DST),
        'task_status_counts': plan_counts,
        'file_count': len(records),
        'files': records,
        'excluded_transient_paths': excluded,
        'snapshot_source_byte_equality': equality,
        'validators': [{'command': x['command'], 'exit_code': x['exit_code'], 'stdout': x['stdout'], 'stderr': x['stderr'], 'validator_source_sha256': x['validator_source_sha256'], 'plan_manifest_sha256': x['plan_manifest_sha256']} for x in runs],
        'relocation_note': 'Task documents retain their original relative links and plan semantics; this snapshot is not a validated replacement at task-plans. Restore to ../tasks before relying on links or running validators there.',
    }
    MANIFEST.parent.mkdir(parents=True, exist_ok=True)
    if MANIFEST.exists():
        raise RuntimeError(f'manifest destination exists: {MANIFEST}')
    MANIFEST.write_text(json.dumps(manifest, indent=2, sort_keys=True) + '\n', encoding='utf-8')
    print(json.dumps({'file_count': len(records), 'excluded': excluded, 'byte_equal': equality, 'status_counts': plan_counts, 'validators': [{'command': r['command'], 'exit_code': r['exit_code'], 'stdout': r['stdout'].strip(), 'stderr': r['stderr'].strip()} for r in runs], 'manifest': str(MANIFEST)}, indent=2))
    if not equality or any(r['exit_code'] for r in runs):
        return 1
    return 0

if __name__ == '__main__':
    raise SystemExit(main())
