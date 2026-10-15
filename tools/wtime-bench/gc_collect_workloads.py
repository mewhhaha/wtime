"""P071 distinct-object GC collection workloads and independent oracle."""
from __future__ import annotations

import hashlib
from pathlib import Path
from typing import Any

HERE = Path(__file__).resolve().parent
FIXTURE_DIR = HERE / "fixtures" / "gc" / "collect"
INPUTS = (1, 17, 32)
SHAPES = {"gc_collect_wide_32": ("wide_32.wat", 32, "wide"),
          "gc_collect_wide_1024": ("wide_1024.wat", 1024, "wide"),
          "gc_collect_chain_256": ("chain_256.wat", 256, "chain")}
RESET_POLICY = "fresh_instance_for_cold_stages_reused_instance_for_warm_call"
WARM_STATE_POLICY = "one initialized graph retained until final clear and collection"
STAGES = ["prepare", "instantiate", "first_call", "warm_call", "load_and_call"]
WASMTIME_UNSUPPORTED = (
    "The pinned wtime-bench Wasmtime build omits GC support required by this fixture."
)
FAMILIES: dict[str, dict[str, Any]] = {
    "gc_collect_wide_32": {"file": "wide_32.wat", "count": 32, "shape": "wide", "wat_sha256": "fe9db4cbaa704972aee3f5e10adcb8506a927960a3e00affdb415fa6ed2824a0", "wasm_sha256": "cafebdf10cdf2da7bd7a0187dad1a6aa39503700903314a2fbf23415fdb58a0d"},
    "gc_collect_wide_1024": {"file": "wide_1024.wat", "count": 1024, "shape": "wide", "wat_sha256": "ffff54c3c4b94bcef91b049ad365f619b9e0f0008deff7ef085abf31c84782b6", "wasm_sha256": "f5ad59960e3f1fc4500e13dd24eff519d270513e5a2fab6c3323656a4108fd2b"},
    "gc_collect_chain_256": {"file": "chain_256.wat", "count": 256, "shape": "chain", "wat_sha256": "0bbe933d9b342553859c6bee065c842ce596bc6ea47150402c61ac53214eacc6", "wasm_sha256": "5e4d90cb7dbc5369933e69306a673b566bc93d1ed5fca8b3126603c00ce0be9e"},
}


def _signed32(value: int) -> int:
    value &= 0xFFFF_FFFF
    return value - 0x1_0000_0000 if value & 0x8000_0000 else value


def oracle(workload_id: str, repetitions: int) -> int:
    """Index checksum plus one exact live-object count per host collection."""
    if type(repetitions) is not int or repetitions not in INPUTS:
        raise ValueError("repetitions must be one of 1, 17, 32")
    try:
        family, suffix = workload_id.rsplit("_n", 1)
        record = FAMILIES[family]
    except (ValueError, KeyError) as exc:
        raise ValueError(f"unknown GC collection workload: {workload_id}") from exc
    if not suffix.isdecimal() or str(repetitions) != suffix:
        raise ValueError("workload id input does not match repetitions")
    count = record["count"]
    checksum = count * (count - 1) // 2
    object_count = count + 1 if record["shape"] == "wide" else count
    return _signed32(checksum + repetitions * object_count)


def max_batch(count: int, repetitions: int) -> int:
    """Use the declared size-based cap, including its N+1 estimate for chain."""
    if type(count) is not int or count not in (32, 256, 1024):
        raise ValueError("count must match a declared fixture")
    if type(repetitions) is not int or repetitions not in INPUTS:
        raise ValueError("repetitions must be one of 1, 17, 32")
    return min(100_000, 100_000_000 // ((count + 1) * (repetitions + 1)))


def fixture_identities(root: Path = FIXTURE_DIR) -> dict[str, dict[str, str]]:
    """Verify the fixed WAT and encoded Wasm bytes under a contained root."""
    base = root.resolve()
    result = {}
    for family, record in FAMILIES.items():
        wat = (base / record["file"]).resolve()
        wasm = (base / record["file"]).with_suffix(".wasm").resolve()
        if not wat.is_relative_to(base) or not wasm.is_relative_to(base):
            raise ValueError(f"{family}: fixture path escapes fixture root")
        actual_wat = hashlib.sha256(wat.read_bytes()).hexdigest()
        actual_wasm = hashlib.sha256(wasm.read_bytes()).hexdigest()
        if actual_wat != record["wat_sha256"]:
            raise ValueError(f"{family}: WAT identity mismatch: {actual_wat}")
        if actual_wasm != record["wasm_sha256"]:
            raise ValueError(f"{family}: Wasm identity mismatch: {actual_wasm}")
        result[family] = {"wat_sha256": actual_wat, "wasm_sha256": actual_wasm}
    return result


def entries() -> list[dict[str, Any]]:
    """Return three repetitions for each of the three fixed GC shapes."""
    fixture_identities()
    rows = []
    for family, record in FAMILIES.items():
        count = record["count"]
        object_count = count + 1 if record["shape"] == "wide" else count
        for repetitions in INPUTS:
            workload_id = f"{family}_n{repetitions}"
            rows.append({
                "id": workload_id,
                "family": family,
                "source": {
                    "kind": "wat_file",
                    "path": f"fixtures/gc/collect/{record['file']}",
                    "sha256": record["wat_sha256"],
                    "wasm_sha256": record["wasm_sha256"],
                    "encoding_status": "encoded",
                },
                "params": [{"name": "repetitions", "type": "i32", "min": repetitions,
                            "max": repetitions, "baseline": repetitions}],
                "result": {"type": "i32"},
                "oracle": {"kind": "python", "module": "gc_collect_workloads.py",
                           "function": "oracle", "independent": True},
                "oracle_arguments": {"repetitions": repetitions},
                "expected_result": oracle(workload_id, repetitions),
                "expected_outcome": "return",
                "required_features": ["gc"],
                "work_units": {"count": object_count * repetitions,
                               "unit": "live_gc_objects_over_collection_callbacks"},
                "callbacks_count": repetitions,
                "max_batch": max_batch(count, repetitions),
                "reset_policy": RESET_POLICY,
                "warm_state_policy": WARM_STATE_POLICY,
                "stages": STAGES.copy(),
                "driver": {"kind": "gc_collect_stages", "profile": "all-phase3-core",
                           "mode": "decoded", "family": family},
                "comparators": {"wasmtime": {"status": "unsupported",
                                                "reason": WASMTIME_UNSUPPORTED}},
                "validation_role": "primary" if family != "gc_collect_chain_256" else "held_out",
                "gc_expectation": {
                    "allocation_count": object_count,
                    "distinct_leaf_count": count if record["shape"] == "wide" else 0,
                    "callbacks_per_call": repetitions,
                    "callback_live": object_count,
                    "live_after_clear_and_collect": 0,
                    "checksum_constant": count * (count - 1) // 2,
                    "oracle_result": oracle(workload_id, repetitions),
                    "allocations_in_run": 0,
                    "reset_in_timing": False,
                },
            })
    return rows
