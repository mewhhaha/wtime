"""P078 shared bulk-copy rows, independent byte oracle, and strict wire validator."""
from __future__ import annotations

import hashlib
from typing import Any

SCHEMA = "wtime-shared-bulk-copy-v1"
SCHEMA_VERSION = 1
MODE = "decoded"
PROFILE = "all-phase3-core"
WORKERS = (1, 4)
BYTE_COUNTS = (512, 32_768, 4_097)  # small, large, held-out
BACKINGS = ("alias", "distinct")
MEMORY_BYTES = 4 * 65_536
REPETITIONS = 32
MAX_BATCH = 1024
CHUNK_BYTES = 32_768
WAT_PATH = "fixtures/threads/shared_bulk_copy.wat"
WASM_SHA256 = "0ab2531cf95cb52c6fac3a249319343d7ef09ade4b618cacb36c0c7bf2af8a1d"
WAT_SHA256 = "67f616d4c45c3e722949cc0bc092ba98da299984a6549df17b95392b247c0bc6"


class SharedBulkError(ValueError):
    pass


def _source_pattern(*, destination: bool) -> bytes:
    if destination:
        return bytes((index * 29 + 7) % 253 for index in range(MEMORY_BYTES))
    return bytes((index * 17 + 31) % 251 for index in range(MEMORY_BYTES))


def _id(backing: str, workers: int, byte_count: int) -> str:
    return f"shared_bulk_copy_{backing}_w{workers}_b{byte_count}"


def _row_values(workload_id: str, workers: int) -> tuple[str, int, int]:
    if type(workers) is not int or workers not in WORKERS:
        raise SharedBulkError("worker count must be a fixed supported value")
    for backing in BACKINGS:
        for byte_count in BYTE_COUNTS:
            if workload_id == _id(backing, workers, byte_count):
                return backing, workers, byte_count
    raise SharedBulkError("unknown fixed shared-bulk workload")


def entries() -> list[dict[str, Any]]:
    """Produce 12 fixed shared-copy rows with immutable source/binary identities."""
    rows = []
    for backing in BACKINGS:
        for workers in WORKERS:
            for byte_count in BYTE_COUNTS:
                rows.append({
                    "schema_version": 2,
                    "id": _id(backing, workers, byte_count),
                    "family": "shared-bulk",
                    "source": {"kind": "wat_file", "path": WAT_PATH,
                               "sha256": WAT_SHA256,
                               "wasm_sha256": WASM_SHA256},
                    "params": [
                        {"name": "workers", "type": "i32", "min": workers,
                         "max": workers, "baseline": workers},
                        {"name": "bytes", "type": "i32", "min": byte_count,
                         "max": byte_count, "baseline": byte_count},
                    ],
                    "result": {"type": "i32"},
                    "oracle": {"kind": "semantic", "function": "oracle",
                               "module": "shared_bulk_workloads.py", "independent": True,
                               "backing": backing, "workers": workers,
                               "bytes": byte_count, "repetitions": REPETITIONS,
                               "memory_bytes": MEMORY_BYTES},
                    "required_features": ["mvp", "threads"],
                    "profile": {"status": "ready",
                                "reason": "public shared-memory worker driver with full-image oracle"},
                    "work_units": {"unit": "complete shared-bulk worker group", "count": 1},
                    "reset_policy": "fresh_instance_per_invocation",
                    "stages": ["steady_state_group"],
                    "comparators": {"wasmtime": {"status": "unsupported",
                                   "reason": "Pinned comparator workload has no matching shared-bulk worker driver."}},
                    "driver": {"kind": "shared_bulk", "profile": PROFILE,
                               "protocol": SCHEMA,
                               "argv": ["--shared-bulk-copy", "FILE", "CASE", "BACKING",
                                        "WORKERS", "BYTES", "STAGE", "BATCH", "MODE", "PROFILE"]},
                    "shared_bulk": {"backing": backing, "workers": workers,
                                    "repetitions": REPETITIONS,
                                    "destination_base": 65_536,
                                    "worker_destination_stride": CHUNK_BYTES,
                                    "source_bytes_preserved": CHUNK_BYTES,
                                    "memory_bytes_per_backing": MEMORY_BYTES},
                })
    return rows


def oracle(workload_id: str, n: int) -> dict[str, Any]:
    """Return compact independent lifecycle expectations for a fixed registry row."""
    backing, workers, byte_count = _row_values(workload_id, n)
    images = _expected_images(backing, workers, byte_count)
    return {
        "backing": backing, "workers": workers, "repetitions": REPETITIONS,
        "worker_returns": [REPETITIONS] * workers,
        "full_backing_sha256": {name: hashlib.sha256(value).hexdigest()
                                 for name, value in images.items()},
        "full_memory_bytes_checked": MEMORY_BYTES * len(images),
        "full_memory_pages_checked": 4 * len(images),
    }


def _expected_images(backing: str, workers: int, byte_count: int) -> dict[str, bytes]:
    source_initial = _source_pattern(destination=False)
    destination_initial = (source_initial if backing == "alias"
                           else _source_pattern(destination=True))
    images = {"shared": bytearray(source_initial)} if backing == "alias" else {
        "source": bytearray(source_initial),
        "destination": bytearray(destination_initial),
    }
    source = source_initial[:byte_count]
    for worker in range(workers):
        destination_start = 65_536 + worker * CHUNK_BYTES
        destination_name = "shared" if backing == "alias" else "destination"
        images[destination_name][destination_start:destination_start + byte_count] = source
    return {name: bytes(value) for name, value in images.items()}


