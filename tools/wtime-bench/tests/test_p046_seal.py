"""Isolated tests for the P046 phase seal and its artifact inventory."""
from __future__ import annotations

import gzip
import hashlib
import io
import json
from pathlib import Path
import sys
import tarfile
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[3]
PRODUCERS = ROOT / "tools/verification/producers"
sys.dont_write_bytecode = True
sys.path.insert(0, str(PRODUCERS))
import p046_verify as p046  # noqa: E402


class P046SealTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="p046-seal-tests-")
        self.parent = Path(self.temporary.name).resolve()
        self.addCleanup(self.temporary.cleanup)
        self.snapshot = {"synthetic": {"sha256": "a" * 64}}

    def root(self, label: str) -> Path:
        path = (self.parent / label).resolve()
        path.mkdir()
        self.assertTrue(path.is_relative_to(self.parent))
        return path

    def write(self, root: Path, relative: str, content: bytes) -> Path:
        path = (root / relative).resolve()
        self.assertTrue(path.is_relative_to(root), "synthetic output escaped phase root")
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(content)
        return path

    def seal(self, root: Path):
        with mock.patch.object(p046, "_seal_source_snapshot", return_value=self.snapshot):
            return p046.seal(root)

    def test_seal_inventory_bundle_hashes_contents_and_is_deterministic(self):
        first, second = self.root("first"), self.root("second")
        for root, stdout, stderr in ((first, b"first stdout", b"first stderr"),
                                     (second, b"other stdout", b"other stderr")):
            self.write(root, "report.json", b'{"status":"passed"}\n')
            self.write(root, "nested/result.bin", b"phase artifact\x00")
            self.write(root, "seal-evidence.stdout", stdout)
            self.write(root, "seal-evidence.stderr", stderr)

        first_inventory = self.seal(first)
        second_inventory = self.seal(second)

        for root, inventory in ((first, first_inventory), (second, second_inventory)):
            inventory_path = (root / "inventory.json").resolve()
            bundle_path = (root / "artifacts.tar.gz").resolve()
            self.assertTrue(inventory_path.is_relative_to(root))
            self.assertTrue(bundle_path.is_relative_to(root))
            self.assertEqual(json.loads(inventory_path.read_text(encoding="utf-8")), inventory)
            bundle = bundle_path.read_bytes()
            self.assertEqual(inventory["bundle"]["sha256"], hashlib.sha256(bundle).hexdigest())
            self.assertEqual(inventory["bundle"]["bytes"], len(bundle))
            self.assertEqual(inventory["file_count"], len(inventory["files"]))
            self.assertIn("seal.helper.executed.py", inventory["files"])
            self.assertEqual(inventory["files"]["report.json"]["sha256"],
                             hashlib.sha256(b'{"status":"passed"}\n').hexdigest())
            self.assertNotIn("inventory.json", inventory["files"])
            self.assertNotIn("artifacts.tar.gz", inventory["files"])
            with tarfile.open(fileobj=io.BytesIO(gzip.decompress(bundle)), mode="r:") as archive:
                members = {member.name: archive.extractfile(member).read()
                           for member in archive.getmembers()}
            self.assertEqual(set(members), set(inventory["files"]))
            self.assertEqual(members["nested/result.bin"], b"phase artifact\x00")
        self.assertEqual(first_inventory["bundle"]["sha256"], second_inventory["bundle"]["sha256"])
        self.assertEqual((first / "artifacts.tar.gz").read_bytes(),
                         (second / "artifacts.tar.gz").read_bytes())

    def test_external_symlink_rejects_without_publishing_inventory(self):
        phase_root, second_root = self.root("phase"), self.root("second-temp-root")
        outside = self.write(second_root, "outside.bin", b"still within second temp root")
        link = (phase_root / "claimed-artifact.bin").resolve()
        self.assertTrue(link.is_relative_to(phase_root))
        link.symlink_to(outside)

        with mock.patch.object(p046, "_seal_source_snapshot", return_value=self.snapshot):
            with self.assertRaisesRegex(ValueError, "symlink outside"):
                p046.seal(phase_root)

        self.assertFalse((phase_root / "inventory.json").exists())
        self.assertFalse((phase_root / "artifacts.tar.gz").exists())
        self.assertEqual(outside.read_bytes(), b"still within second temp root")

    def test_runner_streams_are_excluded_while_artifacts_and_helper_are_included(self):
        root = self.root("streams")
        artifact = b"real phase output"
        self.write(root, "receipt/report.json", artifact)
        self.write(root, "seal-evidence.stdout", b"stdout owned by shared runner")
        self.write(root, "seal-evidence.stderr", b"stderr owned by shared runner")

        inventory = self.seal(root)

        self.assertEqual(inventory["excluded_runner_streams"],
                         ["seal-evidence.stdout", "seal-evidence.stderr"])
        self.assertIn("receipt/report.json", inventory["files"])
        self.assertIn("seal.helper.executed.py", inventory["files"])
        self.assertNotIn("seal-evidence.stdout", inventory["files"])
        self.assertNotIn("seal-evidence.stderr", inventory["files"])
        self.assertTrue((root / "seal-evidence.stdout").is_file())
        bundle = gzip.decompress((root / "artifacts.tar.gz").read_bytes())
        with tarfile.open(fileobj=io.BytesIO(bundle), mode="r:") as archive:
            names = set(archive.getnames())
        self.assertIn("receipt/report.json", names)
        self.assertIn("seal.helper.executed.py", names)
        self.assertNotIn("seal-evidence.stdout", names)
        self.assertNotIn("seal-evidence.stderr", names)


if __name__ == "__main__":
    unittest.main()
