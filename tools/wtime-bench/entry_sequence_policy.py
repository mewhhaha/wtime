"""Strict validation and deterministic schedules for P141 entry-sequence evidence."""

from __future__ import annotations

from typing import Any


SCHEMA = "wtime-entry-sequence-v1"
MAX_U128 = (1 << 128) - 1
MAX_RESIDENT_BYTES = 1_048_576


def schedule(n: int) -> list[dict[str, Any]]:
    """Return the exact ordered 66-body initial pass followed by an f00 revisit."""
    _schedule_threshold(n)
    rows = [
        {"body": body, "visit": visit, "phase": "initial"}
        for body in range(66)
        for visit in range(1, n + 2)
    ]
    rows.extend(
        {"body": 0, "visit": visit, "phase": "revisit"}
        for visit in range(1, n + 2)
    )
    return rows


def event_class(row: dict[str, Any], n: int) -> str:
    """Classify the expected candidate schedule position, not observed cache behavior."""
    _schedule_threshold(n)
    if not isinstance(row, dict) or set(row) != {"body", "visit", "phase"}:
        raise ValueError("schedule row must have exactly body, visit, and phase")
    body, visit, phase = row["body"], row["visit"], row["phase"]
    if type(body) is not int or type(visit) is not int or not 0 <= body < 66:
        raise ValueError("invalid schedule row body or visit")
    if phase not in ("initial", "revisit") or (phase == "revisit" and body != 0):
        raise ValueError("invalid schedule row phase")
    if not 1 <= visit <= n + 1:
        raise ValueError("invalid schedule row visit")
    if visit < n:
        return "cold_initial" if phase == "initial" else "cold_revisit"
    if visit == n:
        return "compile_initial" if phase == "initial" else "recompile"
    return "warm_initial" if phase == "initial" else "warm_revisit"


def validate_payload(
    payload: Any,
    schedule_threshold: int,
    strategy: str,
    threshold: int,
    input: int,
    diagnostic: bool = False,
) -> dict[str, Any]:
    """Validate a P141 timing or diagnostic wire object and return it unchanged."""
    n = _schedule_threshold(schedule_threshold)
    if strategy not in ("eager", "adaptive"):
        raise ValueError("strategy must be eager or adaptive")
    if type(threshold) is not int or threshold < 0 or threshold > (1 << 64) - 1:
        raise ValueError("threshold must be a u64 integer")
    if (strategy == "eager" and threshold != 0) or (
        strategy == "adaptive" and threshold != n
    ):
        raise ValueError("strategy threshold does not match schedule")
    if type(input) is not int or not -(1 << 31) <= input < (1 << 31):
        raise ValueError("input must be an i32 integer")
    if type(diagnostic) is not bool:
        raise ValueError("diagnostic must be bool")
    if not isinstance(payload, dict):
        raise ValueError("payload must be an object")

    common = {
        "schema", "schema_version", "driver", "case", "mode", "feature_profile",
        "source_fuel", "input", "strategy", "threshold", "schedule_threshold",
        "requested_calls", "actual_calls", "instrumented", "diagnostic", "per_call",
    }
    timed_fields = {"prepare_ns", "instantiate_ns"}
    expected = common | (set() if diagnostic else timed_fields)
    if set(payload) != expected:
        raise ValueError("payload fields do not match the selected wire schema")
    actual_threshold = None if strategy == "eager" else threshold
    expected_header = {
        "schema": SCHEMA,
        "schema_version": 1,
        "driver": "entry_sequence",
        "case": "uniform66",
        "mode": "register",
        "feature_profile": "mvp",
        "source_fuel": None,
        "input": input,
        "strategy": strategy,
        "threshold": actual_threshold,
        "schedule_threshold": n,
        "requested_calls": 67 * (n + 1),
        "actual_calls": 67 * (n + 1),
        "instrumented": diagnostic,
        "diagnostic": diagnostic,
    }
    for key, value in expected_header.items():
        if type(payload[key]) is not type(value) or payload[key] != value:
            raise ValueError(f"invalid payload field {key}")
    if not isinstance(payload["per_call"], list) or len(payload["per_call"]) != 67 * (n + 1):
        raise ValueError("per_call length does not match requested calls")

    for index, (row, position) in enumerate(zip(payload["per_call"], schedule(n))):
        if not isinstance(row, dict):
            raise ValueError(f"per_call row {index} must be an object")
        if diagnostic:
            row_fields = {"body", "visit", "phase", "result", "register_dispatches", "fallback_frames"}
            if strategy == "adaptive":
                row_fields |= {
                    "cache_state", "entries", "builder_starts", "published", "evictions",
                    "resident_entries", "retained_bytes", "byte_limit",
                }
        else:
            row_fields = {"body", "visit", "phase", "result", "elapsed_ns"}
        if set(row) != row_fields:
            raise ValueError(f"per_call row {index} has unexpected fields")
        for name in ("body", "visit", "phase"):
            if type(row[name]) is not type(position[name]) or row[name] != position[name]:
                raise ValueError(f"per_call row {index} is out of schedule order")
        if type(row["result"]) is not int or row["result"] != _i32(input + position["body"]):
            raise ValueError(f"per_call row {index} has wrong oracle result")
        if not diagnostic:
            _u128(row["elapsed_ns"], f"per_call[{index}].elapsed_ns")
            continue
        _u128(row["register_dispatches"], f"per_call[{index}].register_dispatches")
        _u128(row["fallback_frames"], f"per_call[{index}].fallback_frames")
        if strategy == "eager":
            if row["register_dispatches"] != 4 or row["fallback_frames"] != 0:
                raise ValueError(f"eager diagnostic counters drift at row {index}")
            continue

        ready = position["visit"] >= n
        expected_state = "ready" if ready else "cold"
        expected_builders = (position["body"] if position["phase"] == "initial" else 66) + int(ready)
        expected_evictions = (
            max(position["body"] - 63, 0) if position["phase"] == "initial" else 3
        )
        expected_counters = {
            "cache_state": expected_state,
            "entries": position["visit"],
            "builder_starts": expected_builders,
            "published": expected_builders,
            "evictions": expected_evictions,
            "register_dispatches": 4 if ready else 0,
            "fallback_frames": 0 if ready else 1,
        }
        for key, value in expected_counters.items():
            if type(row[key]) is not type(value) or row[key] != value:
                raise ValueError(f"adaptive diagnostic {key} drift at row {index}")
        resident = row["resident_entries"]
        retained = row["retained_bytes"]
        limit = row["byte_limit"]
        for key, value in (("resident_entries", resident), ("retained_bytes", retained), ("byte_limit", limit)):
            _u128(value, f"per_call[{index}].{key}")
        expected_resident = min(position["body"] + 1, 64) if position["phase"] == "initial" else 64
        if resident != expected_resident or not 0 < retained <= limit <= MAX_RESIDENT_BYTES:
            raise ValueError(f"adaptive diagnostic cache bounds drift at row {index}")

    if not diagnostic:
        _u128(payload["prepare_ns"], "prepare_ns")
        _u128(payload["instantiate_ns"], "instantiate_ns")
    return payload


def _schedule_threshold(value: Any) -> int:
    if type(value) is not int or not 1 <= value <= 64:
        raise ValueError("schedule threshold must be an integer from 1 through 64")
    return value


def _u128(value: Any, name: str) -> int:
    if type(value) is not int or not 0 <= value <= MAX_U128:
        raise ValueError(f"{name} must be a u128 integer")
    return value


def _i32(value: int) -> int:
    return ((value + (1 << 31)) % (1 << 32)) - (1 << 31)
