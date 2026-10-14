"""Portable P027 integration checks; resolve harness modules from this test's tool dir."""
from __future__ import annotations
import hashlib
import importlib
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest

TOOL = Path(__file__).resolve().parents[1]
if str(TOOL) not in sys.path:
    sys.path.insert(0, str(TOOL))
import compare
import memory_scaling
import memory_workloads
import workloads

MERGER_SPEC = importlib.util.spec_from_file_location(
    "p027_registry_merger", TOOL.parents[1] / "scripts" / "merge_registry.py")
assert MERGER_SPEC and MERGER_SPEC.loader
MERGER = importlib.util.module_from_spec(MERGER_SPEC)
MERGER_SPEC.loader.exec_module(MERGER)

OLD_ROWS_SHA256 = "5e0e9a303077328e8282485823911b46084ba4afab389942accd1662ab4a261a"
OLD_MEMORY_IDS_SHA256 = "f4486706f7c4f1c241c8a6d096426de3268c50a951c33f3fc562cec634343e61"
POLICY = "prepare_plus_instantiate_plus_first_call_excluding_host_seed"


def canonical_sha(value):
    wire = json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()
    return hashlib.sha256(wire).hexdigest()


def seed(length, multiplier, addend):
    page_multiplier = 53 if multiplier == 29 else 37
    return bytearray((i * multiplier + addend + (i >> 16) * page_multiplier) & 255 for i in range(length))


def observation(entry, input_value, expected, images, *, stage="first_call", batch=1):
    driver = memory_scaling._driver(entry)
    selected = len(images) - 1
    return {"schema_version": 2, "case": entry["id"], "stage": stage,
            "input": f"i32:{input_value}", "expected": f"i32:{input_value}",
            "result": f"i32:{input_value}", "mode": "decoded",
            "profile": memory_scaling._profile(entry), "driver": driver,
            "batch": batch, "ns_per_operation": 123.0,
            "host_setup_ns": 456.0, "host_setup_excluded": True,
            "measurement_policy": POLICY, "memory_pages": expected["memories"][selected]["pages"],
            "memory_page_size_log2": 16, "memory_count": len(images),
            "exported_memory_index": selected,
            "all_memory_hex": [bytes(item).hex() for item in images],
            "memory_hex": bytes(images[selected]).hex(),
            "trap_kind": None, "trap_message": None}


