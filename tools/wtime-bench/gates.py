#!/usr/bin/env python3
"""Bind paired benchmark evidence to current sources and build budgets."""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import math
import os
import random
from dataclasses import dataclass
from pathlib import Path
import subprocess
import sys
import tomllib
from typing import Any

_GATE_FILE_DIR = Path(__file__).resolve().parent
_WORKSPACE_OVERRIDE = os.environ.get("WTIME_GATE_WORKSPACE_ROOT")
if _WORKSPACE_OVERRIDE:
    ROOT = Path(_WORKSPACE_OVERRIDE).resolve(strict=True)
    HERE = ROOT / "tools/wtime-bench"
    if not (HERE / "compare.py").is_file():
        raise RuntimeError("WTIME_GATE_WORKSPACE_ROOT does not contain tools/wtime-bench/compare.py")
    if str(HERE) not in sys.path:
        sys.path.insert(0, str(HERE))
else:
    HERE = _GATE_FILE_DIR
    ROOT = HERE.parents[1]
TASKS_PATH = ROOT.parent / "tasks/performance/TASKS.json"
WORKLOADS_PATH = HERE / "workloads.json"
BASELINE_BUILD_PATH = ROOT / "docs/verification/build-final.json"
HARD_LIMITS = {"clean_seconds": 60.0, "leaf_seconds": 5.0, "stripped_bytes": 5 * 1024 * 1024}
RELATIVE_LIMITS = {
    "clean_seconds": 1.25,
    "leaf_seconds": 1.25,
    "release_seconds": 1.25,
    "stripped_bytes": 1.25,
    "clean_peak_rss_kib": 1.25,
    "unchanged_peak_rss_kib": 1.25,
}


class GateError(ValueError):
    """The evidence is incomplete, stale, or internally inconsistent."""


