"""Focused adapter for the complete P013 steady_state_group thread lane.

Each focused timing unit is one complete legacy worker group (21 worker trials)
or SC group (1024 reset trials). A focused batch repeats that complete group.
"""
from __future__ import annotations

import copy
import math
import statistics
from pathlib import Path
from typing import Any, Callable

STAGE = "steady_state_group"
MODE = "decoded"
TRIALS = 21
WAIT_SAMPLES = 1024
WAIT_CLEANUP_TIMEOUT_NS = 1_000_000
MAX_BATCH = 128


def _modules():
    import thread_scaling
    import thread_workloads
    return thread_scaling, thread_workloads


def validate_capability(protocol: dict[str, Any]) -> dict[str, Any]:
    scaling, _ = _modules()
    scaling.validate_protocol(protocol)
    if protocol.get("thread_driver") is not True:
        raise scaling.ThreadMeasurementError("protocol lacks explicit thread_driver capability")
    return protocol


def _case(entry: dict[str, Any]) -> str:
    if entry.get("oracle", {}).get("module") != "thread_workloads.py":
        raise ValueError("focused thread adapter requires a thread_workloads row")
    if entry.get("stages") != [STAGE]:
        raise ValueError("thread row must declare only steady_state_group")
    return entry["id"]


def _xorshift64(value: int) -> int:
    value ^= (value << 13) & ((1 << 64) - 1)
    value ^= value >> 7
    value ^= (value << 17) & ((1 << 64) - 1)
    return value & ((1 << 64) - 1)


def _group_seed(seed: int, offset: int, stride: int) -> int:
    """Jump past prior driver's xorshift seeds so full groups cannot overlap."""
    value = seed
    for _ in range(offset * stride):
        value = _xorshift64(value)
    if value == 0:
        raise ValueError("derived driver seed must remain nonzero")
    return value


def validate_cpu_affinity(value: Any) -> list[int]:
    if (not isinstance(value, list) or len(value) < 2
            or any(type(cpu) is not int or cpu < 0 for cpu in value)
            or len(value) != len(set(value))):
        raise ValueError("thread_cpu_affinity must contain at least two distinct nonnegative CPU ids")
    return value


def _wait_timeouts(entry: dict[str, Any], latency_timeout_ns: int | None,
                   cleanup_timeout_ns: int | None) -> tuple[int, int]:
    scaling, _ = _modules()
    protocol = entry.get("measurement_protocol")
    declared = protocol.get("wait_timeout_ns") if isinstance(protocol, dict) else None
    if type(declared) is not int or not 1 <= declared <= 5_000_000_000:
        raise scaling.ThreadMeasurementError("wait row lacks its fixed supported registry latency timeout")
    latency = declared if latency_timeout_ns is None else latency_timeout_ns
    cleanup = WAIT_CLEANUP_TIMEOUT_NS if cleanup_timeout_ns is None else cleanup_timeout_ns
    if type(latency) is not int or latency != declared:
        raise scaling.ThreadMeasurementError("wait latency timeout differs from the frozen registry value")
    if type(cleanup) is not int or cleanup != WAIT_CLEANUP_TIMEOUT_NS:
        raise scaling.ThreadMeasurementError("wait cleanup timeout differs from the fixed untimed 1 ms policy")
    return latency, cleanup


def validate_raw(entry: dict[str, Any], raw: dict[str, Any], cleanup_raw: dict[str, Any] | None = None,
                 *, latency_timeout_ns: int | None = None,
                 cleanup_timeout_ns: int | None = None) -> dict[str, Any]:
    scaling, model = _modules()
    wid = _case(entry)
    if not isinstance(raw, dict) or raw.get("schema_version") != 1 or raw.get("status") != "ok":
        raise scaling.ThreadMeasurementError("legacy driver report must retain schema_version 1 and status ok")
    if wid.startswith("threads_wait_notify"):
        if not isinstance(cleanup_raw, dict):
            raise scaling.ThreadMeasurementError("wait row requires a separate retained cleanup report")
        latency_timeout, cleanup_timeout = _wait_timeouts(
            entry, latency_timeout_ns, cleanup_timeout_ns)
        latency = scaling._validate_wait_latency(entry, raw, model, WAIT_SAMPLES, latency_timeout)
        cleanup = scaling._validate_wait_cleanup(cleanup_raw, cleanup_timeout, model)
        return {"kind": "wait", "latency": latency, "cleanup": cleanup}
    if cleanup_raw is not None or latency_timeout_ns is not None or cleanup_timeout_ns is not None:
        raise scaling.ThreadMeasurementError("wait cleanup/timeouts are only valid for the wait-notify row")
    if wid.startswith("threads_sc_store_buffering"):
        return {"kind": "sc", "proof": scaling._validate_sc(entry, raw, model)}
    return {"kind": "worker_group", "proof": scaling._validate_atomic(entry, raw, model)}


