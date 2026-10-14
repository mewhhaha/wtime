"""Contained synthetic checks for P055's source-bound dispatch aggregator."""
from __future__ import annotations

import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[3]
import sys
sys.path.insert(0, str(ROOT / 'tools'))
from verification.artifacts import ArtifactStore
PRODUCER_PATH = ROOT / 'tools/verification/producers/p055_decision.py'
spec = importlib.util.spec_from_file_location('p055_decision_tests', PRODUCER_PATH)
producer = importlib.util.module_from_spec(spec)
assert spec and spec.loader
spec.loader.exec_module(producer)


def put(root: Path, relative: str, data: bytes) -> Path:
    path = (root / relative).resolve()
    if not path.is_relative_to(root):
        raise AssertionError(f'fixture path escapes temporary root before write: {path}')
    path.parent.mkdir(parents=True, exist_ok=True)
    if not path.is_relative_to(root):
        raise AssertionError(f'fixture parent escapes temporary root before write: {path.parent}')
    path.write_bytes(data)
    return path


def write_json(root: Path, relative: str, value) -> Path:
    return put(root, relative, json.dumps(value, sort_keys=True).encode())


def ref(path: Path) -> dict:
    path = path.resolve(strict=True)
    return {'path': str(path), 'sha256': hashlib.sha256(path.read_bytes()).hexdigest(),
            'bytes': path.stat().st_size}


def artifact(root: Path, data: bytes) -> dict:
    digest = hashlib.sha256(data).hexdigest()
    source = put(root, f'artifact-inputs/{digest}.bin', data)
    source.chmod(0o600)
    path = (root / 'target/verification').resolve()
    if not path.is_relative_to(root):
        raise AssertionError(f'artifact store escapes temporary root before construction: {path}')
    return ArtifactStore(path).add(source)


def canonical_digest(value) -> str:
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(',', ':'),
                                     allow_nan=False).encode()).hexdigest()


