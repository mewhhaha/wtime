"""Shared-memory thread fixtures, workload contracts, and independent oracles."""

from __future__ import annotations

import hashlib
import math
import subprocess
from pathlib import Path
from typing import Any

HERE = Path(__file__).resolve().parent
FIXTURE_DIR = HERE / "fixtures" / "threads"
WORKERS = (1, 2, 4)
ITERATIONS = (32, 1024)
STAGES = ["steady_state_group"]
RESET_POLICY = "fresh_instance_per_invocation"
FIXTURES = {
    "atomic_add": "atomic_add.wat",
    "ordinary_rw": "ordinary_rw.wat",
    "atomic_load_store": "atomic_load_store.wat",
    "wait_notify": "wait_notify.wat",
    "sc_store_buffering": "sc_store_buffering.wat",
}
WASM_SHA256 = {
    "atomic_add.wat": "46b8bdb39ef9b69deda41fa91c18f3cc4dd7ee7f58ee20dc6f1824d22a558029",
    "atomic_load_store.wat": "85c25183d6ca23da20bbce7ae10b89acfb90499f413fa891151e558e7f4975ce",
    "ordinary_rw.wat": "2a0e01aaf458a30970c52b18855a27a508f6bac0f87dcd9c961a3246af58abdf",
    "sc_store_buffering.wat": "d28965e8c9f3a157e7e3a55f80680e1542654699f3a49016a2b3532f404c4d22",
    "wait_notify.wat": "6a307ae5db566a43b1c3a8753e361f0aef14304ed7e56144f029fa5d6c268e06",
}


def effective_workers(requested: int, quota: int) -> int:
    """Cap requested workers to the measured environment's available quota."""
    if type(requested) is not int or requested not in WORKERS:
        raise ValueError("requested workers must be one of 1, 2, or 4")
    if type(quota) is not int or quota < 1:
        raise ValueError("worker quota must be a positive integer")
    return min(requested, quota)


def oracle(workload_id: str, workers: int, iterations: int) -> dict[str, Any]:
    """Return normalized per-memory state and per-worker results independently."""
    if type(workers) is not int or not 1 <= workers <= max(WORKERS):
        raise ValueError("effective workers must be an integer in 1..4")
    if type(iterations) is not int or iterations not in ITERATIONS:
        raise ValueError("iterations must be one of 32 or 1024")
    if workload_id == "threads_atomic_add_same" or workload_id.startswith("threads_atomic_add_same_w"):
        values = [workers * iterations]
        return {
            "worker_returns": [iterations] * workers,
            "final_memory_values": values,
            "normalized_memory_total": sum(values),
        }
    if workload_id == "threads_atomic_add_distinct" or workload_id.startswith("threads_atomic_add_distinct_w"):
        values = [iterations] * workers
        return {
            "worker_returns": [iterations] * workers,
            "final_memory_values": values,
            "normalized_memory_total": sum(values),
        }
    if (workload_id == "threads_ordinary_rw" or workload_id.startswith("threads_ordinary_rw_n")
            or workload_id == "threads_atomic_load_store"
            or workload_id.startswith("threads_atomic_load_store_n")):
        if workers != 1:
            raise ValueError(f"{workload_id} is an uncontended single-worker control")
        return {
            "worker_returns": [iterations],
            "final_memory_values": [iterations],
            "normalized_memory_total": iterations,
        }
    if workload_id.startswith("threads_sc_store_buffering"):
        if workers != 2:
            raise ValueError("store-buffering requires exactly two workers")
        if iterations != 1024:
            raise ValueError("store-buffering uses 1024 separately reset trials")
        return {
            "trials": iterations,
            "worker_returns_per_trial": "two i32 values, each in {0,1}",
            "forbidden_outcome": [0, 0],
            "memory_reset_per_trial": True,
        }
    raise ValueError(f"unknown thread workload id: {workload_id}")


def wait_oracle(probe: str, *, waiters: int = 1, notify_count: int = 1) -> Any:
    """Independent expected values for wait/notify and cleanup correctness probes."""
    if probe == "notified_wait":
        return 0
    if probe == "value_mismatch":
        return 1
    if probe == "zero_timeout":
        return 2
    if probe == "timeout_cleanup":
        return {"wait_result": 2, "remaining_waiters": 0, "notify_after_cleanup": 0}
    if probe == "cancel_cleanup":
        return {"outcome": "cancelled", "remaining_waiters": 0, "notify_after_cleanup": 0}
    if probe == "notify_count":
        if any(type(x) is not int or x < 0 for x in (waiters, notify_count)):
            raise ValueError("waiters and notify_count must be nonnegative integers")
        return min(waiters, notify_count)
    if probe == "notify_before_wait":
        return {"notify_result": 0, "subsequent_zero_timeout_wait_result": 2}
    raise ValueError(f"unknown wait/notify probe: {probe}")


