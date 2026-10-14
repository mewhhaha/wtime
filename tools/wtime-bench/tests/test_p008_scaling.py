import importlib.util
import json
import tempfile
import unittest
from pathlib import Path
from unittest import mock


SCRIPT = Path(__file__).resolve().parents[1] / 'scaling.py'
spec = importlib.util.spec_from_file_location('p008_scaling', SCRIPT)
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


class RunnerFailureTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='p008-runner-test-')
        self.base = Path(self.temp.name)
        self.binary = self.base / 'fake-bench'
        self.binary.write_text(
            '#!/usr/bin/env python3\n'
            'import json,sys\n'
            'if sys.argv[1:] == ["--protocol"]:\n'
            ' print(json.dumps({"schema_version":2,"instrumented":False,"modes":["decoded"]}))\n'
            ' raise SystemExit(0)\n'
            'print("deliberate fake-driver failure", file=sys.stderr)\n'
            'raise SystemExit(19)\n', encoding='utf-8')
        self.binary.chmod(0o755)
        self.encoder = Path('/workspace/wtime/target/debug/wtime-conformance')

    def tearDown(self):
        self.temp.cleanup()

    def args(self, output):
        return type('Args', (), {
            'binary': self.binary,
            'encoder': self.encoder,
            'output': output,
            'samples': 1,
            'probe_binary': None,
            'tool_dir': SCRIPT.parent,
        })()

    def test_driver_failure_is_recorded_without_synthetic_rows(self):
        output = self.base / 'failed'
        code, report = runner.run(self.args(output))
        self.assertEqual(code, 1)
        self.assertFalse(report['passed'])
        self.assertEqual(report['status'], 'failed')
        self.assertEqual(report['actual_observation_count'], 0)
        self.assertEqual(report['actual_calibration_count'], 0)
        self.assertIn('command failed (19)', report['failures'][0]['message'])
        self.assertTrue(report['source_stable'])
        saved = json.loads((output / 'report.json').read_text())
        self.assertEqual(saved['observations'], [])

    def test_source_instability_is_recorded_even_after_driver_failure(self):
        output = self.base / 'unstable'
        original = runner.source_hashes
        tool_dir = SCRIPT.parent
        first = original(runner._repo_root(tool_dir), tool_dir)
        changed = dict(first)
        changed['changed-during-test'] = '0' * 64
        with mock.patch.object(runner, 'source_hashes', side_effect=[first, changed]):
            code, report = runner.run(self.args(output))
        self.assertEqual(code, 1)
        self.assertFalse(report['passed'])
        self.assertFalse(report['source_stable'])
        self.assertEqual(report['actual_observation_count'], 0)
        self.assertIn('command failed (19)', report['failures'][0]['message'])

    def test_table_mutation_probe_requires_restore_and_has_no_timing_fields(self):
        expected = {
            'schema_version': 2,
            'case': 'scalar_table_mutation',
            'driver': 'table_mutation',
            'diagnostic': True,
            'status': 'mutation_verified',
            'input': 'i32:17',
            'target_a': 'i32:18',
            'target_b': 'i32:131071',
            'target_a_after_restore': 'i32:18',
            'mode': 'decoded',
            'profile': 'all-phase3-core',
        }
        result = runner.subprocess.CompletedProcess([], 0, json.dumps(expected), '')
        with mock.patch.object(runner, '_run_process', return_value=result) as run_process:
            actual = runner._call_table_mutation_probe(
                self.binary, self.base / 'table_controls.wasm',
                {'id': 'scalar_table_mutation_n17'}, 17, 18, 131071,
                'all-phase3-core', None)
        self.assertEqual(actual['target_a_after_restore'], 'i32:18')
        args = run_process.call_args.args[1]
        self.assertEqual(args[0], '--scalar-table-probe')
        self.assertEqual(args[-2:], ['decoded', 'all-phase3-core'])

        bad = dict(expected, target_a_after_restore='i32:19')
        result = runner.subprocess.CompletedProcess([], 0, json.dumps(bad), '')
        with mock.patch.object(runner, '_run_process', return_value=result):
            with self.assertRaises(runner.BaselineError):
                runner._call_table_mutation_probe(
                    self.binary, self.base / 'table_controls.wasm',
                    {'id': 'scalar_table_mutation_n17'}, 17, 18, 131071,
                    'all-phase3-core', None)


if __name__ == '__main__':
    unittest.main()
