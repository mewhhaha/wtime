from __future__ import annotations

import hashlib
import json
from pathlib import Path
import shutil
import sys
import tempfile
import unittest
from unittest.mock import patch

TOOL = Path(__file__).resolve().parents[1]
ROOT = TOOL.parents[1]
if str(TOOL) not in sys.path:
    sys.path.insert(0, str(TOOL))
import selection_manifest as selection  # noqa: E402


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def canonical_digest(value) -> str:
    return digest(json.dumps(value, sort_keys=True, separators=(',', ':')).encode())


class P054SelectionModelTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='wtime-p054-model-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.bench = self.root / 'tools' / 'wtime-bench'
        self.registry_path = self._copy_contained(ROOT / 'tools/wtime-bench/workloads.json',
                                                  self.root / 'registry' / 'workloads.json')
        actual_model = ROOT / 'tools/wtime-bench/scalar_p128_workloads.py'
        self.model_path = self._copy_contained(actual_model, self.bench / 'scalar_p128_workloads.py')
        for source in sorted((ROOT / 'tools/wtime-bench/fixtures/scalar').glob('p128_*.wat')):
            self._copy_contained(source, self.bench / 'fixtures/scalar' / source.name)
        self.registry_before = self.registry_path.read_bytes()
        self.registry_data = json.loads(self.registry_before)
        self.model_hash = digest(self.model_path.read_bytes())
        self.patch_here = patch.object(selection, 'HERE', self.bench)
        self.patch_here.start()
        self.addCleanup(self.patch_here.stop)

    def _copy_contained(self, source: Path, destination: Path) -> Path:
        destination = destination.resolve()
        self.assertTrue(destination.is_relative_to(self.root),
                        f'synthetic copy escaped fresh test root: {destination}')
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, destination)
        return destination

    def _write_json(self, relative: str, value) -> Path:
        destination = (self.root / relative).resolve()
        self.assertTrue(destination.is_relative_to(self.root),
                        f'synthetic output escaped fresh test root: {destination}')
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_text(json.dumps(value, sort_keys=True, indent=2) + '\n')
        return destination

    def _model_rows(self):
        binding = {'fixture_models': [{'name': 'p128_scalar',
                                       'source_sha256': self.model_hash}]}
        return list(selection._fixture_model_rows(binding, {}).values())

    def _manifest(self, with_model=True):
        global_rows = {row['id']: row for row in self.registry_data['workloads']}
        if with_model:
            p128 = {row['id']: row for row in self._model_rows()}
            declarations = [
                ('p128_identity_copy_n1', 'scalar', 'targeted', 'small', ['warm_call']),
                ('p128_literal_wrap_add_n32', 'scalar', 'targeted', 'large', ['first_call']),
                ('p128_literal_bitwise_n17', 'scalar', 'held_out', 'other', ['load_and_call']),
            ]
            selected = {**global_rows, **p128}
        else:
            declarations = [
                ('scalar_accumulator_n1', 'scalar', 'targeted', 'small', ['warm_call']),
                ('scalar_accumulator_n10000', 'scalar', 'targeted', 'large', ['first_call']),
                ('scalar_accumulator_n32', 'scalar', 'held_out', 'other', ['warm_call']),
            ]
            selected = global_rows
        declarations += [
            ('prepare_large', 'original-mvp', 'protected', 'other', ['prepare']),
            ('call_overhead', 'original-mvp', 'protected', 'other', ['first_call', 'load_and_call']),
        ]
        rows = []
        for wid, family, role, scale, stages in declarations:
            rows.append({'family': family, 'id': wid, 'role': role, 'scale': scale,
                         'stages': stages, 'row_sha256': canonical_digest(selected[wid])})
        result = {
            'schema': selection.SCHEMA, 'task': 'P054', 'lane': 'candidate',
            'rationale': 'fixed P128 scalar rows are bound through their independent model',
            'registry_sha256': digest(self.registry_before), 'rows': rows,
            'semantic_cases': ['p054-model-row'],
        }
        if with_model:
            result['fixture_models'] = [{'name': 'p128_scalar', 'source_sha256': self.model_hash}]
        return result

    def test_fixed_model_rows_resolve_and_global_registry_stays_unchanged(self):
        manifest = self._manifest()
        path = self._write_json('manifests/candidate.json', manifest)
        loaded = selection.load(path, self.registry_path)
        rows = selection.rows(loaded, 'scalar', self.registry_path)
        model_rows = {row['id']: row for row in self._model_rows()}
        selected_model = [row for row in rows if row['id'].startswith('p128_')]
        self.assertEqual(len(selected_model), 3)
        for row in selected_model:
            self.assertEqual(row, model_rows[row['id']])
        groups = selection.groups(loaded, self.registry_path)
        self.assertIn(('scalar', 'p128_identity_copy_n1', 'warm_call'), groups)
        self.assertIn(('scalar', 'p128_literal_wrap_add_n32', 'first_call'), groups)
        self.assertIn(('scalar', 'p128_literal_bitwise_n17', 'load_and_call'), groups)
        self.assertEqual(len(self.registry_data['workloads']), 459)
        self.assertEqual(self.registry_path.read_bytes(), self.registry_before)
        self.assertEqual((ROOT / 'tools/wtime-bench/workloads.json').read_bytes(), self.registry_before)

    def test_legacy_manifest_without_fixture_models_remains_valid(self):
        path = self._write_json('manifests/legacy.json', self._manifest(with_model=False))
        loaded = selection.load(path, self.registry_path)
        self.assertNotIn('fixture_models', loaded)
        self.assertTrue(all(not row['id'].startswith('p128_')
                            for row in selection.rows(loaded, registry_path=self.registry_path)))

    def test_bad_binding_unknown_alias_and_global_collision_fail_closed(self):
        bad_cases = []
        unknown_model = self._manifest()
        unknown_model['fixture_models'][0]['name'] = 'arbitrary_module'
        bad_cases.append(('unknown-model', unknown_model, 'unknown or duplicate fixture model'))
        stale = self._manifest()
        stale['fixture_models'][0]['source_sha256'] = '0' * 64
        bad_cases.append(('stale-source', stale, 'source hash mismatch'))
        unknown_alias = self._manifest()
        unknown_alias['rows'][0]['id'] = 'p128_identity_copy_n999'
        bad_cases.append(('unknown-alias', unknown_alias, 'unknown family or workload'))
        for name, manifest, error in bad_cases:
            with self.subTest(name=name):
                path = self._write_json(f'manifests/{name}.json', manifest)
                with self.assertRaisesRegex(ValueError, error):
                    selection.load(path, self.registry_path)

        collision_registry = json.loads(self.registry_before)
        collision_id = 'p128_identity_copy_n1'
        collision_registry['workloads'].append({'id': collision_id, 'stages': ['warm_call']})
        collision_registry['selections']['scalar']['workloads'].append(collision_id)
        collision_path = self._write_json('collision/workloads.json', collision_registry)
        collision_manifest = self._manifest()
        collision_manifest['registry_sha256'] = digest(collision_path.read_bytes())
        with self.assertRaisesRegex(ValueError, 'collides with global registry'):
            selection.load(self._write_json('collision/candidate.json', collision_manifest),
                           collision_path)

    def test_model_row_hash_stage_and_virtual_family_are_checked(self):
        cases = []
        row_hash = self._manifest()
        row_hash['rows'][0]['row_sha256'] = '0' * 64
        cases.append(('row-hash', row_hash, 'row_sha256 mismatch'))
        bad_stage = self._manifest()
        bad_stage['rows'][0]['stages'] = ['setup']
        cases.append(('stage', bad_stage, 'stages must be a unique subset'))
        bad_family = self._manifest()
        bad_family['rows'][0]['family'] = 'simd'
        cases.append(('family', bad_family, 'must use virtual scalar family'))
        for name, manifest, error in cases:
            with self.subTest(name=name):
                path = self._write_json(f'manifests/{name}.json', manifest)
                with self.assertRaisesRegex(ValueError, error):
                    selection.load(path, self.registry_path)

    def test_fixture_model_bindings_require_strict_unique_entries_and_sha_format(self):
        cases = []
        empty = self._manifest()
        empty['fixture_models'] = []
        cases.append(('empty', empty, 'nonempty list'))
        malformed = self._manifest()
        malformed['fixture_models'][0]['path'] = 'elsewhere.py'
        cases.append(('extra-field', malformed, 'exactly name and source_sha256'))
        duplicate = self._manifest()
        duplicate['fixture_models'].append(dict(duplicate['fixture_models'][0]))
        cases.append(('duplicate', duplicate, 'unknown or duplicate fixture model'))
        bad_hash = self._manifest()
        bad_hash['fixture_models'][0]['source_sha256'] = 'not-a-hash'
        cases.append(('bad-hash', bad_hash, 'lowercase SHA-256'))
        for name, manifest, error in cases:
            with self.subTest(name=name):
                path = self._write_json(f'manifests/{name}.json', manifest)
                with self.assertRaisesRegex(ValueError, error):
                    selection.load(path, self.registry_path)


if __name__ == '__main__':
    unittest.main()
