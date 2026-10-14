"""Independent deterministic models and proposal rows for P027 bulk slices.

Both guests expose complete memories for untimed host seeding. The model keeps
the full source and destination images separate and hashes complete images as
well as the operation's destination range.
"""
from __future__ import annotations

import hashlib
from pathlib import Path
from typing import Any

PAGE = 65536
SIZES = (64, 4096, 1 << 20)  # Original six primary measurements remain first.
HELDOUT_SIZES = (0, 17)
ALL_SIZES = SIZES + HELDOUT_SIZES
STAGES = ["prepare", "instantiate", "first_call", "warm_call", "load_and_call"]
HERE = Path(__file__).resolve().parent
FIXTURE_DIR = HERE / "fixtures" / "memory"

# Filled from actual output of the existing encoder; these are source identities.
WASM_SHA256 = {
    "memory_copy_distinct_seeded.wat": "ed96c8762aaa1b5ba97515083ec7bcfcf54b0dec4ed0c06f93552fd53cfeb722",
    "memory_fill_seeded.wat": "ba786f97a94a2ceb1546b05534334aa33154edc7a47a6ebff859521b80fc6f7f",
}


def _sha(data: bytes | bytearray) -> str:
    return hashlib.sha256(data).hexdigest()


def _pattern(length: int, multiplier: int, addend: int, page_multiplier: int) -> bytearray:
    return bytearray((i * multiplier + addend + (i >> 16) * page_multiplier) & 0xFF for i in range(length))


def _u32(n: int) -> int:
    if type(n) is not int or not 0 <= n <= 0x7FFF_FFFF:
        raise ValueError("n must be a nonnegative signed i32")
    return n


def _family(workload_id: str) -> str:
    return workload_id.rsplit("_", 1)[0] if workload_id.rsplit("_", 1)[-1].isdigit() else workload_id


def _id(family: str, n: int) -> str:
    return f"{family}_{n}"


def expected_memory(workload_id: str, n: int) -> dict[str, Any]:
    """Return exact full-memory expectations for one invocation."""
    n = _u32(n)
    family = _family(workload_id)
    if n not in ALL_SIZES:
        raise ValueError("P027 rows use only the three primary and two held-out byte counts")
    if family == "memory_copy_distinct_seeded":
        source = _pattern(16 * PAGE, 29, 7, 53)
        destination = _pattern(17 * PAGE, 13, 101, 37)
        source_initial_sha = _sha(source)
        destination_initial_sha = _sha(destination)
        begin, end = PAGE, PAGE + n
        destination[begin:end] = source[:n]
        memories = [
            {"memory_index": 0, "export": "source", "pages": 16,
             "initial_full_memory_sha256": source_initial_sha,
             "full_memory_sha256": _sha(source),
             "range": [0, len(source)], "range_sha256": _sha(source)},
            {"memory_index": 1, "export": "memory", "pages": 17,
             "initial_full_memory_sha256": destination_initial_sha,
             "full_memory_sha256": _sha(destination),
             "range": [begin, end], "range_sha256": _sha(destination[begin:end])},
        ]
        return {"result": n, "memories": memories, "operation": "memory.copy",
                "source_range": [0, n], "destination_range": [begin, end],
                "setup_policy": "host_write_memory_outside_invocation_timer",
                "setup_patterns": {"source": "(i*29+7)&255", "destination": "(i*13+101)&255"},
                "idempotent": True}
    if family == "memory_fill_seeded":
        memory = _pattern(17 * PAGE, 13, 101, 37)
        initial_sha = _sha(memory)
        begin, end = 4096, 4096 + n
        memory[begin:end] = bytes([90]) * n
        return {"result": n,
                "memories": [{"memory_index": 0, "export": "memory", "pages": 17,
                              "initial_full_memory_sha256": initial_sha,
                              "full_memory_sha256": _sha(memory),
                              "range": [begin, end], "range_sha256": _sha(memory[begin:end])}],
                "operation": "memory.fill", "destination_range": [begin, end],
                "setup_policy": "host_write_memory_outside_invocation_timer",
                "setup_patterns": {"memory": "(i*13+101)&255"},
                "fill_byte": 90, "idempotent": True}
    raise ValueError(f"unknown P027 workload id: {workload_id}")


def _source(filename: str) -> dict[str, str]:
    source = FIXTURE_DIR / filename
    return {"kind": "wat_file", "path": f"fixtures/memory/{filename}",
            "sha256": _sha(source.read_bytes()), "wasm_sha256": WASM_SHA256[filename],
            "encoding_status": "encoded"}


def entries() -> list[dict[str, Any]]:
    rows = []
    families = (
        ("memory_copy_distinct_seeded", "memory_copy_distinct_seeded.wat"),
        ("memory_fill_seeded", "memory_fill_seeded.wat"),
    )

    def make_entry(family: str, filename: str, size: int, *, heldout: bool = False):
        workload_id = _id(family, size)
        row = {
            "id": workload_id,
            "family": family,
            "source": _source(filename),
            "params": [{"name": "bytes", "type": "i32", "min": size,
                        "max": size, "baseline": size}],
            "result": {"type": "i32"},
            "oracle": {"kind": "python", "module": "p027_memory_workloads.py",
                       "function": "oracle", "independent": True},
            "required_features": ["mvp", "bulk_memory"],
            "work_units": {"count": size, "unit": "bytes"},
            "reset_policy": "fresh_instance_for_cold_stage_reused_for_idempotent_warm_call",
            "stages": STAGES.copy(),
            "comparators": {"wasmtime": {"status": "probe_required",
                                           "feature_profile": ["mvp", "bulk_memory"]}},
            "memory_expectation": expected_memory(workload_id, size),
            "expected_outcome": "return",
        }
        if family == "memory_copy_distinct_seeded":
            row["required_features"].append("multi_memory")
            row["comparators"]["wasmtime"]["feature_profile"].append("multi_memory")
        if heldout:
            row["comparison_role"] = "heldout_control"
            row["protected_control"] = True
        return row

    # Preserve the original six primary rows and their ordering.
    for family, filename in families:
        for size in SIZES:
            rows.append(make_entry(family, filename, size))
    # Add protected controls after the unchanged primary block.
    for family, filename in families:
        for size in HELDOUT_SIZES:
            rows.append(make_entry(family, filename, size, heldout=True))
    return rows


def oracle(workload_id: str, n: int) -> int:
    """Scalar oracle is independent of full image materialization and cache-free."""
    _u32(n)
    if _family(workload_id) not in {"memory_copy_distinct_seeded", "memory_fill_seeded"}:
        raise ValueError(f"unknown P027 workload id: {workload_id}")
    return n
