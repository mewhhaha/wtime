"""Ignored P072 live-continuation workload model and observation validator."""
from __future__ import annotations

from typing import Any

SCHEMA = "wtime-continuation-live-v1"
SCHEMA_VERSION = 1
MODE = "decoded"
PROFILE = "all-phase3-core"
COUNTS = (1, 8, 73, 512)
STAGES = ("setup", "capture")
FEATURES = ["mvp", "stack_switching"]
WAT_SHA256 = "c4d1378e50d36ff9ec82b4f78a0f83783664c002c31e27ccfb751da5d97451ac"
WASM_SHA256 = "2e30292dab3ac305fd029a6b8ff92d5af14f96900b40d5053317c75e8ba589a3"
SOURCE_PATH = "fixtures/continuations/depth_0.wat"


class LiveWorkloadError(ValueError):
    pass


def entries() -> list[dict[str, Any]]:
    """Generate fixed-count registry rows sharing the immutable P012 fixture."""
    rows = []
    for count in COUNTS:
        rows.append({
            "schema_version": 2,
            "id": f"continuation_live_{count}",
            "family": "continuations",
            "source": {"kind": "wat_file", "path": SOURCE_PATH,
                       "sha256": WAT_SHA256, "wasm_sha256": WASM_SHA256},
            "params": [{"name": "live_count", "type": "i32", "min": count,
                        "max": count, "baseline": count}],
            "result": {"type": "none"},
            "oracle": {"kind": "semantic", "function": "continuation_live_workloads.py",
                       "module": "continuation_live_workloads.py", "independent": True,
                       "expected_payload": "i32:7", "expected_resume": "i32:1009"},
            "required_features": FEATURES.copy(),
            "profile": {"status": "ready", "reason": "dedicated multi-continuation lifecycle driver"},
            "work_units": {"unit": "continuation lifecycle", "count_from_param": "live_count"},
            "reset_policy": "fresh_instance_per_invocation",
            "stages": list(STAGES),
            "comparators": {"wasmtime": {"status": "unsupported",
                           "reason": "Pinned comparator has no equivalent stack-switching executor."}},
            "driver": {"kind": "continuation_live", "profile": PROFILE,
                       "protocol": SCHEMA, "profile_policy": "all-phase3-core",
                       "cli_flag": "--continuation-live-set"},
            "batch_policy": {"fixed": 1, "fresh_process_per_sample": True},
        })
    return rows


def oracle(entry_id: str, n: int) -> dict[str, Any]:
    if type(n) is not int or n not in COUNTS or entry_id != f"continuation_live_{n}":
        raise LiveWorkloadError("unknown fixed live-continuation workload/count")
    return {"payloads": ["i32:7"] * (n + 1),
            "results": ["i32:1009"] * (n + 1),
            "roots_before": n, "roots_peak": n + 1,
            "gc_live_before": n, "gc_live_peak": n + 1,
            "gc_live_after_drop": 0}


def _need(ok: bool, message: str) -> None:
    if not ok:
        raise LiveWorkloadError(message)


def _positive_int(value: Any) -> bool:
    return type(value) is int and value > 0


def validate_observation(entry: dict[str, Any], observation: dict[str, Any],
                         stage: str) -> dict[str, Any]:
    """Validate complete typed protocol and expose one untampered timed interval."""
    n = entry.get("params", [{}])[0].get("baseline")
    expected = oracle(entry.get("id", ""), n)
    _need(stage in STAGES and stage in entry.get("stages", []), "undeclared stage")
    _need(isinstance(observation, dict), "observation must be an object")
    _need(observation.get("schema") == SCHEMA and observation.get("schema_version") == 1,
          "wire schema differs")
    _need(observation.get("case") == entry["id"] and observation.get("stage") == stage,
          "wire case/stage differs")
    _need(observation.get("live_count") == n and type(observation.get("live_count")) is int,
          "wire live_count differs")
    _need(observation.get("mode") == MODE and observation.get("profile") == PROFILE,
          "wire mode/profile differs")
    _need(observation.get("batch") == 1 and type(observation.get("batch")) is int,
          "wire batch must be exactly one")
    _need(observation.get("instrumented") is False and observation.get("report_complete") is True,
          "wire must be uninstrumented and complete")
    for key in ("payloads", "results"):
        values = observation.get(key)
        _need(isinstance(values, list) and len(values) == n + 1
              and all(type(value) is str for value in values), f"wire {key} count/type differs")
        _need(values == expected[key], f"wire {key} differs from independent oracle")
    for key in ("roots_before", "roots_peak", "gc_live_before", "gc_live_peak",
                "gc_live_after_drop"):
        _need(type(observation.get(key)) is int and observation[key] == expected[key],
              f"wire {key} differs from ownership oracle")
    for key in ("setup_ns", "capture_ns"):
        _need(_positive_int(observation.get(key)), f"wire {key} must be a positive integer")
    _need(type(observation.get("bytes_before")) is int and observation["bytes_before"] > 0,
          "wire bytes_before must be positive")
    _need(type(observation.get("bytes_after_capture")) is int
          and observation["bytes_after_capture"] > observation["bytes_before"],
          "wire capture must retain positive additional continuation bytes")
    _need(type(observation.get("bytes_after_drop")) is int
          and observation["bytes_after_drop"] == 0, "wire cleanup must release continuation bytes")
    elapsed = observation["setup_ns"] if stage == "setup" else observation["capture_ns"]
    return {"workload": entry["id"], "stage": stage, "batch": 1,
            "ns_per_operation": elapsed, "raw_wire_observation": dict(observation)}
