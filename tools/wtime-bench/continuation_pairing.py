"""Strict adapter for the legacy P012 continuation rows in focused_compare."""
from __future__ import annotations

import copy
import json
import math
from pathlib import Path
from typing import Any

import continuation_scaling
import continuation_workloads

MODE = continuation_scaling.MODE
PROFILE = continuation_scaling.PROFILE
BOUNDARY = "Store::gc_payload_bytes sampled outside named timed intervals"
_SUPPORTED = {
    "continuation_depth_0", "continuation_depth_4", "continuation_depth_32",
    "continuation_captured_reference", "continuation_bound_nested",
}
_DEPTH_KEYS = {
    "schema_version", "case", "driver", "mode", "profile", "instrumented",
    "measurement_boundary", "setup_ns", "capture_ns", "resume_ns", "payload",
    "result", "logical_frames", "bytes_per_captured_frame", "numeric_locals",
    "continuation_bytes", "gc_payload_bytes", "collection_with_root",
    "collection_after_drop",
}
_BIND_KEYS = {
    "schema_version", "case", "driver", "mode", "profile", "instrumented",
    "measurement_boundary", "setup_ns", "bind_ns", "resume_ns", "bind_result",
    "result", "worker_calls", "continuation_bytes", "gc_payload_bytes", "collection",
}
_DEPTH_CHECKPOINTS = ("before_capture", "after_capture", "after_root_transfer",
                      "after_resume", "after_drop_and_collect")
_BIND_CHECKPOINTS = ("before", "after_bind", "after_run", "after_collect")
_GC_DEPTH = _DEPTH_CHECKPOINTS
_GC_BIND = ("before_bind", "after_bind", "after_run", "after_collect")


class PairingError(ValueError):
    pass


def matches(entry: Any) -> bool:
    return (isinstance(entry, dict) and isinstance(entry.get("oracle"), dict)
            and entry["oracle"].get("function") == "continuation_workloads.py")


def _fail(entry: dict[str, Any], reason: str) -> PairingError:
    return PairingError(f"{entry.get('id', 'unknown')}: {reason}")


def _modeled(entry: dict[str, Any]) -> dict[str, Any]:
    rows = {row["id"]: row for row in continuation_workloads.workload_entries()}
    row = rows.get(entry.get("id"))
    if row is None:
        raise _fail(entry, "unsupported continuation workload")
    # Generic registry metadata may carry extra benchmark fields. Its source,
    # stages, feature set and semantic oracle projection must still agree.
    for key in ("schema_version", "id", "source", "stages", "required_features"):
        if entry.get(key) != row.get(key):
            raise _fail(entry, f"registry {key} differs from the independent continuation model")
    oracle = entry.get("oracle")
    if (not isinstance(oracle, dict) or oracle.get("kind") != "semantic"
            or oracle.get("function") != "continuation_workloads.py"):
        raise _fail(entry, "legacy continuation oracle identity is invalid")
    for key, value in row["oracle"].items():
        if oracle.get(key) != value:
            raise _fail(entry, f"registry oracle field {key} differs from the independent model")
    if entry.get("params") != []:
        raise _fail(entry, "paired continuation workloads have no numeric params")
    if entry.get("reset_policy") != "fresh_instance_per_invocation":
        raise _fail(entry, "paired continuation rows require a fresh instance")
    if entry.get("driver_protocol") != "continuation-v2":
        raise _fail(entry, "paired continuation rows require continuation-v2")
    if entry.get("result") != {"type": "none"}:
        raise _fail(entry, "lifecycle outcomes cannot be represented as a generic result")
    if entry.get("id") not in _SUPPORTED:
        raise _fail(entry, "continuation lifecycle is outside the paired adapter subset")
    expected_stages = ["setup", "bind", "resume", "accounting"] if entry["id"] == "continuation_bound_nested" else ["setup", "capture", "resume", "accounting"]
    if entry["stages"] != expected_stages:
        raise _fail(entry, "continuation stage declaration changed")
    return row


def _wire(values: list[dict[str, Any]]) -> list[str]:
    return [f"{value['type']}:{value['value']}" for value in values]


