"""Synthetic contract tests for candidate freeze/build record producers."""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import tempfile
import types
import unittest
from unittest import mock

from tools.verification.producers import candidate_freeze, candidate_production


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


class CandidateFreezeTests(unittest.TestCase):
    def test_revalidates_exact_predecessor_and_copies_receipt(self):
        with tempfile.TemporaryDirectory(prefix="candidate-freeze-") as temporary:
            root = Path(temporary).resolve()
            manifest = root / "selection.json"
            predecessor = root / "before" / "manifest.json"
            frozen = root / "freeze.json"
            output = root / "phase" / "selection.json"
            for path, data in ((manifest, b"{}\n"), (predecessor, b"predecessor bytes")):
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(data)
            receipt = {
                "schema": "wtime-candidate-selection-freeze-v1",
                "manifest_path": str(manifest),
                "predecessor_path": str(predecessor),
                "predecessor_sha256": digest(predecessor.read_bytes()),
                "runtime_sources": {"crates/wtime-runtime/src/lib.rs": "a" * 64},
            }
            frozen.write_text(json.dumps(receipt), encoding="utf-8")
            output.parent.mkdir()
            self.assertTrue(output.resolve().is_relative_to(root))
            with mock.patch.object(candidate_freeze.selection_manifest, "verify_frozen") as verify:
                result = candidate_freeze.revalidate(manifest, predecessor, frozen, output)
            verify.assert_called_once_with(manifest.resolve(), frozen.resolve())
            self.assertEqual(result, receipt)
            self.assertEqual(output.read_bytes(), frozen.read_bytes())
            self.assertEqual(json.loads(output.read_text())["runtime_sources"], receipt["runtime_sources"])
            with mock.patch.object(candidate_freeze.selection_manifest, "verify_frozen"):
                with self.assertRaises(FileExistsError):
                    candidate_freeze.revalidate(manifest, predecessor, frozen, output)

    def test_wrong_predecessor_fails_before_output(self):
        with tempfile.TemporaryDirectory(prefix="candidate-freeze-mismatch-") as temporary:
            root = Path(temporary).resolve()
            manifest, frozen = root / "selection.json", root / "freeze.json"
            predecessor, other = root / "before.json", root / "other.json"
            manifest.write_text("{}", encoding="utf-8")
            predecessor.write_text("one", encoding="utf-8")
            other.write_text("two", encoding="utf-8")
            frozen.write_text(json.dumps({
                "predecessor_path": str(predecessor),
                "predecessor_sha256": digest(predecessor.read_bytes()),
            }), encoding="utf-8")
            output = root / "phase" / "selection.json"
            output.parent.mkdir()
            self.assertTrue(output.resolve().is_relative_to(root))
            with mock.patch.object(candidate_freeze.selection_manifest, "verify_frozen"):
                with self.assertRaisesRegex(ValueError, "different predecessor path"):
                    candidate_freeze.revalidate(manifest, other, frozen, output)
            self.assertFalse(output.exists())


