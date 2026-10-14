import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "baseline.py"
SPEC = importlib.util.spec_from_file_location("p001_baseline", SCRIPT)
baseline = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(baseline)


class BaselineArchiveTests(unittest.TestCase):
    def archive(self, root: Path) -> Path:
        archive = root / "archive"
        files = {
            "source/crates/wtime-runtime/src/lib.rs": b"source snapshot",
            "fixtures/integer_loop.wasm": b"fixture bytes",
            "binary/wtime-bench": b"executable bytes",
        }
        records = {}
        for name, contents in files.items():
            path = archive / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(contents)
            records[name] = {"sha256": hashlib.sha256(contents).hexdigest(),
                             "size_bytes": len(contents), "role": "test"}
        archive.mkdir(exist_ok=True)
        (archive / "manifest.json").write_text(json.dumps({"schema": 1, "files": records}))
        return archive

    def test_p001_verified_archive_is_reused_unchanged(self):
        with tempfile.TemporaryDirectory() as directory:
            archive = self.archive(Path(directory))
            before = (archive / "manifest.json").read_bytes()
            manifest = baseline.verify_archive(archive)
            self.assertEqual(len(manifest["files"]), 3)
            self.assertEqual((archive / "manifest.json").read_bytes(), before)

    def test_p001_modified_source_fixture_or_binary_fails_hash_verification(self):
        for rel in ("source/crates/wtime-runtime/src/lib.rs", "fixtures/integer_loop.wasm",
                    "binary/wtime-bench"):
            with self.subTest(rel=rel), tempfile.TemporaryDirectory() as directory:
                archive = self.archive(Path(directory))
                with (archive / rel).open("ab") as stream:
                    stream.write(b"x")
                with self.assertRaisesRegex(ValueError, "hash mismatch"):
                    baseline.verify_archive(archive)

    def test_p001_missing_archived_input_fails_verification(self):
        with tempfile.TemporaryDirectory() as directory:
            archive = self.archive(Path(directory))
            (archive / "fixtures/integer_loop.wasm").unlink()
            with self.assertRaisesRegex(ValueError, "contents differ"):
                baseline.verify_archive(archive)


if __name__ == "__main__":
    unittest.main()
