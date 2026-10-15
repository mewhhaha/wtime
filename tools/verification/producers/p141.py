"""Source-bound individual-entry protocol smoke; no performance promotion."""
from pathlib import Path
import argparse
import json
import os
import shutil
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(ROOT / 'tools/wtime-bench'))
from tools.verification.producers import p138, p139, p140
from tools.verification import production_attestation
import baseline
import entry_sequence_policy as policy
import focused_compare

ARCHIVE = ROOT / 'target/performance/P141-before'
ARCHIVE_SHA = '63b22d3a53dbb40e16bc30676f6ce78bf796faf047ac2faf11f02922ce1f56ca'
COMMIT = '02674dc16b4d94d79646bda22a86969e6146ac99'
THRESHOLDS = (3, 10)
INPUT = 2147483647
# This executed producer collected valid wires before its final attestation
# call exposed a status-string/boolean adapter mismatch. Only its checker is
# superseded; retain and revalidate the immutable observations without timing.
RETAINED_PRODUCER_SHA = 'b2476db5867bc79bc8521b7593b863bc8237a92120351eab5ffd4cc2e728b782'
HELPERS = ['tools/verification/producers/p141.py', 'tools/verification/producers/p141_driver_tests.py',
           'tools/verification/producers/p138.py', 'tools/verification/producers/p139.py',
           'tools/verification/producers/p140.py', 'tools/verification/producers/runtime_production.py',
           'tools/verification/production_attestation.py', 'tools/checks/build_budget.py',
           'tools/checks/dependency_budget.py', 'tools/wtime-bench/entry_sequence_policy.py']

def start(output):
    output = Path(output).absolute()
    p139.need(output == output.resolve() and output.is_relative_to(ROOT / 'target/verification'),
               'P141 output must stay under target/verification')
    output.mkdir(parents=True, exist_ok=False)
    (output / 'helpers').mkdir()
    for name in HELPERS: shutil.copyfile(ROOT / name, output / 'helpers' / Path(name).name)
    p139.write_new(output / 'invocation.json', {'argv': list(sys.argv), 'cwd': str(Path.cwd())})
    return output

def archive():
    manifest = baseline.verify_archive(ARCHIVE)
    p139.need(p139.sha(ARCHIVE / 'manifest.json') == ARCHIVE_SHA
               and manifest['repository_commit'] == COMMIT and manifest['mode'] == 'register',
               'P141 predecessor changed')
    return p139.ref(ARCHIVE / 'manifest.json')

def prerequisites():
    result = {'P138': p139._prerequisite('P138')}
    folder = ROOT / 'docs/benchmarks/performance'
    paths = {name: folder / f'P140-{name}.json' for name in ('completion', 'evidence', 'proof')}
    values = {name: p139.read_json(path, name) for name, path in paths.items()}
    p139.validate_prerequisite_envelope('P140', values['completion'], values['evidence'], values['proof'])
    evidence_ref = values['completion']['evidence'][0]
    p139.need(p139.ref(Path(evidence_ref['path'])) == evidence_ref
               and p139.read_json(Path(evidence_ref['path']), 'retained P140 evidence') == values['evidence'],
               'P140 completion reference changed')
    ref = values['evidence']['phases']['verify']
    receipt_path = Path(values['evidence']['store']) / ref['path']
    p139.need(p139.sha(receipt_path) == ref['sha256'], 'P140 retained phase changed')
    receipt = p139.read_json(receipt_path, 'P140 phase')
    proof_ref = receipt['original_outputs']['smoke']
    p139.need(p139.ref(Path(proof_ref['path'])) == proof_ref
               and p139.read_json(Path(proof_ref['path']), 'P140 retained smoke') == values['proof'],
               'P140 retained proof differs')
    result['P140'] = {name: p139.ref(path) for name, path in paths.items()}
    return result

