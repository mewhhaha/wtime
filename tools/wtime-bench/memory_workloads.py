"""Deterministic memory workload metadata and independent byte-array oracles.

The byte-array model deliberately performs range validation before mutation for
bulk operations. It models guest-visible memory effects separately from the
WAT implementation so differential runners can compare both result and bytes.
"""

from __future__ import annotations

import hashlib
import subprocess
from pathlib import Path
from typing import Any

import p027_memory_workloads as _p027

HERE = Path(__file__).resolve().parent
FIXTURE_DIR = HERE / "fixtures" / "memory"
PAGE = 65536
SIZES = (64, 4096, 1 << 20)
RESET_POLICY = "fresh_instance_per_invocation"
STAGES = ["load_and_call"]

# These hashes bind the current source fixtures to the checked-in Wasm produced
# by the existing conformance encoder. encode_fixtures verifies them on every
# invocation; it never builds the encoder or mutates a fixture.
WASM_SHA256 = {
    "bulk_copy_aliased.wat": "62090599a827936312ab6e8aef30c8dd58950e5541925dc1f104cbcae631ea9f",
    "bulk_copy_backward.wat": "c471424ab8f70544c8910c159fb722762f50a69d8275672baadfdd4be610f24f",
    "bulk_copy_forward.wat": "fa79408dcd1e5a6de0b30efec318fef7af1b32f38fa91aa85e096bc89e8242dd",
    "bulk_fill.wat": "fce9add69d0ce7c47d47bf4df17d3af2a189bd0553f45dc49d26aec4d368bb91",
    "effective_address_overflow.wat": "670a1ec729e5580947ddc083bef8455df6d256918740102938ce4b79cc083ec9",
    "growth.wat": "077018a548b5a666bd3b909d844c7f620162312fa8032f927384ed5946766152",
    "memory64.wat": "760627846a98bffa0faf30cc8cb953b9eb72580a24eda9954d8da9d24ee8a3ff",
    "oob_order.wat": "44cfc8f92c7be0e77a8625cfbe2f1306007ff3e10e8e26aa551a4ee44037dabf",
    "widths.wat": "febd4bc1275c2db30f6d67c722a577c3e5fba64ee072c14175c5fb2f5769df74",
    "widths_shuffled.wat": "e948f8be21a6e70d46be7dd3a1a3d78d700ba7535a6220bf8d213b2392eab511",
    "read_scan.wat": "fa603f253df536b584a90272ab4bb9beb46cb4a0c8e51ca0a2ba3da3eda0ae18",
    "custom_page_one_byte.wat": "29dd399af1a1ba4bf59db09733986c819b5dfc072dce44b3a398adda316ba3b9",
    "multi_memory_second.wat": "577f2cbd04a7c384a896c859ef04cd02573120f375c4d6df5b15790be610a310",
}
WASM_SHA256.update(_p027.WASM_SHA256)

PARAM_CASES = {
    "memory_widths_predictable": ("widths.wat", "accesses", 64),
    "memory_widths_shuffled": ("widths_shuffled.wat", "accesses", 64),
    "memory64_i32_extended": ("memory64.wat", "address", PAGE - 1),
    "memory_growth_success_zero": ("growth.wat", "delta", 0),
    "memory_growth_success_one": ("growth.wat", "delta", 1),
    "memory_growth_success_max": ("growth.wat", "delta", 16),
    "memory_growth_failure": ("growth.wat", "delta", 17),
    "memory_oob_copy_order": ("oob_order.wat", "unused", 0),
    "memory_effective_address_overflow": ("effective_address_overflow.wat", "unused", 0),
    "memory_widths_read_scan": ("read_scan.wat", "bytes", 1024),
    "memory_custom_page_one_byte": ("custom_page_one_byte.wat", "address", 255),
    "memory_multi_memory_second_export": ("multi_memory_second.wat", "unused", 0),
}
WIDTH_ALIAS_COUNTS = (0, 1, 17, 32, 256, 1024)
for _count in WIDTH_ALIAS_COUNTS:
    PARAM_CASES[f"memory_widths_predictable_{_count}"] = (
        "widths.wat", "accesses", _count
    )
    PARAM_CASES[f"memory_widths_shuffled_{_count}"] = (
        "widths_shuffled.wat", "accesses", _count
    )
    PARAM_CASES[f"memory_widths_read_scan_{_count}"] = (
        "read_scan.wat", "bytes", _count
    )
