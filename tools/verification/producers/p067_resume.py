"""Resume the four missing P067 comparisons after a pre-sampling archive guard.

The six completed comparisons are retained verbatim. The failed attempt stays
failed. Only a metadata-current clone of the identical P001 executable is used
for representative workloads; no Rust sources or timed executable are rebuilt.
"""
from __future__ import annotations

import argparse
import hashlib
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(ROOT / 'tools'), str(ROOT / 'tools/wtime-bench'),
                str(Path(__file__).resolve().parent)]
from verification.artifacts import ArtifactStore, digest, need, normal, read_json, verify_normal, write_new
import p067_workflow as workflow

RETAINED = ('eager-vs-decoded', 'eager-vs-compact', 'adaptive-vs-decoded',
            'p001-scalar', 'p001-memory', 'p001-simd')
FAILED_MESSAGE = 'comparison failed: fresh baseline does not bind identical new measurement harness\n'
REBIND = ROOT / 'target/recovery/P067-representative-rebind-0001/rebind-result.json'
ADAPTER = ROOT / 'target/performance/P067-p001-representative-adapter-0001'


def validate_failure(failure, child_result, child_stderr):
    need(failure.get('status') == 'failed'
         and failure.get('error') == 'VerificationError: command failed: measure',
         'P067 resume requires the preserved original collector failure')
    commands = failure.get('commands', [])
    need(len(commands) == 1 and commands[0].get('name') == 'measure'
         and type(commands[0].get('exit_code')) is int and commands[0]['exit_code'] == 1,
         'unexpected original milestone command result')
    need(type(child_result.get('exit_code')) is int and child_result['exit_code'] == 1
         and child_stderr == FAILED_MESSAGE,
         'only the known pre-sampling representative archive guard can be resumed')
    return commands[0]


