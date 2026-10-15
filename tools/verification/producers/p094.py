"""Bind the native design no-go to completed portable cost evidence.

This review reuses finalized statistics; it does not collect samples or estimate
native speed. The finite native limits remain unmeasured design choices.
"""
from __future__ import annotations

import argparse
import math
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(ROOT / "tools"), str(Path(__file__).resolve().parent)]
from verification.artifacts import need, normal, read_json, verify_normal, write_new
import p094_policy
import p109
import p110_builds
from p110_finalize import completed_proof


def cost_inventory(statistics):
    """Summarize both existing runs without combining ratios or uncertainty."""
    result = {}
    for family, reference in sorted(statistics.items()):
        record = read_json(verify_normal(reference))
        summary = record.get('analysis') if 'decision' in record else record
        need(isinstance(summary, dict), 'portable statistics are missing: ' + family)
        protocol = summary.get('analysis', {})
        need(type(protocol.get('bootstrap_resamples')) is int
             and protocol['bootstrap_resamples'] >= 10_000,
             'portable uncertainty protocol is incomplete: ' + family)
        rows = summary.get('per_run', [])
        need(isinstance(rows, list) and rows, 'portable cost rows are empty: ' + family)
        runs = {}
        for row in rows:
            run = row.get('run')
            need(type(run) is int and run in (1, 2)
                 and type(row.get('pairs')) is int and row['pairs'] >= 30,
                 'portable cost row lacks a complete run: ' + family)
            interval = row.get('median_ratio_95ci', {})
            ratio = row.get('candidate_over_baseline', {}).get('median')
            low, high = interval.get('lower'), interval.get('upper')
            need(all(type(value) in (int, float) and math.isfinite(value) and value > 0
                     for value in (ratio, low, high)) and low <= high,
                 'portable cost ratio is invalid: ' + family)
            group = runs.setdefault(run, {'groups': 0, 'warm_groups': 0,
                'cold_groups': 0, 'ratio_minimum': ratio, 'ratio_maximum': ratio})
            group['groups'] += 1
            group['warm_groups'] += row.get('stage') in ('warm_call', 'steady_state_group')
            group['cold_groups'] += row.get('stage') in (
                'prepare', 'instantiate', 'first_call', 'load_and_call')
            group['ratio_minimum'] = min(group['ratio_minimum'], ratio)
            group['ratio_maximum'] = max(group['ratio_maximum'], ratio)
        need(set(runs) == {1, 2}, 'portable review needs both runs: ' + family)
        result[family] = {'statistics': reference, 'runs': runs,
            'scope': 'Range across distinct workload products; no aggregate speedup or native estimate.'}
    need(bool(result), 'portable cost inventory is empty')
    return result


def finalize(output):
    current = p109.sources()
    policy_path = ROOT / 'docs/architecture/native-policy-skeleton.json'
    policy = p094_policy.validate_policy(read_json(policy_path))
    proofs = {}
    prerequisites = {}
    for task in ('P067', 'P080'):
        proof, completion, reference = completed_proof(task)
        need(proof.get('timing_evidence') is True,
             'native decision requires completed portable measurements: ' + task)
        if task == 'P080':
            need(proof.get('core_sources') == current,
                 'specialist cost evidence is stale')
        else:
            # P067's tier reports are the explicit current runtime source binding.
            for name in ('eager', 'adaptive', 'decoded'):
                need(proof.get('tier_checks', {}).get(name),
                     'Register milestone omitted a checked tier')
        proofs[task] = proof
        prerequisites[task] = {'completion': completion, 'proof': reference}
    need(proofs['P067'].get('default_execution') == 'decoded'
         and proofs['P080'].get('retained_specialist_runtime_optimizations') == [],
         'portable policy differs from the native design review')
    need(proofs['P080'].get('p067', {}).get('proof') == prerequisites['P067']['proof'],
         'native review and source-current specialist gate refer to different Register milestones')
    costs = {task: cost_inventory(proof['statistics']) for task, proof in proofs.items()}
    builds = p110_builds.collect()
    need(p109.sources() == current, 'runtime sources changed during native design review')
    write_new(output, {
        'schema': 'wtime-p094-native-decision-v1', 'task': 'P094',
        'status': 'passed', 'decision': 'no_go', 'timing_evidence': False,
        'core_sources': current, 'prerequisites': prerequisites,
        'policy': normal(policy_path),
        'adr': normal(ROOT / 'docs/architecture/performance-native-adr.md'),
        'portable_cost_inventory': costs, 'current_builds': builds,
        'native_enabled': False, 'aot_enabled': False,
        'native_implemented': False, 'native_measurements': [],
        'descendants': policy['descendants'],
        'rationale': [
            'Existing portable comparisons are workload-specific; they do not establish a measured native benefit or native break-even point.',
            'No specialist trial met promotion with retained runtime changes. The selected release keeps one default interpreter and two explicit experimental tiers.',
            'Executable memory, emission correctness, ABI/lifetime review and an additional compiler would add implementation and verification work without an accepted native workload/comparator.',
            'Keep the small safe-Rust default and record finite optional boundaries for a future measured proposal.'
        ],
        'limitations': [
            'No native prototype was timed; no_go is a scope and architecture decision, not evidence that native execution is slower.',
            'Portable costs are reused from finalized paired statistics; ranges are across different products and are not an aggregate performance score.',
            'The finite resource ceilings are provisional design choices, not tested capacities.',
            'No unsafe scanner exception, native target support or AArch64 release completion is granted.'
        ]})


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    finalize(args.output)