def _load_module(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise GateError(f"cannot load {path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


analyze = _load_module("wtime_gate_analyze", HERE / "analyze.py")
compare = _load_module("wtime_gate_compare", HERE / "compare.py")
build_budget = _load_module("wtime_gate_build_budget", ROOT / "tools/checks/build_budget.py")


def _read_json(path: Path, label: str) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError) as exc:
        raise GateError(f"cannot read {label}: {exc}") from exc
    if not isinstance(value, dict):
        raise GateError(f"{label} must be a JSON object")
    return value


def _file_sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def _task(task_id: str) -> dict[str, Any]:
    data = _read_json(TASKS_PATH, "performance task manifest")
    tasks = data.get("tasks")
    if not isinstance(tasks, list):
        raise GateError("performance task manifest has no tasks array")
    matches = [item for item in tasks if isinstance(item, dict) and item.get("id") == task_id]
    if len(matches) != 1:
        raise GateError(f"unknown task {task_id!r}")
    return matches[0]


def _selection_stages(selection: str) -> dict[str, set[str]]:
    data = _read_json(WORKLOADS_PATH, "workload registry")
    record = data.get("selections", {}).get(selection)
    if not isinstance(record, dict):
        raise GateError(f"selection {selection!r} is not declared in workloads.json")
    values = record.get("workloads")
    if not isinstance(values, list) or any(not isinstance(item, str) for item in values):
        raise GateError(f"selection {selection!r} has no valid workload list")
    if not values:
        raise GateError(f"selection {selection!r} has no declared workloads")
    registry = {item.get("id"): item for item in data.get("workloads", []) if isinstance(item, dict)}
    stages = {}
    for workload in values:
        item = registry.get(workload)
        if item is None or not isinstance(item.get("stages"), list):
            raise GateError(f"selection {selection!r} references unknown workload {workload!r}")
        stages[workload] = set(item["stages"])
    return stages


def _current_source_identity(selection: str | None = None) -> dict[str, str]:
    include_memory = compare.selection_uses_oracle_module(selection, "memory_workloads.py")
    include_simd = compare.selection_uses_oracle_module(selection, "simd_workloads.py")
    return compare.sources(include_memory=include_memory, include_simd=include_simd,
                           include_integration=selection == 'integration')


def _check_report(report: dict[str, Any], selection: str) -> tuple[dict[str, Any], dict[str, Any]]:
    if report.get("passed") is not True or report.get("source_stable") is not True:
        raise GateError("comparison report must have passed=true and source_stable=true")
    if report.get("selection") != selection:
        raise GateError("comparison selection does not match the task's predeclared selection")
    metadata = report.get("metadata")
    if not isinstance(metadata, dict):
        raise GateError("comparison report metadata is missing")
    recorded_sources = metadata.get("candidate_sources")
    current_sources = _current_source_identity(selection)
    if not isinstance(recorded_sources, dict) or not recorded_sources:
        raise GateError("comparison candidate_sources identity is missing")
    if recorded_sources != current_sources:
        old = set(recorded_sources)
        current = set(current_sources)
        changed = sorted(path for path in old & current if recorded_sources[path] != current_sources[path])
        missing = sorted(old - current)
        added = sorted(current - old)
        raise GateError(f"comparison source identity is stale (changed={changed[:3]}, missing={missing[:3]}, added={added[:3]})")
    try:
        summary = analyze.analyze(report)
    except (analyze.ReportError, TypeError, KeyError, ValueError) as exc:
        raise GateError(f"paired report validation failed: {exc}") from exc
    return metadata, summary


@dataclass
class _BuildSnapshot:
    """One evaluation's shared Cargo graph, input identities, and dependency scan."""

    metadata: dict[str, Any]
    source_inputs: dict[str, str]
    source_sha256: str
    dependency_sources: dict[str, str]
    dependency_violations: list[str]
    source_inputs_after: dict[str, str] | None = None
    source_sha256_after: str | None = None


def _current_rust_sources() -> dict[str, str]:
    return {
        str(path.relative_to(ROOT)): path.read_text(errors="replace")
        for path in (ROOT / "crates").rglob("*.rs")
    }


def _make_build_snapshot() -> _BuildSnapshot:
    """Capture the Cargo graph and build/dependency inputs once for this gate run."""
    try:
        metadata = build_budget.cargo_metadata()
        inputs = build_budget.source_inputs(metadata)
    except (OSError, subprocess.SubprocessError, json.JSONDecodeError) as exc:
        raise GateError(f"cannot capture current build inputs: {exc}") from exc
    if not isinstance(inputs, dict) or not inputs:
        raise GateError("current build source_inputs are missing")
    sys.path.insert(0, str(ROOT / "tools" / "checks"))
    import dependency_budget
    rust_sources = _current_rust_sources()
    violations = dependency_budget.violations(metadata, rust_sources)
    return _BuildSnapshot(metadata, inputs, build_budget.hash_inputs(inputs), rust_sources, violations)


def _finish_build_snapshot(snapshot: _BuildSnapshot, selection: str,
                           candidate_sources: dict[str, str]) -> None:
    """Recheck inputs after evaluation so a mid-gate source edit still fails closed."""
    try:
        after = build_budget.source_inputs(snapshot.metadata)
    except (OSError, subprocess.SubprocessError, json.JSONDecodeError) as exc:
        raise GateError(f"cannot recheck build source inputs: {exc}") from exc
    snapshot.source_inputs_after = after
    snapshot.source_sha256_after = build_budget.hash_inputs(after)
    if after != snapshot.source_inputs or snapshot.source_sha256_after != snapshot.source_sha256:
        raise GateError("build source inputs changed during gate evaluation")
    if _current_rust_sources() != snapshot.dependency_sources:
        raise GateError("Rust dependency/unsafe scan sources changed during gate evaluation")
    current_benchmark_sources = _current_source_identity(selection)
    if current_benchmark_sources != candidate_sources:
        raise GateError("benchmark source inputs changed during gate evaluation")


def _validate_build(path: Path, *, expected_mode: str, label: str,
                    snapshot: _BuildSnapshot | None = None) -> dict[str, Any]:
    snapshot = snapshot or _make_build_snapshot()
    record = _read_json(path, label)
    current_hash = snapshot.source_sha256
    if record.get("source_stable") is not True:
        raise GateError(f"{label} did not preserve its source inputs")
    if record.get("source_sha256") != current_hash:
        raise GateError(f"{label} source hash is stale")
    inputs = record.get("source_inputs")
    inputs_after = record.get("source_inputs_after")
    current_inputs = snapshot.source_inputs
    if not isinstance(inputs, dict) or inputs != current_inputs:
        raise GateError(f"{label} source_inputs do not match current build inputs")
    if inputs_after != current_inputs or record.get("source_sha256_after") != current_hash:
        raise GateError(f"{label} post-build source identity is stale")
    if record.get("mode") != expected_mode:
        raise GateError(f"{label} must have mode={expected_mode!r}")
    if expected_mode == "default" and record.get("features") != []:
        raise GateError("default build report must not enable optional features")
    if record.get("default_dependency_closure_pass") is not True:
        raise GateError(f"{label} does not prove the default dependency closure")
    budgets = record.get("budgets")
    if not isinstance(budgets, dict):
        raise GateError(f"{label} has no hard budget definitions")
    for name, limit in HARD_LIMITS.items():
        value = record.get(name)
        if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value):
            raise GateError(f"{label} is missing finite {name}")
        if value > limit:
            raise GateError(f"{label} exceeds {name} hard limit ({value} > {limit})")
        configured = budgets.get(name)
        if isinstance(configured, bool) or not isinstance(configured, (int, float)) or configured != limit:
            raise GateError(f"{label} has an unexpected {name} budget")
    if record.get("budget_pass") is not True:
        raise GateError(f"{label} reports budget_pass=false")
    return record


