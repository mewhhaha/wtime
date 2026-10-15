"""Reuse completed P080 pairs after a contract-only final receipt failure.

The original failed phase is kept intact. Its collector command succeeded and
finished both immutable reports. This reader checks that exact failure, every
original consumed file, both reports and their artifact inventory. It performs
no guest execution, timing or statistics.
"""
from __future__ import annotations

import argparse
import hashlib
from pathlib import Path
import shutil
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(ROOT / 'tools'), str(Path(__file__).resolve().parent)]
from verification.artifacts import ArtifactStore, need, normal, read_json, verify_normal, write_new
import p080_workflow as workflow

EXPECTED_FAILURE = ('VerificationError: file changed during verification: '
                    + str(ROOT / 'docs/benchmarks/performance/P080-verification.json'))


def validate_failure(failure):
    need(failure.get('status') == 'failed' and failure.get('error') == EXPECTED_FAILURE,
         'reuse is restricted to the observed contract-file guard failure')
    commands = failure.get('commands', [])
    need(len(commands) == 1 and commands[0].get('name') == 'specialist-comparisons'
         and type(commands[0].get('exit_code')) is int and commands[0]['exit_code'] == 0,
         'the original specialist collector did not complete successfully')
    argv = commands[0].get('argv', [])
    need(len(argv) >= 4 and argv[1:4] == ['-B', str(ROOT / 'tools/verification/producers/p080_workflow.py'), 'measure'],
         'the original phase did not run the declared collector')
    return commands[0]


def retain(completed, freeze_path, verify_dir, output):
    completed = completed.resolve(strict=True)
    need(completed.parent == ROOT / 'target/verification/runs/P080/measure',
         'completed collection must be an original P080 measure attempt')
    failure_path = completed / 'failure.json'
    failure = read_json(failure_path)
    command = validate_failure(failure)
    inputs_path = completed / 'inputs.json'
    inputs = read_json(inputs_path)
    from verification.artifacts import digest
    need(digest(inputs) == failure.get('input_key'), 'failed phase input record changed')
    for component in inputs.get('components', {}).values():
        for name, expected in component.get('files', {}).items():
            source = (ROOT / name).resolve(strict=True)
            need(source.is_relative_to(ROOT) and source.is_file()
                 and workflow.sha(source) == expected,
                 'original collection input is stale: ' + name)
    freeze = read_json(freeze_path)
    private_sources = workflow._private_focused_sources(workflow.PRIVATE_ROOT)
    need(freeze.get('candidate_sources') == private_sources
         and inputs['components']['runtime_sources']['files'] == workflow._core_sources(ROOT),
         'runtime or private collector source differs from the original collection')
    summary_path = completed / 'comparisons/summary.json'
    summary = read_json(summary_path)
    need(summary.get('schema') == 'wtime-p080-focused-comparisons-v1'
         and summary.get('status') == 'passed' and summary.get('source_stable') is True
         and summary.get('timing_evidence') is True
         and summary.get('candidate_sources') == private_sources,
         'original specialist collection is incomplete')
    need(summary.get('freeze') == normal(freeze_path)
         and summary.get('semantic_receipt') == normal(verify_dir / 'receipt.json'),
         'collection selection or semantic receipt differs from current dependencies')
    need(set(summary.get('comparisons', {})) == {'threads', 'continuations'},
         'completed specialist comparison set differs')
    store = ArtifactStore(ROOT / 'target/verification')
    for name in ('stdout', 'stderr'):
        store.verify(command[name])
    need(read_json(completed / 'specialist-comparisons.result.json').get('exit_code') == 0,
         'original collector command result was unsuccessful')
    need(store.read(command['stdout']).decode().strip()
         == '{"schema": "wtime-p080-focused-comparisons-v1", "status": "passed"}',
         'original collector stdout does not report successful completion')
    for family, seeds, declaration, frozen in (
        ('threads', workflow.THREAD_SEEDS, workflow.THREAD_MANIFEST, freeze['thread_freeze']),
        ('continuations', workflow.CONTINUATION_SEEDS, workflow.CONTINUATION_MANIFEST, freeze['continuation_freeze']),
    ):
        retained = summary['comparisons'][family]
        checked = workflow._validate_comparison(
            verify_normal(retained['report']), family, seeds, private_sources,
            declaration, verify_normal(frozen), summary['candidate_binary'],
            summary['semantic_receipt'])
        need(checked == retained, 'completed comparison validation changed: ' + family)
    artifacts_path = verify_normal(summary['artifacts'])
    for reference in read_json(artifacts_path).values():
        store.verify(reference)
    output.mkdir(parents=True, exist_ok=False)
    shutil.copyfile(summary_path, output / 'summary.json')
    shutil.copyfile(artifacts_path, output / 'artifacts.json')
    write_new(output / 'reuse.json', {
        'schema': 'wtime-p080-measurement-reuse-v1', 'task': 'P080', 'status': 'passed',
        'timing_evidence': True, 'new_samples_collected': 0,
        'original_failure': normal(failure_path), 'original_inputs': normal(inputs_path),
        'original_collection': normal(summary_path), 'reused_collection': normal(output / 'summary.json'),
        'reason': 'The collector passed; changing the contract during execution failed its final file guard. All consumed files and raw observations were independently revalidated.',
        'source_current': True, 'original_failed_attempt_preserved': True})


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--completed', type=Path, required=True)
    parser.add_argument('--freeze', type=Path, required=True)
    parser.add_argument('--verify-dir', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    retain(args.completed, args.freeze, args.verify_dir, args.output)
