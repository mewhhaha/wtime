#!/usr/bin/env python3
"""Combine two source-bound fuel-policy decisions without redoing statistics."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'tools/wtime-bench'))
import p127_fuel_policy


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def need(condition, message):
    if not condition:
        raise ValueError(message)


def read_decision(path, policy):
    path = Path(path).resolve(strict=True)
    record = json.loads(path.read_text())
    need(record.get('schema') == 'wtime-verification-statistics-v1'
         and record.get('task') == 'P053' and record.get('status') == 'passed'
         and record.get('statistics_computations') == 1
         and record.get('timing_evidence') is True,
         'P053 needs one completed statistics record per distinct policy dataset')
    for reference in record['inputs'].values():
        need(sha(reference['path']) == reference['sha256'], 'statistics input changed')
    report = json.loads(Path(record['inputs']['report']['path']).read_text())
    setting = p127_fuel_policy.policy(policy)
    need(report['metadata']['settings']['fuel_policy'] == setting,
         'statistics record has the wrong source-fuel policy')
    p127_fuel_policy.cli_value(report['metadata']['settings']['fuel_policy'])
    need(report['passed'] is True and report['source_stable'] is True
         and report['lane'] == 'candidate' and report['selection'] == 'P053',
         'policy dataset is not completed candidate evidence')
    decision = record['decision']['decision']
    need(decision in ('adopted', 'inconclusive', 'rejected'), 'unknown policy decision')
    return record, {'path': str(path), 'sha256': sha(path), 'policy': setting,
                    'report': record['inputs']['report'], 'decision': decision}


def combine(unmetered, metered):
    unmetered_record, unmetered_ref = read_decision(unmetered, 'unmetered')
    metered_record, metered_ref = read_decision(metered, 'metered')
    need(unmetered_ref['report']['sha256'] != metered_ref['report']['sha256'],
         'policies must have distinct datasets')
    for name in ('build', 'optional_build', 'contract', 'candidate_build', 'task_plan'):
        need(unmetered_record['inputs'].get(name) == metered_record['inputs'].get(name),
             'policy decisions consumed different candidate/build/declaration inputs')
    decisions = {unmetered_ref['decision'], metered_ref['decision']}
    decision = ('rejected' if 'rejected' in decisions else
                'inconclusive' if 'inconclusive' in decisions else 'adopted')
    return {'schema': 'wtime-p053-policy-decision-v1', 'task': 'P053',
            'status': 'passed', 'source_stable': True,
            'policy_records': [unmetered_ref, metered_ref],
            'statistics_computations': 2, 'additional_statistics_computations': 0,
            'decision': {'decision': decision,
                         'policy': 'both distinct policies must pass their complete candidate gate'},
            'timing_evidence': True}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--unmetered', type=Path, required=True)
    parser.add_argument('--metered', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    result = combine(args.unmetered, args.metered)
    with args.output.open('x') as stream:
        json.dump(result, stream, indent=2, sort_keys=True, allow_nan=False)
        stream.write('\n')
    print(json.dumps({'status': 'passed', 'decision': result['decision']['decision']}))
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
