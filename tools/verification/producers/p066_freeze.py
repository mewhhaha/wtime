"""Freeze and revalidate the immutable P066 declaration and fixtures."""
from pathlib import Path
import argparse
import hashlib
import json
import shutil
import sys
ROOT = Path(__file__).resolve().parents[3]
BENCH = ROOT / 'tools/wtime-bench'
sys.path[:0] = [str(ROOT), str(BENCH)]
import baseline
import focused_compare
import selection_manifest
from tools.verification.producers import p138, p139
ARCHIVE = ROOT / 'target/performance/P066-before'
ARCHIVE_SHA = '3ac832685a4b55470af29eeb2eb78a032263b6ea2ca3e17552a711d6c2753435'
COMMIT = '8ffa108ee2b3607d14a031304a0da2cd0252f9b9'
PREDECLARATION = ROOT / 'target/performance/P066/predeclaration.json'
SELECTION = ROOT / 'docs/benchmarks/performance/P066-selection.json'
POLICY = ROOT / 'docs/benchmarks/performance/P066-contract.json'
SEEDS = [66001, 66002]
HELPERS = [
    'tools/verification/producers/p066_freeze.py',
    'tools/verification/producers/p138.py', 'tools/verification/producers/p139.py',
    'tools/wtime-bench/baseline.py', 'tools/wtime-bench/compare.py',
    'tools/wtime-bench/focused_compare.py', 'tools/wtime-bench/selection_manifest.py',
    'tools/wtime-bench/execution_identity.py', 'tools/wtime-bench/cold_products.py',
    'tools/wtime-bench/p140_entry_policy.py',
]


def need(ok, message):
    if not ok:
        raise ValueError(message)


def read(path):
    return json.loads(Path(path).read_text())


def ref(path):
    path = Path(path).resolve(strict=True)
    return {'path': str(path), 'sha256': hashlib.sha256(path.read_bytes()).hexdigest()}


def write(path, value):
    with Path(path).open('x') as stream:
        json.dump(value, stream, indent=2, sort_keys=True, allow_nan=False)
        stream.write('\n')


def refs_current(value):
    if isinstance(value, dict):
        if isinstance(value.get('path'), str) and 'sha256' in value:
            actual = ref(value['path'])
            need(all(actual[k] == value[k] for k in actual), 'stale artifact: ' + value['path'])
            if 'bytes' in value:
                need(Path(value['path']).stat().st_size == value['bytes'], 'stale artifact size')
        else:
            for child in value.values():
                refs_current(child)
    elif isinstance(value, list):
        for child in value:
            refs_current(child)


def start(output):
    output = Path(output).absolute()
    need(output == output.resolve() and output.is_relative_to(ROOT / 'target/verification'),
         'P066 output must stay within target/verification')
    output.mkdir(parents=True, exist_ok=False)
    (output / 'helpers').mkdir()
    helpers = {}
    for name in HELPERS:
        path = ROOT / name
        target = output / 'helpers' / path.name
        shutil.copyfile(path, target)
        helpers[path.name] = ref(target)
    write(output / 'invocation.json', {'argv': list(sys.argv), 'cwd': str(Path.cwd())})
    return output, helpers


def declared():
    archive = baseline.verify_archive(ARCHIVE)
    need(ref(ARCHIVE / 'manifest.json')['sha256'] == ARCHIVE_SHA
         and archive['repository_commit'] == COMMIT and archive['mode'] == 'register',
         'P066 immediate predecessor differs')
    frozen = read(PREDECLARATION)
    need(frozen.get('schema') == 'wtime-p066-predeclaration-v1', 'missing P066 predeclaration')
    need(frozen['selection'] == ref(SELECTION) and frozen['policy'] == ref(POLICY)
         and frozen['archive'] == ref(ARCHIVE / 'manifest.json'), 'P066 predeclaration changed')
    selection_manifest.verify_frozen(SELECTION, PREDECLARATION.parent / 'selection-freeze.json')
    refs_current(frozen)
    return frozen


def freeze(output):
    output, helpers = start(output)
    declaration = declared()
    sources = focused_compare.sources()
    # These validators bind actual encoded fixtures and their retained official
    # validation commands, without pretending older runtime proofs are current.
    fixtures = {'counter': p139._validate_fixture_provenance(), 'uniform66': p138.fixture_identity()}
    for filename, key in [('raw-rows.json', 'raw_rows'), ('sequence-rows.json', 'sequence_rows')]:
        write(output / filename, declaration[key])
    shutil.copyfile(PREDECLARATION.parent / 'selection-freeze.json', output / 'selection.json')
    result = {'schema': 'wtime-p066-freeze-v1', 'status': 'passed', 'source_stable': True,
              'predeclaration': ref(PREDECLARATION), 'sources': sources, 'fixtures': fixtures,
              'selection': ref(output / 'selection.json'), 'raw_rows': ref(output / 'raw-rows.json'),
              'sequence_rows': ref(output / 'sequence-rows.json'), 'helpers': helpers,
              'invocation': ref(output / 'invocation.json')}
    need(focused_compare.sources() == sources, 'sources changed during freeze')
    write(output / 'report.json', result)
    return result


def check_frozen(path):
    declaration = declared()
    frozen = read(path)
    need(frozen.get('schema') == 'wtime-p066-freeze-v1' and frozen.get('status') == 'passed'
         and frozen.get('source_stable') is True and frozen['predeclaration'] == ref(PREDECLARATION)
         and frozen['sources'] == focused_compare.sources(), 'P066 frozen sources changed')
    refs_current({key: frozen[key] for key in ('predeclaration', 'selection', 'raw_rows', 'sequence_rows', 'helpers', 'invocation')})
    need(read(frozen['raw_rows']['path']) == declaration['raw_rows']
         and read(frozen['sequence_rows']['path']) == declaration['sequence_rows'], 'P066 frozen rows changed')
    need(frozen['fixtures'] == {'counter': p139._validate_fixture_provenance(), 'uniform66': p138.fixture_identity()},
         'P066 fixture provenance changed')
    selection_manifest.verify_frozen(SELECTION, frozen['selection']['path'])
    need(set(frozen['helpers']) == {Path(name).name for name in HELPERS}, 'freeze helper inventory differs')
    for name in HELPERS:
        need(frozen['helpers'][Path(name).name]['sha256'] == ref(ROOT / name)['sha256'], 'freeze helper source changed')
    invocation = read(frozen['invocation']['path'])
    argv = invocation.get('argv', [])
    need(invocation.get('cwd') == str(ROOT) and len(argv) == 3
         and Path(argv[0]).resolve() == Path(__file__).resolve() and argv[1] == '--output'
         and Path(argv[2]).resolve() == Path(path).resolve().parent, 'freeze invocation differs')
    return frozen, declaration


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    result = freeze(parser.parse_args().output)
    print(json.dumps({'status': result['status'], 'source_stable': result['source_stable']}))
