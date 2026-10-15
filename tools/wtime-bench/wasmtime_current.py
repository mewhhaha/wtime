#!/usr/bin/env python3
"""Freeze, retain builds, and bind the current informational Wasmtime comparison."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess

import run as runner

ROOT = runner.ROOT
HERE = runner.HERE
TIERS = {'decoded': ('decoded', None), 'compact': ('compact', None),
         'register-eager': ('register', None), 'register-adaptive-3': ('register', 3)}


def need(condition, message):
    if not condition:
        raise ValueError(message)


def read(path):
    return json.loads(Path(path).read_text())


def ref(path):
    path = Path(path).resolve(strict=True)
    return {'path': str(path), 'sha256': runner.sha(path), 'bytes': path.stat().st_size}


def write(path, value):
    runner.write_exclusive(path, json.dumps(value, indent=2) + '\n')


def current(freeze):
    record = read(freeze)
    need(record['source_map'] == runner.source_map(None), 'frozen execution sources changed')
    return record


def freeze(output):
    sources = runner.source_map(None)
    archive = output.parent / 'source'
    archive.mkdir()
    for relative, digest in sources.items():
        destination = archive / relative
        need(destination.resolve().is_relative_to(archive.resolve()), 'source archive escape')
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(ROOT / relative, destination)
        need(runner.sha(destination) == digest, 'archived source differs')
    need(sources == runner.source_map(None), 'execution sources changed during freeze')
    write(output, {'status': 'passed', 'scope': 'informational matched MVP comparison; no tier promotion',
                   'base_commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
                   'source_map': sources, 'archived_source_root': str(archive),
                   'tiers': TIERS, 'pairs_per_group_per_run': 30, 'runs': 2,
                   'run_cpus': [0, 2], 'wasmtime_version': '49.0.2'})


def retain(name, frozen, output):
    current(frozen)
    executable = output.parent / 'bin' / name
    executable.parent.mkdir(exist_ok=True)
    need(not executable.exists(), 'retained executable already exists')
    shutil.copy2(HERE / 'target/release/wtime-bench', executable)
    result = subprocess.run([str(executable), '--protocol'], cwd=ROOT, text=True, capture_output=True, check=True)
    protocol = json.loads(result.stdout)
    expected = {'decoded', 'portable'} | ({name} if name != 'decoded' else set())
    need(set(protocol['modes']) == expected and protocol['instrumented'] is False,
         'compiled tier capabilities differ from requested feature configuration')
    current(frozen)
    write(output, {'status': 'passed', 'name': name, 'binary': ref(executable), 'freeze': ref(frozen),
                   'features': [] if name == 'decoded' else [name + '-executor'],
                   'protocol': protocol, 'source_map': runner.source_map(None)})


def verify(frozen, fixture_path, output):
    current(frozen)
    fixtures = read(fixture_path)
    need(fixtures['passed'] and fixtures['fixture_only'], 'missing fixture-only preflight')
    historical = read(ROOT / 'docs/benchmarks/wasmtime-comparison.json')
    need(set(fixtures['workloads']) == set(historical['workloads']), 'workload coverage changed')
    for name, fixture in fixtures['workloads'].items():
        for key in ('input', 'expected_result', 'wasm_bytes', 'wasm_sha256', 'wat_sha256'):
            need(fixture[key] == historical['workloads'][name][key], 'original workload bytes/oracle changed: ' + name)
    checks = []
    for tier, (mode, threshold) in TIERS.items():
        binding = read(output.parent / (mode + '.json'))
        executable = binding['binary']['path']
        need(ref(executable) == binding['binary'], 'retained binary changed')
        fixture = fixtures['workloads']['call_overhead']
        argv = [executable, fixture['wasm_path'], 'call_overhead', str(fixture['input']),
                str(fixture['expected_result']), '5', '20261015', *runner.tier_arguments(mode, threshold)]
        result = subprocess.run(argv, cwd=ROOT, text=True, capture_output=True, timeout=60, check=True)
        rows = runner.parse_timings(result.stdout, 'call_overhead', 5)
        runner.write_exclusive(output.parent / (tier + '-smoke.csv'), result.stdout)
        checks.append({'tier': tier, 'argv': argv, 'observations': len(rows),
                       'correctness': 'passed; actual comparison CLI validates independent expected result',
                       'purpose': 'verification smoke only; excluded from comparison statistics'})
    current(frozen)
    write(output, {'status': 'passed', 'tier_checks': checks, 'tiers': list(TIERS),
                   'fixture_count': 5, 'historical_fixture_identity': 'all five byte-identical',
                   'timing_evidence': False, 'runtime_source_changes': False})


def index(frozen, verify_dir, measure_dir, output):
    freeze_record = current(frozen)
    reports = []
    for tier, (mode, threshold) in TIERS.items():
        binding_path = verify_dir / (mode + '.json')
        binding = read(binding_path)
        need(ref(binding['binary']['path']) == binding['binary'], 'retained binary changed after measurement')
        seeds = []
        for run in (1, 2):
            path = measure_dir / f'{tier}-run{run}.json'
            report = read(path)
            need(report['passed'] is True and report['source_stable'] is True, 'measurement failed')
            need(report['implementation_sources'] == freeze_record['source_map'], 'measurement sources differ from freeze')
            need(report['benchmark_binary']['sha256'] == binding['binary']['sha256'], 'measurement binary differs from retained build')
            need(report['settings']['wtime_mode'] == mode and report['settings']['register_entry_threshold'] == threshold,
                 'measurement execution policy differs')
            need(report['hardware']['selected_cpu'] == (0 if run == 1 else 2), 'measurement affinity differs')
            need(report['samples'] == 30, 'measurement sample count differs')
            seeds.append(report['seed'])
            reports.append({'tier': tier, 'run': run, **ref(path), 'build': ref(binding_path)})
        need(len(set(seeds)) == 2, 'runs must use independent order seeds')
    write(output, {'status': 'passed', 'schema': 'wtime-current-wasmtime-manifest-v1',
                   'informational_only': True, 'freeze': ref(frozen), 'verification': ref(verify_dir / 'verify.json'),
                   'reports': reports, 'runtime_source_changes': False,
                   'notes': 'Two independently ordered runs per tier; no pooled samples or tier promotion.'})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('operation', choices=('freeze', 'retain', 'verify', 'index'))
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--freeze', type=Path)
    parser.add_argument('--name', choices=('decoded', 'compact', 'register'))
    parser.add_argument('--fixtures', type=Path)
    parser.add_argument('--verify-dir', type=Path)
    parser.add_argument('--measure-dir', type=Path)
    args = parser.parse_args()
    if args.operation == 'freeze':
        freeze(args.output)
    elif args.operation == 'retain':
        retain(args.name, args.freeze, args.output)
    elif args.operation == 'verify':
        verify(args.freeze, args.fixtures, args.output)
    else:
        index(args.freeze, args.verify_dir, args.measure_dir, args.output)


if __name__ == '__main__':
    main()
