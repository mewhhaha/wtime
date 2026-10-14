"""Portable SIMD fixtures, independent lane oracles, and schema-v2 entries."""

from __future__ import annotations

import hashlib
import random
import subprocess
from pathlib import Path
from typing import Any

HERE = Path(__file__).resolve().parent
FIXTURE_DIR = HERE / "fixtures" / "simd"
INPUTS = (0, 17, 1000)
STAGES = ["prepare", "instantiate", "first_call", "warm_call", "load_and_call"]
RESET_POLICY = "fresh_instance_for_cold_stages_reused_instance_for_warm_call"
LOOP_FAMILIES = {
    "simd_integer_lanes": ("integer_lanes.wat", "i32x4_lane_op"),
    "simd_scalar_integer_lanes": ("scalar_integer_lanes.wat", "scalar_i32_lane_op"),
    "simd_saturating_select": ("saturating_select.wat", "i8x16_saturating_lane_op"),
    "simd_lane_memory": ("lane_memory.wat", "16_byte_vector_memory_roundtrip"),
    "simd_shuffle": ("shuffle.wat", "i8x16_shuffle"),
    "simd_scalar_shuffle": ("scalar_shuffle.wat", "scalar_i8_lane_xor"),
    "simd_float_arithmetic": ("float_arithmetic.wat", "f32x4_add_mul"),
    "simd_scalar_float_arithmetic": ("scalar_float_arithmetic.wat", "scalar_f32_add_mul"),
    "simd_f64_arithmetic": ("f64_arithmetic.wat", "f64x2_add_mul"),
    "simd_scalar_f64_arithmetic": ("scalar_f64_arithmetic.wat", "scalar_f64_add_mul"),
    "simd_relaxed_swizzle": ("relaxed_swizzle.wat", "i8x16_relaxed_swizzle"),
}
FLOAT_CASES = {
    "simd_float_min_zero": (0, -0x80000000),
    "simd_float_max_zero": (1, 0),
    "simd_float_nan_classify": (2, 1),
    "simd_float_convert_pos_inf": (3, 0x7FFFFFFF),
    "simd_float_convert_nan": (4, 0),
}
TYPED_CASES = {
    "simd_typed_f64_neg_1_75": {
        "file": "typed_f64.wat", "type": "f64_bits", "param": "x",
        "value": 0xBFFC000000000000, "expected": -1,
    },
    "simd_typed_f64_pos_3_5": {
        "file": "typed_f64.wat", "type": "f64_bits", "param": "x",
        "value": 0x400C000000000000, "expected": 3,
    },
    "simd_typed_v128_lanes": {
        "file": "typed_v128.wat", "type": "v128_bits", "param": "v",
        "value": "01000000feffffff0300000004000000", "expected": 6,
    },
}
TYPED_F64_MINMAX_CASES = {
    "simd_f64_min_negative_zero": (0x8000000000000000, 1),
    "simd_f64_min_positive_zero": (0x0000000000000000, 0),
    "simd_f64_min_nan": (0x7FF8000000000000, -1),
    "simd_f64_min_positive_value": (0x4004000000000000, 0),
}
WASM_SHA256: dict[str, str | None] = {
    "float_edges.wat": "6d21067a0c105455857d2d0c46b6b142e98f0794d2f1870da0ea5f467768a506",
    "f64_minmax.wat": "69ed757b2f76f1605f3b4c9332d995887848a44406d99b929df38676c4486b77",
    "float_arithmetic.wat": "ece84319c9f3b540e99b4ddaf2585f778dda6cd8735d11b27f7ba35e6d5fbef0",
    "integer_lanes.wat": "95ad86610276d9db5ca8ea3d6426d087fca8a702225606139e79673de60003a7",
    "integer_lanes_typed.wat": "3a690996697e34089e18a5e9883ce551f6d600dece89e70580fd601ae2a82150",
    "lane_memory.wat": "9c957181e2ca5d74e9a3f9fec4e4cf5fb22e3eea231ccbc4c826c5c8847bfa01",
    "relaxed_swizzle.wat": "2017b5e0e15c1dd4406fc76832f92bcab1ea1f32187b1ca0dcc4c80e7aa121da",
    "relaxed_swizzle_oob.wat": "0f79b71c6396e4e7e9c3f60828edfd1e8b185d83e65b2a1466292d29bb16dc3b",
    "scalar_float_arithmetic.wat": "9a866ed614814bc74a36aad06d6adefc51cd9bf914c8cd7901ee19a13649ce92",
    "scalar_float_edges.wat": "621b5c278b5a3c706996d06a19cf12578c57dbd92e7015d57e4e8a384018405c",
    "scalar_integer_lanes.wat": "71f6ccd9b822a455d0248276eacdf61a61f39a2435000c0d3f12a9d3e3f5a15c",
    "scalar_integer_lanes_typed.wat": "45101f560f2675ae21c5b80b7e89974a3e0bff7814735bbf4609999ec2c44e19",
    "scalar_shuffle.wat": "f7a3e0e209078c5afac21d24285412d7206921427e73fae451e90fa2df8c485a",
    "saturating_select.wat": "22633c8544d3db06b0f47cef19966e357743e0db9a84815fc66d87b390a87bea",
    "shuffle.wat": "b71c56264d750cc8f7300e3b9900534072d92838447a2f600c920e20625f5f77",
    "typed_f64.wat": "8b91d640241294a1dee9cfbe8605287a0b4e3d5e215d0c9aea529f77ec6da841",
    "typed_v128.wat": "12f3541a902e0ea324af36e547d6af9ea2879720ccefb7e4fdb8b37b024b1a12",
    "f64_arithmetic.wat": "4b8677abfcf31c65bf72fb19952629c037e4903a8596a508f32af4ce38a25643",
    "scalar_f64_arithmetic.wat": "6f5956a62e16a5298cb1297d0691bfb26e50bc92243198413748b82580d2f5ac",
    "lane_memory_footprint.wat": "a92144289dae9ec957773fe0f5bfef50c2ef4d188f96338aff50a4cb55af5c2c",
}

