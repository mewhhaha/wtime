"""Synthetic retained representative image checks for the P046 gate."""
from __future__ import annotations

import gzip
import hashlib
from pathlib import Path
import sys
import tempfile
import unittest

BENCH = Path(__file__).resolve().parents[1]
sys.dont_write_bytecode = True
sys.path.insert(0, str(BENCH))

import p046_gate as gate
import representative_compare as compare
import representative_scaling as scaling
import representative_workloads as model
from test_p046_memory_evidence import valid_wire


class P046ImageValidationTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="p046-image-validation-")
        self.root = Path(self.temporary.name).resolve()
        self.addCleanup(self.temporary.cleanup)
        self.entry = model.entries()[0]

    def inside(self, relative: str) -> Path:
        path = (self.root / relative).resolve()
        self.assertTrue(path.is_relative_to(self.root), "synthetic output escaped fresh temp root")
        return path

    def report(self, baseline, candidate):
        path = self.inside("comparison.json")
        path.write_text("{}\n", encoding="utf-8")
        report = {"runs": [{"pairs": [{"workload": self.entry["id"],
                                         "baseline": baseline, "candidate": candidate}]}]}
        return report, {"path": str(path)}

    def retained(self, mode: str):
        wire = valid_wire(self.entry)
        wire["mode"] = mode
        checked = compare.validate_observation(self.entry, wire, model, scaling, 1,
                                               expected_mode=mode)
        return compare.retain_memory_image(checked, self.root)

    def test_valid_images_and_full_proofs_pass(self):
        baseline = self.retained("decoded")
        candidate = self.retained("compact")
        report, reference = self.report(baseline, candidate)

        self.assertEqual(gate._representative_images(report, reference), 1)

    def test_corrupt_compressed_artifact_fails_checksum(self):
        baseline = self.retained("decoded")
        candidate = self.retained("compact")
        artifact = self.inside(baseline["image_ref"]["path"])
        damaged = bytearray(artifact.read_bytes())
        damaged[-1] ^= 0x80
        artifact.write_bytes(damaged)
        report, reference = self.report(baseline, candidate)

        with self.assertRaisesRegex(gate.PortfolioError, "compressed image is stale"):
            gate._representative_images(report, reference)

    def test_rehashed_wrong_full_image_fails_independent_oracle(self):
        baseline = self.retained("decoded")
        candidate = self.retained("compact")
        ref = dict(baseline["image_ref"])
        good = gzip.decompress(self.inside(ref["path"]).read_bytes())
        wrong = bytearray(good)
        wrong[0] ^= 1
        wrong = bytes(wrong)
        compressed = gzip.compress(wrong, mtime=0)
        wrong_path = self.inside("memory-images/wrong.bin.gz")
        wrong_path.write_bytes(compressed)
        ref.update(path="memory-images/wrong.bin.gz",
                   compressed_sha256=hashlib.sha256(compressed).hexdigest(),
                   raw_sha256=hashlib.sha256(wrong).hexdigest(), raw_bytes=len(wrong))
        baseline = dict(baseline, image_ref=ref)
        report, reference = self.report(baseline, candidate)

        with self.assertRaisesRegex(gate.PortfolioError, "retained representative oracle failed"):
            gate._representative_images(report, reference)

    def test_wrong_stored_proof_fails_oracle_binding(self):
        baseline = self.retained("decoded")
        candidate = self.retained("compact")
        bad = dict(baseline["validated_memory_proof"])
        bad["full_memory_sha256"] = "0" * 64
        baseline = dict(baseline, validated_memory_proof=bad)
        report, reference = self.report(baseline, candidate)

        with self.assertRaisesRegex(gate.PortfolioError, "proof differs"):
            gate._representative_images(report, reference)

    def test_escaping_image_path_is_rejected_before_read(self):
        baseline = self.retained("decoded")
        candidate = self.retained("compact")
        baseline = dict(baseline, image_ref=dict(baseline["image_ref"],
                                                  path="memory-images/../../outside.bin.gz"))
        report, reference = self.report(baseline, candidate)

        with self.assertRaisesRegex(gate.PortfolioError, "path escapes"):
            gate._representative_images(report, reference)


if __name__ == "__main__":
    unittest.main()
