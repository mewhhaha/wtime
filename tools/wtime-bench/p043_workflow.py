#!/usr/bin/env python3
"""P043 semantic bindings and build attestation for the shared phase runner."""
from __future__ import annotations
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
METRICS = ('clean_seconds', 'leaf_seconds', 'runtime_leaf_seconds',
           'release_seconds', 'stripped_bytes', 'clean_peak_rss_kib')

def need(ok, message):
    if not ok:
        raise ValueError(message)

def sha(path):
    h = hashlib.sha256()
    with Path(path).open('rb') as stream:
        for block in iter(lambda: stream.read(1 << 20), b''):
            h.update(block)
    return h.hexdigest()

def ref(path):
    path = Path(path).resolve(strict=True)
    return {'path': str(path), 'sha256': sha(path), 'bytes': path.stat().st_size}

def read(path):
    return json.loads(Path(path).read_text())

def write(path, value):
    with Path(path).open('x') as stream:
        stream.write(json.dumps(value, indent=2, sort_keys=True) + '\n')

def runtime_map():
    return {p.relative_to(ROOT).as_posix(): sha(p)
            for p in sorted((ROOT / 'crates/wtime-runtime/src').rglob('*.rs'))}

def verify_ref(value):
    need(ref(value['path']) == {k: value[k] for k in ('path', 'sha256', 'bytes')}, 'stale input reference')

def freeze(out):
    module_spec = importlib.util.spec_from_file_location('p043_baseline', HERE / 'baseline.py')
    baseline = importlib.util.module_from_spec(module_spec)
    module_spec.loader.exec_module(baseline)
    original = baseline.verify_archive(ROOT / 'target/performance/baseline')
    before = ROOT / 'target/performance/P043-before'
    archive = baseline.verify_archive(before)
    predecessor = {v['source_path']: v['sha256'] for v in archive['files'].values()
                   if v.get('source_path', '').startswith('crates/wtime-runtime/src/')}
    need(archive['mode'] == 'compact' and len(predecessor) == 29, 'wrong immediate predecessor')
    p042 = ROOT / 'target/performance/P042/shared-finalization/attempt-0001/indirect-cache-evidence.json'
    proof = read(p042)
    need(proof['status'] == 'passed' and proof['runtime_source_map'] == predecessor, 'P042 lineage differs')
    receipt_path = ROOT / 'target/performance/P043/fixtures/attempt-0001/receipt.json'
    receipt = read(receipt_path)
    need(receipt['status'] == 'passed' and receipt['source_stable']
         and receipt['runtime_predecessor'] == predecessor, 'fixture preparation differs')
    for row in receipt['fixtures'].values():
        verify_ref(row['source']); verify_ref(row['wasm'])
    for row in receipt['image_oracles'].values():
        verify_ref(row)
        need(row['bytes'] == 65536, 'incomplete fuel image')
    for row in receipt['tools'].values():
        verify_ref(row)
    write(out, {'status': 'passed', 'runtime_source_map': predecessor,
                'before_manifest': ref(before / 'manifest.json'),
                'p042_evidence': ref(p042), 'fixture_receipt': ref(receipt_path),
                'original_archive_files': len(original['files']),
                'fixture_count': len(receipt['fixtures']),
                'image_count': len(receipt['image_oracles'])})

def build(directory):
    directory.mkdir(parents=True, exist_ok=False)
    shutil.copy2(__file__, directory / 'helper.executed.py')
    before = runtime_map()
    binary = ROOT / 'tools/wtime-bench/target/release/wtime-bench'
    binary.unlink(missing_ok=True)
    command = ['cargo', 'build', '--locked', '--release', '--manifest-path',
               'tools/wtime-bench/Cargo.toml', '--no-default-features',
               '--features', 'compact-executor', '-j', '4']
    write(directory / 'command.json', {'argv': command, 'runtime_sources': before})
    with (directory / 'stdout.log').open('xb') as stdout, (directory / 'stderr.log').open('xb') as stderr:
        process = subprocess.run(command, cwd=ROOT, stdout=stdout, stderr=stderr)
    record = {'schema': 'wtime-production-command-v1', 'phase': 'build',
              'command': command, 'cwd': str(ROOT), 'exit_code': process.returncode,
              'source_stable': before == runtime_map(), 'runtime_source_map': before,
              'helper_sha256': sha(directory / 'helper.executed.py'),
              'stdout_sha256': sha(directory / 'stdout.log'),
              'stderr_sha256': sha(directory / 'stderr.log')}
    if process.returncode == 0:
        retained = directory / 'wtime-bench'
        shutil.copy2(binary, retained)
        protocol_command = [str(retained), '--protocol']
        protocol = subprocess.run(protocol_command, cwd=ROOT, capture_output=True)
        (directory / 'protocol.stdout.json').write_bytes(protocol.stdout)
        (directory / 'protocol.stderr.log').write_bytes(protocol.stderr)
        record.update(binary_path=str(retained), binary_sha256=sha(retained),
                      protocol_command=protocol_command, protocol_exit=protocol.returncode)
    write(directory / 'build-record.json', record)
    need(process.returncode == 0 and record['source_stable'], 'production build failed or drifted')
    need(record['protocol_exit'] == 0, 'production protocol failed')
    protocol = read(directory / 'protocol.stdout.json')
    need(protocol['instrumented'] is False and 'compact' in protocol['modes'], 'wrong measured product')

