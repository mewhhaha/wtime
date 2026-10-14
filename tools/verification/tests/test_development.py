"""Bounded development-lane regressions using isolated module copies and temp roots."""
import importlib.util
import json
from pathlib import Path
import sys
import unittest
from unittest import mock

import test_runner as _runner_tests

sys.dont_write_bytecode = True


class DevelopmentTests(unittest.TestCase):
    setUp = _runner_tests.WorkflowTests.setUp
    write = _runner_tests.WorkflowTests.write
    save = _runner_tests.WorkflowTests.save
    no_commands = _runner_tests.WorkflowTests.no_commands

    def specification(self):
        spec = _runner_tests.WorkflowTests.specification(self)
        spec['development'] = {'groups': [
            dict(name='runtime', **spec['stages']['freeze']),
            dict(name='fixture', **{**spec['stages']['freeze'], 'uses': ['fixture']})]}
        return spec

    def module(self, name):
        fullname = self.runner.__package__ + '.' + name
        if fullname not in sys.modules:
            spec = importlib.util.spec_from_file_location(fullname, self.package_root / (name + '.py'))
            module = importlib.util.module_from_spec(spec)
            sys.modules[fullname] = module
            spec.loader.exec_module(module)
        return sys.modules[fullname]

    def run_dev(self):
        return self.module('development').run(self.repo, self.base / 'dev-store', self.spec_path)

    def test_independent_groups_warm_reuse_and_selective_invalidation(self):
        first = self.run_dev()
        self.assertEqual(first['lane'], 'development')
        self.assertFalse(first['completion_eligible'])
        self.assertTrue(all(not row['reused'] for row in first['groups'].values()))
        with self.no_commands():
            warm = self.run_dev()
        self.assertTrue(all(row['reused'] for row in warm['groups'].values()))
        self.write(self.repo / 'src/a.rs', '// changed synthetic runtime\n')
        changed = self.run_dev()
        self.assertFalse(changed['groups']['runtime']['reused'])
        self.assertTrue(changed['groups']['fixture']['reused'])
        self.assertFalse((self.base / 'dev-store' / 'state.json').exists())

    def test_failed_group_cannot_reuse_success(self):
        self.spec['development']['groups'] = [self.spec['development']['groups'][0]]
        self.spec['development']['groups'][0]['commands'][0]['argv'].append('--fail')
        self.save()
        for _ in range(2):
            with self.assertRaises(Exception):
                self.run_dev()
        receipts = list((self.base / 'dev-store').rglob('receipt.json'))
        for path in receipts:
            self.assertNotEqual(json.loads(path.read_text()).get('status'), 'passed')

    def test_production_forbidden_commands_rejected_before_launch(self):
        self.spec['synthetic'] = False
        for argv in (["cargo", "build", "--locked"], ["cargo", "test", "--locked", "--release"],
                     ["cargo", "check", "--locked", "--profile=release"],
                     ["{python}", "{repo}/budget.py"], ["{python}", "{repo}/benchmark.py"],
                     ["{python}", "{repo}/statistics.py"]):
            with self.subTest(argv=argv):
                self.spec['development']['groups'][0]['commands'][0]['argv'] = argv
                self.save()
                with self.no_commands(), self.assertRaises(Exception):
                    self.run_dev()

    def test_production_verbose_unittest_executes_once_then_reuses(self):
        self.write(self.repo / 'test_dev_small.py', 'import unittest\nclass Case(unittest.TestCase):\n def test_small(self): self.assertEqual(2+3,5)\n')
        self.spec['synthetic'] = False
        self.spec['development']['groups'] = [{'name': 'python', 'uses': ['runtime'],
            'commands': [{'name': 'unit', 'argv': ['{python}', '-B', '-m', 'unittest', 'test_dev_small', '-v']}],
            'tests': [{'command': 'unit', 'kind': 'unittest', 'required_names': ['test_small'], 'minimum_passed': 1}]}]
        self.save()
        row = self.run_dev()['groups']['python']
        self.assertEqual(row['tests']['unit']['passed'], 1)
        self.assertFalse(row['reused'])
        with self.no_commands():
            self.assertTrue(self.run_dev()['groups']['python']['reused'])

    def test_derived_development_contract_cannot_run_or_finalize_full_lane(self):
        row = self.run_dev()['groups']['runtime']
        store = Path(row['store'])
        derived = next((store / 'contracts').glob('*.json'))
        self.assertTrue(json.loads(derived.read_text())['development_lane'])
        full = self.runner.Workflow(self.repo, store)
        with self.no_commands():
            with self.assertRaises(Exception): full.run(derived, 'verify')
            with self.assertRaises(Exception): full.finalize(derived, self.base / 'publication')

    def test_cli_without_execute_is_inert_with_absent_contract(self):
        source = Path(__file__).resolve().parents[2] / 'verify.py'
        copied = self.write(self.package_root.parent / 'verify.py', source.read_text())
        spec = importlib.util.spec_from_file_location('synthetic_dev_cli', copied)
        cli = importlib.util.module_from_spec(spec)
        # CLI imports the isolated verification package using its normal name.
        package = sys.modules[self.runner.__package__]
        with mock.patch.dict(sys.modules, {'verification': package}), self.no_commands(), mock.patch('builtins.print') as output:
            spec.loader.exec_module(cli)
            self.assertEqual(cli.main(['--repo', str(self.repo), 'dev', '--contract', str(self.repo / 'absent.json')]), 0)
        self.assertIn('inert', str(output.call_args))
        self.assertFalse((self.repo / 'absent.json').exists())


    def test_actual_cli_close_finalizes_full_phases_and_warm_close_launches_nothing(self):
        self.spec = _runner_tests.WorkflowTests.specification(self)
        self.save()
        # Load the actual CLI source, resolving verification imports to isolated copies.
        source = Path(__file__).resolve().parents[2] / 'verify.py'
        spec = importlib.util.spec_from_file_location('synthetic_close_cli', source)
        cli = importlib.util.module_from_spec(spec)
        aliases = {'verification': sys.modules[self.runner.__package__]}
        for name in ('artifacts', 'runner', 'contracts', 'context', 'finalize'):
            aliases['verification.' + name] = self.module(name)
        store = self.base / 'close-store'
        task = self.write(self.repo / 'TASKS.json', '{"tasks":[{"id":"P999","status":"todo"}]}')
        unchanged = task.read_bytes()
        with mock.patch.dict(sys.modules, aliases):
            spec.loader.exec_module(cli)
            for index in (1, 2):
                output = self.base / ('close-output-' + str(index))
                argv = ['--repo', str(self.repo), '--store', str(store), 'close',
                        '--contract', str(self.spec_path), '--output', str(output), '--execute']
                with mock.patch('builtins.print') as printed:
                    if index == 1:
                        self.assertEqual(cli.main(argv), 0)
                    else:
                        with self.no_commands():
                            self.assertEqual(cli.main(argv), 0)
                result = json.loads(printed.call_args.args[0])
                self.assertEqual(set(result['phases']), {'freeze', 'verify', 'measure'})
                self.assertTrue(all(row['reused'] == (index == 2) for row in result['phases'].values()))
                proof = json.loads(Path(result['evidence']).read_text())
                self.assertTrue(proof['passed'])
                self.assertTrue(proof['synthetic'])
                self.assertEqual(proof['status'], 'passed')
                self.assertEqual(set(proof['phases']), {'freeze', 'verify', 'measure'})
                artifacts = self.module('artifacts').ArtifactStore(store)
                for reference in proof['phases'].values():
                    receipt = artifacts.json(reference)
                    self.assertEqual(receipt['lane'], 'full')
                    self.assertTrue(receipt['completion_eligible'])
                    self.assertEqual(receipt['status'], 'passed')
                self.assertTrue(Path(result['completion_record']).is_file())
                self.assertEqual(task.read_bytes(), unchanged)

if __name__ == '__main__':
    unittest.main()