def normalize_metric(entry: dict[str, Any], raw: dict[str, Any], proof: dict[str, Any]) -> tuple[float, str]:
    wid = _case(entry)
    if proof["kind"] == "worker_group":
        denom = raw.get("total_operations_per_trial")
        walls = proof["proof"]["trial_wall_ns"]
        if type(denom) is not int or denom <= 0:
            raise ValueError("validated worker-group operation denominator is missing")
        return statistics.median(w / denom for w in walls), "median_ns_per_operation_across_21_worker_group_trials"
    if proof["kind"] == "sc":
        return statistics.median(proof["proof"]["trial_wall_ns"]), "median_ns_per_litmus_trial_across_1024_reset_trials"
    samples = proof["latency"]["latency"]["samples_ns"]
    return sum(samples), "sum_ns_of_1024_individual_wait_notify_round_trips_per_group"


def retain_groups(entry: dict[str, Any], groups: list[dict[str, Any]],
                  thread_cpu_affinity: list[int] | None = None, *,
                  latency_timeout_ns: int | None = None,
                  cleanup_timeout_ns: int | None = None) -> dict[str, Any]:
    scaling, _ = _modules()
    wid = _case(entry)
    is_wait = wid.startswith("threads_wait_notify")
    if thread_cpu_affinity is not None:
        validate_cpu_affinity(thread_cpu_affinity)
    if not isinstance(groups, list) or not 1 <= len(groups) <= MAX_BATCH:
        raise scaling.ThreadMeasurementError(f"thread batch must be an integer in 1..{MAX_BATCH}")
    if is_wait:
        latency_timeout_ns, cleanup_timeout_ns = _wait_timeouts(
            entry, latency_timeout_ns, cleanup_timeout_ns)
    elif latency_timeout_ns is not None or cleanup_timeout_ns is not None:
        raise scaling.ThreadMeasurementError("wait timeouts were attached to a non-wait thread row")
    retained, legacy_metrics = [], []
    all_seeds: set[int] = set()
    quota_identity = None
    elapsed = 0
    for index, group in enumerate(groups):
        if not isinstance(group, dict) or set(group) != {"raw", "cleanup"}:
            raise scaling.ThreadMeasurementError("thread group must retain exactly raw and cleanup driver outputs")
        if is_wait:
            if not isinstance(group["cleanup"], dict):
                raise scaling.ThreadMeasurementError("each wait group requires its untimed cleanup output")
        elif group["cleanup"] is not None:
            raise scaling.ThreadMeasurementError("atomic and SC groups cannot include wait cleanup output")
        raw = group["raw"]
        proof = validate_raw(entry, raw, group["cleanup"],
            latency_timeout_ns=latency_timeout_ns if is_wait else None,
            cleanup_timeout_ns=cleanup_timeout_ns if is_wait else None)
        metric, legacy_normalization = normalize_metric(entry, raw, proof)
        if not math.isfinite(metric) or metric <= 0:
            raise scaling.ThreadMeasurementError("legacy thread metric must be finite and positive")
        if proof["kind"] == "wait":
            latency = proof["latency"]["latency"]
            group_elapsed = sum(latency["samples_ns"])
            if group_elapsed <= 0:
                raise scaling.ThreadMeasurementError("wait group has no positive summed individual intervals")
            identity = None
            seeds = []
            legacy_metrics.append({"p50_ns": latency["p50_ns"],
                                   "p99_ns": latency["p99_ns"],
                                   "sample_count": latency["sample_count"],
                                   "normalization": "legacy_individual_wait_notify_p50_p99"})
        else:
            raw_proof = proof["proof"]
            if proof["kind"] == "worker_group":
                identity = (raw_proof["effective_workers"], raw_proof["worker_quota"])
                walls = raw_proof["trial_wall_ns"]
                seeds = raw_proof["trial_seeds"]
            elif proof["kind"] == "sc":
                identity = None
                walls = raw_proof["trial_wall_ns"]
                seeds = raw_proof["trial_seeds"]
            else:
                raise scaling.ThreadMeasurementError("unknown validated thread group kind")
            group_elapsed = sum(walls)
            legacy_metrics.append({"value": metric, "normalization": legacy_normalization})
        if index == 0:
            quota_identity = identity
        elif identity != quota_identity:
            raise scaling.ThreadMeasurementError("worker quota/effective-worker identity drifted across groups")
        overlap = all_seeds.intersection(seeds)
        if overlap:
            raise scaling.ThreadMeasurementError("duplicate seed across complete driver groups")
        all_seeds.update(seeds)
        elapsed += group_elapsed
        retained_group = {"raw": copy.deepcopy(raw), "proof": proof}
        if is_wait:
            retained_group["cleanup"] = copy.deepcopy(group["cleanup"])
        retained.append(retained_group)
    if type(elapsed) is not int or elapsed <= 0:
        raise scaling.ThreadMeasurementError("sum of retained timed trial intervals must be positive")
    metric = elapsed / len(groups)
    settings = {"feature_profile": "threads", "features": entry.get("required_features"),
                "fuel": False, "cancellation_quantum": 1,
                "thread_cpu_affinity": copy.deepcopy(thread_cpu_affinity)}
    normalization = "mean_sum_trial_intervals_per_complete_driver_group"
    if is_wait:
        settings.update({"wait_samples": WAIT_SAMPLES,
                         "wait_timeout_ns": latency_timeout_ns,
                         "wait_cleanup_timeout_ns": cleanup_timeout_ns})
        normalization = "mean_sum_individual_wait_notify_intervals_per_1024_sample_group"
    return {
        "stage": STAGE, "mode": MODE, "batch": len(groups),
        "profile": "threads", "settings": settings,
        "result": {"thread_oracle_validated": entry["id"]},
        "ns_per_operation": metric,
        "elapsed_ns": elapsed,
        "normalization": normalization,
        "thread_groups": retained,
        "thread_legacy_metrics": legacy_metrics,
    }