class CandidateProductionTests(unittest.TestCase):
    def make_root(self, root: Path) -> tuple[dict[str, str], dict[str, str]]:
        for crate in candidate_production.CORE_CRATES:
            path = root / "crates" / crate / "src" / "lib.rs"
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(f"// {crate}\n", encoding="utf-8")
        bench_manifest = root / "tools/wtime-bench/Cargo.toml"
        bench_manifest.parent.mkdir(parents=True, exist_ok=True)
        bench_manifest.write_text(
            '[features]\ncompact-executor = ["wtime-runtime/compact-executor"]\n'
            'execution-reference = ["wtime-runtime/execution-reference"]\n', encoding="utf-8")
        target = root / "target" / "release" / "wtime-bench"
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(b"synthetic production binary")
        runtime_path = root / "crates/wtime-runtime/src/lib.rs"
        runtime = {"crates/wtime-runtime/src/lib.rs": digest(runtime_path.read_bytes())}
        return runtime, {**runtime, "synthetic": digest(b"source identity")}

    def test_compact_record_has_gate_schema_paths_and_hashes(self):
        with tempfile.TemporaryDirectory(prefix="candidate-production-") as temporary:
            root = Path(temporary).resolve()
            runtime_map, source_map = self.make_root(root)
            protocol = {"schema_version": 2, "engine": "wtime", "modes": ["decoded", "compact"],
                        "instrumented": False}
            calls = []

            def fake_run(argv, *, cwd, text, capture_output, check):
                calls.append((argv, cwd))
                if argv[0] == "cargo":
                    return types.SimpleNamespace(returncode=0, stdout="build ok\n", stderr="")
                self.assertEqual(argv[1:], ["--protocol"])
                return types.SimpleNamespace(returncode=0, stdout=json.dumps(protocol), stderr="")

            with mock.patch.object(candidate_production, "_source_identity", return_value=source_map), \
                 mock.patch.dict(os.environ, {"CARGO_TARGET_DIR": str(root / "target")}):
                record = candidate_production.build_production(
                    root / "phase" / "production", "compact", root=root, runner=fake_run)

            product = root / "phase" / "production"
            expected_files = ["build-record.json", "bin/wtime-bench", "helper.executed.py",
                              "stdout.log", "stderr.log", "protocol.stdout.json"]
            for name in expected_files:
                path = (product / name).resolve()
                self.assertTrue(path.is_relative_to(root))
                self.assertTrue(path.is_file())
            self.assertEqual(len(calls), 2)
            command = record["command"]
            self.assertEqual(command[:6], ["cargo", "build", "--locked", "--release",
                                           "--no-default-features", "-j"])
            self.assertEqual(command[6], "4")
            self.assertIn("compact-executor", command)
            self.assertEqual(record["schema"], "wtime-production-command-v1")
            self.assertEqual(record["runtime_source_map"], runtime_map)
            self.assertTrue(set(runtime_map).issubset(record["source_identities"]))
            self.assertEqual(record["binary_sha256"], digest((product / "bin/wtime-bench").read_bytes()))
            self.assertEqual(record["stdout_sha256"], digest((product / "stdout.log").read_bytes()))
            self.assertEqual(record["stderr_sha256"], digest((product / "stderr.log").read_bytes()))
            self.assertEqual(record["helper_sha256"], digest((product / "helper.executed.py").read_bytes()))
            self.assertEqual(record["protocol_sha256"], digest((product / "protocol.stdout.json").read_bytes()))
            self.assertEqual(record["protocol_command"], [record["binary_path"], "--protocol"])
            persisted = json.loads((product / "build-record.json").read_text())
            self.assertEqual(persisted, record)

            import sys
            bench = Path(candidate_production.__file__).resolve().parents[2] / "wtime-bench"
            if str(bench) not in sys.path:
                sys.path.insert(0, str(bench))
            import gates
            metadata = {"candidate_binary_sha256": record["binary_sha256"],
                        "candidate_sources": runtime_map,
                        "settings": {"candidate_mode": "compact", "instrumented": False},
                        "capabilities": {"candidate": protocol}}
            result = gates._validate_candidate_build_record(
                product / "build-record.json", metadata, {"metadata": metadata}, expected_root=root)
            self.assertEqual(result["binary_sha256"], record["binary_sha256"])

    def test_decoded_has_no_optional_features_and_strict_mode_attestation(self):
        with tempfile.TemporaryDirectory(prefix="candidate-production-decoded-") as temporary:
            root = Path(temporary).resolve()
            self.make_root(root)
            protocol = {"schema_version": 2, "engine": "wtime", "modes": ["decoded"],
                        "instrumented": False}

            def fake_run(argv, **kwargs):
                stdout = "{}" if argv[0] != "cargo" else "build ok"
                if argv[0] != "cargo":
                    stdout = json.dumps(protocol)
                return types.SimpleNamespace(returncode=0, stdout=stdout, stderr="")

            with mock.patch.object(candidate_production, "_source_identity", return_value={"x": "a" * 64}), \
                 mock.patch.dict(os.environ, {"CARGO_TARGET_DIR": str(root / "target")}):
                record = candidate_production.build_production(
                    root / "phase" / "production", "decoded", root=root, runner=fake_run)
            self.assertEqual(record["feature_flags"], [])
            self.assertNotIn("--features", record["command"])
            self.assertNotIn("execution-reference", record["command"])
            import gates
            metadata = {"candidate_binary_sha256": record["binary_sha256"],
                        "candidate_sources": record["runtime_source_map"],
                        "settings": {"candidate_mode": "decoded", "instrumented": False},
                        "capabilities": {"candidate": protocol}}
            path = root / "phase" / "production" / "build-record.json"
            result = gates._validate_candidate_build_record(path, metadata, {}, expected_root=root)
            self.assertEqual(result["runtime_features"], [])
            metadata["settings"]["candidate_mode"] = "compact"
            with self.assertRaisesRegex(gates.GateError, "feature configuration"):
                gates._validate_candidate_build_record(path, metadata, {}, expected_root=root)
            metadata["settings"]["candidate_mode"] = "decoded"
            record["schema"] = "p038-production-command-v1"
            path.write_text(json.dumps(record))
            with self.assertRaisesRegex(gates.GateError, "unsupported execution mode"):
                gates._validate_candidate_build_record(path, metadata, {}, expected_root=root)

    def test_rejects_instrumented_or_missing_mode_protocol(self):
        for protocol in (
            {"schema_version": 2, "engine": "wtime", "modes": ["compact"], "instrumented": True},
            {"schema_version": 2, "engine": "wtime", "modes": ["decoded"], "instrumented": False},
        ):
            with self.subTest(protocol=protocol), tempfile.TemporaryDirectory(prefix="candidate-protocol-") as temporary:
                root = Path(temporary).resolve()
                self.make_root(root)

                def fake_run(argv, **kwargs):
                    return types.SimpleNamespace(returncode=0,
                        stdout="{}" if argv[0] == "cargo" else json.dumps(protocol), stderr="")

                with mock.patch.object(candidate_production, "_source_identity", return_value={"x": "a" * 64}), \
                     mock.patch.dict(os.environ, {"CARGO_TARGET_DIR": str(root / "target")}):
                    with self.assertRaisesRegex(ValueError, "does not prove uninstrumented"):
                        candidate_production.build_production(
                            root / "phase" / "production", "compact", root=root, runner=fake_run)


if __name__ == "__main__":
    unittest.main()
