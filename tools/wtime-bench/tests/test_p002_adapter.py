import hashlib
import importlib.util
import json
import os
from pathlib import Path
import stat
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "baseline.py"
SPEC = importlib.util.spec_from_file_location("p002_baseline", SCRIPT)
baseline_tool = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(baseline_tool)


def record(path: Path, contents: bytes, role: str):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(contents)
    return {"sha256": hashlib.sha256(contents).hexdigest(), "size_bytes": len(contents),
            "mode_bits": stat.S_IMODE(path.stat().st_mode), "role": role}


class LegacyAdapterArchiveTests(unittest.TestCase):
    def setup_archives(self, root: Path):
        frozen = root / "frozen"
        source_rel = "source/crates/wtime-runtime/src/lib.rs"
        baseline_files = {
            source_rel: record(frozen / source_rel, b"frozen runtime", "source"),
            "binary/wtime-bench": record(frozen / "binary/wtime-bench", b"frozen harness", "release_benchmark_executable"),
        }
        frozen_manifest = {"schema": 1, "files": baseline_files}
        (frozen / "manifest.json").write_text(json.dumps(frozen_manifest))
        protocol = root / "harness/src/protocol.rs"
        protocol.parent.mkdir(parents=True)
        protocol.write_bytes(b"shared protocol")
        (root / "harness/run.py").write_bytes(b"shared runner")

        archive = root / "adapter"
        binary = archive / "binary/wtime-bench"
        adapter_binary = record(binary, b"adapter executable", "legacy_adapter_executable")
        binary.chmod(0o755)
        adapter_binary["mode_bits"] = stat.S_IMODE(binary.stat().st_mode)
        adapter_files = {
            "src/protocol.rs": record(archive / "src/protocol.rs", protocol.read_bytes(), "shared_measurement_protocol"),
            "binary/wtime-bench": adapter_binary,
        }
        adapter_manifest = {
            "schema": 1,
            "kind": "legacy-api-adapter",
            "baseline": str(frozen.resolve()),
            "binary": "binary/wtime-bench",
            "baseline_manifest_sha256": baseline_tool.sha(frozen / "manifest.json"),
            "protocol_sha256": baseline_tool.sha(protocol),
            "shared_harness_identities": {"run.py": baseline_tool.sha(root / "harness/run.py")},
            "frozen_source_identities": {source_rel: baseline_files[source_rel]["sha256"]},
            "binary_sha256": adapter_files["binary/wtime-bench"]["sha256"],
            "files": adapter_files,
        }
        (archive / "manifest.json").write_text(json.dumps(adapter_manifest))
        return frozen, protocol, archive

    def test_p002_matching_frozen_adapter_is_reused_and_verified(self):
        with tempfile.TemporaryDirectory() as directory:
            frozen, protocol, archive = self.setup_archives(Path(directory))
            manifest = baseline_tool.verify_adapter(archive, frozen, protocol)
            self.assertEqual(manifest["kind"], "legacy-api-adapter")
            self.assertEqual(manifest["binary_sha256"], baseline_tool.sha(archive / "binary/wtime-bench"))

    def test_p002_changed_shared_protocol_rejects_existing_adapter(self):
        with tempfile.TemporaryDirectory() as directory:
            frozen, protocol, archive = self.setup_archives(Path(directory))
            protocol.write_bytes(b"changed protocol")
            with self.assertRaisesRegex(ValueError, "protocol identity"):
                baseline_tool.verify_adapter(archive, frozen, protocol)

    def test_p002_changed_shared_runner_rejects_existing_adapter(self):
        with tempfile.TemporaryDirectory() as directory:
            frozen, protocol, archive = self.setup_archives(Path(directory))
            (Path(directory) / "harness/run.py").write_bytes(b"changed runner")
            with self.assertRaisesRegex(ValueError, "shared harness identity"):
                baseline_tool.verify_adapter(archive, frozen, protocol)

    def test_p002_adapter_binary_tampering_fails_manifest_hash(self):
        with tempfile.TemporaryDirectory() as directory:
            frozen, protocol, archive = self.setup_archives(Path(directory))
            with (archive / "binary/wtime-bench").open("ab") as stream:
                stream.write(b"changed")
            with self.assertRaisesRegex(ValueError, "contents differ|hash mismatch"):
                baseline_tool.verify_adapter(archive, frozen, protocol)

    def test_p002_adapter_binary_permission_tampering_fails_verification(self):
        with tempfile.TemporaryDirectory() as directory:
            frozen, protocol, archive = self.setup_archives(Path(directory))
            (archive / "binary/wtime-bench").chmod(0o644)
            with self.assertRaisesRegex(ValueError, "mode mismatch|not executable"):
                baseline_tool.verify_adapter(archive, frozen, protocol)

    def test_p002_changed_frozen_archive_is_rejected_without_rebuild(self):
        with tempfile.TemporaryDirectory() as directory:
            frozen, protocol, archive = self.setup_archives(Path(directory))
            with (frozen / "source/crates/wtime-runtime/src/lib.rs").open("ab") as stream:
                stream.write(b"changed")
            with self.assertRaisesRegex(ValueError, "contents differ|hash mismatch"):
                baseline_tool.verify_adapter(archive, frozen, protocol)


if __name__ == "__main__":
    unittest.main()
