from __future__ import annotations

import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

BENCH = Path(__file__).resolve().parents[1]
ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(BENCH))
import candidate_gate
import p127_fuel_policy


class P053FuelGateTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.entry = {'id': 'scalar_accumulator_n17',
                      'oracle': {'module': 'scalar_workloads.py'}}
        self.entries = [self.entry]
        self.capabilities = {
            side: {'source_fuel_contract': p127_fuel_policy.protocol_contract()}
            for side in ('baseline', 'candidate')
        }

    def setting_and_identities(self, setting, token=None):
        command_path = self.root / 'command.json'
        argv = ['focused_compare.py']
        if token is not None:
            argv += ['--source-fuel', token]
        command_path.write_text(json.dumps({'argv': argv}))
        self.assertTrue(command_path.resolve().is_relative_to(self.root))
        return {'command': {'path': str(command_path)} }

    def test_legacy_false_and_both_explicit_policies(self):
        self.assertIsNone(candidate_gate.validate_fuel_setting(
            False, self.entries, {}, self.capabilities))
        for name, token in (('unmetered', 'none'), ('metered', '50000000')):
            setting = p127_fuel_policy.policy(name)
            identities = self.setting_and_identities(setting, token)
            self.assertEqual(candidate_gate.validate_fuel_setting(
                setting, self.entries, identities, self.capabilities), setting)

    def test_unsupported_family_and_malformed_or_typed_budget_rejected(self):
        setting = p127_fuel_policy.policy('metered')
        identities = self.setting_and_identities(setting, '50000000')
        bad_entries = [{'id': 'memory_probe', 'oracle': {'module': 'memory_workloads.py'}}]
        with self.assertRaisesRegex(ValueError, 'only by scalar'):
            candidate_gate.validate_fuel_setting(setting, bad_entries, identities, self.capabilities)
        for bad in ({}, {'name': 'metered'}, {'name': 'metered', 'source_fuel': None},
                    {'name': 'metered', 'source_fuel': True},
                    {'name': 'metered', 'source_fuel': 50_000_000.0},
                    {'name': 'unmetered', 'source_fuel': 50_000_000}):
            with self.subTest(setting=bad), self.assertRaises(ValueError):
                candidate_gate.validate_fuel_setting(bad, self.entries, identities, self.capabilities)

    def test_archived_command_requires_exact_source_fuel_token(self):
        setting = p127_fuel_policy.policy('metered')
        for token in (None, 'none', '050000000'):
            identities = self.setting_and_identities(setting, token)
            with self.subTest(token=token), self.assertRaisesRegex(ValueError, 'archived command'):
                candidate_gate.validate_fuel_setting(setting, self.entries, identities, self.capabilities)

    def test_both_protocol_capabilities_must_advertise_contract(self):
        setting = p127_fuel_policy.policy('unmetered')
        identities = self.setting_and_identities(setting, 'none')
        for side in ('baseline', 'candidate'):
            bad = {key: dict(value) for key, value in self.capabilities.items()}
            bad[side]['source_fuel_contract'] = {'schema_version': 1, 'accepted': []}
            with self.subTest(side=side), self.assertRaisesRegex(ValueError, 'both paired sides'):
                candidate_gate.validate_fuel_setting(setting, self.entries, identities, bad)

    def test_paired_probe_and_calibration_observations_reject_policy_drift(self):
        setting = p127_fuel_policy.policy('metered')
        valid = {'mode': 'decoded', 'result': 123, 'fuel_policy': 'metered',
                 'source_fuel': 50_000_000}
        drifted = [dict(valid), dict(valid), dict(valid)]
        drifted[0].pop('source_fuel')       # timed pair
        drifted[1]['fuel_policy'] = 'unmetered'  # untimed probe
        drifted[2]['source_fuel'] = True    # calibration attempt; bool is not an integer budget
        for context, observation in zip(('paired', 'probe', 'calibration'), drifted):
            with self.subTest(context=context), patch.object(candidate_gate, 'expected', return_value=123):
                with self.assertRaises(ValueError):
                    candidate_gate.check_observation(self.entry, observation, 'decoded', self.root,
                                                     fuel_setting=setting)


if __name__ == '__main__':
    unittest.main()
