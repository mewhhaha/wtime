#!/usr/bin/env python3
"""Capture separate sampling and development-counter evidence for a selection."""
from __future__ import annotations

import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
from typing import Any, Callable

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
COUNTERS = (
    "source_dispatches", "module_deep_copy_events", "module_deep_copy_bytes",
    "instruction_clone_events", "instruction_clone_bytes", "read_vec_operations",
    "read_fixed_operations", "frame_entries", "argument_allocations",
    "root_snapshot_allocations", "subtype_probes", "continuation_scan_entries",
    "branch_result_allocations", "table_copy_snapshot_chunks", "bulk_slice_chunks",
)
COUNTER_BATCH = 1
PROFILE_BATCH = 256


class ProfileError(RuntimeError):
    """A workload/build failed and therefore cannot produce successful evidence."""


def sha(path: str | Path) -> str:
    digest = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def source_inputs() -> dict[str, str]:
    paths = [ROOT / "Cargo.toml", ROOT / "Cargo.lock", HERE / "Cargo.toml", HERE / "Cargo.lock",
             HERE / "workloads.json", HERE / "workloads.py", HERE / "profile.py", HERE / "run.py"]
    paths += list(ROOT.glob("crates/**/*.rs")) + list(ROOT.glob("crates/**/Cargo.toml"))
    paths += list((HERE / "src").glob("*.rs"))
    paths += list((HERE / "profiling").glob("Cargo.*")) + list((HERE / "profiling/src").glob("*.rs"))
    return {str(path.relative_to(ROOT)): sha(path) for path in sorted(set(paths)) if path.is_file()}


def _affinity(cpu: int) -> Callable[[], None]:
    def set_affinity() -> None:
        os.sched_setaffinity(0, {cpu})
    return set_affinity


def _json_command(command: list[str], runner: Callable[..., Any], *, cpu: int,
                  timeout: int = 300) -> dict[str, Any]:
    try:
        result = runner(command, cwd=ROOT, text=True, capture_output=True, timeout=timeout,
                        preexec_fn=_affinity(cpu))
    except (OSError, subprocess.SubprocessError) as exc:
        raise ProfileError(f"command could not run: {command[0]}: {exc}") from exc
    if result.returncode != 0:
        raise ProfileError(f"workload command failed ({result.returncode}): {result.stderr[-4000:]}")
    try:
        value = json.loads(result.stdout)
    except (json.JSONDecodeError, TypeError) as exc:
        raise ProfileError(f"workload produced invalid JSON: {exc}") from exc
    if not isinstance(value, dict):
        raise ProfileError("workload response must be a JSON object")
    return value


def validate_counter_response(data: dict[str, Any], expected: Any) -> dict[str, int]:
    if data.get("schema_version") != 1 or data.get("diagnostic") is not True:
        raise ProfileError("counter response is not diagnostic schema version 1")
    if "result" not in data or data["result"] != expected:
        raise ProfileError("diagnostic workload result differs from its oracle")
    counters = data.get("counters")
    if not isinstance(counters, dict):
        raise ProfileError("diagnostic response has no counters object")
    normalized = {}
    for key in COUNTERS:
        value = counters.get(key)
        if isinstance(value, bool) or not isinstance(value, int) or value < 0:
            raise ProfileError(f"counter {key} must be a nonnegative integer")
        normalized[key] = value
    return normalized