for _direction in ("forward", "backward"):
    for _size in SIZES:
        PARAM_CASES[f"memory_copy_{_direction}_{_size}"] = (
            f"bulk_copy_{_direction}.wat", "bytes", _size
        )
for _size in SIZES:
    PARAM_CASES[f"memory_fill_{_size}"] = ("bulk_fill.wat", "bytes", _size)
for _size in SIZES:
    PARAM_CASES[f"memory_copy_aliased_{_size}"] = (
        "bulk_copy_aliased.wat", "bytes", _size
    )


def _signed32(value: int) -> int:
    value &= 0xFFFF_FFFF
    return value - 0x1_0000_0000 if value & 0x8000_0000 else value


def _u32(value: int) -> int:
    if type(value) is not int or not 0 <= value <= 0x7FFF_FFFF:
        raise ValueError("n must be a nonnegative signed i32")
    return value


def _workload_family(workload_id: str) -> str:
    return workload_id.rsplit("_", 1)[0] if workload_id.rsplit("_", 1)[-1].isdigit() else workload_id


def _access_base(index: int, shuffled: bool) -> int:
    slot = ((index * 251) & 255) if shuffled else index
    return slot * 32 + 1


def _little_signed(raw: bytes, signed: bool) -> int:
    return int.from_bytes(raw, "little", signed=signed)


def _read_scan_model(n: int) -> tuple[int, bytearray]:
    """Initialize deterministic bytes, then independently model a read-only scan."""
    n = _u32(n)
    if n > PAGE:
        raise ValueError("read scan exceeds one memory page")
    memory = bytearray(PAGE)
    for index in range(n):
        memory[index] = (index * 13 + 7) & 0xFF
    return _signed32(sum(memory[:n])), memory


def _custom_page_model(address: int) -> tuple[int | str, bytearray]:
    address = _u32(address)
    # `pagesize 1` means one-byte pages; 256 initial pages give 256 bytes.
    memory = bytearray(256)
    if address >= len(memory):
        return "trap", memory
    memory[address] = 0x80
    return -128, memory


def _multi_memory_model() -> tuple[int, bytearray, bytearray]:
    """Two distinct default-page memories; selected/exported memory is index 1."""
    first = bytearray(PAGE)
    second = bytearray(PAGE)
    first[:2] = b"\x11\x33"
    second[:2] = b"\x22\x44"
    return 0x11 + 0x22, first, second


def _widths_model(n: int, shuffled: bool) -> tuple[int, bytearray]:
    n = _u32(n)
    if not shuffled and n > 2048:
        raise ValueError("predictable width workload exceeds one memory page")
    memory = bytearray(PAGE)
    total = 0
    for i in range(n):
        base = _access_base(i, shuffled)
        memory[base] = 0xFE
        total += _little_signed(memory[base:base + 1], True)
        total += _little_signed(memory[base:base + 1], False)
        memory[base + 2:base + 4] = (32769).to_bytes(2, "little")
        total += _little_signed(memory[base + 2:base + 4], True)
        total += _little_signed(memory[base + 2:base + 4], False)
        memory[base + 4:base + 8] = (-2147483647).to_bytes(4, "little", signed=True)
        total += _little_signed(memory[base + 4:base + 8], True)
        memory[base + 8] = 0xFE
        total += _little_signed(memory[base + 8:base + 9], True)
        total += _little_signed(memory[base + 8:base + 9], False)
        memory[base + 10:base + 12] = (32769).to_bytes(2, "little")
        total += _little_signed(memory[base + 10:base + 12], True)
        total += _little_signed(memory[base + 10:base + 12], False)
        memory[base + 12:base + 16] = (-2147483647).to_bytes(4, "little", signed=True)
        total += _little_signed(memory[base + 12:base + 16], True)
        total += _little_signed(memory[base + 12:base + 16], False)
        memory[base + 16:base + 24] = (-9223372036854775807).to_bytes(8, "little", signed=True)
        total += _little_signed(memory[base + 16:base + 24], True)
    return _signed32(total), memory


def _bulk_copy_model(n: int, direction: str) -> tuple[int, bytearray]:
    n = _u32(n)
    memory = bytearray(17 * PAGE)
    for i in range(n + 1):
        memory[i] = (i * 29 + 7) & 0xFF
    if direction == "forward":
        dest, src = 1, 0
    elif direction == "backward":
        dest, src = 0, 1
    else:
        raise ValueError(f"unknown copy direction {direction!r}")
    _checked_memmove(memory, dest, src, n)
    return _signed32(sum(memory[:n + 1])), memory


