"""Mocked tests for source-bound current-core runtime-only production builds."""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import tempfile
import types
import unittest
from unittest import mock

from tools.verification.producers import runtime_production as producer


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


class SyntheticBuild:
    def __init__(self, root: Path, *, protocol=None, metadata_fault=None, drift=False,
                 workspace_drift=False):
        self.root = root.resolve()
        self.protocol = protocol or {
            "schema_version": 2, "engine": "wtime",
            "modes": ["decoded", "compact"], "instrumented": False,
        }
        self.metadata_fault = metadata_fault
        self.drift = drift
        self.workspace_drift = workspace_drift
        self.output = None
        self.calls = []
        self.tool_dir = self.root / "fake-tools"
        self.tool_dir.mkdir(parents=True, exist_ok=True)
        for name in ("cargo", "rustc"):
            path = self.tool_dir / name
            path.write_bytes((name + " synthetic executable").encode())

    def which(self, name, path=None):
        return str(self.tool_dir / name)

    def source_map(self):
        paths = [self.root / "Cargo.toml", self.root / "Cargo.lock",
                 self.root / "rust-toolchain.toml"]
        for crate in producer.CORE_CRATES:
            manifest = self.root / "crates" / crate / "Cargo.toml"
            paths.append(manifest)
            paths.extend((manifest.parent / "src").rglob("*.rs"))
        paths.extend((self.root / "tools/wtime-bench/src").rglob("*.rs"))
        paths.extend([self.root / "tools/wtime-bench/Cargo.toml",
                      self.root / "tools/wtime-bench/Cargo.lock"])
        return {path.relative_to(self.root).as_posix(): digest(path.read_bytes())
                for path in paths if path.is_file()}

    def package_rows(self, wrapper: Path):
        rows = []
        names = list(producer.CORE_CRATES) + ["wtime-production-protocol"]
        for name in names:
            manifest = (wrapper / "Cargo.toml" if name == "wtime-production-protocol"
                        else self.root / "crates" / name / "Cargo.toml")
            source = None
            if self.metadata_fault == "external" and name == "wtime-production-protocol":
                source = "registry+https://example.invalid/index"
            if self.metadata_fault == "wrong-core" and name == "wtime-runtime":
                manifest = self.root / "wrong-core/Cargo.toml"
                manifest.parent.mkdir(parents=True, exist_ok=True)
                manifest.write_text("[package]\nname='wrong'\nversion='0.1.0'\n")
            rows.append({"name": name, "id": "local:" + name,
                         "source": source, "manifest_path": str(manifest)})
        return rows

    def metadata(self, wrapper: Path, features=()):
        rows = self.package_rows(wrapper)
        if wrapper == self.root:
            rows = [row for row in rows if row["name"] in producer.CORE_CRATES]
        runtime_features = ["compact-executor"] if "compact-executor" in features else []
        if self.metadata_fault == "runtime-features":
            runtime_features = ["execution-reference"]
        node_features = {
            "wtime-runtime": runtime_features,
            "wtime-production-protocol": list(features),
        }
        return {"workspace_root": str(wrapper), "packages": rows,
                "resolve": {"nodes": [{"id": row["id"], "deps": [],
                                         "features": node_features.get(row["name"], [])}
                                        for row in rows]}}

    def run(self, argv, *, cwd, env=None, text=True, capture_output=True, check=False):
        self.calls.append(list(argv))
        if self.output is not None:
            # Source copies and the helper are retained before any subprocess launch.
            self.assert_archived_before_exec()
        if argv[0] in ("cargo", "rustc") and argv[1:] == ["-Vv"]:
            return types.SimpleNamespace(returncode=0, stdout=f"{argv[0]} synthetic 1.0\n", stderr="")
        if argv[0] == "cargo" and "generate-lockfile" in argv:
            manifest = Path(argv[argv.index("--manifest-path") + 1])
            (manifest.parent / "Cargo.lock").write_text("version = 4\n", encoding="utf-8")
            return types.SimpleNamespace(returncode=0, stdout="", stderr="")
        if argv[0] == "cargo" and "metadata" in argv:
            manifest = Path(argv[argv.index("--manifest-path") + 1])
            workspace = manifest.parent
            features = argv[argv.index("--features") + 1].split(",") if "--features" in argv else []
            if "metadata" in argv and "--no-default-features" not in argv and workspace != self.root:
                raise AssertionError("wrapper metadata omitted --no-default-features")
            if workspace != self.root:
                manifest_text = (workspace / "Cargo.toml").read_text()
                if manifest_text.count("default-features = false") != 2:
                    raise AssertionError("wrapper dependencies must disable default features")
            return types.SimpleNamespace(returncode=0,
                stdout=json.dumps(self.metadata(workspace, features)), stderr="")
        if argv[0] == "cargo" and "build" in argv:
            target = Path(env["CARGO_TARGET_DIR"])
            binary = target / "release/wtime-production-protocol"
            binary.parent.mkdir(parents=True, exist_ok=True)
            binary.write_bytes(b"synthetic executable bytes")
            if self.drift:
                source = self.root / "crates/wtime-runtime/src/lib.rs"
                source.write_text("// changed while fake build ran\n", encoding="utf-8")
            if self.workspace_drift:
                driver = Path(env["CARGO_TARGET_DIR"]).parent / "src/protocol.rs"
                driver.write_text("// workspace changed while fake build ran\n", encoding="utf-8")
            return types.SimpleNamespace(returncode=0, stdout="build ok\n", stderr="")
        if argv[0].endswith("/bin/wtime-bench") and argv[1:] == ["--protocol"]:
            return types.SimpleNamespace(returncode=0, stdout=json.dumps(self.protocol), stderr="")
        raise AssertionError(f"unexpected subprocess command: {argv}")

    def assert_archived_before_exec(self):
        archived = self.output / "source/Cargo.toml"
        if not archived.is_file():
            raise AssertionError("source archive must precede command execution")
        if not (self.output / "helper.executed.py").is_file():
            raise AssertionError("helper source must precede command execution")