def expected(entry: dict[str, Any]) -> dict[str, Any]:
    model = _modeled(entry)
    oracle = model["oracle"]
    if entry["id"] == "continuation_bound_nested":
        return {"kind": "bind_then_resume",
                "bind": {"kind": "returned", "result": ["i32:0"]},
                "resume": {"kind": "returned", "result": _wire(oracle["expected_resume"]["result"])},
                "worker_calls": oracle["expected_worker_calls"]}
    result = {"kind": "suspend_then_resume",
              "suspension": {"kind": "suspended", "payload": _wire(oracle["expected_suspension"]["payload"])},
              "resume": {"kind": "returned", "result": _wire(oracle["expected_resume"]["result"])} }
    if "captured_reference" in oracle:
        result["captured_reference"] = copy.deepcopy(oracle["captured_reference"])
    return result


def argv(entry: dict[str, Any], wasm: str | Path, mode: str) -> list[str]:
    _modeled(entry)
    if mode != MODE:
        raise _fail(entry, "continuation driver supports decoded mode only")
    selector = "bind" if entry["id"] == "continuation_bound_nested" else "depth" if entry["id"].startswith("continuation_depth_") else "root"
    return ["--continuation-driver", str(wasm), entry["id"], MODE, PROFILE, selector]


def _integer_map(entry: dict[str, Any], row: Any, keys: tuple[str, ...], label: str,
                 *, positive_after: str | None = None) -> dict[str, int]:
    if not isinstance(row, dict) or set(row) != set(keys):
        raise _fail(entry, f"{label} checkpoints have the wrong schema")
    if any(type(row[key]) is not int or row[key] < 0 for key in keys):
        raise _fail(entry, f"{label} checkpoints must be nonnegative integers")
    if positive_after is not None and row[positive_after] <= 0:
        raise _fail(entry, f"{label} must show retained payload after capture/bind")
    return row


def _collection(entry: dict[str, Any], row: Any, *, live: int) -> dict[str, int]:
    if not isinstance(row, dict) or set(row) != {"live", "reclaimed"}:
        raise _fail(entry, "collection oracle has the wrong schema")
    if type(row["live"]) is not int or type(row["reclaimed"]) is not int or row["live"] != live or row["reclaimed"] < 0:
        raise _fail(entry, "collection live/reclaimed counts are inconsistent")
    return row


def _positive_time(entry: dict[str, Any], row: dict[str, Any], fields: tuple[str, ...]) -> dict[str, float]:
    result = {}
    for field in fields:
        value = row.get(field)
        if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value) or value <= 0:
            raise _fail(entry, f"{field} must be finite positive driver-reported nanoseconds")
        result[field] = float(value)
    return result


