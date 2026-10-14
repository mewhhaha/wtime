"""P125 fixed-bank GC root snapshot workloads and independent result oracle."""
from __future__ import annotations

import hashlib
from pathlib import Path
from typing import Any

HERE = Path(__file__).resolve().parent
FIXTURE_DIR = HERE / "fixtures" / "gc" / "roots"
INPUTS = (0, 1, 17, 32)
BANKS = (64, 256, 512)
SHAPES = ("numeric", "reference")
RESET_POLICY = "fresh_instance_for_cold_stages_reused_instance_for_warm_call"
STAGES = ["prepare", "instantiate", "first_call", "warm_call", "load_and_call"]
WASMTIME_UNSUPPORTED = (
    "The pinned wtime-bench Wasmtime build omits GC support required by this fixture."
)

FAMILIES: dict[str, dict[str, Any]] = {
    "gc_root_snapshots_numeric_64": {
        "file": "root_numeric_64.wat", "wat_sha256": "774e252694b779c271d153e62e481a0e22aa770015947fcde0a3478ceebceb59",
        "wasm_sha256": "1c6a6a8349ed5788fe3bbef01f1de97755327b874f1ca1bb9a63b8ebe4121013", "shape": "numeric", "bank": 64,
    },
    "gc_root_snapshots_numeric_256": {
        "file": "root_numeric_256.wat", "wat_sha256": "c4e717d0a3514d8a4d784bf72fd8ffaa63d73a001dcaf27454c446b48ec720e9",
        "wasm_sha256": "0afc4043fc2919ddd466d1b4a89281b48281ac5e567d5c49c94cf41d28f33412", "shape": "numeric", "bank": 256,
    },
    "gc_root_snapshots_numeric_512": {
        "file": "root_numeric_512.wat", "wat_sha256": "f0fd64cb87de3404559acdec57c14af619c4a1959f169f69e23ef517d7fb4487",
        "wasm_sha256": "e03dd87300e95c5f1a94f4befe2ff7820e6c91a8bd32b7958abae96e5cfebd93", "shape": "numeric", "bank": 512,
    },
    "gc_root_snapshots_reference_64": {
        "file": "root_reference_64.wat", "wat_sha256": "11c57d5a347b71b492da01678bab8ae9cd853710d9e9526b6c5ddeb730697bdb",
        "wasm_sha256": "ce260e4693744fb37d7f40ef0c30ea7072b47fb64b57715390f15db288cdad8f", "shape": "reference", "bank": 64,
    },
    "gc_root_snapshots_reference_256": {
        "file": "root_reference_256.wat", "wat_sha256": "499df04692c77e5c7c8edf45d2a84a0d41e4b926489dcce8c17f7a129dc3e100",
        "wasm_sha256": "5344dc604f261464555b2f22a497ae74b45811d72faa543124243ae3d881afdb", "shape": "reference", "bank": 256,
    },
    "gc_root_snapshots_reference_512": {
        "file": "root_reference_512.wat", "wat_sha256": "46265b44e96969da2f08dec2650cfd374763a91c827330a450115a787dd7db73",
        "wasm_sha256": "f57cb65c58aefa656614cfe1394321a656494fae0192208d376ecc4db637a94c", "shape": "reference", "bank": 512,
    },
}


def _signed32(value: int) -> int:
    value &= 0xFFFF_FFFF
    return value - 0x1_0000_0000 if value & 0x8000_0000 else value


def oracle(workload_id: str, n: int) -> int:
    """Enumerate each bank slot and callback contribution, independently of Rust."""
    if type(n) is not int or n not in INPUTS:
        raise ValueError("n must be one of 0, 1, 17, 32")
    try:
        family, suffix = workload_id.rsplit("_n", 1)
        record = FAMILIES[family]
    except (ValueError, KeyError) as exc:
        raise ValueError(f"unknown root snapshot workload: {workload_id}") from exc
    if not suffix.isdecimal() or str(n) != suffix:
        raise ValueError("workload id input does not match n")

    total = 0
    for slot in range(record["bank"]):
        total += n + slot if record["shape"] == "numeric" else n
    # Each callback reenters nested() and receives its 42 result; root contributes n.
    for _ in range(n):
        total += 42
    total += n
    return _signed32(total)


def max_batch(bank_slots: int, n: int) -> int:
    """Bound concurrent samples by bank shape and invocation callback count."""
    if type(bank_slots) is not int or bank_slots not in BANKS:
        raise ValueError("bank_slots must be one of 64, 256, 512")
    if type(n) is not int or n not in INPUTS:
        raise ValueError("n must be one of 0, 1, 17, 32")
    return min(100_000, 100_000_000 // ((bank_slots + 8) * (n + 1)))


def fixture_identities(root: Path = FIXTURE_DIR) -> dict[str, dict[str, str]]:
    """Verify WAT and Wasm identities under root, rejecting paths outside root."""
    base = root.resolve()
    result = {}
    for family, record in FAMILIES.items():
        wat = (base / record["file"]).resolve()
        wasm = wat.with_suffix(".wasm").resolve()
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
    """Build the 24 fixed-bank rows after verifying all encoded fixture bytes."""
    fixture_identities()
    rows = []
    for family, record in FAMILIES.items():
        bank = record["bank"]
        for n in INPUTS:
            workload_id = f"{family}_n{n}"
            rows.append({
                "id": workload_id,
                "family": family,
                "source": {
                    "kind": "wat_file",
                    "path": f"fixtures/gc/roots/{record['file']}",
                    "sha256": record["wat_sha256"],
                    "wasm_sha256": record["wasm_sha256"],
                    "encoding_status": "encoded",
                },
                "params": [{"name": "n", "type": "i32", "min": n, "max": n, "baseline": n}],
                "result": {"type": "i32"},
                "oracle": {"kind": "python", "module": "root_workloads.py", "function": "oracle", "independent": True},
                "oracle_arguments": {"n": n},
                "expected_result": oracle(workload_id, n),
                "expected_outcome": "return",
                "required_features": ["gc"],
                "work_units": {"count": bank + n, "unit": "root_bank_slots_and_host_gc_callbacks"},
                "callbacks_count": n,
                "max_batch": max_batch(bank, n),
                "reset_policy": RESET_POLICY,
                "stages": STAGES.copy(),
                "driver": {"kind": "gc_root_stages", "profile": "all-phase3-core", "family": family},
                "comparators": {"wasmtime": {"status": "unsupported", "reason": WASMTIME_UNSUPPORTED}},
                "validation_role": "held_out" if n in (0, 17) else "primary",
                "gc_expectation": {
                    "allocation_count": 1,
                    "bank_slots": bank,
                    "shape": record["shape"],
                    "callbacks_per_call": n,
                    "nested_calls_per_call": n,
                    "callback_live": 1 if n else 0,
                    "callback_reclaimed": 0,
                    "live_after_reset": 0,
                    "payload_bytes_after_reset": 0,
                    "continuation_bytes_after_reset": 0,
                    "oracle_result": oracle(workload_id, n),
                    "reset_in_timing": False,
                },
            })
    return rows
