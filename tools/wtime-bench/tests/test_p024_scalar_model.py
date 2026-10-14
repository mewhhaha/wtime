"""Focused contract tests for P024's normal-path try_table scalar fixture."""
import hashlib
import json
from pathlib import Path
import sys
import unittest

TOOL = Path(__file__).resolve().parents[1]
ROOT = TOOL.parents[1]
sys.path.insert(0, str(TOOL))
import scalar_workloads
import workloads

FAMILY = "scalar_try_table_normal"
INPUTS = (1, 32, 1024, 10000, 0, 17)
# Canonical hashes of the exact pre-P024 registry rows retained in the P025
# candidate source snapshot. These keep this regression independent of target/.
PRE_P024_WORKLOADS_SHA256 = "991d6cd1e741ffa80e9bba1e8fbc01ebb2ac8aeb170c1675635a7c747d278e76"
PRE_P024_SCALAR_SELECTION_SHA256 = "28acd09a54caf1198aa8b3c45986a873f6a81d753bd74c5a9deff2c971b7e013"
SOURCE_SHA256 = "ab92800dd4e4d4053498c317effadedf52568270ab7b476eef74b09638de8964"
WASM_SHA256 = "97068a6310a3bc535369e2b6277b801875c14ca206a707fcb9bca9036f41d0df"