def validate_observation(entry: dict[str, Any], wire: dict[str, Any], batch: int,
                         *, expected_mode: str, expected_stage: str) -> dict[str, Any]:
    model = _modeled(entry)
    workload = entry["id"]
    bound = workload == "continuation_bound_nested"
    stages = {"setup": "setup_ns", "bind": "bind_ns", "resume": "resume_ns"} if bound else {"setup": "setup_ns", "capture": "capture_ns", "resume": "resume_ns"}
    if type(batch) is not int or batch != 1:
        raise _fail(entry, "continuation observations require fresh-process batch one")
    if expected_mode != MODE or expected_stage not in stages:
        raise _fail(entry, "requested mode/stage is not supported by the continuation driver")
    if not isinstance(wire, dict) or set(wire) != (_BIND_KEYS if bound else _DEPTH_KEYS):
        raise _fail(entry, "driver wire schema has missing or unknown fields")
    expected_driver = "bind" if bound else "depth" if workload.startswith("continuation_depth_") else "root"
    if (wire.get("schema_version") != 2 or wire.get("case") != workload
            or wire.get("driver") != expected_driver or wire.get("mode") != MODE
            or wire.get("profile") != PROFILE or wire.get("instrumented") is not False
            or wire.get("measurement_boundary") != BOUNDARY):
        raise _fail(entry, "driver identity, instrumentation, or measurement boundary differs")
    if bound:
        times = _positive_time(entry, wire, ("setup_ns", "bind_ns", "resume_ns"))
        cont = _integer_map(entry, wire["continuation_bytes"], _BIND_CHECKPOINTS, "continuation byte")
        gc = _integer_map(entry, wire["gc_payload_bytes"], _GC_BIND, "GC payload", positive_after="after_bind")
        if cont["before"] != 0 or cont["after_bind"] <= 0 or cont["after_collect"] != 0:
            raise _fail(entry, "bound continuation ownership checkpoints are inconsistent")
        if gc["before_bind"] != 0 or gc["after_collect"] != 0:
            raise _fail(entry, "bound GC payload checkpoints are inconsistent")
        if wire["bind_result"] != "i32:0" or wire["result"] != _wire(model["oracle"]["expected_resume"]["result"]):
            raise _fail(entry, "bind/resume lifecycle result differs from the independent oracle")
        if type(wire["worker_calls"]) is not int or wire["worker_calls"] != model["oracle"]["expected_worker_calls"]:
            raise _fail(entry, "bound worker call count differs from the oracle")
        collection = _collection(entry, wire["collection"], live=0)
        continuation_scaling._validate_bound(model, wire)
        semantic = expected(entry)
        auxiliary = {"continuation_bytes": cont, "gc_payload_bytes": gc, "collection": collection}
    else:
        times = _positive_time(entry, wire, ("setup_ns", "capture_ns", "resume_ns"))
        cont = _integer_map(entry, wire["continuation_bytes"], _DEPTH_CHECKPOINTS, "continuation byte", positive_after="after_capture")
        gc = _integer_map(entry, wire["gc_payload_bytes"], _GC_DEPTH, "GC payload", positive_after="after_capture")
        if (cont["before_capture"] != 0 or cont["after_root_transfer"] != cont["after_capture"]
                or cont["after_drop_and_collect"] != 0 or gc["before_capture"] != 0
                or gc["after_drop_and_collect"] != 0):
            raise _fail(entry, "capture/root-transfer/cleanup accounting is inconsistent")
        semantic = expected(entry)
        if wire["payload"] != semantic["suspension"]["payload"] or wire["result"] != semantic["resume"]["result"]:
            raise _fail(entry, "suspension payload or resumed result differs from the independent oracle")
        if workload.startswith("continuation_depth_"):
            depth = int(workload.rsplit("_", 1)[1])
            frames = continuation_workloads.depth_oracle(depth)["captured_logical_frames"]
            if type(wire["logical_frames"]) is not int or wire["logical_frames"] != frames or type(wire["numeric_locals"]) is not int or wire["numeric_locals"] != frames:
                raise _fail(entry, "captured frame/local count differs from the independent model")
            expected_bpf = cont["after_capture"] / frames
            if isinstance(wire["bytes_per_captured_frame"], bool) or not isinstance(wire["bytes_per_captured_frame"], (int, float)) or not math.isclose(wire["bytes_per_captured_frame"], expected_bpf, rel_tol=1e-12):
                raise _fail(entry, "bytes-per-frame report differs from captured continuation bytes")
        else:
            if wire["logical_frames"] != 1 or wire["numeric_locals"] != 1 or wire["bytes_per_captured_frame"] != cont["after_capture"]:
                raise _fail(entry, "captured-reference accounting dimensions differ")
        rooted_live = 2 if workload == "continuation_captured_reference" else 1
        rooted = _collection(entry, wire["collection_with_root"], live=rooted_live)
        dropped = _collection(entry, wire["collection_after_drop"], live=0)
        continuation_scaling._validate_capture_resume(model, wire, continuation_workloads)
        auxiliary = {"continuation_bytes": cont, "gc_payload_bytes": gc,
                     "collection_with_root": rooted, "collection_after_drop": dropped,
                     "captured_logical_frames": wire["logical_frames"],
                     "captured_numeric_locals": wire["numeric_locals"],
                     "bytes_per_captured_frame": wire["bytes_per_captured_frame"]}
    stage_time = times[stages[expected_stage]]
    return {"mode": expected_mode, "stage": expected_stage, "batch": batch,
            "expected": semantic, "result": semantic, "driver_reported_times_ns": times,
            "stage_time_ns": stage_time, "ns_per_operation": stage_time,
            "raw_wire_observation": copy.deepcopy(wire),
            "continuation_lifecycle": semantic, **auxiliary}


def revalidate_observation(entry: dict[str, Any], observation: dict[str, Any]) -> dict[str, Any]:
    if not isinstance(observation, dict) or not isinstance(observation.get("raw_wire_observation"), dict):
        raise _fail(entry, "retained raw continuation driver result is missing")
    verified = validate_observation(entry, observation["raw_wire_observation"], observation.get("batch"),
                                    expected_mode=observation.get("mode"),
                                    expected_stage=observation.get("stage"))
    if json.dumps(verified, sort_keys=True, separators=(",", ":"), allow_nan=False) != json.dumps(observation, sort_keys=True, separators=(",", ":"), allow_nan=False):
        raise _fail(entry, "normalized continuation observation differs from retained wire evidence")
    return verified