def probe_tools(output: Path, *, which: Callable[[str], str | None] = shutil.which,
                runner: Callable[..., Any] = subprocess.run) -> dict[str, Any]:
    """Discover profiler capability without installation or system changes."""
    tools: dict[str, Any] = {}
    for name in ("perf", "llvm-symbolizer", "addr2line", "gdb"):
        location = which(name)
        tools[name] = {"available": location is not None, "path": location}
    paranoid = Path("/proc/sys/kernel/perf_event_paranoid")
    try:
        tools["perf_event_paranoid"] = paranoid.read_text().strip()
    except OSError:
        tools["perf_event_paranoid"] = "unavailable"
    version_args = {"perf": ["--version"], "llvm-symbolizer": ["--version"],
                    "addr2line": ["--version"], "gdb": ["--version"]}
    for name, args in version_args.items():
        path = tools[name].get("path")
        if not path:
            continue
        try:
            version = runner([path, *args], cwd=ROOT, text=True, capture_output=True, timeout=10)
            tools[name]["version"] = (version.stdout or version.stderr).splitlines()[0:4]
        except (OSError, subprocess.SubprocessError) as exc:
            tools[name]["version_error"] = str(exc)
    for name, command in (("rustc", ["rustc", "-Vv"]), ("cargo", ["cargo", "-V"])):
        try:
            result = runner(command, cwd=ROOT, text=True, capture_output=True, timeout=10)
            tools[name] = {"available": result.returncode == 0,
                           "version": (result.stdout or result.stderr).strip()}
        except (OSError, subprocess.SubprocessError) as exc:
            tools[name] = {"available": False, "version_error": str(exc)}
    perf = tools["perf"].get("path")
    if not perf:
        tools["sampling"] = {"available": False, "reason": "perf executable is unavailable"}
        return tools
    try:
        probe_file = output / "perf-probe.txt"
        check = runner([perf, "stat", "-o", str(probe_file), "--", "/bin/true"], cwd=ROOT,
                       text=True, capture_output=True, timeout=15)
        if check.returncode == 0:
            tools["sampling"] = {"available": True, "probe": "perf stat /bin/true succeeded"}
        else:
            tools["sampling"] = {"available": False,
                                 "reason": (check.stderr or "perf stat probe failed").strip()[-2000:]}
    except (OSError, subprocess.SubprocessError) as exc:
        tools["sampling"] = {"available": False, "reason": str(exc)}
    return tools


def _build(target: Path, *, diagnostic: bool, runner: Callable[..., Any]) -> tuple[list[str], dict[str, str], Path]:
    command = ["cargo", "build", "--locked", "--release", "--manifest-path",
               str(HERE / "profiling/Cargo.toml"), "--target-dir", str(target)]
    env = dict(os.environ)
    if diagnostic:
        command += ["--features", "performance-counters"]
    else:
        env["CARGO_PROFILE_RELEASE_DEBUG"] = "1"
        env["RUSTFLAGS"] = "-Cforce-frame-pointers=yes"
    try:
        result = runner(command, cwd=ROOT, env=env, text=True, capture_output=True, timeout=1800)
    except (OSError, subprocess.SubprocessError) as exc:
        raise ProfileError(f"build could not run: {exc}") from exc
    if result.returncode != 0:
        kind = "counter" if diagnostic else "sampling"
        raise ProfileError(f"{kind} build failed ({result.returncode}): {result.stderr[-4000:]}")
    binary = target / "release" / ("wtime-bench.exe" if os.name == "nt" else "wtime-bench")
    if not binary.is_file():
        raise ProfileError(f"build succeeded but binary is missing: {binary}")
    return command, {key: env[key] for key in ("CARGO_PROFILE_RELEASE_DEBUG", "RUSTFLAGS") if key in env}, binary