def _candidate_features_from_command(command: Any) -> list[str]:
    """Extract the explicit package feature list from a Cargo build argv."""
    if not isinstance(command, list) or any(not isinstance(item, str) for item in command):
        raise GateError("candidate build command must be a string array")
    if len(command) < 2 or command[:2] != ["cargo", "build"]:
        raise GateError("candidate build record is not a Cargo build command")
    if "--locked" not in command or "--release" not in command or "--no-default-features" not in command:
        raise GateError("candidate build must be locked, release, and explicitly disable default features")
    if "--all-features" in command:
        raise GateError("candidate build must not enable --all-features")
    if "--manifest-path" not in command:
        raise GateError("candidate build command has no manifest path")
    manifest_index = command.index("--manifest-path")
    if manifest_index + 1 >= len(command) or command[manifest_index + 1] != "tools/wtime-bench/Cargo.toml":
        raise GateError("candidate build command must target tools/wtime-bench/Cargo.toml")
    values: list[str] = []
    i = 0
    while i < len(command):
        item = command[i]
        if item == "--features":
            if i + 1 >= len(command):
                raise GateError("candidate build --features has no value")
            values.extend(command[i + 1].split(","))
            i += 2
        elif item.startswith("--features="):
            values.extend(item.split("=", 1)[1].split(","))
            i += 1
        else:
            i += 1
    features = [value.strip() for value in values]
    if any(not value for value in features) or len(features) != len(set(features)):
        raise GateError("candidate build must declare a duplicate-free feature list")
    return sorted(features)


def _runtime_forwarded_features(candidate_features: list[str], manifest_path: Path) -> list[str]:
    """Expand benchmark features and return the exact forwarded runtime features."""
    try:
        manifest = tomllib.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError) as exc:
        raise GateError(f"cannot read benchmark Cargo feature graph: {exc}") from exc
    feature_graph = manifest.get("features")
    if not isinstance(feature_graph, dict):
        raise GateError("benchmark Cargo.toml has no feature graph")
    forwarded: set[str] = set()
    visiting: set[str] = set()

    def visit(feature: str) -> None:
        if feature.startswith("wtime-runtime/"):
            forwarded.add(feature)
            return
        if feature.startswith("dep:"):
            return
        children = feature_graph.get(feature)
        if children is None:
            return
        if not isinstance(children, list) or any(not isinstance(child, str) for child in children):
            raise GateError(f"benchmark Cargo feature {feature!r} is malformed")
        if feature in visiting:
            raise GateError(f"benchmark Cargo feature graph is cyclic at {feature!r}")
        visiting.add(feature)
        for child in children:
            visit(child)
        visiting.remove(feature)

    for feature in candidate_features:
        if feature not in feature_graph:
            raise GateError(f"candidate build requested undeclared benchmark feature {feature!r}")
        visit(feature)
    return sorted(forwarded)


