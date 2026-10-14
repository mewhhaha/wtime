#!/usr/bin/env python3
"""Validate and summarize the already-computed P056 specialization portfolio.

This is a decision wrapper only: it never launches the runtime or computes
timing statistics. Inputs are immutable reports produced by the P056 phases.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'tools'))
from verification.artifacts import ArtifactStore, need, normal, read_json, sha, verify_normal, write_new

TASK = 'P056'
SCHEMA = 'wtime-p056-specialization-input-v1'
HISTORY = {
    'P047': ('inconclusive', 'retained_opt_in'),
    'P048': ('rejected', 'removed'),
    'P049': ('rejected', 'removed'),
    'P050': ('inconclusive', 'retained_opt_in'),
    'P053': ('rejected', 'removed'),
    'P054': ('inconclusive', 'removed'),
    'P055': ('inconclusive', 'removed'),
}
MIN_PAIRS = 30


def _family_pairs(p046_gate=None):
    if p046_gate is None:
        bench = str(ROOT / 'tools' / 'wtime-bench')
        if bench not in sys.path:
            sys.path.insert(0, bench)
        import p046_gate as gate
        p046_gate = gate
    return {family: len(p046_gate._expected_groups(family)) * MIN_PAIRS * 2
            for family in p046_gate.FAMILIES}


def _read_ref(ref, label):
    path = verify_normal(ref)
    return path, read_json(path)


def _validate_tier(tier):
    need(tier.get('passed') is True and tier.get('source_stable') is True,
         'P056 tier report failed or is source unstable')
    counts = tier.get('counts')
    need(isinstance(counts, dict)
         and type(counts.get('all_checks_expected')) is int
         and counts['all_checks_expected'] > 0
         and counts.get('all_checks_executed') == counts['all_checks_expected']
         and counts.get('all_checks_attempted') == counts['all_checks_expected']
         and counts.get('failed') == 0 and counts.get('skipped') == 0
         and counts.get('unsupported') == 0,
         'P056 tier report omits full semantic/held-out check coverage')
    sources = tier.get('implementation_sources')
    need(isinstance(sources, dict) and sources, 'P056 tier source accounting is empty')
    for name, expected in sources.items():
        path = (ROOT / name).resolve(strict=True)
        need(path.is_file() and path.is_relative_to(ROOT)
             and sha(path) == expected, f'P056 tier source is stale: {name}')
    return sources


def choose_variants(individual_decisions, combined_decision):
    """Only promote independently adopted variants if the combined gate adopts."""
    need(isinstance(individual_decisions, dict), 'individual decisions must be a map')
    need(combined_decision in ('adopted', 'inconclusive', 'rejected'),
         'combined decision is unknown')
    need(all(value in ('adopted', 'inconclusive', 'rejected')
             for value in individual_decisions.values()),
         'individual variant decision is unknown')
    supported = sorted(name for name, decision in individual_decisions.items()
                       if decision == 'adopted')
    enabled = supported if combined_decision == 'adopted' else []
    outcome = ('adopted' if enabled else
               'inconclusive' if combined_decision == 'adopted' else combined_decision)
    return {'enabled_variants': enabled,
            'decision': outcome,
            'combined_decision': combined_decision,
            'individually_supported': supported}


def _verify_stats(stats_path):
    stats_path = Path(stats_path).resolve(strict=True)
    stats = read_json(stats_path)
    need(stats.get('schema') == 'wtime-verification-statistics-v1'
         and stats.get('task') == TASK and stats.get('status') == 'passed'
         and stats.get('statistics_computations') == 1
         and stats.get('timing_evidence') is True,
         'P056 needs one completed shared portfolio statistics record')
    decision = stats.get('decision')
    need(isinstance(decision, dict) and decision.get('task') == TASK
         and decision.get('status') == 'passed'
         and decision.get('dataset_statistics_computations') == 10
         and decision.get('portfolio_decision_computations') == 1,
         'P056 portfolio decision record is incomplete or recomputed')
    matrix = decision.get('matrix')
    need(isinstance(matrix, dict) and matrix.get('status') == 'passed',
         'complete P046 four-family matrix is missing')
    bench = str(ROOT / 'tools' / 'wtime-bench')
    if bench not in sys.path:
        sys.path.insert(0, bench)
    import p046_gate
    family_pairs = _family_pairs(p046_gate)
    portfolio_pairs = sum(family_pairs.values())
    counts = matrix.get('counts')
    need(isinstance(counts, dict) and set(counts) == {'p001', 'decoded'},
         'P056 matrix must include both baselines')
    for baseline in ('p001', 'decoded'):
        need(isinstance(counts[baseline], dict)
             and set(counts[baseline]) == {'scalar', 'memory', 'simd', 'representative'},
             f'{baseline} portfolio family coverage is incomplete')
        for family, row in counts[baseline].items():
            need(isinstance(row, dict) and type(row.get('pairs_total')) is int
                 and row['pairs_total'] == family_pairs[family],
                 f'{baseline}/{family} is not the complete predeclared portfolio')
    need(matrix.get('p001_pairs_total') == portfolio_pairs
         and matrix.get('decoded_pairs_total') == portfolio_pairs,
         'P056 is missing full decoded and P001 portfolio totals')
    inputs = stats.get('inputs')
    need(isinstance(inputs, dict), 'P056 statistics inputs are missing')
    structural_predecessor = inputs.get('structural_predecessor')
    need(isinstance(structural_predecessor, dict),
         'P056 frozen structural predecessor binding is missing')
    verify_normal(structural_predecessor)
    for baseline in ('structural', 'p001'):
        for family in ('scalar', 'memory', 'simd', 'representative'):
            name = f'report_{baseline}_{family}'
            need(name in inputs, f'P056 full {baseline}/{family} report is missing')
            path = verify_normal(inputs[name])
            report = read_json(path)
            checked = p046_gate._report_groups(
                report, family, 'decoded' if baseline == 'structural' else baseline,
                check_sources=True,
                structural_predecessor=(structural_predecessor
                                        if baseline == 'structural' else None))
            need(checked['pairs_total'] == family_pairs[family],
                 f'P056 {baseline}/{family} report has incomplete groups or held-outs')
    statistical_refs = decision.get('statistics_by_report_sha256')
    need(isinstance(statistical_refs, dict) and len(statistical_refs) == 8,
         'P056 dataset statistics do not bind all eight portfolio reports')
    report_identities = {}
    for family in ('scalar', 'memory', 'simd', 'representative'):
        for baseline, prefix in (('structural', 'decoded'), ('p001', 'p001')):
            ref = inputs[f'report_{baseline}_{family}']
            report_identities[f'{prefix}/{family}'] = ref['sha256']
    need(statistical_refs == report_identities,
         'P056 computed portfolio identities differ from the source-current reports')
    cold_identities = decision.get('cold_statistics_by_report_sha256')
    need(isinstance(cold_identities, dict)
         and cold_identities == {
             'p001': inputs.get('cold_p001', {}).get('sha256'),
             'structural': inputs.get('cold_structural', {}).get('sha256')},
         'P056 cold-control statistics are missing or stale')
    analyses = stats.get('analysis')
    need(isinstance(analyses, dict)
         and set(analyses) == set(report_identities.values()) | set(cold_identities.values())
         and len(analyses) == 10,
         'P056 analysis does not contain exactly the ten bound datasets')
    p121 = decision.get('p121')
    need(isinstance(p121, dict) and p121.get('status') == 'passed'
         and type(p121.get('executed')) is int and p121['executed'] > 0
         and type(p121.get('selected')) is int and p121['selected'] == p121['executed']
         and type(p121.get('optimized_source_events')) is int
         and p121['optimized_source_events'] > 0
         and type(p121.get('fallback_source_events')) is int
         and p121['fallback_source_events'] > 0,
         'source accounting coverage is missing or empty')
    p121_ref = inputs.get('p121')
    need(isinstance(p121_ref, dict), 'P056 source-bound P121 input is missing')
    checked_p121 = p046_gate.validate_p121(p121_ref, check_sources=True)
    need(checked_p121.get('status') == 'passed'
         and checked_p121.get('optimized_source_events') == p121.get('optimized_source_events')
         and checked_p121.get('fallback_source_events') == p121.get('fallback_source_events'),
         'P056 source accounting differs from the source-current P121 producer')
    need(isinstance(decision.get('build'), dict)
         and decision['build'].get('default_budget_pass') is True
         and decision['build'].get('compact_budget_pass') is True
         and decision['build'].get('hard_failures') == [],
         'default/Compact build evidence is missing or failed')
    need(isinstance(decision.get('retained_code'), dict)
         and decision['retained_code'].get('status') == 'passed',
         'retained-code budget evidence is missing')
    need(isinstance(decision.get('cold_statistics_by_report_sha256'), dict)
         and set(decision['cold_statistics_by_report_sha256']) == {'p001', 'structural'},
         'both cold-control statistics are required')
    need(inputs, 'P056 statistics inputs are missing')
    for name, ref in inputs.items():
        verify_normal(ref)
    return stats_path, stats


def _history(rows):
    need(isinstance(rows, list), 'historical variant records are missing')
    by_task = {}
    for row in rows:
        need(isinstance(row, dict), 'historical variant record must be an object')
        task = row.get('task')
        need(task in HISTORY and task not in by_task, 'unknown or duplicate historical variant')
        expected_decision, expected_disposition = HISTORY[task]
        need(row.get('decision') == expected_decision
             and row.get('disposition') == expected_disposition,
             f'{task} historical outcome/disposition differs from its published result')
        _, record = _read_ref(row.get('record'), f'{task} historical record')
        need(record.get('schema') == 'wtime-verification-evidence-v1'
             and record.get('task') == task and record.get('status') == 'passed'
             and record.get('passed') is True and record.get('synthetic') is False
             and record.get('candidate_promoted') is False
             and record.get('outcome') == expected_decision,
             f'{task} immutable published outcome evidence differs')
        if 'refresh' in row:
            _, refresh = _read_ref(row['refresh'], f'{task} refreshed measurement')
            need(refresh.get('schema') == 'wtime-p056-variant-refresh-v1'
                 and refresh.get('task') == task and refresh.get('status') == 'passed'
                 and refresh.get('source_stable') is True
                 and refresh.get('instrumented') is False,
                 f'{task} refreshed measurement is incomplete or not production-bound')
        by_task[task] = row
    need(set(by_task) == set(HISTORY), 'historical P047-P055 variant coverage is incomplete')
    return by_task


def _individual_refreshes(reference):
    _, record = _read_ref(reference, 'P056 individual variant statistics')
    need(record.get('schema') == 'wtime-p056-variant-statistics-v1'
         and record.get('task') == TASK and record.get('status') == 'passed'
         and record.get('dataset_statistics_computations') == 2
         and record.get('additional_bootstrap_computations') == 2
         and record.get('promotion_scope') == 'diagnostic refresh; historical individual outcomes unchanged',
         'P056 individual refresh statistics are incomplete or have a promotion claim')
    input_refs = record.get('inputs')
    need(isinstance(input_refs, dict)
         and set(input_refs) == {'measurements', 'analyzer', 'bootstrap', 'validator', 'producer'},
         'P056 variant statistics inputs are incomplete')
    for name, ref in input_refs.items():
        verify_normal(ref)
    _, refresh_set = _read_ref(input_refs['measurements'], 'P056 raw refresh set')
    need(refresh_set.get('schema') == 'wtime-p056-variant-refresh-set-v1'
         and refresh_set.get('status') == 'passed'
         and refresh_set.get('additional_statistics_computations') == 0
         and set(refresh_set.get('reports', {})) == {'P047', 'P050'},
         'P056 raw refresh set is incomplete')
    variants = record.get('reports')
    need(isinstance(variants, dict) and set(variants) == {'P047', 'P050'},
         'P056 must refresh the complete P047/P050 comparison set')
    for task, row in variants.items():
        need(isinstance(row, dict), f'{task} refresh statistics are missing')
        raw_path, report = _read_ref(row.get('raw'), f'{task} refreshed raw comparison')
        need(row['raw'] == refresh_set['reports'][task],
             f'{task} analyzed report differs from the validated refresh-set input')
        producer_dir = str(ROOT / 'tools/verification/producers')
        if producer_dir not in sys.path:
            sys.path.insert(0, producer_dir)
        import p056_variants
        checked = p056_variants.validate_raw_report(report, raw_path, check_sources=True)
        need(checked.get('task') == task and checked.get('selection') == task
             and checked.get('report_sha256') == row['raw']['sha256']
             and checked.get('source_stable') is True,
             f'{task} source-bound refresh report identity differs')
        need(isinstance(row.get('statistics'), dict),
             f'{task} already-computed refresh statistics are missing')
        need(row.get('report_sha256') == checked['report_sha256']
             and row.get('report_sha256') == row['raw']['sha256'],
             f'{task} refresh analysis is not bound to this raw report')
        need(row['statistics'].get('metadata') == report.get('metadata'),
             f'{task} refresh analysis metadata differs from its raw report')
    return record


def evaluate(statistics_path, evidence_path, output_path):
    stats_path, stats = _verify_stats(statistics_path)
    evidence_path = Path(evidence_path).resolve(strict=True)
    evidence = read_json(evidence_path)
    need(evidence.get('schema') == SCHEMA and evidence.get('task') == TASK
         and evidence.get('status') == 'passed', 'P056 evidence binding is invalid')
    need(evidence.get('optional_unimplemented') == ['P051', 'P052'],
         'P056 optional unimplemented history must remain explicit')
    need(evidence.get('statistics') == normal(stats_path),
         'P056 evidence does not bind the supplied statistics record')
    history = _history(evidence.get('historical_variants'))
    variants_ref = evidence.get('individual_variants')
    _individual_refreshes(variants_ref)
    _, tier = _read_ref(evidence.get('tier'), 'P056 tier report')
    _validate_tier(tier)
    _, receipt = _read_ref(evidence.get('verification'), 'P056 verification receipt')
    need(receipt.get('schema') == 'wtime-verification-phase-v1'
         and receipt.get('task') == TASK and receipt.get('phase') == 'verify'
         and receipt.get('status') == 'passed' and receipt.get('lane') == 'full'
         and receipt.get('completion_eligible') is True and receipt.get('synthetic') is False,
         'P056 requires a completed source-current full verification receipt')
    store_root = next((parent for parent in Path(evidence['verification']['path']).parents
                       if parent.name == 'verification' and parent.parent.name == 'target'), None)
    need(store_root is not None, 'P056 receipt is outside the verification artifact store')
    store = ArtifactStore(store_root)
    for artifact in receipt.get('artifacts', {}).values():
        store.verify(artifact)
    from verification.artifacts import digest
    need(isinstance(receipt.get('inputs'), dict)
         and digest(receipt['inputs']) == receipt.get('input_key'),
         'P056 verification receipt input key is stale')
    for command in receipt.get('commands', []):
        need(command.get('exit_code') == 0, 'P056 verification command failed')
        store.verify(command['stdout'])
        store.verify(command['stderr'])
    outputs = receipt.get('original_outputs')
    need(isinstance(outputs, dict), 'P056 verification output identities are missing')
    for reference in outputs.values():
        verify_normal(reference)
    need(outputs.get('tier') == evidence.get('tier'),
         'P056 tier report differs from the output consumed by verification')
    commands = {row.get('name'): row for row in receipt.get('commands', [])}
    definition = receipt.get('inputs', {}).get('definition', {})
    from verification.runner import test_results
    for group in definition.get('tests', []):
        command = commands.get(group.get('command'))
        need(isinstance(command, dict) and command.get('exit_code') == 0,
             'P056 semantic test command did not pass')
        stdout = store.read(command['stdout']).decode(errors='replace')
        stderr = store.read(command['stderr']).decode(errors='replace')
        parsed = test_results(stdout + '\n' + stderr, group)
        need(parsed == receipt.get('tests', {}).get(group['command']),
             'P056 semantic test record differs from its immutable logs')
    combined = stats['decision'].get('decision')
    need(combined in ('adopted', 'inconclusive', 'rejected'), 'portfolio decision is unknown')
    selected = choose_variants({}, combined)
    # Historical experiments remain opt-in or removed; P056 never reclassifies them.
    opt_in = sorted(task for task, row in history.items()
                    if row['disposition'] == 'retained_opt_in')
    removed = sorted(task for task, row in history.items() if row['disposition'] == 'removed')
    output = {
        'schema': 'wtime-p056-specialization-decision-v1', 'task': TASK,
        'status': 'passed', 'source_stable': True,
        'decision': {'decision': selected['decision'],
                     'portfolio_decision': combined,
                     'policy': 'only variants individually adopted and compatible with the combined gate may be enabled',
                     'enabled_variants': selected['enabled_variants']},
        'retained_opt_in': opt_in, 'removed_variants': removed,
        'optional_unimplemented': evidence['optional_unimplemented'],
        'inputs': {'statistics': normal(stats_path), 'evidence': normal(evidence_path),
                   'tier': evidence['tier'], 'verify_receipt': evidence['verification'],
                   'individual_variants': variants_ref},
        'statistics_computations': 1, 'additional_statistics_computations': 0,
        'timing_evidence': True,
    }
    write_new(output_path, output)
    return output


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--statistics', required=True, type=Path)
    parser.add_argument('--evidence', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    result = evaluate(args.statistics, args.evidence, args.output)
    print(json.dumps({'status': result['status'], 'decision': result['decision']['decision']}))


if __name__ == '__main__':
    main()
