import importlib.util
import json
import os
from pathlib import Path
import tempfile
import types
import unittest
import sys
from unittest import mock

TEST_PATH = Path(__file__).resolve()
SOURCE = Path(os.environ.get('WTIME_COMPARE_PATH', '')) if os.environ.get('WTIME_COMPARE_PATH') else None
if SOURCE is None:
    installed = TEST_PATH.parents[1] / 'compare.py'
    SOURCE = installed if installed.is_file() else TEST_PATH.with_name('compare.py')
BENCH_ROOT = SOURCE.resolve().parent
if not (BENCH_ROOT / 'workloads.py').is_file():
    BENCH_ROOT = Path('/workspace/wtime/tools/wtime-bench')
sys.path.insert(0, str(BENCH_ROOT))
spec = importlib.util.spec_from_file_location('p030_compare_stage', SOURCE)
compare = importlib.util.module_from_spec(spec)
spec.loader.exec_module(compare)
# The staged comparator lives under /tmp; bind its repository-relative inputs
# to the checkout just as the installed tests will.
compare.HERE = BENCH_ROOT
compare.ROOT = BENCH_ROOT.parents[1]

SIDES = [('baseline', 'baseline.exe', 'decoded'),
         ('candidate', 'candidate.exe', 'portable')]
FIXTURE = {'id': 'scalar_case', 'scalar': True}

