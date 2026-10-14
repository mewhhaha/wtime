"""Deterministic scalar benchmark fixture entries and independent oracles."""

from __future__ import annotations

import hashlib
import subprocess
from pathlib import Path
from typing import Any

HERE = Path(__file__).resolve().parent
FIXTURE_DIR = HERE / "fixtures" / "scalar"
PRIMARY_INPUTS = (1, 32, 1024, 10000)
HELD_OUT_INPUTS = (0, 17)
INPUTS = PRIMARY_INPUTS + HELD_OUT_INPUTS
RECURSION_INPUTS = (0, 1, 17, 32)
STAGES = ["prepare", "instantiate", "first_call", "warm_call", "load_and_call"]
RESET_POLICY = "fresh_instance_for_cold_stages_reused_instance_for_warm_call"
# P042 cache-pattern rows are oracle-only additions. Keep them out of FAMILIES
# so scalar_workloads.entries() and the original 85-row selection stay fixed.
P042_EXTRA_FAMILIES = (
    "scalar_table_indirect_2targets",
    "scalar_table_indirect_3targets",
    "scalar_typed_call_ref_2targets",
    "scalar_typed_call_ref_3targets",
    "scalar_table_between_calls",
)
FAMILIES = {
    "scalar_accumulator": ("accumulator.wat", "integer_add"),
    "scalar_dependent": ("dependent.wat", "dependent_lcg_step"),
    "scalar_branch_entropy": ("branch_entropy.wat", "entropy_branch_iteration"),
    "scalar_br_table": ("br_table.wat", "br_table_iteration"),
    "scalar_direct_calls": ("direct_calls.wat", "direct_guest_call"),
    "scalar_indirect_calls": ("indirect_calls.wat", "indirect_guest_call"),
    "scalar_tail_calls": ("tail_calls.wat", "tail_guest_call"),
    "scalar_dynamic_api": ("dynamic_api.wat", "dynamic_public_call"),
    "scalar_multivalue_branch": ("multivalue_branch.wat", "multivalue_branch"),
    "scalar_ordinary_recursion": ("ordinary_recursion.wat", "ordinary_guest_call"),
    "scalar_call_ref": ("call_ref.wat", "typed_function_reference_call"),
    "scalar_table_mutation": ("table_controls.wat", "mutable_indirect_guest_call"),
    "scalar_host_call": ("host_call.wat", "host_callback"),
    "scalar_try_table_normal": ("try_table_normal.wat", "try_table_iteration"),
    "scalar_try_table_normal": ("try_table_normal.wat", "try_table_iteration"),
}
FEATURES = {
    "scalar_multivalue_branch": ["mvp", "multi_value"],
    "scalar_ordinary_recursion": ["mvp"],
    "scalar_call_ref": ["mvp", "reference_types", "function_references"],
    "scalar_table_mutation": ["mvp", "reference_types"],
    "scalar_try_table_normal": ["mvp", "exception_handling"],
    "scalar_try_table_normal": ["mvp", "exception_handling"],
}
WASM_SHA256 = {
    "accumulator.wat": "ef61bf7afc44af7b71ef78eadcba48c0d85949d7d9096df8fbbbab53c05036b1",
    "br_table.wat": "d43d5dd71d5ca1c3dae820aa85c13a2d69415e028b8d48097fed4558f743163a",
    "branch_entropy.wat": "e4c87dbab50a2438cb1f749bc6fd6f9c834c81484acd42864735bd1ad1b6dec0",
    "call_ref.wat": "4f766dc9e29324ea942c892982363dec0ade98565288878a70967ce3b33f322b",
    "dependent.wat": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c",
    "direct_calls.wat": "f6adbecce99c05bf06f213f4f5c0d5d9f001074acbbb2e6bbe814696bb866e45",
    "dynamic_api.wat": "c493d0aae68d0c5aa67fc0b562e616c45eb3d42b0302951158a763fa202ce589",
    "host_call.wat": "c60311d57c929c3ff1d29adb70501e7bbf8419517acbd86c8d072438b610ebb7",
    "indirect_calls.wat": "4b42e82f54ea6ab7d0f62c70766c904e41d6a408e57fa14b5abbf514c0da78a9",
    "multivalue_branch.wat": "03ffdc2041ae8ba0c8d078422d36152cf1d35f1293c94a4074fe7e83791743ae",
    "ordinary_recursion.wat": "e1bd7ea6ada8b15b2770ac387f8fc2256f20d7f87c4bc34e7559b239d8875ba1",
    "table_controls.wat": "458f2b111e48d2efca98083b8c56860c43e82be8752813d1e2814d23c28d4f43",
    "tail_calls.wat": "3e23be294e7a9eda62c89dbc366b6a6cab90edfbda532da27a7e4fc6d0aa4a56",
    "try_table_normal.wat": "97068a6310a3bc535369e2b6277b801875c14ca206a707fcb9bca9036f41d0df",
    "try_table_normal.wat": "97068a6310a3bc535369e2b6277b801875c14ca206a707fcb9bca9036f41d0df",
}