def _validate_candidate_build_record(path: Path, metadata: dict[str, Any], report: dict[str, Any],
                                    *, expected_root: Path | None = None) -> dict[str, Any]:
    """Validate the retained candidate build, binary and exact protocol against the report."""
    record_path = path.resolve(strict=True)
    record = _read_json(record_path, "candidate production build record")
    root = (expected_root or ROOT).resolve(strict=True)
    if record.get("schema") == "wtime-runtime-only-production-v1":
        tools_root = ROOT / "tools"
        if str(tools_root) not in sys.path:
            sys.path.insert(0, str(tools_root))
        from verification.production_attestation import validate_runtime_record
        try:
            return validate_runtime_record(record_path, metadata, report, root)
        except (ValueError, KeyError, OSError, TypeError) as error:
            raise GateError(f"runtime-only production attestation failed: {error}") from error
    if (record.get("schema") not in {"p038-production-command-v1", "wtime-production-command-v1"}
            or record.get("phase") != "build"
            or record.get("exit_code") != 0 or record.get("source_stable") is not True
            or Path(record.get("cwd", "")).resolve() != root):
        raise GateError("candidate build record is not a successful, source-stable production build")
    command_features = _candidate_features_from_command(record.get("command"))
    settings = metadata.get("settings")
    mode = settings.get("candidate_mode") if isinstance(settings, dict) else None
    expected_features = {"decoded": [], "compact": ["compact-executor"]}
    if mode not in expected_features or (record["schema"] == "p038-production-command-v1" and mode != "compact"):
        raise GateError("candidate build has an unsupported execution mode")
    if command_features != expected_features[mode]:
        raise GateError(f"candidate build must attest exactly the {mode} feature configuration")
    if "candidate_build_features" in metadata:
        reported_build_features = metadata.get("candidate_build_features")
        if (not isinstance(reported_build_features, list)
                or sorted(reported_build_features) != command_features):
            raise GateError("comparison candidate build features conflict with the successful build command")

    # Bind the retained binary to both the build record and the paired comparison.
    binary = Path(record.get("binary_path", "")).resolve(strict=True)
    binary_hash = record.get("binary_sha256")
    if not binary.is_file() or not isinstance(binary_hash, str) or _file_sha(binary) != binary_hash:
        raise GateError("candidate build record binary path/SHA is invalid")
    if metadata.get("candidate_binary_sha256") != binary_hash:
        raise GateError("candidate build binary SHA differs from the paired comparison")

    # The existing comparison validator already proves all recorded candidate sources are current.
    # Require the build receipt's complete runtime source map to equal that report's runtime subset.
    candidate_sources = metadata.get("candidate_sources")
    if not isinstance(candidate_sources, dict) or not candidate_sources:
        raise GateError("comparison candidate source map is missing")
    runtime_sources = {name: digest for name, digest in candidate_sources.items()
                       if name.startswith("crates/wtime-runtime/src/")}
    recorded_runtime = record.get("runtime_source_map")
    if (not runtime_sources or not isinstance(recorded_runtime, dict)
            or recorded_runtime != runtime_sources
            or any(not isinstance(digest, str) or len(digest) != 64 for digest in recorded_runtime.values())):
        raise GateError("candidate build runtime sources differ from the paired comparison source map")

    protocol_command = record.get("protocol_command")
    if (record.get("protocol_exit") != 0 or protocol_command != [str(binary), "--protocol"]):
        raise GateError("candidate build record does not prove the exact successful --protocol command")
    protocol_path = record_path.parent / "protocol.stdout.json"
    protocol = _read_json(protocol_path, "retained candidate protocol output")
    settings = metadata.get("settings")
    capabilities = metadata.get("capabilities")
    reported_protocol = capabilities.get("candidate") if isinstance(capabilities, dict) else None
    if (protocol.get("schema_version") != 2 or protocol.get("engine") != "wtime"
            or mode not in protocol.get("modes", []) or protocol.get("instrumented") is not False
            or not isinstance(settings, dict) or settings.get("candidate_mode") != mode
            or settings.get("instrumented") is not False or protocol != reported_protocol):
        raise GateError("retained candidate protocol does not exactly match the uninstrumented comparison capability")

    helper_path = record_path.parent / "helper.executed.py"
    stdout_path = record_path.parent / "stdout.log"
    stderr_path = record_path.parent / "stderr.log"
    if (not helper_path.is_file() or _file_sha(helper_path) != record.get("helper_sha256")
            or not stdout_path.is_file() or _file_sha(stdout_path) != record.get("stdout_sha256")
            or not stderr_path.is_file() or _file_sha(stderr_path) != record.get("stderr_sha256")):
        raise GateError("candidate production build record logs/helper do not match their recorded hashes")

    runtime_features = _runtime_forwarded_features(
        command_features, root / "tools/wtime-bench/Cargo.toml")
    expected_runtime = [] if mode == "decoded" else ["wtime-runtime/compact-executor"]
    if runtime_features != expected_runtime:
        raise GateError("candidate build does not forward exactly its declared runtime features")
    return {
        "record": {"path": str(record_path), "sha256": _file_sha(record_path), "bytes": record_path.stat().st_size},
        "command_features": command_features,
        "runtime_features": runtime_features,
        "binary_sha256": binary_hash,
        "runtime_source_sha256": hashlib.sha256(json.dumps(
            recorded_runtime, sort_keys=True, separators=(",", ":")
        ).encode()).hexdigest(),
        "protocol": protocol,
        "protocol_sha256": _file_sha(protocol_path),
    }


def _validate_optional_features(optional_build: dict[str, Any] | None,
                                requested_features: list[str],
                                optional_build_path: Path | None) -> None:
    if requested_features:
        if optional_build_path is None or optional_build is None:
            raise GateError("comparison requires an optional feature build report")
        if set(optional_build.get("features", [])) != set(requested_features):
            raise GateError("optional build features do not match the candidate's forwarded runtime features")
    elif optional_build_path is not None:
        raise GateError("--optional-build supplied but the candidate build forwards no runtime features")


def _check_default_dependencies(snapshot: _BuildSnapshot | None = None) -> list[str]:
    snapshot = snapshot or _make_build_snapshot()
    errors = snapshot.dependency_violations
    if errors:
        raise GateError("default dependency/unsafe gate failed: " + "; ".join(errors[:5]))
    return errors


