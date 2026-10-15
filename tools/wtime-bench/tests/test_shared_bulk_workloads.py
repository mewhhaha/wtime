"""Contained pure tests for P078 shared-bulk rows and full-image validation."""
from __future__ import annotations

import importlib.util
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
BENCH = HERE.parent
sys.dont_write_bytecode = True
sys.path.insert(0, str(BENCH))
import workloads

spec = importlib.util.spec_from_file_location("shared_bulk_workloads", BENCH / "shared_bulk_workloads.py")
model = importlib.util.module_from_spec(spec)
assert spec.loader is not None
spec.loader.exec_module(model)


def contained(root: Path, path: Path) -> Path:
    resolved_root = root.resolve()
    resolved = path.resolve()
    if not resolved.is_relative_to(resolved_root):
        raise AssertionError(f"synthetic path escapes fresh test root: {resolved}")
    return resolved


def wire(entry, batch=1):
    expected = model.oracle(entry["id"], entry["params"][0]["baseline"])
    return {
        "schema": model.SCHEMA, "schema_version": model.SCHEMA_VERSION,
        "case": entry["id"], "mode": model.MODE, "profile": model.PROFILE,
        "stage": "steady_state_group",
        "backing": expected["backing"], "workers": expected["workers"],
        "bytes": entry["params"][1]["baseline"], "repetitions": model.REPETITIONS,
        "batch": batch, "instrumented": False, "report_complete": True,
        "elapsed_ns": 20_000 * batch,
        "worker_returns": expected["worker_returns"] * batch,
        "worker_affinity": [[1, 2, 3, 4] for _ in range(expected["workers"])],
        "calls_checked": batch * expected["workers"], "every_group_full_memory_checked": True,
        "backings_hex": {name: value.hex() for name, value in model.expected(entry).items()},
    }


class SharedBulkWorkloadTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="p078-shared-bulk-")
        self.root = Path(self.temp.name).resolve()
        self.addCleanup(self.temp.cleanup)
        self.entries = model.entries()
        fixture = contained(self.root, self.root / model.WAT_PATH)
        contained(self.root, fixture.parent).mkdir(parents=True, exist_ok=True)
        source = (BENCH / model.WAT_PATH).read_bytes()
        self.assertEqual(hashlib.sha256(source).hexdigest(), model.WAT_SHA256)
        contained(self.root, fixture).write_bytes(source)
        fixture_wasm = contained(self.root, fixture.with_suffix(".wasm"))
        wasm_source = (BENCH / model.WAT_PATH).with_suffix(".wasm").read_bytes()
        contained(self.root, fixture_wasm).write_bytes(wasm_source)

    def test_rows_cover_alias_and_distinct_at_small_large_and_heldout_sizes(self):
        self.assertEqual(len(self.entries), 12)
        self.assertEqual({row["shared_bulk"]["backing"] for row in self.entries}, {"alias", "distinct"})
        self.assertEqual({row["shared_bulk"]["workers"] for row in self.entries}, {1, 4})
        self.assertEqual({row["params"][1]["baseline"] for row in self.entries}, {512, 4097, 32768})
        self.assertTrue(all(row["shared_bulk"]["repetitions"] == 32 for row in self.entries))
        self.assertTrue(all(row["source"]["wasm_sha256"] == model.WASM_SHA256 for row in self.entries))
        fixture_wasm = contained(self.root, self.root / model.WAT_PATH).with_suffix(".wasm")
        self.assertEqual(hashlib.sha256(fixture_wasm.read_bytes()).hexdigest(), model.WASM_SHA256)

    def test_alias_four_worker_oracle_checks_full_backing_and_returns(self):
        entry = next(row for row in self.entries
                     if row["id"] == "shared_bulk_copy_alias_w4_b32768")
        expected = model.oracle(entry["id"], entry["params"][0]["baseline"])
        observed = model.validate_observation(entry, wire(entry), expected_thread_cpus=[1, 2, 3, 4])
        self.assertEqual(expected["worker_returns"], [32] * 4)
        self.assertEqual(expected["full_memory_bytes_checked"], model.MEMORY_BYTES)
        self.assertEqual(expected["full_memory_pages_checked"], 4)
        self.assertEqual(observed["result"], expected)
        self.assertEqual(observed["memory_proof"]["full_backing_sha256"]["shared"],
                         expected["full_backing_sha256"]["shared"])

    def test_distinct_sources_stay_unchanged_and_destinations_are_verified(self):
        entry = next(row for row in self.entries
                     if row["id"] == "shared_bulk_copy_distinct_w4_b4097")
        expected = model.oracle(entry["id"], entry["params"][0]["baseline"])
        observed = model.validate_observation(entry, wire(entry), expected_thread_cpus=[1, 2, 3, 4])
        self.assertEqual(set(model.expected(entry)), {"source", "destination"})
        self.assertEqual(observed["memory_proof"]["full_memory_bytes_checked"],
                         2 * model.MEMORY_BYTES)
        self.assertEqual(observed["memory_proof"]["full_memory_pages_checked"], 8)
        self.assertEqual(observed["memory_proof"]["full_backing_sha256"],
                         expected["full_backing_sha256"])

    def test_truncated_or_corrupt_image_and_bad_worker_results_fail(self):
        entry = next(row for row in self.entries
                     if row["id"] == "shared_bulk_copy_distinct_w1_b512")
        for mutate in (
            lambda row: row["backings_hex"].update(source=row["backings_hex"]["source"][:-2]),
            lambda row: row["worker_returns"].__setitem__(0, 31),
            lambda row: row.__setitem__("report_complete", False),
            lambda row: row.__setitem__("extra", True),
        ):
            row = wire(entry)
            mutate(row)
            with self.subTest(row_keys=sorted(row)), self.assertRaises(model.SharedBulkError):
                model.validate_observation(entry, row)

    def test_batch_affinity_and_checked_worker_calls_are_bound(self):
        entry = next(row for row in self.entries
                     if row["id"] == "shared_bulk_copy_distinct_w4_b512")
        row = wire(entry, batch=3)
        normalized = model.validate_observation(
            entry, row, expected_batch=3, expected_thread_cpus=[1, 2, 3, 4])
        self.assertEqual(normalized["batch"], 3)
        self.assertEqual(normalized["ns_per_operation"], 20_000)
        maximum = wire(entry, batch=model.MAX_BATCH)
        self.assertEqual(model.validate_observation(entry, maximum)["batch"], model.MAX_BATCH)
        too_many = wire(entry, batch=model.MAX_BATCH + 1)
        with self.assertRaises(model.SharedBulkError):
            model.validate_observation(entry, too_many)
        for mutate in (
            lambda value: value.__setitem__("calls_checked", 3),
            lambda value: value["worker_affinity"][0].reverse(),
            lambda value: value["worker_affinity"][1].__setitem__(0, 5),
        ):
            bad = wire(entry, batch=3)
            mutate(bad)
            with self.subTest(affinity=bad["worker_affinity"]), self.assertRaises(model.SharedBulkError):
                model.validate_observation(
                    entry, bad, expected_batch=3, expected_thread_cpus=[1, 2, 3, 4])

    def test_registry_rows_validate_in_a_fresh_contained_copy(self):
        root = self.root
        registry_path = contained(root, root / "workloads.json")
        registry = json.loads((BENCH / "workloads.json").read_text())
        row_ids = {row["id"] for row in self.entries}
        registry["workloads"] = [row for row in registry["workloads"]
                                  if row["id"] not in row_ids]
        registry["workloads"].extend(self.entries)
        registry["selections"]["shared-bulk"] = {
            "status": "ready", "workloads": [row["id"] for row in self.entries]}
        contained(root, registry_path).write_text(json.dumps(registry))
        validated = workloads.load_registry(registry_path)
        self.assertEqual([row["id"] for row in workloads.selection("shared-bulk", validated)],
                         [row["id"] for row in self.entries])

    def test_complete_length_but_wrong_guest_write_fails_full_image_oracle(self):
        entry = next(row for row in self.entries
                     if row["id"] == "shared_bulk_copy_alias_w1_b512")
        row = wire(entry)
        raw = bytearray.fromhex(row["backings_hex"]["shared"])
        raw[65_536] ^= 1
        row["backings_hex"]["shared"] = raw.hex()
        with self.assertRaises(model.SharedBulkError):
            model.validate_observation(entry, row)


if __name__ == "__main__":
    unittest.main()
