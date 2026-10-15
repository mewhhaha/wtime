"""Freeze P142 rows and immutable prerequisite/core identities."""
from pathlib import Path
import argparse
import json
import shutil
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(ROOT / 'tools/wtime-bench'))
from tools.verification.producers import p138, p139
import baseline
import entry_batch_adapter as adapter

ARCHIVE = ROOT / 'target/performance/P142-before'
ARCHIVE_SHA = '3870d08abba86ab4d72428bc47ea8ac454c0022fcd94b59c8a6cf68208bdd618'
COMMIT = '6efc75e941857993ab1ed1bc71dd0afa445fb51e'
SEEDS = [142001, 142002]
THRESHOLDS = [3, 10]
INPUT = 2147483647
PREREQUISITES = {
    'P139': '52cfccaf7cfa5fca57d98a68bfb918d455b0a1fa92c346a9ea87ffcc0bd45a19',
    'P141': '6e3f8044a61e4746e750e63ff334d694ee17b1dd096efe5a119930673815d5ac',
}
HELPERS = ['tools/verification/producers/p142_freeze.py', 'tools/verification/producers/p138.py',
           'tools/verification/producers/p139.py', 'tools/checks/build_budget.py', 'tools/checks/dependency_budget.py']

def start(output, helpers):
    output = Path(output).absolute()
    p139.need(output == output.resolve() and output.is_relative_to(ROOT / 'target/verification'),
               'P142 output must stay under target/verification')
    output.mkdir(parents=True, exist_ok=False)
    (output / 'helpers').mkdir()
    for name in helpers: shutil.copyfile(ROOT / name, output / 'helpers' / Path(name).name)
    p139.write_new(output / 'invocation.json', {'argv': list(sys.argv), 'cwd': str(Path.cwd())})
    return output

def prerequisites():
    result = {}
    for task, proof_sha in PREREQUISITES.items():
        paths = {name: ROOT / f'docs/benchmarks/performance/{task}-{name}.json'
                 for name in ('completion', 'evidence', 'proof')}
        values = {name: p139.read_json(path, task + ' ' + name) for name, path in paths.items()}
        p139.validate_prerequisite_envelope(task, values['completion'], values['evidence'], values['proof'])
        ref = values['completion']['evidence'][0]
        p139.need(p139.ref(Path(ref['path'])) == ref and p139.sha(paths['proof']) == proof_sha
                   and p139.read_json(Path(ref['path']), 'retained evidence') == values['evidence'],
                   'P142 immutable prerequisite changed')
        result[task] = {name: p139.ref(path) for name, path in paths.items()}
    return result

def inputs():
    manifest = baseline.verify_archive(ARCHIVE)
    p139.need(p139.sha(ARCHIVE / 'manifest.json') == ARCHIVE_SHA
               and manifest['repository_commit'] == COMMIT and manifest['mode'] == 'register',
               'P142 immediate predecessor changed')
    core = p139._runtime_source_map()
    fixture = p138.fixture_identity()
    return {'archive': p139.ref(ARCHIVE / 'manifest.json'), 'prerequisites': prerequisites(),
            'core_sources': core, 'harness_sources': {name: p139.sha(path) for name, path in
                                                   adapter._harness_inputs(ROOT / 'tools/wtime-bench').items()},
            'fixture': fixture, 'p138_build_budgets': p139._validate_p138_build_budgets(core)}

def freeze(output):
    output = start(output, HELPERS)
    before = inputs()
    rows = [{'fixture': before['fixture']['wasm']['path'], 'input': INPUT, 'schedule_threshold': n}
            for n in THRESHOLDS]
    p139.write_new(output / 'rows.json', rows)
    result = {'schema': 'wtime-p142-freeze-v1', 'status': 'passed', 'source_stable': True,
              'outcome': 'contract_only', 'timing_evidence': False, 'inputs': before,
              'invocation': p139.ref(output / 'invocation.json'),
              'helper_refs': {Path(name).name: p139.ref(output / 'helpers' / Path(name).name) for name in HELPERS},
              'rows': p139.ref(output / 'rows.json'), 'seeds': SEEDS, 'smoke_pairs_per_row': 2}
    p139.need(inputs() == before, 'P142 inputs changed during freeze')
    p139.write_new(output / 'report.json', result)
    return result

def check(path):
    result = p139.read_json(path, 'P142 freeze')
    p139.need(result.get('schema') == 'wtime-p142-freeze-v1' and result.get('status') == 'passed'
               and result.get('source_stable') is True and result['inputs'] == inputs()
               and result['seeds'] == SEEDS and result['smoke_pairs_per_row'] == 2
               and result['rows'] == p139.ref(Path(result['rows']['path'])), 'P142 frozen inputs differ')
    expected = [{'fixture': result['inputs']['fixture']['wasm']['path'], 'input': INPUT, 'schedule_threshold': n}
                for n in THRESHOLDS]
    p139.need(json.loads(Path(result['rows']['path']).read_text()) == expected, 'P142 frozen rows differ')
    p139.need(result['invocation'] == p139.ref(path.parent / 'invocation.json'), 'P142 freeze invocation changed')
    invocation = p139.read_json(path.parent / 'invocation.json', 'freeze invocation')
    argv = invocation['argv']
    p139.need(invocation['cwd'] == str(ROOT) and len(argv) == 3 and argv[1] == '--output'
               and (ROOT / argv[0]).resolve() == Path(__file__).resolve()
               and (ROOT / argv[2]).resolve() == path.parent.resolve(), 'P142 freeze argv differs')
    p139.need(set(result['helper_refs']) == {Path(name).name for name in HELPERS}, 'P142 freeze helper set differs')
    for source in HELPERS:
        copied = path.parent / 'helpers' / Path(source).name
        p139.need(result['helper_refs'][copied.name] == p139.ref(copied)
                   and p139.sha(copied) == p139.sha(ROOT / source), 'P142 freeze helper changed')
    return result

if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    print(json.dumps({'status': freeze(args.output)['status'], 'timing_evidence': False}))
