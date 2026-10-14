"""Fast P026 adapter checks. No benchmark processes, Wasm execution, or builds."""
from __future__ import annotations
import importlib.util
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

REPO = Path(__file__).resolve().parents[3]
BENCH = REPO / 'tools/wtime-bench'
STAGE = BENCH
sys.path[:0] = [str(STAGE), str(BENCH)]
import simd_scaling
import simd_workloads
import workloads
import analyze

spec = importlib.util.spec_from_file_location('compare', STAGE / 'compare.py')
compare = importlib.util.module_from_spec(spec)
sys.modules['compare'] = compare
spec.loader.exec_module(compare)
compare.HERE = BENCH
compare.ROOT = REPO
# Load the staged gate code against the real repository data paths while
# directing its compare import to this reviewed adapter snapshot.
gates_spec = importlib.util.spec_from_file_location('p026_staged_gates', STAGE / 'gates.py')
gates = importlib.util.module_from_spec(gates_spec)
gate_source = (STAGE / 'gates.py').read_text()
gate_source = gate_source.replace(
    'HERE = Path(__file__).resolve().parent', f'HERE = Path({str(BENCH)!r})')
gate_source = gate_source.replace(
    'ROOT = HERE.parents[1]', f'ROOT = Path({str(REPO)!r})')
gate_source = gate_source.replace(
    'HERE / "compare.py"', f'Path({str(STAGE / "compare.py")!r})')
sys.modules[gates_spec.name] = gates
exec(compile(gate_source, str(STAGE / 'gates.py'), 'exec'), gates.__dict__)
gates.compare.HERE = BENCH
gates.compare.ROOT = REPO