class P055DecisionTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='p055-decision-')
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()

    def shared_identities(self, baseline_label='shared'):
        result = {}
        for name in ('selection_manifest', 'selection_freeze', 'baseline_manifest'):
            label = baseline_label if name == 'baseline_manifest' else 'shared'
            content = f'{name}:{label}\n'.encode()
            path = put(self.root, f'common/{name}-{label}.json', content)
            result[name] = ref(path)
        return result

    def make_variant(self, variant: str, verdict: str, *, label=None,
                     baseline_label='shared', actual_shape=None):
        label = label or variant
        expected_shape = producer.VARIANTS[variant]
        shape = actual_shape or expected_shape
        source_text = (
            'enum Shape { Split, Handlers }\n'
            f'const SELECTED: Shape = Shape::{shape};\n'
        ).encode()
        source_path = put(self.root, f'{label}/source/dispatch_experiment.rs', source_text)
        source_sha = hashlib.sha256(source_text).hexdigest()
        binary = put(self.root, f'{label}/bin/wtime-bench', f'{label} binary bytes\n'.encode())
        source_manifest = write_json(self.root, f'{label}/source-manifest.json', {
            'source_identities': {producer.SOURCE: source_sha},
            'archived_sources': {producer.SOURCE: {'path': str(source_path), 'sha256': source_sha}},
        })
        build_record_path = write_json(self.root, f'{label}/production-record.json', {
            'schema': 'wtime-runtime-only-production-v1', 'mode': 'compact',
            'source_stable': True, 'instrumented': False, 'exit_code': 0,
            'binary_path': str(binary),
            'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
            'source_identities': {producer.SOURCE: source_sha},
            'archived_sources': {producer.SOURCE: {'path': str(source_path), 'sha256': source_sha}},
            'source_manifest': {'path': str(source_manifest), 'sha256': hashlib.sha256(source_manifest.read_bytes()).hexdigest()},
        })
        default_build = write_json(self.root, f'{label}/default-build.json',
                                   {'budget_pass': True, 'source_stable': True})
        compact_build = write_json(self.root, f'{label}/compact-build.json',
                                   {'budget_pass': True, 'source_stable': True})
        common = self.shared_identities(baseline_label)
        semantic_inputs = {
            'components': {'runtime': {'files': {producer.SOURCE: source_sha}}},
            'definition': {'tests': [
                {'command': 'compact-semantics', 'kind': 'cargo',
                 'required_names': sorted(producer.REQUIRED_TESTS), 'minimum_passed': 3, 'features': []},
                {'command': 'python-tooling', 'kind': 'unittest',
                 'required_names': ['test_p055_fixture'], 'minimum_passed': 1},
            ]},
            'source_delta': {'baseline_sha256': common['selection_freeze']['sha256'],
                             'delta': {'changed': [producer.SOURCE]}},
        }
        log = ''.join(f'test {name} ... ok\n' for name in sorted(producer.REQUIRED_TESTS))
        log += 'test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s\n'
        archived_log = artifact(self.root, log.encode())
        archived_stderr = artifact(self.root, b'')
        python_log = b'test_p055_fixture (P055Tests.test_p055_fixture) ... ok\n\nRan 1 test in 0.001s\n\nOK\n'
        archived_python_log = artifact(self.root, python_log)
        elf_probe = artifact(self.root, b'\x7fELF' + b'gzip artifact test' * 20)
        output_artifacts = {
            'default_build': artifact(self.root, default_build.read_bytes()),
            'compact_build': artifact(self.root, compact_build.read_bytes()),
            'production_record': artifact(self.root, build_record_path.read_bytes()),
        }
        semantic_receipt = write_json(self.root, f'target/verification/runs/P055/verify/{label}/receipt.json', {
            'schema': 'wtime-verification-phase-v1', 'task': 'P055', 'phase': 'verify',
            'status': 'passed', 'lane': 'full', 'completion_eligible': True, 'synthetic': False,
            'input_key': canonical_digest(semantic_inputs), 'inputs': semantic_inputs,
            'tests': {'compact-semantics': {'passed': 3, 'failed': 0, 'ignored': 0,
                                             'names': sorted(producer.REQUIRED_TESTS),
                                             'required_names': sorted(producer.REQUIRED_TESTS), 'features': []},
                     'python-tooling': {'passed': 1, 'failed': 0, 'ignored': 0,
                                        'names': ['P055Tests.test_p055_fixture'],
                                        'required_names': ['test_p055_fixture'], 'features': []}},
            'commands': [
                {'name': 'compact-semantics', 'exit_code': 0,
                 'stdout': archived_log, 'stderr': archived_stderr},
                {'name': 'python-tooling', 'exit_code': 0,
                 'stdout': archived_python_log, 'stderr': archived_stderr},
            ],
            'artifacts': {'command:compact-semantics:stdout': archived_log,
                          'command:compact-semantics:stderr': archived_stderr,
                          'command:python-tooling:stdout': archived_python_log,
                          'command:python-tooling:stderr': archived_stderr,
                          'gzip-probe': elf_probe,
                          **output_artifacts},
            'original_outputs': {
                'default_build': ref(default_build), 'compact_build': ref(compact_build),
                'production_record': ref(build_record_path),
            },
        })
        declaration = {'task': 'P055', 'selection': 'frozen P055 rows'}
        report_path = write_json(self.root, f'{label}/comparison/report.json', {
            'schema': 'wtime-focused-comparison-v1', 'lane': 'candidate',
            'passed': True, 'source_stable': True, 'selection': 'P055',
            'declaration': declaration,
            'identities': {**common, 'candidate': ref(binary), 'semantic_receipt': ref(semantic_receipt)},
            'metadata': {
                'settings': {'candidate_mode': 'compact'},
                'candidate_binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
                'candidate_sources': {producer.SOURCE: source_sha},
            },
        })
        comparison_source = put(self.root, f'{label}/comparison/sources/{producer.SOURCE}', source_text)
        self.assertEqual(hashlib.sha256(comparison_source.read_bytes()).hexdigest(), source_sha)
        contract = common['selection_manifest']
        shared_inputs = {}
        for name in ('task_plan', 'gate', 'analyzer', 'producer', 'helper_execution_identity'):
            path = put(self.root, f'common/{name}.json', f'{name}:shared\n'.encode())
            shared_inputs[name] = ref(path)
        stats_inputs = {
            'report': ref(report_path), 'build': ref(default_build),
            'optional_build': ref(compact_build), 'candidate_build': ref(build_record_path),
            'contract': contract, **shared_inputs,
        }
        stats_path = write_json(self.root, f'{label}/statistics.json', {
            'schema': 'wtime-verification-statistics-v1', 'task': 'P055',
            'status': 'passed', 'statistics_computations': 1, 'timing_evidence': True,
            'source_binding': {'scope': 'frozen candidate declaration'},
            'inputs': stats_inputs, 'decision': {'decision': verdict},
        })
        return stats_path, semantic_receipt

    def combine(self, split_verdict, handlers_verdict, **kwargs):
        split, split_receipt = self.make_variant('split', split_verdict, **kwargs)
        handlers, handlers_receipt = self.make_variant('handlers', handlers_verdict, **kwargs)
        return producer.combine(split, split_receipt, handlers, handlers_receipt)

    def test_split_preferred_when_both_variants_are_accepted_and_stats_are_not_recomputed(self):
        result = self.combine('adopted', 'adopted')
        self.assertEqual(result['decision']['decision'], 'adopted')
        self.assertEqual(result['selected_variant'], 'split')
        self.assertEqual(result['statistics_computations'], 2)
        self.assertEqual(result['aggregate_statistics_computations'], 0)
        self.assertEqual(result['additional_statistics_computations'], 0)
        self.assertEqual([item['candidate_source']['selected_shape'] for item in result['variants']],
                         ['Split', 'Handlers'])

    def test_handlers_selected_when_split_is_not_accepted_but_handlers_is(self):
        result = self.combine('rejected', 'adopted')
        self.assertEqual((result['decision']['decision'], result['selected_variant']),
                         ('adopted', 'handlers'))

    def test_inconclusive_propagates_when_neither_variant_is_accepted(self):
        result = self.combine('inconclusive', 'rejected')
        self.assertEqual((result['decision']['decision'], result['selected_variant']),
                         ('inconclusive', None))
        self.assertEqual(self.combine('rejected', 'rejected')['decision']['decision'], 'rejected')

    def test_wrong_copied_shape_and_skipped_semantic_case_are_rejected(self):
        split, split_receipt = self.make_variant('split', 'adopted', actual_shape='Handlers')
        handlers, handlers_receipt = self.make_variant('handlers', 'adopted')
        with self.assertRaisesRegex(ValueError, 'does not select Shape::Split'):
            producer.combine(split, split_receipt, handlers, handlers_receipt)

        split, split_receipt = self.make_variant('split', 'adopted', label='skipped')
        receipt = json.loads(split_receipt.read_text())
        receipt['tests']['compact-semantics']['names'].pop()
        split_receipt.write_text(json.dumps(receipt))
        stats = json.loads(split.read_text())
        report_path = Path(stats['inputs']['report']['path'])
        report = json.loads(report_path.read_text())
        report['identities']['semantic_receipt'] = ref(split_receipt)
        report_path.write_text(json.dumps(report))
        stats['inputs']['report'] = ref(report_path)
        split.write_text(json.dumps(stats))
        # The receipt argument is hash-bound as a retained source-current phase output.
        handlers, handlers_receipt = self.make_variant('handlers', 'adopted', label='skipped-handlers')
        with self.assertRaisesRegex(ValueError, 'omits a required P055 case'):
            producer.combine(split, split_receipt, handlers, handlers_receipt)

        split, split_receipt = self.make_variant('split', 'adopted', label='bad-build-gate')
        handlers, handlers_receipt = self.make_variant('handlers', 'adopted', label='bad-build-handlers')
        stats = json.loads(split.read_text())
        build_path = Path(stats['inputs']['build']['path'])
        build_path.write_text(json.dumps({'budget_pass': False, 'source_stable': True}))
        stats['inputs']['build'] = ref(build_path)
        receipt = json.loads(split_receipt.read_text())
        receipt['original_outputs']['default_build'] = ref(build_path)
        receipt['artifacts']['default_build'] = artifact(self.root, build_path.read_bytes())
        split_receipt.write_text(json.dumps(receipt))
        report_path = Path(stats['inputs']['report']['path'])
        report = json.loads(report_path.read_text())
        report['identities']['semantic_receipt'] = ref(split_receipt)
        report_path.write_text(json.dumps(report))
        stats['inputs']['report'] = ref(report_path)
        split.write_text(json.dumps(stats))
        with self.assertRaisesRegex(ValueError, 'default_build build gate did not pass'):
            producer.combine(split, split_receipt, handlers, handlers_receipt)

    def test_changed_consumed_input_same_report_or_different_baseline_rejected(self):
        split, split_receipt = self.make_variant('split', 'adopted')
        handlers, handlers_receipt = self.make_variant('handlers', 'adopted')
        stats = json.loads(split.read_text())
        consumed = Path(stats['inputs']['build']['path'])
        consumed.write_bytes(b'changed consumed build input')
        with self.assertRaisesRegex(ValueError, 'statistics input build input changed'):
            producer.combine(split, split_receipt, handlers, handlers_receipt)

        split, split_receipt = self.make_variant('split', 'adopted', label='baseline-split')
        handlers, handlers_receipt = self.make_variant('handlers', 'adopted', label='baseline-handlers',
                                                       baseline_label='different')
        with self.assertRaisesRegex(ValueError, 'different frozen baseline_manifest'):
            producer.combine(split, split_receipt, handlers, handlers_receipt)

    def test_same_comparison_and_candidate_build_verdict_shape_are_rejected(self):
        split, split_receipt = self.make_variant('split', 'adopted')
        handlers, handlers_receipt = self.make_variant('handlers', 'adopted')
        handler_record = json.loads(handlers.read_text())
        split_record = json.loads(split.read_text())
        handler_record['inputs']['report'] = split_record['inputs']['report']
        handlers.write_text(json.dumps(handler_record))
        with self.assertRaises(ValueError):
            producer.combine(split, split_receipt, handlers, handlers_receipt)

        split, split_receipt = self.make_variant('split', 'adopted', label='same-binary-split')
        handlers, handlers_receipt = self.make_variant('handlers', 'adopted', label='same-binary-handlers')
        stats = json.loads(handlers.read_text())
        report_path = Path(stats['inputs']['report']['path'])
        report = json.loads(report_path.read_text())
        split_stats = json.loads(split.read_text())
        split_report_path = Path(split_stats['inputs']['report']['path'])
        split_report = json.loads(split_report_path.read_text())
        report['identities']['candidate'] = split_report['identities']['candidate']
        report['metadata']['candidate_binary_sha256'] = split_report['metadata']['candidate_binary_sha256']
        report_path.write_text(json.dumps(report))
        stats['inputs']['report'] = ref(report_path)
        handlers.write_text(json.dumps(stats))
        with self.assertRaisesRegex(ValueError, 'comparison and production binary identities differ'):
            producer.combine(split, split_receipt, handlers, handlers_receipt)

        split, split_receipt = self.make_variant('split', 'accepted', label='bad-verdict')
        handlers, handlers_receipt = self.make_variant('handlers', 'adopted', label='bad-verdict-handlers')
        with self.assertRaisesRegex(ValueError, 'verdict is unknown'):
            producer.combine(split, split_receipt, handlers, handlers_receipt)


if __name__ == '__main__':
    unittest.main()