def _record_perf(tools: dict[str, Any], output: Path, workload: dict[str, Any], fixture: Path,
                 binary: Path, input_value: int, expected: Any, cpu: int,
                 runner: Callable[..., Any]) -> dict[str, Any]:
    sampling = tools.get("sampling", {})
    row: dict[str, Any] = {"workload": workload["id"], "available": False}
    if not sampling.get("available"):
        row["capability_gap"] = sampling.get("reason", "perf sampling is unavailable")
        return row
    perf = tools["perf"]["path"]
    data_path = output / "sampling" / f"{workload['id']}.perf.data"
    report_path = output / "sampling" / f"{workload['id']}.perf-report.txt"
    script_path = output / "sampling" / f"{workload['id']}.perf-script.txt"
    command = [str(binary), "--single-case", str(fixture), workload["id"], str(input_value),
               str(expected), "warm_call", str(PROFILE_BATCH), "decoded"]
    record_cmd = [perf, "record", "--call-graph", "dwarf", "-F", "99", "-o", str(data_path), "--", *command]
    result = runner(record_cmd, cwd=ROOT, text=True, capture_output=True, timeout=600,
                    preexec_fn=_affinity(cpu))
    row["command"] = record_cmd
    if result.returncode != 0:
        row["capability_gap"] = (result.stderr or "perf record failed").strip()[-2000:]
        return row
    report_cmd = [perf, "report", "--stdio", "--input", str(data_path), "--sort", "symbol,dso"]
    report = runner(report_cmd, cwd=ROOT, text=True, capture_output=True, timeout=60)
    script_cmd = [perf, "script", "--input", str(data_path)]
    script = runner(script_cmd, cwd=ROOT, text=True, capture_output=True, timeout=60)
    if report.returncode != 0 or script.returncode != 0:
        row["capability_gap"] = "perf data recorded but report/script symbolization failed"
        row["data_file"] = str(data_path.relative_to(output)) if data_path.exists() else None
        return row
    report_path.write_text(report.stdout, encoding="utf-8")
    script_path.write_text(script.stdout, encoding="utf-8")
    row.update(available=True, data_file=str(data_path.relative_to(output)), data_sha256=sha(data_path),
               report_file=str(report_path.relative_to(output)), script_file=str(script_path.relative_to(output)),
               report_sha256=sha(report_path), script_sha256=sha(script_path),
               record_command=record_cmd, report_command=report_cmd, script_command=script_cmd,
               sampled_batch=PROFILE_BATCH,
               binary_sha256=sha(binary),
               attribution_note="CPU samples are profile evidence, not production timing results")
    return row