def observe(executable: str | Path, wasm: str | Path, entry: dict[str, Any], *,
            process: Callable[..., dict[str, Any]], seed: int | None = None,
            cpu: list[int] | None = None,
            thread_cpu_affinity: list[int] | None = None,
            latency_timeout_ns: int | None = None,
            cleanup_timeout_ns: int | None = None, batch: int = 1) -> dict[str, Any]:
    """Run existing thread-driver CLI through injected focused process wrapper."""
    scaling, _model = _modules()
    wid = _case(entry)
    if type(batch) is not int or not 1 <= batch <= MAX_BATCH:
        raise ValueError(f"thread batch must be an integer in 1..{MAX_BATCH}")
    validate_cpu_affinity(thread_cpu_affinity)
    if cpu != thread_cpu_affinity:
        raise ValueError("thread process CPU mask differs from retained affinity identity")
    wasm_s = str(wasm)
    if wid.startswith("threads_wait_notify"):
        if seed is not None:
            raise ValueError("wait-latency groups do not accept or synthesize random seeds")
        latency_timeout_ns, cleanup_timeout_ns = _wait_timeouts(
            entry, latency_timeout_ns, cleanup_timeout_ns)
        groups = []
        for _ in range(batch):
            raw = process(executable, ["--wait-latency", wasm_s, str(WAIT_SAMPLES),
                                      str(latency_timeout_ns)], cpu)
            cleanup = process(executable, ["--wait-cleanup", wasm_s,
                                           str(cleanup_timeout_ns)], cpu)
            groups.append({"raw": raw, "cleanup": cleanup})
        return retain_groups(entry, groups, thread_cpu_affinity,
                             latency_timeout_ns=latency_timeout_ns,
                             cleanup_timeout_ns=cleanup_timeout_ns)
    if type(seed) is not int or not 1 <= seed <= (1 << 64) - 1:
        raise ValueError("seed must be a positive unsigned 64-bit integer")
    if latency_timeout_ns is not None or cleanup_timeout_ns is not None:
        raise scaling.ThreadMeasurementError("wait timeouts are not valid for atomic/SC observations")
    groups = []
    for offset in range(batch):
        stride = 1024 if wid.startswith("threads_sc_store_buffering") else TRIALS
        group_seed = _group_seed(seed, offset, stride)
        if wid.startswith("threads_sc_store_buffering"):
            raw = process(executable, ["--sc-litmus", wasm_s, "1024", str(group_seed)], cpu)
        else:
            config = entry["thread_config"]
            raw = process(executable, ["--thread-case", wasm_s, wid,
                str(config["requested_workers"]), str(config["iterations_per_worker"]), str(TRIALS), str(group_seed)], cpu)
        groups.append({"raw": raw, "cleanup": None})
    return retain_groups(entry, groups, thread_cpu_affinity)


