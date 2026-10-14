import contextlib
import importlib.util
import io
import os
from pathlib import Path
import sys
import tempfile
import types
import unittest
from unittest import mock

TEST_PATH = Path(__file__).resolve()
SOURCE = Path(os.environ['WTIME_COMPARE_PATH']) if os.environ.get('WTIME_COMPARE_PATH') else None
if SOURCE is None:
    installed = TEST_PATH.parents[1] / 'compare.py'
    SOURCE = installed if installed.is_file() else TEST_PATH.with_name('compare.py')
BENCH_ROOT = SOURCE.resolve().parent
if not (BENCH_ROOT / 'workloads.py').is_file():
    BENCH_ROOT = Path('/workspace/wtime/tools/wtime-bench')
sys.path.insert(0, str(BENCH_ROOT))
spec = importlib.util.spec_from_file_location('p031_compare_stage', SOURCE)
compare = importlib.util.module_from_spec(spec)
spec.loader.exec_module(compare)
compare.HERE = BENCH_ROOT
compare.ROOT = BENCH_ROOT.parents[1]

import baseline
import checkpoint
import run as original
import scalar_workloads
import workloads

LEGACY = {'call_overhead': (100, 'warm result'),
          'integer_loop': (10000, 'loop result'),
          'branch_calls': (5000, 'branch result')}
SIDES = [('baseline', 'baseline.exe', 'decoded'),
         ('candidate', 'candidate.exe', 'portable')]

