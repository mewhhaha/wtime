"""Contained synthetic checks for combining source-bound P053 policy decisions."""
from __future__ import annotations

import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[3]
PRODUCER_PATH = ROOT / 'tools/verification/producers/p053_decision.py'
spec = importlib.util.spec_from_file_location('p053_decision_tests', PRODUCER_PATH)
producer = importlib.util.module_from_spec(spec)
assert spec and spec.loader
spec.loader.exec_module(producer)


def digest(path: Path) -> str:
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def put(root: Path, relative: str, data: bytes) -> Path:
    """Write fixture bytes only after proving the resolved target is contained."""
    path = (root / relative).resolve()
    if not path.is_relative_to(root):
        raise AssertionError(f'fixture path escapes temporary root: {path}')
    path.parent.mkdir(parents=True, exist_ok=True)
    if not path.is_relative_to(root):
        raise AssertionError(f'fixture parent escapes temporary root: {path.parent}')
    path.write_bytes(data)
    return path


class P053DecisionTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='p053-decision-')
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()

    def make_record(self, policy: str, verdict: str, label: str):
        setting = producer.p127_fuel_policy.policy(policy)
        report = {
            'metadata': {'settings': {'fuel_policy': setting}},
            'passed': True,
            'source_stable': True,
            'lane': 'candidate',
            'selection': 'P053',
        }
        report_path = put(self.root, f'{label}/report.json', json.dumps(report).encode())
        shared = {}
        for name in ('build', 'optional_build', 'contract', 'candidate_build', 'task_plan'):
            path = put(self.root, f'shared/{name}.json', f'{name}:synthetic\n'.encode())
            shared[name] = {'path': str(path), 'sha256': digest(path)}
        inputs = {'report': {'path': str(report_path), 'sha256': digest(report_path)}, **shared}
        record = {
            'schema': 'wtime-verification-statistics-v1',
            'task': 'P053',
            'status': 'passed',
            'statistics_computations': 1,
            'timing_evidence': True,
            'inputs': inputs,
            'decision': {'decision': verdict},
        }
        record_path = put(self.root, f'{label}/statistics.json', json.dumps(record).encode())
        return record_path, record, report_path

    def test_two_distinct_typed_policy_datasets_aggregate_mixed_verdict(self):
        unmetered, _, _ = self.make_record('unmetered', 'adopted', 'unmetered')
        metered, _, _ = self.make_record('metered', 'inconclusive', 'metered')
        result = producer.combine(unmetered, metered)
        self.assertEqual(result['decision']['decision'], 'inconclusive')
        self.assertEqual(result['statistics_computations'], 2)
        self.assertEqual(result['additional_statistics_computations'], 0)
        refs = result['policy_records']
        self.assertEqual([item['policy'] for item in refs], [
            producer.p127_fuel_policy.policy('unmetered'),
            producer.p127_fuel_policy.policy('metered')])
        self.assertNotEqual(refs[0]['report']['sha256'], refs[1]['report']['sha256'])
        self.assertNotEqual(refs[0]['path'], refs[1]['path'])
        self.assertTrue(all(Path(item['path']).resolve().is_relative_to(self.root) for item in refs))

    def test_rejected_policy_takes_precedence_over_other_verdicts(self):
        unmetered, _, _ = self.make_record('unmetered', 'rejected', 'unmetered')
        metered, _, _ = self.make_record('metered', 'adopted', 'metered')
        self.assertEqual(producer.combine(unmetered, metered)['decision']['decision'], 'rejected')

    def test_wrong_policy_missing_statistics_and_stale_consumed_input_reject(self):
        unmetered, record, report = self.make_record('unmetered', 'adopted', 'unmetered')
        metered, _, _ = self.make_record('metered', 'adopted', 'metered')
        wrong_report = json.loads(report.read_text())
        wrong_report['metadata']['settings']['fuel_policy'] = producer.p127_fuel_policy.policy('metered')
        report.write_text(json.dumps(wrong_report))
        record['inputs']['report']['sha256'] = digest(report)
        unmetered.write_text(json.dumps(record))
        with self.assertRaisesRegex(ValueError, 'wrong source-fuel policy'):
            producer.combine(unmetered, metered)

        record['status'] = 'running'
        unmetered.write_text(json.dumps(record))
        with self.assertRaisesRegex(ValueError, 'completed statistics record'):
            producer.combine(unmetered, metered)

        record['status'] = 'passed'
        unmetered.write_text(json.dumps(record))
        consumed = Path(record['inputs']['contract']['path'])
        consumed.write_bytes(b'changed after statistics')
        with self.assertRaisesRegex(ValueError, 'statistics input changed'):
            producer.combine(unmetered, metered)

    def test_different_candidate_build_or_declaration_inputs_reject(self):
        for key in ('build', 'candidate_build', 'contract', 'task_plan'):
            with self.subTest(input=key):
                unmetered, record, _ = self.make_record('unmetered', 'adopted', f'u-{key}')
                metered, other, _ = self.make_record('metered', 'adopted', f'm-{key}')
                changed = put(self.root, f'm-{key}/different-{key}.json', b'different identity')
                other['inputs'][key] = {'path': str(changed), 'sha256': digest(changed)}
                metered.write_text(json.dumps(other))
                with self.assertRaisesRegex(ValueError, 'different candidate/build/declaration'):
                    producer.combine(unmetered, metered)

    def test_missing_statistics_field_rejected(self):
        unmetered, record, _ = self.make_record('unmetered', 'adopted', 'unmetered')
        metered, _, _ = self.make_record('metered', 'adopted', 'metered')
        del record['statistics_computations']
        unmetered.write_text(json.dumps(record))
        with self.assertRaises((KeyError, ValueError)):
            producer.combine(unmetered, metered)


if __name__ == '__main__':
    unittest.main()