def revalidate(entry: dict[str, Any], observation: dict[str, Any], mode: str, stage: str | None = None,
               expected_thread_cpus: list[int] | None = None) -> None:
    """Candidate-gate hook; rerun all legacy full validators over retained reports."""
    scaling, _model = _modules()
    wid = _case(entry)
    is_wait = wid.startswith("threads_wait_notify")
    if not isinstance(observation, dict):
        raise scaling.ThreadMeasurementError("thread observation must be an object")
    if mode != MODE or observation.get("mode") != MODE or observation.get("stage") != STAGE:
        raise scaling.ThreadMeasurementError("thread observation mode/stage changed")
    if stage is not None and stage != STAGE:
        raise scaling.ThreadMeasurementError("thread product stage changed")
    batch = observation.get("batch")
    if type(batch) is not int or not 1 <= batch <= MAX_BATCH or observation.get("profile") != "threads":
        raise scaling.ThreadMeasurementError("thread batch/profile changed")
    settings = observation.get("settings")
    if not isinstance(settings, dict):
        raise scaling.ThreadMeasurementError("thread execution settings are missing")
    recorded_cpus = settings.get("thread_cpu_affinity")
    validate_cpu_affinity(recorded_cpus)
    if expected_thread_cpus is not None and recorded_cpus != expected_thread_cpus:
        raise scaling.ThreadMeasurementError("thread CPU affinity differs from selected mask")
    wanted = {"feature_profile": "threads", "features": entry.get("required_features"),
              "fuel": False, "cancellation_quantum": 1,
              "thread_cpu_affinity": recorded_cpus}
    latency_timeout_ns = cleanup_timeout_ns = None
    if is_wait:
        latency_timeout_ns, cleanup_timeout_ns = _wait_timeouts(
            entry, settings.get("wait_timeout_ns"), settings.get("wait_cleanup_timeout_ns"))
        wanted.update({"wait_samples": WAIT_SAMPLES,
                       "wait_timeout_ns": latency_timeout_ns,
                       "wait_cleanup_timeout_ns": cleanup_timeout_ns})
    if settings != wanted:
        raise scaling.ThreadMeasurementError("thread execution settings changed")
    fields = {"stage", "mode", "batch", "profile", "settings", "result",
              "ns_per_operation", "elapsed_ns", "normalization", "thread_groups",
              "thread_legacy_metrics"}
    if set(observation) != fields:
        raise scaling.ThreadMeasurementError("thread normalized observation fields changed")
    retained = observation.get("thread_groups")
    if not isinstance(retained, list) or len(retained) != batch:
        raise scaling.ThreadMeasurementError("retained complete driver group count differs from batch")
    rebuilt_groups = [{"raw": item.get("raw"), "cleanup": item.get("cleanup") if is_wait else None}
                      for item in retained if isinstance(item, dict)]
    rebuilt = retain_groups(entry, rebuilt_groups, recorded_cpus,
        latency_timeout_ns=latency_timeout_ns,
        cleanup_timeout_ns=cleanup_timeout_ns)
    if len(rebuilt["thread_groups"]) != len(retained):
        raise scaling.ThreadMeasurementError("malformed retained driver group")
    if rebuilt["thread_groups"] != retained:
        raise scaling.ThreadMeasurementError("retained full thread validator proofs differ")
    wanted_result = {"thread_oracle_validated": entry["id"]}
    if observation.get("result") != wanted_result:
        raise scaling.ThreadMeasurementError("thread normalized oracle marker differs")
    metric, normalization = rebuilt["ns_per_operation"], rebuilt["normalization"]
    timing = observation.get("ns_per_operation")
    if type(timing) not in (int, float) or not math.isfinite(timing) or timing != metric:
        raise scaling.ThreadMeasurementError("thread metric differs from exact sum of retained timed intervals")
    if observation.get("normalization") != normalization:
        raise scaling.ThreadMeasurementError("thread metric normalization declaration differs")
    if type(observation.get("elapsed_ns")) is not int or observation.get("elapsed_ns") != rebuilt["elapsed_ns"]:
        raise scaling.ThreadMeasurementError("elapsed_ns differs from exact retained timed intervals")
    if observation.get("thread_legacy_metrics") != rebuilt["thread_legacy_metrics"]:
        raise scaling.ThreadMeasurementError("legacy median/per-operation metric differs from validator recomputation")


def expected(entry: dict[str, Any]) -> dict[str, str]:
    return {"thread_oracle_validated": _case(entry)}


def worker_identities(observation: dict[str, Any]) -> list[tuple[int, int]]:
    """Return the ordered effective worker/quota proof for every worker group."""
    identities = []
    for group in observation.get("thread_groups", []):
        proof = group.get("proof", {}) if isinstance(group, dict) else {}
        if proof.get("kind") == "worker_group":
            values = proof.get("proof", {})
            identities.append((values.get("effective_workers"), values.get("worker_quota")))
    return identities


def paired_worker_identities_match(baseline: dict[str, Any], candidate: dict[str, Any]) -> bool:
    return worker_identities(baseline) == worker_identities(candidate)