def _ratio(candidate: Any, baseline: Any) -> float | None:
    if (isinstance(candidate, bool) or not isinstance(candidate, (int, float))
            or isinstance(baseline, bool) or not isinstance(baseline, (int, float))
            or not math.isfinite(candidate) or not math.isfinite(baseline) or baseline <= 0):
        return None
    return float(candidate) / float(baseline)


def _relative_build_growth(candidate: dict[str, Any], baseline: dict[str, Any]) -> dict[str, float]:
    growth = {}
    for field, limit in RELATIVE_LIMITS.items():
        ratio = _ratio(candidate.get(field), baseline.get(field))
        if ratio is not None and ratio > limit:
            growth[field] = ratio
    return growth


def _load_contract(path: Path, selection: str) -> tuple[dict[str, Any], str]:
    contract = _read_json(path, "promotion contract")
    if contract.get("schema_version") != 1 or contract.get("selection") != selection:
        raise GateError("promotion contract schema_version/selection does not match")
    expected = {"schema_version", "selection", "primary_workloads", "protected_workloads"}
    if set(contract) != expected:
        raise GateError("promotion contract must contain exactly schema_version, selection, primary_workloads, and protected_workloads")
    roles: dict[str, list[str]] = {}
    for field in ("primary_workloads", "protected_workloads"):
        values = contract[field]
        if not isinstance(values, list) or not values or any(not isinstance(v, str) or not v for v in values):
            raise GateError(f"promotion contract {field} must be a nonempty string array")
        if len(set(values)) != len(values):
            raise GateError(f"promotion contract {field} contains duplicates")
        roles[field] = values
    primary, protected = set(roles["primary_workloads"]), set(roles["protected_workloads"])
    if primary & protected:
        raise GateError("promotion contract primary and protected workloads overlap")
    declared = set(_selection_stages(selection))
    if primary | protected != declared:
        missing, unknown = sorted(declared - (primary | protected)), sorted((primary | protected) - declared)
        raise GateError(f"promotion contract must cover the full selection (missing={missing[:4]}, unknown={unknown[:4]})")
    return contract, _file_sha(path)


def _family_bootstrap(report: dict[str, Any], primary: set[str], run_index: int,
                      contract_hash: str) -> dict[str, Any]:
    """Equal-weight inputs within workloads and workloads within the primary family."""
    _, runs = analyze._validated_runs(report)
    run = runs[run_index - 1]
    grouped: dict[str, dict[tuple[str, str], list[float]]] = {}
    for key, pairs in sorted(run["groups"].items()):
        workload, stage, input_id, fixture = key
        if workload in primary and stage == "warm_call":
            logs = [math.log(float(p["candidate"]["ns_per_operation"]) /
                             float(p["baseline"]["ns_per_operation"])) for p in pairs]
            grouped.setdefault(workload, {})[(input_id, fixture)] = sorted(logs)
    if set(grouped) != primary:
        raise GateError(f"primary family warm coverage incomplete in run {run_index}")
    for workload in primary:
        if not grouped[workload]:
            raise GateError(f"primary family has no warm groups for {workload} in run {run_index}")

    def aggregate(samples: dict[str, dict[tuple[str, str], float]]) -> float:
        workload_logs = []
        for workload in sorted(primary):
            # A workload's fixtures are equally weighted within each input; inputs are then equal.
            by_input: dict[str, list[float]] = {}
            for (input_id, _fixture), value in samples[workload].items():
                by_input.setdefault(input_id, []).append(value)
            input_logs = [sum(values) / len(values) for _, values in sorted(by_input.items())]
            # Inputs are represented by their median log-ratio; fixture multiplicity
            # within one input is averaged so it cannot overweight that input.
            workload_logs.append(analyze._percentile(input_logs, .5))
        return math.exp(sum(workload_logs) / len(workload_logs))

    observed: dict[str, dict[tuple[str, str], float]] = {}
    for workload, groups in sorted(grouped.items()):
        observed[workload] = {key: analyze._percentile(logs, .5) for key, logs in sorted(groups.items())}
    point = aggregate(observed)
    seed_material = f"P023-family\0{contract_hash}\0{run_index}\0{run['seed']}"
    seed = int.from_bytes(hashlib.sha256(seed_material.encode()).digest()[:8], "big")
    rng = random.Random(seed)
    draws = []
    for _ in range(analyze.BOOTSTRAP_RESAMPLES):
        sampled: dict[str, dict[tuple[str, str], float]] = {}
        for workload, groups in sorted(grouped.items()):
            sampled[workload] = {}
            for key, logs in sorted(groups.items()):
                n = len(logs)
                values = [logs[rng.randrange(n)] for _ in range(n)]
                sampled[workload][key] = analyze._percentile(values, .5)
        draws.append(aggregate(sampled))
    return {"median_ratio": point,
            "ci95": {"lower": analyze._percentile(draws, .025), "upper": analyze._percentile(draws, .975)},
            "bootstrap_resamples": analyze.BOOTSTRAP_RESAMPLES, "bootstrap_seed": seed,
            "workloads": len(primary), "input_fixture_groups": sum(len(v) for v in grouped.values())}


