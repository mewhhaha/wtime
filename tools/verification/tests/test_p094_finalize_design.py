"""Synthetic tests for the phase-bound P094 design outcome."""
import json
from pathlib import Path
import tempfile
import unittest
import sys
from unittest import mock

REPOSITORY_ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(REPOSITORY_ROOT / 'tools'))

from verification import finalize as finalizer
from verification.artifacts import VerificationError, digest


class _Store:
    def __init__(self, decision):
        self.decision = decision

    def json(self, _artifact):
        return {'decision': self.decision}


class _Workflow:
    def __init__(self, decision):
        self.root = Path('/synthetic/store')
        self.store = _Store(decision)

    def _receipt(self, _task, phase):
        return {'phase': phase}, {
            'artifacts': {'decision': 'synthetic-artifact'},
            'completion_eligible': True,
            'lane': 'full',
            'input_key': digest({}),
        }

    def _inputs(self, _spec, _phase):
        return {}

    def _revalidate(self, _spec, _phase, _receipt):
        return None


class DesignDecisionFinalizerTests(unittest.TestCase):
    def run_case(self, completion, artifact_decision=None):
        with tempfile.TemporaryDirectory(prefix='p094-finalize-', dir='/tmp') as temporary:
            root = Path(temporary).resolve()
            spec_path = root / 'spec.json'
            spec_path.write_text('{}\n')
            output = root / 'final'
            spec = {'task': 'P094', 'stages': {'verify': {}}, 'completion': completion}
            workflow = _Workflow(artifact_decision)
            with mock.patch.object(finalizer, 'load_spec', return_value=spec):
                result = finalizer.finalize(workflow, spec_path, output)
                evidence = json.loads((output / 'evidence.json').read_text())
                return result, evidence

    @staticmethod
    def decision_contract(outcome, **extra):
        return {
            'required_phases': ['verify'],
            'decision': {'phase': 'verify', 'artifact': 'decision'},
            **extra,
        }, outcome

    def test_design_contract_accepts_only_go_and_no_go_from_phase_artifact(self):
        for outcome in ('go', 'no_go'):
            with self.subTest(outcome=outcome):
                config, artifact = self.decision_contract(outcome, design_decision=True)
                result, evidence = self.run_case(config, artifact)
                self.assertEqual(result['outcome'], outcome)
                self.assertEqual(evidence['decision'], outcome)

    def test_design_contract_rejects_each_ordinary_outcome(self):
        for outcome in ('adopted', 'rejected', 'inconclusive', 'contract_only'):
            with self.subTest(outcome=outcome), self.assertRaises(VerificationError):
                config, artifact = self.decision_contract(outcome, design_decision=True)
                self.run_case(config, artifact)

    def test_ordinary_contract_keeps_existing_outcomes_and_rejects_design_outcomes(self):
        for outcome in ('adopted', 'rejected', 'inconclusive', 'contract_only'):
            with self.subTest(outcome=outcome):
                config, artifact = self.decision_contract(outcome)
                result, evidence = self.run_case(config, artifact)
                self.assertEqual(result['outcome'], outcome)
                self.assertEqual(evidence['outcome'], outcome)
        static, _ = self.run_case({'required_phases': ['verify'], 'outcome': 'contract_only'})
        self.assertEqual(static['outcome'], 'contract_only')
        for outcome in ('go', 'no_go'):
            with self.subTest(outcome=outcome), self.assertRaises(VerificationError):
                config, artifact = self.decision_contract(outcome)
                self.run_case(config, artifact)

    def test_design_flag_requires_an_exact_boolean(self):
        for value in (1, 0, None, 'true', [], {}, 1.0):
            with self.subTest(value=value), self.assertRaises(VerificationError):
                config, artifact = self.decision_contract('go', design_decision=value)
                self.run_case(config, artifact)

    def test_design_contract_requires_a_verified_decision_artifact(self):
        with self.assertRaises(VerificationError):
            self.run_case({'required_phases': ['verify'], 'outcome': 'go', 'design_decision': True})

    def test_final_outcome_must_be_a_string(self):
        with self.assertRaises(VerificationError):
            config, _ = self.decision_contract(True)
            self.run_case(config, True)


if __name__ == '__main__':
    unittest.main()
