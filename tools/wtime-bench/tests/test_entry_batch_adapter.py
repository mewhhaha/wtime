"""Synthetic source-bound adapter checks; never seeds repository provenance paths."""
from __future__ import annotations

import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import stat
import tempfile
import unittest
from unittest import mock

SOURCE = Path(__file__).resolve().parents[1] / "entry_batch_adapter.py"
spec = importlib.util.spec_from_file_location("entry_batch_adapter", SOURCE)
adapter = importlib.util.module_from_spec(spec)
assert spec.loader
spec.loader.exec_module(adapter)


def _hash(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _inputs(root: Path) -> tuple[Path, Path]:
    archive = root / "archive"
    harness = root / "harness"
    files = {
        "source/Cargo.toml": b"[workspace]\nresolver='3'\n",
        "source/crates/wtime-runtime/Cargo.toml": b"[package]\nname='wtime-runtime'\n[features]\nregister-executor=[]\n",
        "source/crates/wtime-runtime/src/lib.rs": b"pub fn with_execution_policy() {} pub fn with_register_entry_threshold() {}\n",
        "source/crates/wtime-types/Cargo.toml": b"[package]\nname='wtime-types'\n",
        "source/crates/wtime-types/src/lib.rs": b"pub struct FeatureSet;\n",
        "source/crates/wtime-decode/Cargo.toml": b"[package]\nname='wtime-decode'\n",
        "source/crates/wtime-decode/src/lib.rs": b"\n",
        "source/crates/wtime-validate/Cargo.toml": b"[package]\nname='wtime-validate'\n",
        "source/crates/wtime-validate/src/lib.rs": b"\n",
    }
    records = {}
    for rel, data in files.items():
        path = archive / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        records[rel] = {"role": "source", "sha256": _hash(data), "size_bytes": len(data)}
    manifest = {"schema": 1, "mode": "register",
                "execution_policy": {"resolved_mode": "register", "protocol_modes": ["register"]},
                "files": records}
    (archive / "manifest.json").write_text(json.dumps(manifest))
    for name, data in {
        "protocol.rs": b"pub fn try_main(_: &[String]) -> bool { false }\n",
        "entry_batch_driver.rs": b"// synthetic driver\n",
        "counter_driver.rs": b"// synthetic leaf\n",
    }.items():
        path = harness / "src" / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
    return archive, harness


def _archive_state(archive: Path) -> dict[str, tuple[bytes, int]]:
    return {p.relative_to(archive).as_posix(): (p.read_bytes(), p.stat().st_mtime_ns)
            for p in archive.rglob("*") if p.is_file()}


def _current_inputs(root: Path) -> tuple[Path, Path]:
    repo = root / "current-repo"
    harness = root / "current-harness"
    (repo / "Cargo.toml").parent.mkdir(parents=True, exist_ok=True)
    (repo / "Cargo.toml").write_text('[workspace]\nmembers = ["crates/*", "tools/wtime-conformance"]\n')
    (repo / "Cargo.lock").write_text("version = 4\n")
    crates = {
        "wtime-runtime": (b"[package]\nname='wtime-runtime'\n[features]\nregister-executor=[]\n",
                          b"pub fn with_execution_policy() {} pub fn with_register_entry_threshold() {}\n"),
        "wtime-types": (b"[package]\nname='wtime-types'\n", b"pub struct FeatureSet;\n"),
        "wtime-decode": (b"[package]\nname='wtime-decode'\n", b"\n"),
        "wtime-validate": (b"[package]\nname='wtime-validate'\n", b"\n"),
        "wtime-cli": (b"[package]\nname='wtime-cli'\n", b"fn main() {}\n"),
    }
    for name, (manifest, source) in crates.items():
        base = repo / "crates" / name
        (base / "src").mkdir(parents=True)
        (base / "Cargo.toml").write_bytes(manifest)
        (base / "src/lib.rs").write_bytes(source)
    conformance = repo / "tools/wtime-conformance"
    (conformance / "src").mkdir(parents=True)
    (conformance / "Cargo.toml").write_text("[package]\nname='wtime-conformance'\n")
    (conformance / "src/main.rs").write_text("fn main() {}\n")
    harness.mkdir(parents=True)
    for name, data in {
        "protocol.rs": b"pub fn try_main(_: &[String]) -> bool { false }\n",
        "entry_batch_driver.rs": b"// current driver\n",
        "counter_driver.rs": b"// current leaf\n",
    }.items():
        path = harness / "src" / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
    return repo, harness


class AdapterTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory(prefix="p136-entry-adapter-")
        self.root = Path(self.tmp.name).resolve()
        self.archive, self.harness = _inputs(self.root)
        self.output = self.root / "output"
        self.real_run = adapter.subprocess.run

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def _cargo(self, argv, cwd=None, text=True, capture_output=True, check=False, **kwargs):
        argv = list(argv)
        if argv[0] == "git":
            return self.real_run(argv, **kwargs)
        if argv[1] == "metadata":
            return adapter.subprocess.CompletedProcess(argv, 0, json.dumps({
                "target_directory": str(Path(argv[-1]).parent / "target")}), "")
        if argv[1] == "generate-lockfile":
            lock = Path(argv[-1]).parent / "Cargo.lock"
            lock.write_text("version = 4\n\n[[package]]\nname = \"wtime-register-entry-adapter\"\nversion = \"0.1.0\"\n")
            return adapter.subprocess.CompletedProcess(argv, 0, "", "")
        if argv[1] == "build":
            workspace = Path(argv[argv.index("--manifest-path") + 1]).parent
            executable = workspace / "target/release/wtime-register-entry-adapter"
            executable.parent.mkdir(parents=True)
            executable.write_bytes(b"synthetic-adapter-executable")
            executable.chmod(executable.stat().st_mode | stat.S_IXUSR)
            message = {"reason": "compiler-artifact", "target": {"name": "wtime-register-entry-adapter"},
                       "executable": str(executable)}
            return adapter.subprocess.CompletedProcess(argv, 0, json.dumps(message) + "\n", "")
        if argv[1:] == ["-Vv"] or argv[1:] == ["-V"]:
            return adapter.subprocess.CompletedProcess(argv, 0, "synthetic version\n", "")
        raise AssertionError(f"unexpected subprocess command: {argv}")

    def test_build_verify_is_source_bound_and_preserves_archive(self) -> None:
        before = _archive_state(self.archive)
        with mock.patch.object(adapter.subprocess, "run", side_effect=self._cargo):
            result = adapter.build(self.archive, self.harness, self.output)
        self.assertEqual(result["schema"], "wtime.source-bound-register-entry-adapter.v1")
        self.assertEqual(result["kind"], "source-bound-register-entry-adapter")
        self.assertEqual(result["features"], ["register-executor", "runtime-only"])
        self.assertFalse(result["instrumented"])
        self.assertEqual(len(result["copied_core_sources"]), 9)
        self.assertIn("src/counter_driver.rs", result["protocol_leaf_sources"])
        self.assertEqual(before, _archive_state(self.archive))
        self.assertEqual(result, adapter.verify(self.output, self.archive, self.harness))
        for path in self.output.rglob("*"):
            self.assertTrue(path.resolve().is_relative_to(self.output))
        commands = json.loads((self.output / result["commands"]).read_text())
        build = next(command for command in commands["commands"] if command["label"] == "cargo-build")
        self.assertNotIn("CARGO_TARGET_DIR", " ".join(build["argv"]))

    def test_verify_rejects_stale_overlay_dirty_copy_and_extra_file(self) -> None:
        with mock.patch.object(adapter.subprocess, "run", side_effect=self._cargo):
            result = adapter.build(self.archive, self.harness, self.output)
        leaf = self.harness / "src/counter_driver.rs"
        old = leaf.read_bytes()
        leaf.write_bytes(old + b"// stale\n")
        with self.assertRaisesRegex(ValueError, "overlay"):
            adapter.verify(self.output, self.archive, self.harness)
        leaf.write_bytes(old)
        copied = self.output / "source/crates/wtime-runtime/src/lib.rs"
        copied.write_bytes(copied.read_bytes() + b"// dirty\n")
        with self.assertRaisesRegex(ValueError, "dirty"):
            adapter.verify(self.output, self.archive, self.harness)
        # Restore mapped bytes so the next rejection specifically tests extras.
        record = result["copied_core_sources"]["crates/wtime-runtime/src/lib.rs"]
        copied.write_bytes((self.archive / record["archive_path"]).read_bytes())
        (self.output / "unbound.bin").write_bytes(b"x")
        with self.assertRaisesRegex(ValueError, "extra"):
            adapter.verify(self.output, self.archive, self.harness)

    def test_foreign_archive_mode_and_missing_constructor_reject_before_output(self) -> None:
        manifest_path = self.archive / "manifest.json"
        manifest = json.loads(manifest_path.read_text())
        manifest["mode"] = "decoded"
        manifest_path.write_text(json.dumps(manifest))
        with self.assertRaisesRegex(ValueError, "not Register"):
            adapter.build(self.archive, self.harness, self.output)
        self.assertFalse(self.output.exists())
        archive2, harness2 = _inputs(self.root / "second")
        runtime = archive2 / "source/crates/wtime-runtime/src/lib.rs"
        runtime.write_text("pub fn with_execution_policy() {}\n")
        manifest = json.loads((archive2 / "manifest.json").read_text())
        rel = "source/crates/wtime-runtime/src/lib.rs"
        data = runtime.read_bytes()
        manifest["files"][rel].update(sha256=_hash(data), size_bytes=len(data))
        (archive2 / "manifest.json").write_text(json.dumps(manifest))
        with self.assertRaisesRegex(ValueError, "constructors"):
            adapter.build(archive2, harness2, self.root / "other-output")

    def test_archive_symlink_members_and_harness_symlink_leaves_reject(self) -> None:
        outside = self.root / "outside.rs"
        outside.write_text("// outside\n")
        member = self.archive / "source/escape.rs"
        member.symlink_to(outside)
        with self.assertRaisesRegex(ValueError, "symlink member"):
            adapter._load_archive(self.archive)
        member.unlink()
        leaf = self.harness / "src/escape.rs"
        leaf.symlink_to(outside)
        with self.assertRaisesRegex(ValueError, "symlink"):
            adapter._harness_inputs(self.harness)

    def test_generated_entrypoint_manifest_and_command_log_are_bound(self) -> None:
        with mock.patch.object(adapter.subprocess, "run", side_effect=self._cargo):
            result = adapter.build(self.archive, self.harness, self.output)
        main = self.output / "workspace/src/main.rs"
        main.write_text(main.read_text() + "// altered\n")
        with self.assertRaisesRegex(ValueError, "entrypoint"):
            adapter.verify(self.output, self.archive, self.harness)
        main.write_text(adapter.ENTRYPOINT)
        cargo_toml = self.output / "workspace/Cargo.toml"
        cargo_toml.write_text(cargo_toml.read_text() + "\n# altered\n")
        with self.assertRaisesRegex(ValueError, "Cargo.toml"):
            adapter.verify(self.output, self.archive, self.harness)
        cargo_toml.write_text(adapter._workspace_manifest())
        command_path = self.output / result["commands"]
        log = json.loads(command_path.read_text())
        original_log = json.loads(json.dumps(log))
        log["commands"][0]["returncode"] = True
        command_path.write_text(json.dumps(log))
        with self.assertRaisesRegex(ValueError, "did not succeed"):
            adapter.verify(self.output, self.archive, self.harness)
        log = json.loads(json.dumps(original_log))
        rustc = next(entry for entry in log["commands"] if entry["label"] == "rustc-version")
        rustc["argv"][-1] = "-v"
        command_path.write_text(json.dumps(log))
        with self.assertRaisesRegex(ValueError, "toolchain version commands"):
            adapter.verify(self.output, self.archive, self.harness)
        log = json.loads(json.dumps(original_log))
        rustc = next(entry for entry in log["commands"] if entry["label"] == "rustc-version")
        rustc["stdout"] = "different compiler\n"
        command_path.write_text(json.dumps(log))
        with self.assertRaisesRegex(ValueError, "toolchain output"):
            adapter.verify(self.output, self.archive, self.harness)
        log = json.loads(json.dumps(original_log))
        log["commands"].append(dict(log["commands"][0]))
        command_path.write_text(json.dumps(log))
        with self.assertRaisesRegex(ValueError, "duplicate"):
            adapter.verify(self.output, self.archive, self.harness)

    def test_failed_build_keeps_executed_helper_and_command_evidence(self) -> None:
        def failing(argv, cwd=None, text=True, capture_output=True, check=False, **kwargs):
            if argv[1] == "metadata":
                return self._cargo(argv, cwd, text, capture_output, check, **kwargs)
            return adapter.subprocess.CompletedProcess(argv, 9, "failed stdout", "failed stderr")

        with mock.patch.object(adapter.subprocess, "run", side_effect=failing):
            with self.assertRaisesRegex(RuntimeError, r"failed \(9\)"):
                adapter.build(self.archive, self.harness, self.output)
        staging = list(self.root.glob(".output.staging-*"))
        self.assertEqual(len(staging), 1)
        helper = staging[0] / "commands/entry_batch_adapter.py"
        log = staging[0] / "commands/commands.json"
        self.assertTrue(helper.is_file())
        self.assertTrue(log.is_file())
        self.assertEqual(json.loads(log.read_text())["commands"][-1]["returncode"], 9)

    def test_current_verification_survives_unrelated_commit_but_rejects_source_change(self) -> None:
        repo, harness = _current_inputs(self.root)
        output = repo / "target/candidate-output"
        with mock.patch.object(adapter.subprocess, "run", side_effect=self._cargo):
            result = adapter.build_current(repo, harness, output)
        initial_head = result["repository_head"]
        self.assertIsNone(initial_head)
        git = ["git", "-C", str(repo)]
        subprocess.run(git + ["init", "-q"], check=True)
        subprocess.run(git + ["config", "user.email", "adapter-test@example.invalid"], check=True)
        subprocess.run(git + ["config", "user.name", "Adapter Test"], check=True)
        subprocess.run(git + ["add", "."], check=True)
        subprocess.run(git + ["commit", "-qm", "initial"], check=True)
        (repo / "README.md").write_text("unrelated repository note\n")
        subprocess.run(git + ["add", "README.md"], check=True)
        subprocess.run(git + ["commit", "-qm", "unrelated"], check=True)
        self.assertNotEqual(adapter._git_head(repo), initial_head)
        self.assertEqual(adapter.verify_current(output, repo, harness), result)
        source = repo / "crates/wtime-runtime/src/lib.rs"
        source.write_bytes(source.read_bytes() + b"// changed\n")
        with self.assertRaisesRegex(ValueError, "source membership or hashes are stale"):
            adapter.verify_current(output, repo, harness)

    def test_current_candidate_binds_live_workspace_and_rejects_stale_sources(self) -> None:
        repo, harness = _current_inputs(self.root)
        output = repo / "target/candidate-output"
        before = {rel: (path.read_bytes(), path.stat().st_mtime_ns)
                  for rel, path in adapter._current_source_paths(repo).items()}
        with mock.patch.object(adapter.subprocess, "run", side_effect=self._cargo):
            result = adapter.build_current(repo, harness, output)
        self.assertEqual(result["schema"], "wtime.source-bound-register-entry-candidate.v1")
        self.assertEqual(result["kind"], "source-bound-register-entry-candidate")
        self.assertEqual(result["source_origin"], "current-workspace")
        self.assertNotIn("archive", result)
        self.assertIn("crates/wtime-cli/src/lib.rs", result["core_source_map"])
        self.assertIn("tools/wtime-conformance/src/main.rs", result["core_source_map"])
        self.assertEqual(result["protocol_sha256"], result["overlay_sources"]["src/protocol.rs"]["sha256"])
        self.assertEqual(result["shared_leaf_map"]["src/counter_driver.rs"],
                         result["overlay_sources"]["src/counter_driver.rs"]["sha256"])
        after = {rel: (path.read_bytes(), path.stat().st_mtime_ns)
                 for rel, path in adapter._current_source_paths(repo).items()}
        self.assertEqual(before, after)
        self.assertEqual(result, adapter.verify_current(output, repo, harness))
        source = repo / "crates/wtime-runtime/src/lib.rs"
        source.write_bytes(source.read_bytes() + b"// stale\n")
        with self.assertRaisesRegex(ValueError, "source membership or hashes are stale"):
            adapter.verify_current(output, repo, harness)


if __name__ == "__main__":
    unittest.main(verbosity=2)
