"""Strict validation and deterministic schedules for P141 entry-sequence evidence."""

from __future__ import annotations

from typing import Any


SCHEMA = "wtime-entry-sequence-v1"
ACTIVATION_SCHEMA = "wtime-entry-sequence-activation-v1"
ACTIVATION_DRIVER = "entry_sequence_activation"
ACTIVATION_CAPACITY_BASIS = "slots_capacity_u64_plus_labels_capacity"
MAX_U128 = (1 << 128) - 1
MAX_U64 = (1 << 64) - 1
MAX_RESIDENT_BYTES = 1_048_576
MAX_ACTIVATION_BYTES = 2 * 1_048_576
ACTIVATION_ROW_FIELDS = {
    "activation_peak_bytes", "activation_peak_frames", "activation_current_bytes",
    "activation_current_frames", "activation_active_scopes", "activation_capacity_samples",
    "activation_overflowed",
}


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


def validate_activation_payload(
    payload: Any,
    schedule_threshold: int,
    strategy: str,
    threshold: int,
    input: int,
) -> dict[str, Any]:
    """Validate the activation-capacity extension to P141's diagnostic wire format."""
    import copy

    if not isinstance(payload, dict):
        raise ValueError("payload must be an object")
    if payload.get("schema") != ACTIVATION_SCHEMA or payload.get("driver") != ACTIVATION_DRIVER:
        raise ValueError("payload is not an activation diagnostic")
    if payload.get("activation_capacity_basis") != ACTIVATION_CAPACITY_BASIS:
        raise ValueError("invalid activation_capacity_basis")
    rows = payload.get("per_call")
    if not isinstance(rows, list):
        raise ValueError("per_call must be a list")

    normalized = copy.deepcopy(payload)
    normalized["schema"] = SCHEMA
    normalized["driver"] = "entry_sequence"
    del normalized["activation_capacity_basis"]
    hot_peak_bytes: int | None = None
    for index, row in enumerate(normalized["per_call"]):
        if not isinstance(row, dict):
            raise ValueError(f"per_call row {index} must be an object")
        missing = ACTIVATION_ROW_FIELDS - set(row)
        if missing:
            raise ValueError(f"per_call row {index} is missing activation fields")
        for name in ACTIVATION_ROW_FIELDS - {"activation_overflowed"}:
            value = row[name]
            if type(value) is not int or not 0 <= value <= MAX_U64:
                raise ValueError(f"per_call[{index}].{name} must be a u64 integer")
        if type(row["activation_overflowed"]) is not bool or row["activation_overflowed"]:
            raise ValueError(f"per_call row {index} reports activation overflow")
        if row["activation_current_bytes"] != 0 or row["activation_current_frames"] != 0 or row["activation_active_scopes"] != 0:
            raise ValueError(f"per_call row {index} activation scopes were not reset")
        if row["activation_capacity_samples"] == 0:
            raise ValueError(f"per_call row {index} has no activation capacity samples")

        dispatches = row.get("register_dispatches")
        if type(dispatches) is not int:
            raise ValueError(f"per_call row {index} has invalid register_dispatches")
        peak_bytes = row["activation_peak_bytes"]
        peak_frames = row["activation_peak_frames"]
        if dispatches > 0:
            if peak_frames != 1 or not 0 < peak_bytes <= MAX_ACTIVATION_BYTES:
                raise ValueError(f"per_call row {index} has invalid hot activation peak")
            if hot_peak_bytes is None:
                hot_peak_bytes = peak_bytes
            elif peak_bytes != hot_peak_bytes:
                raise ValueError("hot activation peak bytes drift across uniform rows")
        elif peak_bytes != 0 or peak_frames != 0:
            raise ValueError(f"per_call row {index} has a nonzero cold activation peak")

        for name in ACTIVATION_ROW_FIELDS:
            del row[name]

    validate_payload(normalized, schedule_threshold, strategy, threshold, input, diagnostic=True)
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
