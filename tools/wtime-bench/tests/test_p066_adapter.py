"""Contained synthetic checks for the P066 frozen typed-adapter draft."""
from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True
DRAFT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('p066_adapter_draft', DRAFT/'p123_adapter.py')
adapter = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = adapter
spec.loader.exec_module(adapter)


class P066AdapterDraftTests(unittest.TestCase):
    def test_historical_compact_manifest_and_register_numeric_capability(self):
        record = {'mode': 'compact', 'legacy_api': False,
                  'build': {'features': ['runtime-only', 'compact-executor']}}
        manifest = {'features': {'default': ['runtime-only', 'compact-executor'],
                    'runtime-only': [], 'legacy-api': [],
                    'compact-executor': ['wtime-runtime/compact-executor']}}
        protocol = {'engine': 'wtime', 'instrumented': False, 'modes': ['compact']}
        adapter.validate_mode_contract(record, manifest, protocol)
        register = {'mode': 'register', 'legacy_api': False,
                    'build': {'features': ['runtime-only', 'register-executor']}}
        manifest['features']['default'] = register['build']['features']
        manifest['features']['register-executor'] = ['wtime-runtime/register-executor']
        protocol.update(modes=['register'], register_entry_threshold_contract=dict(adapter.REGISTER_ENTRY_CONTRACT))
        adapter.validate_mode_contract(register, manifest, protocol)
        for key in ('schema_version', 'minimum'):
            bad = dict(protocol, register_entry_threshold_contract=dict(adapter.REGISTER_ENTRY_CONTRACT, **{key: True}))
            with self.assertRaises(ValueError):
                adapter.validate_mode_contract(register, manifest, bad)

    def test_explicit_independent_run_seeds_preserve_legacy_schedule(self):
        sys.path.insert(0, str(DRAFT))
        self.addCleanup(sys.path.remove, str(DRAFT))
        import focused_compare
        self.assertEqual(focused_compare.run_seeds(7), {0: 7, 1: 1000010})
        self.assertEqual(focused_compare.run_seeds(7, [66001, 66002]), {0: 66001, 1: 66002})
        self.assertEqual(focused_compare.run_seeds(7, [66001, 66002], True), {0: 66001})
        for bad in ([1, 1], [True, 2], [-1, 2], [1, 2**64], [1]):
            with self.assertRaises(ValueError):
                focused_compare.run_seeds(7, bad)

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='p066-adapter-draft-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()

    def write_inside(self, path: Path, payload: bytes):
        path = path.resolve()
        path.relative_to(self.root)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(payload)
        return path

    def test_register_mode_requires_current_api_and_exact_feature_set(self):
        data = {'mode': 'register', 'legacy_api': False}
        file = self.write_inside(self.root/'inputs/mode.json', json.dumps(data).encode())
        self.assertEqual(json.loads(file.read_text()), data)
        self.assertEqual(adapter.adapter_features('register', False),
                         ['runtime-only', 'register-executor'])
        self.assertEqual(adapter.adapter_features('decoded', True),
                         ['runtime-only', 'legacy-api'])
        with self.assertRaisesRegex(ValueError, 'legacy API'):
            adapter.adapter_features('register', True)
        with self.assertRaisesRegex(ValueError, 'unsupported adapter mode'):
            adapter.adapter_features('adaptive', False)

    def test_register_verifier_requires_forwarded_feature_and_capability(self):
        features = adapter.adapter_features('register', False)
        cargo = {'features': {
            'default': features, 'runtime-only': [], 'legacy-api': [],
            'compact-executor': ['wtime-runtime/compact-executor'],
            'register-executor': ['wtime-runtime/register-executor'],
        }}
        protocol = {'engine': 'wtime', 'instrumented': False, 'modes': ['decoded', 'register'],
                    'register_entry_threshold_contract': adapter.REGISTER_ENTRY_CONTRACT}
        record = {'mode': 'register', 'legacy_api': False,
                  'build': {'features': features}}
        config = self.write_inside(self.root/'adapter/config.json',
                                   json.dumps({'record': record, 'cargo': cargo,
                                               'protocol': protocol}).encode())
        content = json.loads(config.read_text())
        adapter.validate_mode_contract(content['record'], content['cargo'], content['protocol'])

        wrong_features = json.loads(json.dumps(content))
        wrong_features['record']['build']['features'] = ['runtime-only']
        with self.assertRaisesRegex(ValueError, 'mode/feature'):
            adapter.validate_mode_contract(wrong_features['record'], wrong_features['cargo'],
                                           wrong_features['protocol'])
        missing_capability = json.loads(json.dumps(content))
        missing_capability['protocol'].pop('register_entry_threshold_contract')
        with self.assertRaisesRegex(ValueError, 'threshold capability'):
            adapter.validate_mode_contract(missing_capability['record'], missing_capability['cargo'],
                                           missing_capability['protocol'])

    def test_fresh_cargo_build_uses_explicit_target_dir_argument(self):
        target = (self.root/'fresh-target').resolve()
        target.relative_to(self.root)
        target.mkdir()
        argv = ['cargo','build','--locked','--offline','--release','--manifest-path',
                str(self.root/'Cargo.toml'),'-j','4',*adapter._cargo_target_args(target)]
        log = self.write_inside(self.root/'logs/build.command.json',
                                json.dumps({'argv': argv, 'cwd': str(self.root)}).encode())
        recorded = json.loads(log.read_text())['argv']
        self.assertEqual(recorded[-2:], ['--target-dir', str(target)])
        self.assertEqual(recorded[1], 'build')


if __name__ == '__main__':
    unittest.main()
