"""Revalidate a completed P067 collection after a contract-only guard failure.

No guest, compiler, bootstrap or statistics work is performed. The original
failed phase, its exact inputs and all completed comparison reports survive.
"""
from __future__ import annotations

import argparse
import hashlib
from pathlib import Path
import shutil
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(ROOT / 'tools'), str(ROOT / 'tools/wtime-bench'),
                str(Path(__file__).resolve().parent)]
from verification.artifacts import ArtifactStore, digest, need, normal, read_json, verify_normal, write_new
import p067_workflow as workflow

REPORTS = {'eager-vs-decoded', 'eager-vs-compact', 'adaptive-vs-decoded',
           'p001-scalar', 'p001-memory', 'p001-simd', 'p001-representative',
           'p001-original-mvp', 'entry-totals', 'entry-sequence'}
EXPECTED_FAILURE = ('VerificationError: file changed during verification: '
                    + str(ROOT / 'docs/benchmarks/performance/P067-verification.json'))


def validate_failure(failure):
    need(failure.get('status') == 'failed' and failure.get('error') == EXPECTED_FAILURE,
         'P067 reuse requires the exact contract-only final guard failure')
    commands = failure.get('commands', [])
    need(len(commands) == 1 and commands[0].get('name') == 'measure'
         and type(commands[0].get('exit_code')) is int and commands[0]['exit_code'] == 0,
         'the original complete milestone collector did not succeed')
    argv = commands[0].get('argv', [])
    need(len(argv) >= 4 and argv[1:4] == ['-B', str(ROOT / 'tools/verification/producers/p067_workflow.py'), 'measure'],
         'the failed phase ran a different collector')
    return commands[0]


def retain(completed, freeze_path, verify_dir, output):
    import focused_compare
    completed = completed.resolve(strict=True)
    need(completed.parent == ROOT / 'target/verification/runs/P067/measure',
         'P067 reuse must name an original measurement attempt')
    failure_path = completed / 'failure.json'
    failure = read_json(failure_path)
    command = validate_failure(failure)
    inputs_path = completed / 'inputs.json'
    inputs = read_json(inputs_path)
    need(digest(inputs) == failure.get('input_key'), 'original P067 inputs changed')
    for component in inputs.get('components', {}).values():
        for name, expected in component.get('files', {}).items():
            path = (ROOT / name).resolve(strict=True)
            need(path.is_relative_to(ROOT) and path.is_file()
                 and hashlib.sha256(path.read_bytes()).hexdigest() == expected,
                 'original P067 collection input is stale: ' + name)
    original = completed / 'measure'
    collection_path = original / 'report.json'
    collection = read_json(collection_path)
    before = focused_compare.sources()
    need(collection.get('schema') == 'wtime-p067-measurements-v1'
         and collection.get('task') == 'P067' and collection.get('status') == 'passed'
         and collection.get('timing_evidence') is True and collection.get('source_stable') is True
         and collection.get('source_inputs') == before,
         'P067 collection is incomplete or execution inputs differ')
    need(collection.get('freeze') == normal(freeze_path)
         and collection.get('verify_receipt') == normal(verify_dir / 'receipt.json'),
         'P067 collection used different selections or semantic evidence')
    need(set(collection.get('reports', {})) == REPORTS,
         'P067 collection omits or substitutes a milestone comparison')
    store = ArtifactStore(ROOT / 'target/verification')
    for key in ('stdout', 'stderr'):
        store.verify(command[key])
    need(read_json(completed / 'measure.result.json').get('exit_code') == 0,
         'P067 collector command result failed')
    for name, reference in collection['reports'].items():
        verify_normal(reference)
        need(read_json(original / (name + '.result.json')).get('exit_code') == 0,
             'P067 comparison command failed: ' + name)
    artifacts_path = verify_normal(collection['artifacts'])
    for reference in read_json(artifacts_path).values():
        store.verify(reference)
    need(focused_compare.sources() == before, 'P067 execution inputs changed during reuse')
    output.mkdir(parents=True, exist_ok=False)
    shutil.copyfile(collection_path, output / 'report.json')
    shutil.copyfile(artifacts_path, output / 'artifacts.json')
    write_new(output / 'reuse.json', {
        'schema': 'wtime-p067-measurement-reuse-v1', 'task': 'P067', 'status': 'passed',
        'timing_evidence': True, 'new_samples_collected': 0,
        'original_failure': normal(failure_path), 'original_inputs': normal(inputs_path),
        'original_collection': normal(collection_path), 'reused_collection': normal(output / 'report.json'),
        'source_current': True, 'original_failed_attempt_preserved': True,
        'reason': 'All ten collectors completed. Only the outer contract-file guard failed after an analysis declaration changed; exact raw pairs and all consumed files were revalidated.'})


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--completed', type=Path, required=True)
    parser.add_argument('--freeze', type=Path, required=True)
    parser.add_argument('--verify-dir', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    retain(args.completed, args.freeze, args.verify_dir, args.output)
