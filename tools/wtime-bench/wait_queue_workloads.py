"""P077 fixed wait-queue cohort rows and independent full-memory validator."""
from __future__ import annotations

import hashlib
from functools import lru_cache
from typing import Any

SCHEMA = "wtime-wait-queue-cohort-v1"
SCHEMA_VERSION = 1
MODE = "decoded"
PROFILE = "threads"
WAITERS_ADDRESSES = ((2, 1), (32, 1), (8, 1), (2, 2), (32, 8), (8, 4))
MAX_BATCH = 4096
WAIT_TIMEOUT_NS = 5_000_000_000
MAX_NOTIFY_ATTEMPTS = 1_000_000
MEMORY_BYTES = 65_536
MEMORY_PAGES = 1
READY_OFFSET = 4096
WAT_PATH = "fixtures/threads/wait_queue_cohort.wat"
WAT_SHA256 = "699371f6bf7153748c80e0160c1b69a88046ad85f23967fae489eafa86de261a"
WASM_SHA256 = "162d57d39e470d40bff2ab5f7543946480bfe27dfd41d16115049bf8616b95f4"
READINESS_EVIDENCE = "guest acknowledgement before wait; queue registration is not observed before timing"
TIMING_BOUNDARY = "waiter gate release through all waiter and drain results; guest readiness and failed notify probes included; host setup/join/image/cleanup checks excluded"


class WaitQueueError(ValueError):
    """A wait-queue workload or observation violates its fixed contract."""


def _workload_id(waiters: int, addresses: int) -> str:
    return f"wait_queue_cohort_w{waiters}_a{addresses}"


def _fixed(workload_id: str) -> tuple[int, int]:
    for waiters, addresses in WAITERS_ADDRESSES:
        if workload_id == _workload_id(waiters, addresses):
            return waiters, addresses
    raise WaitQueueError("unknown fixed wait-queue workload")


def entries() -> list[dict[str, Any]]:
    """Return six immutable-size rows for targeted, held-out, and fanout cases."""
    return [{
        "schema_version": 2,
        "id": _workload_id(waiters, addresses),
        "family": "wait-queues",
        "source": {"kind": "wat_file", "path": WAT_PATH,
                   "sha256": WAT_SHA256, "wasm_sha256": WASM_SHA256,
                   "encoding_status": "encoded"},
        "params": [
            {"name": "waiters", "type": "i32", "min": waiters,
             "max": waiters, "baseline": waiters},
            {"name": "addresses", "type": "i32", "min": addresses,
             "max": addresses, "baseline": addresses},
        ],
        "result": {"type": "i32"},
        "oracle": {"kind": "semantic", "function": "oracle",
                   "module": "wait_queue_workloads.py", "independent": True,
                   "waiters": waiters, "addresses": addresses,
                   "memory_bytes": MEMORY_BYTES, "memory_pages": MEMORY_PAGES},
        "required_features": ["threads"],
        "profile": {"status": "ready",
                    "reason": "source-bound shared wait queue cohort with full-memory lifecycle oracle"},
        "work_units": {"unit": "complete shared-memory waiter cohort", "count": 1},
        "reset_policy": "fresh_instance_per_invocation",
        "stages": ["steady_state_group"],
        "comparators": {"wasmtime": {"status": "unsupported",
                       "reason": "Pinned comparator has no matching shared-memory waiter cohort driver."}},
        "driver": {"kind": "wait_queue_cohort", "profile": PROFILE,
                   "protocol": SCHEMA,
                   "argv": ["--wait-queue-cohort", "FILE", "CASE", "WAITERS",
                            "ADDRESSES", "STAGE", "BATCH", "MODE", "PROFILE"]},
        "wait_queue_cohort": {
            "waiters": waiters, "addresses": addresses,
            "timeout_ns": WAIT_TIMEOUT_NS, "memory_bytes": MEMORY_BYTES,
            "max_notify_attempts": MAX_NOTIFY_ATTEMPTS,
            "ready_flag_offset": READY_OFFSET,
            "readiness_evidence": READINESS_EVIDENCE,
            "timing_boundary": TIMING_BOUNDARY,
        },
    } for waiters, addresses in WAITERS_ADDRESSES]