def _signed(value: int) -> int:
    value &= 0xFFFF_FFFF
    return value - 0x1_0000_0000 if value & 0x8000_0000 else value


def table_mutation_probe(n: int) -> tuple[int, int]:
    """Return outputs with target A/B; target setup itself is outside timing."""
    first = 1
    second = 1
    for _ in range(n):
        first = (first + 1) & 0xFFFF_FFFF
        second = (second * 2 + 1) & 0xFFFF_FFFF
    return _signed(first), _signed(second)


def oracle(workload_id: str, n: int) -> int:
    """Return the signed i32 result for a fixture family and runtime input."""
    if type(n) is not int or not 0 <= n <= 0x7FFF_FFFF:
        raise ValueError("n must be a nonnegative signed i32")
    family = workload_id
    for candidate in P042_EXTRA_FAMILIES:
        if workload_id == candidate or workload_id.startswith(candidate + "_n"):
            # Every target computes x + 1 and the fixture folds one successful
            # call per loop iteration from zero, so n calls produce exactly n.
            return _signed(n)
    for candidate in FAMILIES:
        if workload_id == candidate or workload_id.startswith(candidate + "_n"):
            family = candidate
            break
    if family == "scalar_accumulator" or family == "scalar_try_table_normal":
        return _signed(n * (n - 1) // 2)
    if family == "scalar_dependent" or family in {
        "scalar_direct_calls", "scalar_indirect_calls", "scalar_tail_calls",
        "scalar_ordinary_recursion", "scalar_call_ref",
    }:
        value = 1
        for _ in range(n):
            if family == "scalar_dependent":
                value = (value * 1664525 + 1013904223) & 0xFFFF_FFFF
            else:
                value = (value * 3 + 1) & 0xFFFF_FFFF
        return _signed(value)
    if family == "scalar_branch_entropy":
        state = 1
        for _ in range(n):
            state = (state * 1664525 + 1013904223) & 0xFFFF_FFFF
            if state & 0x8000_0000:
                state ^= 1511506142
            else:
                state = (state + 265443576) & 0xFFFF_FFFF
        return _signed(state)
    if family == "scalar_br_table":
        increments = (1, 3, 7, 13)
        return _signed(sum(increments[i & 3] for i in range(n)))
    if family == "scalar_dynamic_api":
        return _signed(n + 1)
    if family == "scalar_multivalue_branch":
        value = 1
        for i in range(n):
            value = ((value + 1) if i & 1 else (value * 3)) & 0xFFFF_FFFF
        return _signed(value)
    if family == "scalar_table_mutation":
        return table_mutation_probe(n)[0]
    if family == "scalar_host_call":
        # The deterministic test callback returns its i32 argument plus one.
        return _signed(n)
    raise ValueError(f"unknown scalar workload id: {workload_id}")


def _family_inputs(family: str) -> tuple[int, ...]:
    if family == "scalar_ordinary_recursion":
        return RECURSION_INPUTS
    return INPUTS


def entries() -> list[dict[str, Any]]:
    """Return full schema-v2 entries with predeclared held-out cases and probes."""
    result: list[dict[str, Any]] = []
    for family, (filename, unit) in FAMILIES.items():
        source_path = FIXTURE_DIR / filename
        source_hash = hashlib.sha256(source_path.read_bytes()).hexdigest()
        required = FEATURES.get(family, ["mvp"]).copy()
        feature_profile = required[1:] if len(required) > 1 else ["mvp"]
        if family == "scalar_tail_calls":
            required.append("tail_call")
            feature_profile = ["tail_call"]
        for n in _family_inputs(family):
            wid = f"{family}_n{n}"
            entry: dict[str, Any] = {
                "id": wid,
                "family": family,
                "source": {
                    "kind": "wat_file",
                    "path": f"fixtures/scalar/{filename}",
                    "sha256": source_hash,
                    "wasm_sha256": WASM_SHA256[filename],
                    "encoding_status": "encoded",
                },
                "params": [{"name": "n", "type": "i32", "min": n, "max": n, "baseline": n}],
                "result": {"type": "i32"},
                "oracle": {
                    "kind": "python",
                    "module": "scalar_workloads.py",
                    "function": "oracle",
                    "independent": True,
                },
                "oracle_arguments": {"n": n},
                "required_features": required.copy(),
                "work_units": {"count": n if family != "scalar_dynamic_api" else 1,
                               "unit": unit},
                "reset_policy": RESET_POLICY,
                "stages": STAGES.copy(),
                "comparators": {
                    "wasmtime": {
                        "status": "probe_required",
                        "feature_profile": feature_profile.copy(),
                    }
                },
                "validation_role": "held_out" if n in HELD_OUT_INPUTS else "primary",
                "api_comparison": {
                    "dynamic_values": {"status": "available", "entrypoint": "Store::invoke"},
                    "typed_guest_call": {
                        "status": "unavailable",
                        "reason": "The public API exposes dynamic Store::invoke for guest functions; create_typed_host_func is a host callback API, not a typed guest-call API.",
                    },
                },
            }
            if family == "scalar_ordinary_recursion":
                entry["driver_requirements"] = {
                    "timed_input_max": 32,
                    "correctness_probes": [
                        {"input": 1024, "expected_outcome": "call_depth_limit"},
                        {"input": 10000, "expected_outcome": "call_depth_limit"},
                    ],
                    "probe_timings": "excluded; these inputs verify ordinary recursion reaches the configured logical-depth limit",
                }
            elif family == "scalar_table_mutation":
                before, after = table_mutation_probe(n)
                entry["measurement_status"] = "driver_setup_required"
                entry["driver_requirements"] = {
                    "setup_export": "set_target",
                    "timed_export": "run",
                    "correctness_probes": [
                        {"setup": {"export": "set_target", "args": [0]}, "run_args": [n], "expected": before},
                        {"setup": {"export": "set_target", "args": [1]}, "run_args": [n], "expected": after},
                        {"export": "probe_null", "expected_outcome": "uninitialized_element_trap"},
                        {"export": "probe_mismatch", "expected_outcome": "indirect_call_type_mismatch_trap"},
                    ],
                    "timing_rule": "set_target outside timed batches; null and mismatched probes are correctness-only",
                }
            elif family == "scalar_host_call":
                entry["measurement_status"] = "host_setup_required"
                entry["driver_requirements"] = {
                    "imports": [{"module": "host", "name": "tick", "signature": "(i32)->i32"}],
                    "callback_oracle": "return argument + 1",
                    "timing_rule": "host import creation and linker setup outside invocation timing",
                }
            result.append(entry)
    return result


def encode_fixtures(encoder: str | Path, targetdir: str | Path) -> dict[str, str]:
    """Encode fixture WAT with an already-built encoder, never building it."""
    target = Path(targetdir)
    target.mkdir(parents=True, exist_ok=True)
    hashes: dict[str, str] = {}
    for filename, _unit in FAMILIES.values():
        source = FIXTURE_DIR / filename
        output = target / f"{source.stem}.wasm"
        subprocess.run([str(encoder), "encode", str(source), "-o", str(output)],
                       check=True, cwd=HERE.parent.parent)
        hashes[source.name] = hashlib.sha256(output.read_bytes()).hexdigest()
    return hashes
