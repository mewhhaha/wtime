import importlib.util
import json
import tempfile
import unittest
from pathlib import Path
from unittest import mock


HERE = Path(__file__).resolve().parents[1]
REPO_ROOT = HERE.parents[1]
import sys
sys.path.insert(0, str(HERE))

import memory_scaling as runner
import memory_workloads
import workloads

TOOL_DIR = HERE
ENCODER = REPO_ROOT / 'target/debug/wtime-conformance'
registry = runner._load_registry(TOOL_DIR)


class MemoryRunnerFailureTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='p009-memory-runner-test-')
        self.base = Path(self.temp.name)
        self.binary = self.base / 'fake-memory-driver'
        self.binary.write_text(
            '#!/usr/bin/env python3\n'
            'import json,sys\n'
            'if sys.argv[1:] == ["--protocol"]:\n'
            ' print(json.dumps({"schema_version":2,"instrumented":False,"modes":["decoded"]}))\n'
            ' raise SystemExit(0)\n'
            'print("deliberate fake memory-driver failure", file=sys.stderr)\n'
            'raise SystemExit(19)\n', encoding='utf-8')
        self.binary.chmod(0o755)

    def tearDown(self):
        self.temp.cleanup()

    def test_memory_selection_merges_new_cases_and_retains_historical_control(self):
        loaded = workloads.load_registry()
        selected = workloads.selection('memory', loaded)
        selected_ids = {entry['id'] for entry in selected}
        new_ids = {entry['id'] for entry in memory_workloads.entries()}
        self.assertTrue(new_ids.issubset(selected_ids))
        self.assertIn('memory_fill_scan', selected_ids)
        self.assertEqual(selected_ids, new_ids | {'memory_fill_scan'})

    def test_registry_compacts_bytes_but_preserves_oracle_digests_and_ranges(self):
        registry_path = TOOL_DIR / 'workloads.json'
        self.assertLess(registry_path.stat().st_size, 512 * 1024)
        loaded = workloads.load_registry()
        by_id = {entry['id']: entry for entry in loaded['workloads']}
        for entry in memory_workloads.entries():
            full = memory_workloads.expected_memory(
                entry['id'], entry['params'][0]['baseline'])
            compact = by_id[entry['id']]['memory_expectation']
            expected = dict(full)
            if 'bytes_hex' in expected:
                expected.pop('bytes_hex')
                expected['bytes_oracle'] = 'expected_memory'
            self.assertEqual(compact, expected, entry['id'])
            self.assertNotIn('bytes_hex', compact)

    def test_nondefault_page_size_checks_full_byte_count_and_page_log2(self):
        entry = next(e for e in memory_workloads.entries()
                     if e['family'] == 'memory_custom_page_one_byte')
        expected = memory_workloads.expected_memory(entry['id'], 255)
        data = {
            'schema_version': 2, 'case': entry['id'], 'stage': 'load_and_call',
            'input': 'i32:255', 'expected': 'i32:-128', 'result': 'i32:-128',
            'batch': 1, 'ns_per_operation': 1.0, 'mode': 'decoded',
            'profile': 'mvp', 'driver': 'plain', 'memory_pages': 256,
            'memory_page_size_log2': 0, 'memory_hex': expected['bytes_hex'],
            'trap_kind': None, 'trap_message': None,
        }
        runner._validate_observation(data, entry, 255, expected,
                                     'decoded', 'mvp', 'plain')
        bad_length = dict(data, memory_hex=bytes(512).hex())
        with self.assertRaisesRegex(runner.BaselineError, 'invalid size'):
            runner._validate_observation(bad_length, entry, 255, expected,
                                         'decoded', 'mvp', 'plain')
        bad_log = dict(data, memory_page_size_log2=1)
        with self.assertRaisesRegex(runner.BaselineError, 'page-size mismatch'):
            runner._validate_observation(bad_log, entry, 255, expected,
                                         'decoded', 'mvp', 'plain')

    def test_driver_arguments_keep_typed_i32_input(self):
        entry = memory_workloads.entries()[0]
        args = runner._memory_driver_args(Path('fixture.wasm'), entry, 64,
                                          'i32:1', 'mvp', 'plain')
        self.assertEqual(args[3], 'i32:64')

    def test_multimemory_checks_selected_index_and_each_python_digest(self):
        entry = next(e for e in memory_workloads.entries()
                     if e['family'] == 'memory_multi_memory_second_export')
        expected = memory_workloads.expected_memory(entry['id'], 0)
        first, second = memory_workloads._multi_memory_model()[1:]
        data = {
            'schema_version': 2, 'case': entry['id'], 'stage': 'load_and_call',
            'input': 'i32:0', 'expected': 'i32:51', 'result': 'i32:51',
            'batch': 1, 'ns_per_operation': 1.0, 'mode': 'decoded',
            'profile': 'all-phase3-core', 'driver': 'multi', 'memory_pages': 1,
            'memory_page_size_log2': 16, 'memory_hex': second.hex(),
            'memory_count': 2, 'exported_memory_index': 1,
            'all_memory_hex': [first.hex(), second.hex()],
            'trap_kind': None, 'trap_message': None,
        }
        runner._validate_observation(data, entry, 0, expected,
                                     'decoded', 'all-phase3-core', 'multi')
        wrong_memory = dict(data, all_memory_hex=[bytes(len(first)).hex(), second.hex()])
        with self.assertRaisesRegex(runner.BaselineError, 'all-memory digest mismatch'):
            runner._validate_observation(wrong_memory, entry, 0, expected,
                                         'decoded', 'all-phase3-core', 'multi')

    def test_wasmtime_preflight_records_capability_without_timings(self):
        entry = next(e for e in memory_workloads.entries()
                     if e['family'] == 'memory_widths_predictable')
        fixture_name = Path(entry['source']['path']).name
        fixture_record = {fixture_name: {'wasm_sha256': entry['source']['wasm_sha256']}}
        response = {'schema_version': 2, 'status': 'supported',
                    'compile_execute_checked': True}
        process_result = runner.subprocess.CompletedProcess([], 0, json.dumps(response), '')
        output = Path(self.temp.name) / 'output'
        with mock.patch.object(runner, '_run_process', return_value=process_result) as invoked:
            rows = runner._wasmtime_capabilities(
                self.binary, output, [entry], registry, fixture_record, None)
        self.assertEqual(rows[0]['status'], 'supported')
        self.assertNotIn('ns_per_operation', rows[0])
        args = invoked.call_args.args[1]
        self.assertEqual(args[0], '--capability')
        self.assertTrue(args[2].startswith('i32:'))
        self.assertTrue(args[3].startswith('i32:'))

        trap = next(e for e in memory_workloads.entries()
                    if e['family'] == 'memory_oob_copy_order')
        trap_rows = runner._wasmtime_capabilities(
            self.binary, output, [trap], registry, fixture_record, None)
        self.assertEqual(trap_rows[0]['status'], 'unsupported')
        self.assertIn('trap-category oracle', trap_rows[0]['reason'])

    def test_candidate_failure_records_no_synthetic_observation(self):
        if not ENCODER.is_file():
            self.skipTest('existing conformance encoder is unavailable')
        output = self.base / 'failed'
        args = type('Args', (), {
            'binary': self.binary, 'encoder': ENCODER, 'output': output,
            'samples': 1, 'tool_dir': TOOL_DIR, 'probe_binary': None,
        })()
        code, report = runner.run(args)
        self.assertEqual(code, 1)
        self.assertFalse(report['passed'])
        self.assertEqual(report['actual_observation_count'], 0)
        self.assertIn('command failed (19)', report['failures'][0]['message'])
        self.assertTrue(report['source_stable'])
        self.assertEqual(json.loads((output / 'report.json').read_text())['observations'], [])

    def test_full_memory_hash_mismatch_is_rejected(self):
        entry = next(e for e in memory_workloads.entries() if e['id'] == 'memory_growth_success_zero')
        expected = registry.expected_memory(entry['id'], 0)
        data = {
            'schema_version': 2, 'case': entry['id'], 'stage': 'load_and_call',
            'input': 'i32:0', 'expected': 'i32:1', 'result': 'i32:1',
            'batch': 1, 'ns_per_operation': 1.0, 'mode': 'decoded',
            'profile': 'mvp', 'driver': 'plain', 'memory_pages': 1,
            'memory_hex': (bytes(65535) + b'\x01').hex(),
            'trap_kind': None, 'trap_message': None,
        }
        with self.assertRaisesRegex(runner.BaselineError, 'full-memory hash mismatch'):
            runner._validate_observation(data, entry, 0, expected, 'decoded', 'mvp', 'plain')

    def test_generic_trap_code_does_not_satisfy_out_of_bounds_oracle(self):
        entry = next(e for e in memory_workloads.entries() if e['id'] == 'memory_effective_address_overflow')
        expected = registry.expected_memory(entry['id'], 0)
        data = {
            'schema_version': 2, 'case': entry['id'], 'stage': 'load_and_call',
            'input': 'i32:0', 'expected': 'trap', 'result': 'trap',
            'batch': 1, 'ns_per_operation': 1.0, 'mode': 'decoded',
            'profile': 'all-phase3-core', 'driver': 'plain', 'memory_pages': 1,
            'memory_hex': expected['bytes_hex'],
            'trap_kind': 'Some(Other)', 'trap_message': 'generic trap',
        }
        with self.assertRaisesRegex(runner.BaselineError, 'wrong trap category'):
            runner._validate_observation(data, entry, 0, expected,
                                         'decoded', 'all-phase3-core', 'plain')


if __name__ == '__main__':
    unittest.main()
