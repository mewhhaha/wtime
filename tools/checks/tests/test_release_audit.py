import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

SCRIPT = Path(__file__).parents[1] / 'release_audit.py'
spec = importlib.util.spec_from_file_location('release_audit', SCRIPT)
audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)


class ReleaseAuditTests(unittest.TestCase):
    def allocation_report(self, root):
        worker = root / 'target/debug/wtime-allocation-check'
        worker.parent.mkdir(parents=True)
        worker.write_bytes(b'worker binary fixture')
        (root / 'Cargo.lock').write_text('lock')
        (root / 'Cargo.toml').write_text('workspace')
        return {
            'passed': True,
            'source_stable': True,
            'source_sha256': audit.core_source_hash(root),
            'architecture': 'x86_64',
            'command': [str(worker.resolve())],
            'worker_sha256': audit.sha256(worker),
            'address_space_bytes': 128 * 1024 * 1024,
            'requested_allocation_bytes': 512 * 1024 * 1024,
            'executed': 1,
            'exit_code': 0,
            'stdout': json.dumps({
                'allocator_failed': True,
                'guest_table_allocation_failed': True,
                'frame_allocation_failed': True,
                'recovery_passed': True,
                'executed': 1,
            }),
            'stderr': '',
        }

    def test_t169_case_01_missing_evidence_blocks_release(self):
        errors = audit.audit({'schema_version': 1, 'evidence': {}})
        self.assertTrue(any('missing required evidence block: core3' in error for error in errors))
        self.assertTrue(any('missing required evidence block: allocation_failure' in error for error in errors))

    def test_t169_case_02_current_core_test_source_hash_is_required(self):
        current = audit.core_source_hash()
        manifest = {'schema_version': 1, 'source_sha256': current}
        errors = audit.audit(manifest)
        self.assertNotIn('manifest source_sha256 is missing or stale', errors)
        manifest['source_sha256'] = '0' * 64
        errors = audit.audit(manifest)
        self.assertIn('manifest source_sha256 is missing or stale', errors)

    def test_t169_case_03_unsupported_behavior_cannot_be_claimed_complete(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / 'Cargo.toml').write_text('[workspace]\n')
            (root / 'Cargo.lock').write_text('')
            lock_dir = root / 'docs/architecture'
            lock_dir.mkdir(parents=True)
            (lock_dir / 'spec-lock.json').write_text(json.dumps({
                'core_spec_commit': 'abc', 'excluded_sources': [{'feature_id': 'js-api'}],
                'sources': [],
            }))
            (lock_dir / 'opcodes.json').write_text(json.dumps({'instructions': [], 'non_opcode_features': []}))
            (lock_dir / 'corpus-lock.json').write_text(json.dumps({'revision': 'def', 'files': []}))
            tasks = root / 'tasks'
            tasks.mkdir()
            (tasks / 'TASKS.json').write_text(json.dumps({'tasks': [{'id': 'T169', 'dependencies': []}]}))
            manifest = {'schema_version': 1, 'source_sha256': audit.core_source_hash(root),
                        'spec_revision': 'abc', 'corpus_revision': 'def',
                        'supported_claims': ['js-api'], 'unsupported_behavior': ['js-api'],
                        'evidence': {}}
            errors = audit.audit(manifest, root=root, tasks_root=tasks)
            self.assertIn('unsupported behavior claimed complete: js-api', errors)

    def test_t169_case_04_report_hash_mismatch_blocks_release(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            path = root / 'report.json'
            path.write_text('{"passed": true}')
            errors = []
            audit._report(root, {'path': 'report.json', 'sha256': '0' * 64}, 'core3', errors)
            self.assertEqual(errors, ['evidence core3: report hash mismatch'])

    def test_t169_case_05_real_allocation_failure_schema_and_worker_binding(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            report = self.allocation_report(root)
            errors = []
            audit._check_allocation_failure(report, errors, root)
            self.assertEqual(errors, [])
            report['worker_sha256'] = '0' * 64
            audit._check_allocation_failure(report, errors := [], root)
            self.assertIn('evidence allocation_failure: worker binary fingerprint is missing or stale', errors)

    def test_t169_case_06_allocation_failure_requires_recovery_under_pressure(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            report = self.allocation_report(root)
            report['stdout'] = json.dumps({'allocator_failed': True, 'recovery_passed': False, 'executed': 1})
            errors = []
            audit._check_allocation_failure(report, errors, root)
            self.assertIn('evidence allocation_failure: worker did not report allocator failure and successful recovery', errors)

    def test_t169_case_07_stale_allocation_source_blocks_release(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            report = self.allocation_report(root)
            report['source_sha256'] = '0' * 64
            errors = []
            audit._check_allocation_failure(report, errors, root)
            self.assertIn('evidence allocation_failure: core source hash is missing or stale', errors)

    def conformance_report(self, root):
        binary = root / 'target/release/wtime-conformance'
        binary.parent.mkdir(parents=True)
        binary.write_bytes(b'conformance executable')
        source = root / 'engine.rs'
        source.write_text('// implementation fixture\n')
        return {
            'selected': 1, 'executed': 1, 'failed': 0, 'skipped': 0,
            'unsupported': 0, 'text_tool_errors': 0,
            'profile': 'stable-core', 'suite': 'core3',
            'source_hash': 'corpus-lock:def;fixture:abc',
            'engine_revision': 'dirty-fixture',
            'binary_sha256': audit.sha256(binary),
            'implementation_sources': {'engine.rs': audit.sha256(source)},
            'core_spec_commit': 'abc',
        }

    def test_t169_case_08_conformance_binary_must_match_current_executable(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            report = self.conformance_report(root)
            errors = []
            audit._check_conformance(report, 'core3', errors, root, 'abc', 'def')
            self.assertEqual(errors, [])
            (root / 'target/release/wtime-conformance').write_bytes(b'replaced executable')
            errors = []
            audit._check_conformance(report, 'core3', errors, root, 'abc', 'def')
            self.assertIn('evidence core3: executed binary fingerprint is stale', errors)

    def test_t169_case_09_conformance_binary_must_be_present(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            report = self.conformance_report(root)
            (root / 'target/release/wtime-conformance').unlink()
            errors = []
            audit._check_conformance(report, 'core3', errors, root, 'abc', 'def')
            self.assertIn('evidence core3: current conformance executable is missing', errors)

    def test_t169_case_10_native_evidence_follows_supported_targets(self):
        x64_only = audit.required_evidence_keys({
            'toolchain': {'supported_targets': ['x86_64-unknown-linux-gnu']}
        })
        self.assertIn('native_x86_64', x64_only)
        self.assertNotIn('native_aarch64', x64_only)
        both = audit.required_evidence_keys({
            'toolchain': {'supported_targets': ['x86_64-unknown-linux-gnu', 'aarch64-unknown-linux-gnu']}
        })
        self.assertIn('native_x86_64', both)
        self.assertIn('native_aarch64', both)
        omitted = audit.required_evidence_keys({})
        self.assertIn('native_x86_64', omitted)
        self.assertIn('native_aarch64', omitted)

    def test_t169_case_11_portable_mode_validates_digests_without_local_elfs(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            alloc_report = self.allocation_report(root)
            conf_report = self.conformance_report(root)
            (root / 'target/debug/wtime-allocation-check').unlink()
            (root / 'target/release/wtime-conformance').unlink()

            errors = []
            audit._check_allocation_failure(
                alloc_report, errors, root, verify_local_binaries=False
            )
            self.assertEqual(errors, [])

            conf_errors = []
            audit._check_conformance(
                conf_report, 'core3', conf_errors, root, 'abc', 'def',
                verify_local_binaries=False,
            )
            self.assertEqual(conf_errors, [])

            bad_alloc = {**alloc_report, 'worker_sha256': 'not-a-sha'}
            bad_errors = []
            audit._check_allocation_failure(
                bad_alloc, bad_errors, root, verify_local_binaries=False
            )
            self.assertIn(
                'evidence allocation_failure: worker binary fingerprint is missing or malformed',
                bad_errors,
            )

    def test_t169_case_12_evidence_target_dir_isolated_and_dirty_rev_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / 'Cargo.lock').write_text('lock')
            (root / 'Cargo.toml').write_text('workspace')
            ev_worker = root / 'target/evidence/debug/wtime-allocation-check'
            ev_worker.parent.mkdir(parents=True)
            ev_worker.write_bytes(b'evidence worker')
            # Simulate cargo test --all-features writing a different binary to target/debug
            clobbered = root / 'target/debug/wtime-allocation-check'
            clobbered.parent.mkdir(parents=True)
            clobbered.write_bytes(b'all-features worker')
            report = {
                'passed': True,
                'source_stable': True,
                'source_sha256': audit.core_source_hash(root),
                'architecture': 'x86_64',
                'command': ['target/evidence/debug/wtime-allocation-check'],
                'worker_sha256': audit.sha256(ev_worker),
                'address_space_bytes': 128 * 1024 * 1024,
                'requested_allocation_bytes': 512 * 1024 * 1024,
                'executed': 1,
                'exit_code': 0,
                'stdout': json.dumps({
                    'allocator_failed': True,
                    'guest_table_allocation_failed': True,
                    'frame_allocation_failed': True,
                    'recovery_passed': True,
                    'executed': 1,
                }),
                'stderr': '',
            }
            errors = []
            audit._check_allocation_failure(report, errors, root, verify_local_binaries=True)
            self.assertEqual(errors, [])

            # Missing frame_allocation_failed must fail closed
            no_frame = {**report, 'stdout': json.dumps({
                'allocator_failed': True,
                'guest_table_allocation_failed': True,
                'frame_allocation_failed': False,
                'recovery_passed': True,
                'executed': 1,
            })}
            errors = []
            audit._check_allocation_failure(no_frame, errors, root, verify_local_binaries=True)
            self.assertIn('evidence allocation_failure: worker did not report allocator failure and successful recovery', errors)

            # Dirty engine_revision must fail closed
            conf = self.conformance_report(root)
            conf['engine_revision'] = '1c18cc8-dirty'
            conf_errors = []
            audit._check_conformance(conf, 'core3', conf_errors, root, 'abc', 'def')
            self.assertIn('evidence core3: engine revision is missing or dirty', conf_errors)

    def test_t169_case_13_missing_interaction_corpus_blocks_release(self):
        manifest = {
            'schema_version': 1,
            'evidence': {
                'interactions': {'path': 'docs/verification/interactions.json', 'sha256': '0' * 64},
            },
        }
        errors = audit.audit(manifest)
        self.assertIn('evidence interaction_corpus: missing corpus_report or corpus_sha256', errors)


if __name__ == '__main__':
    unittest.main()