def reuse_budgets(source, directory):
    """Reuse independently successful build commands from a failed sibling check."""
    sys.path.insert(0, str(ROOT / 'tools'))
    sys.path.insert(0, str(ROOT / 'tools/checks'))
    import build_budget
    from verification.artifacts import ArtifactStore
    from verification.runner import execution_environment
    directory = directory.resolve()
    inputs = read(source / 'inputs.json')
    need(inputs['lane'] == 'full', 'build reports must come from the full lane')
    need(inputs['tools'][str(ROOT / 'tools/checks/build_budget.py')] == sha(ROOT / 'tools/checks/build_budget.py'),
         'build producer changed since execution')
    dependency_helper = 'tools/checks/dependency_budget.py'
    need(inputs['components']['build_tools']['files'][dependency_helper] == sha(ROOT / dependency_helper),
         'dependency policy changed since build execution')
    need(execution_environment(ROOT, inputs['definition']) == inputs['environment'],
         'build compiler, configuration or environment changed')
    metadata = build_budget.cargo_metadata()
    current = build_budget.source_inputs(metadata)
    store = ArtifactStore(ROOT / 'target/verification')
    records = {}
    for label, filename, features in [('default-build', 'build.json', []),
                                      ('optional-build', 'optional-build.json', ['wtime-runtime/compact-executor'])]:
        command = read(source / (label + '.command.json'))
        result_path = source / (label + '.result.json')
        result = read(result_path)
        expected = [sys.executable, '-B', str(ROOT / 'tools/checks/build_budget.py')]
        if features:
            expected += ['--features', features[0]]
        expected += ['--output', str(source / filename)]
        need(command['argv'] == expected and command['cwd'] == str(ROOT)
             and result['argv'] == expected and result['cwd'] == str(ROOT)
             and result['exit_code'] == 0, 'missing independently successful build command')
        store.verify(result['stdout']); store.verify(result['stderr'])
        report_path = source / filename
        report = read(report_path)
        need(report['source_inputs'] == report['source_inputs_after'] == current
             and report['source_sha256'] == report['source_sha256_after'] == build_budget.hash_inputs(current)
             and report['source_stable'] and report['budget_pass']
             and report['features'] == features and report['target_initially_empty'],
             'build report is stale, failed or has wrong settings')
        destination = directory / filename
        need(not destination.exists(), 'refusing to replace imported build report')
        shutil.copy2(report_path, destination)
        records[label] = {'report': ref(report_path), 'command': ref(source / (label + '.command.json')),
                          'result': ref(result_path), 'stdout': result['stdout'], 'stderr': result['stderr']}
    need(build_budget.source_inputs(metadata) == current, 'source drift during build report validation')
    write(directory / 'build-reuse.json', {'status': 'passed', 'commands_reexecuted': 0,
                                         'source_stable': True, 'records': records,
                                         'original_inputs': ref(source / 'inputs.json'),
                                         'original_verification_phase_status': 'failed',
                                         'scope': 'Only the two independently successful source-bound build commands; the failed diagnostics are not reused.'})

