"""Diagnostic-only, exhaustive warm calibration before any P056 paired work."""
from __future__ import annotations

import argparse
from contextlib import contextmanager
import json
import math
import os
from pathlib import Path
import shutil
import sys
import time

ROOT = Path(__file__).resolve().parents[3]
sys.dont_write_bytecode = True
sys.path.insert(0, str(ROOT / 'tools/wtime-bench'))
sys.path.insert(0, str(Path(__file__).resolve().parent))
import baseline
import compare
import focused_compare as focused
import p123_adapter
import selection_manifest
import simd_scaling
import simd_workloads
import workloads
from p046_verify import FAMILIES, P001_BINARY, P001_MANIFEST, _production_sources, read, ref, write_new
from p056_workflow import checked_ref, predecessor, need

SCHEMA = 'wtime-p056-calibration-preflight-v1'
TARGET_NS = 20_000_000
SELECTIONS = {
    'P047': ROOT / 'docs/benchmarks/performance/P056-P047-refresh-selection.json',
    'P050': ROOT / 'docs/benchmarks/performance/P050-selection.json',
}


def warm_groups():
    groups = []
    for family in (*FAMILIES, 'original-mvp'):
        for route in ('p001', 'structural'):
            for entry in workloads.selection(family):
                if 'warm_call' in entry['stages']:
                    groups.append({'route': route, 'selection': family, 'entry': entry})
    for task, path in SELECTIONS.items():
        declaration = selection_manifest.load(path)
        selection_manifest.load_data(declaration)
        need(declaration['task'] == task, 'refresh selection task differs')
        for entry in selection_manifest.rows(declaration):
            if 'warm_call' in entry['stages']:
                groups.append({'route': task, 'selection': task, 'entry': entry})
    keys = [group_key(group) for group in groups]
    need(len(keys) == len(set(keys)), 'duplicate warm calibration group')
    return groups


def group_key(group):
    return '/'.join((group['route'], group['selection'], group['entry']['id'], 'warm_call'))


def calibrate_groups(groups, observer, emit):
    """Collect diagnostic confirmations only, preserving every failed group."""
    expected = [group_key(group) for group in groups]
    results = []
    for group in groups:
        key = group_key(group)
        row = {'group': key, 'workload': group['entry']['id'], 'stage': 'warm_call',
               'capacity': focused.batch_limit(group['entry']), 'target_ns': TARGET_NS,
               'attempts': [], 'status': 'running'}
        emit({'event': 'group_start', 'group': key, 'capacity': row['capacity']})
        def observe(side, batch):
            emit({'event': 'diagnostic_request', 'group': key, 'side': side, 'batch': batch})
            observation = observer(group, side, batch)
            row['attempts'].append({'side': side, 'batch': batch, 'observation': observation})
            emit({'event': 'diagnostic_observation', 'group': key, 'side': side,
                  'batch': batch, 'observation': observation})
            return observation
        try:
            batch, audit = focused.calibrate(group['entry'], observe)
            row.update(status='passed', batch=batch, confirmations=audit)
        except (ValueError, RuntimeError, AssertionError) as exc:
            row.update(status='failed', error=f'{key}: {exc}')
        results.append(row)
        emit({'event': 'group_complete', 'group': key, 'status': row['status'],
              **({'error': row['error']} if 'error' in row else {})})
    return {'schema': SCHEMA, 'task': 'P056', 'lane': 'diagnostic',
            'completion_eligible': False, 'timing_evidence': False,
            'paired_sample_events': 0, 'statistics_computations': 0,
            'expected_groups': expected, 'attempted_groups': [row['group'] for row in results],
            'passed_groups': [row['group'] for row in results if row['status'] == 'passed'],
            'expected_group_count': len(expected), 'attempted_group_count': len(results),
            'passed_group_count': sum(row['status'] == 'passed' for row in results),
            'failure_count': sum(row['status'] != 'passed' for row in results),
            'groups': results}


