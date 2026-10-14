"""P113 workload metadata and independent full-state expectations."""
from __future__ import annotations

import hashlib
import math
from typing import Any

STAGES = ["prepare", "instantiate", "first_call", "warm_call", "load_and_call"]
INPUT = 32
PROFILE = "all-phase3-core"
MODE = "compact"
PAGE_BYTES = 65_536

PRIMARY = (
    "integration_gc_prefix",
    "integration_eh_prefix",
    "integration_cont_prefix",
)
PROTECTED = (
    "integration_numeric_control",
    "integration_ref_entry_control",
    "integration_cont_entry_control",
)
SELECTED = PRIMARY + PROTECTED
ROLES = {**{case: "primary" for case in PRIMARY},
         **{case: "protected" for case in PROTECTED}}
FILES = {
    "integration_gc_prefix": "integration_gc_prefix.wat",
    "integration_eh_prefix": "integration_eh_prefix.wat",
    "integration_cont_prefix": "integration_cont_prefix.wat",
    "integration_numeric_control": "integration_numeric_control.wat",
    "integration_ref_entry_control": "integration_ref_entry_control.wat",
    "integration_cont_entry_control": "integration_cont_entry_control.wat",
}
FEATURES = {
    "integration_gc_prefix": ["mvp", "gc"],
    "integration_eh_prefix": ["mvp", "exception_handling"],
    "integration_cont_prefix": ["mvp", "stack_switching"],
    "integration_numeric_control": ["mvp"],
    "integration_ref_entry_control": ["mvp", "gc"],
    "integration_cont_entry_control": ["mvp", "stack_switching"],
}
WORK_UNITS = {
    "integration_gc_prefix": (67, "numeric_prefix_source_instruction"),
    "integration_eh_prefix": (66, "numeric_prefix_source_instruction"),
    "integration_cont_prefix": (67, "numeric_prefix_source_instruction"),
    "integration_numeric_control": (65, "numeric_prefix_source_instruction"),
    "integration_ref_entry_control": (1, "reference_entry_instruction"),
    "integration_cont_entry_control": (1, "continuation_entry_instruction"),
}


def _signed_i32(value: int) -> int:
    value &= 0xFFFF_FFFF
    return value - 0x1_0000_0000 if value & 0x8000_0000 else value


def _memory_word(value: int | None = None) -> bytes:
    memory = bytearray(PAGE_BYTES)
    if value is not None:
        memory[:4] = (value & 0xFFFF_FFFF).to_bytes(4, "little")
    return bytes(memory)


def expected(case: str, n: int = INPUT) -> dict[str, Any]:
    """Return the complete normalized result and state oracle for one row."""
    if case not in ROLES:
        raise ValueError(f"unknown integration workload: {case}")
    if type(n) is not int or not -(1 << 31) <= n < (1 << 31):
        raise ValueError("integration input must be a signed i32")

    if case in {"integration_gc_prefix", "integration_eh_prefix"}:
        result = _signed_i32(n + 32)
        outcome: dict[str, Any] = {"kind": "returned", "values": [f"i32:{result}"]}
        payload: dict[str, Any] = {"values": []}
        memory = _memory_word(result)
        lifecycle = ({
            "gc_allocation_count": 1,
            "post_release_collection": {"live": 0, "reclaimed": 1},
        } if case == "integration_gc_prefix" else {})
    elif case == "integration_cont_prefix":
        result = "suspended"
        outcome = {"kind": "suspended"}
        payload = {"values": [{"type": "i32", "value": _signed_i32(n + 32)}]}
        memory = _memory_word(7)
        lifecycle = {
            "resume_input": "i32:7",
            "resume_result": {"kind": "returned", "values": ["i32:7"]},
            "second_resume": {"kind": "trap", "error_kind": "Trap"},
            "capture_bytes_minimum": 2,
            "after_resume_bytes_minimum": 1,
            "after_drop_and_collect_bytes": 0,
            "collection_after_root_release": {"live": 0, "reclaimed_minimum": 1},
        }
    elif case == "integration_numeric_control":
        result = _signed_i32(n + 32)
        outcome = {"kind": "returned", "values": [f"i32:{result}"]}
        payload = {"values": []}
        memory = _memory_word()
        lifecycle = {}
    elif case == "integration_ref_entry_control":
        result = 1
        outcome = {"kind": "returned", "values": [f"i32:{result}"]}
        payload = {"values": []}
        memory = _memory_word()
        lifecycle = {}
    else:  # integration_cont_entry_control
        result = "suspended"
        outcome = {"kind": "suspended"}
        payload = {"values": []}
        memory = _memory_word(7)
        lifecycle = {
            "resume_input": "i32:7",
            "resume_result": {"kind": "returned", "values": ["i32:7"]},
            "second_resume": {"kind": "trap", "error_kind": "Trap"},
            "capture_bytes_minimum": 2,
            "after_resume_bytes_minimum": 1,
            "after_drop_and_collect_bytes": 0,
            "collection_after_root_release": {"live": 0, "reclaimed_minimum": 1},
        }

    initial_memory = _memory_word()
    normalized_result = ("suspended" if result == "suspended"
                         else {"kind": "returned", "values": [f"i32:{result}"]})
    return {
        "case": case,
        "input": f"i32:{n}",
        "expected": normalized_result,
        "result": normalized_result,
        "outcome": outcome,
        "payload": payload,
        "memory_count": 1,
        "memory_pages": 1,
        "memory_hex": memory.hex(),
        "memory_sha256": hashlib.sha256(memory).hexdigest(),
        "initial_memory_hex": initial_memory.hex(),
        "initial_memory_sha256": hashlib.sha256(initial_memory).hexdigest(),
        "lifecycle": lifecycle,
    }