RELAXED_OOB_CASES = {"simd_relaxed_swizzle_oob_17": ("relaxed_swizzle_oob.wat", 17)}
SCALAR_FLOAT_EDGE_CASES = {
    "simd_scalar_float_min_zero": (0, -0x80000000),
    "simd_scalar_float_max_zero": (1, 0),
    "simd_scalar_float_nan_classify": (2, 1),
    "simd_scalar_float_convert_pos_inf": (3, 0x7FFFFFFF),
    "simd_scalar_float_convert_nan": (4, 0),
}
FOOTPRINT_VECTOR_COUNTS = {
    "simd_lane_memory_footprint_16": 1,
    "simd_lane_memory_footprint_4096": 256,
    "simd_lane_memory_footprint_65536": 4096,
}


def randomized_lane_inputs() -> list[tuple[str, tuple[int, int, int, int]]]:
    rng = random.Random(0x5010)
    return [(f"{index:02d}", tuple(rng.randrange(-(1 << 31), 1 << 31)
                                    for _ in range(4)))
            for index in range(30)]


def _random_lane_result(lanes: tuple[int, int, int, int], n: int) -> int:
    values = [_u32(lane) for lane in lanes]
    additions = (1, _u32(-3), 5, 6)
    factors = (3, 5, 7, 9)
    masks = (324508639, _u32(-610839777), 1431655765, _u32(-1431655766))
    for _ in range(n):
        values = [(_u32((_u32(lane + add) * factor)) ^ mask) & 0xFFFF_FFFF
                  for lane, add, factor, mask in zip(values, additions, factors, masks)]
    return _reduce_i32_lanes([_signed(lane) for lane in values])


def _lane_bits(lanes: tuple[int, int, int, int]) -> str:
    return b"".join(_u32(lane).to_bytes(4, "little") for lane in lanes).hex()


def _u32(value: int) -> int:
    return value & 0xFFFF_FFFF


def _signed(value: int) -> int:
    value &= 0xFFFF_FFFF
    return value - 0x1_0000_0000 if value & 0x8000_0000 else value


def _reduce_i32_lanes(lanes: list[int]) -> int:
    total = 0
    for lane in lanes:
        total = _u32(total + lane)
    return _signed(total)


def _f32(value: float) -> float:
    import struct
    return struct.unpack("<f", struct.pack("<f", value))[0]


def _float_arithmetic(n: int) -> int:
    values = [1.25, -2.5, 3.75, -4.0]
    deltas = [0.5, -0.25, 0.125, 1.0]
    for _ in range(n):
        values = [_f32(_f32(value + delta) * 1.0)
                  for value, delta in zip(values, deltas)]
    return _reduce_i32_lanes([int(value) for value in values])