def validate_result(report, groups):
    expected = [group_key(group) for group in groups]
    need(report.get('schema') == SCHEMA and report.get('lane') == 'diagnostic'
         and report.get('completion_eligible') is False and report.get('timing_evidence') is False
         and report.get('paired_sample_events') == 0 and report.get('statistics_computations') == 0,
         'calibration diagnostics cannot supply paired evidence or completion')
    for field in ('expected_groups', 'attempted_groups', 'passed_groups'):
        need(report.get(field) == expected, 'warm calibration membership differs: ' + field)
    for field in ('expected_group_count', 'attempted_group_count', 'passed_group_count'):
        need(report.get(field) == len(expected), 'warm calibration count differs: ' + field)
    need(report.get('failure_count') == 0 and len(report.get('groups', [])) == len(groups),
         'warm calibration has failed or missing groups')
    for group, row in zip(groups, report['groups']):
        cap = focused.batch_limit(group['entry'])
        need(row.get('group') == group_key(group) and row.get('status') == 'passed'
             and row.get('target_ns') == TARGET_NS and row.get('capacity') == cap,
             'warm calibration identity differs')
        audit = row.get('confirmations', [])
        need(1 <= len(audit) <= 8 and row.get('batch') == audit[-1]['batch'],
             'warm confirmation attempts missing or unbounded')
        for attempt in audit:
            batch = attempt.get('batch')
            need(type(batch) is int and 1 <= batch <= cap, 'calibration exceeds driver capacity')
            need(set(attempt.get('observations', {})) == {'baseline', 'candidate'},
                 'both actual sides must confirm the requested batch')
            for obs in attempt['observations'].values():
                ns = obs.get('ns_per_operation')
                need(type(obs.get('batch')) is int and obs.get('batch') == batch and not isinstance(ns, bool)
                     and isinstance(ns, (int, float)) and math.isfinite(ns) and ns > 0,
                     'invalid confirmation batch or timing')
        need(all(obs['ns_per_operation'] * row['batch'] >= TARGET_NS
                 for obs in audit[-1]['observations'].values()),
             'warm confirmation did not reach 20 ms on both sides')
    return True


@contextmanager
def logged_process(emit):
    original = compare.process
    def process(executable, argv, cpu):
        emit({'event': 'process_request', 'argv': [str(executable), *map(str, argv)],
              'cpu': cpu, 'cwd': str(ROOT)})
        try:
            result = original(executable, argv, cpu)
        except Exception as exc:
            emit({'event': 'process_failure', 'error': str(exc)})
            raise
        emit({'event': 'process_response', 'observation': result})
        return result
    compare.process = process
    try:
        yield
    finally:
        compare.process = original


def boundary_probes(sides, entry, fixture, cpu, output, *, memory_entry=None, memory_fixture=None):
    """Exercise the real Rust endpoint above the former cap and reject overflow."""
    results = {}
    for name, (binary, mode) in sides.items():
        obs = focused.observe(binary, entry, fixture, 'warm_call', 100_001, mode, cpu, output)
        need(obs['batch'] == 100_001, 'real typed driver did not accept 100001 calls: ' + name)
        allowed = sorted(set(simd_scaling.allowed_wire_results(entry, simd_workloads)),
                         key=lambda value: int(value.removeprefix('i32:')))
        args = ['--typed-allowed-case' if len(allowed) > 1 else '--typed-case', fixture,
                entry['id'], simd_scaling.typed_input(entry), '|'.join(allowed), 'warm_call',
                1_000_001, mode, focused.profile(entry)]
        try:
            compare.process(binary, args, cpu)
        except compare.ComparisonError as exc:
            need('typed batch exceeds driver capacity 1000000' in str(exc),
                 'overflow failed for an unexpected reason: ' + name + ': ' + str(exc))
            results[name] = {'accepted_batch': 100_001, 'observation': obs,
                             'overflow_rejected': True, 'overflow_error': str(exc)}
        else:
            raise ValueError('real typed driver accepted an over-cap batch: ' + name)
        if memory_entry is not None:
            need(memory_fixture is not None, 'memory boundary fixture missing')
            memory_allowed = sorted(set(simd_scaling.allowed_wire_results(memory_entry, simd_workloads)),
                                    key=lambda value: int(value.removeprefix('i32:')))
            args = ['--simd-memory-case', memory_fixture, memory_entry['id'],
                    simd_scaling.typed_input(memory_entry), '|'.join(memory_allowed),
                    'warm_call', 100_001, mode, focused.profile(memory_entry)]
            try:
                compare.process(binary, args, cpu)
            except compare.ComparisonError as exc:
                need('typed batch exceeds driver capacity 100000' in str(exc),
                     'memory overflow failed for an unexpected reason: ' + name + ': ' + str(exc))
                results[name]['memory_overflow_rejected'] = True
                results[name]['memory_overflow_error'] = str(exc)
            else:
                raise ValueError('real memory typed driver accepted an over-cap batch: ' + name)
    need(simd_scaling.batch_limit({'memory_expectation': {}}) == 100_000,
         'memory-sensitive typed capacity changed')
    return results