# Ordered IDs from the immutable P025 retained registry before P024. These
# protect the original rows while allowing later tasks to append new rows.
ORIGINAL_IDS = (
    'call_overhead', 'integer_loop', 'memory_fill_scan', 'branch_calls',
    'prepare_large', 'scalar_accumulator_n1', 'scalar_accumulator_n32', 'scalar_accumulator_n1024',
    'scalar_accumulator_n10000', 'scalar_accumulator_n0', 'scalar_accumulator_n17', 'scalar_dependent_n1',
    'scalar_dependent_n32', 'scalar_dependent_n1024', 'scalar_dependent_n10000', 'scalar_dependent_n0',
    'scalar_dependent_n17', 'scalar_branch_entropy_n1', 'scalar_branch_entropy_n32', 'scalar_branch_entropy_n1024',
    'scalar_branch_entropy_n10000', 'scalar_branch_entropy_n0', 'scalar_branch_entropy_n17', 'scalar_br_table_n1',
    'scalar_br_table_n32', 'scalar_br_table_n1024', 'scalar_br_table_n10000', 'scalar_br_table_n0',
    'scalar_br_table_n17', 'scalar_direct_calls_n1', 'scalar_direct_calls_n32', 'scalar_direct_calls_n1024',
    'scalar_direct_calls_n10000', 'scalar_direct_calls_n0', 'scalar_direct_calls_n17', 'scalar_indirect_calls_n1',
    'scalar_indirect_calls_n32', 'scalar_indirect_calls_n1024', 'scalar_indirect_calls_n10000', 'scalar_indirect_calls_n0',
    'scalar_indirect_calls_n17', 'scalar_tail_calls_n1', 'scalar_tail_calls_n32', 'scalar_tail_calls_n1024',
    'scalar_tail_calls_n10000', 'scalar_tail_calls_n0', 'scalar_tail_calls_n17', 'scalar_dynamic_api_n1',
    'scalar_dynamic_api_n32', 'scalar_dynamic_api_n1024', 'scalar_dynamic_api_n10000', 'scalar_dynamic_api_n0',
    'scalar_dynamic_api_n17', 'scalar_multivalue_branch_n1', 'scalar_multivalue_branch_n32', 'scalar_multivalue_branch_n1024',
    'scalar_multivalue_branch_n10000', 'scalar_multivalue_branch_n0', 'scalar_multivalue_branch_n17', 'scalar_ordinary_recursion_n0',
    'scalar_ordinary_recursion_n1', 'scalar_ordinary_recursion_n17', 'scalar_ordinary_recursion_n32', 'scalar_call_ref_n1',
    'scalar_call_ref_n32', 'scalar_call_ref_n1024', 'scalar_call_ref_n10000', 'scalar_call_ref_n0',
    'scalar_call_ref_n17', 'scalar_table_mutation_n1', 'scalar_table_mutation_n32', 'scalar_table_mutation_n1024',
    'scalar_table_mutation_n10000', 'scalar_table_mutation_n0', 'scalar_table_mutation_n17', 'scalar_host_call_n1',
    'scalar_host_call_n32', 'scalar_host_call_n1024', 'scalar_host_call_n10000', 'scalar_host_call_n0',
    'scalar_host_call_n17', 'simd_integer_lanes_n0', 'simd_integer_lanes_n17', 'simd_integer_lanes_n1000',
    'simd_scalar_integer_lanes_n0', 'simd_scalar_integer_lanes_n17', 'simd_scalar_integer_lanes_n1000', 'simd_saturating_select_n0',
    'simd_saturating_select_n17', 'simd_saturating_select_n1000', 'simd_lane_memory_n0', 'simd_lane_memory_n17',
    'simd_lane_memory_n1000', 'simd_shuffle_n0', 'simd_shuffle_n17', 'simd_shuffle_n1000',
    'simd_scalar_shuffle_n0', 'simd_scalar_shuffle_n17', 'simd_scalar_shuffle_n1000', 'simd_float_arithmetic_n0',
    'simd_float_arithmetic_n17', 'simd_float_arithmetic_n1000', 'simd_scalar_float_arithmetic_n0', 'simd_scalar_float_arithmetic_n17',
    'simd_scalar_float_arithmetic_n1000', 'simd_f64_arithmetic_n0', 'simd_f64_arithmetic_n17', 'simd_f64_arithmetic_n1000',
    'simd_scalar_f64_arithmetic_n0', 'simd_scalar_f64_arithmetic_n17', 'simd_scalar_f64_arithmetic_n1000', 'simd_relaxed_swizzle_n0',
    'simd_relaxed_swizzle_n17', 'simd_relaxed_swizzle_n1000', 'simd_lane_memory_footprint_16', 'simd_lane_memory_footprint_4096',
    'simd_lane_memory_footprint_65536', 'simd_float_min_zero', 'simd_float_max_zero', 'simd_float_nan_classify',
    'simd_float_convert_pos_inf', 'simd_float_convert_nan', 'simd_typed_f64_neg_1_75', 'simd_typed_f64_pos_3_5',
    'simd_typed_v128_lanes', 'simd_f64_min_negative_zero', 'simd_f64_min_positive_zero', 'simd_f64_min_nan',
    'simd_f64_min_positive_value', 'simd_scalar_float_min_zero', 'simd_scalar_float_max_zero', 'simd_scalar_float_nan_classify',
    'simd_scalar_float_convert_pos_inf', 'simd_scalar_float_convert_nan', 'simd_relaxed_swizzle_oob_17', 'simd_integer_lanes_random_00',
    'simd_scalar_integer_lanes_random_00', 'simd_integer_lanes_random_01', 'simd_scalar_integer_lanes_random_01', 'simd_integer_lanes_random_02',
    'simd_scalar_integer_lanes_random_02', 'simd_integer_lanes_random_03', 'simd_scalar_integer_lanes_random_03', 'simd_integer_lanes_random_04',
    'simd_scalar_integer_lanes_random_04', 'simd_integer_lanes_random_05', 'simd_scalar_integer_lanes_random_05', 'simd_integer_lanes_random_06',
    'simd_scalar_integer_lanes_random_06', 'simd_integer_lanes_random_07', 'simd_scalar_integer_lanes_random_07', 'simd_integer_lanes_random_08',
    'simd_scalar_integer_lanes_random_08', 'simd_integer_lanes_random_09', 'simd_scalar_integer_lanes_random_09', 'simd_integer_lanes_random_10',
    'simd_scalar_integer_lanes_random_10', 'simd_integer_lanes_random_11', 'simd_scalar_integer_lanes_random_11', 'simd_integer_lanes_random_12',
    'simd_scalar_integer_lanes_random_12', 'simd_integer_lanes_random_13', 'simd_scalar_integer_lanes_random_13', 'simd_integer_lanes_random_14',
    'simd_scalar_integer_lanes_random_14', 'simd_integer_lanes_random_15', 'simd_scalar_integer_lanes_random_15', 'simd_integer_lanes_random_16',
    'simd_scalar_integer_lanes_random_16', 'simd_integer_lanes_random_17', 'simd_scalar_integer_lanes_random_17', 'simd_integer_lanes_random_18',
    'simd_scalar_integer_lanes_random_18', 'simd_integer_lanes_random_19', 'simd_scalar_integer_lanes_random_19', 'simd_integer_lanes_random_20',
    'simd_scalar_integer_lanes_random_20', 'simd_integer_lanes_random_21', 'simd_scalar_integer_lanes_random_21', 'simd_integer_lanes_random_22',
    'simd_scalar_integer_lanes_random_22', 'simd_integer_lanes_random_23', 'simd_scalar_integer_lanes_random_23', 'simd_integer_lanes_random_24',
    'simd_scalar_integer_lanes_random_24', 'simd_integer_lanes_random_25', 'simd_scalar_integer_lanes_random_25', 'simd_integer_lanes_random_26',
    'simd_scalar_integer_lanes_random_26', 'simd_integer_lanes_random_27', 'simd_scalar_integer_lanes_random_27', 'simd_integer_lanes_random_28',
    'simd_scalar_integer_lanes_random_28', 'simd_integer_lanes_random_29', 'simd_scalar_integer_lanes_random_29', 'gc_packed_array_n1',
    'gc_packed_array_n32', 'gc_packed_array_n1024', 'gc_struct_graph_n1', 'gc_struct_graph_n32',
    'gc_struct_graph_n1024', 'gc_nominal_exact_casts_n1', 'gc_nominal_exact_casts_n32', 'gc_nominal_exact_casts_n1024',
    'gc_typed_function_refs_n1', 'gc_typed_function_refs_n32', 'gc_typed_function_refs_n1024', 'gc_descriptor_graph_n1',
    'gc_descriptor_graph_n32', 'gc_descriptor_graph_n1024', 'gc_descriptor_cross_module_n1', 'gc_descriptor_cross_module_n32',
    'gc_descriptor_cross_module_n1024', 'gc_root_lifecycle_n1', 'gc_root_lifecycle_n32', 'gc_root_lifecycle_n1024',
    'gc_host_reentry_collect_n1', 'gc_host_reentry_collect_n32', 'gc_host_reentry_collect_n1024', 'gc_continuation_root_n1',
    'gc_continuation_root_n32', 'gc_continuation_root_n1024', 'exception_caught_throw_n1', 'exception_caught_throw_n32',
    'exception_caught_throw_n1024', 'exception_normal_path_n1', 'exception_normal_path_n32', 'exception_normal_path_n1024',
    'exception_cross_call_catch_n1', 'exception_cross_call_catch_n32', 'exception_cross_call_catch_n1024', 'exception_host_throw_catch_n1',
    'exception_host_throw_catch_n32', 'exception_host_throw_catch_n1024', 'exception_throw_ref_unwind_n1', 'exception_throw_ref_unwind_n32',
    'exception_throw_ref_unwind_n1024', 'exception_uncaught_throw_n1', 'exception_uncaught_throw_n32', 'exception_uncaught_throw_n1024',
    'continuation_depth_0', 'continuation_depth_4', 'continuation_depth_32', 'continuation_captured_reference',
    'continuation_bound_nested', 'continuation_one_shot', 'continuation_cross_module_consumer', 'threads_atomic_add_same_w1_n32',
    'threads_atomic_add_same_w1_n1024', 'threads_atomic_add_same_w2_n32', 'threads_atomic_add_same_w2_n1024', 'threads_atomic_add_same_w4_n32',
    'threads_atomic_add_same_w4_n1024', 'threads_atomic_add_distinct_w1_n32', 'threads_atomic_add_distinct_w1_n1024', 'threads_atomic_add_distinct_w2_n32',
    'threads_atomic_add_distinct_w2_n1024', 'threads_atomic_add_distinct_w4_n32', 'threads_atomic_add_distinct_w4_n1024', 'threads_ordinary_rw_n32',
    'threads_ordinary_rw_n1024', 'threads_atomic_load_store_n32', 'threads_atomic_load_store_n1024', 'threads_wait_notify_latency',
    'threads_sc_store_buffering', 'representative_parser_hash_small', 'representative_parser_hash_medium', 'representative_parser_hash_held_out',
    'representative_image_filter_small', 'representative_image_filter_medium', 'representative_image_filter_held_out', 'representative_graph_bfs_small',
    'representative_graph_bfs_medium', 'representative_graph_bfs_held_out', 'memory_widths_predictable', 'memory_widths_shuffled',
    'memory64_i32_extended', 'memory_growth_success_zero', 'memory_growth_success_one', 'memory_growth_success_max',
    'memory_growth_failure', 'memory_oob_copy_order', 'memory_effective_address_overflow', 'memory_widths_read_scan',
    'memory_custom_page_one_byte', 'memory_multi_memory_second_export', 'memory_copy_forward_64', 'memory_copy_forward_4096',
    'memory_copy_forward_1048576', 'memory_copy_backward_64', 'memory_copy_backward_4096', 'memory_copy_backward_1048576',
    'memory_fill_64', 'memory_fill_4096', 'memory_fill_1048576', 'memory_copy_aliased_64',
    'memory_copy_aliased_4096', 'memory_copy_aliased_1048576',
)