def _comparison_decision(summary: dict[str, Any], selection: str, *,
                         report: dict[str, Any] | None = None,
                         contract: dict[str, Any] | None = None,
                         contract_hash: str | None = None) -> tuple[str, list[str], dict[str, Any]]:
    primary_stages = _selection_stages(selection)
    primary = set(contract["primary_workloads"]) if contract else set(primary_stages)
    protected = set(contract["protected_workloads"]) if contract else set(primary_stages) - primary
    rows = summary.get("per_run")
    if not isinstance(rows, list) or not rows:
        raise GateError("analyzer produced no per-run measurements")
    by_key = {}
    by_workload_stage = {}
    for row in rows:
        if not isinstance(row, dict):
            continue
        key = (row.get("run"), row.get("workload"), row.get("stage"))
        by_workload_stage.setdefault(key, []).append(row)
        # Historical contracts did not distinguish input/fixture, retain their prior lookup.
        by_key.setdefault(key, row)
    issues: list[str] = []
    indeterminate: list[str] = []
    checked: list[dict[str, Any]] = []
    workload_seen = set()
    runs = {row.get("run") for row in rows if isinstance(row, dict)}
    if runs != {1, 2}:
        raise GateError("analyzer did not return exactly two nonempty runs")

    if contract is not None:
        declared = primary | protected
        reported = {row.get("workload") for row in rows if isinstance(row, dict)}
        unknown = reported - declared
        if unknown:
            raise GateError(f"comparison contains workloads outside its promotion contract: {sorted(unknown)[:4]}")
        exact = {(row.get("run"), row.get("workload"), row.get("stage"),
                  row.get("input"), row.get("fixture")) for row in rows if isinstance(row, dict)}
        for run in (1, 2):
            for workload, stages in primary_stages.items():
                groups = {(input_id, fixture) for row_run, row_workload, stage, input_id, fixture in exact
                          if row_run == run and row_workload == workload}
                if not groups:
                    raise GateError(f"comparison is missing input/fixture coverage for {workload} in run {run}")
                for stage in stages:
                    missing = [(input_id, fixture) for input_id, fixture in groups
                               if (run, workload, stage, input_id, fixture) not in exact]
                    if missing:
                        raise GateError(f"comparison is missing {stage} input/fixture groups for {workload} in run {run}: {missing[:3]}")

    primary_warm_workloads = {workload for workload, stages in primary_stages.items()
                              if "warm_call" in stages and workload in primary}
    missing_primary = [(run, workload) for run in (1, 2) for workload in sorted(primary_warm_workloads)
                       if (run, workload, "warm_call") not in by_key]
    if missing_primary:
        raise GateError(f"comparison is missing primary warm measurements: {missing_primary[:4]}")
    primary_warm = [(run, workload) for run in (1, 2) for workload in sorted(primary_warm_workloads)]
    if not primary_warm:
        indeterminate.append("no primary warm_call measurements are available")
    for run, workload in primary_warm:
        row = by_key[(run, workload, "warm_call")]
        median = row["candidate_over_baseline"]["median"]
        upper = row["median_ratio_95ci"]["upper"]
        lower = row["median_ratio_95ci"]["lower"]
        checked.append({"run": run, "workload": workload, "stage": "warm_call",
                        "median_ratio": median, "ci95": row["median_ratio_95ci"],
                        "required_median_max": 0.95, "required_upper_max_exclusive": 1.0})
        workload_seen.add(workload)
        if contract is None and (median > 0.95 or upper >= 1.0):
            if lower > 1.0:
                issues.append(f"primary warm workload {workload} regressed in run {run}")
            elif upper < 1.0 and median > 0.95:
                issues.append(f"primary warm workload {workload} missed the 5% improvement threshold in run {run}")
            else:
                indeterminate.append(f"primary warm workload {workload} is inconclusive in run {run}")
    for workload, stages in primary_stages.items():
        for stage in stages:
            if any((run, workload, stage) not in by_key for run in (1, 2)):
                raise GateError(f"comparison is missing selected {stage} measurements for {workload}")

    if contract is not None:
        if report is None or not contract_hash:
            raise GateError("contract promotion requires the validated raw report and contract hash")
        aggregate_checks = []
        for run in (1, 2):
            family = _family_bootstrap(report, primary, run, contract_hash)
            aggregate_checks.append({"run": run, **family, "required_median_max": 0.95,
                                     "required_upper_max_exclusive": 1.0})
            median, upper = family["median_ratio"], family["ci95"]["upper"]
            lower = family["ci95"]["lower"]
            if median > 0.95 or upper >= 1.0:
                if lower > 1.0:
                    issues.append(f"primary family regressed in run {run}")
                elif upper < 1.0 and median > 0.95:
                    issues.append(f"primary family missed the 5% improvement threshold in run {run}")
                else:
                    indeterminate.append(f"primary family is inconclusive in run {run}")
        checked.append({"primary_family": aggregate_checks})
        for row in rows:
            if not isinstance(row, dict) or row.get("stage") != "warm_call":
                continue
            run, workload = row.get("run"), row.get("workload")
            upper = row["median_ratio_95ci"]["upper"]
            lower = row["median_ratio_95ci"]["lower"]
            checked.append({"run": run, "workload": workload, "stage": "warm_call",
                            "input": row.get("input"), "fixture": row.get("fixture"),
                            "median_ratio": row["candidate_over_baseline"]["median"],
                            "ci95": row["median_ratio_95ci"], "required_upper_max": 1.05,
                            "role": "primary" if workload in primary else "protected"})
            if workload in primary | protected and upper > 1.05:
                if lower > 1.05:
                    issues.append(f"warm {contract_role(workload, primary)} {workload} group exceeds 1.05 in run {run}")
                else:
                    indeterminate.append(f"warm {contract_role(workload, primary)} {workload} group interval crosses 1.05 in run {run}")

    for row in rows:
        if not isinstance(row, dict):
            continue
        run, workload, stage = row.get("run"), row.get("workload"), row.get("stage")
        median = row["candidate_over_baseline"]["median"]
        upper = row["median_ratio_95ci"]["upper"]
        lower = row["median_ratio_95ci"]["lower"]
        limit = 1.25 if stage == "prepare" else 1.05 if stage in ("first_call", "instantiate", "load_and_call") else None
        if limit is not None:
            checked.append({"run": run, "workload": workload, "stage": stage,
                            "median_ratio": median, "ci95": row["median_ratio_95ci"],
                            "required_upper_max": limit})
            if upper > limit:
                if lower > limit:
                    issues.append(f"{stage} control {workload} exceeds {limit:.2f} in run {run}")
                else:
                    indeterminate.append(f"{stage} control {workload} interval crosses {limit:.2f} in run {run}")
        if contract is None and stage == "warm_call" and workload not in primary:
            checked.append({"run": run, "workload": workload, "stage": stage,
                            "median_ratio": median, "ci95": row["median_ratio_95ci"],
                            "required_upper_max": 1.05})
            if upper > 1.05:
                if lower > 1.05:
                    issues.append(f"protected warm control {workload} exceeds 1.05 in run {run}")
                else:
                    indeterminate.append(f"protected warm control {workload} interval crosses 1.05 in run {run}")

    if issues:
        decision = "rejected"
    elif indeterminate:
        decision = "inconclusive"
    else:
        decision = "adopted"
    info = {"checks": checked, "primary_workloads": sorted(primary)}
    if contract is not None:
        info.update({"protected_workloads": sorted(protected), "family_contract_sha256": contract_hash})
    return decision, issues + indeterminate, info