def snapshot():
    sources = focused.sources()
    sources.update(_production_sources())
    for path in (Path(__file__), ROOT / 'tools/verification/producers/p056_workflow.py',
                 ROOT / 'tools/verification/producers/p046_verify.py',
                 ROOT / 'tools/wtime-bench/p123_adapter.py', *SELECTIONS.values()):
        sources[str(path.relative_to(ROOT))] = compare.sha(path)
    return dict(sorted(sources.items()))


def run(args):
    output = args.output.resolve()
    target = (ROOT / 'target/verification').resolve()
    need(output.is_relative_to(target) and not output.exists(), 'diagnostic output must be fresh and contained')
    output.mkdir(parents=True)
    start = time.monotonic()
    write_new(output / 'command.json', {'argv': sys.argv, 'cwd': str(ROOT)})
    before = snapshot()
    for name, digest in before.items():
        if name.endswith('.py'):
            dest = output / 'sources' / name
            dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / name, dest)
            need(compare.sha(dest) == digest, 'helper changed while archiving: ' + name)
    def emit(event):
        with (output / 'journal.jsonl').open('a') as stream:
            stream.write(json.dumps(event, sort_keys=True, allow_nan=False) + '\n')
    emit({'event': 'start', 'paired_sample_events': 0})
    try:
        declaration, archive = predecessor(args.frozen)
        verification = args.verify_dir.resolve(strict=True)
        need(verification.is_relative_to(target), 'verification directory outside shared store')
        baselines_path = verification / 'baselines/report.json'
        build_path = verification / 'production/build-record.json'
        baselines, build = read(baselines_path), read(build_path)
        need(baselines.get('task') == 'P056' and baselines.get('status') == 'passed', 'baseline build is incomplete')
        need(build.get('source_stable') is True and build.get('source_identities') == _production_sources(),
             'production build source identity is stale')
        refs = [ref(args.frozen), ref(baselines_path), ref(build_path), ref(verification / 'receipt.json')]
        binary = Path(build['binary_path']).resolve(strict=True)
        need(compare.sha(binary) == build['binary_sha256'], 'production binary is stale')
        refs.append(ref(binary))
        routes = {}
        adapters = {'p001': baselines['p001_adapter'], 'structural': baselines['structural_adapter'],
                    **baselines['individual_adapters']}
        archives = {'p001': ROOT / 'target/performance/baseline', 'structural': archive,
                    'p047-base': ROOT / 'target/performance/P047-before',
                    'p047-only': ROOT / 'target/performance/P050-before'}
        for name, reference in adapters.items():
            manifest = checked_ref(reference)
            checked = p123_adapter.verify(manifest.parent, archives[name])
            executable = manifest.parent / 'binary/wtime-bench'
            refs.extend((reference, ref(executable), ref(archives[name] / 'manifest.json')))
            routes[name] = (executable, checked['mode'])
        routes['production'] = (binary, 'compact')
        pairs = {'p001': (routes['p001'], routes['production']),
                 'structural': (routes['structural'], routes['production']),
                 'P047': (routes['p047-base'], routes['p047-only']),
                 'P050': (routes['p047-only'], routes['production'])}
        original = ROOT / 'target/performance/baseline'
        baseline.verify_archive(original)
        need(compare.sha(original / 'manifest.json') == P001_MANIFEST
             and compare.sha(original / 'binary/wtime-bench') == P001_BINARY,
             'immutable P001 encoder archive changed')
        encoder = original / 'binary/wtime-conformance'
        refs.append(ref(encoder))
        groups = warm_groups()
        entries = {}
        for group in groups:
            entry = group['entry']
            if entry['id'] in entries:
                old = entries[entry['id']]
                need(old['source'] == entry['source'] and old['oracle_arguments'] == entry['oracle_arguments'],
                     'workload identity differs between selections: ' + entry['id'])
            entries[entry['id']] = entry
        fixtures_dir = output / 'fixtures'
        fixtures_dir.mkdir()
        fixtures = focused.prepare_fixtures(list(entries.values()), archive, fixtures_dir, encoder)
        fixture_refs = {name: ref(path) for name, path in fixtures.items()}
        refs.extend(fixture_refs.values())
        cpu = min(os.sched_getaffinity(0))
        with logged_process(emit):
            capabilities = {}
            for name, (exe, mode) in routes.items():
                cap = compare.process(exe, ['--protocol'], cpu)
                need(cap.get('instrumented') is False and mode in cap.get('modes', [])
                     and cap.get('batch_limits') == {'typed_numeric': 1_000_000, 'typed_memory': 100_000},
                     'driver advertised stale numeric/memory capacities: ' + name)
                capabilities[name] = cap
            typed = entries['simd_typed_f64_neg_1_75']
            memory = entries['simd_lane_memory_n0']
            boundary = boundary_probes(routes, typed, fixtures[typed['id']], cpu, output,
                                       memory_entry=memory, memory_fixture=fixtures[memory['id']])
            def observer(group, side, batch):
                exe, mode = pairs[group['route']][side == 'candidate']
                entry = group['entry']
                need(focused.profile(entry) in next(cap['features'] for name, cap in capabilities.items()
                     if routes[name] == (exe, mode)), 'driver lacks selected workload profile')
                return focused.observe(exe, entry, fixtures[entry['id']], 'warm_call', batch, mode, cpu, output)
            report = calibrate_groups(groups, observer, emit)
        report.update(status='failed' if report['failure_count'] else 'passed',
                      source_stable=before == snapshot(), source_identities=before,
                      immutable_inputs=refs, fixture_files=fixture_refs,
                      capabilities=capabilities, real_typed_boundaries=boundary,
                      selected_cpu=cpu, elapsed_seconds=time.monotonic() - start)
        for reference in refs:
            checked_ref(reference)
        need(report['source_stable'], 'sources changed during calibration diagnostics')
        write_new(output / 'report.json', report)
        validate_result(report, groups)
        emit({'event': 'passed', 'group_count': len(groups), 'paired_sample_events': 0})
        return report
    except Exception as exc:
        emit({'event': 'failed', 'error': str(exc), 'paired_sample_events': 0})
        if not (output / 'report.json').exists():
            write_new(output / 'report.json', {'schema': SCHEMA, 'task': 'P056', 'status': 'failed',
                      'error': str(exc), 'paired_sample_events': 0, 'statistics_computations': 0,
                      'elapsed_seconds': time.monotonic() - start})
        raise


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--verify-dir', type=Path, required=True)
    parser.add_argument('--frozen', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    result = run(parser.parse_args())
    print(json.dumps({'task': 'P056', 'status': result['status'],
                      'warm_groups': result['passed_group_count'], 'paired_sample_events': 0}))


if __name__ == '__main__':
    main()