def _f64_arithmetic(n: int) -> int:
    # Binary fractions and multiplication by one keep every intermediate exact.
    a, b = 1.25, -2.5
    for _ in range(n):
        a = (a + 0.5) * 1.0
        b = (b - 0.25) * 1.0
    return _signed(int(a) + int(b))


def _lane_memory(n: int) -> tuple[int, bytes, bytes]:
    lanes = [_signed(_u32(base + increment * n))
             for base, increment in zip((1, 2, 3, 4), (1, 2, 3, 4))]
    raw = b"".join(_u32(lane).to_bytes(4, "little") for lane in lanes)
    full = raw + bytes(65536 - len(raw))
    return _reduce_i32_lanes(lanes), raw, full


def _footprint_memory(vector_count: int) -> tuple[int, bytes, bytes]:
    touched = b"".join(
        b"".join(value.to_bytes(4, "little") for value in (2, 4, 6, 8))
        for _ in range(vector_count)
    )
    full = touched + bytes(65536 - len(touched))
    return _signed(20 * vector_count), touched, full


def oracle(workload_id: str, value: int | None = None) -> int:
    """Compute the exact scalar checksum for a workload and its typed input."""
    if workload_id in FOOTPRINT_VECTOR_COUNTS:
        vectors = FOOTPRINT_VECTOR_COUNTS[workload_id]
        if value != vectors:
            raise ValueError(f"{workload_id} expects {vectors} vectors")
        return _footprint_memory(vectors)[0]
    for prefix in ("simd_integer_lanes_random_", "simd_scalar_integer_lanes_random_"):
        if workload_id.startswith(prefix):
            suffix = workload_id.removeprefix(prefix)
            lanes = dict(randomized_lane_inputs()).get(suffix)
            if lanes is None:
                raise ValueError(f"unknown randomized lane input: {workload_id}")
            return _random_lane_result(lanes, 17)
    for family in LOOP_FAMILIES:
        if workload_id == family or workload_id.startswith(family + "_n"):
            if type(value) is not int or not 0 <= value <= 100000:
                raise ValueError(f"{family} input must be an integer in 0..100000")
            n = value
            if family in {"simd_integer_lanes", "simd_scalar_integer_lanes"}:
                lanes = [0x7FFF_FFFF, _u32(-2), 3, 4]
                adds = [1, _u32(-3), 5, 6]
                multipliers = [3, 5, 7, 9]
                masks = [324508639, _u32(-610839777), 1431655765, _u32(-1431655766)]
                for _ in range(n):
                    lanes = [(_u32((_u32(lane + add) * factor)) ^ mask)
                             for lane, add, factor, mask in zip(lanes, adds, multipliers, masks)]
                return _reduce_i32_lanes([_signed(lane) for lane in lanes])
            if family == "simd_saturating_select":
                lanes = [120, -120, 10, -10, 127, -128, 1, -1, 3, 4, -5, -6, 50, -50, 0, 0]
                deltas = [20, -20, -20, 20, 1, -1, 2, -2, 8, 9, -10, -11, 90, -90, 127, -128]
                for _ in range(n):
                    updated = [max(-128, min(127, lane + delta)) for lane, delta in zip(lanes, deltas)]
                    lanes = [updated[index] if index % 2 == 0 else lane
                             for index, lane in enumerate(lanes)]
                return _reduce_i32_lanes(lanes)
            if family == "simd_lane_memory":
                return _lane_memory(n)[0]
            if family in {"simd_shuffle", "simd_scalar_shuffle"}:
                return 376 if n & 1 else 0
            if family in {"simd_float_arithmetic", "simd_scalar_float_arithmetic"}:
                return _float_arithmetic(n)
            if family in {"simd_f64_arithmetic", "simd_scalar_f64_arithmetic"}:
                return _f64_arithmetic(n)
            if family == "simd_relaxed_swizzle":
                # Every swizzle index is in 0..15, so the relaxed result is exact.
                return 120 if n & 1 else 0

    if workload_id in FLOAT_CASES:
        return FLOAT_CASES[workload_id][1]
    if workload_id in TYPED_CASES:
        return TYPED_CASES[workload_id]["expected"]
    if workload_id in TYPED_F64_MINMAX_CASES:
        return TYPED_F64_MINMAX_CASES[workload_id][1]
    if workload_id in SCALAR_FLOAT_EDGE_CASES:
        return SCALAR_FLOAT_EDGE_CASES[workload_id][1]
    if workload_id in RELAXED_OOB_CASES:
        return 0
    raise ValueError(f"unknown SIMD workload id: {workload_id}")