def contract_role(workload: str, primary: set[str]) -> str:
    return "primary" if workload in primary else "protected"


def evaluate(task_id: str, comparison_dir: Path, build_path: Path,
             optional_build_path: Path | None = None,
             contract_path: Path | None = None,
             candidate_build_record_path: Path | None = None) -> dict[str, Any]:
    task = _task(task_id)
    selection = task.get("measurement_selection")
    if not isinstance(selection, str):
        raise GateError(f"{task_id} has no declared measurement_selection")
    report_path = comparison_dir if comparison_dir.is_file() else comparison_dir / "report.json"
    report = _read_json(report_path, "comparison report")
    metadata, summary = _check_report(report, selection)
    build_snapshot = _make_build_snapshot()
    contract = contract_hash = None
    if contract_path is not None:
        contract, contract_hash = _load_contract(contract_path, selection)
    mode = task.get("execution_mode")
    settings = metadata.get("settings")
    if not isinstance(settings, dict) or settings.get("candidate_mode") != mode:
        raise GateError("comparison candidate_mode does not match the task's declared execution mode")
    candidate_build_attestation = None
    if candidate_build_record_path is not None:
        candidate_build_attestation = _validate_candidate_build_record(
            candidate_build_record_path, metadata, report)
    default_build = _validate_build(build_path, expected_mode="default", label="default build report",
                                    snapshot=build_snapshot)

    if candidate_build_attestation is not None:
        requested_features = candidate_build_attestation["runtime_features"]
    else:
        requested_features = metadata.get("candidate_build_features", metadata.get("candidate_features", []))
        if requested_features is None:
            requested_features = []
        if not isinstance(requested_features, list) or any(not isinstance(item, str) for item in requested_features):
            raise GateError("candidate build features must be a string array")
    optional_build = None
    if requested_features:
        if optional_build_path is None:
            raise GateError("comparison requires an optional feature build report")
        optional_build = _validate_build(optional_build_path, expected_mode="optional-feature",
                                         label="optional build report", snapshot=build_snapshot)
    _validate_optional_features(optional_build, requested_features, optional_build_path)

    baseline = _read_json(BASELINE_BUILD_PATH, "frozen original build report")
    relative_growth = _relative_build_growth(default_build, baseline)
    optional_relative_growth = _relative_build_growth(optional_build, default_build) if optional_build else {}
    dependency_errors = _check_default_dependencies(build_snapshot)
    decision, reasons, comparison_checks = _comparison_decision(
        summary, selection, report=report, contract=contract, contract_hash=contract_hash)
    if not default_build.get("budget_pass"):
        decision = "rejected"
        reasons.append("default build report failed its hard budget")
    if relative_growth or optional_relative_growth:
        explanation = metadata.get("relative_build_growth_explanation")
        if not isinstance(explanation, str) or not explanation.strip():
            decision = "inconclusive"
            reasons.append("relative build/RSS/size growth above 1.25 requires a documented decision")
    kind = task.get("kind")
    if kind in ("foundation", "gate") and decision == "adopted":
        decision = "contract_only"
        reasons.append("task is an integrity/tooling gate with no independent speedup claim")

    candidate_sources = metadata.get("candidate_sources")
    if not isinstance(candidate_sources, dict):
        raise GateError("comparison candidate_sources identity is missing")
    _finish_build_snapshot(build_snapshot, selection, candidate_sources)

    return {
        "schema_version": 1,
        "task": task_id,
        "selection": selection,
        "decision": decision,
        "reasons": reasons,
        "source_identity": {
            "comparison_report_sha256": _file_sha(report_path),
            "candidate_sources_sha256": hashlib.sha256(json.dumps(
                metadata["candidate_sources"], sort_keys=True, separators=(",", ":")
            ).encode()).hexdigest(),
            "build_source_sha256": default_build["source_sha256"],
        },
        "comparison": comparison_checks,
        "promotion_contract": ({"contents": contract, "sha256": contract_hash} if contract else None),
        "build": {
            "default_report": str(build_path.resolve()),
            "default_report_sha256": _file_sha(build_path),
            "default_budget_pass": default_build["budget_pass"],
            "relative_growth_to_frozen_original": relative_growth,
            "optional_relative_growth_to_default": optional_relative_growth,
            "relative_growth_explanation": metadata.get("relative_build_growth_explanation"),
            "optional_report": str(optional_build_path.resolve()) if optional_build_path else None,
            "optional_features": optional_build.get("features", []) if optional_build else [],
            "candidate_build_record": candidate_build_attestation["record"] if candidate_build_attestation else None,
            "candidate_build_features": candidate_build_attestation["command_features"] if candidate_build_attestation else [],
            "candidate_runtime_features": candidate_build_attestation["runtime_features"] if candidate_build_attestation else [],
            "candidate_binary_sha256": candidate_build_attestation["binary_sha256"] if candidate_build_attestation else None,
            "candidate_runtime_source_sha256": candidate_build_attestation["runtime_source_sha256"] if candidate_build_attestation else None,
            "candidate_protocol_sha256": candidate_build_attestation["protocol_sha256"] if candidate_build_attestation else None,
        },
        "default_dependency_closure": {"passed": not dependency_errors, "violations": dependency_errors},
        "analysis": summary,
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--task", required=True)
    parser.add_argument("--comparison", type=Path, required=True)
    parser.add_argument("--build", type=Path, required=True)
    parser.add_argument("--optional-build", type=Path)
    parser.add_argument("--contract", type=Path, help="predeclared primary/protected workload contract JSON")
    parser.add_argument("--candidate-build-record", type=Path,
                        help="source-bound successful candidate Cargo build/protocol record")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        result = evaluate(args.task, args.comparison, args.build, args.optional_build, args.contract,
                          args.candidate_build_record)
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(result, indent=2, sort_keys=True, allow_nan=False) + "\n")
    except (OSError, GateError, analyze.ReportError, ValueError, KeyError, TypeError) as exc:
        result = {"schema_version": 1, "task": args.task, "decision": "rejected", "error": str(exc)}
        try:
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
        except OSError:
            pass
        print(f"gates.py: {exc}", file=sys.stderr)
        return 2
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0 if result["decision"] in ("adopted", "contract_only") else 1


if __name__ == "__main__":
    raise SystemExit(main())