ORIGINAL_SCALAR_IDS = (
    'call_overhead', 'integer_loop', 'branch_calls', 'scalar_accumulator_n1',
    'scalar_accumulator_n32', 'scalar_accumulator_n1024', 'scalar_accumulator_n10000', 'scalar_accumulator_n0',
    'scalar_accumulator_n17', 'scalar_dependent_n1', 'scalar_dependent_n32', 'scalar_dependent_n1024',
    'scalar_dependent_n10000', 'scalar_dependent_n0', 'scalar_dependent_n17', 'scalar_branch_entropy_n1',
    'scalar_branch_entropy_n32', 'scalar_branch_entropy_n1024', 'scalar_branch_entropy_n10000', 'scalar_branch_entropy_n0',
    'scalar_branch_entropy_n17', 'scalar_br_table_n1', 'scalar_br_table_n32', 'scalar_br_table_n1024',
    'scalar_br_table_n10000', 'scalar_br_table_n0', 'scalar_br_table_n17', 'scalar_direct_calls_n1',
    'scalar_direct_calls_n32', 'scalar_direct_calls_n1024', 'scalar_direct_calls_n10000', 'scalar_direct_calls_n0',
    'scalar_direct_calls_n17', 'scalar_indirect_calls_n1', 'scalar_indirect_calls_n32', 'scalar_indirect_calls_n1024',
    'scalar_indirect_calls_n10000', 'scalar_indirect_calls_n0', 'scalar_indirect_calls_n17', 'scalar_tail_calls_n1',
    'scalar_tail_calls_n32', 'scalar_tail_calls_n1024', 'scalar_tail_calls_n10000', 'scalar_tail_calls_n0',
    'scalar_tail_calls_n17', 'scalar_dynamic_api_n1', 'scalar_dynamic_api_n32', 'scalar_dynamic_api_n1024',
    'scalar_dynamic_api_n10000', 'scalar_dynamic_api_n0', 'scalar_dynamic_api_n17', 'scalar_multivalue_branch_n1',
    'scalar_multivalue_branch_n32', 'scalar_multivalue_branch_n1024', 'scalar_multivalue_branch_n10000', 'scalar_multivalue_branch_n0',
    'scalar_multivalue_branch_n17', 'scalar_ordinary_recursion_n0', 'scalar_ordinary_recursion_n1', 'scalar_ordinary_recursion_n17',
    'scalar_ordinary_recursion_n32', 'scalar_call_ref_n1', 'scalar_call_ref_n32', 'scalar_call_ref_n1024',
    'scalar_call_ref_n10000', 'scalar_call_ref_n0', 'scalar_call_ref_n17', 'scalar_table_mutation_n1',
    'scalar_table_mutation_n32', 'scalar_table_mutation_n1024', 'scalar_table_mutation_n10000', 'scalar_table_mutation_n0',
    'scalar_table_mutation_n17', 'scalar_host_call_n1', 'scalar_host_call_n32', 'scalar_host_call_n1024',
    'scalar_host_call_n10000', 'scalar_host_call_n0', 'scalar_host_call_n17',
)


