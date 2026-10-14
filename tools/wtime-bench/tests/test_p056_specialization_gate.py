"""Synthetic, contained checks for the P056 decision wrapper."""
from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]
PRODUCER = ROOT / 'tools/verification/producers/p056.py'
sys.path.insert(0, str(ROOT / 'tools'))
sys.path.insert(0, str(ROOT / 'tools/wtime-bench'))
from verification.artifacts import normal

spec = importlib.util.spec_from_file_location('p056_under_test', PRODUCER)
p056 = importlib.util.module_from_spec(spec)
spec.loader.exec_module(p056)


class P056SpecializationGateTests(unittest.TestCase):
    def contained(self, root: Path, relative: str) -> Path:
        path = (root / relative).resolve()
        self.assertTrue(path.is_relative_to(root.resolve()), path)
        return path

    def write_json(self, root: Path, relative: str, value) -> Path:
        path = self.contained(root, relative)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(value, sort_keys=True) + '\n')
        return path

    def stats_fixture(self, root: Path):
        p121 = self.write_json(root, 'inputs/p121.json', {'task': 'P121'})
        predecessor = self.write_json(root, 'inputs/structural-predecessor.json', {'task': 'P056'})
        inputs = {'p121': normal(p121), 'structural_predecessor': normal(predecessor)}
        for baseline in ('structural', 'p001'):
            for family in ('scalar', 'memory', 'simd', 'representative'):
                report = self.write_json(root, f'inputs/{baseline}-{family}.json',
                                         {'baseline': baseline, 'selection': family})
                inputs[f'report_{baseline}_{family}'] = normal(report)
        for baseline in ('structural', 'p001'):
            cold = self.write_json(root, f'inputs/cold-{baseline}.json', {'baseline': baseline})
            inputs[f'cold_{baseline}'] = normal(cold)
        statistical_refs = {}
        family_pairs = p056._family_pairs()
        portfolio_pairs = sum(family_pairs.values())
        for family in ('scalar', 'memory', 'simd', 'representative'):
            statistical_refs[f'decoded/{family}'] = inputs[f'report_structural_{family}']['sha256']
            statistical_refs[f'p001/{family}'] = inputs[f'report_p001_{family}']['sha256']
        decision = {
            'task': 'P056', 'status': 'passed', 'dataset_statistics_computations': 10,
            'portfolio_decision_computations': 1, 'decision': 'adopted',
            'matrix': {'status': 'passed', 'counts': {
                baseline: {family: {'pairs_total': family_pairs[family]} for family in
                           ('scalar', 'memory', 'simd', 'representative')}
                for baseline in ('p001', 'decoded')},
                'p001_pairs_total': portfolio_pairs,
                'decoded_pairs_total': portfolio_pairs},
            'p121': {'status': 'passed', 'executed': 10, 'selected': 10,
                     'optimized_source_events': 12, 'fallback_source_events': 8},
            'build': {'default_budget_pass': True, 'compact_budget_pass': True,
                      'hard_failures': []},
            'retained_code': {'status': 'passed'},
            'cold_statistics_by_report_sha256': {'p001': 'a', 'structural': 'b'},
            'statistics_by_report_sha256': statistical_refs,
        }
        decision['cold_statistics_by_report_sha256'] = {
            'p001': inputs['cold_p001']['sha256'],
            'structural': inputs['cold_structural']['sha256']}
        stats = {'schema': 'wtime-verification-statistics-v1', 'task': 'P056',
                 'status': 'passed', 'statistics_computations': 1,
                 'timing_evidence': True, 'decision': decision, 'inputs': inputs,
                 'analysis': {digest: {} for digest in
                              list(statistical_refs.values()) +
                              list(decision['cold_statistics_by_report_sha256'].values())}}
        path = self.write_json(root, 'inputs/statistics.json', stats)
        return path, stats

    def test_combined_regression_blocks_individually_adopted_variants(self):
        result = p056.choose_variants({'fusion_a': 'adopted', 'fusion_b': 'adopted'},
                                      'rejected')
        self.assertEqual(result['individually_supported'], ['fusion_a', 'fusion_b'])
        self.assertEqual(result['enabled_variants'], [])
        self.assertEqual(result['decision'], 'rejected')

    def test_inconclusive_combined_gate_keeps_individual_gains_unpromoted(self):
        result = p056.choose_variants({'fusion_a': 'adopted', 'fusion_b': 'adopted'},
                                      'inconclusive')
        self.assertEqual(result['enabled_variants'], [])
        self.assertEqual(result['decision'], 'inconclusive')

    def test_combined_adoption_without_variant_specific_support_stays_inconclusive(self):
        result = p056.choose_variants({}, 'adopted')
        self.assertEqual(result['enabled_variants'], [])
        self.assertEqual(result['decision'], 'inconclusive')

    def test_missing_cold_build_retained_or_source_accounting_evidence_fails(self):
        with tempfile.TemporaryDirectory(prefix='p056-gate-', dir='/workspace/wtime/target/performance') as temp:
            root = Path(temp).resolve()
            path, stats = self.stats_fixture(root)
            import p046_gate
            with patch.object(p046_gate, 'validate_p121', return_value={
                    'status': 'passed', 'optimized_source_events': 12,
                    'fallback_source_events': 8}), patch.object(
                    p046_gate, '_report_groups', side_effect=lambda report, family, baseline,
                    check_sources, structural_predecessor=None:
                    {'pairs_total': p056._family_pairs(p046_gate)[family]}):
                self.assertEqual(p056._verify_stats(path)[1]['task'], 'P056')
                for section, field in (('build', 'compact_budget_pass'),
                                       ('retained_code', 'status'),
                                       ('cold_statistics_by_report_sha256', 'p001')):
                    broken = json.loads(json.dumps(stats))
                    if section == 'cold_statistics_by_report_sha256':
                        del broken['decision'][section][field]
                    elif section == 'retained_code':
                        del broken['decision'][section][field]
                    else:
                        del broken['decision'][section][field]
                    bad_path = self.write_json(root, f'cases/missing-{section}.json', broken)
                    with self.assertRaises(ValueError):
                        p056._verify_stats(bad_path)
                source_path = Path(stats['inputs']['p121']['path'])
                source_path.write_text('{"task":"P121","changed":true}\n')
                with self.assertRaises(ValueError):
                    p056._verify_stats(path)
                broken = json.loads(json.dumps(stats))
                broken['decision']['p121']['optimized_source_events'] = 0
                bad_path = self.write_json(root, 'cases/zero-source-events.json', broken)
                with self.assertRaises(ValueError):
                    p056._verify_stats(bad_path)

    def test_missing_held_out_tier_coverage_fails(self):
        source = ROOT / 'tools/verification/producers/p056.py'
        base = {'passed': True, 'source_stable': True,
                'implementation_sources': {'tools/verification/producers/p056.py': p056.sha(source)},
                'counts': {'all_checks_expected': 4, 'all_checks_attempted': 4,
                           'all_checks_executed': 4, 'failed': 0, 'skipped': 0,
                           'unsupported': 0}}
        p056._validate_tier(base)
        stale = json.loads(json.dumps(base))
        stale['counts']['all_checks_executed'] = 3
        with self.assertRaises(ValueError):
            p056._validate_tier(stale)
        skipped = json.loads(json.dumps(base))
        skipped['counts']['skipped'] = 1
        with self.assertRaises(ValueError):
            p056._validate_tier(skipped)

    def test_historical_opt_in_and_removed_dispositions_are_enforced(self):
        with tempfile.TemporaryDirectory(prefix='p056-history-', dir='/workspace/wtime/target/performance') as temp:
            root = Path(temp).resolve()
            rows = []
            for task, (decision, disposition) in p056.HISTORY.items():
                record = self.write_json(root, f'history/{task}.json',
                                         {'schema': 'wtime-verification-evidence-v1',
                                          'task': task, 'status': 'passed', 'passed': True,
                                          'synthetic': False, 'candidate_promoted': False,
                                          'outcome': decision})
                rows.append({'task': task, 'decision': decision, 'disposition': disposition,
                             'record': normal(record)})
            result = p056._history(rows)
            self.assertEqual({t for t, row in result.items()
                              if row['disposition'] == 'retained_opt_in'}, {'P047', 'P050'})
            rows[0]['disposition'] = 'adopted'
            with self.assertRaises(ValueError):
                p056._history(rows)

    def test_stale_historical_record_is_rejected(self):
        with tempfile.TemporaryDirectory(prefix='p056-stale-', dir='/workspace/wtime/target/performance') as temp:
            root = Path(temp).resolve()
            rows = []
            for task, (decision, disposition) in p056.HISTORY.items():
                record = self.write_json(root, f'history/{task}.json', {
                    'schema': 'wtime-verification-evidence-v1', 'task': task,
                    'status': 'passed', 'passed': True, 'synthetic': False,
                    'candidate_promoted': False, 'outcome': decision})
                ref = normal(record)
                rows.append({'task': task, 'decision': decision, 'disposition': disposition,
                             'record': ref})
            (root / 'history/P047.json').write_text('{"task":"P047","changed":true}\n')
            with self.assertRaises(ValueError):
                p056._history(rows)

    def test_current_full_registry_pair_counts_include_all_memory_stages(self):
        import p046_gate
        pairs = p056._family_pairs(p046_gate)
        self.assertEqual(len(p046_gate._expected_groups('memory')), 181)
        self.assertEqual(pairs['memory'], 10_860)
        self.assertEqual(sum(pairs.values()), 71_100)


if __name__ == '__main__':
    unittest.main()