@lru_cache(maxsize=None)
def _expected_image(waiters: int) -> bytes:
    image = bytearray(MEMORY_BYTES)
    image[READY_OFFSET:READY_OFFSET + 4] = waiters.to_bytes(4, "little")
    return bytes(image)


def expected(entry: dict[str, Any]) -> dict[str, bytes]:
    """Return an immutable byte-for-byte final shared-memory oracle."""
    workload_id = entry.get("id")
    waiters, addresses = _fixed(workload_id)
    params = entry.get("params")
    _need(isinstance(params, list) and len(params) == 2,
          "registry row must declare exactly two fixed parameters")
    values = {item.get("name"): item.get("baseline") for item in params
              if isinstance(item, dict)}
    _need(values == {"waiters": waiters, "addresses": addresses},
          "registry parameters differ from the fixed row identity")
    return {"shared": _expected_image(waiters)}


def oracle(workload_id: str, n: int) -> dict[str, Any]:
    """Return compact deterministic cohort expectations (n is fixed waiters)."""
    waiters, addresses = _fixed(workload_id)
    _need(type(n) is int and n == waiters,
          "waiter count differs from the fixed workload ID")
    image = _expected_image(waiters)
    return {
        "waiters": waiters,
        "addresses": addresses,
        "worker_returns": [0] * waiters,
        "notify_total": waiters,
        "cleanup_counts": [0] * addresses,
        "pre_wait_ready_counts": [waiters],
        "full_backing_sha256": {"shared": hashlib.sha256(image).hexdigest()},
        "full_memory_bytes_checked": MEMORY_BYTES,
        "full_memory_pages_checked": MEMORY_PAGES,
    }


def _need(ok: bool, message: str) -> None:
    if not ok:
        raise WaitQueueError(message)


def _int(value: Any) -> bool:
    return type(value) is int