def entries() -> list[dict[str, Any]]:
    """Return six workload rows; the root integrator fills both fixture hashes."""
    result = []
    for case in SELECTED:
        expected_state = expected(case, INPUT)
        result.append({
            "id": case,
            "family": case,
            "role": ROLES[case],
            "source": {
                "kind": "wat_file",
                "path": f"fixtures/integration_p113/{FILES[case]}",
            },
            "params": [{"name": "n", "type": "i32", "min": INPUT,
                        "max": INPUT, "baseline": INPUT}],
            "result": {"type": "i32"},
            "oracle": {"kind": "python", "module": "integration_workloads.py",
                       "function": "expected", "independent": True},
            "oracle_arguments": {"n": INPUT},
            "expected_result": (7 if expected_state["result"] == "suspended"
                                else int(expected_state["result"]["values"][0].split(":")[1])),
            "expected_outcome": expected_state["outcome"],
            "expected_payload": expected_state["payload"],
            "required_features": FEATURES[case].copy(),
            "work_units": {"count": WORK_UNITS[case][0], "unit": WORK_UNITS[case][1]},
            "stages": STAGES.copy(),
            "reset_policy": "fresh_instance_for_cold_stages_reused_instance_for_warm_call",
            "comparators": {"wasmtime": (
                {"status": "probe_required", "feature_profile": ["mvp"]}
                if case == "integration_numeric_control" else
                {"status": "unsupported",
                 "reason": "The pinned Wasmtime benchmark build lacks the GC, exception-handling, or stack-switching feature required by this case."}
            )},
        })
    return result


def validate_observation(data: dict[str, Any], case: str, stage: str,
                         n: int = INPUT, *, expected_batch: int | None = None) -> dict[str, Any]:
    """Strictly validate one production row, including full state/lifecycle.

    Lifecycle values are emitted by the driver's untimed correctness path and
    must describe the fixture's complete checked invocation. The timing row
    itself remains uninstrumented and uses a one-call cold sample or the paired
    warm batch supplied by the comparison adapter.
    """
    if case not in ROLES:
        raise ValueError(f"unknown integration workload: {case}")
    if stage not in STAGES:
        raise ValueError(f"unknown integration stage: {stage}")
    oracle = expected(case, n)
    wanted = {
        "schema_version": 2,
        "case": case,
        "stage": stage,
        "input": oracle["input"],
        "expected": oracle["expected"],
        "result": oracle["result"],
        "mode": MODE,
        "profile": PROFILE,
        "instrumented": False,
        "diagnostic": False,
        "memory_count": oracle["memory_count"],
        "memory_pages": oracle["memory_pages"],
        "outcome": oracle["outcome"],
        "payload": oracle["payload"],
    }
    for key, value in wanted.items():
        if data.get(key) != value:
            raise ValueError(f"{case}/{stage}: {key} does not match independent expectation")
    memory = _validate_memory(data, oracle, case)
    _validate_lifecycle(data.get("lifecycle"), oracle["lifecycle"], case)
    elapsed = data.get("ns_per_operation")
    if isinstance(elapsed, bool) or not isinstance(elapsed, (int, float)) \
            or not math.isfinite(elapsed) or elapsed <= 0:
        raise ValueError(f"{case}/{stage}: timing must be finite and positive")
    batch = data.get("batch")
    if stage == "warm_call" and expected_batch is None:
        raise ValueError("warm_call validation requires the shared paired batch")
    required_batch = 1 if stage != "warm_call" else expected_batch
    if (isinstance(batch, bool) or not isinstance(batch, int) or batch < 1
            or batch > 100_000 or batch != required_batch):
        raise ValueError(f"{case}/{stage}: invalid batch")
    return {**oracle, "memory_bytes": len(memory),
            "memory_sha256": hashlib.sha256(memory).hexdigest()}