def _checked_memmove(memory: bytearray, dest: int, src: int, length: int) -> None:
    """Validate both whole ranges before applying memmove semantics."""
    size = len(memory)
    if src < 0 or dest < 0 or length < 0 or src > size or dest > size:
        raise IndexError("out of bounds memory access")
    if length > size - src or length > size - dest:
        raise IndexError("out of bounds memory access")
    copied = bytes(memory[src:src + length])
    memory[dest:dest + length] = copied


def _oob_order_model() -> tuple[str, bytearray]:
    """Trap while preserving the earlier store and suppressing the later one."""
    memory = bytearray(PAGE)
    memory[:4] = b"\x01\x02\x03\x04"
    memory[8] = 170
    try:
        _checked_memmove(memory, 65534, 0, 4)
    except IndexError:
        return "trap", memory
    raise AssertionError("out-of-bounds copy unexpectedly succeeded")


def _prior_store_trap_model() -> tuple[str, bytearray]:
    memory = bytearray(PAGE)
    memory[8] = 170
    # Address 1 plus a maximum-u32 memarg offset is out of range.
    return "trap", memory


def oracle(workload_id: str, n: int) -> int | str:
    """Return the signed i32 or expected trap for one memory workload."""
    if workload_id.startswith(("memory_copy_distinct_seeded_", "memory_fill_seeded_")):
        return _p027.oracle(workload_id, n)
    family = _workload_family(workload_id)
    if family == "memory_widths_read_scan":
        return _read_scan_model(n)[0]
    if family == "memory_custom_page_one_byte":
        return _custom_page_model(n)[0]
    if family == "memory_multi_memory_second_export":
        return _multi_memory_model()[0]
    if family == "memory_widths_predictable":
        return _widths_model(n, False)[0]
    if family == "memory_widths_shuffled":
        return _widths_model(n, True)[0]
    if family == "memory64_i32_extended":
        _u32(n)
        if n >= PAGE:
            return "trap"
        return -128
    if family == "memory_growth_success_zero":
        _u32(n)
        return 1
    if family == "memory_growth_success_one":
        _u32(n)
        return 2
    if family == "memory_growth_success_max":
        _u32(n)
        return 17
    if family == "memory_growth_failure":
        _u32(n)
        return -1
    if family == "memory_oob_copy_order":
        return _oob_order_model()[0]
    if family == "memory_effective_address_overflow":
        return _prior_store_trap_model()[0]
    for direction in ("forward", "backward"):
        if family == f"memory_copy_{direction}" or family == "memory_copy_aliased":
            return _bulk_copy_model(n, "forward" if "aliased" in family else direction)[0]
    if family == "memory_fill":
        n = _u32(n)
        return _signed32(90 * n)
    raise ValueError(f"unknown memory workload id: {workload_id}")


