"""ArtifactStore contract tests; run only against an isolated temporary store."""
from __future__ import annotations
import hashlib
import importlib
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parents[3]
TOOLS = REPO / 'tools'
if str(TOOLS) not in sys.path:
    sys.path.insert(0, str(TOOLS))

from verification.artifacts import ArtifactStore, VerificationError, sha

class ArtifactStoreTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='artifact-store-contract-')
        self.base = Path(self.temp.name)
        self.inputs = self.base / 'inputs'
        self.inputs.mkdir()
        self.store_path = self.base / 'store'
        self.store = ArtifactStore(self.store_path)

    def tearDown(self):
        self.temp.cleanup()

    def file(self, name: str, payload: bytes, mode: int = 0o644) -> Path:
        path = self.inputs / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(payload)
        path.chmod(mode)
        return path

    def object_path(self, ref: dict) -> Path:
        rel = ref.get('path')
        self.assertIsInstance(rel, str)
        path = (self.store_path / rel).resolve(strict=True)
        self.assertTrue(path.is_relative_to(self.store_path.resolve()))
        return path

    def test_duplicate_physical_fixture_has_two_logical_bindings_one_object(self):
        wat = self.file('fixture.wat', b'(module (func))')
        bindings = {
            'fixture.primary': {'path': str(wat.resolve()), 'sha256': sha(wat), 'bytes': wat.stat().st_size},
            'fixture.protected': {'path': str(wat.resolve()), 'sha256': sha(wat), 'bytes': wat.stat().st_size},
        }
        plan = self.store.plan(bindings)
        refs = plan
        self.assertIsInstance(refs, dict)
        self.assertEqual(set(refs), set(bindings))
        self.assertEqual(refs['fixture.primary']['sha256'], refs['fixture.protected']['sha256'])
        self.assertEqual(refs['fixture.primary']['path'], refs['fixture.protected']['path'])
        self.assertEqual(self.store.read(refs['fixture.primary']), wat.read_bytes())
        self.assertEqual(self.store.read(refs['fixture.protected']), wat.read_bytes())

    def test_same_bytes_at_distinct_paths_share_object_and_receipt_identity(self):
        first = self.file('first.log', b'completed\n')
        second = self.file('nested/second.log', b'completed\n')
        a = self.store.add(first)
        b = self.store.add(second)
        self.assertNotEqual(first.resolve(), second.resolve())
        self.assertEqual(a['sha256'], b['sha256'])
        self.assertEqual(a['bytes'], b['bytes'])
        self.assertEqual(a['path'], b['path'])
        self.assertEqual(self.store.read(a), self.store.read(b))

    def test_conflicting_expected_hash_length_or_mode_is_rejected(self):
        fixture = self.file('fixture.wat', b'abc', 0o640)
        digest = sha(fixture)
        with self.assertRaises(VerificationError):
            self.store.add(fixture, expected={'sha256': '0' * 64})
        with self.assertRaises(VerificationError):
            self.store.plan({'fixture.a': {'path': str(fixture), 'sha256': digest, 'bytes': 4}})
        with self.assertRaises(VerificationError):
            self.store.plan({'../fixture': {'path': str(fixture)}})
        with self.assertRaises(VerificationError):
            self.store.plan({'fixture.a': {'path': str(fixture), 'mode': '0o600'}})

    def test_empty_log_is_valid_but_stale_expected_bytes_are_rejected(self):
        empty = self.file('empty.log', b'')
        ref = self.store.add(empty, expected={'sha256': hashlib.sha256(b'').hexdigest(),
                                              'bytes': 0, 'mode': oct(empty.stat().st_mode & 0o7777)})
        self.assertEqual(self.store.read(ref), b'')
        changing = self.file('changing.log', b'old')
        stale_digest = sha(changing)
        stale = {'sha256': stale_digest, 'bytes': len(b'old'),
                 'mode': oct(changing.stat().st_mode & 0o7777)}
        changing.write_bytes(b'new')
        with self.assertRaises(VerificationError):
            self.store.add(changing, expected=stale)

    def test_corrupt_raw_object_and_path_escape_are_rejected(self):
        source = self.file('raw.dat', b'raw artifact')
        ref = self.store.add(source)
        obj = self.object_path(ref)
        obj.chmod(0o644)
        obj.write_bytes(b'corrupt')
        with self.assertRaises(VerificationError):
            self.store.verify(ref)
        escaped = dict(ref, object_path='../outside')
        with self.assertRaises(VerificationError):
            self.store.verify(escaped)

    def test_corrupt_gzip_elf_and_materialize_mode_roundtrip(self):
        executable = self.file('native-elf', b'\x7fELF' + bytes(range(64)), 0o751)
        ref = self.store.add(executable, expected=sha(executable))
        self.assertEqual(ref.get('encoding'), 'gzip')
        self.assertEqual(ref.get('mode'), '0o751')
        materialized = self.base / 'materialized' / 'native-elf'
        self.store.materialize(ref, materialized)
        self.assertEqual(materialized.read_bytes(), executable.read_bytes())
        self.assertEqual(materialized.stat().st_mode & 0o777, 0o751)
        with self.assertRaises(VerificationError):
            self.store.materialize(ref, materialized)
        gzip_object = self.object_path(ref)
        gzip_object.chmod(0o644)
        gzip_object.write_bytes(gzip_object.read_bytes()[:5])
        with self.assertRaises(VerificationError):
            self.store.verify(ref)

    def test_elf_payloads_deduplicate_and_roundtrip_original_modes(self):
        payload = b'\x7fELF' + b'program payload'
        first = self.file('first-elf', payload, 0o751)
        second = self.file('second-elf', payload, 0o711)
        a = self.store.add(first)
        b = self.store.add(second)
        self.assertEqual(a['encoding'], 'gzip')
        self.assertEqual(a['path'], b['path'])
        self.assertEqual(self.store.read(a), payload)
        self.assertEqual(self.store.read(b), payload)
        self.assertEqual(a['mode'], '0o751')
        self.assertEqual(b['mode'], '0o711')
        out_a = self.base / 'out-a'
        out_b = self.base / 'out-b'
        self.store.materialize(a, out_a)
        self.store.materialize(b, out_b)
        self.assertEqual(out_a.read_bytes(), payload)
        self.assertEqual(out_b.read_bytes(), payload)
        self.assertEqual(out_a.stat().st_mode & 0o777, 0o751)
        self.assertEqual(out_b.stat().st_mode & 0o777, 0o711)

    def test_plan_checks_all_bindings_before_storing_any_payload(self):
        good = self.file('good.wat', b'(module)')
        bad = self.file('bad.wat', b'(module bad)')
        bindings = {
            'valid.fixture': {'path': str(good.resolve()), 'sha256': sha(good)},
            'stale.fixture': {'path': str(bad.resolve()), 'sha256': 'f' * 64},
        }
        with self.assertRaises(VerificationError):
            self.store.plan(bindings)
        objects = self.store_path / 'objects'
        payload_files = [p for p in objects.rglob('*') if p.is_file()] if objects.exists() else []
        self.assertEqual(payload_files, [], 'plan stored an earlier payload before validating all bindings')

if __name__ == '__main__':
    unittest.main()
