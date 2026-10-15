"""Bind the three P066 lanes without rerunning guests or statistics."""
from pathlib import Path
import argparse
import shutil
import sys
ROOT = Path(__file__).resolve().parents[3]
BENCH = ROOT / 'tools/wtime-bench'
sys.path[:0] = [str(ROOT), str(BENCH)]
import candidate_gate
import entry_batch_analyze as totals
import entry_batch_compare as batches
import entry_sequence_analyze as sequences
import entry_sequence_compare as entries
from tools.verification.producers import p066_gate
from tools.verification.producers.p066_freeze import (
    ARCHIVE, POLICY, SELECTION, SEEDS, need, read, ref, write, refs_current, check_frozen)
import json
def start(output):
    output = Path(output).absolute()
    need(output == output.resolve() and output.is_relative_to(ROOT / 'target/verification'),
         'P066 output must stay within target/verification')
    output.mkdir(parents=True, exist_ok=False)
    (output / 'helpers').mkdir()
    helpers = {}
    for path in (Path(__file__), Path(p066_gate.__file__)):
        target = output / 'helpers' / path.name
        shutil.copyfile(path, target)
        helpers[path.name] = ref(target)
    write(output / 'invocation.json', {'argv': list(sys.argv), 'cwd': str(Path.cwd())})
    return output, helpers


def check_analysis(path, module, row_values):
    analysis_ref = ref(path)
    analysis = read(path)
    need(ref(path) == analysis_ref, 'analysis changed while reading')
    need(analysis.get('status') == 'passed' and analysis.get('timing_evidence') is True
         and analysis.get('gate_eligible') is True and analysis.get('decision') is None
         and analysis.get('threshold_selection') is None and analysis.get('scaled_by_calls') is False,
         'supplementary analysis is not full production evidence')
    refs_current(analysis.get('comparison'))
    root, report = module.verify_current_collection(Path(analysis['comparison']['path']).parent)
    identity = report['identity']
    pair_module = batches if module is totals else entries
    need(analysis['report_sha256'] == pair_module.digest(report)
         and analysis['collection_identity_sha256'] == pair_module.digest(identity), 'analysis/raw binding differs')
    need(identity['smoke'] is False and identity['pairs_per_row'] == 30 and identity['seeds'] == SEEDS,
         'supplementary sample matrix differs')
    need(identity['archive_manifest'] == pair_module.reference(ARCHIVE / 'manifest.json'), 'supplementary predecessor differs')
    expected = pair_module.normalize_rows(row_values)
    need(pair_module.digest(identity['rows']) == pair_module.digest(expected), 'supplementary rows differ')
    actual_inputs = totals.bound_inputs(report) if module is totals else sequences._reference_map(report)
    need(analysis['input_refs'] == actual_inputs, 'analysis source/input binding differs')
    refs_current(actual_inputs)
    maps = {'analysis_source_refs': {'entry_batch_analyze.py', 'analyze.py', 'bootstrap.py'},
            'analysis_helpers': {'entry_batch_analyze.py', 'analyze.py', 'bootstrap.py'}} if module is totals else {
        'analysis_source_refs': {'entry_sequence_analyze.py', 'entry_sequence_compare.py',
                                 'entry_sequence_policy.py', 'analyze.py', 'bootstrap.py'}}
    for key, names in maps.items():
        need(isinstance(analysis.get(key), dict) and set(analysis[key]) == names, 'statistics helper inventory differs')
    for key in ('analysis_source_refs', 'analysis_helpers'):
        for name, item in analysis.get(key, {}).items():
            refs_current(item)
            need(item['sha256'] == ref(BENCH / name)['sha256'], 'statistics helper differs')
    invocation_path = Path(path).parent / 'invocation.json'
    invocation = read(invocation_path)
    argv = invocation.get('argv', [])
    need(invocation.get('cwd') == str(ROOT) and argv
         and Path(argv[0]).resolve() == Path(module.__file__).resolve()
         and '--allow-smoke' not in argv, 'analysis invocation differs')
    for flag, expected_path in [('--comparison', root), ('--output', Path(path).parent)]:
        need(argv.count(flag) == 1 and argv.index(flag) + 1 < len(argv)
             and Path(argv[argv.index(flag) + 1]).resolve() == expected_path.resolve(), 'analysis argv differs')
    need(module is totals or 'invocation' in analysis, 'sequence analysis invocation is missing')
    if 'invocation' in analysis:
        need(analysis['invocation'] == pair_module.reference(invocation_path), 'analysis invocation reference differs')
    need(ref(path) == analysis_ref, 'analysis changed while validating')
    return analysis, {'analysis': analysis_ref, 'collection': ref(root / 'report.json'), 'invocation': ref(invocation_path)}