def freeze(output):
    output = start(output)
    current = focused_compare.sources()
    result = {'schema': 'wtime-p141-freeze-v1', 'status': 'passed', 'source_stable': True,
              'outcome': 'contract_only', 'timing_evidence': False, 'archive': archive(),
              'prerequisites': prerequisites(), 'fixture': p138.fixture_identity(), 'sources': current,
              'p138_build_budgets': p139._validate_p138_build_budgets(p139._runtime_source_map())}
    p139.need(focused_compare.sources() == current, 'P141 sources changed during freeze')
    p139.write_new(output / 'report.json', result)
    return result

def frozen_inputs(path):
    frozen = p139.read_json(path, 'P141 freeze')
    p139.need(frozen.get('schema') == 'wtime-p141-freeze-v1' and frozen.get('status') == 'passed'
               and frozen.get('source_stable') is True and frozen['sources'] == focused_compare.sources()
               and frozen['archive'] == archive() and frozen['prerequisites'] == prerequisites()
               and frozen['fixture'] == p138.fixture_identity()
               and frozen['p138_build_budgets'] == p139._validate_p138_build_budgets(p139._runtime_source_map()),
               'P141 frozen inputs changed')
    return frozen

def driver(path, frozen, diagnostic):
    report = p139.read_json(path, 'driver tests')
    features = (['runtime-only', 'compact-executor', 'register-executor', 'execution-reference'] if diagnostic
                else ['runtime-only', 'register-executor'])
    p139.need(report.get('status') == 'passed' and report.get('source_stable') is True
               and report['sources'] == frozen['sources'] and report['features'] == features
               and report.get('instrumented') is diagnostic and report.get('timing_evidence') is False,
               'P141 driver test identity differs')
    stdout, stderr = path.parent / 'tests.stdout', path.parent / 'tests.stderr'
    sys.path.insert(0, str(ROOT / 'tools'))
    from verification.runner import test_results
    from tools.verification.producers.p141_driver_tests import NAMES
    tests = test_results(stdout.read_text() + '\n' + stderr.read_text(),
                        {'required_names': NAMES, 'minimum_passed': 18 if diagnostic else 12,
                         'features': features, 'unfiltered': True})
    p139.need(tests == report['tests'] and report['binaries']
               and all(p139.sha(Path(name)) == digest for name, digest in report['binaries'].items()),
               'P141 actual tests or executable changed')
    command = p139.read_json(path.parent / 'tests.command.json', 'actual Cargo command')
    p139.need(command == {'argv': ['cargo', 'test', '--locked', '--offline', '--features', ','.join(features), '--', '--nocapture'],
                          'cwd': str(path.parent)}
               and p139.read_json(path.parent / 'tests.result.json', 'actual Cargo result') == {'exit_code': 0},
               'P141 Cargo test command changed')
    traces = [json.loads(line.removeprefix('P141_TRACE ')) for line in stderr.read_text().splitlines()
              if line.startswith('P141_TRACE ')]
    p139.need(len(traces) == (4 if diagnostic else 0), 'P141 diagnostic trace count differs')
    settings = set()
    for trace in traces:
        n, strategy = trace['schedule_threshold'], trace['strategy']
        p139.need(n in THRESHOLDS, 'unexpected diagnostic threshold')
        policy.validate_payload(trace, n, strategy, 0 if strategy == 'eager' else n, INPUT, True)
        settings.add((n, strategy))
    if diagnostic: p139.need(settings == {(n, side) for n in THRESHOLDS for side in ('eager', 'adaptive')},
                              'P141 diagnostic settings differ')
    return {'report': p139.ref(path), 'stdout': p139.ref(stdout), 'stderr': p139.ref(stderr), 'traces': traces}

