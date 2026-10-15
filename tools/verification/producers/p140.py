"""Freeze and check adaptive Register stage settings; smoke is non-promoting."""
from pathlib import Path
import argparse
import json
import os
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[3]
BENCH = ROOT / 'tools/wtime-bench'
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(BENCH))
from tools.verification.producers import p139
from tools.verification import production_attestation
import baseline
import candidate_gate
import compare
import focused_compare
import p140_entry_policy as policy
import selection_manifest

ARCHIVE = ROOT / 'target/performance/P140-before'
ARCHIVE_SHA = '2b5adfc824b1e1ac3dbab65e3b1cef369e2267d2116b9cb803db7bc1242f7010'
COMMIT = '95b5e89756bfc9612aff3057aaa494c74cc67c30'
SELECTION = ROOT / 'docs/benchmarks/performance/P140-selection.json'
HELPERS = ['tools/verification/producers/p140.py', 'tools/verification/producers/p140_driver_tests.py',
           'tools/verification/producers/p139.py', 'tools/verification/producers/p138.py',
           'tools/verification/producers/runtime_production.py', 'tools/verification/production_attestation.py',
           'tools/checks/build_budget.py', 'tools/checks/dependency_budget.py']

def start(output):
    output = Path(output).absolute()
    p139.need(output == output.resolve() and output.is_relative_to(ROOT / 'target/verification'),
               'P140 output must stay under target/verification')
    output.mkdir(parents=True, exist_ok=False)
    (output / 'helpers').mkdir()
    for name in HELPERS:
        shutil.copyfile(ROOT / name, output / 'helpers' / Path(name).name)
    p139.write_new(output / 'invocation.json', {'argv': list(sys.argv), 'cwd': str(Path.cwd())})
    return output

def archive():
    manifest = baseline.verify_archive(ARCHIVE)
    p139.need(p139.sha(ARCHIVE / 'manifest.json') == ARCHIVE_SHA
               and manifest['repository_commit'] == COMMIT and manifest['mode'] == 'register',
               'P140 predecessor changed')
    return manifest

def prerequisite():
    folder = ROOT / 'docs/benchmarks/performance'
    paths = {name: folder / f'P139-{name}.json' for name in ('completion', 'evidence', 'proof')}
    values = {name: p139.read_json(path, name) for name, path in paths.items()}
    p139.validate_prerequisite_envelope('P139', values['completion'], values['evidence'], values['proof'])
    evidence = values['completion']['evidence'][0]
    p139.need(p139.ref(Path(evidence['path'])) == evidence
               and p139.read_json(Path(evidence['path']), 'retained P139 evidence') == values['evidence'],
               'P139 completion reference changed')
    return {name: p139.ref(path) for name, path in paths.items()}

def freeze(output):
    output = start(output)
    archive()
    current = focused_compare.sources()
    core = p139._runtime_source_map()
    budgets = p139._validate_p138_build_budgets(core)
    selection_manifest.freeze(SELECTION, ARCHIVE / 'manifest.json', output / 'selection-freeze.json')
    result = {'schema': 'wtime-p140-freeze-v1', 'status': 'passed', 'source_stable': True, 'outcome': 'contract_only',
              'timing_evidence': False, 'archive': p139.ref(ARCHIVE / 'manifest.json'),
              'selection': p139.ref(SELECTION), 'selection_freeze': p139.ref(output / 'selection-freeze.json'),
              'prerequisite': prerequisite(), 'sources': current, 'p138_build_budgets': budgets}
    p139.need(focused_compare.sources() == current, 'sources changed during P140 freeze')
    p139.write_new(output / 'report.json', result)
    return result

def frozen_inputs(path):
    frozen = p139.read_json(path, 'P140 freeze')
    archive()
    p139.need(frozen.get('schema') == 'wtime-p140-freeze-v1' and frozen.get('status') == 'passed'
               and frozen['archive'] == p139.ref(ARCHIVE / 'manifest.json')
               and frozen['selection'] == p139.ref(SELECTION)
               and frozen['selection_freeze'] == p139.ref(Path(frozen['selection_freeze']['path']))
               and frozen['prerequisite'] == prerequisite()
               and frozen['sources'] == focused_compare.sources(), 'P140 frozen inputs changed')
    p139.need(frozen['p138_build_budgets'] == p139._validate_p138_build_budgets(p139._runtime_source_map()),
               'P140 reused core budgets are stale')
    declaration = selection_manifest.verify_frozen(SELECTION, Path(frozen['selection_freeze']['path']))
    return frozen, declaration

