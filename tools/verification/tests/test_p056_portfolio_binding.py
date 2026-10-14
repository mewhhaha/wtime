from __future__ import annotations

import hashlib
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]
BENCH = ROOT / "tools/wtime-bench"
PRODUCERS = ROOT / "tools/verification/producers"
sys.path.insert(0, str(BENCH))
sys.path.insert(0, str(PRODUCERS))
import p046_gate as gate
import p046_measure

CORE_CRATES = ("wtime-runtime", "wtime-types", "wtime-decode", "wtime-validate")

def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()

class P056PortfolioBindingTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()

    def write_inside(self, path: Path, data: bytes):
        path = path.resolve()
        self.assertTrue(path.is_relative_to(self.root), path)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        return path

    def fixture(self, root: Path | None = None):
        root = (root or self.root).resolve()
        self.assertTrue(root.is_relative_to(self.root), root)
        paths = ["Cargo.toml", "Cargo.lock", "rust-toolchain.toml"]
        for crate in CORE_CRATES:
            paths += [f"crates/{crate}/Cargo.toml", f"crates/{crate}/src/lib.rs"]
        paths += [f"crates/wtime-runtime/src/generated/{index:03}.rs"
                  for index in range(113 - len(paths))]
        current = {}
        for index, relative in enumerate(paths):
            payload = f"synthetic core source {index}\n".encode()
            self.write_inside(root / "current" / relative, payload)
            current[relative] = digest(payload)
        self.assertEqual(len(current), 113)
        archive = root / "archive"
        files = {}
        for relative, source_hash in current.items():
            payload = (root / "current" / relative).read_bytes()
            stored = f"source/{relative}"
            self.write_inside(archive / stored, payload)
            files[stored] = {"role": "source", "source_path": relative,
                             "sha256": source_hash, "size_bytes": len(payload)}
        manifest = {"schema": 1, "kind": "synthetic-test-archive",
                    "mode": "compact", "files": files}
        manifest_bytes = (json.dumps(manifest, sort_keys=True) + "\n").encode()
        manifest_path = self.write_inside(archive / "manifest.json", manifest_bytes)
        runtime = {path: value for path, value in current.items()
                   if path.startswith("crates/wtime-runtime/src/") and path.endswith(".rs")}
        declaration = {"schema": "wtime-p056-predecessor-v1", "task": "P056",
                       "status": "passed", "source_stable": True,
                       "archive": {"path": str(manifest_path),
                                   "sha256": digest(manifest_bytes),
                                   "bytes": len(manifest_bytes)},
                       "runtime_source_map": runtime, "core_source_map": current,
                       "source_current": True}
        declaration_path = self.write_inside(
            root / "declaration.json",
            (json.dumps(declaration, sort_keys=True) + "\n").encode())
        declaration_bytes = declaration_path.read_bytes()
        reference = {"path": str(declaration_path), "sha256": digest(declaration_bytes),
                     "bytes": len(declaration_bytes)}
        return declaration, reference, current, archive

    def test_current_declaration_and_archive_are_exactly_bound(self):
        declaration, reference, current, _archive = self.fixture()
        verified = gate._load_structural_predecessor(
            reference, current_core_source_map=current)
        self.assertEqual(verified.declaration, declaration)
        self.assertEqual(len(verified.declaration["core_source_map"]), 113)
        self.assertEqual(verified.archive_path, Path(declaration["archive"]["path"]))

    def test_stale_reference_and_substituted_archive_manifest_reject(self):
        declaration, reference, current, archive = self.fixture()
        stale = dict(reference, sha256="0" * 64)
        with self.assertRaisesRegex(gate.PortfolioError, "declaration hash is stale"):
            gate._load_structural_predecessor(stale, current_core_source_map=current)
        changed = b"substituted archive source\n"
        self.write_inside(archive / "source/crates/wtime-runtime/src/lib.rs", changed)
        manifest_path = archive / "manifest.json"
        manifest = json.loads(manifest_path.read_text())
        manifest["files"]["source/crates/wtime-runtime/src/lib.rs"]["sha256"] = digest(changed)
        manifest["files"]["source/crates/wtime-runtime/src/lib.rs"]["size_bytes"] = len(changed)
        manifest_bytes = (json.dumps(manifest, sort_keys=True) + "\n").encode()
        self.write_inside(manifest_path, manifest_bytes)
        # The test helper rewrites only inside the temporary root.
        manifest_path.write_bytes(manifest_bytes)
        declaration["archive"] = {"path": str(manifest_path), "sha256": digest(manifest_bytes),
                                   "bytes": len(manifest_bytes)}
        declaration_path = self.write_inside(
            self.root / "substituted-declaration.json",
            (json.dumps(declaration, sort_keys=True) + "\n").encode())
        raw = declaration_path.read_bytes()
        substituted_ref = {"path": str(declaration_path), "sha256": digest(raw), "bytes": len(raw)}
        with self.assertRaisesRegex(gate.PortfolioError, "archive full core source map differs"):
            gate._load_structural_predecessor(
                substituted_ref, current_core_source_map=current)

    def test_complete_matrix_summary_counts_all_ten_datasets_without_retiming(self):
        verify_dir = self.root / "verify"
        (verify_dir / "production").mkdir(parents=True)
        (verify_dir / "baselines").mkdir()
        candidate = self.write_inside(self.root / "candidate.bin", b"synthetic production binary\n")
        p001_manifest = self.write_inside(self.root / "p001/manifest.json", b"synthetic p001 adapter\n")
        p001 = self.write_inside(self.root / "p001/reference.json", b"synthetic p001 reference\n")
        structural_manifest = self.write_inside(self.root / "structural/manifest.json", b"synthetic structural adapter\n")
        structural = self.write_inside(self.root / "structural/reference.json", b"synthetic structural reference\n")
        production = {"binary_path": str(candidate), "binary_sha256": digest(candidate.read_bytes()),
                      "source_stable": True}
        baseline_report = {
            "p001_adapter": {"path": str(p001), "sha256": digest(p001_manifest.read_bytes())},
            "structural_adapter": {"path": str(structural), "sha256": digest(structural_manifest.read_bytes())},
        }
        self.write_inside(verify_dir / "production/build-record.json",
                          (json.dumps(production) + "\n").encode())
        self.write_inside(verify_dir / "baselines/report.json",
                          (json.dumps(baseline_report) + "\n").encode())
        declaration = self.write_inside(self.root / "predecessor.json", b"synthetic predecessor\n")
        predecessor = {"path": str(declaration), "sha256": digest(declaration.read_bytes()),
                       "bytes": declaration.stat().st_size}
        output = self.root / "measurements"
        calls = []

        def fake_comparison(argv, destination, label, **_kwargs):
            destination = Path(destination).resolve()
            self.assertTrue(destination.is_relative_to(self.root), destination)
            calls.append(label)
            command_log = destination / "commands.jsonl"
            if not command_log.exists():
                self.write_inside(command_log, b"")
            report_path = destination / label / "report.json"
            self.write_inside(report_path, b'{"synthetic":true}\n')
            return report_path

        with patch.object(p046_measure, "_structural_reference", return_value=predecessor), \
             patch.object(p046_measure, "_comparison", side_effect=fake_comparison):
            summary = p046_measure.measure(verify_dir, output, cold=True,
                                           structural_predecessor=predecessor,
                                           cache=self.root / "cache")

        self.assertEqual(len(calls), 10)
        self.assertEqual(set(calls), {
            f"{baseline}-{family}"
            for baseline in ("p001", "structural")
            for family in gate.FAMILIES
        } | {"cold-p001-original-mvp", "cold-structural-original-mvp"})
        self.assertEqual(summary["status"], "passed")
        self.assertEqual(summary["portfolio_pair_count"], 144720)
        self.assertEqual(set(summary["reports"]), {"p001", "structural"})
        for baseline in ("p001", "structural"):
            self.assertEqual(set(summary["reports"][baseline]), set(gate.FAMILIES))
        self.assertEqual(set(summary["cold_controls"]), {"p001", "structural"})
        saved = json.loads((output / "report.json").read_text())
        self.assertEqual(saved, summary)

    def test_default_p046_structural_binding_still_rejects_legacy_adapter(self):
        adapter = self.write_inside(self.root / "adapter/manifest.json", b"{}\n").parent
        wrong_predecessor = self.root / "wrong-predecessor"
        wrong_predecessor.mkdir()
        import baseline
        import p123_adapter
        report = {"metadata": {"baseline_archive_path": str(adapter)}}
        with patch.object(baseline, "verify_archive", return_value={"kind": "frozen-typed-adapter"}), \
             patch.object(p123_adapter, "verify", return_value={"legacy_api": True, "mode": "decoded"}):
            with self.assertRaisesRegex(gate.PortfolioError, "current API"):
                gate._verify_baseline_archive(report, "decoded", check_sources=True)
        with patch.object(baseline, "verify_archive", return_value={"kind": "frozen-typed-adapter"}), \
             patch.object(p123_adapter, "verify", return_value={
                 "legacy_api": False, "mode": "decoded", "baseline": str(wrong_predecessor)}):
            with self.assertRaisesRegex(gate.PortfolioError, "predeclared predecessor"):
                gate._verify_baseline_archive(report, "decoded", check_sources=True)

if __name__ == "__main__":
    unittest.main()
