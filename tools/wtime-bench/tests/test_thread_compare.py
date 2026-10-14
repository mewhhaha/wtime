import copy
import sys
import unittest
from unittest import mock
from pathlib import Path

TOOLS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(TOOLS))
import thread_compare as adapter
import thread_scaling
import thread_workloads
import focused_compare


def row(wid):
    return next(e for e in thread_workloads.entries() if e["id"] == wid)


def xorshift64(value):
    value ^= (value << 13) & ((1 << 64) - 1)
    value ^= value >> 7
    value ^= (value << 17) & ((1 << 64) - 1)
    return value & ((1 << 64) - 1)


def trial_seeds(seed, count):
    values = []
    for _ in range(count):
        seed = xorshift64(seed)
        values.append(seed)
    return values


def atomic_raw(entry, seed=1, quota=8):
    config = entry["thread_config"]
    workers = config["requested_workers"]
    iterations = config["iterations_per_worker"]
    expected = thread_workloads.oracle(entry["id"], workers, iterations)
    return {
        "schema_version": 1, "status": "ok",
        "case": entry["id"], "requested_workers": workers, "effective_workers": workers,
        "worker_quota": quota, "iterations_per_worker": iterations,
        "timing_boundary": "start_barrier_to_last_worker_result_receive; setup/readback/join excluded",
        "trials": 21, "trial_seeds": trial_seeds(seed, 21),
        "trial_wall_ns": [210000 + i for i in range(21)],
        "trial_worker_returns": [expected["worker_returns"] for _ in range(21)],
        "trial_final_memory_values": [expected["final_memory_values"] for _ in range(21)],
        "total_operations_per_trial": workers * iterations,
    }


def sc_raw(entry, seed=1):
    return {"schema_version": 1, "status": "ok", "case": entry["id"], "trials": 1024, "memories_per_trial": 2,
            "reset_per_trial": True, "trial_seeds": trial_seeds(seed, 1024),
            "trial_wall_ns": [30000] * 1024, "trial_results": [[0, 1]] * 1024,
            "forbidden_result": [0, 0]}