def invoke(output, name, argv):
    p139.write_new(output / f'{name}.command.json', {'argv': argv, 'cwd': str(ROOT)})
    with (output / f'{name}.stdout').open('x') as stdout, (output / f'{name}.stderr').open('x') as stderr:
        result = subprocess.run(argv, cwd=ROOT, stdout=stdout, stderr=stderr)
    p139.write_new(output / f'{name}.result.json', {'exit_code': result.returncode})
    p139.need(result.returncode == 0, f'P140 {name} command failed')
    return json.loads((output / f'{name}.stdout').read_text())

def smoke(freeze_path, production_path, threshold, output):
    output = start(output)
    frozen, declaration = frozen_inputs(freeze_path)
    setting = policy.request(declaration, 'register', threshold)
    record = p139.read_json(production_path, 'P140 production build')
    binaries = {'baseline': ARCHIVE / 'binary/wtime-bench', 'candidate': Path(record['binary_path'])}
    fixture = ARCHIVE / 'fixtures/call_overhead.wasm'
    cpu = sorted(os.sched_getaffinity(0))[0]
    capabilities, observations, commands = {}, {}, {}
    for side, binary in binaries.items():
        capabilities[side] = invoke(output, side + '-protocol', [str(binary), '--protocol'])
        argv = ['taskset', '-c', str(cpu), str(binary), '--single-case', str(fixture),
                'call_overhead', '100', '101', 'warm_call', '8', 'register']
        if setting[side] is not None: argv += ['--register-entry-threshold', str(setting[side])]
        observations[side] = invoke(output, side, argv)
        commands[side] = p139.ref(output / f'{side}.command.json')
    metadata = {'settings': {'candidate_mode': 'register', 'baseline_mode': 'register',
                            'instrumented': False, 'register_entry_threshold': setting},
                'candidate_sources': frozen['sources'], 'capabilities': capabilities,
                'candidate_binary_sha256': p139.sha(binaries['candidate'])}
    result = {'schema': 'wtime-p140-smoke-v1', 'status': 'passed', 'passed': True,
              'source_stable': True, 'outcome': 'contract_only', 'timing_evidence': False,
              'gate_eligible': False, 'candidate_promoted': False, 'threshold': threshold,
              'freeze': p139.ref(freeze_path), 'production': p139.ref(production_path),
              'declaration': declaration, 'metadata': metadata, 'observations': observations,
              'binaries': {side: p139.ref(binary) for side, binary in binaries.items()},
              'fixture': p139.ref(fixture), 'cpu': cpu, 'commands': commands,
              'helper_refs': {Path(name).name: p139.ref(output / 'helpers' / Path(name).name) for name in HELPERS}}
    p139.write_new(output / 'report.json', result)
    return check(output / 'report.json')