def run_profile(selection_name: str, output: str | Path, *, cpu: int | None = None,
                runner: Callable[..., Any] = subprocess.run,
                which: Callable[[str], str | None] = shutil.which) -> dict[str, Any]:
    import run as original
    import workloads

    if selection_name in {'scalar', 'memory', 'representative'}:
        import profile_routes
        return profile_routes.run_profile(selection_name, output, cpu=cpu, runner=runner, which=which)

    out = Path(output).resolve()
    out.mkdir(parents=True, exist_ok=False)
    workloads_selected = workloads.selection(selection_name)
    allowed = sorted(os.sched_getaffinity(0))
    selected_cpu = allowed[0] if cpu is None else cpu
    if selected_cpu not in allowed:
        raise ProfileError("requested CPU is outside available affinity")
    sampling_target = out / "sampling-target"
    counters_target = out / "counter-target"
    report: dict[str, Any] = {
        "schema_version": 1, "passed": False, "selection": selection_name,
        "recorded_at_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "metadata": {"host": {"platform": platform.platform(), "cpu_affinity_available": allowed,
                               "selected_cpu": selected_cpu,
                               "cpu_quota": _read_text(Path("/sys/fs/cgroup/cpu.max"), "unavailable")},
                     "tools": {}, "source_inputs": {}, "artifacts": {},
                     "sampling_policy": {"profile": "release with debug symbols",
                                         "CARGO_PROFILE_RELEASE_DEBUG": "1",
                                         "RUSTFLAGS": "-Cforce-frame-pointers=yes",
                                         "target_dir": str(sampling_target)},
                     "counter_policy": {"feature": "performance-counters", "instrumented": True,
                                        "target_dir": str(counters_target),
                                        "instrumentation_bias": "diagnostic event counts only; not comparable latency"}},
        "counters": [], "cpu_profiles": [], "limitations": [
            "Hardware samples and development counters come from separate binaries.",
            "Counters are attribution clues and do not establish production CPU percentages.",
        ],
    }
    report_path = out / "profile.json"
    try:
        report["metadata"]["source_inputs"] = source_inputs()
        tools = probe_tools(out, which=which, runner=runner)
        report["metadata"]["tools"] = tools
        sample_command, sample_flags, sample_binary = _build(sampling_target, diagnostic=False, runner=runner)
        counter_command, counter_flags, counter_binary = _build(counters_target, diagnostic=True, runner=runner)
        report["metadata"]["artifacts"] = {
            "sampling": {"binary": str(sample_binary.relative_to(out)), "binary_sha256": sha(sample_binary),
                         "build_command": sample_command, "build_flags": sample_flags,
                         "target_dir": str(sampling_target.relative_to(out))},
            "counters": {"binary": str(counter_binary.relative_to(out)), "binary_sha256": sha(counter_binary),
                         "build_command": counter_command,
                         "build_flags": {**counter_flags, "features": "performance-counters"},
                         "target_dir": str(counters_target.relative_to(out))},
        }
        report["metadata"]["artifacts"]["distinct_binaries"] = sample_binary.resolve() != counter_binary.resolve()
        if not report["metadata"]["artifacts"]["distinct_binaries"]:
            raise ProfileError("sampling and counter binaries share an artifact path")
        (out / "sampling").mkdir()
        (out / "counters").mkdir()
        for workload in workloads_selected:
            name = workload["id"]
            fixture = HERE / "fixtures" / f"{name}.wasm"
            fixture_hash = sha(fixture)
            if fixture_hash != workload["source"]["wasm_sha256"]:
                raise ProfileError(f"{name}: fixture hash differs from workload registry")
            params = workload["params"]
            input_value = params[0]["baseline"] if params else 0
            expected = original.oracle(name, input_value) if workload["result"]["type"] != "none" else None
            command = [str(counter_binary), "--diagnostic", str(fixture), str(input_value),
                       json.dumps(expected), "decoded", "mvp"]
            # The diagnostic protocol takes expected as its CLI scalar; null is passed as JSON null.
            observation = _json_command(command, runner, cpu=selected_cpu)
            counters = validate_counter_response(observation, expected)
            counter_file = out / "counters" / f"{name}.json"
            original_work = workload["work_units"]
            unit_count = original_work.get("count", input_value if "count_from_param" in original_work else None)
            counter_row = {"workload": name, "fixture_sha256": fixture_hash, "input": input_value,
                           "expected": expected, "binary_sha256": report["metadata"]["artifacts"]["counters"]["binary_sha256"],
                           "command": command, "work_unit": original_work["unit"], "work_unit_count": unit_count,
                           "counters": counters,
                           "source_dispatches_per_work_unit": (counters["source_dispatches"] / unit_count
                                                               if isinstance(unit_count, int) and unit_count > 0 else None)}
            counter_file.write_text(json.dumps(counter_row, indent=2, sort_keys=True) + "\n", encoding="utf-8")
            counter_row["artifact"] = str(counter_file.relative_to(out))
            counter_row["artifact_sha256"] = sha(counter_file)
            report["counters"].append(counter_row)
            report["cpu_profiles"].append(_record_perf(tools, out, workload, fixture, sample_binary,
                                                       input_value, expected, selected_cpu, runner))
        if not report["counters"]:
            raise ProfileError("selection produced no counter cases")
        if report["metadata"]["source_inputs"] != source_inputs():
            raise ProfileError("source inputs changed during profiling")
        for artifact_name, binary in (("sampling", sample_binary), ("counters", counter_binary)):
            if sha(binary) != report["metadata"]["artifacts"][artifact_name]["binary_sha256"]:
                raise ProfileError(f"{artifact_name} binary changed during profiling")
        report["passed"] = True
    except Exception as exc:
        report["error"] = str(exc)
        report_path.write_text(json.dumps(report, indent=2, sort_keys=True, allow_nan=False) + "\n", encoding="utf-8")
        if isinstance(exc, ProfileError):
            raise
        raise ProfileError(str(exc)) from exc
    report_path.write_text(json.dumps(report, indent=2, sort_keys=True, allow_nan=False) + "\n", encoding="utf-8")
    return report


def _read_text(path: Path, fallback: str) -> str:
    try:
        return path.read_text().strip()
    except OSError:
        return fallback


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--selection", required=True)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--cpu", type=int)
    args = parser.parse_args(argv)
    try:
        result = run_profile(args.selection, args.output, cpu=args.cpu)
    except (ProfileError, OSError, ValueError) as exc:
        print(f"profile.py: {exc}", file=sys.stderr)
        return 2
    print(f"profile complete: {len(result['counters'])} counter cases; "
          f"{sum(1 for item in result['cpu_profiles'] if item.get('available'))} sampled profiles")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