class LegacyCalibrationTests(unittest.TestCase):
    def test_legacy_single_case_positive_batch_keeps_cli_and_integer_oracle(self):
        for case, (input_value, _label) in LEGACY.items():
            with self.subTest(case=case):
                expected = {'call_overhead': 101, 'integer_loop': 123456,
                            'branch_calls': 5555}[case]
                response = {'schema_version': 2, 'case': case, 'stage': 'warm_call',
                            'input': input_value, 'expected': expected, 'result': expected,
                            'batch': 37, 'ns_per_operation': 900_000.0, 'mode': 'decoded'}
                with mock.patch.object(compare, 'process', return_value=response) as process:
                    row = compare.observation('legacy.exe', '/frozen/case.wasm', case,
                                              input_value, expected, 'warm_call', 37,
                                              'decoded', 4)
                process.assert_called_once_with('legacy.exe',
                    ['--single-case', '/frozen/case.wasm', case, input_value,
                     expected, 'warm_call', 37, 'decoded'], 4)
                self.assertEqual(row['result'], expected)
                self.assertEqual(row['batch'], 37)
                bad = dict(response, result=expected + 1)
                with mock.patch.object(compare, 'process', return_value=bad):
                    with self.assertRaisesRegex(compare.ComparisonError, 'oracle mismatch'):
                        compare.observation('legacy.exe', '/frozen/case.wasm', case,
                                            input_value, expected, 'warm_call', 37,
                                            'decoded', 4)

    def test_legacy_rows_share_exact_batch_after_both_side_confirmations(self):
        for case, (input_value, _label) in LEGACY.items():
            with self.subTest(case=case):
                fixture = {'id': case, 'scalar': False, 'memory': False,
                           'input': input_value, 'expected': 77}
                calls = []
                estimates = iter([12, 9])
                def observe(exe, got_fixture, stage, batch, mode, cpu):
                    calls.append((exe, got_fixture['id'], stage, batch, mode))
                    count = next(estimates) if batch == 0 else batch
                    return {'batch': count, 'ns_per_operation': 2_000_000.0,
                            'result': fixture['expected']}
                with mock.patch.object(compare, '_observe', side_effect=observe):
                    result = compare._calibrate_scalar_shared_batch(fixture, SIDES, 4)
                self.assertEqual(result['batch'], 12)
                self.assertEqual([row[3] for row in calls], [0, 0, 12, 12])
                self.assertTrue(all(row['calibrated_total_ns'] >= 20_000_000
                                    for row in result['sides']))
                self.assertEqual(calls[0][1], case)

    def test_only_typed_rows_and_explicit_legacy_ids_are_admitted_and_caps_fail_closed(self):
        self.assertTrue(compare._is_calibratable_scalar_fixture(
            {'id': 'scalar_test', 'scalar': True, 'memory': False}))
        for invalid in ({'id': 'scalar_malformed', 'scalar': True, 'memory': True},
                        {'id': 'new_legacy', 'scalar': False, 'memory': False},
                        {'id': 'call_overhead', 'scalar': False, 'memory': True},
                        {'id': 'call_overhead', 'scalar': None, 'memory': False},
                        {'id': 'call_overhead', 'scalar': False, 'memory': None}):
            self.assertFalse(compare._is_calibratable_scalar_fixture(invalid))
        with self.assertRaisesRegex(compare.ComparisonError, 'explicit legacy control'):
            compare._calibrate_scalar_shared_batch(
                {'id': 'not_a_control', 'scalar': False, 'memory': False}, SIDES, 4)
        fixture = {'id': 'integer_loop', 'scalar': False, 'memory': False}
        rows = [{'batch': 1, 'ns_per_operation': 1.0},
                {'batch': 1, 'ns_per_operation': 1.0},
                {'batch': 1, 'ns_per_operation': 1.0},
                {'batch': 1, 'ns_per_operation': 1.0},
                {'batch': 10, 'ns_per_operation': 1.0},
                {'batch': 10, 'ns_per_operation': 1.0}]
        def observe(*args): return rows.pop(0)
        with mock.patch.object(compare, '_observe', side_effect=observe):
            with self.assertRaisesRegex(compare.ComparisonError, 'calibration_audit=') as error:
                compare._calibrate_scalar_shared_batch(fixture, SIDES, 4,
                                                       max_batch=10, max_attempts=2)
        self.assertIn('"batch": 10', str(error.exception))

    def test_legacy_only_selection_reports_policy_and_six_records(self):
        selected = [dict(row, stages=['warm_call']) for row in workloads.selection('scalar')
                    if row['id'] in LEGACY]
        self.assertEqual({row['id'] for row in selected}, set(LEGACY))
        expected_by_id = {row['id']: original.oracle(
            row['id'], row['params'][0]['baseline']) for row in selected}
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            archive = root / 'archive'; frozen = root / 'frozen'
            archive.mkdir(); frozen.mkdir()
            (archive / 'manifest.json').write_text('{}')
            (frozen / 'manifest.json').write_text('{}')
            baseline_exe = root / 'baseline-bin'; baseline_exe.write_bytes(b'baseline')
            candidate = root / 'candidate-bin'; candidate.write_bytes(b'candidate')
            output = root / 'result'
            expected_hash_by_path = {}
            for row in selected:
                path = frozen / 'fixtures' / (row['id'] + '.wasm')
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(row['id'].encode())
                expected_hash_by_path[path.resolve()] = row['source']['wasm_sha256']
            real_sha = compare.sha
            def bound_sha(path):
                resolved = Path(path).resolve()
                return expected_hash_by_path.get(resolved, real_sha(path))
            calls = []
            def process(exe, args, cpu):
                args = list(args)
                calls.append((Path(exe).name, args))
                if args == ['--protocol']:
                    return {'schema_version': 2, 'engine': 'wtime',
                            'instrumented': False, 'modes': ['decoded'],
                            'features': ['mvp']}
                self.assertEqual(args[0], '--single-case')
                _, _fixture, case, input_value, expected, stage, requested, mode = args
                requested_count = int(requested)
                count = (12 if Path(exe).name == 'baseline-bin' else 9) if requested_count == 0 else requested_count
                return {'schema_version': 2, 'case': case, 'stage': stage,
                        'input': input_value, 'expected': expected,
                        'result': expected, 'batch': count,
                        'ns_per_operation': 2_000_000.0, 'mode': mode}
            class FakeJournal:
                def __init__(self, *args, **kwargs): pass
                @staticmethod
                def preflight(*args, **kwargs): pass
                def append(self, *args, **kwargs): pass
                def close(self): pass
                def write_report_once(self, report): pass
            baseline_info = {'frozen': frozen, 'executable': baseline_exe,
                             'mode': 'decoded', 'kind': 'current-source-archive'}
            args = types.SimpleNamespace(build_growth_explanation_file=None,
                selection='scalar', output=output, baseline=archive, candidate=candidate,
                candidate_mode='decoded', pairs=1, seed=37,
                cpu=sorted(os.sched_getaffinity(0))[0])
            with mock.patch.object(baseline, 'verify_archive', return_value={}), \
                 mock.patch.object(compare, '_baseline_identity', return_value=baseline_info), \
                 mock.patch.object(workloads, 'selection', return_value=selected), \
                 mock.patch.object(compare, 'process', side_effect=process), \
                 mock.patch.object(compare, 'sha', side_effect=bound_sha), \
                 mock.patch.object(compare.subprocess, 'check_output', return_value='rustc mock'), \
                 mock.patch.object(checkpoint.Journal, 'preflight'), \
                 mock.patch.object(checkpoint, 'Journal', FakeJournal):
                with contextlib.redirect_stdout(io.StringIO()):
                    report = compare.run(args)
            self.assertTrue(report['passed'])
            policy = report['metadata']['settings']['scalar_warm_calibration']
            self.assertEqual(policy['typed_scalar_row_count'], 0)
            self.assertEqual(policy['legacy_control_count'], 3)
            self.assertEqual(policy['legacy_control_ids'],
                             ['branch_calls', 'call_overhead', 'integer_loop'])
            records = report['warm_calibrations']
            self.assertEqual(len(records), 6)
            self.assertTrue(all(row['workload_kind'] == 'legacy_single_case_control'
                                for row in records))
            self.assertTrue(all(row['paired_batch'] == 12 for row in records))
            self.assertTrue(all(row[side]['result'] == expected_by_id[row['workload']]
                                for run in report['runs'] for row in run['pairs']
                                for side in ('baseline', 'candidate')))
            self.assertTrue(all(call[1][0] == '--single-case' for call in calls
                                if call[1] != ['--protocol']))

    def test_scalar_run_emits_170_bounded_calibrations_with_both_row_kinds(self):
        selected = [dict(row, stages=['warm_call'])
                    for row in workloads.selection('scalar')]
        self.assertEqual(len(selected), 85)
        expected_by_id = {}
        for row in selected:
            n = row['params'][0]['baseline'] if row['params'] else 0
            expected_by_id[row['id']] = (scalar_workloads.oracle(row['id'], n)
                if row.get('family', '').startswith('scalar_')
                else original.oracle(row['id'], n))
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            archive = root / 'archive'; frozen = root / 'frozen'
            archive.mkdir(); frozen.mkdir()
            (archive / 'manifest.json').write_text('{}')
            (frozen / 'manifest.json').write_text('{}')
            baseline_exe = root / 'baseline-bin'; baseline_exe.write_bytes(b'baseline')
            candidate = root / 'candidate-bin'; candidate.write_bytes(b'candidate')
            output = root / 'result'
            expected_hash_by_path = {}
            def encode(entry, directory, _encoder):
                fixture = Path(directory) / (entry['id'] + '.wasm')
                fixture.write_bytes(('fixture:' + entry['id']).encode())
                expected_hash_by_path[fixture.resolve()] = entry['source']['wasm_sha256']
                return fixture
            for row in selected:
                if not row.get('family', '').startswith('scalar_'):
                    path = frozen / 'fixtures' / (row['id'] + '.wasm')
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_bytes(('legacy:' + row['id']).encode())
                    expected_hash_by_path[path.resolve()] = row['source']['wasm_sha256']
            real_sha = compare.sha
            def bound_sha(path):
                resolved = Path(path).resolve()
                return expected_hash_by_path.get(resolved, real_sha(path))
            auto_batches = {'baseline-bin': 12, 'candidate-bin': 9}
            def observe(exe, fixture, stage, batch, mode, cpu):
                side = Path(exe).name
                count = auto_batches[side] if batch == 0 else batch
                expected = fixture['expected']
                return {'batch': count, 'ns_per_operation': 2_000_000.0,
                        'input': fixture['input'], 'expected': expected, 'result': expected,
                        'mode': mode, 'wire_observation': {'result': expected}}
            class FakeJournal:
                def __init__(self, *args, **kwargs): pass
                @staticmethod
                def preflight(*args, **kwargs): pass
                def append(self, *args, **kwargs): pass
                def close(self): pass
                def write_report_once(self, report): pass
            profile_set = sorted({row['driver']['profile'] for row in selected if row.get('driver')})
            capabilities = {'schema_version': 2, 'instrumented': False, 'engine': 'wtime',
                            'modes': ['decoded'], 'features': profile_set}
            baseline_info = {'frozen': frozen, 'executable': baseline_exe,
                             'mode': 'decoded', 'kind': 'current-source-archive'}
            args = types.SimpleNamespace(build_growth_explanation_file=None,
                selection='scalar', output=output, baseline=archive, candidate=candidate,
                candidate_mode='decoded', pairs=1, seed=31,
                cpu=sorted(os.sched_getaffinity(0))[0])
            with mock.patch.object(baseline, 'verify_archive', return_value={}), \
                 mock.patch.object(compare, '_baseline_identity', return_value=baseline_info), \
                 mock.patch.object(compare, '_verify_scalar_harness_sources'), \
                 mock.patch.object(workloads, 'selection', return_value=selected), \
                 mock.patch.object(compare, '_encode_scalar_fixture', side_effect=encode), \
                 mock.patch.object(compare, '_preflight_scalar_side'), \
                 mock.patch.object(compare, 'process', return_value=capabilities), \
                 mock.patch.object(compare, '_observe', side_effect=observe), \
                 mock.patch.object(compare, 'sha', side_effect=bound_sha), \
                 mock.patch.object(compare.subprocess, 'check_output', return_value='rustc mock'), \
                 mock.patch.object(checkpoint.Journal, 'preflight'), \
                 mock.patch.object(checkpoint, 'Journal', FakeJournal):
                with contextlib.redirect_stdout(io.StringIO()):
                    report = compare.run(args)
            self.assertTrue(report['passed'])
            settings = report['metadata']['settings']['scalar_warm_calibration']
            self.assertEqual(settings['policy'], 'shared_exact_batch_both_sides_confirmed')
            self.assertEqual(settings['legacy_control_ids'],
                             ['branch_calls', 'call_overhead', 'integer_loop'])
            records = report['warm_calibrations']
            self.assertEqual(len(records), 170)
            self.assertEqual(sum(row['workload_kind'] == 'typed_scalar_driver' for row in records), 164)
            self.assertEqual(sum(row['workload_kind'] == 'legacy_single_case_control' for row in records), 6)
            for record in records:
                self.assertEqual(record['paired_batch'], 12)
                self.assertEqual(len(record['attempts']), 1)
                self.assertEqual(len(record['attempts'][0]['sides']), 2)
                self.assertTrue(all(side['calibrated_total_ns'] >= 20_000_000
                                    for side in record['sides']))
            pairs = [pair for run in report['runs'] for pair in run['pairs']]
            self.assertEqual(len(pairs), 170)
            self.assertTrue(all(pair['expected'] == expected_by_id[pair['workload']]
                                for pair in pairs))
            self.assertTrue(all(pair[side]['result'] == expected_by_id[pair['workload']]
                                for pair in pairs for side in ('baseline', 'candidate')))

if __name__ == '__main__': unittest.main()