def smoke(freeze_path, diagnostic_path, production_driver_path, production_path, output):
    output = start(output)
    frozen = frozen_inputs(freeze_path)
    diagnostic = driver(diagnostic_path, frozen, True)
    production_tests = driver(production_driver_path, frozen, False)
    record = p139.read_json(production_path, 'production build')
    binary = Path(record['binary_path'])
    cpu = sorted(os.sched_getaffinity(0))[0]
    fixture = frozen['fixture']['wasm']
    observations = []
    for n in THRESHOLDS:
        for strategy in ('eager', 'adaptive'):
            name = f'{n}-{strategy}'
            threshold = 0 if strategy == 'eager' else n
            argv = ['taskset', '-c', str(cpu), str(binary), '--entry-sequence', fixture['path'],
                    str(INPUT), str(n), 'register', strategy, str(threshold)]
            wire = p140.invoke(output, name, argv)
            policy.validate_payload(wire, n, strategy, threshold, INPUT)
            observations.append({'name': name, 'schedule_threshold': n, 'strategy': strategy,
                                 'threshold': threshold, 'payload': wire})
    result = {'schema': 'wtime-p141-smoke-v1', 'status': 'passed', 'source_stable': True,
              'outcome': 'contract_only', 'timing_evidence': False, 'gate_eligible': False, 'candidate_promoted': False,
              'thresholds': list(THRESHOLDS), 'counts': {'diagnostic_observations': 4, 'timing_observations': 4,
                                                       'individual_calls_per_lane': 2010},
              'freeze': p139.ref(freeze_path), 'production': p139.ref(production_path), 'binary': p139.ref(binary),
              'diagnostic': diagnostic, 'production_tests': production_tests, 'cpu': cpu,
              'observations': observations, 'fixture': fixture,
              'helper_refs': {Path(name).name: p139.ref(output / 'helpers' / Path(name).name) for name in HELPERS}}
    p139.write_new(output / 'report.json', result)
    return check(output / 'report.json')

def check(path):
    report = p139.read_json(path, 'P141 smoke')
    p139.need(report.get('schema') == 'wtime-p141-smoke-v1' and report.get('status') == 'passed'
               and report.get('source_stable') is True and report.get('outcome') == 'contract_only'
               and all(report.get(key) is False for key in ('timing_evidence', 'gate_eligible', 'candidate_promoted')),
               'P141 smoke envelope differs')
    frozen = frozen_inputs(Path(report['freeze']['path']))
    p139.need(report['freeze'] == p139.ref(Path(report['freeze']['path']))
               and report['production'] == p139.ref(Path(report['production']['path']))
               and report['fixture'] == frozen['fixture']['wasm'] and report['thresholds'] == list(THRESHOLDS),
               'P141 smoke inputs changed')
    for key, diagnostic in (('diagnostic', True), ('production_tests', False)):
        p139.need(report[key] == driver(Path(report[key]['report']['path']), frozen, diagnostic),
                   'P141 retained driver evidence changed')
    record = p139.read_json(Path(report['production']['path']), 'production build')
    metadata = {'settings': {'candidate_mode': 'register', 'instrumented': False},
                'candidate_sources': frozen['sources'], 'candidate_binary_sha256': report['binary']['sha256'],
                'capabilities': {'candidate': record['protocol']}}
    production_attestation.validate_runtime_record(Path(report['production']['path']), metadata,
                                                   {**report, 'passed': report['status'] == 'passed'}, ROOT)
    p139.need(report['binary'] == p139.ref(Path(record['binary_path'])), 'P141 production binary changed')
    p139.need(len(report['observations']) == 4, 'P141 observation count differs')
    total = 0
    for observed, (n, strategy) in zip(report['observations'], [(n, side) for n in THRESHOLDS for side in ('eager', 'adaptive')]):
        name, threshold = f'{n}-{strategy}', 0 if strategy == 'eager' else n
        p139.need(observed['name'] == name and observed['schedule_threshold'] == n
                   and observed['strategy'] == strategy and observed['threshold'] == threshold,
                   'P141 observation settings differ')
        wire = json.loads((path.parent / f'{name}.stdout').read_text())
        policy.validate_payload(wire, n, strategy, threshold, INPUT)
        p139.need(observed['payload'] == wire, 'P141 retained wire changed')
        expected = ['taskset', '-c', str(report['cpu']), report['binary']['path'], '--entry-sequence',
                    report['fixture']['path'], str(INPUT), str(n), 'register', strategy, str(threshold)]
        p139.need(p139.read_json(path.parent / f'{name}.command.json', 'sequence command')
                   == {'argv': expected, 'cwd': str(ROOT)}
                   and p139.read_json(path.parent / f'{name}.result.json', 'sequence result') == {'exit_code': 0}
                   and (path.parent / f'{name}.stderr').read_bytes() == b'', 'P141 actual sequence command differs')
        total += wire['actual_calls']
    p139.need(report['counts'] == {'diagnostic_observations': 4, 'timing_observations': 4, 'individual_calls_per_lane': total}
               and total == 2010, 'P141 call count differs')
    expected = {Path(name).name: name for name in HELPERS}
    p139.need(set(report['helper_refs']) == set(expected), 'P141 helper set differs')
    for name, source in expected.items():
        copied = path.parent / 'helpers' / name
        allowed = {p139.sha(ROOT / source)}
        if name == 'p141.py': allowed.add(RETAINED_PRODUCER_SHA)
        p139.need(report['helper_refs'][name] == p139.ref(copied) and p139.sha(copied) in allowed,
                   'P141 executed helper changed')
    p139.need(focused_compare.sources() == frozen['sources'], 'P141 sources changed during checking')
    return report