class P026CompareReviewTests(unittest.TestCase):
    def test_simd_cli_dispatches_to_adapter_before_legacy_path(self):
        args = mock.Mock(selection='simd', build_growth_explanation_file=None)
        sentinel = object()
        with mock.patch.object(compare, '_run_simd', return_value=sentinel) as adapter:
            self.assertIs(compare.run(args), sentinel)
        adapter.assert_called_once_with(args, None)

    def test_recursive_sources_are_preserved_and_simd_wat_is_added(self):
        all_sources = compare.sources(include_simd=True)
        scalar_sources = compare.sources()
        simd_wat = next((BENCH / 'fixtures/simd').glob('*.wat'))
        key = simd_wat.relative_to(REPO).as_posix()
        self.assertIn(key, all_sources)
        self.assertNotIn(key, scalar_sources)
        # These come from the live recursive source inventory, not a hand-maintained list.
        for rel in ('tools/wtime-bench/simd_workloads.py',
                    'tools/wtime-bench/simd_scaling.py',
                    'tools/wtime-bench/gates.py'):
            self.assertIn(rel, all_sources)
        self.assertIn('Cargo.toml', all_sources)
        self.assertIn('tools/wtime-bench/src/protocol.rs', all_sources)

    def test_gate_current_source_identity_includes_selected_simd_wat(self):
        identity = gates._current_source_identity('simd')
        expected = (BENCH / 'fixtures/simd/integer_lanes.wat').relative_to(REPO).as_posix()
        self.assertIn(expected, identity)
        self.assertEqual(identity, compare.sources(include_simd=True))

    def test_typed_protocol_arguments_and_allowed_result_normalization(self):
        entry = next(e for e in workloads.selection('simd')
                     if e['id'] == 'simd_relaxed_swizzle_oob_17')
        input_value = simd_scaling.typed_input(entry)
        allowed = simd_scaling.allowed_wire_results(entry, simd_workloads)
        profile = simd_scaling.execution_profile(entry)
        self.assertEqual(allowed, ['i32:0', 'i32:16'])
        response = {'schema_version': 2, 'case': entry['id'], 'stage': 'warm_call',
                    'input': input_value, 'expected': 'i32:0|i32:16',
                    'result': 'i32:16', 'batch': 30, 'ns_per_operation': 700000.0,
                    'mode': 'decoded', 'profile': profile}
        with mock.patch.object(compare, 'process', return_value=response) as process:
            observation = compare._simd_observation(
                '/mock/bench', '/mock/case.wasm', entry, input_value,
                entry['oracle_arguments']['index'], allowed, 'warm_call', 30,
                'decoded', profile, 4, model=simd_workloads, expected_batch=30)
        self.assertEqual(process.call_args.args[1], [
            '--typed-allowed-case', '/mock/case.wasm', entry['id'], input_value,
            'i32:0|i32:16', 'warm_call', '30', 'decoded', profile])
        self.assertEqual(observation['wire_result'], 16)
        self.assertEqual(observation['result'], {'allowed_results': [0, 16]})

    def test_simd_memory_uses_exact_memory_protocol_and_oracle(self):
        entry = next(e for e in workloads.selection('simd')
                     if e['id'] == 'simd_lane_memory_footprint_16')
        input_value = simd_scaling.typed_input(entry)
        allowed = simd_scaling.allowed_wire_results(entry, simd_workloads)
        profile = simd_scaling.execution_profile(entry)
        _result, _touched, full = simd_workloads._footprint_memory(1)
        response = {'schema_version': 2, 'case': entry['id'], 'stage': 'first_call',
                    'input': input_value, 'expected': allowed[0], 'result': allowed[0],
                    'batch': 1, 'ns_per_operation': 100.0, 'mode': 'decoded',
                    'profile': profile, 'memory_pages': 1,
                    'memory_page_size_log2': 16, 'memory_hex': full.hex()}
        with mock.patch.object(compare, 'process', return_value=response) as process:
            observation = compare._simd_observation(
                '/mock/bench', '/mock/fixture.wasm', entry, input_value, 1, allowed,
                'first_call', 1, 'decoded', profile, 4, model=simd_workloads,
                expected_batch=1)
        self.assertEqual(process.call_args.args[1], [
            '--simd-memory-case', '/mock/fixture.wasm', entry['id'], input_value,
            allowed[0], 'first_call', '1', 'decoded', profile])
        self.assertEqual(observation['memory_full_sha256'],
                         entry['memory_expectation']['full_memory_sha256'])
        changed = bytearray(full)
        changed[-1] ^= 1
        with mock.patch.object(compare, 'process', return_value=dict(response,
                                                                     memory_hex=bytes(changed).hex())):
            with self.assertRaisesRegex(compare.ComparisonError, 'full memory state differs'):
                compare._simd_observation(
                    '/mock/bench', '/mock/fixture.wasm', entry, input_value, 1,
                    allowed, 'first_call', 1, 'decoded', profile, 4,
                    model=simd_workloads, expected_batch=1)

    def test_capability_handshake_rejects_instrumentation_missing_profiles_or_types(self):
        good = {'schema_version': 2, 'engine': 'wtime', 'instrumented': False,
                'modes': ['decoded'], 'features': ['simd', 'simd-relaxed'],
                'types': ['i32', 'i64', 'f32_bits', 'f64_bits', 'v128_bits']}
        compare._validate_simd_capability('candidate', good, 'decoded', ['simd'])
        for bad in (dict(good, instrumented=True),
                    dict(good, features=['simd']),
                    dict(good, types=['i32'])):
            with self.subTest(bad=bad), self.assertRaises(compare.ComparisonError):
                compare._validate_simd_capability('candidate', bad, 'decoded',
                                                  ['simd', 'simd-relaxed'])

    def test_normalized_allowed_set_is_accepted_by_paired_analyzer(self):
        allowed = [0, 16]
        report = {'schema_version': 2, 'passed': True, 'source_stable': True,
                  'selection': 'simd', 'metadata': {'settings': {'instrumented': False}},
                  'runs': []}
        digest = 'a' * 64
        for run_id, seed in ((0, 901), (1, 902)):
            pairs = []
            for index in range(30):
                pair = {'pair_id': f'{run_id}:{index}', 'workload': 'simd_relaxed_swizzle_oob_17',
                        'stage': 'warm_call', 'input': 'i32:17', 'wasm_sha256': digest,
                        'expected': {'allowed_results': allowed},
                        'order': ['baseline', 'candidate']}
                for side, ns in (('baseline', 100.0), ('candidate', 95.0)):
                    pair[side] = {'ns_per_operation': ns, 'batch': 10,
                                  'result': {'allowed_results': allowed},
                                  'wire_result': 16, 'mode': 'decoded',
                                  'settings': {'features': ['simd-relaxed'], 'fuel': False,
                                               'cancellation_quantum': 1}}
                pairs.append(pair)
            report['runs'].append({'run_id': run_id, 'seed': seed, 'pairs': pairs})
        summary = analyze.analyze(report)
        self.assertEqual(len(summary['per_run']), 2)
        self.assertTrue(all(row['workload'] == 'simd_relaxed_swizzle_oob_17'
                            for row in summary['per_run']))

    def test_promotion_contract_covers_all_114_simd_workloads(self):
        selected = workloads.selection('simd')
        self.assertEqual(len(selected), 114)
        stages = gates._selection_stages('simd')
        self.assertEqual(set(stages), {entry['id'] for entry in selected})
        primary = sorted(entry['id'] for entry in selected
                         if entry.get('family') in {'simd_lane_memory',
                                                    'simd_lane_memory_footprint'} and entry.get('work_units',{}).get('count',0)>0)
        protected = sorted(set(stages) - set(primary))
        self.assertEqual(len(primary), 5)
        self.assertEqual(len(protected), 109)
        contract = {'schema_version': 1, 'selection': 'simd',
                    'primary_workloads': primary, 'protected_workloads': protected}
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'contract.json'
            path.write_text(json.dumps(contract))
            loaded, digest = gates._load_contract(path, 'simd')
            self.assertEqual(loaded, contract)
            self.assertEqual(len(digest), 64)
            contract['protected_workloads'].pop()
            path.write_text(json.dumps(contract))
            with self.assertRaisesRegex(gates.GateError, 'cover the full selection'):
                gates._load_contract(path, 'simd')


if __name__ == '__main__':
    unittest.main()
