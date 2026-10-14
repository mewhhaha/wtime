"""Synthetic-only checks for the staged P033 GC comparison adapter."""
from __future__ import annotations

import hashlib
import importlib.util
import json
import os
from pathlib import Path
import sys
import tempfile
import types
import unittest
from unittest import mock

BENCH = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(BENCH))
SCRIPT = BENCH / 'gc_compare.py'
spec = importlib.util.spec_from_file_location('p033_gc_compare', SCRIPT)
gc = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gc)


def sha_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def warm_entry(workload='gc_warm_test_n17', n=17, expected=7, allocation=0):
    return {
        'id': workload, 'family': 'gc_test_family',
        'source': {'path': 'fixtures/gc/test.wat', 'sha256': 'wat-hash',
                   'wasm_sha256': 'wasm-hash'},
        'params': [{'name': 'n', 'type': 'i32', 'min': n, 'max': n, 'baseline': n}],
        'oracle': {'module': 'gc_warm_workloads.py'},
        'driver': {'kind': 'gc_scalar_stages', 'profile': gc.PROFILE},
        'gc_expectation': {'allocation_count': allocation, 'oracle_result': expected},
    }


def wire(entry, stage='warm_call', batch=1, expected=7, *, sample=None):
    n = entry['params'][0]['baseline']
    prepare, instantiate, first = 2_000_000.0, 3_000_000.0, 5_000_000.0
    if stage == 'prepare': sample = prepare
    elif stage == 'instantiate': sample = instantiate
    elif stage == 'first_call': sample = first
    elif stage == 'load_and_call': sample = prepare + instantiate + first
    else: sample = sample if sample is not None else 20_000_000.0
    measured = sample * batch if stage == 'warm_call' else first
    calls = batch + 1 if stage == 'warm_call' else 1
    alloc = entry['gc_expectation']['allocation_count']
    return {
        'schema_version': 2, 'driver': gc.WARM_DRIVER_NAME, 'case': entry['id'],
        'family': entry['family'], 'input': n, 'mode': gc.MODE, 'profile': gc.PROFILE,
        'stage': stage, 'batch': batch,
        'max_batch': min(gc.MAX_BATCH, gc.MAX_PRODUCT // max(n, 1)),
        'instrumented': False, 'measurement_policy': gc.MEASUREMENT_POLICY,
        'report_complete': True, 'result': f'i32:{expected}', 'sample_ns': sample,
        'prepare_ns': prepare, 'instantiate_ns': instantiate, 'first_call_ns': first,
        'measured_guest_ns': measured, 'reset_ns': 1_000_000.0,
        'calls_checked': calls, 'total_reclaimed': calls * alloc,
        'reclaimed_per_call': alloc, 'live_after_reset': 0,
        'payload_bytes_after_reset': 0, 'continuation_bytes_after_reset': 0,
    }


class P033GcCompareTests(unittest.TestCase):
    def test_pair_matrix_is_exactly_225_groups_per_run_and_13500_pairs(self):
        counts = gc.pair_count()
        self.assertEqual(counts, {
            'legacy_workload_count': 45, 'warm_workload_count': 36,
            'groups_per_run': 225, 'groups_total': 450,
            'pairs_per_group': 30, 'pair_events_per_run': 6750,
            'pair_event_count': 13500, 'side_observation_count': 27000,
        })
        for pairs in (29, 31):
            with self.subTest(pairs=pairs), self.assertRaises(gc.GcComparisonError):
                gc.pair_count(pairs=pairs)

    def test_contract_requires_exact_12_primary_69_protected_partition(self):
        contract = json.loads((BENCH.parents[1] / 'docs/benchmarks/performance/P033-contract.json').read_text())
        ids = contract['primary_workloads'] + contract['protected_workloads']
        warm_ids = [wid for wid in ids if wid.startswith('gc_warm_')]
        checked = gc.validate_contract(contract, ids, warm_ids)
        self.assertEqual((checked['primary_workload_count'], checked['protected_workload_count']), (12, 69))
        bad = dict(contract, protected_workloads=contract['protected_workloads'][:-1])
        with self.assertRaisesRegex(gc.GcComparisonError, 'partition'):
            gc.validate_contract(bad, ids, warm_ids)
        bad = dict(contract, unexpected='not allowed')
        with self.assertRaisesRegex(gc.GcComparisonError, 'unknown fields'):
            gc.validate_contract(bad, ids, warm_ids)
        bad = dict(contract, schema_version=True)
        with self.assertRaisesRegex(gc.GcComparisonError, 'partition'):
            gc.validate_contract(bad, ids, warm_ids)

    def test_wire_validator_checks_actual_stage_interval_calls_and_gc_reset(self):
        entry = warm_entry()
        for stage in gc.STAGES:
            batch = 3 if stage == 'warm_call' else 1
            row = wire(entry, stage, batch)
            checked = gc.validate_warm_observation(entry, row, stage=stage, batch=batch,
                                                   mode=gc.MODE, expected=7)
            self.assertEqual(checked['result'], 7)
        for bad in (dict(wire(entry), result='i32:8'),
                    dict(wire(entry), calls_checked=1),
                    dict(wire(entry), live_after_reset=1),
                    dict(wire(entry), input=True), dict(wire(entry), batch=True),
                    dict(wire(entry), max_batch=True), dict(wire(entry), calls_checked=True),
                    dict(wire(entry), total_reclaimed=True), dict(wire(entry), reclaimed_per_call=True),
                    dict(wire(entry), live_after_reset=False),
                    dict(wire(entry), instrumented=0), dict(wire(entry), report_complete=1),
                    dict(wire(entry), sample_ns=1.0)):
            with self.subTest(bad=bad), self.assertRaises(gc.GcComparisonError):
                gc.validate_warm_observation(entry, bad, stage='warm_call', batch=1,
                                             mode=gc.MODE, expected=7)

    def test_shared_calibration_grows_once_to_common_confirmed_batch(self):
        entry = warm_entry()
        observed = []
        def observe(side, batch):
            observed.append((side, batch))
            ns = 4_000_000.0 if side == 'candidate' else 5_000_000.0
            return wire(entry, 'warm_call', batch, sample=ns)
        result = gc.calibrate_shared(entry, gc.MODE, 7, observe)
        self.assertEqual(result['paired_batch'], 6)
        self.assertEqual([attempt['batch'] for attempt in result['attempts']], [1, 6])
        self.assertEqual(len(observed), 4)
        self.assertTrue(all(side['measured_guest_ns'] >= gc.TARGET_NS
                            for side in result['sides']))

    def test_input_schema_rejects_boolean_integer_bounds(self):
        entry = warm_entry()
        for field in ('baseline', 'min', 'max'):
            bad = json.loads(json.dumps(entry))
            bad['params'][0][field] = True
            with self.subTest(field=field), self.assertRaises(gc.GcComparisonError):
                gc._input(bad)

    def test_cold_wire_results_are_normalized_before_pair_comparison(self):
        old = {'id': 'exception_uncaught_throw_n1', 'params': [
            {'name': 'n', 'type': 'i32', 'min': 1, 'max': 1, 'baseline': 1}]}
        class Model:
            @staticmethod
            def oracle(workload, n):
                return {'kind': 'exception', 'tag': 'number', 'payload': [n]}
        sample = {'workload': old['id'], 'expected': Model.oracle(old['id'], 1),
                  'result': None, 'outcome': 'exception', 'tag': 'number', 'payload': ['i32:1'], 'stage': 'load_and_call',
                  'batch': 1, 'mode': gc.MODE, 'profile': gc.PROFILE,
                  'ns_per_operation': 123.0,
                  'wire_observation': {'outcome': 'exception', 'result': None,
                                       'tag': 'number', 'payload': ['i32:1']}}
        normalized = gc._normalize_cold(old, sample, Model)
        self.assertEqual(normalized['result'], {'kind': 'exception', 'tag': 'number', 'payload': [1]})
        self.assertEqual(normalized['expected'], normalized['result'])
        bad = dict(sample, result='i32:1')
        with self.assertRaises(gc.GcComparisonError):
            gc._normalize_cold(old, bad, Model)
        integer = {'id': 'gc_struct_graph_n1', 'params': [
            {'name': 'n', 'type': 'i32', 'min': 1, 'max': 1, 'baseline': 1}]}
        class IntegerModel:
            @staticmethod
            def oracle(workload, n): return 7
        integer_sample = {'workload': integer['id'], 'expected': 7, 'result': 'i32:7',
            'outcome': 'returned', 'tag': None, 'payload': None, 'stage': 'load_and_call',
            'batch': 1, 'mode': gc.MODE, 'profile': gc.PROFILE, 'ns_per_operation': 123.0,
            'wire_observation': {'outcome': 'returned', 'result': 'i32:7', 'tag': None, 'payload': None}}
        self.assertEqual(gc._normalize_cold(integer, integer_sample, IntegerModel)['result'], 7)
        bad_wire = dict(sample, wire_observation={'outcome': 'exception', 'result': None,
                                                   'tag': 'wrong', 'payload': ['i32:1']})
        with self.assertRaises(gc.GcComparisonError):
            gc._normalize_cold(old, bad_wire, Model)

    def test_logical_provenance_rejects_absolute_and_parent_paths(self):
        self.assertEqual(gc._logical_fixture_path('fixtures/gc/thing.wat', 'case'),
                         'tools/wtime-bench/fixtures/gc/thing.wat')
        self.assertEqual(gc._logical_fixture_path('fixtures/exceptions/failure.wat', 'case'),
                         'tools/wtime-bench/fixtures/exceptions/failure.wat')
        for path in ('/tmp/thing.wat', 'fixtures/gc/../x.wat', 'fixtures/simd/thing.wat'):
            with self.subTest(path=path), self.assertRaises(gc.GcComparisonError):
                gc._logical_fixture_path(path, 'case')

    def test_complete_mocked_run_writes_only_inside_temporary_output_root(self):
        contract = json.loads((BENCH.parents[1] / 'docs/benchmarks/performance/P033-contract.json').read_text())
        warm_ids = list(contract['primary_workloads']) + [
            wid for wid in contract['protected_workloads'] if wid.startswith('gc_warm_')]
        old_ids = [wid for wid in contract['protected_workloads'] if wid not in set(warm_ids)]
        self.assertEqual((len(old_ids), len(warm_ids)), (45, 36))
        with tempfile.TemporaryDirectory(prefix='p033-gc-test-') as temporary:
            sandbox = Path(temporary).resolve()
            root = sandbox / 'repo'; bench = root / 'tools/wtime-bench'
            frozen = sandbox / 'archive'
            fixtures = bench / 'fixtures/gc'
            fixtures.mkdir(parents=True)
            frozen_sources = frozen / 'source/tools/wtime-bench'
            frozen_sources.mkdir(parents=True)
            wat_bytes, wasm_bytes = b'(module (func))\n', b'encoded-test-module'
            wat_hash, wasm_hash = sha_bytes(wat_bytes), sha_bytes(wasm_bytes)
            (fixtures / 'test.wat').write_bytes(wat_bytes)
            (fixtures / 'test.wasm').write_bytes(wasm_bytes)
            provider_bytes = b'(module ;; provider\n)\n'
            provider_hash = sha_bytes(provider_bytes)
            (fixtures / 'provider.wat').write_bytes(provider_bytes)
            (fixtures / 'provider.wasm').write_bytes(wasm_bytes)
            for path, content in ((frozen_sources/'fixtures/gc/test.wat', wat_bytes),
                                  (frozen_sources/'fixtures/gc/test.wasm', wasm_bytes),
                                  (frozen_sources/'fixtures/gc/provider.wat', provider_bytes),
                                  (frozen_sources/'fixtures/gc/provider.wasm', wasm_bytes)):
                path.parent.mkdir(parents=True, exist_ok=True); path.write_bytes(content)
            simple_sources = {
                'workloads.json': b'{}\n', 'gc_workloads.py': b'# old model\n',
                'gc_warm_workloads.py': b'# warm model\n', 'gc_scaling.py': b'# old runner\n',
                'stable.py': b'# harness source\n',
            }
            manifest_files = {
                'source/tools/wtime-bench/fixtures/gc/test.wat': {
                    'role': 'source', 'source_path': 'tools/wtime-bench/fixtures/gc/test.wat',
                    'sha256': wat_hash},
                'source/tools/wtime-bench/fixtures/gc/test.wasm': {
                    'role': 'source', 'source_path': 'tools/wtime-bench/fixtures/gc/test.wasm',
                    'sha256': wasm_hash},
                'source/tools/wtime-bench/fixtures/gc/provider.wat': {
                    'role': 'source', 'source_path': 'tools/wtime-bench/fixtures/gc/provider.wat',
                    'sha256': provider_hash},
                'source/tools/wtime-bench/fixtures/gc/provider.wasm': {
                    'role': 'source', 'source_path': 'tools/wtime-bench/fixtures/gc/provider.wasm',
                    'sha256': wasm_hash}}
            for name, content in simple_sources.items():
                live = bench / name; live.write_bytes(content)
                archived = frozen_sources / name; archived.write_bytes(content)
                manifest_files[f'source/tools/wtime-bench/{name}'] = {
                    'role': 'source', 'source_path': f'tools/wtime-bench/{name}',
                    'sha256': sha_bytes(content)}
            protocol = bench / 'src/protocol.rs'; protocol.parent.mkdir(); protocol.write_bytes(b'protocol')
            (frozen_sources/'src/protocol.rs').parent.mkdir(parents=True, exist_ok=True)
            (frozen_sources/'src/protocol.rs').write_bytes(b'protocol')
            manifest_files['source/tools/wtime-bench/src/protocol.rs'] = {
                'role': 'source', 'source_path': 'tools/wtime-bench/src/protocol.rs',
                'sha256': sha_bytes(b'protocol')}
            encoder = root / 'target/debug/wtime-conformance'; encoder.parent.mkdir(parents=True)
            encoder.write_bytes(b'encoder'); encoder.chmod(0o755)
            contract_path = root / 'docs/benchmarks/performance/P033-contract.json'
            contract_path.parent.mkdir(parents=True); contract_path.write_text(json.dumps(contract))
            candidate = sandbox / 'candidate'; candidate.write_bytes(b'candidate'); candidate.chmod(0o755)
            baseline_exe = sandbox / 'baseline'; baseline_exe.write_bytes(b'baseline'); baseline_exe.chmod(0o755)
            archive_manifest = {'files': manifest_files}
            archive_root = frozen; archive_root.mkdir(parents=True, exist_ok=True)
            (archive_root / 'manifest.json').write_text(json.dumps({'synthetic': True}))

            old_rows = []
            old_answers = {}
            for index, wid in enumerate(old_ids):
                n = int(wid.rsplit('_n', 1)[1])
                answer = {'kind': 'exception', 'tag': 'number', 'payload': [n]} if wid == 'exception_uncaught_throw_n1' else 7
                old_answers[wid] = answer
                old_row = {'id': wid, 'family': wid.rsplit('_n', 1)[0],
                    'source': {'path': 'fixtures/gc/test.wat', 'sha256': wat_hash,
                               'wasm_sha256': wasm_hash},
                    'params': [{'name': 'n', 'type': 'i32', 'min': n, 'max': n, 'baseline': n}],
                    'stages': ['load_and_call'], 'reset_policy': 'fresh_instance_per_invocation',
                    'oracle': {'module': 'gc_workloads.py'}}
                if wid.startswith('gc_descriptor_cross_module_'):
                    old_row['setup_modules'] = [{'module_name': 'provider',
                        'path': 'fixtures/gc/provider.wat', 'sha256': provider_hash,
                        'wasm_sha256': wasm_hash}]
                old_rows.append(old_row)
            warm_rows = []
            warm_answers = {}
            for wid in warm_ids:
                n = int(wid.rsplit('_n', 1)[1])
                warm_answers[wid] = 7
                warm_rows.append({'id': wid, 'family': wid.rsplit('_n', 1)[0],
                    'source': {'path': 'fixtures/gc/test.wat', 'sha256': wat_hash,
                               'wasm_sha256': wasm_hash},
                    'params': [{'name': 'n', 'type': 'i32', 'min': n, 'max': n, 'baseline': n}],
                    'stages': list(gc.STAGES),
                    'reset_policy': 'fresh_instance_for_cold_stages_reused_instance_for_warm_call',
                    'oracle': {'module': 'gc_warm_workloads.py'},
                    'driver': {'kind': gc.WARM_DRIVER_NAME, 'profile': gc.PROFILE},
                    'comparators': {'wasmtime': {'status': 'unsupported'}},
                    'gc_expectation': {'allocation_count': 0, 'oracle_result': 7}})
            ids = old_ids + warm_ids
            registry = {'workloads': old_rows + warm_rows,
                        'selections': {'gc-exceptions': {'status': 'ready', 'workloads': ids}}}

            old_mod = types.ModuleType('gc_workloads'); old_mod.__file__ = str(bench / 'gc_workloads.py')
            old_mod.entries = lambda: old_rows
            old_mod.oracle = lambda wid, n: old_answers[wid]
            warm_mod = types.ModuleType('gc_warm_workloads'); warm_mod.__file__ = str(bench / 'gc_warm_workloads.py')
            warm_mod.entries = lambda: warm_rows
            warm_mod.oracle = lambda wid, n: warm_answers[wid]
            workload_mod = types.ModuleType('workloads'); workload_mod.load_registry = lambda: registry
            baseline_mod = types.ModuleType('baseline'); baseline_mod.verify_archive = lambda path: archive_manifest
            info = {'kind': 'current-source-archive', 'mode': gc.MODE, 'frozen': frozen,
                    'executable': baseline_exe}
            compare_mod = types.ModuleType('compare')
            compare_mod._baseline_identity = lambda *args: info
            compare_mod._verify_scalar_harness_sources = lambda *args: None
            current_source_map = {
                'tools/wtime-bench/stable.py': sha_bytes(b'# harness source\n'),
                'tools/wtime-bench/fixtures/gc/test.wat': wat_hash,
                'tools/wtime-bench/fixtures/gc/test.wasm': wasm_hash,
                'tools/wtime-bench/fixtures/gc/provider.wat': provider_hash,
                'tools/wtime-bench/fixtures/gc/provider.wasm': wasm_hash,
                'tools/wtime-bench/src/protocol.rs': sha_bytes(b'protocol'),
            }
            compare_mod.sources = lambda: current_source_map
            compare_mod.ComparisonError = ValueError
            compare_mod._read_build_growth_explanation = lambda path: None
            growth_checks = []
            compare_mod._verify_build_growth_explanation = lambda record: growth_checks.append(record)
            def fake_process(exe, arguments, cpu):
                if arguments == ['--protocol']:
                    return {'schema_version': 2, 'instrumented': False, 'modes': [gc.MODE],
                            'features': [gc.PROFILE]}
                if arguments[0] != '--gc-scalar-driver':
                    raise AssertionError(arguments)
                _, _wasm, workload, family, typed_n, mode, profile, stage, batch_text = arguments
                n, batch = int(typed_n.removeprefix('i32:')), int(batch_text)
                entry = next(item for item in warm_rows if item['id'] == workload)
                return wire(entry, stage, batch, expected=7)
            compare_mod.process = fake_process
            setup_probes = []
            def fake_cold(exe, fixture, entry, n, stage, batch, mode, cpu):
                if entry['id'].startswith('gc_descriptor_cross_module_'):
                    setup_probes.append(len(fixture['setup_modules']))
                expected = old_answers[entry['id']]
                if isinstance(expected, dict):
                    wire_result = {'outcome': 'exception', 'result': None, 'tag': expected['tag'],
                                   'payload': [f'i32:{value}' for value in expected['payload']]}
                else:
                    wire_result = {'outcome': 'returned', 'result': f'i32:{expected}',
                                   'tag': None, 'payload': None}
                return {'workload': entry['id'], 'expected': expected, 'result': wire_result['result'],
                        'outcome': wire_result['outcome'], 'tag': wire_result['tag'],
                        'payload': wire_result['payload'], 'stage': stage, 'batch': batch,
                        'mode': mode, 'profile': gc.PROFILE, 'ns_per_operation': 100.0,
                        'wire_observation': wire_result}
            cold = types.SimpleNamespace(cold_observation=fake_cold)
            args = types.SimpleNamespace(selection='gc-exceptions', pairs=30, output=sandbox/'output'/'run',
                baseline=archive_root, candidate=candidate, candidate_mode=gc.MODE,
                cpu=min(os.sched_getaffinity(0)), seed=7, contract=contract_path)
            self.assertFalse(args.output.exists())
            self.assertTrue(args.output.resolve().is_relative_to(sandbox))
            def encoder_run(argv, **kwargs):
                Path(argv[4]).write_bytes(wasm_bytes)
                return types.SimpleNamespace(returncode=0, stdout='', stderr='')
            old_root, old_here, old_contract = gc.ROOT, gc.HERE, gc.CONTRACT_PATH
            try:
                gc.ROOT, gc.HERE, gc.CONTRACT_PATH = root, bench, contract_path
                with mock.patch.dict(sys.modules, {
                    'baseline': baseline_mod, 'compare': compare_mod, 'workloads': workload_mod,
                    'gc_workloads': old_mod, 'gc_warm_workloads': warm_mod,
                }), mock.patch.object(gc, '_load_file', return_value=cold), \
                     mock.patch.object(gc.subprocess, 'run', side_effect=encoder_run):
                    bad_contract_path = root/'docs/benchmarks/performance/P033-contract-with-unknown.json'
                    bad_contract_path.write_text(json.dumps(dict(contract, unexpected='reject')))
                    bad_output = sandbox/'output'/'bad-contract'
                    args.contract, args.output = bad_contract_path, bad_output
                    with self.assertRaisesRegex(gc.GcComparisonError, 'unknown fields'):
                        gc.run(args)
                    self.assertFalse(bad_output.exists(), 'invalid contract mutated output before validation')
                    args.contract, args.output = contract_path, sandbox/'output'/'run'
                    growth_file = sandbox/'growth-explanation.md'; growth_file.write_text('measured build growth note\n')
                    growth = {'path': growth_file, 'metadata': {
                        'relative_build_growth_explanation': 'measured build growth note\n',
                        'relative_build_growth_explanation_source': {
                            'path': 'docs/benchmarks/performance/P033-growth.md',
                            'sha256': sha_bytes(b'measured build growth note\n')}}}
                    report = gc.run(args, growth_explanation=growth)
            finally:
                gc.ROOT, gc.HERE, gc.CONTRACT_PATH = old_root, old_here, old_contract
            self.assertTrue(report['passed'])
            self.assertEqual(report['counts']['pair_event_count'], 13500)
            self.assertEqual(len(report['warm_calibrations']), 72)
            self.assertEqual(sum(len(run['pairs']) for run in report['runs']), 13500)
            self.assertEqual(len(report['correctness_probes']), 162)
            self.assertEqual(sum(probe['stage'] == 'load_and_call' for probe in report['correctness_probes']), 90)
            self.assertEqual(sum(probe['stage'] == 'first_call' for probe in report['correctness_probes']), 72)
            self.assertTrue(setup_probes and all(count == 1 for count in setup_probes))
            self.assertEqual(growth_checks, [growth, growth])
            self.assertEqual(report['metadata']['relative_build_growth_explanation'],
                             'measured build growth note\n')
            self.assertTrue((args.output/'report.json').is_file())
            for path in args.output.rglob('*'):
                self.assertTrue(path.resolve().is_relative_to(sandbox), path)
            identity = json.loads((args.output/'checkpoint.jsonl').read_text().splitlines()[0])['identity']
            self.assertEqual(identity['candidate_sources'], current_source_map)
            self.assertFalse(any(str(sandbox) in key for key in identity['candidate_sources']))
            self.assertFalse(any(str(sandbox) in value for value in identity['candidate_sources'].values()))
            self.assertEqual(identity['settings']['baseline_mode'], gc.MODE)
            self.assertEqual(identity['settings']['candidate_mode'], gc.MODE)


if __name__ == '__main__':
    unittest.main()