class RuntimeProductionTests(unittest.TestCase):
    def make_root(self, root: Path) -> SyntheticBuild:
        root.mkdir(parents=True, exist_ok=True)
        (root / "Cargo.toml").write_text("[workspace]\n", encoding="utf-8")
        (root / "Cargo.lock").write_text("version = 4\n", encoding="utf-8")
        (root / "rust-toolchain.toml").write_text("[toolchain]\nchannel = 'test'\n", encoding="utf-8")
        for crate in producer.CORE_CRATES:
            crate_dir = root / "crates" / crate
            (crate_dir / "src").mkdir(parents=True)
            (crate_dir / "Cargo.toml").write_text(
                f"[package]\nname='{crate}'\nversion='0.1.0'\n", encoding="utf-8")
            (crate_dir / "src/lib.rs").write_text(f"// {crate}\n", encoding="utf-8")
        bench = root / "tools/wtime-bench"
        (bench / "src").mkdir(parents=True)
        (bench / "Cargo.toml").write_text("[package]\nname='placeholder'\n", encoding="utf-8")
        (bench / "Cargo.lock").write_text("version = 4\n", encoding="utf-8")
        for name in ("main.rs", "protocol.rs", "scalar_driver.rs", "gc_roots_driver.rs"):
            (bench / "src" / name).write_text(f"// {name}\n", encoding="utf-8")
        return SyntheticBuild(root)

    def invoke(self, root: Path, mode: str, *, runner=None, synth=None):
        synth = synth or self.make_root(root)
        output = root / "target/verification/runs/task/verify/attempt-0001/production"
        synth.output = output
        with mock.patch.object(producer, "_source_identity", side_effect=lambda _: synth.source_map()), \
             mock.patch.object(producer.shutil, "which", side_effect=synth.which):
            record = producer.build(output, mode, root=root, runner=runner or synth.run)
        return record, output, synth

    def assert_tree_contained(self, root: Path):
        for path in root.rglob("*"):
            self.assertTrue(path.resolve().is_relative_to(root.resolve()), str(path))

    def test_decoded_and_compact_have_separate_exact_feature_roles(self):
        for mode, runtime_features, cargo_features in (
            ("decoded", [], ["runtime-only"]),
            ("compact", ["compact-executor"], ["runtime-only", "compact-executor"]),
        ):
            with self.subTest(mode=mode), tempfile.TemporaryDirectory(prefix="runtime-prod-") as temp:
                root = Path(temp).resolve() / "repo"
                synth = self.make_root(root)
                record, output, synth = self.invoke(root, mode, synth=synth)
                self.assertEqual(record["schema"], "wtime-runtime-only-production-v1")
                self.assertEqual(record["mode"], mode)
                self.assertEqual(record["harness_features"], ["runtime-only"])
                self.assertEqual(record["runtime_features"], runtime_features)
                self.assertEqual(record["feature_flags"], runtime_features)
                self.assertEqual(record["cargo_features"], cargo_features)
                self.assertEqual(record["instrumented"], False)
                command = record["command"]
                self.assertEqual(command[:7], ["cargo", "build", "--locked", "--offline",
                                               "--release", "--no-default-features", "-j4"])
                self.assertEqual(command[command.index("--features") + 1], ",".join(cargo_features))
                self.assertIn("--manifest-path", command)
                self.assertEqual(record["core_resolution"], {
                    crate: str((root / "crates" / crate / "Cargo.toml").resolve())
                    for crate in producer.CORE_CRATES})
                self.assertEqual(record["metadata"]["summary"]["packages"],
                                 sorted(set(producer.CORE_CRATES) | {"wtime-production-protocol"}))
                self.assertEqual(record["metadata"]["summary"]["external_dependencies"], 0)
                self.assertEqual(record["protocol"]["instrumented"], False)
                self.assertEqual(record["protocol"]["modes"], ["decoded", "compact"])
                metadata_call = next(call for call in synth.calls if "metadata" in call
                                     and str(root / "Cargo.toml") not in call)
                self.assertIn("--no-default-features", metadata_call)
                self.assertEqual(metadata_call[metadata_call.index("--features") + 1],
                                 ",".join(cargo_features))
                self.assertTrue(record["source_identities"])
                self.assertTrue(record["compiled_inputs"])
                self.assertEqual(record["binary_sha256"], digest((output / "bin/wtime-bench").read_bytes()))
                self.assertEqual(record["helper_sha256"], digest((output / "helper.executed.py").read_bytes()))
                self.assertEqual(record["protocol_sha256"], digest((output / "protocol.stdout.json").read_bytes()))
                for log in record["logs"].values():
                    self.assertEqual(log["sha256"], digest(Path(log["path"]).read_bytes()))
                self.assertEqual(record["cache_manifest"]["sha256"],
                                 digest(Path(record["cache_manifest"]["path"]).read_bytes()))
                self.assertTrue(record["source_identities"].keys() <= record["archived_sources"].keys())
                for item in record["compiled_inputs"].values():
                    archive_path = Path(item["archive_path"])
                    self.assertTrue(archive_path.is_relative_to(output))
                    self.assertEqual(digest(archive_path.read_bytes()), item["sha256"])
                self.assertEqual(record["workspace_manifest"]["path"],
                                 str(output / "compiled/Cargo.toml"))
                generated_manifest = Path(record["workspace_manifest"]["path"]).read_text()
                self.assertIn('wtime-runtime = { path = ', generated_manifest)
                self.assertIn(', default-features = false }', generated_manifest)
                self.assertEqual(generated_manifest.count('default-features = false'), 2)
                self.assertTrue(record["stdout_log"]["path"].endswith("/stdout.log"))
                self.assertIn("generated:src/main.rs", record["compiled_inputs"])
                self.assertNotIn("tools/wtime-bench/src/main.rs", record["compiled_inputs"])
                self.assertTrue(any("--protocol" in call for call in synth.calls))
                self.assertFalse(any("--single-case" in call for call in synth.calls))
                self.assert_tree_contained(root)

    def test_stable_workspace_preserves_unchanged_mtime_and_reuses_target(self):
        with tempfile.TemporaryDirectory(prefix="runtime-cache-") as temp:
            root = Path(temp).resolve() / "repo"
            synth = self.make_root(root)
            first, first_output, _ = self.invoke(root, "decoded", synth=synth)
            workspace = root / "target/verification/build-caches/runtime-protocol/decoded/workspace"
            stable_driver = workspace / "src/protocol.rs"
            changed_driver = workspace / "src/scalar_driver.rs"
            stable_mtime = stable_driver.stat().st_mtime_ns
            changed_mtime = changed_driver.stat().st_mtime_ns
            target = workspace / "target"
            (root / "tools/wtime-bench/src/scalar_driver.rs").write_text("// updated driver\n")
            second_output = root / "target/verification/runs/task/verify/attempt-0002/production"
            synth.output = second_output
            with mock.patch.object(producer, "_source_identity", side_effect=lambda _: synth.source_map()), \
                 mock.patch.object(producer.shutil, "which", side_effect=synth.which):
                second = producer.build(second_output, "decoded", root=root, runner=synth.run)
            self.assertEqual(stable_driver.stat().st_mtime_ns, stable_mtime)
            self.assertGreater(changed_driver.stat().st_mtime_ns, changed_mtime)
            first_cache = json.loads(Path(first["cache_manifest"]["path"]).read_text())
            second_cache = json.loads(Path(second["cache_manifest"]["path"]).read_text())
            self.assertEqual(first_cache["workspace"], second_cache["workspace"])
            self.assertEqual(first_cache["target"], second_cache["target"])
            self.assertEqual(target, workspace / "target")
            self.assertNotEqual(first["input_key"], second["input_key"])
            third_output = root / "target/verification/runs/task/verify/attempt-0003/production"
            synth.output = third_output
            with mock.patch.object(producer, "_source_identity", side_effect=lambda _: synth.source_map()), \
                 mock.patch.object(producer.shutil, "which", side_effect=synth.which):
                third = producer.build(third_output, "decoded", root=root, runner=synth.run)
            self.assertEqual(second["input_key"], third["input_key"])
            self.assertTrue(first_output.is_dir() and second_output.is_dir())
            self.assert_tree_contained(root)

    def test_bad_core_resolution_and_external_package_fail_before_build(self):
        for fault in ("wrong-core", "external"):
            with self.subTest(fault=fault), tempfile.TemporaryDirectory(prefix="runtime-metadata-") as temp:
                root = Path(temp).resolve() / "repo"
                synth = self.make_root(root)
                synth.metadata_fault = fault
                output = root / "target/verification/runs/task/production"
                synth.output = output
                with mock.patch.object(producer, "_source_identity", side_effect=lambda _: synth.source_map()), \
                     mock.patch.object(producer.shutil, "which", side_effect=synth.which):
                    with self.assertRaises(ValueError):
                        producer.build(output, "decoded", root=root, runner=synth.run)
                self.assertFalse(any("build" in call for call in synth.calls))
                self.assert_tree_contained(root)

    def test_wrong_resolved_runtime_feature_is_rejected_before_build(self):
        with tempfile.TemporaryDirectory(prefix="runtime-features-") as temp:
            root = Path(temp).resolve() / "repo"
            synth = self.make_root(root)
            synth.metadata_fault = "runtime-features"
            output = root / "target/verification/runs/task/production"
            synth.output = output
            with mock.patch.object(producer, "_source_identity", side_effect=lambda _: synth.source_map()), \
                 mock.patch.object(producer.shutil, "which", side_effect=synth.which):
                with self.assertRaisesRegex(ValueError, "resolved runtime features"):
                    producer.build(output, "decoded", root=root, runner=synth.run)
            self.assertFalse(any("build" in call for call in synth.calls))
            self.assert_tree_contained(root)

    def test_protocol_must_match_mode_and_be_uninstrumented(self):
        protocols = [
            {"schema_version": 2, "engine": "wtime", "modes": ["decoded"], "instrumented": True},
            {"schema_version": 2, "engine": "wtime", "modes": ["compact"], "instrumented": False},
        ]
        for protocol in protocols:
            with self.subTest(protocol=protocol), tempfile.TemporaryDirectory(prefix="runtime-protocol-") as temp:
                root = Path(temp).resolve() / "repo"
                synth = self.make_root(root)
                synth.protocol = protocol
                output = root / "target/verification/runs/task/production"
                synth.output = output
                with mock.patch.object(producer, "_source_identity", side_effect=lambda _: synth.source_map()), \
                     mock.patch.object(producer.shutil, "which", side_effect=synth.which):
                    with self.assertRaisesRegex(ValueError, "does not prove uninstrumented"):
                        producer.build(output, "decoded", root=root, runner=synth.run)
                self.assert_tree_contained(root)

    def test_source_drift_during_build_fails_closed(self):
        with tempfile.TemporaryDirectory(prefix="runtime-drift-") as temp:
            root = Path(temp).resolve() / "repo"
            synth = self.make_root(root)
            synth.drift = True
            output = root / "target/verification/runs/task/production"
            synth.output = output
            with mock.patch.object(producer, "_source_identity", side_effect=lambda _: synth.source_map()), \
                 mock.patch.object(producer.shutil, "which", side_effect=synth.which):
                with self.assertRaisesRegex(ValueError, "sources changed"):
                    producer.build(output, "decoded", root=root, runner=synth.run)
            self.assert_tree_contained(root)

    def test_workspace_compiled_input_drift_fails_closed(self):
        with tempfile.TemporaryDirectory(prefix="runtime-workspace-drift-") as temp:
            root = Path(temp).resolve() / "repo"
            synth = self.make_root(root)
            synth.workspace_drift = True
            output = root / "target/verification/runs/task/production"
            synth.output = output
            with mock.patch.object(producer, "_source_identity", side_effect=lambda _: synth.source_map()), \
                 mock.patch.object(producer.shutil, "which", side_effect=synth.which):
                with self.assertRaisesRegex(ValueError, "compiled input .* changed at workspace_path"):
                    producer.build(output, "decoded", root=root, runner=synth.run)
            self.assert_tree_contained(root)

    def test_output_escape_is_rejected_before_any_write(self):
        with tempfile.TemporaryDirectory(prefix="runtime-output-escape-") as temp:
            base = Path(temp).resolve()
            root = base / "repo"
            self.make_root(root)
            outside = base / "outside"
            with mock.patch.object(producer, "_source_identity", side_effect=AssertionError("must not hash")):
                with self.assertRaisesRegex(ValueError, "escapes"):
                    producer.build(outside, "decoded", root=root, runner=lambda *a, **k: None)
            self.assertFalse(outside.exists())
            self.assertFalse((root / "target/verification/build-caches").exists())


if __name__ == "__main__":
    unittest.main()