def expected_memory(workload_id: str, n: int) -> dict[str, Any]:
    """Return independent host-memory expectations for a workload invocation.

    Bytes are hex encoded to keep metadata JSON-safe. ``range`` scopes the
    digest to bytes touched or intentionally observed by the workload; the
    separate ``initial_pages`` and ``full_range_precheck`` fields describe the
    guest memory contract without claiming an incomplete digest is whole-memory.
    """
    if workload_id.startswith(("memory_copy_distinct_seeded_", "memory_fill_seeded_")):
        return _p027.expected_memory(workload_id, n)
    family = _workload_family(workload_id)
    if family == "memory_widths_read_scan":
        result, memory = _read_scan_model(n)
        return {"memory_index": 0, "initial_pages": 1, "page_size_bytes": PAGE,
                "range": [0, n], "bytes_hex": memory[:n].hex(),
                "sha256": hashlib.sha256(memory[:n]).hexdigest(),
                "full_memory_sha256": hashlib.sha256(memory).hexdigest(),
                "result": result, "setup_in_timed_run": True,
                "scan_phase": "read_only"}
    if family == "memory_custom_page_one_byte":
        result, memory = _custom_page_model(n)
        return {"memory_index": 0, "initial_pages": 256, "page_size_bytes": 1,
                "range": [0, len(memory)], "bytes_hex": memory.hex(),
                "sha256": hashlib.sha256(memory).hexdigest(),
                "full_memory_sha256": hashlib.sha256(memory).hexdigest(),
                "result": result, "custom_page_size": True}
    if family == "memory_multi_memory_second_export":
        result, first, second = _multi_memory_model()
        return {"memory_index": 1, "initial_pages": 1, "page_size_bytes": PAGE,
                "range": [0, 2], "bytes_hex": second[:2].hex(),
                "sha256": hashlib.sha256(second[:2]).hexdigest(),
                "full_memory_sha256": hashlib.sha256(second).hexdigest(),
                "result": result, "all_memory_sha256": [
                    hashlib.sha256(first).hexdigest(), hashlib.sha256(second).hexdigest()],
                "memory_count": 2, "exported_memory_index": 1,
                "memory_exports": {"memory0": 0, "memory": 1}}
    if family.startswith("memory_widths_"):
        result, memory = _widths_model(n, family.endswith("shuffled"))
        end = max((_access_base(i, family.endswith("shuffled")) + 24 for i in range(n)), default=0)
        return {"memory_index": 0, "initial_pages": 1, "range": [0, end],
                "bytes_hex": memory[:end].hex(),
                "sha256": hashlib.sha256(memory[:end]).hexdigest(),
                "full_memory_sha256": hashlib.sha256(memory).hexdigest(), "result": result}
    if family == "memory64_i32_extended":
        n = _u32(n)
        if n >= PAGE:
            return {"memory_index": 0, "initial_pages": 1, "trap": "out_of_bounds",
                    "range": [0, PAGE], "bytes_hex": bytes(PAGE).hex(),
                    "full_memory_sha256": hashlib.sha256(bytes(PAGE)).hexdigest()}
        memory = bytearray(PAGE)
        memory[n] = 0x80
        return {"memory_index": 0, "initial_pages": 1, "range": [n, n + 1],
                "bytes_hex": "80", "sha256": hashlib.sha256(b"\x80").hexdigest(),
                "full_memory_sha256": hashlib.sha256(memory).hexdigest(),
                "result": -128, "address_extension": "i32.extend_i32_u"}
    if family.startswith("memory_copy_"):
        direction = "forward" if "aliased" in family else "backward" if "backward" in family else "forward"
        result, memory = _bulk_copy_model(n, direction)
        end = n + 1
        return {"memory_index": 0, "initial_pages": 17, "range": [0, end],
                "bytes_hex": memory[:end].hex(),
                "sha256": hashlib.sha256(memory[:end]).hexdigest(), "result": result,
                "full_memory_sha256": hashlib.sha256(memory).hexdigest(),
                "overlap": direction, "alias_memories": "aliased" in family,
                "full_range_precheck": True}
    if family == "memory_fill":
        n = _u32(n)
        result = _signed32(90 * n)
        memory = bytearray(17 * PAGE)
        memory[32:32 + n] = bytes([90]) * n
        return {"memory_index": 0, "initial_pages": 17, "range": [32, 32 + n],
                "bytes_hex": (bytes([90]) * n).hex(),
                "sha256": hashlib.sha256(bytes([90]) * n).hexdigest(),
                "full_memory_sha256": hashlib.sha256(memory).hexdigest(),
                "result": result, "full_range_precheck": True}
    if family.startswith("memory_growth_"):
        delta = _u32(n)
        failed = family == "memory_growth_failure"
        pages = 1 if failed else 1 + delta
        return {"memory_index": 0, "initial_pages": 1, "pages_after": pages,
                "result": -1 if failed else pages,
                "full_memory_sha256": hashlib.sha256(bytes(pages * PAGE)).hexdigest(),
                "growth": "failure" if failed else "success"}
    if family == "memory_oob_copy_order":
        outcome, memory = _oob_order_model()
        return {"memory_index": 0, "initial_pages": 1, "trap": outcome,
                "range": [0, PAGE], "bytes_hex": memory.hex(),
                "sha256": hashlib.sha256(memory).hexdigest(),
                "full_memory_sha256": hashlib.sha256(memory).hexdigest(),
                "preserved_prior_store": [8, 170], "suppressed_later_store": [9, 204],
                "full_range_precheck": True}
    if family == "memory_effective_address_overflow":
        outcome, memory = _prior_store_trap_model()
        return {"memory_index": 0, "initial_pages": 1, "trap": outcome,
                "range": [0, PAGE], "bytes_hex": memory.hex(),
                "sha256": hashlib.sha256(memory).hexdigest(),
                "full_memory_sha256": hashlib.sha256(memory).hexdigest(),
                "preserved_prior_store": [8, 170], "suppressed_later_store": [9, 204],
                "effective_address": {"base": 1, "offset": 0xFFFF_FFFF},
                "full_range_precheck": True}
    raise ValueError(f"unknown memory workload id: {workload_id}")