class P027HarnessIntegrationTests(unittest.TestCase):
    def test_registry_appends_ten_rows_and_preserves_original_304_row_prefix(self):
        registry = workloads.load_registry(TOOL / "workloads.json")
        rows = registry["workloads"]
        # The historical P027 prefix is immutable; later families may append.
        self.assertGreaterEqual(len(rows), 314)
        self.assertEqual(canonical_sha(rows[:304]), OLD_ROWS_SHA256)
        selected = registry["selections"]["memory"]["workloads"]
        self.assertEqual(len(selected), 35)
        self.assertEqual(canonical_sha(selected[:25]), OLD_MEMORY_IDS_SHA256)
        additions = [row for row in rows[304:]
                     if row["family"] in {"memory_copy_distinct_seeded", "memory_fill_seeded"}]
        self.assertEqual(len(additions), 10)
        self.assertEqual(selected[25:], [row["id"] for row in additions])
        self.assertEqual([row["id"] for row in additions],
                         [row["id"] for row in memory_workloads.entries()
                          if row["family"] in {"memory_copy_distinct_seeded", "memory_fill_seeded"}])
        compare._validate_memory_model_rows(
            [row for row in selected_entries(registry)
             if row["id"] != "memory_fill_scan"], memory_workloads.entries())
        base = json.loads(json.dumps(registry))
        base["workloads"] = base["workloads"][:304]
        base["selections"]["memory"]["workloads"] = selected[:25]
        base_ids = {row["id"] for row in base["workloads"]}
        for declaration in base["selections"].values():
            declaration["workloads"] = [
                workload_id for workload_id in declaration["workloads"]
                if workload_id in base_ids
            ]
        with tempfile.TemporaryDirectory() as temporary:
            source = Path(temporary) / "base.json"
            destination = Path(temporary) / "merged.json"
            source.write_text(json.dumps(base), encoding="utf-8")
            created = MERGER.merge_registry(source, destination)
            self.assertEqual(canonical_sha(created["workloads"][:304]), OLD_ROWS_SHA256)
            self.assertEqual(len(workloads.load_registry(destination)["workloads"]), 314)

    def test_encoder_hashes_and_driver_routing_are_merged_without_old_changes(self):
        entries = memory_workloads.entries()
        self.assertEqual(len(entries), 34)
        self.assertEqual(len(memory_workloads.WASM_SHA256), 15)
        for entry in entries:
            source = entry["source"]
            self.assertEqual(hashlib.sha256((TOOL / source["path"]).read_bytes()).hexdigest(),
                             source["sha256"])
            self.assertEqual(memory_workloads.WASM_SHA256[Path(source["path"]).name],
                             source["wasm_sha256"])
            if entry["family"] in {"memory_copy_distinct_seeded", "memory_fill_seeded"}:
                wasm = (TOOL / source["path"]).with_suffix(".wasm")
                self.assertEqual(hashlib.sha256(wasm.read_bytes()).hexdigest(),
                                 source["wasm_sha256"])
        self.assertEqual(memory_scaling._driver({"family": "memory_copy_distinct_seeded"}), "seeded-copy")
        self.assertEqual(memory_scaling._driver({"family": "memory_fill_seeded"}), "seeded-fill")
        self.assertEqual(memory_scaling._driver({"family": "memory_copy_aliased"}), "alias")
        self.assertEqual(memory_scaling._driver({"family": "memory_multi_memory_second_export"}), "multi")
        self.assertEqual(memory_scaling._driver({"family": "memory_widths_predictable"}), "plain")
        self.assertIn("memory_copy_distinct_seeded", compare.MEMORY_STAGED_FAMILIES)
        self.assertIn("memory_fill_seeded", compare.MEMORY_STAGED_FAMILIES)

    def test_heldout_copy_zero_and_odd_ranges_match_independent_image_hashes(self):
        for n in (0, 17):
            entry = next(row for row in memory_workloads.entries()
                         if row["id"] == f"memory_copy_distinct_seeded_{n}")
            expected = memory_workloads.expected_memory(entry["id"], n)
            source = seed(16 * 65536, 29, 7)
            destination = seed(17 * 65536, 13, 101)
            destination[65536:65536+n] = source[:n]
            actual = observation(entry, n, expected, [source, destination])
            verified = memory_scaling._validate_seeded_observation(
                actual, entry, n, expected, "decoded", memory_scaling._profile(entry),
                "seeded-copy", stage="first_call", expected_batch=1)
            self.assertEqual(verified["memories"][1]["range"], [65536, 65536+n])
            self.assertEqual(verified["memories"][1]["range_sha256"],
                             expected["memories"][1]["range_sha256"])

    def test_seeded_copy_rejects_bad_setup_source_image_and_destination_range(self):
        entry = next(row for row in memory_workloads.entries()
                     if row["id"] == "memory_copy_distinct_seeded_17")
        expected = memory_workloads.expected_memory(entry["id"], 17)
        source = seed(16 * 65536, 29, 7)
        destination = seed(17 * 65536, 13, 101)
        destination[65536:65553] = source[:17]
        valid = observation(entry, 17, expected, [source, destination])
        for key, value in (("host_setup_excluded", False), ("measurement_policy", "old_wall_timer"),
                           ("driver", "plain"), ("exported_memory_index", 0)):
            bad = dict(valid)
            bad[key] = value
            with self.subTest(key=key), self.assertRaises(memory_scaling.BaselineError):
                memory_scaling._validate_seeded_observation(
                    bad, entry, 17, expected, "decoded", memory_scaling._profile(entry),
                    "seeded-copy", stage="first_call", expected_batch=1)
        mutated_source = bytearray(source)
        mutated_source[0] ^= 1
        bad_source = observation(entry, 17, expected, [mutated_source, destination])
        with self.assertRaisesRegex(memory_scaling.BaselineError, "full image digest"):
            memory_scaling._validate_seeded_observation(
                bad_source, entry, 17, expected, "decoded", memory_scaling._profile(entry),
                "seeded-copy", stage="first_call", expected_batch=1)
        mutated_destination = bytearray(destination)
        mutated_destination[65536 + 2] ^= 1
        bad_range = observation(entry, 17, expected, [source, mutated_destination])
        with self.assertRaises(memory_scaling.BaselineError):
            memory_scaling._validate_seeded_observation(
                bad_range, entry, 17, expected, "decoded", memory_scaling._profile(entry),
                "seeded-copy", stage="first_call", expected_batch=1)

    def test_fill_warm_stage_checks_calibrated_batch_and_full_image(self):
        n = 17
        entry = next(row for row in memory_workloads.entries()
                     if row["id"] == f"memory_fill_seeded_{n}")
        expected = memory_workloads.expected_memory(entry["id"], n)
        image = seed(17 * 65536, 13, 101)
        image[4096:4113] = bytes([90]) * n
        actual = observation(entry, n, expected, [image], stage="warm_call", batch=4)
        validated = memory_scaling._validate_observation(
            actual, entry, n, expected, "decoded", memory_scaling._profile(entry),
            "seeded-fill", stage="warm_call", expected_batch=4)
        self.assertEqual(validated["batch"], 4)
        bad = dict(actual, batch=1)
        with self.assertRaises(memory_scaling.BaselineError):
            memory_scaling._validate_observation(
                bad, entry, n, expected, "decoded", memory_scaling._profile(entry),
                "seeded-fill", stage="warm_call", expected_batch=4)

    def test_unified_validator_keeps_seeded_timing_strict_and_direct_cli_defaults(self):
        n = 0
        entry = next(row for row in memory_workloads.entries()
                     if row["id"] == "memory_fill_seeded_0")
        expected = memory_workloads.expected_memory(entry["id"], n)
        image = seed(17 * 65536, 13, 101)
        actual = observation(entry, n, expected, [image], stage="load_and_call", batch=1)
        driver = memory_scaling._driver(entry)
        profile = memory_scaling._profile(entry)
        cmd = memory_scaling._memory_driver_args(Path("fixture.wasm"), entry, n,
                                                  "i32:0", profile, driver)
        self.assertEqual(cmd[0], "--memory-driver")
        self.assertEqual(cmd[2:5], [entry["id"], "i32:0", "i32:0"])
        self.assertEqual(cmd[-1], "seeded-fill")
        self.assertEqual(actual["stage"], "load_and_call")
        self.assertTrue(actual["host_setup_excluded"])
        memory_scaling._validate_observation(actual, entry, n, expected,
                                              "decoded", profile, driver,
                                              stage="load_and_call", expected_batch=1)
        no_timing = dict(actual)
        no_timing.pop("ns_per_operation")
        with self.assertRaisesRegex(memory_scaling.BaselineError, "invalid timing"):
            memory_scaling._validate_observation(no_timing, entry, n, expected,
                                                  "decoded", profile, driver,
                                                  require_timing=False,
                                                  stage="load_and_call", expected_batch=1)

        staged_entry = next(row for row in memory_workloads.entries()
                            if row["id"] == "memory_fill_seeded_17")
        staged_expected = memory_workloads.expected_memory(staged_entry["id"], 17)
        staged_image = seed(17 * 65536, 13, 101)
        staged_image[4096:4113] = bytes([90]) * 17
        staged_data = observation(staged_entry, 17, staged_expected, [staged_image],
                                  stage="warm_call", batch=4)
        seen = []
        old_process = compare.process
        try:
            compare.process = lambda exe, args, cpu: seen.append(args) or staged_data
            result = compare.memory_observation(
                "unused-binary", "fixture.wasm", staged_entry, 17, staged_expected,
                "warm_call", 4, "decoded", 4, memory_module=memory_workloads)
        finally:
            compare.process = old_process
        self.assertEqual(seen[0][0], "--memory-staged")
        self.assertEqual(seen[0][5:7], ["warm_call", 4])
        self.assertEqual(seen[0][-1], "seeded-fill")
        self.assertEqual(result["batch"], 4)


def selected_entries(registry):
    by_id = {row["id"]: row for row in registry["workloads"]}
    return [by_id[wid] for wid in registry["selections"]["memory"]["workloads"]]


if __name__ == "__main__":
    unittest.main()
