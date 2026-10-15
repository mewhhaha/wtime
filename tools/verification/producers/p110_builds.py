"""Read and validate retained source-identical build-budget reports for P110.

This reader does not build or time anything. It binds three historical reports
to the current Cargo workspace inputs, validates their retained command/result
records, and reports optional-tier costs against the same-source default row.
"""
from __future__ import annotations

import hashlib
import importlib.util
import json
import math
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[3]
REPORT_DIR = ROOT / "target/verification/runs/P067/verify/attempt-0001"
MODES = {
    "default": ("default", None),
    "register": ("optional-feature", "wtime-runtime/register-executor"),
    "compact": ("optional-feature", "wtime-runtime/compact-executor"),
}
METRICS = (
    "clean_seconds",
    "unchanged_seconds",
    "leaf_seconds",
    "runtime_leaf_seconds",
    "release_seconds",
    "stripped_bytes",
    "clean_peak_rss_kib",
    "unchanged_peak_rss_kib",
)
LIMITS = {"clean_seconds": 60.0, "leaf_seconds": 5.0, "stripped_bytes": 5 * 1024 * 1024}
RELATIVE_REVIEW_LIMIT = 1.25


def _load_build_budget():
    path = ROOT / "tools/checks/build_budget.py"
    spec = importlib.util.spec_from_file_location("p110_build_budget", path)
    if spec is None or spec.loader is None:
        raise ValueError("cannot load existing build-budget helpers")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def _need(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def _is_sha(value: Any) -> bool:
    if not isinstance(value, str) or len(value) != 64:
        return False
    try:
        int(value, 16)
    except ValueError:
        return False
    return True


def _finite_positive(value: Any) -> bool:
    return type(value) in (int, float) and math.isfinite(value) and value > 0


def _validate_one(name: str, record: dict[str, Any], current_inputs: dict[str, str],
                  *, expected_jobs: int | None = None, hash_inputs=None) -> None:
    _need(isinstance(record, dict), f"{name} build report must be an object")
    _need(record.get("schema_version") == 2, f"{name} build report has an unsupported schema")
    expected_mode, feature = MODES[name]
    _need(record.get("mode") == expected_mode, f"{name} build mode is wrong")
    expected_features = [] if feature is None else [feature]
    _need(record.get("features") == expected_features, f"{name} build features are wrong")
    if feature is None:
        _need(record.get("feature_package") is None and record.get("feature_name") is None,
              "default build declares an optional feature")
    else:
        package, feature_name = feature.split("/", 1)
        _need(record.get("feature_package") == package and record.get("feature_name") == feature_name,
              f"{name} feature metadata is inconsistent")

    inputs = record.get("source_inputs")
    _need(isinstance(inputs, dict) and bool(inputs), f"{name} build source_inputs are missing")
    _need(inputs == current_inputs, f"{name} build does not match current Cargo source inputs")
    for path, digest in inputs.items():
        _need(isinstance(path, str) and _is_sha(digest), f"{name} has a malformed source input")
    inputs_after = record.get("source_inputs_after")
    isolated_after = record.get("isolated_source_inputs_after")
    _need(inputs_after == inputs and isolated_after == inputs,
          f"{name} build inputs were not restored after leaf/build checks")

    if hash_inputs is None:
        hash_inputs = _load_build_budget().hash_inputs
    expected_hash = hash_inputs(inputs)
    _need(record.get("source_sha256") == expected_hash
          and record.get("source_sha256_after") == expected_hash
          and record.get("isolated_workspace_source_sha256") == expected_hash,
          f"{name} source hash does not bind its exact source map")
    _need(record.get("source_stable") is True, f"{name} build sources were unstable")
    _need(inputs.get("Cargo.lock") == record.get("lock_sha256"), f"{name} Cargo.lock hash is inconsistent")
    _need("crates/wtime-cli/src/main.rs" in inputs
          and "crates/wtime-runtime/src/numeric.rs" in inputs,
          f"{name} input map omits the measured CLI/runtime leaves")

    jobs = record.get("jobs")
    logical_cpus = record.get("logical_cpus")
    _need(type(jobs) is int and jobs > 0 and type(logical_cpus) is int and logical_cpus > 0,
          f"{name} job count is malformed")
    _need(jobs == min(logical_cpus, 5), f"{name} job count differs from the build-budget protocol")
    if expected_jobs is not None:
        _need(jobs == expected_jobs, "build reports used different job counts")
    command = ["cargo", "build", "--locked", "--offline", "-j", str(jobs)]
    if feature is not None:
        command += ["--features", feature]
    _need(record.get("build_command") == command, f"{name} build command/settings changed")
    _need(record.get("target_initially_empty") is True, f"{name} clean build target was not empty")
    _need(isinstance(record.get("cargo_version"), str) and record["cargo_version"].startswith("cargo "),
          f"{name} Cargo version is missing")
    _need(isinstance(record.get("rustc"), str) and record["rustc"].startswith("rustc "),
          f"{name} rustc identity is missing")
    _need(isinstance(record.get("cpu"), str) and bool(record["cpu"]), f"{name} CPU identity is missing")
    _need(isinstance(record.get("compile_environment"), dict), f"{name} compile environment is malformed")
    _need(isinstance(record.get("resolved_graph"), list) and bool(record["resolved_graph"]),
          f"{name} resolved Cargo graph is missing")

    budgets = record.get("budgets")
    _need(budgets == LIMITS, f"{name} build budget definitions changed")
    _need(record.get("default_dependency_closure_pass") is True
          and record.get("default_dependency_violations") == [],
          f"{name} did not retain a clean default dependency/unsafe scan")
    for metric in METRICS:
        value = record.get(metric)
        _need(_finite_positive(value), f"{name} is missing a positive {metric}")
    for metric in ("stripped_bytes", "clean_peak_rss_kib", "unchanged_peak_rss_kib"):
        _need(type(record.get(metric)) is int and record[metric] > 0,
              f"{name} {metric} must be a positive integer")
    _need(record["clean_seconds"] <= LIMITS["clean_seconds"]
          and record["leaf_seconds"] <= LIMITS["leaf_seconds"]
          and record["runtime_leaf_seconds"] <= LIMITS["leaf_seconds"]
          and record["stripped_bytes"] <= LIMITS["stripped_bytes"],
          f"{name} exceeds a hard build budget")
    _need(record.get("budget_pass") is True, f"{name} build report did not pass its hard budgets")


def validate_reports(reports: dict[str, dict[str, Any]], current_inputs: dict[str, str],
                     command_results: dict[str, dict[str, Any]],
                     dependency_stdout: str, *, hash_inputs=None) -> dict[str, Any]:
    """Validate report objects and return a descriptive, non-causal summary."""
    _need(set(reports) == set(MODES), "default, Register, and Compact build reports are required")
    _need(set(command_results) == set(MODES), "all three successful build command records are required")
    _need(isinstance(current_inputs, dict) and bool(current_inputs), "current Cargo source map is missing")
    if hash_inputs is None:
        hash_inputs = _load_build_budget().hash_inputs

    common: dict[str, Any] | None = None
    expected_jobs: int | None = None
    for name in MODES:
        record = reports[name]
        _validate_one(name, record, current_inputs, expected_jobs=expected_jobs, hash_inputs=hash_inputs)
        if expected_jobs is None:
            expected_jobs = record["jobs"]
        identity = {
            "jobs": record["jobs"],
            "logical_cpus": record["logical_cpus"],
            "cpu": record["cpu"],
            "cargo_version": record["cargo_version"],
            "rustc": record["rustc"],
            "lock_sha256": record["lock_sha256"],
            "compile_environment": record["compile_environment"],
        }
        if common is None:
            common = identity
        else:
            _need(identity == common, "build reports used different toolchains or settings")
        result = command_results[name]
        _need(isinstance(result, dict) and result.get("exit_code") == 0,
              f"{name} budget command did not succeed")
        feature = MODES[name][1]
        argv = result.get("argv")
        _need(isinstance(argv, list) and all(isinstance(item, str) for item in argv)
              and any(item.endswith("/tools/checks/build_budget.py") for item in argv),
              f"{name} command record does not invoke build_budget.py")
        script_index = next(i for i, item in enumerate(argv) if item.endswith("/tools/checks/build_budget.py"))
        output_path = argv[script_index + 2] if len(argv) > script_index + 2 else ""
        _need(argv[script_index + 1] == "--output" and output_path.endswith(f"/{name}-build.json"),
              f"{name} command output path is wrong")
        if feature is None:
            _need("--features" not in argv and argv[-1].endswith("-build.json"),
                  "default build command enabled an optional feature")
        else:
            _need(argv[-2:] == ["--features", feature],
                  f"{name} command did not select its exact feature")
        _need(result.get("name") == f"{name}-budget", f"{name} command record label is wrong")
        _need(result.get("cwd") in (None, str(ROOT)), f"{name} command cwd is outside the checkout")

    _need(isinstance(dependency_stdout, str)
          and "Production dependency/unsafe budget: 0 violations" in dependency_stdout,
          "retained dependency report does not prove zero default external/unsafe violations")
    _need(isinstance(reports["default"].get("default_dependency_violations"), list)
          and reports["default"]["default_dependency_violations"] == [],
          "default dependency violations are not empty")

    default = reports["default"]
    ratios: dict[str, dict[str, float]] = {}
    review_required = False
    for tier in ("register", "compact"):
        tier_ratios = {}
        for metric in METRICS:
            denominator = default[metric]
            _need(_finite_positive(denominator), "default baseline metric is invalid")
            ratio = reports[tier][metric] / denominator
            _need(math.isfinite(ratio) and ratio > 0, f"{tier} {metric} ratio is invalid")
            tier_ratios[metric] = ratio
            review_required |= ratio > RELATIVE_REVIEW_LIMIT
        ratios[tier] = tier_ratios

    inputs_hash = hash_inputs(current_inputs)
    return {
        "schema": "wtime-p110-build-budgets-v1",
        "status": "passed",
        "source_sha256": inputs_hash,
        "source_input_count": len(current_inputs),
        "source_inputs_current": True,
        "toolchain": common,
        "leaf_restoration": {
            "cli_leaf": "crates/wtime-cli/src/main.rs",
            "runtime_leaf": "crates/wtime-runtime/src/numeric.rs",
            "source_bytes_match_before_after": True,
            "basis": "exact current source_inputs, source_inputs_after, and isolated_source_inputs_after equality",
            "timestamps_recorded": False,
        },
        "dependency_budget": {
            "status": "passed",
            "default_external_or_unsafe_violations": 0,
        },
        "baseline": {
            "kind": "same-source default build report",
            "mode": "default",
            "source_sha256": inputs_hash,
            "metrics": {metric: default[metric] for metric in METRICS},
            "interpretation": "reference row for optional-tier cost ratios only",
        },
        "optional_tier_review": {
            "kind": "unpaired optional-tier versus same-source default build-budget observations",
            "threshold": RELATIVE_REVIEW_LIMIT,
            "ratios": ratios,
            "review_required": review_required,
            "speedup_claim": False,
        },
        "p001_build_comparison": {
            "available": False,
            "status": "unavailable",
            "reason": "no comparable original P001 build-budget report is retained",
        },
        "reports": {
            name: {"metrics": {metric: reports[name][metric] for metric in METRICS},
                   "mode": reports[name]["mode"], "features": reports[name]["features"]}
            for name in MODES
        },
    }


def collect() -> dict[str, Any]:
    """Read the immutable P067 report set and validate it against current inputs."""
    build_budget = _load_build_budget()
    metadata = build_budget.cargo_metadata()
    current_inputs = build_budget.source_inputs(metadata)
    reports: dict[str, dict[str, Any]] = {}
    results: dict[str, dict[str, Any]] = {}
    for name in MODES:
        reports[name] = json.loads((REPORT_DIR / f"{name}-build.json").read_text())
        results[name] = json.loads((REPORT_DIR / f"{name}-budget.result.json").read_text())
        _check_captured_build_command(name, results[name], reports[name], REPORT_DIR)
    dependency_result = json.loads((REPORT_DIR / "dependency-budget.result.json").read_text())
    dependency_stdout = (REPORT_DIR / "dependency-budget.stdout").read_text()
    _need(dependency_result.get("exit_code") == 0, "dependency-budget command did not succeed")
    _check_captured_output(dependency_result, REPORT_DIR / "dependency-budget.stdout",
                           REPORT_DIR / "dependency-budget.stderr")
    dependency_command = json.loads((REPORT_DIR / "dependency-budget.command.json").read_text())
    _need(dependency_command.get("argv") == dependency_result.get("argv")
          and dependency_command.get("cwd") == dependency_result.get("cwd"),
          "dependency-budget command/result records disagree")
    _need(dependency_result.get("name") == "dependency-budget"
          and dependency_result["argv"][-1].endswith("/tools/checks/dependency_budget.py"),
          "dependency-budget stage did not invoke the existing checker")
    summary=validate_reports(reports, current_inputs, results, dependency_stdout)
    _need(build_budget.source_inputs(metadata)==current_inputs,"build inputs changed during retained budget review")
    from verification.artifacts import normal
    summary['retained_reports']={name:normal(REPORT_DIR/f'{name}-build.json') for name in MODES}
    summary['retained_commands']={name:normal(REPORT_DIR/f'{name}-budget.result.json') for name in MODES}
    summary['retained_dependency_report']=normal(REPORT_DIR/'dependency-budget.stdout')
    return summary


def _check_captured_output(result: dict[str, Any], stdout_path: Path, stderr_path: Path) -> None:
    for key, path in (("stdout", stdout_path), ("stderr", stderr_path)):
        reference = result.get(key)
        _need(isinstance(reference, dict) and path.is_file(), f"retained {key} log is missing")
        data = path.read_bytes()
        _need(reference.get("sha256") == hashlib.sha256(data).hexdigest()
              and reference.get("bytes") == len(data), f"retained {key} log differs from command result")


def _check_captured_build_command(name: str, result: dict[str, Any],
                                  report: dict[str, Any], directory: Path) -> None:
    _need(result.get("exit_code") == 0, f"{name} build-budget command failed")
    command = json.loads((directory / f"{name}-budget.command.json").read_text())
    _need(command.get("argv") == result.get("argv") and command.get("cwd") == result.get("cwd"),
          f"{name} command declaration/result differ")
    stdout_path = directory / f"{name}-budget.stdout"
    stderr_path = directory / f"{name}-budget.stderr"
    _check_captured_output(result, stdout_path, stderr_path)
    try:
        printed = json.loads(stdout_path.read_text())
    except json.JSONDecodeError as error:
        raise ValueError(f"{name} build command output is not JSON") from error
    printed_fields = ("mode", "features", "clean_seconds", "unchanged_seconds", "leaf_seconds",
                      "runtime_leaf_seconds", "release_seconds", "stripped_bytes",
                      "clean_peak_rss_kib", "source_stable", "budget_pass")
    _need(set(printed) == set(printed_fields), f"{name} command output fields changed")
    for key in printed_fields:
        _need(printed.get(key) == report.get(key), f"{name} command output disagrees on {key}")
