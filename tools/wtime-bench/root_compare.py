"""Strict, subprocess-free validation for the P125 GC root stage driver."""
from __future__ import annotations

import copy
import math
from typing import Any

import root_workloads

PROFILE = "all-phase3-core"
MODE = "decoded"
STAGES = tuple(root_workloads.STAGES)
MEASUREMENT_POLICY = "sum_guest_invoke_intervals_excluding_each_oracle_and_gc_reset"


class RootComparisonError(ValueError):
    pass


def _fail(entry: dict[str, Any], message: str) -> RootComparisonError:
    return RootComparisonError(f"{entry.get('id', 'unknown')}: {message}")


def _int(value: Any, name: str, entry: dict[str, Any]) -> int:
    if type(value) is not int:
        raise _fail(entry, f"{name} must be a JSON integer")
    return value


def _positive(value: Any, name: str, entry: dict[str, Any]) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise _fail(entry, f"{name} must be numeric")
    if not math.isfinite(value) or value <= 0:
        raise _fail(entry, f"{name} must be finite and positive")
    return float(value)


def _close(actual: float, expected: float, name: str, entry: dict[str, Any]) -> None:
    if not math.isclose(actual, expected, rel_tol=1e-12, abs_tol=1e-6):
        raise _fail(entry, f"{name} differs from its declared timing interval")


def validate_observation(entry: dict[str, Any], wire: dict[str, Any], expected_batch: int,
                         *, expected_mode: str = MODE,
                         expected_stage: str | None = None) -> dict[str, Any]:
    """Validate and normalize one complete ``gc_root_stages`` wire object."""
    if not isinstance(entry, dict) or not isinstance(wire, dict):
        raise RootComparisonError("entry and root driver observation must be objects")
    workload = entry.get("id", "unknown")
    try:
        n = entry["params"][0]["baseline"]
        if type(n) is not int:
            raise ValueError("model input is not an integer")
        model_row = next(row for row in root_workloads.entries() if row["id"] == workload)
        if entry != model_row:
            raise ValueError("workload entry differs from the independent root model")
        expected = root_workloads.oracle(workload, n)
        family = entry["family"]
        model = root_workloads.FAMILIES[family]
        expectation = entry["gc_expectation"]
        cap = root_workloads.max_batch(model["bank"], n)
    except Exception as exc:
        raise _fail(entry, f"unknown or inconsistent root workload model: {exc}") from exc

    stage = expected_stage if expected_stage is not None else wire.get("stage")
    batch = expected_batch
    if type(batch) is not int or batch < 1 or batch > cap:
        raise _fail(entry, "batch is outside the model-derived root driver cap")
    if stage not in STAGES:
        raise _fail(entry, "stage is not a supported root stage")
    if stage != "warm_call" and batch != 1:
        raise _fail(entry, "cold stage batch must be one")
    if expected_mode != MODE:
        raise _fail(entry, "root driver mode must be decoded")

    expected_wire = {
        "schema_version": 2, "driver": "gc_root_stages", "case": workload,
        "family": family, "input": n, "mode": expected_mode, "profile": PROFILE,
        "stage": stage, "batch": batch, "max_batch": cap, "instrumented": False,
        "measurement_policy": MEASUREMENT_POLICY, "report_complete": True,
        "shape": expectation["shape"], "bank_slots": expectation["bank_slots"],
        "result": f"i32:{expected}",
    }
    wire_fields = set(expected_wire) | {
        "sample_ns", "prepare_ns", "instantiate_ns", "first_call_ns",
        "measured_guest_ns", "reset_ns", "calls_checked", "total_reclaimed",
        "reclaimed_per_call", "live_after_reset", "payload_bytes_after_reset",
        "continuation_bytes_after_reset", "callbacks_checked", "nested_calls_checked",
        "callback_live", "callback_reclaimed", "callback_payload_min",
        "callback_payload_max",
    }
    if set(wire) != wire_fields:
        raise _fail(entry, "driver wire schema has missing or unknown fields")
    for key, value in expected_wire.items():
        if type(value) is int:
            actual = _int(wire.get(key), key, entry)
        else:
            actual = wire.get(key)
        if actual != value or (type(value) is bool and type(actual) is not bool):
            raise _fail(entry, f"{key} differs from the model or requested observation")

    calls = batch + 1 if stage == "warm_call" else 1
    callbacks = calls * expectation["callbacks_per_call"]
    exact = {
        "calls_checked": calls, "total_reclaimed": calls,
        "reclaimed_per_call": 1, "live_after_reset": 0,
        "payload_bytes_after_reset": 0, "continuation_bytes_after_reset": 0,
        "callbacks_checked": callbacks, "nested_calls_checked": callbacks,
        "callback_live": 1 if callbacks else 0, "callback_reclaimed": 0,
        "callback_payload_min": None,
        "callback_payload_max": None,
    }
    for key, value in exact.items():
        actual = _int(wire.get(key), key, entry)
        if value is not None and actual != value:
            raise _fail(entry, f"{key} differs from the complete GC/callback proof")
    payload_min = wire["callback_payload_min"]
    payload_max = wire["callback_payload_max"]
    if callbacks:
        if payload_min <= 0 or payload_min != payload_max:
            raise _fail(entry, "callback payload bounds must match and be positive")
    elif payload_min != 0 or payload_max != 0:
        raise _fail(entry, "callback payload bounds must be zero when there are no callbacks")

    for field in ("sample_ns", "prepare_ns", "instantiate_ns", "first_call_ns",
                  "measured_guest_ns", "reset_ns"):
        _positive(wire.get(field), field, entry)
    measured = float(wire["measured_guest_ns"])
    intervals = {
        "prepare": float(wire["prepare_ns"]),
        "instantiate": float(wire["instantiate_ns"]),
        "first_call": float(wire["first_call_ns"]),
        "warm_call": measured / batch,
        "load_and_call": float(wire["prepare_ns"]) + float(wire["instantiate_ns"]) + float(wire["first_call_ns"]),
    }
    _close(float(wire["sample_ns"]), intervals[stage], "sample_ns", entry)
    _close(measured, measured if stage == "warm_call" else float(wire["first_call_ns"]),
           "measured_guest_ns", entry)
    retained = copy.deepcopy(wire)
    return {"case": workload, "stage": stage, "batch": batch,
            "ns_per_operation": float(wire["sample_ns"]), "result": expected,
            "input": n, "mode": expected_mode, "instrumented": False,
            "driver_result": retained}


def revalidate_observation(entry: dict[str, Any], observation: dict[str, Any]) -> dict[str, Any]:
    """Revalidate retained full wire evidence and require normalized equality."""
    if not isinstance(observation, dict) or not isinstance(observation.get("driver_result"), dict):
        raise _fail(entry, "retained observation is missing its full driver_result")
    wire = observation["driver_result"]
    normalized = validate_observation(entry, wire, wire.get("batch"),
                                      expected_mode=observation.get("mode", MODE),
                                      expected_stage=observation.get("stage"))
    if normalized != observation:
        raise _fail(entry, "retained normalized observation differs from its complete driver proof")
    return normalized