RETAINED_STATISTICS = ROOT / 'target/verification/runs/P066/analyze/attempt-0001'
STATISTICS_PINS = {
    'focused-statistics.json': '29dbf754ad0df1298a6c1fd5d7f2b4aaa7b08d1303661c90d96de712a63975bf',
    'totals/report.json': '65c579c4416bbdcfc9bc6098afc20622e589ce3d815d755707f7c506171bf9d7',
    'sequence/report.json': 'c673785cd274e71a63dd7fef9b6bc63cd11b02556b9047e288c0eac65d08f93a',
    'inputs.json': 'fbe9091872ffeeff8c611eb75324df05b1f68315aadec8f9409b8e335ecf40a0',
}


def retained_statistics(paths):
    from tools.verification.artifacts import ArtifactStore
    from tools.verification.runner import execution_environment
    need([Path(path).resolve() for path in paths] == [RETAINED_STATISTICS / name for name in (
        'focused-statistics.json', 'totals/report.json', 'sequence/report.json')], 'statistics reuse paths differ')
    refs = {name: ref(RETAINED_STATISTICS / name) for name in STATISTICS_PINS}
    need(all(item['sha256'] == STATISTICS_PINS[name] for name, item in refs.items()), 'retained statistics changed')
    prior = read(RETAINED_STATISTICS / 'inputs.json')
    need(execution_environment(ROOT, prior['definition']) == prior['environment'], 'statistics environment changed')
    store = ArtifactStore(ROOT / 'target/verification')
    commands = {}
    for name in ('p066-focused-statistics', 'p066-totals-analysis', 'p066-sequence-analysis'):
        path = RETAINED_STATISTICS / (name + '.result.json')
        record = read(path)
        need(record.get('exit_code') == 0 and record.get('cwd') == str(ROOT), 'original statistics command failed')
        for key in ('stdout', 'stderr'):
            store.verify(record[key])
        commands[name] = {'result': ref(path), 'stdout': record['stdout'], 'stderr': record['stderr']}
    return {'status': 'passed', 'reports': refs, 'commands': commands,
            'statistics_recomputed': False, 'guest_or_build_launched': False}


def analyze(freeze_path, focused_path, total_path, sequence_path, output):
    output, helpers = start(output)
    frozen, declaration = check_frozen(freeze_path)
    statistics_reuse = retained_statistics((focused_path, total_path, sequence_path))
    focused_ref = ref(focused_path)
    focused = read(focused_path)
    need(ref(focused_path) == focused_ref, 'focused statistics changed while reading')
    need(focused.get('schema') == 'wtime-verification-statistics-v1' and focused.get('task') == 'P066'
         and focused.get('status') == 'passed' and focused.get('timing_evidence') is True
         and focused.get('statistics_computations') == 1, 'focused statistics not passed')
    refs_current(focused['inputs'])
    need(focused['inputs']['contract']['path'] == str(SELECTION), 'focused selection differs')
    report = candidate_gate.validate(focused['inputs']['report']['path'])
    need([run['seed'] for run in report['runs']] == SEEDS, 'focused run seeds differ from frozen policy')
    need(report['metadata']['settings']['register_entry_threshold'] == {'baseline': None, 'candidate': 3},
         'focused policy differs')
    total, total_refs = check_analysis(total_path, totals, declaration['raw_rows'])
    sequence, sequence_refs = check_analysis(sequence_path, sequences, declaration['sequence_rows'])
    result = p066_gate.combine(focused['decision'], total, sequence)
    result.update({'schema': 'wtime-p066-combined-gate-v1', 'task': 'P066', 'status': 'passed',
                   'source_stable': True, 'freeze': ref(freeze_path), 'policy': ref(POLICY),
                   'statistics_reuse': statistics_reuse,
                   'focused_statistics': focused_ref, 'totals': total_refs, 'sequences': sequence_refs,
                   'helpers': helpers, 'invocation': ref(output / 'invocation.json'),
                   'statistics_recomputed': False, 'guest_or_build_launched': False,
                   'threshold_selection': None, 'candidate_promoted': result['decision'] == 'adopted'})
    check_frozen(freeze_path)
    refs_current(focused['inputs'])
    refs_current([focused_ref, total_refs, sequence_refs, helpers, statistics_reuse['reports']])
    for name, item in helpers.items():
        need(item['sha256'] == ref(Path(__file__).with_name(name))['sha256'], 'proof helper source changed')
    # Source-current collection validation performs no collection or statistics.
    totals.verify_current_collection(Path(total_refs['collection']['path']).parent)
    sequences.verify_current_collection(Path(sequence_refs['collection']['path']).parent)
    write(output / 'report.json', result)
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    subs = parser.add_subparsers(dest='operation', required=True)
    a = subs.add_parser('analyze')
    for name in ('freeze', 'focused-statistics', 'totals-analysis', 'sequence-analysis', 'output'):
        a.add_argument('--' + name, type=Path, required=True)
    args = parser.parse_args()
    result = analyze(args.freeze, args.focused_statistics, args.totals_analysis, args.sequence_analysis, args.output)
    print(json.dumps({'status': result['status'], 'source_stable': result['source_stable']}))
