"""P043 memory64 boundary preflight mode and full-image checks."""
from __future__ import annotations

import hashlib
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

TOOL = Path(__file__).resolve().parents[1]
if str(TOOL) not in sys.path:
    sys.path.insert(0, str(TOOL))
import compare
import memory_scaling
import memory_workloads


PAGE = memory_workloads.PAGE
IMAGE = bytes(PAGE)
IMAGE_HEX = IMAGE.hex()
IMAGE_SHA = hashlib.sha256(IMAGE).hexdigest()


class P043MemoryPreflightTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.entry = next(
            entry for entry in memory_workloads.entries()
            if entry["family"] == "memory64_i32_extended"
        )
        cls.expected = memory_workloads.expected_memory(cls.entry["id"], PAGE)

    def _wire(self, mode):
        return {
            "schema_version": 2,
            "case": self.entry["id"],
            "stage": "load_and_call",
            "input": f"i32:{PAGE}",
            "expected": "trap",
            "result": "trap",
            "mode": mode,
            "profile": "all-phase3-core",
            "driver": "plain",
            "batch": 1,
            "ns_per_operation": 1.0,
            "trap_kind": "Some(MemoryOutOfBounds)",
            "trap_message": "out of bounds memory access",
            "memory_pages": 1,
            "memory_page_size_log2": 16,
            "exported_memory_index": 0,
            "memory_hex": IMAGE_HEX,
        }

    def _probe(self, mode, wire):
        with tempfile.TemporaryDirectory(prefix="p043-memory-preflight-") as temp:
            root = Path(temp).resolve()
            fixture = root / "memory64.wasm"
            self.assertTrue(fixture.resolve().is_relative_to(root))
            with mock.patch.object(compare, "process", return_value=wire) as process:
                result = compare.memory_boundary_probe(
                    Path("diagnostic-exe"), fixture, self.entry, self.expected,
                    mode, None, memory_workloads,
                )
            return result, process.call_args.args[1]

    def test_selected_mode_is_forwarded_and_validated(self):
        for mode in ("decoded", "compact", "reference"):
            with self.subTest(mode=mode):
                (verified, wire), args = self._probe(mode, self._wire(mode))
                self.assertEqual(args[5], mode)
                self.assertEqual(verified["result"], "trap")
                self.assertEqual(verified["full_memory_sha256"], IMAGE_SHA)
                self.assertEqual(wire["mode"], mode)
                self.assertNotIn("memory_hex", wire)

    def test_wrong_returned_mode_is_rejected_without_memory_payload(self):
        wire = self._wire("decoded")
        self.assertEqual(len(wire["memory_hex"]), PAGE * 2)
        with self.assertRaises(memory_scaling.BaselineError) as caught:
            self._probe("compact", wire)
        message = str(caught.exception)
        self.assertIn("outcome/identity mismatch", message)
        self.assertNotIn(IMAGE_HEX, message)

    def test_tampered_full_memory_image_is_rejected(self):
        wire = self._wire("compact")
        wire["memory_hex"] = "01" + wire["memory_hex"][2:]
        with self.assertRaises(memory_scaling.BaselineError) as caught:
            self._probe("compact", wire)
        self.assertIn("full-memory hash mismatch", str(caught.exception))


if __name__ == "__main__":
    unittest.main()
