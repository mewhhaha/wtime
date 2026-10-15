"""Contained pure protocol and registry tests for P077 wait-queue cohorts."""
from __future__ import annotations

import hashlib
import importlib.util
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

spec = importlib.util.spec_from_file_location("wait_queue_workloads", BENCH / "wait_queue_workloads.py")
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
    waiters = entry["params"][0]["baseline"]
    addresses = entry["params"][1]["baseline"]
    return {
        "schema": model.SCHEMA,
        "schema_version": model.SCHEMA_VERSION,
        "case": entry["id"],
        "mode": model.MODE,
        "profile": model.PROFILE,
        "stage": "steady_state_group",
        "waiters": waiters,
        "addresses": addresses,
        "batch": batch,
        "timeout_ns": model.WAIT_TIMEOUT_NS,
        "instrumented": False,
        "report_complete": True,
        "every_group_full_memory_checked": True,
        "elapsed_ns": 80_000 * batch,
        "calls_checked": batch * (waiters + 1 + addresses),
        "worker_returns": [0] * (batch * waiters),
        "notify_totals": [waiters] * batch,
        "notify_attempts": [waiters + 2] * batch,
        "cleanup_counts": [0] * (batch * addresses),
        "pre_wait_ready_counts": [waiters] * batch,
        "worker_affinity": [[1, 2, 3, 4] for _ in range(waiters)],
        "readiness_evidence": model.READINESS_EVIDENCE,
        "timing_boundary": model.TIMING_BOUNDARY,
        "backings_hex": {name: value.hex() for name, value in model.expected(entry).items()},
    }


class WaitQueueWorkloadTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="p077-wait-queue-")
        self.root = Path(self.temp.name).resolve()
        self.addCleanup(self.temp.cleanup)
        self.entries = model.entries()
        self.entry = next(row for row in self.entries if row["id"] == "wait_queue_cohort_w32_a8")

    def test_rows_bind_six_waiter_and_fanout_shapes_to_real_fixture_hashes(self):
        self.assertEqual(len(self.entries), 6)
        self.assertEqual({(row["params"][0]["baseline"], row["params"][1]["baseline"])
                          for row in self.entries}, set(model.WAITERS_ADDRESSES))
        source_path = BENCH / model.WAT_PATH
        wasm_path = source_path.with_suffix(".wasm")
        self.assertEqual(hashlib.sha256(source_path.read_bytes()).hexdigest(), model.WAT_SHA256)
        self.assertEqual(hashlib.sha256(wasm_path.read_bytes()).hexdigest(), model.WASM_SHA256)
        self.assertTrue(all(row["source"]["wasm_sha256"] == model.WASM_SHA256
                            for row in self.entries))

    def test_expected_image_is_full_zero_page_with_only_acknowledgement_word(self):
        image = model.expected(self.entry)["shared"]
        self.assertIsInstance(image, bytes)
        self.assertEqual(len(image), model.MEMORY_BYTES)
        self.assertEqual(image[:model.READY_OFFSET], bytes(model.READY_OFFSET))
        self.assertEqual(image[model.READY_OFFSET:model.READY_OFFSET + 4], (32).to_bytes(4, "little"))
        self.assertEqual(image[model.READY_OFFSET + 4:], bytes(model.MEMORY_BYTES - model.READY_OFFSET - 4))

    def test_valid_batch_normalizes_whole_cohort_cost_and_full_memory_proof(self):
        row = wire(self.entry, batch=3)
        normalized = model.validate_observation(
            self.entry, row, expected_batch=3, expected_thread_cpus=[1, 2, 3, 4])
        self.assertEqual(normalized["ns_per_operation"], 80_000)
        self.assertEqual(normalized["result"], model.oracle(self.entry["id"], 32))
        self.assertEqual(normalized["memory_proof"]["full_memory_bytes_checked"], 65_536)
        self.assertEqual(normalized["raw_wire_observation"], row)

    def test_zero_guest_memory_image_is_rejected_even_when_counts_claim_success(self):
        row = wire(self.entry)
        row["backings_hex"]["shared"] = bytes(model.MEMORY_BYTES).hex()
        with self.assertRaises(model.WaitQueueError):
            model.validate_observation(self.entry, row)

    def test_missing_or_nonzero_waiter_results_are_rejected(self):
        row = wire(self.entry)
        row["worker_returns"].pop()
        with self.assertRaises(model.WaitQueueError):
            model.validate_observation(self.entry, row)
        row = wire(self.entry)
        row["worker_returns"][0] = 1
        with self.assertRaises(model.WaitQueueError):
            model.validate_observation(self.entry, row)

    def test_notify_attempts_counts_cleanup_and_call_accounting_are_bounded(self):
        for key, value in (("notify_attempts", [model.MAX_NOTIFY_ATTEMPTS + 1]),
                           ("notify_totals", [31]), ("cleanup_counts", [0] * 7),
                           ("calls_checked", 1)):
            row = wire(self.entry)
            row[key] = value
            with self.subTest(key=key), self.assertRaises(model.WaitQueueError):
                model.validate_observation(self.entry, row)

    def test_protocol_flags_types_and_exact_fields_are_strict(self):
        for key, value in (("timeout_ns", True), ("instrumented", True),
                           ("report_complete", False), ("timing_boundary", "short")):
            row = wire(self.entry)
            row[key] = value
            with self.subTest(key=key), self.assertRaises(model.WaitQueueError):
                model.validate_observation(self.entry, row)
        row = wire(self.entry)
        row["unexpected"] = 1
        with self.assertRaises(model.WaitQueueError):
            model.validate_observation(self.entry, row)

    def test_affinity_batch_and_readiness_claim_are_bound(self):
        row = wire(self.entry, batch=2)
        with self.assertRaises(model.WaitQueueError):
            model.validate_observation(self.entry, row, expected_batch=3)
        row = wire(self.entry)
        row["worker_affinity"][0] = [1, 2]
        with self.assertRaises(model.WaitQueueError):
            model.validate_observation(self.entry, row, expected_thread_cpus=[1, 2, 3, 4])
        row = wire(self.entry)
        row["readiness_evidence"] = "worker was queued before timer"
        with self.assertRaises(model.WaitQueueError):
            model.validate_observation(self.entry, row)

    def test_registry_copy_and_all_outputs_are_contained_in_fresh_temp_root(self):
        registry_path = contained(self.root, self.root / "registry" / "workloads.json")
        registry_path.parent.mkdir(parents=True, exist_ok=True)
        actual = workloads.load_registry(BENCH / "workloads.json")
        selected = workloads.selection("wait-queues", actual)
        synthetic = model.entries()
        self.assertEqual([entry["id"] for entry in selected],
                         [entry["id"] for entry in synthetic])
        self.assertEqual(selected, synthetic)
        registry = json.loads((BENCH / "workloads.json").read_text(encoding="utf-8"))
        ids = {entry["id"] for entry in synthetic}
        registry["workloads"] = [entry for entry in registry["workloads"] if entry["id"] not in ids]
        registry["workloads"].extend(synthetic)
        registry["selections"]["wait-queues"] = {
            "status": "ready", "workloads": [entry["id"] for entry in synthetic]
        }
        fixture = contained(self.root, self.root / "fixtures" / "threads" / "wait_queue_cohort.wat")
        fixture.parent.mkdir(parents=True, exist_ok=True)
        fixture.write_bytes((BENCH / model.WAT_PATH).read_bytes())
        wasm = contained(self.root, fixture.with_suffix(".wasm"))
        wasm.write_bytes((BENCH / model.WAT_PATH).with_suffix(".wasm").read_bytes())
        registry_path.write_text(json.dumps(registry), encoding="utf-8")
        validated = workloads.load_registry(registry_path)
        self.assertEqual(len(workloads.selection("wait-queues", validated)), 6)
        self.assertTrue(registry_path.resolve().is_relative_to(self.root))


if __name__ == "__main__":
    unittest.main()