def reuse_coverage(source, directory):
    """Revalidate passed diagnostics independently of the paired timing driver."""
    sys.path.insert(0, str(ROOT / 'tools'))
    from verification.artifacts import ArtifactStore
    from verification.contracts import identity
    from verification.runner import execution_environment
    receipt = read(source / 'receipt.json')
    need(receipt['status'] == 'passed' and receipt['lane'] == 'full'
         and receipt['completion_eligible'] and not receipt['synthetic'],
         'diagnostics require a successful full-lane receipt')
    inputs = receipt['inputs']
    for name in ('runtime', 'compilation', 'fixtures', 'diagnostics', 'promotion'):
        need(identity(ROOT, inputs['components'][name]['declaration']) == inputs['components'][name],
             'diagnostic input changed: ' + name)
    need(execution_environment(ROOT, inputs['definition']) == inputs['environment'],
         'diagnostic compiler or execution environment changed')
    store = ArtifactStore(ROOT / 'target/verification')
    coverage = store.json(receipt['artifacts']['coverage'])
    need(ref(source / 'coverage/coverage.json') == receipt['original_outputs']['coverage'],
         'original diagnostic report changed')
    need(coverage['status'] == 'passed' and coverage['source_stable']
         and coverage['source_map_before'] == coverage['source_map_after']
         and coverage['selected_count'] == 35 and len(coverage['rows']) == 35
         and coverage['primary_count'] == 2 and coverage['protected_count'] == 33,
         'incomplete diagnostic coverage')
    for path, expected in coverage['source_map_after'].items():
        resolved = (ROOT / path).resolve(strict=True)
        need(resolved.is_relative_to(ROOT) and sha(resolved) == expected,
             'diagnostic producer or model changed: ' + path)
    def validate_refs(value):
        if isinstance(value, dict):
            if {'path', 'sha256', 'bytes'} <= value.keys():
                verify_ref(value)
            for child in value.values(): validate_refs(child)
        elif isinstance(value, list):
            for child in value: validate_refs(child)
    validate_refs(coverage)
    destination = directory / 'coverage'
    destination.mkdir(exist_ok=False)
    shutil.copy2(source / 'coverage/coverage.json', destination / 'coverage.json')
    write(directory / 'coverage-reuse.json', {'status': 'passed', 'commands_reexecuted': 0,
                'original_receipt': ref(source / 'receipt.json'),
                'original_coverage': receipt['artifacts']['coverage'],
                'scope': 'All diagnostic build inputs, producers, fixture images, command logs and binaries revalidated; the changed timing driver is independent.'})

def decide(statistics, verify_dir, out):
    record = read(statistics)
    need(record['status'] == 'passed' and record['statistics_computations'] == 1, 'missing single gate computation')
    decision = record['decision']['decision']
    coverage = read(verify_dir / 'coverage/coverage.json')
    need(coverage['status'] == 'passed' and coverage['source_stable'], 'failed memory diagnostics')
    ratios = {}
    for name in ('build', 'optional-build'):
        candidate = read(verify_dir / (name + '.json'))
        predecessor = read(ROOT / f'target/performance/P043/predeclaration/attempt-0001/predecessor-{name}.json')
        need(candidate['budget_pass'] and candidate['source_stable'], 'failed hard build budget')
        ratios[name] = {metric: candidate[metric] / predecessor[metric] for metric in METRICS}
    growth_ok = all(ratio <= 1.25 for mode in ratios.values() for ratio in mode.values())
    reasons = list(record['decision']['reasons'])
    if not growth_ok and decision == 'adopted':
        decision = 'inconclusive'
    if not growth_ok:
        reasons.append('Predeclared immediate-predecessor build growth exceeds 1.25; promotion withheld.')
    write(out, {'status': 'passed', 'decision': decision, 'statistics': ref(statistics),
                'coverage': ref(verify_dir / 'coverage/coverage.json'),
                'immediate_predecessor_build_ratios': ratios, 'build_growth_pass': growth_ok,
                'reasons': reasons, 'default_policy': 'decoded', 'compact_opt_in': True})

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=['freeze', 'build', 'decide', 'reuse-budgets', 'reuse-coverage'])
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--statistics', type=Path)
    parser.add_argument('--verify-dir', type=Path)
    parser.add_argument('--source', type=Path)
    parser.add_argument('--execute', action='store_true')
    args = parser.parse_args()
    if not args.execute:
        print('inert: pass --execute'); return 0
    try:
        if args.action == 'freeze': freeze(args.output)
        elif args.action == 'build': build(args.output)
        elif args.action == 'reuse-budgets': reuse_budgets(args.source.resolve(strict=True), args.output)
        elif args.action == 'reuse-coverage': reuse_coverage(args.source.resolve(strict=True), args.output)
        else: decide(args.statistics, args.verify_dir, args.output)
        print('passed: ' + args.action)
        return 0
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print(str(error), file=sys.stderr)
        return 2

if __name__ == '__main__':
    raise SystemExit(main())