def allowed_results(workload_id: str, value: int | None = None) -> set[int]:
    """Return the independent allowed checksum set, including relaxed SIMD cases."""
    if workload_id in RELAXED_OOB_CASES:
        # The proposal allows one implementation-wide projection for out-of-range
        # indices 16..127: zero or the selected lane after masking to four bits.
        return {0, 16}
    if workload_id.startswith("simd_relaxed_swizzle"):
        return {oracle(workload_id, value)}
    if workload_id == "simd_float_nan_classify":
        # NaN payloads vary; the test observes the unordered comparison mask only.
        return {1}
    return {oracle(workload_id, value)}


def expected_memory(workload_id: str, n: int) -> dict[str, Any]:
    if workload_id in FOOTPRINT_VECTOR_COUNTS:
        vectors = FOOTPRINT_VECTOR_COUNTS[workload_id]
        if n != vectors:
            raise ValueError(f"{workload_id} expects {vectors} vectors, received {n}")
        result, touched, full = _footprint_memory(vectors)
        return {"memory_index": 0, "export": "memory", "range": [0, len(touched)],
                "sha256": hashlib.sha256(touched).hexdigest(),
                "full_memory_sha256": hashlib.sha256(full).hexdigest(),
                "page_count": 1, "page_size_bytes": 65536,
                "bytes_oracle": "expected_memory",
                "setup_policy": "initialize_and_update_inside_timed_run", "result": result}
    if workload_id != "simd_lane_memory" and not workload_id.startswith("simd_lane_memory_n"):
        raise ValueError(f"no memory oracle for SIMD workload: {workload_id}")
    result, touched, full = _lane_memory(n)
    return {"memory_index": 0, "export": "memory", "range": [0, 16],
            "sha256": hashlib.sha256(touched).hexdigest(),
            "full_memory_sha256": hashlib.sha256(full).hexdigest(),
            "page_count": 1, "page_size_bytes": 65536,
            "bytes_oracle": "expected_memory", "result": result}


def _base_entry(workload_id: str, filename: str, required: list[str],
                profile: list[str]) -> dict[str, Any]:
    path = FIXTURE_DIR / filename
    wasm_sha = WASM_SHA256[filename]
    entry = {
        "id": workload_id,
        "source": {
            "kind": "wat_file",
            "path": f"fixtures/simd/{filename}",
            "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
            "wasm_sha256": wasm_sha,
            "encoding_status": "encoded" if wasm_sha is not None else "pending",
        },
        "result": {"type": "i32"},
        "oracle": {
            "kind": "python",
            "module": "simd_workloads.py",
            "function": "allowed_results",
            "independent": True,
        },
        "required_features": required,
        "reset_policy": RESET_POLICY,
        "stages": STAGES.copy(),
        "comparators": {
            "wasmtime": {"status": "probe_required", "feature_profile": profile},
        },
    }
    if wasm_sha is None:
        entry["source"]["encoding_note"] = "awaiting existing conformance encoder after P023 timing freeze"
    return entry