def validate_observation(entry: dict[str, Any], observation: dict[str, Any], *,
                         expected_batch: int | None = None,
                         expected_thread_cpus: list[int] | None = None) -> dict[str, Any]:
    """Strictly validate all cohort outcomes and the final complete memory image."""
    _need(isinstance(entry, dict), "registry entry must be an object")
    workload_id = entry.get("id")
    params = entry.get("params")
    _need(isinstance(params, list) and len(params) == 2,
          "registry row must declare exactly two fixed parameters")
    values = {item.get("name"): item.get("baseline") for item in params
              if isinstance(item, dict)}
    waiters, addresses = _fixed(workload_id)
    _need(values == {"waiters": waiters, "addresses": addresses},
          "registry parameters differ from the fixed workload ID")
    expected_result = oracle(workload_id, waiters)
    expected_hex = _expected_image(waiters).hex()
    _need(isinstance(observation, dict), "wire observation must be an object")
    keys = {
        "schema", "schema_version", "case", "mode", "profile", "stage",
        "waiters", "addresses", "batch", "timeout_ns", "instrumented",
        "report_complete", "every_group_full_memory_checked", "elapsed_ns",
        "calls_checked", "worker_returns", "notify_totals", "notify_attempts",
        "cleanup_counts", "pre_wait_ready_counts", "worker_affinity",
        "readiness_evidence", "timing_boundary", "backings_hex",
    }
    _need(set(observation) == keys, "wire fields differ from the declared protocol")
    fixed_fields = {
        "schema": SCHEMA, "schema_version": SCHEMA_VERSION,
        "case": workload_id, "mode": MODE, "profile": PROFILE,
        "stage": "steady_state_group", "waiters": waiters,
        "addresses": addresses, "timeout_ns": WAIT_TIMEOUT_NS,
        "instrumented": False, "report_complete": True,
        "every_group_full_memory_checked": True,
        "readiness_evidence": READINESS_EVIDENCE,
        "timing_boundary": TIMING_BOUNDARY,
    }
    for key, value in fixed_fields.items():
        _need(type(observation.get(key)) is type(value) and observation[key] == value,
              f"wire {key} differs from the workload contract")

    batch = observation["batch"]
    _need(_int(batch) and 1 <= batch <= MAX_BATCH,
          f"wire batch must be an integer in 1..{MAX_BATCH}")
    if expected_batch is not None:
        _need(_int(expected_batch) and batch == expected_batch,
              "wire batch differs from the predeclared batch")
    elapsed_ns = observation["elapsed_ns"]
    _need(_int(elapsed_ns) and elapsed_ns > 0,
          "wire elapsed_ns must be a positive integer cohort-time sum")
    calls = observation["calls_checked"]
    expected_calls = batch * (waiters + 1 + addresses)
    _need(_int(calls) and calls == expected_calls,
          "checked call count differs from batch times waiter, drain, and cleanup calls")

    returns = observation["worker_returns"]
    _need(isinstance(returns, list) and len(returns) == batch * waiters
          and all(_int(value) and value == 0 for value in returns),
          "worker returns must contain one zero result per waiter in every group")
    totals = observation["notify_totals"]
    _need(isinstance(totals, list) and len(totals) == batch
          and all(_int(value) and value == waiters for value in totals),
          "each group must report exactly the requested successful notifications")
    attempts = observation["notify_attempts"]
    _need(isinstance(attempts, list) and len(attempts) == batch
          and all(_int(value) and waiters <= value <= MAX_NOTIFY_ATTEMPTS for value in attempts),
          "each group must report bounded notify attempts including failed probes")
    cleanup = observation["cleanup_counts"]
    _need(isinstance(cleanup, list) and len(cleanup) == batch * addresses
          and all(_int(value) and value == 0 for value in cleanup),
          "cleanup must find zero remaining waiters on every address")
    ready = observation["pre_wait_ready_counts"]
    _need(isinstance(ready, list) and len(ready) == batch
          and all(_int(value) and value == waiters for value in ready),
          "pre-wait acknowledgements must count every worker in every group")

    affinity = observation["worker_affinity"]
    _need(isinstance(affinity, list) and len(affinity) == waiters,
          "worker affinity must report one mask per waiter")
    for mask in affinity:
        _need(isinstance(mask, list) and bool(mask)
              and all(_int(cpu) and cpu >= 0 for cpu in mask)
              and mask == sorted(set(mask)),
              "worker affinity masks must be nonempty sorted unique CPU IDs")
    _need(all(mask == affinity[0] for mask in affinity),
          "workers reported different affinity masks")
    if expected_thread_cpus is not None:
        _need(isinstance(expected_thread_cpus, list)
              and bool(expected_thread_cpus)
              and all(_int(cpu) and cpu >= 0 for cpu in expected_thread_cpus)
              and expected_thread_cpus == sorted(set(expected_thread_cpus))
              and all(mask == expected_thread_cpus for mask in affinity),
              "worker affinity differs from the predeclared CPU mask")

    images = observation["backings_hex"]
    _need(isinstance(images, dict) and set(images) == {"shared"},
          "wire shared-memory backing set differs")
    encoded = images["shared"]
    _need(type(encoded) is str and len(encoded) == MEMORY_BYTES * 2
          and all(char in "0123456789abcdef" for char in encoded),
          "wire shared backing is not a complete lowercase full-memory image")
    _need(encoded == expected_hex,
          "wire shared backing differs from the independent full-memory oracle")
    digest = hashlib.sha256(bytes.fromhex(encoded)).hexdigest()
    _need(digest == expected_result["full_backing_sha256"]["shared"],
          "wire full-memory digest differs from the independent oracle")

    return {
        "mode": MODE, "profile": PROFILE, "stage": "steady_state_group",
        "batch": batch, "worker_affinity": affinity,
        "ns_per_operation": elapsed_ns / batch,
        "result": expected_result,
        "memory_proof": {"full_backing_sha256": {"shared": digest},
                         "full_memory_bytes_checked": MEMORY_BYTES,
                         "full_memory_pages_checked": MEMORY_PAGES},
        "raw_wire_observation": dict(observation),
    }
