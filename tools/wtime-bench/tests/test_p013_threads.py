import hashlib
import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(HERE))

import thread_workloads


class ThreadWorkloadTests(unittest.TestCase):
    def test_entries_cover_worker_scaling_and_bind_encoded_fixtures(self):
        entries = thread_workloads.entries()
        self.assertEqual(len(entries), 18)
        atomic = [entry for entry in entries if entry["id"].startswith("threads_atomic_add_")]
        self.assertEqual(len(atomic), 12)
        self.assertEqual(
            {(entry["thread_config"]["requested_workers"],
              entry["thread_config"]["iterations_per_worker"])
             for entry in atomic},
            {(workers, iterations)
             for workers in (1, 2, 4) for iterations in (32, 1024)},
        )
        for entry in entries:
            source = HERE / entry["source"]["path"]
            filename = source.name
            self.assertEqual(hashlib.sha256(source.read_bytes()).hexdigest(),
                             entry["source"]["sha256"])
            self.assertEqual(entry["source"]["wasm_sha256"],
                             thread_workloads.WASM_SHA256[filename])
            self.assertEqual(entry["source"]["encoding_status"], "encoded")
            self.assertEqual(entry["required_features"], ["threads"])
            self.assertEqual(entry["comparators"]["wasmtime"]["status"], "unsupported")
            self.assertFalse(entry["thread_config"]["setup_timed"])
            self.assertFalse(entry["thread_config"]["teardown_timed"])

    def test_atomic_oracles_check_shared_and_distinct_memory_totals(self):
        same = thread_workloads.oracle("threads_atomic_add_same_w2_n32", 2, 32)
        distinct = thread_workloads.oracle("threads_atomic_add_distinct_w4_n1024", 4, 1024)
        # These checks are direct arithmetic expectations, independent of fixture loops.
        self.assertEqual(same["final_memory_values"], [64])
        self.assertEqual(same["normalized_memory_total"], 64)
        self.assertEqual(same["worker_returns"], [32, 32])
        self.assertEqual(distinct["final_memory_values"], [1024, 1024, 1024, 1024])
        self.assertEqual(distinct["normalized_memory_total"], 4096)
        self.assertEqual(thread_workloads.oracle("threads_ordinary_rw_n32", 1, 32)[
            "normalized_memory_total"], 32)
        self.assertEqual(thread_workloads.oracle("threads_atomic_load_store_n1024", 1, 1024)[
            "final_memory_values"], [1024])
        self.assertEqual(thread_workloads.effective_workers(4, 2), 2)
        self.assertEqual(thread_workloads.effective_workers(4, 3), 3)
        self.assertEqual(thread_workloads.oracle("threads_atomic_add_same_w4_n32", 3, 32)[
            "normalized_memory_total"], 96)

    def test_wait_notify_probes_record_cleanup_and_exact_notification_counts(self):
        self.assertEqual(thread_workloads.wait_oracle("notified_wait"), 0)
        self.assertEqual(thread_workloads.wait_oracle("value_mismatch"), 1)
        self.assertEqual(thread_workloads.wait_oracle("zero_timeout"), 2)
        self.assertEqual(thread_workloads.wait_oracle("notify_count", waiters=2, notify_count=1), 1)
        self.assertEqual(thread_workloads.wait_oracle("notify_count", waiters=2, notify_count=8), 2)
        self.assertEqual(thread_workloads.wait_oracle("timeout_cleanup"), {
            "wait_result": 2, "remaining_waiters": 0, "notify_after_cleanup": 0,
        })
        latency = next(entry for entry in thread_workloads.entries()
                       if entry["id"] == "threads_wait_notify_latency")
        protocol = latency["measurement_protocol"]
        self.assertEqual(protocol["kind"], "individual_latency_samples")
        self.assertTrue(protocol["waiter_registration_must_be_observed_before_timer"])
        self.assertTrue(protocol["batched_mean_is_not_a_latency_sample"])
        self.assertGreaterEqual(len(latency["correctness_probes"]), 6)

    def test_latency_summary_uses_individual_samples_and_withholds_small_sample_p99(self):
        observations = [
            {"sample_kind": "one_wait_notify_round_trip", "registration_observed": True,
             "outcome": "notified", "notify_to_return_ns": duration}
            for duration in range(1, 101)
        ]
        summary = thread_workloads.summarize_latency(observations)
        self.assertEqual(summary["sample_count"], 100)
        self.assertEqual(summary["p50_ns"], 50)
        self.assertEqual(summary["p99_ns"], 99)
        self.assertEqual(len(summary["samples_ns"]), 100)
        small = thread_workloads.summarize_latency(observations[:32])
        self.assertIsNone(small["p99_ns"])
        self.assertEqual(small["p99_status"], "insufficient_individual_samples")
        with self.assertRaisesRegex(ValueError, "one wait/notify round trip"):
            thread_workloads.summarize_latency([12])
        bad = [dict(observations[0], registration_observed=False)]
        with self.assertRaisesRegex(ValueError, "registered waiter"):
            thread_workloads.summarize_latency(bad)

    def test_store_buffering_oracle_forbids_sc_outcome_zero_zero(self):
        entry = next(item for item in thread_workloads.entries()
                     if item["id"] == "threads_sc_store_buffering")
        self.assertEqual(entry["expected_result"]["forbidden_outcome"], [0, 0])
        self.assertTrue(entry["worker_protocol"]["reset_and_barrier_timed"] is False)
        source = (HERE / "fixtures" / "threads" / "sc_store_buffering.wat").read_text()
        self.assertIn('"store_x_load_y"', source)
        self.assertIn('"store_y_load_x"', source)
        self.assertIn("i32.atomic.store 0", source)
        self.assertIn("i32.atomic.load 1", source)


if __name__ == "__main__":
    unittest.main()