def _canonical_sha(value):
    payload = json.dumps(value, sort_keys=True, separators=(",", ":")).encode()
    return hashlib.sha256(payload).hexdigest()


class P024ScalarModelTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.registry = json.loads((TOOL / "workloads.json").read_text())
        cls.rows = [entry for entry in cls.registry["workloads"]
                    if entry.get("family") == FAMILY]
        cls.model_rows = [entry for entry in scalar_workloads.entries()
                          if entry["family"] == FAMILY]
        cls.source = TOOL / "fixtures/scalar/try_table_normal.wat"
        cls.wasm = TOOL / "fixtures/scalar/try_table_normal.wasm"

    def test_primary_and_heldout_inputs_use_signed_triangular_oracle(self):
        self.assertEqual(tuple(entry["oracle_arguments"]["n"] for entry in self.model_rows), INPUTS)
        for entry in self.model_rows:
            n = entry["oracle_arguments"]["n"]
            expected = (n * (n - 1) // 2) & 0xFFFF_FFFF
            if expected & 0x8000_0000:
                expected -= 0x1_0000_0000
            self.assertEqual(scalar_workloads.oracle(entry["id"], n), expected)
            role = "held_out" if n in {0, 17} else "primary"
            self.assertEqual(entry["validation_role"], role)

    def test_required_features_profile_result_and_stages_are_declared(self):
        self.assertEqual(len(self.rows), len(INPUTS))
        for row in self.rows:
            self.assertEqual(row["required_features"], ["mvp", "exception_handling"])
            self.assertEqual(row["driver"], {"kind": "plain", "profile": "all-phase3-core"})
            self.assertEqual(row["result"], {"type": "i32"})
            self.assertEqual(row["stages"], scalar_workloads.STAGES)
            self.assertEqual(row["comparators"]["wasmtime"]["feature_profile"], ["exception_handling"])

    def test_original_cc0_wat_and_existing_encoded_wasm_match_metadata(self):
        self.assertTrue(self.source.is_file())
        self.assertTrue(self.wasm.is_file())
        self.assertIn("SPDX-License-Identifier: CC0-1.0", self.source.read_text())
        self.assertEqual(hashlib.sha256(self.source.read_bytes()).hexdigest(), SOURCE_SHA256)
        self.assertEqual(hashlib.sha256(self.wasm.read_bytes()).hexdigest(), WASM_SHA256)
        self.assertEqual(scalar_workloads.WASM_SHA256[self.source.name], WASM_SHA256)
        for row in self.model_rows:
            self.assertEqual(row["source"]["sha256"], SOURCE_SHA256)
            self.assertEqual(row["source"]["wasm_sha256"], WASM_SHA256)
            self.assertEqual(row["source"]["encoding_status"], "encoded")

    def test_registry_adds_six_rows_and_preserves_the_298_archived_rows(self):
        added_ids = {row["id"] for row in self.rows}
        self.assertEqual([row["id"] for row in self.rows], [row["id"] for row in self.model_rows])
        for row, model_row in zip(self.rows, self.model_rows):
            self.assertEqual({key: value for key, value in row.items() if key != "driver"}, model_row)
        rows_by_id = {row["id"]: row for row in self.registry["workloads"]}
        self.assertTrue(set(ORIGINAL_IDS).issubset(rows_by_id))
        original_rows = [rows_by_id[workload_id] for workload_id in ORIGINAL_IDS]
        self.assertEqual(_canonical_sha(original_rows), PRE_P024_WORKLOADS_SHA256)
        selection = self.registry["selections"]["scalar"]["workloads"]
        original_selection = [workload_id for workload_id in selection
                              if workload_id in set(ORIGINAL_SCALAR_IDS)]
        self.assertEqual(tuple(original_selection), ORIGINAL_SCALAR_IDS)
        self.assertEqual(_canonical_sha(original_selection), PRE_P024_SCALAR_SELECTION_SHA256)
        self.assertGreaterEqual(len(self.registry["workloads"]), 304)
        self.assertLess((TOOL / "workloads.json").stat().st_size, 512 * 1024)
        self.assertEqual(workloads.validate_registry(self.registry)["workloads"], self.registry["workloads"])


if __name__ == "__main__":
    unittest.main()
