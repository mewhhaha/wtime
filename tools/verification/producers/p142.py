"""Revalidate P142 source-bound smoke and its one descriptive analysis."""
from pathlib import Path
import argparse
import json
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(ROOT / 'tools/wtime-bench'))
from tools.verification.producers import p139, p142_freeze as freezing
import entry_sequence_compare as pairs
import entry_sequence_analyze as analysis
import entry_sequence_policy as policy

HELPERS = ['tools/verification/producers/p142.py', *freezing.HELPERS,
           'tools/wtime-bench/entry_sequence_compare.py', 'tools/wtime-bench/entry_sequence_analyze.py',
           'tools/wtime-bench/entry_sequence_policy.py', 'tools/wtime-bench/entry_batch_adapter.py',
           'tools/wtime-bench/entry_batch_compare.py', 'tools/wtime-bench/checkpoint.py',
           'tools/wtime-bench/analyze.py', 'tools/wtime-bench/bootstrap.py']

def verify(freeze_path, collection, analysis_path, output):
    output = freezing.start(output, HELPERS)
    frozen = freezing.check(freeze_path)
    root, report = pairs.verify_current_collection(collection)
    identity = report['identity']
    expected_rows = json.loads(Path(frozen['rows']['path']).read_text())
    p139.need(identity['rows'] == pairs.normalize_rows(expected_rows)
               and identity['archive_manifest'] == pairs.reference(freezing.ARCHIVE / 'manifest.json')
               and identity['repo'] == str(ROOT) and identity['harness'] == str(ROOT / 'tools/wtime-bench')
               and identity['seeds'] == freezing.SEEDS and identity['smoke'] is True
               and identity['pairs_per_row'] == 2 and report['pair_count'] == 8
               and report['timing_evidence'] is False and report['gate_eligible'] is False,
               'P142 smoke matrix or source context differs')
    before = pairs.digest(report)
    analyzed = p139.read_json(analysis_path, 'P142 analysis')
    p139.need(analyzed.get('schema') == analysis.SCHEMA and analyzed.get('status') == 'passed'
               and analyzed['method'] == analysis.METHOD and analyzed['metric'] == 'per_process_event_latency'
               and analyzed['report_sha256'] == before and analyzed['collection_identity_sha256'] == pairs.digest(identity)
               and analyzed['comparison'] == pairs.reference(root / 'report.json')
               and analyzed['input_refs'] == analysis._reference_map(report)
               and analyzed['timing_evidence'] is False and analyzed['gate_eligible'] is False
               and analyzed['decision'] is None and analyzed['threshold_selection'] is None
               and analyzed['scaled_by_calls'] is False and len(analyzed['runs']) == 2,
               'P142 analysis binding or smoke labels differ')
    sources = {'entry_sequence_analyze.py': analysis.__file__, 'entry_sequence_compare.py': pairs.__file__,
               'entry_sequence_policy.py': policy.__file__, 'analyze.py': analysis.statistics.__file__,
               'bootstrap.py': Path(analysis.statistics.__file__).with_name('bootstrap.py')}
    p139.need(set(analyzed['analysis_source_refs']) == set(sources), 'P142 analysis helper set differs')
    for name, source in sources.items():
        ref = analyzed['analysis_source_refs'][name]
        p139.need(ref == pairs.reference(analysis_path.parent / 'helpers' / name) and ref['sha256'] == p139.sha(Path(source)),
                   'P142 analysis helper changed')
    invocation_path = analysis_path.parent / 'invocation.json'
    p139.need(analyzed['invocation'] == pairs.reference(invocation_path), 'P142 analysis invocation changed')
    invocation = p139.read_json(invocation_path, 'analysis invocation')
    argv = invocation['argv']
    p139.need(invocation['cwd'] == str(ROOT) and len(argv) == 6
               and (ROOT / argv[0]).resolve() == Path(analysis.__file__).resolve()
               and argv[1] == '--comparison' and (ROOT / argv[2]).resolve() == root
               and argv[3] == '--output' and (ROOT / argv[4]).resolve() == analysis_path.parent.resolve()
               and argv[5] == '--allow-smoke', 'P142 analysis argv differs')
    groups = 0
    for run_id, run in enumerate(analyzed['runs']):
        p139.need(run['run_id'] == run_id and run['seed'] == freezing.SEEDS[run_id]
                   and len(run['rows']) == 2, 'P142 analyzed run differs')
        for row, settings in zip(run['rows'], identity['rows']):
            p139.need(row['row_id'] == settings['row_id'] and row['settings'] == settings
                       and row['process_pairs'] == 2, 'P142 analyzed row differs')
            classes = {policy.event_class(position, settings['schedule_threshold'])
                       for position in policy.schedule(settings['schedule_threshold'])}
            p139.need(len(row['classes']) == len(classes)
                       and {item['event_class'] for item in row['classes']} == classes, 'P142 event classes differ')
            for item in row['classes']:
                calls = sum(policy.event_class(position, settings['schedule_threshold']) == item['event_class']
                            for position in policy.schedule(settings['schedule_threshold']))
                p139.need(item['raw_call_count_per_process'] == calls and item['process_pairs'] == 2,
                           'P142 event sample unit differs')
                for side in pairs.SIDES:
                    side_stats = item[side]
                    p139.need(side_stats['process_count'] == 2 and side_stats['raw_call_count'] == 2 * calls
                               and side_stats['calls_per_process'] == calls, 'P142 process and call counts differ')
                p139.need(set(item['metrics']) == {'p50', 'p99', 'max'}, 'P142 process metric set differs')
                for metric in item['metrics'].values():
                    p139.need(metric['candidate_over_baseline'] is None and metric['ci95'] is None
                               and metric['bootstrap_resamples'] == 0 and metric['bootstrap_seed'] is None,
                               'P142 smoke manufactured uncertainty')
                groups += 1
    calls = sum(observed['payload']['actual_calls'] for run in report['runs'] for pair in run['pairs']
                for observed in pair['observations'].values())
    p139.need(calls == 8040 and groups == 24, 'P142 observed work differs')
    result = {'schema': 'wtime-p142-verification-v1', 'status': 'passed', 'source_stable': True,
              'outcome': 'contract_only', 'timing_evidence': False, 'gate_eligible': False,
              'candidate_promoted': False, 'decision': None, 'threshold_selection': None,
              'guest_or_build_launched': False, 'statistics_recomputed': False,
              'counts': {'runs': 2, 'rows': 2, 'pairs': 8, 'processes': 16, 'individual_calls': calls,
                         'event_groups': groups}, 'freeze': p139.ref(freeze_path),
              'collection': p139.ref(root / 'report.json'), 'analysis': p139.ref(analysis_path),
              'builds': {side: identity['sides'][side]['build_record'] for side in pairs.SIDES},
              'core_build_budgets': frozen['inputs']['p138_build_budgets']}
    p139.need(freezing.check(freeze_path) == frozen and pairs.verify_current_collection(collection)[1] == report
               and pairs.digest(report) == before, 'P142 inputs changed during proof validation')
    p139.write_new(output / 'report.json', result)
    return result

if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--freeze', type=Path, required=True)
    parser.add_argument('--collection', type=Path, required=True)
    parser.add_argument('--analysis', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    print(json.dumps({'status': verify(args.freeze, args.collection, args.analysis, args.output)['status'],
                      'timing_evidence': False}))
