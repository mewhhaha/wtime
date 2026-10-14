"""Synthetic, subprocess-mocked tests for the runtime production attestation reader."""
from __future__ import annotations

import hashlib
import json
from pathlib import Path
import shutil
import tempfile
import types
import unittest
from unittest import mock

from tools.verification import production_attestation as attestation
from tools.verification.producers import runtime_production as producer


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def safe_write(boundary: Path, path: Path, data: bytes) -> None:
    boundary = boundary.resolve(strict=True)
    if not path.resolve(strict=False).is_relative_to(boundary):
        raise AssertionError(f"synthetic write escapes fresh temporary root: {path}")
    path.parent.mkdir(parents=True, exist_ok=True)
    if not path.parent.resolve(strict=True).is_relative_to(boundary):
        raise AssertionError(f"synthetic parent escapes fresh temporary root: {path.parent}")
    path.write_bytes(data)


class SyntheticBuild:
    """A small fully mocked producer environment rooted inside one temp directory."""

    def __init__(self, root: Path, boundary: Path):
        self.root = root.resolve()
        self.boundary = boundary.resolve(strict=True)
        if not self.root.is_relative_to(self.boundary):
            raise AssertionError("synthetic repository escaped its fresh temporary root")
        self.root.mkdir(parents=True)
        safe_write(self.boundary, self.root / "Cargo.toml", b"[workspace]\n")
        safe_write(self.boundary, self.root / "Cargo.lock", b"version = 4\n")
        safe_write(self.boundary, self.root / "rust-toolchain.toml", b"[toolchain]\nchannel = 'test'\n")
        for crate in producer.CORE_CRATES:
            crate_dir = self.root / "crates" / crate
            (crate_dir / "src").mkdir(parents=True)
            safe_write(self.boundary, crate_dir / "Cargo.toml",
                       f"[package]\nname='{crate}'\nversion='0.1.0'\n".encode())
            safe_write(self.boundary, crate_dir / "src/lib.rs", f"// {crate}\n".encode())
        bench = self.root / "tools/wtime-bench"
        (bench / "src").mkdir(parents=True)
        safe_write(self.boundary, bench / "Cargo.toml", b"[package]\nname='placeholder'\n")
        safe_write(self.boundary, bench / "Cargo.lock", b"version = 4\n")
        for name in ("main.rs", "protocol.rs", "scalar_driver.rs", "gc_roots_driver.rs"):
            safe_write(self.boundary, bench / "src" / name, f"// {name}\n".encode())
        self.tool_dir = self.root / "fake-tools"
        self.tool_dir.mkdir()
        for name in ("cargo", "rustc"):
            safe_write(self.boundary, self.tool_dir / name, (name + " synthetic executable").encode())
        self.output = None
        self.protocol = {"schema_version": 2, "engine": "wtime",
                         "modes": ["decoded", "compact", "register"], "instrumented": False}

    def which(self, name, path=None):
        return str(self.tool_dir / name)

    def source_map(self):
        paths = [self.root / "Cargo.toml", self.root / "Cargo.lock", self.root / "rust-toolchain.toml"]
        for crate in producer.CORE_CRATES:
            manifest = self.root / "crates" / crate / "Cargo.toml"
            paths.append(manifest)
            paths.extend((manifest.parent / "src").rglob("*.rs"))
        paths.extend((self.root / "tools/wtime-bench/src").rglob("*.rs"))
        paths.extend([self.root / "tools/wtime-bench/Cargo.toml", self.root / "tools/wtime-bench/Cargo.lock"])
        return {path.relative_to(self.root).as_posix(): digest(path.read_bytes())
                for path in paths if path.is_file()}

    def metadata(self, workspace: Path, features=()):
        rows = []
        names = list(producer.CORE_CRATES) + ["wtime-production-protocol"]
        for name in names:
            manifest = (workspace / "Cargo.toml" if name == "wtime-production-protocol"
                        else self.root / "crates" / name / "Cargo.toml")
            rows.append({"name": name, "id": "local:" + name, "source": None,
                         "manifest_path": str(manifest)})
        if workspace == self.root:
            rows = [row for row in rows if row["name"] in producer.CORE_CRATES]
        runtime_features = [feature for feature in ("compact-executor", "register-executor")
                            if feature in features]
        nodes = []
        for row in rows:
            name = row["name"]
            node_features = (runtime_features if name == "wtime-runtime" else
                             list(features) if name == "wtime-production-protocol" else [])
            nodes.append({"id": row["id"], "deps": [], "features": node_features})
        return {"workspace_root": str(workspace), "packages": rows,
                "resolve": {"nodes": nodes}}

    def run(self, argv, *, cwd, env=None, text=True, capture_output=True, check=False):
        if self.output is not None:
            if not (self.output / "source/Cargo.toml").is_file() or not (self.output / "helper.executed.py").is_file():
                raise AssertionError("archive and helper source must precede subprocess execution")
        if argv[0] in ("cargo", "rustc") and argv[1:] == ["-Vv"]:
            return types.SimpleNamespace(returncode=0, stdout=f"{argv[0]} synthetic 1.0\n", stderr="")
        if argv[0] == "cargo" and "generate-lockfile" in argv:
            manifest = Path(argv[argv.index("--manifest-path") + 1])
            safe_write(self.boundary, manifest.parent / "Cargo.lock", b"version = 4\n")
            return types.SimpleNamespace(returncode=0, stdout="", stderr="")
        if argv[0] == "cargo" and "metadata" in argv:
            manifest = Path(argv[argv.index("--manifest-path") + 1])
            features = argv[argv.index("--features") + 1].split(",") if "--features" in argv else []
            return types.SimpleNamespace(returncode=0,
                                         stdout=json.dumps(self.metadata(manifest.parent, features)), stderr="")
        if argv[0] == "cargo" and "build" in argv:
            binary = Path(env["CARGO_TARGET_DIR"]) / "release/wtime-production-protocol"
            binary.parent.mkdir(parents=True, exist_ok=True)
            safe_write(self.boundary, binary, b"synthetic executable bytes")
            return types.SimpleNamespace(returncode=0, stdout="build ok\n", stderr="")
        if argv[0].endswith("/bin/wtime-bench") and argv[1:] == ["--protocol"]:
            return types.SimpleNamespace(returncode=0, stdout=json.dumps(self.protocol), stderr="")
        raise AssertionError(f"unexpected mocked subprocess command: {argv}")


