"""P033 warm GC workload metadata and independent result oracles."""
from __future__ import annotations

import copy
import hashlib
import json
from pathlib import Path
from typing import Any

HERE = Path(__file__).resolve().parent
FIXTURE_DIR = HERE / "fixtures" / "gc"
BASE_REGISTRY = HERE / "workloads.json"
INPUTS = (1, 32, 1024, 10000, 0, 17)
STAGES = ["prepare", "instantiate", "first_call", "warm_call", "load_and_call"]
RESET_POLICY = "fresh_instance_for_cold_stages_reused_instance_for_warm_call"
WASMTIME_UNSUPPORTED = (
    "The pinned wtime-bench Wasmtime 49.0.2 build omits its gc Cargo feature; "
    "wasm_exceptions also requires GC compiled into the build."
)

# Fixture hashes bind these rows to their checked-in WAT and encoded Wasm.
FAMILIES: dict[str, dict[str, Any]] = {
    "gc_warm_nominal_exact_casts": {
        "family": "gc_nominal_exact_casts", "file": "nominal_exact_casts.wat",
        "wat_sha256": "278b748822210879d166c0131c10b88ac32b38bf8437c1ada05667e1d8156ada",
        "wasm_sha256": "6fc8a2cac95a90fe0d1a3d7c1cf1a9f57041df41f6ec8ccae747c1368b9f03e2",
        "unit": "cast_checks", "features": ["gc"],
    },
    "gc_warm_typed_function_refs": {
        "family": "gc_typed_function_refs", "file": "typed_function_refs.wat",
        "wat_sha256": "5d62ffda642de10f8ddbfa828512e8bc7d36d3cbcd70ce2e52f5532c030d990b",
        "wasm_sha256": "c07a43c30d251223ebd9769ef3742eb32ec5d3ef751d6a9f0e1507ad353b4cb7",
        "unit": "typed_calls", "features": ["gc", "function_references"],
    },
    "gc_warm_type_churn_64": {
        "family": "gc_type_churn_64", "file": "type_churn_64.wat",
        "wat_sha256": "6688be1ceb64af3446c2027fa03639e136fec8d27646915888755f13c4fd1de0",
        "wasm_sha256": "37663a4ba8344cc5d09120a231c4e71699d1a9038e9a9788a5b5bfc03fe2b930",
        "unit": "indirect_calls", "features": ["gc", "function_references"],
    },
    "gc_warm_type_churn_256": {
        "family": "gc_type_churn_256", "file": "type_churn_256.wat",
        "wat_sha256": "f5431b25768d26b0022060c7c5d8fb58049c55393d3f68e1b9b3657db527c5b2",
        "wasm_sha256": "bd136a0aeae3917c1af98d7a50de8f8b78b7db1ed1e4b3a92ebc5fa3bb172f17",
        "unit": "indirect_calls", "features": ["gc", "function_references"],
    },
    "gc_warm_type_churn_1024": {
        "family": "gc_type_churn_1024", "file": "type_churn_1024.wat",
        "wat_sha256": "cc0e5ea4534c2464bf1b1b286ece900942882174da20af3d506ddff6f8ef813b",
        "wasm_sha256": "09f4dcbb91c1575f2f4ad2115d6198ae7959f048c66028ada960b5b4a2061bfd",
        "unit": "indirect_calls", "features": ["gc", "function_references"],
    },
    "gc_warm_indirect_subtype_calls": {
        "family": "gc_indirect_subtype_calls", "file": "type_repeat_128.wat",
        "wat_sha256": "bb8a419be00efc018b7715550878ec1aed52c20140acacf2b1aa840ca8ad94c0",
        "wasm_sha256": "a85c15e59addfdd67d5d246a309fd5bbfe958b744a73863baec0500791d13084",
        "unit": "indirect_subtype_calls", "features": ["gc", "function_references"],
    },
}


def _signed32(value: int) -> int:
    value &= 0xFFFF_FFFF
    return value - 0x1_0000_0000 if value & 0x8000_0000 else value


def _count(n: int) -> int:
    if type(n) is not int or not 0 <= n <= 10_000:
        raise ValueError("n must be an integer in 0..=10000")
    return n


def _family(workload_id: str) -> str:
    if not isinstance(workload_id, str):
        raise ValueError("workload id must be a string")
    for family_id, record in FAMILIES.items():
        prefix = family_id + "_n"
        if workload_id.startswith(prefix):
            suffix = workload_id[len(prefix):]
            if not suffix.isdecimal():
                break
            n = int(suffix)
            if str(n) != suffix or n not in INPUTS:
                break
            return record["family"]
    raise ValueError(f"unknown P033 GC warm workload id: {workload_id}")


def oracle(workload_id: str, n: int) -> int:
    """Return an independently derived signed-i32 result for a workload."""
    n = _count(n)
    family = _family(workload_id)
    encoded_n = int(workload_id.rsplit("_n", 1)[1])
    if encoded_n != n:
        raise ValueError("workload id input does not match n")
    if family == "gc_nominal_exact_casts":
        # Base succeeds, exact-base fails, child succeeds for every child value.
        total = sum(1 + 0 + 1 for _ in range(n))
    elif family == "gc_typed_function_refs":
        # call_ref receives i and returns i+1; run accumulates each call result.
        total = sum(index + 1 for index in range(n))
    elif family == "gc_indirect_subtype_calls":
        total = sum(128 for _ in range(n))
    else:
        # type_churn_N dispatches to slot i % N, whose function returns that
        # slot. Enumerating the slot cycle is independent of the quotient formula.
        width = int(family.rsplit("_", 1)[1])
        total = sum(index % width for index in range(n))
    return _signed32(total)


