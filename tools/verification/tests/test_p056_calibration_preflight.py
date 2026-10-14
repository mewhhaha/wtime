"""Diagnostic-only P056 calibration preflight contracts."""
from __future__ import annotations

import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]
BENCH = ROOT / 'tools/wtime-bench'
PRODUCERS = ROOT / 'tools/verification/producers'
sys.dont_write_bytecode = True
sys.path.insert(0, str(BENCH))
sys.path.insert(0, str(PRODUCERS))

import compare
import focused_compare as focused
import p046_verify
import p056_calibrate as producer
import selection_manifest
import workloads


class P056CalibrationPreflightTests(unittest.TestCase):
    def test_exact_full_warm_membership(self):
        # Independently enumerate the declared workload and refresh selections.
        expected = []
        for family in (*p046_verify.FAMILIES, 'original-mvp'):
            for route in ('p001', 'structural'):
                for entry in workloads.selection(family):
                    if 'warm_call' in entry['stages']:
                        expected.append('/'.join((route, family, entry['id'], 'warm_call')))
        for task, path in producer.SELECTIONS.items():
            declaration = selection_manifest.load(path)
            selection_manifest.load_data(declaration)
            for entry in selection_manifest.rows(declaration):
                if 'warm_call' in entry['stages']:
                    expected.append('/'.join((task, task, entry['id'], 'warm_call')))
        actual = [producer.group_key(group) for group in producer.warm_groups()]
        self.assertEqual(len(expected), 490)
        self.assertEqual(actual, expected)
        self.assertEqual(len(actual), len(set(actual)))

    def test_fast_numeric_calibration_exceeds_100k_on_both_sides(self):
        entry = {'id': 'synthetic-numeric', 'oracle': {'module': 'run.py'}}
        group = {'route': 'synthetic', 'selection': 'synthetic', 'entry': entry}
        seen = []

        def observe(_group, side, batch):
            seen.append((side, batch))
            return {'batch': batch, 'ns_per_operation': 150.0}

        report = producer.calibrate_groups([group], observe, lambda _event: None)
        row = report['groups'][0]
        self.assertEqual(row['status'], 'passed')
        self.assertGreater(row['batch'], 100_000)
        self.assertGreaterEqual(row['batch'] * 150.0, producer.TARGET_NS)
        self.assertEqual([side for side, _ in seen[-2:]], ['baseline', 'candidate'])
        self.assertEqual({batch for _, batch in seen[-2:]}, {row['batch']})
        self.assertTrue(producer.validate_result(report, [group]))

    def test_capacity_failure_records_group_before_any_pairs(self):
        groups = [
            {'route': 'memory', 'selection': 'synthetic', 'entry': {
                'id': 'memory-a', 'oracle': {'module': 'memory_workloads.py'}}},
            {'route': 'memory', 'selection': 'synthetic', 'entry': {
                'id': 'memory-b', 'oracle': {'module': 'memory_workloads.py'}}},
        ]
        events = []
        observed = []

        def observe(group, side, batch):
            observed.append((producer.group_key(group), side, batch))
            return {'batch': batch, 'ns_per_operation': 1.0}

        report = producer.calibrate_groups(groups, observe, events.append)
        self.assertEqual([row['group'] for row in report['groups']],
                         [producer.group_key(group) for group in groups])
        self.assertEqual(report['attempted_group_count'], 2)
        self.assertEqual(report['failure_count'], 2)
        self.assertTrue(all(row['capacity'] == 100_000 and row['status'] == 'failed'
                            for row in report['groups']))
        for group in groups:
            key = producer.group_key(group)
            starts = [i for i, event in enumerate(events)
                      if event.get('event') == 'group_start' and event.get('group') == key]
            requests = [i for i, event in enumerate(events)
                        if event.get('event') == 'diagnostic_request' and event.get('group') == key]
            first_observation = next(i for i, item in enumerate(observed)
                                     if item[0] == key)
            self.assertEqual(len(starts), 1)
            self.assertTrue(requests)
            self.assertLess(starts[0], requests[0])
            self.assertEqual(observed[first_observation][1], 'baseline')
        self.assertEqual(report['paired_sample_events'], 0)
        self.assertEqual(report['statistics_computations'], 0)

    def test_batch_mismatch_and_nonfinite_timing_fail_closed(self):
        entry = {'id': 'numeric', 'oracle': {'module': 'run.py'}}
        with self.assertRaisesRegex(ValueError, 'batch mismatch'):
            focused.calibrate(entry, lambda _side, batch: {
                'batch': batch + (1 if _side == 'candidate' else 0),
                'ns_per_operation': 1.0,
            })
        for timing in (float('nan'), float('inf'), 0.0):
            with self.subTest(timing=timing), self.assertRaisesRegex(ValueError, 'invalid calibration timing'):
                focused.calibrate(entry, lambda _side, batch: {
                    'batch': batch, 'ns_per_operation': timing,
                })

    def test_stale_file_identity_rejected(self):
        with tempfile.TemporaryDirectory(prefix='p056-calibration-ref-') as temporary:
            root = Path(temporary).resolve()
            payload = root / 'receipt.json'
            self.assertTrue(payload.resolve().is_relative_to(root))
            payload.write_text('{"status":"fresh"}\n')
            reference = p046_verify.ref(payload)
            self.assertEqual(producer.checked_ref(reference), payload.resolve())
            self.assertTrue(payload.resolve().is_relative_to(root))
            payload.write_text('{"status":"mutated"}\n')
            with self.assertRaisesRegex(ValueError, 'immutable input changed'):
                producer.checked_ref(reference)

    def test_result_requires_exact_membership_and_confirmation(self):
        groups = [
            {'route': 'scalar', 'selection': 'synthetic', 'entry': {
                'id': 'numeric-a', 'oracle': {'module': 'run.py'}}},
            {'route': 'scalar', 'selection': 'synthetic', 'entry': {
                'id': 'numeric-b', 'oracle': {'module': 'run.py'}}},
        ]
        report = producer.calibrate_groups(
            groups, lambda _group, _side, batch: {
                'batch': batch, 'ns_per_operation': 20_000_000.0}, lambda _event: None)
        self.assertTrue(producer.validate_result(report, groups))
        mutations = []
        missing = json.loads(json.dumps(report))
        missing['groups'].pop()
        mutations.append(missing)
        changed_batch = json.loads(json.dumps(report))
        changed_batch['groups'][0]['confirmations'][-1]['batch'] += 1
        mutations.append(changed_batch)
        too_short = json.loads(json.dumps(report))
        too_short['groups'][0]['confirmations'][-1]['observations']['candidate']['ns_per_operation'] = 1.0
        mutations.append(too_short)
        for bad in mutations:
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                producer.validate_result(bad, groups)

    def test_real_driver_boundaries_preserve_memory_cap(self):
        entry = next(row for row in workloads.selection('simd')
                     if 'warm_call' in row['stages'])
        sides = {'baseline': (Path('/synthetic/baseline'), 'compact'),
                 'candidate': (Path('/synthetic/candidate'), 'compact')}
        with tempfile.TemporaryDirectory(prefix='p056-calibration-boundary-') as temporary:
            root = Path(temporary).resolve()
            output = root / 'output'
            self.assertTrue(output.resolve().is_relative_to(root))
            output.mkdir()
            overflow = compare.ComparisonError(
                'typed batch exceeds driver capacity 1000000')
            with patch.object(focused, 'observe', side_effect=lambda _binary, _entry, _fixture,
                              _stage, batch, _mode, _cpu, _output: {
                                  'batch': batch, 'ns_per_operation': 1.0}) as observe, \
                 patch.object(compare, 'process', side_effect=overflow) as process:
                result = producer.boundary_probes(sides, entry, 'synthetic-fixture.wasm',
                                                  0, output)
            self.assertEqual(observe.call_count, 2)
            self.assertEqual([call.args[4] for call in observe.call_args_list], [100_001, 100_001])
            self.assertEqual(process.call_count, 2)
            for call in process.call_args_list:
                self.assertEqual(int(call.args[1][-3]), 1_000_001)
            self.assertTrue(all(row['overflow_rejected'] for row in result.values()))
            self.assertEqual(focused.batch_limit({'oracle': {'module': 'memory_workloads.py'}}),
                             100_000)


if __name__ == '__main__':
    unittest.main()