def _required_feature(filename: str) -> list[str]:
    features = ["mvp"]
    if filename.startswith("bulk_") or filename == "oob_order.wat":
        features.append("bulk_memory")
    if filename == "memory64.wat":
        features.append("memory64")
    if filename == "custom_page_one_byte.wat":
        features.append("custom_page_sizes")
    if filename == "multi_memory_second.wat":
        features.append("multi_memory")
    return features


def entries() -> list[dict[str, Any]]:
    """Return schema-v2 entries for memory widths, bulk operations and growth."""
    entries: list[dict[str, Any]] = []
    for wid, (filename, param, value) in PARAM_CASES.items():
        wat = FIXTURE_DIR / filename
        source_hash = hashlib.sha256(wat.read_bytes()).hexdigest()
        family = _workload_family(wid)
        is_trap = family in {"memory_oob_copy_order", "memory_effective_address_overflow"}
        features = _required_feature(filename)
        if is_trap:
            units = {"count": 1, "unit": "trap_probe"}
        elif "copy" in family or "fill" in family:
            units = {"count": value, "unit": "bytes"}
        elif family == "memory_widths_read_scan":
            units = {"count": value * 2, "unit": "memory_operations"}
        elif "widths" in family:
            units = {"count": value * 19, "unit": "memory_operations"}
        elif family == "memory_custom_page_one_byte":
            units = {"count": 2, "unit": "memory_operations"}
        elif family == "memory_multi_memory_second_export":
            units = {"count": 4, "unit": "memory_operations"}
        elif "growth" in family:
            units = {"count": 1, "unit": "grow_attempts"}
        else:
            units = {"count": 1, "unit": "memory_accesses"}
        expectation = expected_memory(wid, value)
        if "bytes_hex" in expectation:
            # Keep workload manifests compact. Runners resolve this reference
            # to expected_memory() and compare actual byte ranges directly.
            expectation.pop("bytes_hex")
            expectation["bytes_oracle"] = "expected_memory"
        entries.append({
            "id": wid,
            "family": family,
            "source": {"kind": "wat_file", "path": f"fixtures/memory/{filename}",
                       "sha256": source_hash, "wasm_sha256": WASM_SHA256[filename],
                       "encoding_status": "encoded"},
            "params": [{"name": param, "type": "i32", "min": value, "max": value,
                        "baseline": value}],
            "result": {"type": "none" if is_trap else "i32"},
            "oracle": {"kind": "python", "module": "memory_workloads.py",
                       "function": "oracle", "independent": True},
            "required_features": features,
            "work_units": units,
            "reset_policy": ("fresh_instance_for_cold_stages_reused_instance_for_warm_call"
                             if family in {"memory_widths_predictable", "memory_widths_shuffled", "memory_widths_read_scan"}
                             else RESET_POLICY),
            "stages": (["prepare", "instantiate", "first_call", "warm_call", "load_and_call"]
                       if family in {"memory_widths_predictable", "memory_widths_shuffled", "memory_widths_read_scan"}
                       else STAGES.copy()),
            "comparators": {"wasmtime": {"status": "probe_required",
                            "feature_profile": features.copy()}},
            "memory_expectation": expectation,
            "expected_outcome": "trap" if is_trap else "return",
        })
    additions = _p027.entries()
    for row in additions:
        row["oracle"]["module"] = "memory_workloads.py"
        row["reset_policy"] = "fresh_instance_for_cold_stages_reused_instance_for_warm_call"
    return entries + additions


def encode_fixtures(encoder: str | Path, targetdir: str | Path) -> dict[str, str]:
    """Encode with an existing wtime-conformance binary and verify frozen hashes."""
    target = Path(targetdir)
    target.mkdir(parents=True, exist_ok=True)
    hashes: dict[str, str] = {}
    for filename, expected_hash in WASM_SHA256.items():
        source = FIXTURE_DIR / filename
        output = target / f"{source.stem}.wasm"
        subprocess.run([str(encoder), "encode", str(source), "-o", str(output)],
                       check=True, cwd=HERE.parent.parent)
        digest = hashlib.sha256(output.read_bytes()).hexdigest()
        if digest != expected_hash:
            raise ValueError(f"encoded fixture identity mismatch for {filename}: {digest}")
        hashes[filename] = digest
    return hashes