def fixture_identities() -> dict[str, dict[str, str]]:
    """Verify WAT and encoded Wasm bytes against frozen identities."""
    result = {}
    for workload_family, record in FAMILIES.items():
        stem = Path(record["file"]).stem
        wat = FIXTURE_DIR / record["file"]
        wasm = FIXTURE_DIR / f"{stem}.wasm"
        actual_wat = hashlib.sha256(wat.read_bytes()).hexdigest()
        if actual_wat != record["wat_sha256"]:
            raise ValueError(f"{workload_family}: WAT identity mismatch: {actual_wat}")
        expected_wasm = record["wasm_sha256"]
        if not expected_wasm:
            raise ValueError(
                f"{workload_family}: encoded Wasm SHA-256 is missing; cannot register this model"
            )
        if not wasm.is_file():
            raise ValueError(f"{workload_family}: encoded Wasm fixture is missing: {wasm}")
        actual_wasm = hashlib.sha256(wasm.read_bytes()).hexdigest()
        if actual_wasm != expected_wasm:
            raise ValueError(f"{workload_family}: Wasm identity mismatch: {actual_wasm}")
        result[workload_family] = {"wat_sha256": actual_wat, "wasm_sha256": actual_wasm}
    return result


def _build_entries() -> list[dict[str, Any]]:
    """Build the declared P033 rows after fixture identity validation."""
    rows = []
    for workload_family, record in FAMILIES.items():
        filename = record["file"]
        for n in INPUTS:
            workload_id = f"{workload_family}_n{n}"
            rows.append({
                "id": workload_id,
                "family": record["family"],
                "source": {
                    "kind": "wat_file",
                    "path": f"fixtures/gc/{filename}",
                    "sha256": record["wat_sha256"],
                    "wasm_sha256": record["wasm_sha256"],
                    "encoding_status": "encoded" if record["wasm_sha256"] else "pending",
                },
                "params": [{"name": "n", "type": "i32", "min": n,
                            "max": n, "baseline": n}],
                "result": {"type": "i32"},
                "oracle": {"kind": "python", "module": "gc_warm_workloads.py",
                           "function": "oracle", "independent": True},
                "oracle_arguments": {"n": n},
                "required_features": list(record["features"]),
                "work_units": {"count": n, "unit": record["unit"]},
                "reset_policy": RESET_POLICY,
                "stages": STAGES.copy(),
                "driver": {"kind": "gc_scalar_stages", "profile": "all-phase3-core",
                           "family": record["family"]},
                "comparators": {"wasmtime": {"status": "unsupported",
                    "reason": WASMTIME_UNSUPPORTED}},
                "expected_outcome": "return",
                "validation_role": "held_out" if n in (0, 17) else "primary",
                "gc_expectation": {
                    "allocation_count": n if record["family"] == "gc_nominal_exact_casts" else 0,
                    "oracle_result": oracle(workload_id, n),
                    "live_after_reset": 0,
                    "payload_bytes_after_reset": 0,
                    "continuation_bytes_after_reset": 0,
                    "reset_in_timing": False,
                },
            })
    return rows




def entries() -> list[dict[str, Any]]:
    """Produce rows only after every WAT/Wasm identity is verified."""
    fixture_identities()
    return _build_entries()


def _append_rows(registry: dict[str, Any], addition: list[dict[str, Any]]) -> dict[str, Any]:
    """Append already validated rows to a copy and preserve all old prefixes."""
    updated = copy.deepcopy(registry)
    old_workloads = updated.get("workloads")
    selections = updated.get("selections")
    if not isinstance(old_workloads, list) or not isinstance(selections, dict):
        raise ValueError("registry must contain workloads and selections")
    selection = selections.get("gc-exceptions")
    if not isinstance(selection, dict) or selection.get("status") != "ready":
        raise ValueError("gc-exceptions selection is not ready")
    old_ids = [row.get("id") for row in old_workloads if isinstance(row, dict)]
    if len(old_ids) != len(old_workloads) or len(old_ids) != len(set(old_ids)):
        raise ValueError("registry workload IDs are malformed or duplicated")
    if len(old_workloads) != 314:
        raise ValueError(f"expected the frozen 314-row registry prefix, found {len(old_workloads)}")
    addition_ids = [row["id"] for row in addition]
    if set(old_ids).intersection(addition_ids):
        raise ValueError("P033 append-only IDs already exist")
    old_selection = selection.get("workloads")
    if not isinstance(old_selection, list) or len(old_selection) != 45:
        raise ValueError("expected the unchanged 45-row legacy gc-exceptions selection")
    old_workloads.extend(addition)
    selection["workloads"].extend(addition_ids)
    return updated


def append_to_registry(registry: dict[str, Any]) -> dict[str, Any]:
    """Return a copy with rows appended after fixture identity checks."""
    return _append_rows(registry, entries())


def append_to_live_registry() -> dict[str, Any]:
    """Append in memory from the configured registry; never writes it."""
    return append_to_registry(json.loads(BASE_REGISTRY.read_text()))