def resume(completed, frozen, verify_dir, output):
    import execution_identity
    import focused_compare
    import p123_adapter

    completed = completed.resolve(strict=True)
    need(completed.parent == ROOT / 'target/verification/runs/P067/measure',
         'P067 resume must name a retained measurement attempt')
    original = completed / 'measure'
    failure_path = completed / 'failure.json'
    failure = read_json(failure_path)
    command = validate_failure(failure,
        read_json(original / 'p001-representative.result.json'),
        (original / 'p001-representative.stderr').read_text())
    need(not (original / 'p001-representative').exists(),
         'the failed representative run produced data; it cannot be silently replaced')
    expected_argv = ['-B', str(ROOT / 'tools/verification/producers/p067_workflow.py'),
        'measure', '--freeze', str(frozen), '--verify-dir', str(verify_dir),
        '--output', str(original)]
    need(command.get('argv', [])[1:] == expected_argv,
         'original milestone used different selections or semantic verification')
    inputs_path = completed / 'inputs.json'
    inputs = read_json(inputs_path)
    need(digest(inputs) == failure.get('input_key'), 'original measurement inputs changed')
    for component in inputs.get('components', {}).values():
        for name, expected in component.get('files', {}).items():
            path = (ROOT / name).resolve(strict=True)
            need(path.is_relative_to(ROOT) and path.is_file()
                 and hashlib.sha256(path.read_bytes()).hexdigest() == expected,
                 'original execution input is stale: ' + name)
    expected_results = {name + '.result.json' for name in RETAINED} | {'p001-representative.result.json'}
    need({path.name for path in original.glob('*.result.json')} == expected_results,
         'original comparison set differs from the recorded partial collection')
    store = ArtifactStore(ROOT / 'target/verification')
    for key in ('stdout', 'stderr'):
        store.verify(command[key])
    before = focused_compare.sources()
    reports = {}
    for name in RETAINED:
        result = read_json(original / (name + '.result.json'))
        need(type(result.get('exit_code')) is int and result['exit_code'] == 0,
             'retained comparison did not succeed: ' + name)
        report_path = original / name / 'report.json'
        report = read_json(report_path)
        need(report.get('passed') is True and report.get('source_stable') is True,
             'retained comparison is incomplete: ' + name)
        if name.startswith('p001-'):
            execution_identity.verify_inputs(report)
        else:
            need(report['metadata']['candidate_sources'] == before,
                 'retained focused execution inputs differ')
        reports[name] = normal(report_path)

    rebound = read_json(REBIND)
    need(rebound.get('status') == 'passed', 'baseline metadata refresh did not pass')
    old_adapter = p123_adapter.verify(workflow.BUILDS / 'p001', ROOT / 'target/performance/baseline')
    new_adapter = p123_adapter.verify(ADAPTER, ROOT / 'target/performance/baseline')
    need(new_adapter['binary_sha256'] == old_adapter['binary_sha256']
         and new_adapter['frozen_core_identities'] == old_adapter['frozen_core_identities'],
         'metadata refresh changed the baseline executable or core')
    differences = {name for name in set(old_adapter['shared_harness_identities'])
                   | set(new_adapter['shared_harness_identities'])
                   if old_adapter['shared_harness_identities'].get(name)
                   != new_adapter['shared_harness_identities'].get(name)}
    need(differences == {'tools/wtime-bench/candidate_gate.py'},
         'metadata refresh changed more than the analysis helper identity')
    output.mkdir(parents=True, exist_ok=False)
    candidate = str(workflow.BUILDS / 'register/binary/wtime-bench')
    for family in ('representative', 'original-mvp'):
        name = 'p001-' + family
        baseline = ADAPTER if family == 'representative' else workflow.BUILDS / 'p001'
        workflow.invoke(output, name, [sys.executable, '-B', str(workflow.BENCH / 'compare.py'),
            '--baseline', str(baseline), '--candidate', candidate, '--candidate-mode', 'register',
            '--selection', family, '--pairs', '30', '--seed', '67001', '--output', str(output / name)])
        reports[name] = normal(output / name / 'report.json')
    freeze_record = read_json(frozen)
    for name, tool, flag, rows in [
            ('entry-totals', 'entry_batch_compare.py', '--adapter', 'entry_rows'),
            ('entry-sequence', 'entry_sequence_compare.py', '--baseline', 'sequence_rows')]:
        workflow.invoke(output, name, [sys.executable, '-B', str(workflow.BENCH / tool),
            '--archive', str(workflow.ENTRY_ARCHIVE), flag, str(workflow.BUILDS / 'entry-baseline'),
            '--candidate', str(workflow.BUILDS / 'entry-current'), '--repo', str(ROOT),
            '--harness', str(workflow.BENCH), '--rows', freeze_record[rows]['path'],
            '--pairs', '30', '--seeds', *map(str, workflow.SEEDS), '--output', str(output / name)])
        reports[name] = normal(output / name / 'report.json')
    need(focused_compare.sources() == before, 'execution inputs changed during resumed collection')
    for reference in reports.values():
        verify_normal(reference)
    artifacts = {}
    for directory in (original, output):
        for path in sorted(directory.rglob('*')):
            if path.is_file():
                artifacts[str(path.relative_to(ROOT))] = store.add(path)
    write_new(output / 'artifacts.json', artifacts)
    write_new(output / 'reuse.json', {
        'schema': 'wtime-p067-partial-collection-resume-v1', 'task': 'P067', 'status': 'passed',
        'original_failure': normal(failure_path), 'original_inputs': normal(inputs_path),
        'original_failed_attempt_preserved': True, 'source_current': True,
        'retained_comparisons': list(RETAINED), 'retimed_comparisons': [],
        'new_comparisons': ['p001-representative', 'p001-original-mvp', 'entry-totals', 'entry-sequence'],
        'representative_baseline_rebind': normal(REBIND),
        'reason': 'The representative baseline archive rejected an analysis-only helper identity before sampling. Its metadata-current clone retains the exact same baseline Rust sources and executable. Six complete comparisons are reused verbatim; only four missing comparisons are collected.'})
    write_new(output / 'report.json', {
        'schema': 'wtime-p067-measurements-v1', 'task': 'P067', 'status': 'passed',
        'timing_evidence': True, 'source_stable': True, 'reports': reports,
        'artifacts': normal(output / 'artifacts.json'), 'source_inputs': before,
        'freeze': normal(frozen), 'verify_receipt': normal(verify_dir / 'receipt.json'),
        'partial_collection_resume': normal(output / 'reuse.json')})


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--completed', type=Path, required=True)
    parser.add_argument('--freeze', type=Path, required=True)
    parser.add_argument('--verify-dir', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    resume(args.completed, args.freeze, args.verify_dir, args.output)
