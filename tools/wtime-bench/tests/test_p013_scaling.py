"""Fail-closed unit tests for the staged P013 runner; intentionally not executed here."""
from __future__ import annotations

import importlib.util
from pathlib import Path
import unittest


import sys
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import thread_scaling as runner


class Model:
    @staticmethod
    def effective_workers(requested, quota):
        return min(requested, quota)

    @staticmethod
    def oracle(workload, workers, iterations):
        if workload == "threads_sc_store_buffering": return {"forbidden_outcome": [0, 0]}
        return {"worker_returns": [iterations] * workers,
                "final_memory_values": [workers * iterations]}

    @staticmethod
    def wait_oracle(probe):
        return {"timeout_cleanup": {"wait_result": 2, "remaining_waiters": 0,
                                    "notify_after_cleanup": 0},
                "value_mismatch": 1, "zero_timeout": 2,
                "cancel_cleanup": {"outcome": "cancelled", "remaining_waiters": 0,
                                   "notify_after_cleanup": 0}}[probe]


class ThreadScalingValidationTests(unittest.TestCase):
    def test_atomic_requires_complete_21_trials_and_oracle_outputs(self):
        entry = {"id": "threads_atomic_add_same_w2_n32",
                 "thread_config": {"requested_workers": 2, "iterations_per_worker": 32}}
        result = {"case": entry["id"], "requested_workers": 2, "effective_workers": 2,
                  "worker_quota": 2, "iterations_per_worker": 32,
                  "timing_boundary": "start_barrier_to_last_worker_result_receive; setup/readback/join excluded",
                  "trials": 21, "trial_seeds": list(range(1, 22)),
                  "trial_wall_ns": [1] * 21, "trial_worker_returns": [[32, 32]] * 21,
                  "trial_final_memory_values": [[64]] * 21, "total_operations_per_trial": 64}
        self.assertEqual(runner._validate_atomic(entry, result, Model)["effective_workers"], 2)
        result["trial_worker_returns"] = result["trial_worker_returns"][:-1]
        with self.assertRaises(runner.ThreadMeasurementError):
            runner._validate_atomic(entry, result, Model)

    def test_sc_litmus_rejects_forbidden_outcome_and_missing_trials(self):
        entry = {"id": "threads_sc_store_buffering"}
        row = {"case": entry["id"], "trials": 1024, "memories_per_trial": 2,
               "reset_per_trial": True, "trial_seeds": list(range(1, 1025)),
               "trial_wall_ns": [1] * 1024, "trial_results": [[1, 0]] * 1024,
               "forbidden_result": [0, 0]}
        self.assertEqual(len(runner._validate_sc(entry, row, Model)["trial_results"]), 1024)
        row["trial_results"][7] = [0, 0]
        with self.assertRaises(runner.ThreadMeasurementError):
            runner._validate_sc(entry, row, Model)
        row["trial_results"] = [[1, 0]] * 1023
        with self.assertRaises(runner.ThreadMeasurementError):
            runner._validate_sc(entry, row, Model)

    def test_cleanup_requires_actual_timeout_and_released_registration(self):
        timeout = 10_000_000
        row = {"schema_version": 1, "status": "ok", "case": "threads_wait_cleanup",
               "timeout_ns": timeout, "timeout_elapsed_ns": timeout,
               "timeout_result": 2, "value_mismatch_result": 1, "zero_timeout_result": 2,
               "timeout_notify_after": 0, "cancellation_error_kind": "ResourceLimit",
               "cancellation_message": "execution cancelled during atomic wait", "cancellation_notify_after": 0,
               "registration_observer_available": False}
        self.assertEqual(runner._validate_wait_cleanup(row, timeout, Model)["timeout_elapsed_ns"], timeout)
        for key, bad_value in (("timeout_elapsed_ns", timeout - 1),
                               ("cancellation_notify_after", 1),
                               ("value_mismatch_result", 0)):
            broken = dict(row, **{key: bad_value})
            with self.subTest(key=key), self.assertRaises(runner.ThreadMeasurementError):
                runner._validate_wait_cleanup(broken, timeout, Model)

    def test_protocol_must_prove_decoded_uninstrumented_execution(self):
        valid = {"schema_version": 2, "engine": "wtime", "modes": ["decoded"],
                 "instrumented": False}
        self.assertEqual(runner.validate_protocol(valid), valid)
        for key, value in (("instrumented", True), ("modes", ["portable"]),
                           ("schema_version", 1)):
            with self.subTest(key=key), self.assertRaises(runner.ThreadMeasurementError):
                runner.validate_protocol(dict(valid, **{key: value}))


if __name__ == "__main__":
    unittest.main()
