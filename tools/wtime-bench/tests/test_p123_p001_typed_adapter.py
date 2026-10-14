"""P123 capability report and immutable-source binding tests.

Synthetic reports and archives are created in a fresh temporary root. These
tests exercise the producer's strict checker; actual guest probes belong to
the full P123 verification command.
"""
from __future__ import annotations

import hashlib
import importlib.util
import json
from pathlib import Path
import stat
import sys
import tempfile
import unittest
from unittest import mock

sys.dont_write_bytecode = True

BENCH = Path(__file__).resolve().parents[1]
PRODUCER_PATH = BENCH.parent / "verification" / "producers" / "p123.py"
BASELINE_PATH = BENCH / "baseline.py"


def load(path: Path, name: str):
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise AssertionError(f"cannot load {path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


producer = load(PRODUCER_PATH, "p123_producer_tests")
baseline = load(BASELINE_PATH, "p123_baseline_tests")
adapter = load(BENCH / "p123_adapter.py", "p123_adapter_tests")


def digest(payload: bytes) -> str:
    return hashlib.sha256(payload).hexdigest()


def synthetic_report():
    """Return complete registry-shaped claims for strict validator tests."""
    import workloads

    families = {}
    for family, count in producer.COUNTS.items():
        entries = workloads.selection(family)
        rows = []
        for entry in entries:
            rows.append({
                "id": entry["id"],
                "passed": True,
                "stages": list(entry["stages"]),
                "fixture_sha256": entry["source"]["wasm_sha256"],
                "observations": [
                    {"stage": stage, "oracle_checked": True, "mode": "decoded"}
                    for stage in entry["stages"]
                ],
            })
        families[family] = {
            "supported": count,
            "passed": count,
            "unsupported": 0,
            "rows": rows,
        }
    return {
        "schema": "p123-capabilities-v1",
        "status": "passed",
        "timing_evidence": False,
        "source_stable": True,
        "compile_source_stable": True,
        "baseline_manifest_sha256": producer.P001_MANIFEST,
        "original_binary_sha256": producer.P001_BINARY,
        "adapter_manifest_sha256": "a" * 64,
        "binary_sha256": "b" * 64,
        "families": families,
    }


def record_file(root: Path, relative: str, contents: bytes, *, executable=False):
    root = root.resolve()
    path = (root / relative).resolve()
    path.relative_to(root)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(contents)
    if executable:
        path.chmod(0o755)
    return {
        "sha256": digest(contents),
        "size_bytes": len(contents),
        "mode_bits": stat.S_IMODE(path.stat().st_mode),
        "role": "synthetic_test_file",
    }


def synthetic_adapter(root: Path):
    """Build a minimal, internally consistent archive tree under ``root``."""
    frozen = root / "baseline"
    adapter_root = root / "adapter"
    frozen_files = {}
    for crate in adapter.CORE:
        for relative, contents in (
            (f"source/crates/{crate}/Cargo.toml", f"[package]\nname='{crate}'\n".encode()),
            (f"source/crates/{crate}/src/lib.rs", f"// {crate}\n".encode()),
        ):
            row = record_file(frozen, relative, contents)
            row["role"] = "source"
            frozen_files[relative] = row
    record_file(frozen, "binary/wtime-bench", b"frozen original", executable=True)
    frozen_files["binary/wtime-bench"] = {
        "sha256": digest(b"frozen original"), "size_bytes": len(b"frozen original"),
        "mode_bits": 0o755, "role": "release_benchmark_executable",
    }
    (frozen / "manifest.json").write_text(json.dumps({"schema": 1, "files": frozen_files}))

    bench = root / "tools" / "wtime-bench"
    (bench / "src").mkdir(parents=True)
    protocol_bytes = b"mod protocol {}\n"
    (bench / "src/protocol.rs").write_bytes(protocol_bytes)
    entrypoint_bytes = adapter.ENTRYPOINT.encode()
    (bench / "src/main.rs").write_bytes(entrypoint_bytes)
    main_bytes = entrypoint_bytes
    route_bytes = b"frozen route helper"
    cargo_bytes = b"[package]\nname='synthetic-bench'\nversion='0.1.0'\n"
    lock_bytes = b"# synthetic locked graph\n"
    (bench / "Cargo.toml").write_bytes(cargo_bytes)
    (bench / "Cargo.lock").write_bytes(lock_bytes)
    harness = {
        "tools/wtime-bench/route.py": digest(route_bytes),
        "tools/wtime-bench/Cargo.toml": digest(cargo_bytes),
        "tools/wtime-bench/Cargo.lock": digest(lock_bytes),
        "tools/wtime-bench/src/main.rs": digest(main_bytes),
        "tools/wtime-bench/src/protocol.rs": digest(protocol_bytes),
    }
    cargo = (
        "[package]\nname='synthetic-adapter'\nversion='0.1.0'\nedition='2024'\n"
        "[dependencies]\n"
        f"wtime-runtime = {{ path = {json.dumps(str(frozen / 'source/crates/wtime-runtime'))} }}\n"
        f"wtime-types = {{ path = {json.dumps(str(frozen / 'source/crates/wtime-types'))} }}\n"
    ).encode()
    adapter_files = {
        "Cargo.toml": record_file(adapter_root, "Cargo.toml", cargo),
        "source/tools/wtime-bench/route.py": record_file(
            adapter_root, "source/tools/wtime-bench/route.py", route_bytes
        ),
        "source/tools/wtime-bench/Cargo.toml": record_file(
            adapter_root, "source/tools/wtime-bench/Cargo.toml", cargo_bytes
        ),
        "source/tools/wtime-bench/Cargo.lock": record_file(
            adapter_root, "source/tools/wtime-bench/Cargo.lock", lock_bytes
        ),
        "source/tools/wtime-bench/src/main.rs": record_file(
            adapter_root, "source/tools/wtime-bench/src/main.rs", main_bytes
        ),
        "source/tools/wtime-bench/src/protocol.rs": record_file(
            adapter_root, "source/tools/wtime-bench/src/protocol.rs", protocol_bytes
        ),
        "src/main.rs": record_file(adapter_root, "src/main.rs", entrypoint_bytes),
        "src/protocol.rs": record_file(adapter_root, "src/protocol.rs", protocol_bytes),
        "binary/wtime-bench": record_file(
            adapter_root, "binary/wtime-bench", b"typed frozen executable", executable=True
        ),
    }
    for name, row in adapter_files.items():
        if name.startswith("source/"):
            row["role"] = "source"
            row["source_path"] = name.removeprefix("source/")
    core_resolution = {
        crate: str((frozen / "source/crates" / crate / "Cargo.toml").resolve())
        for crate in adapter.CORE
    }
    manifest = {
        "schema": 1,
        "kind": adapter.KIND,
        "baseline": str(frozen.resolve()),
        "binary": "binary/wtime-bench",
        "baseline_manifest_sha256": digest((frozen / "manifest.json").read_bytes()),
        "frozen_core_identities": adapter.core_identities(baseline.verify_archive(frozen)),
        "shared_harness_identities": harness,
        "protocol_sha256": digest(protocol_bytes),
        "binary_sha256": digest(b"typed frozen executable"),
        "mode": "decoded",
        "legacy_api": True,
        "protocol": {"instrumented": False, "modes": ["decoded"], "engine": "wtime"},
        "build": {
            "features": ["runtime-only", "legacy-api"],
            "core_resolution": core_resolution,
        },
        "files": adapter_files,
    }
    (adapter_root / "manifest.json").write_text(json.dumps(manifest))
    return adapter_root, frozen, bench, harness


class P123TypedAdapterTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="p123-typed-adapter-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()

    def write_inside_root(self, path: Path, payload: bytes):
        resolved = path.resolve()
        resolved.relative_to(self.root)
        resolved.parent.mkdir(parents=True, exist_ok=True)
        resolved.write_bytes(payload)
        return resolved

    def test_p123_adapter_is_hash_bound_to_immutable_p001(self):
        report = synthetic_report()
        self.assertIs(producer.validate_report(report), report)

        changed_manifest = dict(report)
        changed_manifest["baseline_manifest_sha256"] = "0" * 64
        with self.assertRaisesRegex(ValueError, "immutable P001"):
            producer.validate_report(changed_manifest)

        changed_binary = dict(report)
        changed_binary["original_binary_sha256"] = "1" * 64
        with self.assertRaisesRegex(ValueError, "immutable P001"):
            producer.validate_report(changed_binary)

        adapter_root, frozen, bench, harness = synthetic_adapter(self.root)
        with mock.patch.object(adapter, "HERE", bench), \
                mock.patch.object(adapter, "ROOT", self.root), \
                mock.patch.object(adapter, "harness_identities", return_value=harness):
            checked = adapter.verify(adapter_root, frozen)
            self.assertEqual(checked["frozen_core_identities"],
                             adapter.core_identities(baseline.verify_archive(frozen)))

            changed_core = frozen / "source/crates/wtime-runtime/src/lib.rs"
            changed_core.write_bytes(b"mutated synthetic frozen core")
            with self.assertRaisesRegex(ValueError, "hash mismatch"):
                adapter.verify(adapter_root, frozen)

    def test_p123_typed_scalar_and_memory_routes_preserve_oracles(self):
        report = producer.validate_report(synthetic_report())
        for family, expected_rows in (("scalar", 85), ("memory", 35)):
            with self.subTest(family=family):
                value = report["families"][family]
                self.assertEqual(value["supported"], expected_rows)
                self.assertEqual(value["passed"], expected_rows)
                self.assertEqual(value["unsupported"], 0)
                self.assertTrue(all(
                    observation["oracle_checked"] is True
                    for row in value["rows"]
                    for observation in row["observations"]
                ))

    def test_p123_simd_representative_capabilities_are_actual(self):
        report = producer.validate_report(synthetic_report())
        for family, expected_rows in (("simd", 114), ("representative", 9)):
            with self.subTest(family=family):
                rows = report["families"][family]["rows"]
                self.assertEqual(len(rows), expected_rows)
                self.assertEqual(
                    {observation["mode"] for row in rows for observation in row["observations"]},
                    {"decoded"},
                )
                self.assertTrue(all(row["passed"] is True for row in rows))

        false_claim = synthetic_report()
        false_claim["families"]["simd"]["rows"][0]["observations"][0]["mode"] = "unsupported"
        with self.assertRaisesRegex(ValueError, "oracle/mode"):
            producer.validate_report(false_claim)

    def test_p123_rejects_mutated_archive_and_false_capabilities(self):
        archive = self.root / "baseline"
        source_rel = "source/crates/wtime-runtime/src/lib.rs"
        original = b"synthetic frozen core"
        source = self.write_inside_root(archive / source_rel, original)
        manifest = {
            "schema": 1,
            "files": {
                source_rel: {
                    "sha256": digest(original),
                    "size_bytes": len(original),
                    "mode_bits": stat.S_IMODE(source.stat().st_mode),
                    "role": "source",
                }
            },
        }
        self.write_inside_root(archive / "manifest.json", json.dumps(manifest).encode())
        baseline.verify_archive(archive)
        source.write_bytes(b"mutated synthetic frozen core")
        with self.assertRaisesRegex(ValueError, "hash mismatch"):
            baseline.verify_archive(archive)

        false_claim = synthetic_report()
        false_claim["families"]["memory"]["passed"] = 34
        with self.assertRaisesRegex(ValueError, "invalid capability counts memory"):
            producer.validate_report(false_claim)

        false_claim = synthetic_report()
        false_claim["families"]["representative"]["rows"].pop()
        with self.assertRaisesRegex(ValueError, "capability row membership representative"):
            producer.validate_report(false_claim)

    def test_adapter_binds_compiled_harness_and_keeps_archive_integrity(self):
        adapter_root, frozen, bench, harness = synthetic_adapter(self.root)
        with mock.patch.object(adapter, "HERE", bench), \
                mock.patch.object(adapter, "ROOT", self.root), \
                mock.patch.object(adapter, "harness_identities", return_value=harness):
            manifest_path = adapter_root / "manifest.json"
            manifest = json.loads(manifest_path.read_text())
            cargo_path = adapter_root / "Cargo.toml"
            cargo = cargo_path.read_text().replace(
                str(frozen / "source/crates/wtime-runtime"),
                str(self.root / "substituted-current-runtime"),
            )
            cargo_path.write_text(cargo)
            manifest["files"]["Cargo.toml"] = {
                **manifest["files"]["Cargo.toml"],
                "sha256": digest(cargo.encode()),
                "size_bytes": len(cargo.encode()),
            }
            manifest_path.write_text(json.dumps(manifest))
            with self.assertRaisesRegex(ValueError, "Cargo substituted core sources"):
                adapter.verify(adapter_root, frozen)

        # Current Python analysis changes do not alter the frozen typed
        # adapter. Its source archive is still checked against the manifest.
        good_adapter, frozen, bench, harness = synthetic_adapter(self.root / "analysis")
        with mock.patch.object(adapter, "HERE", bench), \
                mock.patch.object(adapter, "ROOT", good_adapter.parent), \
                mock.patch.object(adapter, "harness_identities", return_value=harness):
            self.assertEqual(adapter.verify(good_adapter, frozen)["binary_sha256"],
                             digest(b"typed frozen executable"))
            archived_python = good_adapter / "source/tools/wtime-bench/route.py"
            archived_python.write_bytes(b"corrupted frozen helper")
            with self.assertRaisesRegex(ValueError, "archive hash mismatch"):
                adapter.verify(good_adapter, frozen)

    def test_current_python_execution_identity_and_compiled_rust_are_separate(self):
        identity_module = load(BENCH / "execution_identity.py", "p123_execution_identity_tests")
        recorded = {
            "tools/wtime-bench/compare.py": digest(b"timer v1"),
            "tools/wtime-bench/run.py": digest(b"oracle v1"),
            "tools/wtime-bench/analyze.py": digest(b"analysis v1"),
            "crates/wtime-runtime/src/lib.rs": "1" * 64,
        }
        current = dict(recorded)
        current["tools/wtime-bench/analyze.py"] = digest(b"analysis v2")
        self.assertEqual(identity_module.verify(recorded, current)["excluded_analysis_inputs"].keys(),
                         {"tools/wtime-bench/analyze.py"})
        for path in ("tools/wtime-bench/compare.py", "tools/wtime-bench/run.py"):
            changed = dict(recorded)
            changed[path] = digest(b"changed execution input")
            with self.subTest(path=path), self.assertRaisesRegex(
                    ValueError, "execution source identity is stale"):
                identity_module.verify(recorded, changed)

        for relative, replacement in (("src/protocol.rs", b"changed compiled protocol\n"),
                                      ("Cargo.lock", b"changed locked graph\n")):
            with self.subTest(relative=relative):
                adapter_root, frozen, bench, harness = synthetic_adapter(
                    self.root / ("rust-drift-" + relative.replace("/", "-")))
                with mock.patch.object(adapter, "HERE", bench), \
                        mock.patch.object(adapter, "ROOT", adapter_root.parent), \
                        mock.patch.object(adapter, "harness_identities", return_value=harness):
                    (bench / relative).write_bytes(replacement)
                    with self.assertRaisesRegex(ValueError, "current Cargo/Rust harness differs"):
                        adapter.verify(adapter_root, frozen)


if __name__ == "__main__":
    unittest.main()