def check(path):
    path = Path(path).resolve(strict=True)
    report = p139.read_json(path, 'P140 smoke')
    p139.need(report.get('schema') == 'wtime-p140-smoke-v1' and report.get('status') == 'passed'
               and report.get('passed') is True and report.get('source_stable') is True
               and report.get('outcome') == 'contract_only' and report.get('timing_evidence') is False
               and report.get('gate_eligible') is False and report.get('candidate_promoted') is False,
               'P140 smoke envelope is invalid')
    frozen, declaration = frozen_inputs(Path(report['freeze']['path']))
    p139.need(report['freeze'] == p139.ref(Path(report['freeze']['path']))
               and report['declaration'] == declaration
               and report['production'] == p139.ref(Path(report['production']['path']))
               and report['fixture'] == p139.ref(ARCHIVE / 'fixtures/call_overhead.wasm'),
               'P140 smoke input references changed')
    metadata = report['metadata']
    setting = candidate_gate.validate_entry_setting(declaration, metadata['settings'],
               {'command': p139.ref(path.parent / 'invocation.json')}, metadata['capabilities'])
    p139.need(report['threshold'] == setting['candidate'] == 3
               and metadata['candidate_sources'] == frozen['sources'], 'P140 threshold/source identity differs')
    production_attestation.validate_runtime_record(Path(report['production']['path']), metadata, report, ROOT)
    production = p139.read_json(Path(report['production']['path']), 'production record')
    expected_binaries = {'baseline': ARCHIVE / 'binary/wtime-bench', 'candidate': Path(production['binary_path'])}
    selected = next(row for row in selection_manifest.rows(declaration) if row['id'] == 'call_overhead')
    p139.need(report['fixture']['sha256'] == selected['source']['wasm_sha256'],
               'P140 smoke fixture differs from frozen selection')
    for side in ('baseline', 'candidate'):
        binary = Path(report['binaries'][side]['path'])
        p139.need(report['binaries'][side] == p139.ref(expected_binaries[side]), 'P140 binary changed')
        argv = ['taskset', '-c', str(report['cpu']), str(binary), '--single-case', report['fixture']['path'],
                'call_overhead', '100', '101', 'warm_call', '8', 'register']
        if setting[side] is not None: argv += ['--register-entry-threshold', str(setting[side])]
        command_path = path.parent / f'{side}.command.json'
        p139.need(report['commands'][side] == p139.ref(command_path)
                   and p139.read_json(command_path, 'driver command') == {'argv': argv, 'cwd': str(ROOT)}
                   and p139.read_json(path.parent / f'{side}.result.json', 'driver exit') == {'exit_code': 0}
                   and (path.parent / f'{side}.stderr').read_bytes() == b'', 'P140 actual command differs')
        wire = json.loads((path.parent / f'{side}.stdout').read_text())
        p139.need(wire == report['observations'][side], 'P140 retained wire changed')
        compare.validate_observation(wire, 'call_overhead', 'warm_call', 100, 101, 8, 'register')
        policy.validate_wire(wire, setting[side])
        cap = json.loads((path.parent / f'{side}-protocol.stdout').read_text())
        p139.need(cap == metadata['capabilities'][side] and cap.get('instrumented') is False
                   and 'register' in cap.get('modes', []), 'P140 protocol capability differs')
        p139.need(p139.read_json(path.parent / f'{side}-protocol.command.json', 'protocol command')
                   == {'argv': [str(binary), '--protocol'], 'cwd': str(ROOT)}
                   and p139.read_json(path.parent / f'{side}-protocol.result.json', 'protocol result') == {'exit_code': 0}
                   and (path.parent / f'{side}-protocol.stderr').read_bytes() == b'', 'P140 protocol command differs')
    expected = {Path(name).name: name for name in HELPERS}
    p139.need(set(report['helper_refs']) == set(expected), 'P140 helper set differs')
    for name, source in expected.items():
        helper = path.parent / 'helpers' / name
        p139.need(report['helper_refs'][name] == p139.ref(helper) and p139.sha(helper) == p139.sha(ROOT / source),
                   'P140 executed helper changed')
    p139.need(focused_compare.sources() == frozen['sources'], 'P140 sources changed during checking')
    return report

if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest='command', required=True)
    freezing = sub.add_parser('freeze'); freezing.add_argument('--output', type=Path, required=True)
    collecting = sub.add_parser('smoke')
    collecting.add_argument('--freeze', type=Path, required=True)
    collecting.add_argument('--production', type=Path, required=True)
    collecting.add_argument('--register-entry-threshold', type=int, required=True)
    collecting.add_argument('--output', type=Path, required=True)
    checking = sub.add_parser('check'); checking.add_argument('--report', type=Path, required=True)
    args = parser.parse_args()
    result = freeze(args.output) if args.command == 'freeze' else (
        smoke(args.freeze, args.production, args.register_entry_threshold, args.output)
        if args.command == 'smoke' else check(args.report))
    print(json.dumps({'status': result['status'], 'timing_evidence': False}))
