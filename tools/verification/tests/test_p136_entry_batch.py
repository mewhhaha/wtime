"""Synthetic integrity checks for P136 evidence envelopes.

All records are created in a fresh temporary root. These tests never seed or
modify the repository's provenance, archive, or verification artifact paths.
"""
from __future__ import annotations

import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

SOURCE = Path(__file__).resolve().parents[1] / "producers/p136.py"
spec = importlib.util.spec_from_file_location("p136_producer_tests", SOURCE)
p136 = importlib.util.module_from_spec(spec)
assert spec and spec.loader
spec.loader.exec_module(p136)


def _ref(path: Path) -> dict:
    data = path.read_bytes()
    return {"path": str(path.resolve()), "sha256": hashlib.sha256(data).hexdigest(), "bytes": len(data)}


class P136EvidenceEnvelopeTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory(prefix="p136-evidence-")
        self.root = Path(self.temp.name).resolve()

    def tearDown(self) -> None:
        self.temp.cleanup()

    def evidence(self) -> tuple[Path, dict]:
        files = {
            "archive/manifest.json": {"schema": 1, "mode": "register"},
            "adapter/adapter.json": {"kind": "source-bound-register-entry-adapter", "executable_sha256": "a" * 64},
            "candidate/adapter.json": {"kind": "source-bound-register-entry-candidate", "executable_sha256": "b" * 64},
            "collection/report.json": {"status": "passed", "timing_evidence": False,
                                        "smoke": True, "same_source_smoke": False},
        }
        paths = {}
        for rel, value in files.items():
            path = self.root / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(json.dumps(value), encoding="utf-8")
            paths[rel] = path
        report = {
            "schema": "wtime-p136-verification-v1", "status": "passed", "timing_evidence": False,
            "decision": "integrity", "smoke": True, "same_source_smoke": False,
            "source_stable": True,
            "predecessor": _ref(paths["archive/manifest.json"]),
            "adapter": _ref(paths["adapter/adapter.json"]),
            "candidate": _ref(paths["candidate/adapter.json"]),
            "collection": _ref(paths["collection/report.json"]),
            "adapter_executable_sha256": "a" * 64,
            "candidate_executable_sha256": "b" * 64,
            "counts": {"rows": 8, "pairs": 32, "runs": 2, "named_cases": len(p136.CASES)},
        }
        report_path = self.root / "evidence.json"
        report_path.write_text(json.dumps(report), encoding="utf-8")
        self.assertTrue(report_path.resolve().is_relative_to(self.root))
        self.assertTrue(all(path.resolve().is_relative_to(self.root) for path in paths.values()))
        return report_path, paths

    def test_rejects_foreign_adapter_identity(self) -> None:
        path, paths = self.evidence()
        paths["adapter/adapter.json"].write_text(json.dumps({"kind": "legacy-api-adapter"}), encoding="utf-8")
        report = json.loads(path.read_text())
        report["adapter"] = _ref(paths["adapter/adapter.json"])
        path.write_text(json.dumps(report), encoding="utf-8")
        with self.assertRaisesRegex(p136.P136Error, "distinct verified Register executables"):
            p136.verify_evidence(path)

    def test_rejects_candidate_binary_or_source_identity_change(self) -> None:
        path, paths = self.evidence()
        paths["candidate/adapter.json"].write_text(json.dumps({
            "kind": "source-bound-register-entry-candidate", "executable_sha256": "c" * 64,
        }), encoding="utf-8")
        report = json.loads(path.read_text())
        report["candidate"] = _ref(paths["candidate/adapter.json"])
        path.write_text(json.dumps(report), encoding="utf-8")
        with self.assertRaisesRegex(p136.P136Error, "distinct verified Register executables"):
            p136.verify_evidence(path)

    def test_rejects_smoke_only_or_incomplete_pair_receipts(self) -> None:
        path, _ = self.evidence()
        report = json.loads(path.read_text())
        report["counts"]["pairs"] = 31
        path.write_text(json.dumps(report), encoding="utf-8")
        with self.assertRaisesRegex(p136.P136Error, "count summary"):
            p136.verify_evidence(path)

    def test_rejects_checkpoint_or_raw_record_mutation(self) -> None:
        path, paths = self.evidence()
        paths["collection/report.json"].write_text("{truncated", encoding="utf-8")
        with self.assertRaisesRegex(p136.P136Error, "artifact hash/size/path is stale"):
            p136.verify_evidence(path)


if __name__ == "__main__":
    unittest.main()