class ThreadAdapterTests(unittest.TestCase):
    def test_capability_requires_explicit_thread_driver(self):
        p = {"schema_version": 2, "engine": "wtime", "instrumented": False,
             "modes": ["decoded"], "thread_driver": True}
        self.assertIs(adapter.validate_capability(p), p)
        p["thread_driver"] = False
        with self.assertRaises(thread_scaling.ThreadMeasurementError):
            adapter.validate_capability(p)

    def test_thread_cpu_mask_validation(self):
        self.assertEqual(focused_compare.parse_thread_cpus("1,2", [1, 2, 4], required=True), [1, 2])
        for raw in (None, "1", "1,1", "1,3", "x,2"):
            with self.subTest(raw=raw), self.assertRaises(ValueError):
                focused_compare.parse_thread_cpus(raw, [1, 2, 4], required=True)

    def test_thread_process_sets_mask_and_preserves_compare_errors(self):
        completed = type("Completed", (), {"returncode": 0, "stdout": '{"status":"ok"}', "stderr": ""})()
        with mock.patch.object(focused_compare.subprocess, "run", return_value=completed) as run, \
             mock.patch.object(focused_compare.os, "sched_setaffinity") as set_affinity:
            self.assertEqual(focused_compare.thread_process("candidate", ["--x", 3], [1, 2]), {"status": "ok"})
            command, kwargs = run.call_args
            self.assertEqual(command[0], ["candidate", "--x", "3"])
            kwargs["preexec_fn"]()
            set_affinity.assert_called_once_with(0, {1, 2})
        failed = type("Completed", (), {"returncode": 4, "stdout": "", "stderr": "bad"})()
        with mock.patch.object(focused_compare.subprocess, "run", return_value=failed):
            with self.assertRaisesRegex(focused_compare.compare.ComparisonError, "process failed"):
                focused_compare.thread_process("candidate", [], [1, 2])
        for stdout, message in (("not-json", "missing or malformed"), ("[]", "response must be an object")):
            malformed = type("Completed", (), {"returncode": 0, "stdout": stdout, "stderr": ""})()
            with mock.patch.object(focused_compare.subprocess, "run", return_value=malformed):
                with self.assertRaisesRegex(focused_compare.compare.ComparisonError, message):
                    focused_compare.thread_process("candidate", [], [1, 2], timeout=17)

    def test_atomic_full_trials_normalize_and_revalidate(self):
        e = row("threads_ordinary_rw_n32")
        obs = adapter.retain_groups(e, [{"raw": atomic_raw(e), "cleanup": None}], [1, 2])
        adapter.revalidate(e, obs, "decoded", "steady_state_group", [1, 2])
        self.assertEqual(obs["batch"], 1)
        self.assertEqual(obs["thread_groups"][0]["proof"]["proof"]["trial_seeds"], trial_seeds(1, 21))
        bad = copy.deepcopy(obs)
        bad["thread_groups"][0]["raw"]["trial_final_memory_values"][0] = [31]
        with self.assertRaises(thread_scaling.ThreadMeasurementError):
            adapter.revalidate(e, bad, "decoded", "steady_state_group", [1, 2])

    def test_legacy_schema_and_status_are_required_before_timing_proofs(self):
        e = row("threads_ordinary_rw_n32")
        valid = atomic_raw(e)
        for malformed in (None, [], {**valid, "schema_version": 2}, {**valid, "status": "failed"}):
            with self.subTest(raw=type(malformed).__name__), self.assertRaises(thread_scaling.ThreadMeasurementError):
                adapter.retain_groups(e, [{"raw": malformed, "cleanup": None}], [1, 2])

    def test_observe_routes_existing_worker_cli(self):
        e = row("threads_ordinary_rw_n1024")
        seen = []
        def process(exe, argv, cpu):
            seen.append((exe, argv, cpu))
            return atomic_raw(e, seed=int(argv[-1]))
        obs = adapter.observe("candidate", "fixture.wasm", e, seed=991, process=process,
                              cpu=[1, 2], thread_cpu_affinity=[1, 2], batch=2)
        self.assertEqual(len(seen), 2)
        self.assertEqual([call[1][-1] for call in seen], ["991", str(adapter._group_seed(991, 1, 21))])
        adapter.revalidate(e, obs, "decoded", "steady_state_group", [1, 2])

    def test_focused_producer_dispatches_batched_thread_profile(self):
        e = row("threads_ordinary_rw_n32")
        seen = []
        def process(exe, argv, cpu):
            seen.append((argv, cpu))
            return atomic_raw(e, seed=int(argv[-1]))
        with mock.patch.object(focused_compare, "thread_process", side_effect=process):
            obs = focused_compare.observe("candidate", e, "fixture.wasm", "steady_state_group", 2,
                                          "decoded", 0, Path("unused-output"), seed=37, thread_cpus=[1, 2])
        self.assertEqual(focused_compare.profile(e), "threads")
        self.assertEqual(len(seen), 2)
        self.assertEqual([item[1] for item in seen], [[1, 2], [1, 2]])
        self.assertEqual(obs["batch"], 2)
        self.assertEqual(obs["settings"]["thread_cpu_affinity"], [1, 2])
        adapter.revalidate(e, obs, "decoded", "steady_state_group", [1, 2])

    def test_scalar_control_keeps_integer_cpu_pin(self):
        import scalar_workloads
        e = scalar_workloads.entries()[0]
        with mock.patch.object(focused_compare.compare, "scalar_observation", return_value={"mode": "decoded"}) as scalar:
            focused_compare.observe("candidate", e, "scalar.wasm", e["stages"][0], 1,
                                    "decoded", 3, Path("unused-output"))
        self.assertEqual(scalar.call_args.args[-1], 3)

    def test_sc_retains_all_reset_trials_and_rejects_forbidden_outcome(self):
        e = row("threads_sc_store_buffering")
        obs = adapter.retain_groups(e, [{"raw": sc_raw(e), "cleanup": None}], [1, 2])
        adapter.revalidate(e, obs, "decoded", "steady_state_group", [1, 2])
        self.assertEqual(len(obs["thread_groups"][0]["raw"]["trial_results"]), 1024)
        bad = copy.deepcopy(obs)
        bad["thread_groups"][0]["raw"]["trial_results"][987] = [0, 0]
        with self.assertRaises(thread_scaling.ThreadMeasurementError):
            adapter.revalidate(e, bad, "decoded", "steady_state_group", [1, 2])

    def test_batch_timing_proof_and_full_group_contract(self):
        e = row("threads_ordinary_rw_n32")
        groups = [{"raw": atomic_raw(e, seed=adapter._group_seed(1, i, 21)), "cleanup": None}
                  for i in range(3)]
        obs = adapter.retain_groups(e, groups, [1, 2])
        expected_elapsed = sum(sum(g["raw"]["trial_wall_ns"]) for g in groups)
        self.assertEqual(obs["batch"], 3)
        self.assertEqual(obs["elapsed_ns"], expected_elapsed)
        self.assertEqual(obs["ns_per_operation"], expected_elapsed / 3)
        self.assertEqual(obs["normalization"], "mean_sum_trial_intervals_per_complete_driver_group")
        self.assertEqual(len(obs["thread_legacy_metrics"]), 3)
        adapter.revalidate(e, obs, "decoded", "steady_state_group", [1, 2])
        for mutate in (
            lambda x: x["thread_groups"].pop(),
            lambda x: x.update(elapsed_ns=x["elapsed_ns"] + 1),
            lambda x: x.update(ns_per_operation=x["ns_per_operation"] + 1),
            lambda x: x.update(normalization="made_up"),
            lambda x: x.update(thread_legacy_metrics=[]),
        ):
            bad = copy.deepcopy(obs)
            mutate(bad)
            with self.assertRaises(thread_scaling.ThreadMeasurementError):
                adapter.revalidate(e, bad, "decoded", "steady_state_group", [1, 2])

    def test_batch_rejects_duplicate_seeds_quota_drift_and_cap(self):
        e = row("threads_ordinary_rw_n32")
        with self.assertRaises(thread_scaling.ThreadMeasurementError):
            adapter.retain_groups(e, [{"raw": atomic_raw(e, 1), "cleanup": None},
                                      {"raw": atomic_raw(e, 1), "cleanup": None}], [1, 2])
        with self.assertRaises(thread_scaling.ThreadMeasurementError):
            adapter.retain_groups(e, [{"raw": atomic_raw(e, 1, quota=8), "cleanup": None},
                                      {"raw": atomic_raw(e, adapter._group_seed(1, 1, 21), quota=4),
                                       "cleanup": None}], [1, 2])
        calls = []
        with self.assertRaises(ValueError):
            adapter.observe("candidate", "fixture.wasm", e, seed=1,
                            process=lambda *args: calls.append(args), cpu=[1, 2],
                            thread_cpu_affinity=[1, 2], batch=129)
        self.assertEqual(calls, [])

    def test_paired_candidate_gate_identity_requires_same_worker_quota(self):
        e = row("threads_ordinary_rw_n32")
        baseline = adapter.retain_groups(e, [{"raw": atomic_raw(e, quota=8), "cleanup": None}], [1, 2])
        candidate = adapter.retain_groups(e, [{"raw": atomic_raw(e, quota=4), "cleanup": None}], [1, 2])
        self.assertFalse(adapter.paired_worker_identities_match(baseline, candidate))
        candidate = adapter.retain_groups(e, [{"raw": atomic_raw(e, quota=8), "cleanup": None}], [1, 2])
        self.assertTrue(adapter.paired_worker_identities_match(baseline, candidate))

    def test_wait_timeout_contract_mismatch_fails_closed(self):
        e = row("threads_wait_notify_latency")
        # Registry currently declares 1s; old cleanup validator permits at most 100ms.
        declared = e["measurement_protocol"]["wait_timeout_ns"]
        self.assertEqual(declared, 1_000_000_000)
        calls = []
        with self.assertRaises(thread_scaling.ThreadMeasurementError):
            adapter.observe("candidate", "wait.wasm", e, seed=4,
                            process=lambda *args: calls.append(args), cpu=[1, 2],
                            thread_cpu_affinity=[1, 2], timeout_ns=declared)
        self.assertEqual(calls, [])


if __name__ == "__main__":
    unittest.main()