class ProductionAttestationTests(unittest.TestCase):
    def create_case(self, mode: str):
        Path("/tmp/runtime-attestation-tests").mkdir(parents=True, exist_ok=True)
        temporary = tempfile.TemporaryDirectory(prefix="case-", dir="/tmp/runtime-attestation-tests")
        outer = Path(temporary.name).resolve()
        root = outer / "repo"
        synth = SyntheticBuild(root, outer)
        output = root / "target/verification/runs/task/verify/attempt-0001/production"
        synth.output = output
        with mock.patch.object(producer, "_source_identity", side_effect=lambda _: synth.source_map()), \
                mock.patch.object(producer.shutil, "which", side_effect=synth.which):
            record = producer.build(output, mode, root=root, runner=synth.run)
        metadata_obj = {
            "settings": {"candidate_mode": mode, "instrumented": False},
            "candidate_sources": synth.source_map(),
            "candidate_binary_sha256": record["binary_sha256"],
            "capabilities": {"candidate": record["protocol"]},
        }
        report = {"passed": True, "source_stable": True}
        return temporary, root, output / "build-record.json", record, metadata_obj, report

    def assert_valid(self, case):
        _temporary, root, record_path, record, metadata, report = case
        result = attestation.validate_runtime_record(record_path, metadata, report, root)
        self.assertEqual(result["record"]["sha256"], digest(record_path.read_bytes()))
        self.assertEqual(result["binary_sha256"], record["binary_sha256"])
        self.assertEqual(result["protocol"], record["protocol"])
        self.assertEqual(result["lane"], "runtime-only-production")
        expected_runtime = [f"wtime-runtime/{feature}"
                            for feature in record["runtime_features"]]
        self.assertEqual(result["runtime_features"], expected_runtime)
        return result

    def test_decoded_compact_and_register_receipts_validate(self):
        for mode in ("decoded", "compact", "register"):
            with self.subTest(mode=mode):
                case = self.create_case(mode)
                try:
                    result = self.assert_valid(case)
                    self.assertEqual(result["command_features"], case[3]["runtime_features"])
                    wrapper = Path(case[3]["workspace_manifest"]["path"]).read_text(encoding="utf-8")
                    self.assertIn(
                        'wtime-runtime = { path = "' + str(case[1] / "crates/wtime-runtime")
                        + '", default-features = false }', wrapper)
                    self.assertIn(
                        'wtime-types = { path = "' + str(case[1] / "crates/wtime-types")
                        + '", default-features = false }', wrapper)
                    if mode in ("compact", "register"):
                        _temporary, root, record_path, record, metadata, report = case
                        workspace = Path(record["workspace_manifest"]["workspace_path"]).parent
                        self.assertTrue(workspace.resolve().is_relative_to(root.resolve()))
                        shutil.rmtree(workspace)
                        self.assertEqual(
                            attestation.validate_runtime_record(record_path, metadata, report, root), result)
                finally:
                    case[0].cleanup()

    def test_rejects_tampered_record_and_archives(self):
        mutations = {
            "feature-set": lambda record, _output: record.__setitem__("cargo_features", ["runtime-only", "bad"]),
            "core-resolution": lambda record, _output: record["core_resolution"].__setitem__("wtime-runtime", "/tmp/other/Cargo.toml"),
            "unknown-compiled-input": lambda record, _output: record["compiled_inputs"].__setitem__("surprise.rs", {}),
        }
        for label, mutate in mutations.items():
            with self.subTest(tamper=label):
                case = self.create_case("decoded")
                try:
                    temporary, root, record_path, record, metadata, report = case
                    mutate(record, record_path.parent)
                    safe_write(root, record_path, json.dumps(record).encode())
                    with self.assertRaises(attestation.AttestationError):
                        attestation.validate_runtime_record(record_path, metadata, report, root)
                finally:
                    case[0].cleanup()

        for label, relative in (("generated-entrypoint", "compiled/src/main.rs"),
                                ("archived-source", "source/Cargo.toml")):
            with self.subTest(tamper=label):
                case = self.create_case("compact")
                try:
                    temporary, root, record_path, _record, metadata, report = case
                    path = record_path.parent / relative
                    safe_write(root, path, path.read_bytes() + b"tampered\n")
                    with self.assertRaises(attestation.AttestationError):
                        attestation.validate_runtime_record(record_path, metadata, report, root)
                finally:
                    case[0].cleanup()

    def test_rejects_mode_protocol_binary_and_metadata_tampering(self):
        for label in ("instrumentation", "protocol", "binary", "metadata-features"):
            with self.subTest(tamper=label):
                case = self.create_case("decoded")
                try:
                    _temporary, root, record_path, record, metadata, report = case
                    if label == "instrumentation":
                        metadata["settings"]["instrumented"] = True
                    elif label == "protocol":
                        metadata["capabilities"]["candidate"]["instrumented"] = True
                    elif label == "binary":
                        safe_write(root, Path(record["binary_path"]), b"different binary")
                    else:
                        safe_write(root, Path(record["metadata"]["path"]),
                                   json.dumps({"packages": [{"name": "wtime-runtime", "source": "registry"}]}).encode())
                    with self.assertRaises(attestation.AttestationError):
                        attestation.validate_runtime_record(record_path, metadata, report, root)
                finally:
                    case[0].cleanup()

    def test_rejects_external_packages_and_wrong_core_paths_in_cargo_metadata(self):
        for label in ("external-package", "wrong-core-path"):
            with self.subTest(tamper=label):
                case = self.create_case("compact")
                try:
                    _temporary, root, record_path, record, metadata, report = case
                    refs = record["metadata"]
                    for key in ("path", "after_path"):
                        path = Path(refs[key])
                        value = json.loads(path.read_text(encoding="utf-8"))
                        package = next(p for p in value["packages"]
                                       if p["name"] == ("wtime-production-protocol" if label == "external-package"
                                                         else "wtime-runtime"))
                        if label == "external-package":
                            package["source"] = "registry+https://invalid.example/index"
                        else:
                            package["manifest_path"] = str(root / "wrong-core/Cargo.toml")
                        safe_write(root, path, json.dumps(value).encode())
                        refs["sha256" if key == "path" else "after_sha256"] = digest(path.read_bytes())
                        log_name = ("metadata-before.stdout" if key == "path" else "metadata-after.stdout")
                        record["logs"][log_name]["sha256"] = digest(path.read_bytes())
                    safe_write(root, record_path, json.dumps(record).encode())
                    with self.assertRaises(attestation.AttestationError):
                        attestation.validate_runtime_record(record_path, metadata, report, root)
                finally:
                    case[0].cleanup()


if __name__ == "__main__":
    unittest.main()
