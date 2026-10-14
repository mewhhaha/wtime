from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

BENCH = Path(__file__).resolve().parents[1]
ROOT = BENCH.parents[1]
if str(BENCH) not in sys.path:
    sys.path.insert(0, str(BENCH))

import compare
import focused_compare
import scalar_p128_workloads


def contained_output(root: Path) -> Path:
    root = root.resolve()
    output = (root / 'output').resolve()
    if root not in output.parents:
        raise AssertionError(f'output escaped fresh test root: {output}')
    output.mkdir()
    return output


def scalar_wire(entry, n, expected, stage, batch, mode, profile, driver):
    return {
        'schema_version': 2,
        'case': entry['id'],
        'stage': stage,
        'input': f'i32:{n}',
        'expected': f'i32:{expected}',
        'result': f'i32:{expected}',
        'batch': batch,
        'ns_per_operation': 2.5,
        'mode': mode,
        'profile': profile,
        'driver': driver,
        'diagnostic': False,
    }


class P054ScalarRouteTests(unittest.TestCase):
    def test_each_admitted_p128_shape_uses_typed_scalar_compare_wire(self):
        rows = [row for row in scalar_p128_workloads.entries()
                if row['oracle_arguments']['n'] == 17]
        self.assertEqual(len(rows), 3)
        for entry in rows:
            with self.subTest(workload=entry['id']), tempfile.TemporaryDirectory() as td:
                root = Path(td).resolve()
                output = contained_output(root)
                fixture = root / 'synthetic-fixture.wasm'
                n = entry['oracle_arguments']['n']
                expected = scalar_p128_workloads.oracle(entry['id'], n)
                seen = []

                def process(executable, argv, cpu):
                    seen.append((executable, list(argv), cpu))
                    wire_entry = {'id': argv[2]}
                    return scalar_wire(wire_entry, n, expected, argv[5], int(argv[6]),
                                       argv[7], argv[8], argv[9])

                with patch.object(compare, 'process', side_effect=process):
                    observed = focused_compare.observe(
                        'candidate', entry, str(fixture), 'warm_call', 3,
                        'compact', 2, output)

                self.assertEqual(len(seen), 1)
                self.assertEqual(seen[0][1], [
                    '--scalar-driver', str(fixture), entry['id'], 17, expected,
                    'warm_call', 3, 'compact', 'mvp', 'plain'])
                self.assertEqual(observed['input'], n)
                self.assertEqual(observed['expected'], expected)
                self.assertEqual(observed['result'], expected)
                self.assertEqual(observed['batch'], 3)
                self.assertEqual(observed['mode'], 'compact')
                self.assertEqual(observed['profile'], 'mvp')
                self.assertEqual(observed['driver'], 'plain')
                self.assertIn(root, output.parents)
                self.assertIn(root, fixture.parents)

    def test_legacy_scalar_route_keeps_its_existing_argv(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td).resolve()
            output = contained_output(root)
            entry = {
                'id': 'scalar_accumulator_n17', 'family': 'scalar_accumulator',
                'oracle': {'module': 'scalar_workloads.py'},
                'params': [{'name': 'n', 'type': 'i32', 'baseline': 17}],
                'driver': {'profile': 'mvp', 'kind': 'plain'},
            }
            calls = []

            def process(executable, argv, cpu):
                calls.append(list(argv))
                return scalar_wire({'id': entry['id']}, 17, 136,
                                   argv[5], int(argv[6]), argv[7], argv[8], argv[9])

            with patch.object(compare, 'process', side_effect=process):
                observed = focused_compare.observe(
                    'candidate', entry, str(root / 'legacy.wasm'),
                    'warm_call', 3, 'decoded', 1, output)
            self.assertEqual(calls[0], [
                '--scalar-driver', str(root / 'legacy.wasm'), 'scalar_accumulator_n17',
                17, 136, 'warm_call', 3, 'decoded', 'mvp', 'plain'])
            self.assertEqual(observed['result'], 136)
            self.assertNotIn('source_fuel', observed)
            self.assertIn(root, output.parents)

    def test_p128_rejects_unknown_case_wrong_oracle_or_family_before_process(self):
        mutations = ('id', 'oracle', 'family')
        for mutation in mutations:
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as td:
                root = Path(td).resolve()
                output = contained_output(root)
                entry = next(row for row in scalar_p128_workloads.entries()
                             if row['oracle_arguments']['n'] == 17)
                entry = dict(entry)
                if mutation == 'id':
                    entry['id'] = 'p128_identity_copy_n999'
                elif mutation == 'oracle':
                    entry['oracle'] = dict(entry['oracle'], function='wrong_oracle')
                else:
                    entry['family'] = 'unsupported_p128_family'
                calls = []
                with patch.object(compare, 'process', side_effect=lambda *args: calls.append(args)), \
                     self.assertRaises((ValueError, compare.ComparisonError)):
                    focused_compare.observe('candidate', entry, str(root / 'fixture.wasm'),
                                            'first_call', 1, 'decoded', 0, output)
                self.assertEqual(calls, [])
                self.assertIn(root, output.parents)

    def test_p128_rejects_invalid_stage_driver_and_source_fuel(self):
        mutations = ('stage', 'driver', 'source_fuel')
        for mutation in mutations:
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as td:
                root = Path(td).resolve()
                output = contained_output(root)
                entry = next(row for row in scalar_p128_workloads.entries()
                             if row['oracle_arguments']['n'] == 17)
                if mutation == 'stage':
                    stage, fuel = 'setup', None
                elif mutation == 'driver':
                    entry = dict(entry, driver={'kind': 'unsupported', 'profile': 'mvp'})
                    stage, fuel = 'first_call', None
                else:
                    stage, fuel = 'first_call', {'source_fuel': 50_000_000}
                calls = []
                with patch.object(compare, 'process', side_effect=lambda *args: calls.append(args)), \
                     self.assertRaises((ValueError, compare.ComparisonError)):
                    focused_compare.observe('candidate', entry, str(root / 'fixture.wasm'),
                                            stage, 1, 'decoded', 0, output,
                                            fuel_setting=fuel)
                self.assertEqual(calls, [])
                self.assertIn(root, output.parents)


if __name__ == '__main__':
    unittest.main()
