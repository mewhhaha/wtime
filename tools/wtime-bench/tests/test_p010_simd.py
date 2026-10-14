import hashlib
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(HERE))

import simd_workloads


class SimdWorkloadTests(unittest.TestCase):
    def test_entries_bind_sources_features_and_encoded_binary_hashes(self):
        entries = simd_workloads.entries()
        self.assertEqual(len(entries), 114)
        self.assertEqual([entry["params"][0]["baseline"] for entry in entries[:3]], [0, 17, 1000])
        for entry in entries:
            source = HERE / entry["source"]["path"]
            self.assertEqual(hashlib.sha256(source.read_bytes()).hexdigest(), entry["source"]["sha256"])
            self.assertEqual(entry["source"]["wasm_sha256"],
                             simd_workloads.WASM_SHA256[Path(entry["source"]["path"]).name])
            if simd_workloads.WASM_SHA256[Path(entry["source"]["path"]).name] is None:
                self.assertIsNone(entry["source"]["wasm_sha256"])
                self.assertEqual(entry["source"]["encoding_status"], "pending")
                self.assertIn("awaiting existing conformance encoder", entry["source"]["encoding_note"])
            else:
                self.assertEqual(entry["source"]["encoding_status"], "encoded")
            self.assertEqual(entry["result"]["type"], "i32")
            self.assertEqual(entry["comparators"]["wasmtime"]["status"], "probe_required")
        self.assertTrue(any("relaxed_simd" in entry["required_features"] for entry in entries))
        self.assertTrue(any("saturating_float_to_int" in entry["required_features"] for entry in entries))

    def test_wrapping_lane_oracle_and_memory_effects(self):
        self.assertEqual(simd_workloads.oracle("simd_lane_memory", 0), 10)
        self.assertEqual(simd_workloads.oracle("simd_lane_memory", 17), 180)
        self.assertEqual(simd_workloads.oracle("simd_lane_memory", 1000), 10010)

        lanes = [0x7FFF_FFFF, 0xFFFF_FFFE, 3, 4]
        additions = [1, 0xFFFF_FFFD, 5, 6]
        factors = [3, 5, 7, 9]
        masks = [324508639, 0xDB97531F, 1431655765, 0xAAAA_AAAA]
        updated = [((((lane + add) * factor) & 0xFFFF_FFFF) ^ mask) & 0xFFFF_FFFF
                   for lane, add, factor, mask in zip(lanes, additions, factors, masks)]
        total = sum(updated) & 0xFFFF_FFFF
        expected = total - 0x1_0000_0000 if total & 0x8000_0000 else total
        self.assertEqual(simd_workloads.oracle("simd_integer_lanes", 1), expected)

    def test_saturation_and_strict_float_edge_allowed_results(self):
        initial = [120, -120, 10, -10, 127, -128, 1, -1, 3, 4, -5, -6, 50, -50, 0, 0]
        deltas = [20, -20, -20, 20, 1, -1, 2, -2, 8, 9, -10, -11, 90, -90, 127, -128]
        expected_lanes = [max(-128, min(127, a + b)) if i % 2 == 0 else a
                          for i, (a, b) in enumerate(zip(initial, deltas))]
        self.assertEqual(simd_workloads.oracle("simd_saturating_select", 1), sum(expected_lanes))
        self.assertEqual(simd_workloads.allowed_results("simd_float_min_zero"), {-0x80000000})
        self.assertEqual(simd_workloads.allowed_results("simd_float_max_zero"), {0})
        self.assertEqual(simd_workloads.allowed_results("simd_float_nan_classify"), {1})
        self.assertEqual(simd_workloads.allowed_results("simd_float_convert_pos_inf"), {0x7FFFFFFF})
        self.assertEqual(simd_workloads.allowed_results("simd_float_convert_nan"), {0})

    def test_relaxed_swizzle_has_a_singleton_allowed_set(self):
        for n in (0, 1, 17, 1000):
            self.assertEqual(simd_workloads.allowed_results("simd_relaxed_swizzle", n),
                             {120 if n % 2 else 0})
        fixture = (HERE / "fixtures" / "simd" / "relaxed_swizzle.wat").read_text()
        self.assertIn("i8x16.relaxed_swizzle", fixture)
        contract = next(entry["allowed_result_contract"] for entry in simd_workloads.entries()
                        if entry["family"] == "simd_relaxed_swizzle")
        self.assertIn("indices are 0..15", contract["explanation"])

    def test_typed_float_and_vector_controls(self):
        entries = {entry["id"]: entry for entry in simd_workloads.entries()}
        self.assertEqual(entries["simd_typed_f64_neg_1_75"]["params"][0]["value"],
                         0xBFFC000000000000)
        self.assertEqual(entries["simd_typed_f64_neg_1_75"]["expected_result"], -1)
        self.assertEqual(entries["simd_typed_v128_lanes"]["params"][0]["value"],
                         "01000000feffffff0300000004000000")
        self.assertEqual(entries["simd_typed_v128_lanes"]["expected_result"], 6)
        self.assertEqual(simd_workloads.oracle("simd_f64_min_negative_zero"), 1)
        self.assertEqual(simd_workloads.oracle("simd_f64_min_positive_zero"), 0)
        self.assertEqual(simd_workloads.oracle("simd_f64_min_nan"), -1)
        f64 = (HERE / "fixtures" / "simd" / "f64_minmax.wat").read_text()
        self.assertIn("f64.min", f64)
        self.assertIn("i64.reinterpret_f64", f64)

    def test_scalar_and_vector_kernels_have_equal_explicit_work(self):
        entries = {entry["id"]: entry for entry in simd_workloads.entries()}
        for n in (0, 1, 17, 1000):
            for vector_id, scalar_id in (
                (f"simd_integer_lanes_n{n}", f"simd_scalar_integer_lanes_n{n}"),
                (f"simd_shuffle_n{n}", f"simd_scalar_shuffle_n{n}"),
                (f"simd_float_arithmetic_n{n}", f"simd_scalar_float_arithmetic_n{n}"),
                (f"simd_f64_arithmetic_n{n}", f"simd_scalar_f64_arithmetic_n{n}"),
            ):
                self.assertEqual(simd_workloads.oracle(vector_id, n),
                                 simd_workloads.oracle(scalar_id, n))
                if n in simd_workloads.INPUTS:
                    self.assertEqual(entries[vector_id]["work_units"]["count"],
                                     entries[scalar_id]["work_units"]["count"])
                    self.assertEqual(entries[vector_id]["equivalent_to"], scalar_id)

    def test_lane_memory_exports_and_checks_full_memory_state(self):
        for n in (0, 17, 1000):
            expected = simd_workloads.expected_memory(f"simd_lane_memory_n{n}", n)
            lanes = [1 + n, 2 + 2 * n, 3 + 3 * n, 4 + 4 * n]
            touched = b"".join((lane & 0xFFFF_FFFF).to_bytes(4, "little") for lane in lanes)
            self.assertEqual(expected["range"], [0, 16])
            self.assertEqual(expected["sha256"], hashlib.sha256(touched).hexdigest())
            self.assertEqual(expected["full_memory_sha256"],
                             hashlib.sha256(touched + bytes(65536 - 16)).hexdigest())
        fixture = (HERE / "fixtures" / "simd" / "lane_memory.wat").read_text()
        self.assertIn('(export "memory")', fixture)

    def test_relaxed_swizzle_records_complete_out_of_range_set(self):
        self.assertEqual(simd_workloads.allowed_results("simd_relaxed_swizzle_oob_17", 17),
                         {0, 16})
        entry = next(item for item in simd_workloads.entries()
                     if item["id"] == "simd_relaxed_swizzle_oob_17")
        self.assertEqual(entry["expected_results"], [0, 16])
        fixture = (HERE / "fixtures" / "simd" / "relaxed_swizzle_oob.wat").read_text()
        self.assertIn("i8x16.relaxed_swizzle", fixture)
        self.assertIn("i8x16.splat", fixture)

    def test_seeded_randomized_lane_inputs_have_independent_vector_and_scalar_rows(self):
        cases = simd_workloads.randomized_lane_inputs()
        self.assertEqual(len(cases), 30)
        self.assertEqual(cases[:1], [("00", (-546124472, 2071602222, 1651529902, 1239225157))])
        entries = {entry["id"]: entry for entry in simd_workloads.entries()}
        mask32 = 0xFFFF_FFFF
        additions = (1, (-3) & mask32, 5, 6)
        factors = (3, 5, 7, 9)
        masks = (324508639, (-610839777) & mask32, 1431655765, (-1431655766) & mask32)
        for suffix, lanes in cases:
            values = [lane & mask32 for lane in lanes]
            for _ in range(17):
                values = [((((v + a) & mask32) * f & mask32) ^ m) & mask32
                          for v, a, f, m in zip(values, additions, factors, masks)]
            total = sum(values) & mask32
            expected = total - (1 << 32) if total & (1 << 31) else total
            vector_id = f"simd_integer_lanes_random_{suffix}"
            scalar_id = f"simd_scalar_integer_lanes_random_{suffix}"
            self.assertEqual(entries[vector_id]["expected_result"], expected)
            self.assertEqual(entries[scalar_id]["expected_result"], expected)
            self.assertEqual(simd_workloads.oracle(vector_id), expected)
            self.assertEqual(simd_workloads.oracle(scalar_id), expected)
            self.assertEqual(entries[vector_id]["params"][1]["value"],
                             b"".join((lane & mask32).to_bytes(4, "little") for lane in lanes).hex())

    def test_f64_vector_and_scalar_arithmetic_use_exact_finite_values(self):
        # All values are multiples of 1/4, and multiply-by-one is exact in binary64.
        for n in (0, 17, 1000):
            vector = simd_workloads.oracle(f"simd_f64_arithmetic_n{n}", n)
            scalar = simd_workloads.oracle(f"simd_scalar_f64_arithmetic_n{n}", n)
            self.assertEqual(vector, scalar)
            entries = {entry["id"]: entry for entry in simd_workloads.entries()}
            self.assertEqual(entries[f"simd_f64_arithmetic_n{n}"]["work_units"]["count"], n * 4)
            self.assertEqual(entries[f"simd_scalar_f64_arithmetic_n{n}"]["work_units"]["count"], n * 4)

    def test_memory_footprint_fixtures_cover_16b_4kib_and_64kib(self):
        entries = {entry["id"]: entry for entry in simd_workloads.entries()}
        expected = {
            "simd_lane_memory_footprint_16": 16,
            "simd_lane_memory_footprint_4096": 4096,
            "simd_lane_memory_footprint_65536": 65536,
        }
        for workload_id, footprint_bytes in expected.items():
            entry = entries[workload_id]
            memory = entry["memory_expectation"]
            self.assertEqual(memory["range"], [0, footprint_bytes])
            self.assertEqual(memory["bytes_oracle"], "expected_memory")
            touched = b"".join(
                b"".join(value.to_bytes(4, "little") for value in (2, 4, 6, 8))
                for _ in range(footprint_bytes // 16)
            )
            self.assertEqual(memory["sha256"], hashlib.sha256(touched).hexdigest())
            self.assertEqual(memory["full_memory_sha256"], hashlib.sha256(
                touched + bytes(65536 - footprint_bytes)).hexdigest())
            self.assertEqual(entry["expected_result"], simd_workloads.oracle(
                workload_id, entry["params"][0]["value"]))
            self.assertEqual(memory["setup_policy"], "initialize_and_update_inside_timed_run")
            self.assertNotIn("bytes_hex", memory)
            self.assertNotIn("full_memory_hex", memory)

    def test_added_fixtures_encode_to_pinned_hashes(self):
        encoder = Path("/workspace/wtime/target/debug/wtime-conformance")
        if not encoder.is_file():
            self.skipTest("existing conformance encoder unavailable; no build requested")
        with tempfile.TemporaryDirectory() as tempdir:
            hashes = simd_workloads.encode_fixtures(encoder, tempdir)
        self.assertEqual(hashes, simd_workloads.WASM_SHA256)


if __name__ == "__main__":
    unittest.main()
