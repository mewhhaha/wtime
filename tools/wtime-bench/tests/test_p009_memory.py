import hashlib
import json
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(HERE))

import memory_workloads
import workloads


class MemoryWorkloadTests(unittest.TestCase):
    def test_entries_bind_sources_features_and_host_memory_expectations(self):
        entries = memory_workloads.entries()
        self.assertGreaterEqual(len(entries), 17)
        ids = {entry["id"] for entry in entries}
        self.assertIn("memory_copy_forward_1048576", ids)
        self.assertIn("memory_copy_backward_4096", ids)
        self.assertIn("memory_copy_aliased_64", ids)
        self.assertIn("memory64_i32_extended", ids)
        self.assertIn("memory_oob_copy_order", ids)
        self.assertIn("memory_effective_address_overflow", ids)
        self.assertIn("memory_widths_read_scan", ids)
        self.assertIn("memory_custom_page_one_byte", ids)
        self.assertIn("memory_multi_memory_second_export", ids)
        for entry in entries:
            source = HERE / entry["source"]["path"]
            self.assertEqual(hashlib.sha256(source.read_bytes()).hexdigest(),
                             entry["source"]["sha256"], entry["id"])
            self.assertEqual(len(entry["source"]["wasm_sha256"]), 64)
            self.assertTrue(entry["memory_expectation"])
            if entry["family"] in {"memory_widths_predictable", "memory_widths_shuffled", "memory_widths_read_scan"}:
                self.assertEqual(entry["reset_policy"], "fresh_instance_for_cold_stages_reused_instance_for_warm_call")
                self.assertEqual(set(entry["stages"]), {"prepare", "instantiate", "first_call", "warm_call", "load_and_call"})
            else:
                self.assertEqual(entry["reset_policy"], "fresh_instance_per_invocation")
                self.assertNotIn("warm_call", entry["stages"])
        memory64 = next(item for item in entries if item["id"] == "memory64_i32_extended")
        self.assertIn("memory64", memory64["required_features"])
        self.assertEqual(memory64["memory_expectation"]["address_extension"], "i32.extend_i32_u")
        imported = next(item for item in entries if item["id"] == "memory_copy_aliased_64")
        self.assertTrue(imported["memory_expectation"]["alias_memories"])
        widths = next(item for item in entries if item["id"] == "memory_widths_predictable")
        self.assertEqual(widths["work_units"], {"count": 64 * 19, "unit": "memory_operations"})
        custom = next(item for item in entries if item["id"] == "memory_custom_page_one_byte")
        self.assertIn("custom_page_sizes", custom["required_features"])
        multi = next(item for item in entries if item["id"] == "memory_multi_memory_second_export")
        self.assertIn("multi_memory", multi["required_features"])
        self.assertEqual(multi["memory_expectation"]["memory_index"], 1)
        workloads.validate_registry({
            "schema_version": 2,
            "workloads": entries,
            "selections": {"memory": {"status": "ready", "workloads": sorted(ids)}},
        })

    def test_signed_widths_and_unaligned_little_endian_stores(self):
        result, memory = memory_workloads._widths_model(1, False)
        # At base 1, the intentionally unaligned stores have exact low bytes.
        self.assertEqual(memory[1], 0xFE)
        self.assertEqual(memory[3:5], b"\x01\x80")
        self.assertEqual(memory[5:9], b"\x01\x00\x00\x80")
        self.assertEqual(memory[9], 0xFE)
        self.assertEqual(memory[11:13], b"\x01\x80")
        self.assertEqual(memory[13:17], b"\x01\x00\x00\x80")
        self.assertEqual(memory[17:25], b"\x01\x00\x00\x00\x00\x00\x00\x80")
        # Explicit sign/zero extension contributions, wrapped as an i32 result.
        expected = -2 + 254 - 32767 + 32769 - 2147483647
        expected += -2 + 254 - 32767 + 32769 - 2147483647 + 2147483649
        expected += -9223372036854775807
        expected &= 0xFFFF_FFFF
        expected = expected - 0x1_0000_0000 if expected & 0x8000_0000 else expected
        self.assertEqual(result, expected)
        self.assertEqual(memory_workloads.oracle("memory_widths_predictable", 1), expected)
        _, predictable = memory_workloads._widths_model(2, False)
        _, shuffled = memory_workloads._widths_model(2, True)
        self.assertNotEqual(predictable[:128], shuffled[:128],
                            "same scalar values must exercise different address order")

    def test_overlapping_bulk_copy_matches_independent_memmove_at_all_sizes(self):
        for size in memory_workloads.SIZES:
            source = bytes((index * 29 + 7) & 0xFF for index in range(size + 1))
            for direction, destination, start in (("forward", 1, 0), ("backward", 0, 1)):
                expected = bytearray(source)
                temporary = source[start:start + size]
                expected[destination:destination + size] = temporary
                result, actual = memory_workloads._bulk_copy_model(size, direction)
                self.assertEqual(actual[:size + 1], expected, (direction, size))
                checksum = sum(expected) & 0xFFFF_FFFF
                signed_checksum = checksum - 0x1_0000_0000 if checksum & 0x8000_0000 else checksum
                self.assertEqual(result, signed_checksum, (direction, size))
                self.assertEqual(memory_workloads.oracle(f"memory_copy_{direction}_{size}", size),
                                 signed_checksum)
        alias = memory_workloads.expected_memory("memory_copy_aliased_64", 64)
        self.assertTrue(alias["alias_memories"])
        self.assertEqual(alias["bytes_hex"],
                         memory_workloads.expected_memory("memory_copy_forward_64", 64)["bytes_hex"])

    def test_bulk_precheck_and_trap_order_preserve_only_prior_effects(self):
        memory = bytearray(b"abcdefgh")
        before = bytes(memory)
        with self.assertRaises(IndexError):
            memory_workloads._checked_memmove(memory, 7, 0, 2)
        self.assertEqual(bytes(memory), before, "failed full-range validation must mutate nothing")

        expected = memory_workloads.expected_memory("memory_oob_copy_order", 0)
        raw = bytes.fromhex(expected["bytes_hex"])
        self.assertEqual(expected["trap"], "trap")
        self.assertEqual(raw[8], 170)
        self.assertEqual(raw[9], 0, "the store after the trapping copy must not happen")
        self.assertEqual(raw[:4], b"\x01\x02\x03\x04", "failed copy must not partially write")
        self.assertTrue(expected["full_range_precheck"])

        overflow = memory_workloads.expected_memory("memory_effective_address_overflow", 0)
        overflow_bytes = bytes.fromhex(overflow["bytes_hex"])
        self.assertEqual(overflow_bytes[8:10], b"\xaa\x00")
        self.assertEqual(overflow["effective_address"], {"base": 1, "offset": 0xFFFF_FFFF})

    def test_memory64_growth_fill_and_encoder_outputs(self):
        self.assertEqual(memory_workloads.oracle("memory64_i32_extended", 65535), -128)
        self.assertEqual(memory_workloads.expected_memory("memory64_i32_extended", 65535)["bytes_hex"], "80")
        self.assertEqual(memory_workloads.oracle("memory_growth_success_max", 16), 17)
        self.assertEqual(memory_workloads.expected_memory("memory_growth_failure", 17)["pages_after"], 1)
        self.assertEqual(memory_workloads.oracle("memory_fill_4096", 4096), 90 * 4096)
        self.assertEqual(memory_workloads.expected_memory("memory_fill_64", 64)["bytes_hex"], "5a" * 64)

        encoder = Path("/workspace/wtime/target/debug/wtime-conformance")
        if not encoder.is_file():
            self.skipTest("existing conformance encoder is unavailable")
        with tempfile.TemporaryDirectory() as tempdir:
            hashes = memory_workloads.encode_fixtures(encoder, tempdir)
            self.assertEqual(hashes, memory_workloads.WASM_SHA256)
            for filename, digest in hashes.items():
                encoded = Path(tempdir, Path(filename).with_suffix(".wasm"))
                self.assertEqual(hashlib.sha256(encoded.read_bytes()).hexdigest(), digest)

    def test_scan_custom_page_size_and_selected_second_memory_oracles(self):
        scan_id = "memory_widths_read_scan"
        scan = memory_workloads.expected_memory(scan_id, 1024)
        scan_bytes = bytes((index * 13 + 7) & 0xFF for index in range(1024))
        self.assertEqual(scan["bytes_hex"], scan_bytes.hex())
        self.assertEqual(scan["result"], sum(scan_bytes))
        self.assertTrue(scan["setup_in_timed_run"])
        self.assertEqual(scan["scan_phase"], "read_only")
        self.assertEqual(scan["full_memory_sha256"], hashlib.sha256(
            scan_bytes + bytes(memory_workloads.PAGE - len(scan_bytes))).hexdigest())

        custom_id = "memory_custom_page_one_byte"
        custom = memory_workloads.expected_memory(custom_id, 255)
        self.assertEqual(custom["page_size_bytes"], 1)
        self.assertEqual(custom["initial_pages"] * custom["page_size_bytes"], 256)
        self.assertEqual(custom["result"], -128)
        self.assertEqual(custom["full_memory_sha256"], hashlib.sha256(bytes(255) + b"\x80").hexdigest())

        multi_id = "memory_multi_memory_second_export"
        multi = memory_workloads.expected_memory(multi_id, 0)
        self.assertEqual(multi["memory_count"], 2)
        self.assertEqual(multi["exported_memory_index"], 1)
        self.assertEqual(multi["memory_index"], 1)
        self.assertEqual(multi["memory_exports"], {"memory0": 0, "memory": 1})
        self.assertEqual(multi["bytes_hex"], "2244")
        self.assertNotEqual(multi["all_memory_sha256"][0], multi["all_memory_sha256"][1])
        self.assertEqual(memory_workloads.oracle(multi_id, 0), 0x11 + 0x22)


if __name__ == "__main__":
    unittest.main()
