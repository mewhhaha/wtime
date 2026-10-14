"""Synthetic P034 model tests only; no CLI/Cargo/runtime or installed registry writes."""
import copy
import hashlib
import json
from pathlib import Path
import shutil
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent
TOOL = HERE.parent if HERE.name == 'tests' else HERE
sys.path.insert(0, str(TOOL))
import gc_layout_workloads as model
import workloads


def predecessor_registry():
    registry = json.loads(model.BASE_REGISTRY.read_bytes())
    if len(registry['workloads']) == 386:
        expected = model.entries()
        if registry['workloads'][350:] != expected:
            raise AssertionError('installed P034 appendix differs from independently hash-bound model')
        old_ids = registry['selections']['gc-exceptions']['workloads']
        if registry['selections']['gc-layouts'] != {'status': 'ready', 'workloads': old_ids + [row['id'] for row in expected]}:
            raise AssertionError('installed gc-layouts selection differs from exact81+36 model')
        registry['workloads'] = registry['workloads'][:350]
        del registry['selections']['gc-layouts']
    if len(registry['workloads']) != 350:
        raise AssertionError('wrong predecessor size')
    return registry


class ModelTests(unittest.TestCase):
    def test_independent_oracle_bounds_and_input_identity(self):
        for family in model.FAMILIES:
            for n in model.INPUTS:
                self.assertEqual(model.oracle(f'{family}_n{n}', n), 255*n)
        for n in [-1, 10001, True, 2]:
            with self.assertRaises(ValueError):
                model.oracle(f'gc_layout_array_16_n{n}', n)
        for wid in ['gc_layout_array_16_n017', 'gc_layout_array_16_n32', 'unknown_n17']:
            with self.assertRaises(ValueError):
                model.oracle(wid, 17)
        self.assertEqual(model.max_batch(0), 100000)
        self.assertEqual(model.max_batch(10000), 10000)

    def test_36_rows_bind_actual_three_encoded_fixtures_and_one_allocation(self):
        identities = model.fixture_identities()
        self.assertEqual(len(identities), 3)
        rows = model.entries()
        self.assertEqual(len(rows), 36)
        self.assertEqual(len({r['id'] for r in rows}), 36)
        self.assertEqual(len({r['source']['path'] for r in rows}), 3)
        for row in rows:
            n = row['params'][0]['baseline']
            self.assertEqual(row['work_units']['count'], n)
            self.assertEqual(row['gc_expectation']['allocation_count'], 1)
            self.assertEqual(model.oracle(row['id'], n), 255*n)
            self.assertEqual(row['stages'], model.STAGES)
            self.assertEqual(row['driver']['export'], 'run' if '_array_' in row['id'] else 'run_struct')
            self.assertEqual(row['comparators']['wasmtime']['status'], 'unsupported')

    def test_prefix_selections_compact_logical_hashes_and_contract_counts(self):
        original = predecessor_registry()
        before = copy.deepcopy(original)
        proposal = model.append_to_registry(original)
        self.assertEqual(original, before)
        self.assertEqual(proposal['workloads'][:350], original['workloads'])
        for name, selected in original['selections'].items():
            self.assertEqual(proposal['selections'][name], selected)
        self.assertEqual(len(proposal['workloads']), 386)
        self.assertEqual(len(proposal['selections']['gc-exceptions']['workloads']), 81)
        self.assertEqual(len(proposal['selections']['gc-layouts']['workloads']), 117)
        self.assertEqual(workloads.validate_registry(proposal), proposal)
        wire = model.compact_registry_bytes(proposal)
        self.assertLess(len(wire), model.REGISTRY_BYTE_CAP)
        decoded = json.loads(wire)
        self.assertEqual(model.canonical_sha(decoded['workloads'][:350]), model.canonical_sha(original['workloads']))
        self.assertEqual(decoded, proposal)
        contract = model.contract(original)
        self.assertEqual(len(contract['primary_workloads']), 24)
        self.assertEqual(len(contract['protected_workloads']), 93)
        self.assertFalse(set(contract['primary_workloads']) & set(contract['protected_workloads']))
        self.assertEqual(set(contract), {'schema_version', 'selection', 'primary_workloads', 'protected_workloads'})
        measurement = model.measurement_contract(original)
        self.assertEqual((measurement['groups_per_run'], measurement['groups_total'], measurement['pairs_total'],
                          measurement['warm_calibrations_total']), (405, 810, 24300, 144))
        self.assertEqual(measurement['workload_sides_per_run'], 234)
        self.assertEqual(measurement['gate_contract_canonical_sha256'], model.canonical_sha(contract))
        self.assertEqual(model.selected_matrix(proposal), model.EXPECTED_MATRIX)

    def test_missing_changed_fixture_and_duplicate_append_fail_closed(self):
        # Every synthetic file and deletion remains below this fresh root.
        scratch = Path('/tmp/P034-support/synthetic-tmp')
        if scratch.is_symlink() or not scratch.resolve().is_relative_to(Path('/tmp/P034-support').resolve()):
            raise ValueError('synthetic scratch must remain under staging root')
        scratch.mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(prefix='case-', dir=scratch) as tmp:
            root = Path(tmp)
            self.assertTrue(root.resolve().is_relative_to(scratch.resolve()))
            copied = root / 'fixtures'
            shutil.copytree(model.FIXTURE_DIR, copied)
            wasm = copied / 'packed_gc_types_16.wasm'
            with patch.object(model, 'FIXTURE_DIR', copied):
                wasm.write_bytes(wasm.read_bytes() + b'changed')
                with self.assertRaises(ValueError):
                    model.entries()
                wasm.unlink()
                with self.assertRaises(FileNotFoundError):
                    model.entries()
        registry = predecessor_registry()
        registry['selections']['gc-layouts'] = {'status': 'pending', 'workloads': []}
        with self.assertRaises(ValueError):
            model.append_to_registry(registry)


if __name__ == '__main__':
    unittest.main()
