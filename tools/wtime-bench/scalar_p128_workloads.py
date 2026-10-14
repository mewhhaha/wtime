"""Fixed, independent model for the admitted P128 scalar fixtures."""
from __future__ import annotations

import hashlib
from pathlib import Path
from typing import Any

HERE = Path(__file__).resolve().parent
FIXTURE_DIR = HERE / "fixtures" / "scalar"
FIXED_INPUTS = (0, 1, 17, 32, 1024, 10000)
STAGES = ("prepare", "instantiate", "first_call", "warm_call", "load_and_call")
MODES = ("decoded", "compact")
PROFILE = "mvp"
STATIC_SOURCE_EVENTS = 22

FIXTURES = {
    "p128_identity_copy": {
        "file": "p128_identity_copy.wat",
        "shape": "identity_copy",
        "unit": "identity_local_copy",
        "wat_sha256": "690abafcb954c712285c0348eacd4d9b5167638a77829377402fd3583e7cc952",
        "wasm_sha256": "f5840979b2bea309cd41979df0e184adb799c72849a9030abcf8c54c8fe635f0",
    },
    "p128_literal_wrap_add": {
        "file": "p128_literal_wrap_add.wat",
        "shape": "literal_wrap_add",
        "unit": "literal_wrapping_i32_add",
        "wat_sha256": "7cb90f9723a0233241e9d067f378c1eef035b9d145ad52e5e414136b8095ca1e",
        "wasm_sha256": "d4c60b1f47c56dbcf1c4fbabeb496ad02544ad369f4c41e999a65ee51dee2e24",
    },
    "p128_literal_bitwise": {
        "file": "p128_literal_bitwise.wat",
        "shape": "literal_bitwise",
        "unit": "literal_i32_and",
        "wat_sha256": "094d32e7834b68b3a00d0575fe2c4b31c1bfcf82b3a135a9112c6c551f809c8c",
        "wasm_sha256": "847405edb388190cd07fa847124885df786d11317e95dfc1ba2dbb73294f8aa7",
    },
}
MASK32 = 0xFFFF_FFFF
LITERAL_ADD = 0x7FFF_FFFF + 1
LITERAL_AND = 0x1357_9BDF & MASK32


def _signed_i32(value: int) -> int:
    value &= MASK32
    return value - 0x1_0000_0000 if value & 0x8000_0000 else value


def _shape_for_alias(workload_id: str, n: int) -> str:
    if type(n) is not int or n not in FIXED_INPUTS:
        raise ValueError("n must be one of the six fixed P128 inputs")
    for family, metadata in FIXTURES.items():
        if workload_id == f"{family}_n{n}":
            return metadata["shape"]
    raise ValueError(f"unknown or malformed P128 workload alias: {workload_id!r}")


def oracle(workload_id: str, n: int) -> int:
    """Compute signed-i32 output independently from fixture execution."""
    shape = _shape_for_alias(workload_id, n)
    if shape == "identity_copy":
        return _signed_i32(n)
    if shape == "literal_wrap_add":
        return _signed_i32((n * LITERAL_ADD) & MASK32)
    if shape == "literal_bitwise":
        return _signed_i32((n * LITERAL_AND) & MASK32)
    raise AssertionError(f"unmapped admitted P128 shape: {shape}")


def source_event_count(n: int) -> int:
    """Return exact dynamic source events, including implicit function end."""
    if type(n) is not int or n not in FIXED_INPUTS:
        raise ValueError("n must be one of the six fixed P128 inputs")
    return 9 + 14 * n


def _verify_fixture_bytes(family: str, metadata: dict[str, str]) -> None:
    wat = FIXTURE_DIR / metadata["file"]
    if hashlib.sha256(wat.read_bytes()).hexdigest() != metadata["wat_sha256"]:
        raise ValueError(f"admitted WAT hash mismatch for {family}")


def entries() -> list[dict[str, Any]]:
    """Return the exact 18 fixed model rows; no open-ended aliases are accepted."""
    rows: list[dict[str, Any]] = []
    for family, metadata in FIXTURES.items():
        _verify_fixture_bytes(family, metadata)
        for n in FIXED_INPUTS:
            workload_id = f"{family}_n{n}"
            rows.append({
                "id": workload_id,
                "family": family,
                "source": {
                    "kind": "wat_file",
                    "path": f"fixtures/scalar/{metadata['file']}",
                    "sha256": metadata["wat_sha256"],
                    "wasm_sha256": metadata["wasm_sha256"],
                    "encoding_status": "encoded",
                },
                "params": [{"name": "n", "type": "i32", "min": n, "max": n, "baseline": n}],
                "result": {"type": "i32"},
                "oracle": {"kind": "python", "module": "scalar_p128_workloads.py", "function": "oracle", "independent": True},
                "oracle_arguments": {"n": n},
                "required_features": ["mvp"],
                "profile": PROFILE,
                "work_units": {"count": n, "unit": metadata["unit"]},
                "work_opportunities_per_loop": 1,
                "source_event_model": {"static": STATIC_SOURCE_EVENTS, "dynamic": source_event_count(n)},
                "reset_policy": "fresh_instance_for_cold_stages_reused_instance_for_warm_call",
                "stages": list(STAGES),
                "modes": list(MODES),
                "expected_i32": oracle(workload_id, n),
                "validation_role": "foundation_model_only",
            })
    return rows