def summarize_latency(samples: list[dict[str, Any]], *, minimum_samples: int = 100) -> dict[str, Any]:
    """Summarize individually recorded notify-to-return samples, never batches."""
    if not isinstance(samples, list) or not samples:
        raise ValueError("latency samples must be a nonempty list of individual observations")
    if type(minimum_samples) is not int or minimum_samples < 1:
        raise ValueError("minimum_samples must be a positive integer")
    values: list[int] = []
    for sample in samples:
        if not isinstance(sample, dict) or sample.get("sample_kind") != "one_wait_notify_round_trip":
            raise ValueError("each latency observation must describe one wait/notify round trip")
        if sample.get("registration_observed") is not True or sample.get("outcome") != "notified":
            raise ValueError("latency observation needs a registered waiter and notify outcome")
        elapsed = sample.get("notify_to_return_ns")
        if type(elapsed) is not int or elapsed < 0:
            raise ValueError("notify_to_return_ns must be a nonnegative integer")
        values.append(elapsed)
    ordered = sorted(values)

    def nearest_rank(percent: int) -> int:
        return ordered[max(0, math.ceil(percent * len(ordered) / 100) - 1)]

    enough = len(ordered) >= minimum_samples
    return {
        "sample_kind": "individual_wait_notify_round_trips",
        "sample_count": len(ordered),
        "p50_ns": nearest_rank(50),
        "p99_ns": nearest_rank(99) if enough else None,
        "p99_status": "reported" if enough else "insufficient_individual_samples",
        "samples_ns": values,
    }


def entries() -> list[dict[str, Any]]:
    """Return schema-v2 thread entries with host-driver and synchronization contracts."""
    result: list[dict[str, Any]] = []
    for contention in ("same", "distinct"):
        for workers in WORKERS:
            for iterations in ITERATIONS:
                wid = f"threads_atomic_add_{contention}_w{workers}_n{iterations}"
                args = oracle(f"threads_atomic_add_{contention}", workers, iterations)
                result.append(_entry(
                    wid, "atomic_add.wat", "atomic_rmw_add",
                    workers=workers, iterations=iterations,
                    backing="one_common_shared_memory" if contention == "same"
                    else "one_shared_memory_per_worker",
                    operations_per_worker=iterations,
                    work_units=workers * iterations,
                    oracle_name="atomic_add_same" if contention == "same"
                    else "atomic_add_distinct",
                    oracle_arguments={"workers": workers, "iterations": iterations},
                    expected=args,
                    contention="contended" if workers > 1 and contention == "same"
                    else "uncontended_control" if workers == 1 or contention == "distinct"
                    else "same_memory_contention",
                ))

    for workload, filename, unit in (
        ("threads_ordinary_rw", "ordinary_rw.wat", "ordinary_shared_load_store_pair"),
        ("threads_atomic_load_store", "atomic_load_store.wat", "atomic_load_store_pair"),
    ):
        for iterations in ITERATIONS:
            result.append(_entry(
                f"{workload}_n{iterations}", filename, unit,
                workers=1, iterations=iterations, backing="one_common_shared_memory",
                operations_per_worker=iterations, work_units=iterations,
                oracle_name=workload.removeprefix("threads_"),
                oracle_arguments={"workers": 1, "iterations": iterations},
                expected=oracle(workload, 1, iterations),
                contention="uncontended_control",
            ))

    result.append(_entry(
        "threads_wait_notify_latency", "wait_notify.wat", "individual_wait_notify_round_trip",
        workers=2, iterations=1, backing="one_common_shared_memory",
        operations_per_worker=1, work_units=1,
        oracle_name="wait_notify", oracle_arguments={"probe": "notified_wait"},
        expected=wait_oracle("notified_wait"), contention="synchronization_latency",
        measurement_status="host_setup_required",
        measurement_protocol={
            "kind": "individual_latency_samples",
            "target_samples": 1024,
            "sample_kind": "one_wait_notify_round_trip",
            "timed_interval": "notify_call_start_to_waiter_return",
            "waiter_registration_must_be_observed_before_timer": True,
            "wait_timeout_ns": 1_000_000_000,
            "setup_and_teardown_timed": False,
            "p99_requires_individual_samples": 100,
            "batched_mean_is_not_a_latency_sample": True,
        },
        correctness_probes=[
            {"probe": "value_mismatch", "args": {"expected": 1, "timeout_ns": 1_000_000},
             "expected": wait_oracle("value_mismatch")},
            {"probe": "zero_timeout", "args": {"expected": 0, "timeout_ns": 0},
             "expected": wait_oracle("zero_timeout")},
            {"probe": "timeout_cleanup", "args": {"expected": 0, "timeout_ns": 1_000_000},
             "expected": wait_oracle("timeout_cleanup")},
            {"probe": "cancel_cleanup", "args": {"expected": 0, "timeout_ns": -1},
             "expected": wait_oracle("cancel_cleanup")},
            {"probe": "notify_count", "waiters": 2, "notify_count": 1,
             "expected": wait_oracle("notify_count", waiters=2, notify_count=1)},
            {"probe": "notify_before_wait", "expected": wait_oracle("notify_before_wait")},
        ],
    ))
    sc_expected = oracle("threads_sc_store_buffering", 2, 1024)
    result.append(_entry(
        "threads_sc_store_buffering", "sc_store_buffering.wat", "sc_store_then_load_pair",
        workers=2, iterations=1024, backing="two_distinct_shared_memories",
        operations_per_worker=1, work_units=1024,
        oracle_name="sc_store_buffering", oracle_arguments={"workers": 2, "iterations": 1024},
        expected=sc_expected, contention="cross_memory_sequential_consistency_control",
        measurement_status="host_setup_required",
        worker_protocol={
            "exports": ["store_x_load_y", "store_y_load_x"],
            "memory_imports": {"x": "first shared memory", "y": "second shared memory"},
            "per_trial": "zero both memories, wait for two-worker barrier, release both workers, record both results",
            "reset_and_barrier_timed": False,
            "forbidden_result": [0, 0],
        },
    ))
    return result


