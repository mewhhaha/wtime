"""Bounded memory-image archival checks for P046 representative evidence."""
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

import representative_compare as compare
import representative_scaling as scaling
import representative_workloads as model


def valid_wire(entry: dict) -> dict:
    size = entry["params"][0]["baseline"]
    expected = model.oracle(entry["id"], size)
    image = model.expected_memory_image(entry["id"], size)
    return {
        "schema_version": 2,
        "case": entry["id"],
        "stage": "run_after_setup",
        "input": str(size),
        "expected": f"i32:{expected['result']}",
        "result": f"i32:{expected['result']}",
        "batch": 1,
        "calls_checked": 1,
        "report_complete": True,
        "setup_reset_before_each_call": True,
        "setup_excluded_from_timing": True,
        "instrumented": False,
        "measured_guest_ns": 250.0,
        "ns_per_operation": 250.0,
        "mode": "decoded",
        "profile": "mvp",
        "memory_pages": entry["memory_page_count"],
        "memory_page_size_log2": 16,
        "memory_hex": image.hex(),
        "expected_output_sha256": expected["output_sha256"],
    }


def validate(entry: dict, wire: dict) -> dict:
    return compare.validate_observation(entry, wire, model, scaling, 1,
                                        expected_mode="decoded")


class P046MemoryEvidenceTests(unittest.TestCase):
    def setUp(self):
        self.entries = model.entries()
        self.assertEqual(len(self.entries), 9)

    def temp_root(self):
        temporary = tempfile.TemporaryDirectory(prefix="p046-memory-evidence-")
        self.addCleanup(temporary.cleanup)
        root = Path(temporary.name).resolve()
        image_dir = root / "memory-images"
        image_dir.resolve().relative_to(root)
        return root

    def test_same_image_reuses_one_hashed_artifact_and_keeps_observation_fields(self):
        root = self.temp_root()
        entry = self.entries[0]
        first_wire = valid_wire(entry)
        second_wire = dict(first_wire)
        first = compare.retain_memory_image(validate(entry, first_wire), root)
        second = compare.retain_memory_image(validate(entry, second_wire), root)

        self.assertEqual(first["image_ref"], second["image_ref"])
        self.assertEqual(len(list((root / "memory-images").glob("*.bin.gz"))), 1)
        self.assertNotIn("memory_hex", first)
        self.assertNotIn("memory_hex", first["raw_wire_observation"])
        self.assertEqual(first["ns_per_operation"], 250.0)
        self.assertTrue(first["setup_reset_before_each_call"])
        ref = first["image_ref"]
        artifact = (root / ref["path"]).resolve()
        artifact.relative_to(root)
        raw = gzip.decompress(artifact.read_bytes())
        self.assertEqual(hashlib.sha256(raw).hexdigest(), ref["raw_sha256"])
        self.assertEqual(hashlib.sha256(artifact.read_bytes()).hexdigest(), ref["compressed_sha256"])

    def test_changed_valid_image_gets_a_distinct_content_address(self):
        root = self.temp_root()
        first_entry, second_entry = self.entries[0], self.entries[1]
        first = compare.retain_memory_image(validate(first_entry, valid_wire(first_entry)), root)
        second = compare.retain_memory_image(validate(second_entry, valid_wire(second_entry)), root)

        self.assertNotEqual(first["image_ref"]["raw_sha256"], second["image_ref"]["raw_sha256"])
        self.assertNotEqual(first["image_ref"]["path"], second["image_ref"]["path"])
        self.assertEqual(len(list((root / "memory-images").glob("*.bin.gz"))), 2)

    def test_oracle_rejects_bad_image_before_any_archive_is_written(self):
        root = self.temp_root()
        entry = self.entries[0]
        bad = valid_wire(entry)
        image = bytearray.fromhex(bad["memory_hex"])
        image[0] ^= 1
        bad["memory_hex"] = image.hex()

        with self.assertRaises((compare.RepresentativeComparisonError,
                                scaling.RepresentativeError)):
            validated = validate(entry, bad)
            compare.retain_memory_image(validated, root)
        self.assertFalse((root / "memory-images").exists())

    def test_corrupt_existing_content_address_fails_closed(self):
        root = self.temp_root()
        entry = self.entries[0]
        validated = validate(entry, valid_wire(entry))
        first = compare.retain_memory_image(validated, root)
        artifact = (root / first["image_ref"]["path"]).resolve()
        artifact.relative_to(root)
        corrupted = bytearray(artifact.read_bytes())
        corrupted[-1] ^= 0x80
        artifact.write_bytes(corrupted)

        with self.assertRaisesRegex(compare.RepresentativeComparisonError,
                                    "existing memory image artifact is corrupt"):
            compare.retain_memory_image(validated, root)


if __name__ == "__main__":
    unittest.main()