def validate_preflight(data: dict[str, Any], case: str, n: int = INPUT) -> dict[str, Any]:
    """Validate an untimed full-state check against the independent oracle."""
    oracle = expected(case, n)
    for key in ("case", "input", "expected", "result", "outcome", "payload",
                "memory_count", "memory_pages"):
        if data.get(key) != oracle[key]:
            raise ValueError(f"{case}: preflight {key} does not match independent expectation")
    if data.get("mode") != MODE or data.get("profile") != PROFILE:
        raise ValueError(f"{case}: preflight execution settings mismatch")
    if data.get("instrumented") is not False:
        raise ValueError(f"{case}: preflight must use the uninstrumented production harness")
    memory = _validate_memory(data, oracle, case)
    _validate_lifecycle(data.get("lifecycle"), oracle["lifecycle"], case)
    return {**oracle, "memory_bytes": len(memory),
            "memory_sha256": hashlib.sha256(memory).hexdigest()}


def _validate_memory(data: dict[str, Any], oracle: dict[str, Any], case: str) -> bytes:
    raw = data.get("memory_hex")
    if not isinstance(raw, str):
        raise ValueError(f"{case}: memory_hex must be present")
    try:
        memory = bytes.fromhex(raw)
    except ValueError as exc:
        raise ValueError(f"{case}: malformed memory_hex") from exc
    if len(memory) != PAGE_BYTES or memory.hex() != oracle["memory_hex"]:
        raise ValueError(f"{case}: full memory image does not match independent expectation")
    if data.get("memory_count") != 1 or data.get("memory_pages") != 1:
        raise ValueError(f"{case}: memory count/page count mismatch")
    wire_digest = data.get("memory_sha256")
    digest = hashlib.sha256(memory).hexdigest()
    if wire_digest is not None and wire_digest != digest:
        raise ValueError(f"{case}: supplied memory_sha256 mismatch")
    if case in {"integration_cont_prefix", "integration_cont_entry_control"}:
        initial_raw = data.get("initial_memory_hex")
        try:
            initial = bytes.fromhex(initial_raw) if isinstance(initial_raw, str) else b""
        except ValueError as exc:
            raise ValueError(f"{case}: malformed initial_memory_hex") from exc
        if len(initial) != PAGE_BYTES or initial.hex() != oracle["initial_memory_hex"]:
            raise ValueError(f"{case}: initial/captured memory image mismatch")
    return memory


def _validate_lifecycle(actual: Any, expected_lifecycle: dict[str, Any], case: str) -> None:
    if not isinstance(actual, dict):
        raise ValueError(f"{case}: lifecycle must be an object")
    if "capture_bytes_minimum" in expected_lifecycle:
        capture_bytes = actual.get("capture_bytes")
        if type(capture_bytes) is not int or capture_bytes < expected_lifecycle['capture_bytes_minimum']:
            raise ValueError(f"{case}: capture_bytes must be a positive integer")
        after_resume = actual.get('after_resume_bytes')
        if type(after_resume) is not int or not 1 <= after_resume < capture_bytes:
            raise ValueError(f"{case}: after_resume_bytes must retain a header but release the captured segment")
        for field in ("resume_input", "resume_result", "second_resume",
                      "after_drop_and_collect_bytes"):
            if actual.get(field) != expected_lifecycle[field]:
                raise ValueError(f"{case}: lifecycle {field} mismatch")
        collection = actual.get("collection_after_root_release")
        if not isinstance(collection, dict) or collection.get("live") != 0 \
                or type(collection.get("reclaimed")) is not int \
                or collection["reclaimed"] < expected_lifecycle["collection_after_root_release"]["reclaimed_minimum"]:
            raise ValueError(f"{case}: root-release collection must reclaim the consumed continuation")
    elif "gc_allocation_count" in expected_lifecycle:
        if actual.get("gc_allocation_count") != expected_lifecycle["gc_allocation_count"]:
            raise ValueError(f"{case}: GC allocation count mismatch")
        collection = actual.get("post_release_collection")
        if not isinstance(collection, dict) or collection != expected_lifecycle["post_release_collection"]:
            raise ValueError(f"{case}: post-release GC collection mismatch")
    elif actual != expected_lifecycle:
        raise ValueError(f"{case}: lifecycle does not match independent expectation")