def revalidate(path, output):
    output = start(output)
    sources = focused_compare.sources()
    smoke = check(path)
    result = {'schema': 'wtime-p141-verification-v1', 'status': 'passed', 'source_stable': True,
              'outcome': 'contract_only', 'timing_evidence': False, 'gate_eligible': False,
              'candidate_promoted': False, 'guest_or_build_launched': False, 'smoke': p139.ref(path),
              'counts': smoke['counts'], 'thresholds': smoke['thresholds'],
              'tests': {name: p139.read_json(Path(smoke[key]['report']['path']), 'driver tests')['tests']
                        for name, key in (('diagnostic', 'diagnostic'), ('production', 'production_tests'))},
              'retained': {'driver_diagnostic': smoke['diagnostic']['report'],
                           'driver_production': smoke['production_tests']['report'],
                           'production': smoke['production'], 'smoke': p139.ref(path)},
              'producer': p139.ref(output / 'helpers/p141.py'),
              'retained_producer_sha256': p139.sha(path.parent / 'helpers/p141.py')}
    p139.need(focused_compare.sources() == sources, 'P141 sources changed during revalidation')
    p139.write_new(output / 'report.json', result)
    return result

if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest='command', required=True)
    freezing = sub.add_parser('freeze'); freezing.add_argument('--output', type=Path, required=True)
    collecting = sub.add_parser('smoke')
    collecting.add_argument('--freeze', type=Path, required=True)
    collecting.add_argument('--driver-diagnostic', type=Path, required=True)
    collecting.add_argument('--driver-production', type=Path, required=True)
    collecting.add_argument('--production', type=Path, required=True)
    collecting.add_argument('--output', type=Path, required=True)
    checking = sub.add_parser('check'); checking.add_argument('--report', type=Path, required=True)
    validating = sub.add_parser('revalidate')
    validating.add_argument('--report', type=Path, required=True)
    validating.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if args.command == 'freeze': result = freeze(args.output)
    elif args.command == 'smoke':
        result = smoke(args.freeze, args.driver_diagnostic, args.driver_production, args.production, args.output)
    elif args.command == 'revalidate': result = revalidate(args.report, args.output)
    else: result = check(args.report)
    print(json.dumps({'status': result['status'], 'timing_evidence': False}))