def expected(entry: dict[str, Any]) -> dict[str, bytes]:
    """Build complete expected images from one fixed registry entry."""
    params = {item.get("name"): item.get("baseline")
              for item in entry.get("params", []) if isinstance(item, dict)}
    backing, workers, byte_count = _row_values(entry.get("id"), params.get("workers"))
    if params.get("bytes") != byte_count:
        raise SharedBulkError("registry byte count differs from the fixed workload ID")
    return _expected_images(backing, workers, byte_count)


def _need(ok: bool, message: str) -> None:
    if not ok:
        raise SharedBulkError(message)


def validate_observation(entry: dict[str, Any], observation: dict[str, Any], *,
                         expected_batch: int | None = None,
                         expected_thread_cpus: list[int] | None = None) -> dict[str, Any]:
    """Validate complete measured groups and normalize timing plus memory proof."""
    params = {item.get("name"): item.get("baseline")
              for item in entry.get("params", []) if isinstance(item, dict)}
    workload_id, workers = entry.get("id"), params.get("workers")
    expected_result = oracle(workload_id, workers)
    expected_images_hex = {name: data.hex() for name, data in expected(entry).items()}
    _, _, byte_count = _row_values(workload_id, workers)
    _need(isinstance(observation, dict), "wire observation must be an object")
    keys = {"schema", "schema_version", "case", "mode", "profile", "stage",
            "backing", "workers", "bytes", "repetitions", "batch", "instrumented",
            "report_complete", "elapsed_ns", "calls_checked",
            "every_group_full_memory_checked", "worker_returns", "worker_affinity",
            "backings_hex"}
    _need(set(observation) == keys, "wire fields differ from the declared protocol")
    for key, value in (("schema", SCHEMA), ("schema_version", SCHEMA_VERSION),
                       ("case", workload_id), ("mode", MODE), ("profile", PROFILE),
                       ("stage", "steady_state_group"), ("backing", expected_result["backing"]),
                       ("workers", expected_result["workers"]), ("bytes", byte_count),
                       ("repetitions", REPETITIONS), ("instrumented", False),
                       ("report_complete", True), ("every_group_full_memory_checked", True)):
        _need(type(observation.get(key)) is type(value) and observation[key] == value,
              f"wire {key} differs from the workload contract")
    batch = observation["batch"]
    _need(type(batch) is int and 1 <= batch <= MAX_BATCH,
          f"wire batch must be an integer in 1..{MAX_BATCH}")
    if expected_batch is not None:
        _need(type(expected_batch) is int and batch == expected_batch,
              "wire batch differs from the predeclared batch")
    workers = expected_result["workers"]
    calls_checked = batch * workers
    _need(type(observation["calls_checked"]) is int
          and observation["calls_checked"] == calls_checked,
          "wire checked call count differs from batch times workers")
    elapsed_ns = observation["elapsed_ns"]
    _need(type(elapsed_ns) is int and elapsed_ns > 0,
          "wire elapsed_ns must be a positive integer group-time sum")
    returns = observation["worker_returns"]
    expected_returns = expected_result["worker_returns"] * batch
    _need(isinstance(returns, list) and all(type(value) is int for value in returns)
          and returns == expected_returns, "worker return count/value differs")
    affinity = observation["worker_affinity"]
    _need(isinstance(affinity, list) and len(affinity) == workers,
          "worker affinity must report one mask per worker")
    for mask in affinity:
        _need(isinstance(mask, list) and bool(mask)
              and all(type(cpu) is int and cpu >= 0 for cpu in mask)
              and mask == sorted(set(mask)),
              "worker affinity masks must be nonempty sorted unique CPU IDs")
    _need(all(mask == affinity[0] for mask in affinity),
          "workers reported different affinity masks")
    if expected_thread_cpus is not None:
        _need(isinstance(expected_thread_cpus, list)
              and all(type(cpu) is int and cpu >= 0 for cpu in expected_thread_cpus)
              and expected_thread_cpus == sorted(set(expected_thread_cpus))
              and all(mask == expected_thread_cpus for mask in affinity),
              "worker affinity differs from the predeclared CPU mask")
    images = observation["backings_hex"]
    _need(isinstance(images, dict) and set(images) == set(expected_images_hex),
          "wire backing set differs")
    actual_hashes = {}
    for name, encoded in images.items():
        _need(type(encoded) is str and len(encoded) == MEMORY_BYTES * 2
              and all(char in "0123456789abcdef" for char in encoded),
              f"wire backing {name} is not a complete lowercase full-memory image")
        raw = bytes.fromhex(encoded)
        digest = hashlib.sha256(raw).hexdigest()
        actual_hashes[name] = digest
        _need(encoded == expected_images_hex[name],
              f"wire backing {name} differs from the independent full-memory oracle")
        _need(digest == expected_result["full_backing_sha256"][name],
              f"wire backing {name} digest differs")
    _need(actual_hashes == expected_result["full_backing_sha256"],
          "computed full-backing digests differ from the oracle")
    _need(expected_result["full_memory_bytes_checked"] == MEMORY_BYTES * len(images),
          "full-memory byte accounting differs")
    _need(expected_result["full_memory_pages_checked"] == 4 * len(images),
          "full-memory page accounting differs")
    return {"mode": MODE, "profile": PROFILE, "stage": "steady_state_group",
            "batch": batch, "worker_affinity": affinity,
            "ns_per_operation": elapsed_ns / batch,
            "result": expected_result,
            "memory_proof": {"full_backing_sha256": actual_hashes,
                             "full_memory_bytes_checked": expected_result["full_memory_bytes_checked"],
                             "full_memory_pages_checked": expected_result["full_memory_pages_checked"]},
            "raw_wire_observation": dict(observation)}