class ScalarSharedCalibrationTests(unittest.TestCase):
    def calibrate(self, observations, **kwargs):
        seen = []
        def observe(exe, fixture, stage, batch, mode, cpu):
            seen.append((exe, stage, batch, mode, cpu))
            result = observations.pop(0)
            if callable(result):
                result = result(batch)
            return dict(result)
        with mock.patch.object(compare, '_observe', side_effect=observe):
            result = compare._calibrate_scalar_shared_batch(FIXTURE, SIDES, 4, **kwargs)
        return result, seen

    def test_rechecks_both_sides_after_growth_until_shared_interval_is_long_enough(self):
        # Auto estimates suggest batch 30; at that shared batch the faster side
        # is still short. Recheck both at the grown batch, then both meet the target.
        observations = [
            {'batch': 30, 'ns_per_operation': 800_000.0},
            {'batch': 24, 'ns_per_operation': 900_000.0},
            {'batch': 30, 'ns_per_operation': 410_000.0},
            {'batch': 30, 'ns_per_operation': 700_000.0},
            {'batch': 54, 'ns_per_operation': 410_000.0},
            {'batch': 54, 'ns_per_operation': 450_000.0},
        ]
        result, seen = self.calibrate(observations)
        self.assertEqual(result['batch'], 54)
        self.assertEqual([x[2] for x in seen], [0, 0, 30, 30, 54, 54])
        self.assertTrue(all(side['calibrated_total_ns'] >= 20_000_000
                            for side in result['sides']))
        self.assertEqual([attempt['batch'] for attempt in result['attempts']], [30, 54])
        self.assertEqual(len(result['attempts'][0]['sides']), 2)

    def test_growth_uses_fastest_observed_side_and_positive_batches_are_exact(self):
        observations = [
            {'batch': 10, 'ns_per_operation': 3_000_000.0},
            {'batch': 8, 'ns_per_operation': 2_000_000.0},
            {'batch': 10, 'ns_per_operation': 1_000_000.0},
            {'batch': 10, 'ns_per_operation': 1_000_000.0},
            {'batch': 22, 'ns_per_operation': 1_000_000.0},
            {'batch': 22, 'ns_per_operation': 1_000_000.0},
        ]
        result, seen = self.calibrate(observations)
        self.assertEqual(result['batch'], 22)
        self.assertEqual([row[2] for row in seen], [0, 0, 10, 10, 22, 22])

    def test_bad_timing_and_batch_echo_fail_closed(self):
        for value in (0.0, float('nan'), float('inf'), -1.0):
            with self.subTest(value=value):
                with self.assertRaisesRegex(compare.ComparisonError, 'finite and positive'):
                    self.calibrate([{'batch': 3, 'ns_per_operation': value},
                                    {'batch': 3, 'ns_per_operation': 1_000_000.0}])
        with self.assertRaisesRegex(compare.ComparisonError, 'differs from request'):
            self.calibrate([{'batch': 2, 'ns_per_operation': 1_000_000.0},
                            {'batch': 2, 'ns_per_operation': 1_000_000.0},
                            {'batch': 3, 'ns_per_operation': 1_000_000.0},
                            {'batch': 4, 'ns_per_operation': 1_000_000.0}])

    def test_cap_and_attempt_exhaustion_fail_closed(self):
        with self.assertRaisesRegex(compare.ComparisonError, 'did not reach'):
            self.calibrate([{'batch': 2, 'ns_per_operation': 1.0},
                            {'batch': 2, 'ns_per_operation': 1.0},
                            {'batch': 2, 'ns_per_operation': 1.0},
                            {'batch': 2, 'ns_per_operation': 1.0}], max_batch=2)
        # Each exact request is echoed but observed intervals stay below target.
        observations = [{'batch': 1, 'ns_per_operation': 1.0},
                        {'batch': 1, 'ns_per_operation': 1.0}]
        observations += [{'batch': 1, 'ns_per_operation': 1.0},
                        {'batch': 1, 'ns_per_operation': 1.0},
                        {'batch': 10, 'ns_per_operation': 1.0},
                        {'batch': 10, 'ns_per_operation': 1.0}]
        with self.assertRaisesRegex(compare.ComparisonError, 'did not reach'):
            self.calibrate(observations, max_batch=10, max_attempts=2)


    def test_invalid_batches_overflow_and_tiny_positive_timing_fail_safely(self):
        for invalid in (True, 0, 101):
            with self.subTest(invalid=invalid):
                with self.assertRaisesRegex(compare.ComparisonError, 'invalid scalar calibration batch'):
                    self.calibrate([{'batch': invalid, 'ns_per_operation': 1.0},
                                    {'batch': 2, 'ns_per_operation': 1.0}], max_batch=100)
        with self.assertRaisesRegex(compare.ComparisonError, 'interval must be finite'):
            self.calibrate([{'batch': 2, 'ns_per_operation': 1e308},
                            {'batch': 2, 'ns_per_operation': 1e308}])
        observations = [{'batch': 1, 'ns_per_operation': 5e-324},
                        {'batch': 2, 'ns_per_operation': 5e-324},
                        {'batch': 2, 'ns_per_operation': 5e-324},
                        {'batch': 2, 'ns_per_operation': 5e-324},
                        {'batch': 100, 'ns_per_operation': 5e-324},
                        {'batch': 100, 'ns_per_operation': 5e-324}]
        with self.assertRaisesRegex(compare.ComparisonError, 'calibration_audit=') as caught:
            self.calibrate(observations, max_batch=100, max_attempts=2)
        audit_text = str(caught.exception).split('calibration_audit=', 1)[1]
        audit = json.loads(audit_text)
        self.assertEqual([attempt['batch'] for attempt in audit['attempts']], [2, 100])

    def test_compare_run_retains_scalar_audit_and_pairs_with_same_oracle_batch(self):
        import baseline
        import checkpoint
        import scalar_workloads
        import workloads
        entry = next(row for row in workloads.selection('scalar')
                     if row['id'] == 'scalar_accumulator_n17')
        entry = dict(entry)
        entry['stages'] = ['warm_call']
        expected = scalar_workloads.oracle(entry['id'], 17)
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            archive = root / 'archive'; frozen = root / 'frozen'
            archive.mkdir(); frozen.mkdir()
            (archive / 'manifest.json').write_text('{}')
            (frozen / 'manifest.json').write_text('{}')
            baseline_exe = root / 'baseline-bin'; baseline_exe.write_bytes(b'baseline')
            candidate = root / 'candidate-bin'; candidate.write_bytes(b'candidate')
            output = root / 'result'
            fixture_hash = 'fixture-source-hash'
            def encode(_entry, directory, _encoder):
                fixture = Path(directory) / 'fixture.wasm'
                fixture.write_bytes(b'deterministic-encoded-test-fixture')
                return fixture
            estimates = {'baseline-bin': 12, 'candidate-bin': 9}
            observations = []
            def observe(exe, fixture, stage, batch, mode, cpu):
                side = Path(exe).name
                observations.append((side, stage, batch, mode))
                count = estimates[side] if batch == 0 else batch
                return {'batch': count, 'ns_per_operation': 2_000_000.0,
                        'input': 17, 'expected': expected, 'result': expected,
                        'mode': mode, 'wire_observation': {'result': f'i32:{expected}'}}
            class FakeJournal:
                def __init__(self, *args, **kwargs): pass
                @staticmethod
                def preflight(*args, **kwargs): pass
                def append(self, *args, **kwargs): pass
                def close(self): pass
                def write_report_once(self, report): pass
            protocol_capability = {'schema_version': 2, 'instrumented': False,
                                   'engine': 'wtime', 'modes': ['decoded'],
                                   'features': ['mvp']}
            args = types.SimpleNamespace(
                build_growth_explanation_file=None, selection='scalar', output=output,
                baseline=archive, candidate=candidate, candidate_mode='decoded',
                pairs=1, seed=23, cpu=sorted(os.sched_getaffinity(0))[0])
            baseline_info = {'frozen': frozen, 'executable': baseline_exe,
                             'mode': 'decoded', 'kind': 'current-source-archive'}
            with mock.patch.object(baseline, 'verify_archive', return_value={}), \
                 mock.patch.object(compare, '_baseline_identity', return_value=baseline_info), \
                 mock.patch.object(compare, '_verify_scalar_harness_sources'), \
                 mock.patch.object(workloads, 'selection', return_value=[entry]), \
                 mock.patch.object(compare, '_encode_scalar_fixture', side_effect=encode), \
                 mock.patch.object(compare, '_preflight_scalar_side'), \
                 mock.patch.object(compare, 'process', return_value=protocol_capability), \
                 mock.patch.object(compare.subprocess, 'check_output', return_value='rustc mock'), \
                 mock.patch.object(compare, '_observe', side_effect=observe), \
                 mock.patch.object(checkpoint.Journal, 'preflight'), \
                 mock.patch.object(checkpoint, 'Journal', FakeJournal):
                report = compare.run(args)
            self.assertTrue(report['passed'])
            policy = report['metadata']['settings']['scalar_warm_calibration']
            self.assertEqual(policy['policy'], 'shared_exact_batch_both_sides_confirmed')
            self.assertEqual(policy['typed_scalar_row_count'], 1)
            self.assertEqual(policy['legacy_control_count'], 0)
            self.assertEqual(policy['legacy_control_ids'], [])
            self.assertEqual(policy['scope'],
                             'selected typed scalar-driver rows and selected explicit legacy controls')
            self.assertEqual(policy['legacy_scalar_rows'],
                             'shared exact-batch calibration through --single-case; ordinary integer JSON oracle')
            self.assertEqual(len(report['warm_calibrations']), 2)
            for calibration in report['warm_calibrations']:
                self.assertEqual(calibration['paired_batch'], 12)
                self.assertEqual([row['batch'] for row in calibration['attempts']], [12])
                self.assertTrue(all(side['calibrated_total_ns'] >= 20_000_000
                                    for side in calibration['sides']))
            pairs = [pair for run in report['runs'] for pair in run['pairs']]
            self.assertEqual(len(pairs), 2)
            self.assertTrue(all(pair['expected'] == expected for pair in pairs))
            self.assertTrue(all(pair[side]['result'] == expected
                                for pair in pairs for side in ('baseline', 'candidate')))
            timed = [row for row in observations if row[2] > 0]
            self.assertTrue(all(row[2] == 12 for row in timed))

    def test_legacy_batch_selection_and_non_scalar_observation_paths_unchanged(self):
        rows = [{'batch': 64}, {'batch': 256}]
        self.assertEqual(compare._common_calibrated_batch({'scalar': True}, rows), 64)
        self.assertEqual(compare._common_calibrated_batch({'memory': True}, rows), 256)
        fixture = {'id': 'legacy', 'path': 'fixture.wasm', 'input': 1, 'expected': 2}
        with mock.patch.object(compare, 'observation', return_value={'batch': 1}) as legacy:
            self.assertEqual(compare._observe('exe', fixture, 'first_call', 1, 'decoded', 4),
                             {'batch': 1})
        legacy.assert_called_once_with('exe', 'fixture.wasm', 'legacy', 1, 2,
                                       'first_call', 1, 'decoded', 4)

if __name__ == '__main__':
    unittest.main()