def entries() -> list[dict[str, Any]]:
    """Return schema-v2 SIMD entries: scaled loops and fixed typed edge controls."""
    result: list[dict[str, Any]] = []
    for family, (filename, unit) in LOOP_FAMILIES.items():
        scalar_kernel = family in {
            "simd_scalar_integer_lanes", "simd_scalar_shuffle",
            "simd_scalar_float_arithmetic", "simd_scalar_f64_arithmetic",
        }
        required = ["mvp"] if scalar_kernel else ["mvp", "simd"]
        profile = ["mvp"] if scalar_kernel else ["simd"]
        if family in {"simd_scalar_float_arithmetic", "simd_scalar_f64_arithmetic"}:
            required.append("saturating_float_to_int")
            profile = ["saturating_float_to_int"]
        if family == "simd_relaxed_swizzle":
            required.append("relaxed_simd")
            profile.append("relaxed_simd")
        for n in INPUTS:
            wid = f"{family}_n{n}"
            entry = _base_entry(wid, filename, required.copy(), profile.copy())
            entry.update({
                "family": family,
                "params": [{"name": "n", "type": "i32", "min": n, "max": n, "baseline": n}],
                "work_units": {"count": n, "unit": unit},
                "oracle_arguments": {"n": n},
            })
            if family in {"simd_integer_lanes", "simd_scalar_integer_lanes"}:
                entry["work_units"] = {"count": n * 12,
                                        "unit": "lane_arithmetic_ops" if family == "simd_integer_lanes" else "scalar_arithmetic_ops"}
                entry["equivalent_to"] = (f"simd_scalar_integer_lanes_n{n}" if family == "simd_integer_lanes"
                                           else f"simd_integer_lanes_n{n}")
            elif family in {"simd_shuffle", "simd_scalar_shuffle"}:
                entry["work_units"] = {"count": n * 16,
                                        "unit": "lane_xor_ops" if family == "simd_shuffle" else "scalar_xor_ops"}
                entry["equivalent_to"] = (f"simd_scalar_shuffle_n{n}" if family == "simd_shuffle"
                                           else f"simd_shuffle_n{n}")
            elif family in {"simd_float_arithmetic", "simd_scalar_float_arithmetic"}:
                entry["work_units"] = {"count": n * 8,
                                        "unit": "lane_float_ops" if family == "simd_float_arithmetic" else "scalar_float_ops"}
                entry["equivalent_to"] = (f"simd_scalar_float_arithmetic_n{n}" if family == "simd_float_arithmetic"
                                           else f"simd_float_arithmetic_n{n}")
            elif family in {"simd_f64_arithmetic", "simd_scalar_f64_arithmetic"}:
                entry["work_units"] = {"count": n * 4,
                                        "unit": "lane_float_ops" if family == "simd_f64_arithmetic" else "scalar_float_ops"}
                entry["equivalent_to"] = (f"simd_scalar_f64_arithmetic_n{n}" if family == "simd_f64_arithmetic"
                                           else f"simd_f64_arithmetic_n{n}")
            if family == "simd_lane_memory":
                entry["memory_expectation"] = expected_memory(wid, n)
            if family == "simd_relaxed_swizzle":
                entry["allowed_result_contract"] = {
                    "kind": "exact_set",
                    "explanation": "All relaxed_swizzle indices are 0..15, so the relaxed result matches the strict swizzle exactly.",
                }
            result.append(entry)

    for workload_id, vectors in FOOTPRINT_VECTOR_COUNTS.items():
        entry = _base_entry(workload_id, "lane_memory_footprint.wat", ["mvp", "simd"], ["simd"])
        expectation = expected_memory(workload_id, vectors)
        entry.update({
            "family": "simd_lane_memory_footprint",
            "params": [{"name": "vectors", "type": "i32", "value": vectors}],
            "work_units": {"count": vectors, "unit": "vector_load_add_store_roundtrips"},
            "oracle_arguments": {"vectors": vectors},
            "expected_result": expectation["result"],
            "memory_expectation": expectation,
            "setup_policy": "memory_initialization_and_updates_are_inside_run_and_timing",
        })
        result.append(entry)

    for workload_id, (case, expected) in FLOAT_CASES.items():
        entry = _base_entry(workload_id, "float_edges.wat", ["mvp", "simd", "saturating_float_to_int"],
                            ["simd", "saturating_float_to_int"])
        entry.update({
            "params": [{"name": "case", "type": "i32", "min": case, "max": case, "baseline": case}],
            "work_units": {"count": 1, "unit": "strict_float_edge_control"},
            "oracle_arguments": {"case": case},
            "expected_result": expected,
            "allowed_result_contract": {
                "kind": "exact_set",
                "explanation": "NaN payload is not returned; the NaN case returns only the unordered comparison mask.",
            },
        })
        result.append(entry)

    for workload_id, case in TYPED_CASES.items():
        feature = ["mvp", "simd"] if case["type"] == "v128_bits" else ["mvp"]
        profile = ["simd"] if case["type"] == "v128_bits" else ["mvp"]
        entry = _base_entry(workload_id, case["file"], feature, profile)
        entry.update({
            "params": [{"name": case["param"], "type": case["type"], "value": case["value"]}],
            "work_units": {"count": 1, "unit": "typed_edge_control"},
            "oracle_arguments": {"value": case["value"]},
            "expected_result": case["expected"],
            "setup_policy": "typed input and expected result are prepared before timing",
        })
        result.append(entry)
    for workload_id, (value, expected) in TYPED_F64_MINMAX_CASES.items():
        entry = _base_entry(workload_id, "f64_minmax.wat", ["mvp"], ["mvp"])
        entry.update({
            "params": [{"name": "x", "type": "f64_bits", "value": value}],
            "work_units": {"count": 1, "unit": "strict_f64_minmax_edge_control"},
            "oracle_arguments": {"value": value},
            "expected_result": expected,
            "allowed_result_contract": {
                "kind": "exact_set",
                "explanation": "NaN is classified before extracting a result; signed-zero min is checked through its IEEE sign bit.",
            },
            "setup_policy": "typed input and expected result are prepared before timing",
        })
        result.append(entry)
    for workload_id, (case, expected) in SCALAR_FLOAT_EDGE_CASES.items():
        entry = _base_entry(workload_id, "scalar_float_edges.wat",
                            ["mvp", "saturating_float_to_int"], ["mvp-saturating"])
        entry.update({"family": "simd_scalar_float_edges",
                      "params": [{"name": "case", "type": "i32", "min": case,
                                  "max": case, "baseline": case}],
                      "work_units": {"count": 1, "unit": "strict_scalar_float_edge_control"},
                      "oracle_arguments": {"case": case}, "expected_result": expected,
                      "equivalent_to": f"simd_float_{['min_zero','max_zero','nan_classify','convert_pos_inf','convert_nan'][case]}"})
        result.append(entry)
    for workload_id, (filename, value) in RELAXED_OOB_CASES.items():
        entry = _base_entry(workload_id, filename, ["mvp", "simd", "relaxed_simd"],
                            ["simd-relaxed"])
        entry.update({"family": workload_id, "params": [{"name": "index", "type": "i32",
                      "min": value, "max": value, "baseline": value}],
                      "work_units": {"count": 16, "unit": "relaxed_swizzle_lanes"},
                      "oracle_arguments": {"index": value},
                      "expected_results": sorted(allowed_results(workload_id, value)),
                      "allowed_result_contract": {"kind": "exact_set",
                        "explanation": "For all 16 lanes, index 17 allows either zero or source lane 1 under the proposal's implementation-wide out-of-range projection."}})
        result.append(entry)
    for suffix, lanes in randomized_lane_inputs():
        expected = _random_lane_result(lanes, 17)
        vector_id = f"simd_integer_lanes_random_{suffix}"
        scalar_id = f"simd_scalar_integer_lanes_random_{suffix}"
        vector = _base_entry(vector_id, "integer_lanes_typed.wat", ["mvp", "simd"], ["simd"])
        vector.update({"family": "simd_integer_lanes_randomized",
                       "params": [{"name": "n", "type": "i32", "value": 17},
                                  {"name": "initial", "type": "v128_bits", "value": _lane_bits(lanes)}],
                       "work_units": {"count": 17 * 12, "unit": "lane_arithmetic_ops"},
                       "oracle_arguments": {"n": 17, "lanes": list(lanes)},
                       "expected_result": expected, "equivalent_to": scalar_id})
        result.append(vector)
        scalar = _base_entry(scalar_id, "scalar_integer_lanes_typed.wat", ["mvp"], ["mvp"])
        scalar.update({"family": "simd_scalar_integer_lanes_randomized",
                       "params": [{"name": "n", "type": "i32", "value": 17}]
                                  + [{"name": f"lane{i}", "type": "i32", "value": lane}
                                     for i, lane in enumerate(lanes)],
                       "work_units": {"count": 17 * 12, "unit": "scalar_arithmetic_ops"},
                       "oracle_arguments": {"n": 17, "lanes": list(lanes)},
                       "expected_result": expected, "equivalent_to": vector_id})
        result.append(scalar)
    return result


def encode_fixtures(encoder: str | Path, targetdir: str | Path) -> dict[str, str]:
    """Encode all unique SIMD WAT fixtures with an existing encoder executable."""
    target = Path(targetdir)
    target.mkdir(parents=True, exist_ok=True)
    hashes: dict[str, str] = {}
    for filename in sorted({name for name, _unit in LOOP_FAMILIES.values()} |
                           {"float_edges.wat", "scalar_float_edges.wat", "relaxed_swizzle_oob.wat"} |
                           {"lane_memory_footprint.wat"} |
                           {"integer_lanes_typed.wat", "scalar_integer_lanes_typed.wat"} |
                           {case["file"] for case in TYPED_CASES.values()} |
                           {"f64_minmax.wat"}):
        source = FIXTURE_DIR / filename
        output = target / (source.stem + ".wasm")
        subprocess.run([str(encoder), "encode", str(source), "-o", str(output)],
                       check=True, cwd=HERE.parent.parent)
        hashes[filename] = hashlib.sha256(output.read_bytes()).hexdigest()
    return hashes