def _entry(
    workload_id: str,
    filename: str,
    unit: str,
    *,
    workers: int,
    iterations: int,
    backing: str,
    operations_per_worker: int,
    work_units: int,
    oracle_name: str,
    oracle_arguments: dict[str, Any],
    expected: Any,
    contention: str,
    measurement_status: str = "host_setup_required",
    measurement_protocol: dict[str, Any] | None = None,
    correctness_probes: list[dict[str, Any]] | None = None,
    worker_protocol: dict[str, Any] | None = None,
) -> dict[str, Any]:
    source_path = FIXTURE_DIR / filename
    entry: dict[str, Any] = {
        "id": workload_id,
        "family": workload_id.rsplit("_w", 1)[0] if "_w" in workload_id else workload_id,
        "source": {
            "kind": "wat_file",
            "path": f"fixtures/threads/{filename}",
            "sha256": hashlib.sha256(source_path.read_bytes()).hexdigest(),
            "wasm_sha256": WASM_SHA256.get(filename, "0" * 64),
            "encoding_status": "encoded" if filename in WASM_SHA256 else "pending",
        },
        "params": [{"name": "iterations", "type": "i32", "min": iterations,
                    "max": iterations, "baseline": iterations}],
        "result": {"type": "i32"},
        "oracle": {"kind": "python", "module": "thread_workloads.py",
                   "function": "wait_oracle" if oracle_name == "wait_notify" else "oracle",
                   "independent": True},
        "oracle_arguments": oracle_arguments,
        "expected_result": expected,
        "expected_result_worker_policy": "recompute using effective_workers after quota capping",
        "required_features": ["threads"],
        "work_units": {"count": work_units, "unit": unit,
                       "per_worker": operations_per_worker},
        "reset_policy": RESET_POLICY,
        "stages": STAGES.copy(),
        "comparators": {
            "wasmtime": {
                "status": "unsupported",
                "reason": "The pinned default Wasmtime oracle is configured without threads/shared-memory support; capability preflight must emit no timing row.",
            }
        },
        "measurement_status": measurement_status,
        "thread_config": {
            "requested_workers": workers,
            "worker_quota_cap": True,
            "effective_workers": "min(requested_workers, available_parallelism)",
            "iterations_per_worker": iterations,
            "shared_backing": backing,
            "start_barrier": "all workers signal ready; coordinator releases one start barrier",
            "setup_timed": False,
            "teardown_timed": False,
            "memory_reset": "zero initialized i32 at byte offset 0 before each trial",
            "contention_role": contention,
        },
        "trial_protocol": {
            "independent_trials": 21,
            "seed_source": "recorded per-trial seed list derived from the user-supplied base seed",
            "seed_use": "deterministic worker launch skew only; operation counts and expected values do not depend on seeds",
            "raw_trial_results_retained": True,
        },
        "expected_memory": {
            "normalization": "little-endian unsigned i32 words",
            "total_field": "normalized_memory_total",
            "expected": expected.get("normalized_memory_total") if isinstance(expected, dict) else None,
        },
        "timing_rule": "Report coordinator wall time for the joined worker group and total completed operations; never sum overlapping worker durations.",
    }
    if measurement_protocol is not None:
        entry["measurement_protocol"] = measurement_protocol
    if correctness_probes is not None:
        entry["correctness_probes"] = correctness_probes
    if worker_protocol is not None:
        entry["worker_protocol"] = worker_protocol
    return entry


def encode_fixtures(encoder: str | Path, targetdir: str | Path) -> dict[str, str]:
    """Encode WAT fixtures using an already-built encoder; never builds it."""
    target = Path(targetdir)
    target.mkdir(parents=True, exist_ok=True)
    hashes: dict[str, str] = {}
    for filename in FIXTURES.values():
        source = FIXTURE_DIR / filename
        output = target / f"{source.stem}.wasm"
        subprocess.run([str(encoder), "encode", str(source), "-o", str(output)],
                       check=True, cwd=HERE.parent.parent)
        hashes[filename] = hashlib.sha256(output.read_bytes()).hexdigest()
    return hashes
