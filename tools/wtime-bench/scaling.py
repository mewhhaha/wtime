#!/usr/bin/env python3
"""Run source-bound P008 scalar scaling baselines against an existing driver.

The benchmark executable and encoder are inputs; this runner never builds.
It encodes the 13 registry fixtures, checks all 76 scalar inputs through the
independent Python oracle, and stores only observations returned by the
standalone ``--scalar-driver`` protocol.
"""
from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import importlib.util
import json
import math
import os
from pathlib import Path
import platform
import subprocess
import sys
from typing import Any


DEFAULT_TOOL_DIR = Path(__file__).resolve().parent
MODE = "decoded"
STAGES = ("prepare", "instantiate", "first_call", "warm_call", "load_and_call")
PROFILE_ALL = "all-phase3-core"
PROFILE_MVP = "mvp"
PROBE_TRAPS = {
    "probe_null": "NullReference",
    "probe_mismatch": "IndirectCallTypeMismatch",
    "depth_probe": "ResourceLimit",
}


class BaselineError(RuntimeError):
    pass


def sha(path: Path) -> str:
    digest = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def _load_scalar(tool_dir: Path):
    path = tool_dir / "scalar_workloads.py"
    spec = importlib.util.spec_from_file_location("p008_scalar_registry", path)
    if spec is None or spec.loader is None:
        raise BaselineError(f"cannot load scalar workload registry: {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def _source_paths(root: Path, tool_dir: Path) -> list[Path]:
    paths = [
        root / "Cargo.toml", root / "Cargo.lock",
        tool_dir / "Cargo.toml", tool_dir / "Cargo.lock",
        tool_dir / "scalar_workloads.py", tool_dir / "workloads.py",
        tool_dir / "workloads.json",
    ]
    paths.extend((root / "crates").rglob("Cargo.toml"))
    paths.extend((root / "crates").rglob("*.rs"))
    paths.extend((tool_dir / "src").rglob("*.rs"))
    paths.extend((tool_dir / "fixtures/scalar").glob("*.wat"))
    paths.append(Path(__file__).resolve())
    return sorted({p.resolve() for p in paths if p.is_file()})


def _repo_root(tool_dir: Path) -> Path:
    for candidate in tool_dir.resolve().parents:
        if (candidate / "crates/wtime-runtime/Cargo.toml").is_file():
            return candidate
    raise BaselineError(f"cannot locate repository root above tool directory {tool_dir}")


def source_hashes(root: Path, tool_dir: Path) -> dict[str, str]:
    result = {}
    for path in _source_paths(root, tool_dir):
        try:
            key = path.relative_to(root).as_posix()
        except ValueError:
            key = f"runner:{path}"
        result[key] = sha(path)
    return result


def source_set_hash(records: dict[str, str]) -> str:
    packed = json.dumps(records, sort_keys=True, separators=(",", ":")).encode()
    return hashlib.sha256(packed).hexdigest()


def _cpu() -> tuple[int | None, list[int]]:
    try:
        allowed = sorted(os.sched_getaffinity(0))
    except (AttributeError, OSError):
        return None, []
    if not allowed:
        return None, []
    return allowed[0], allowed


def _run_process(executable: Path, args: list[str], cpu: int | None,
                 timeout: int = 180) -> subprocess.CompletedProcess[str]:
    command = [str(executable), *args]
    kwargs: dict[str, Any] = {
        "text": True, "capture_output": True, "timeout": timeout,
    }
    if cpu is not None and hasattr(os, "sched_setaffinity"):
        kwargs["preexec_fn"] = lambda: os.sched_setaffinity(0, {cpu})
    try:
        return subprocess.run(command, **kwargs)
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise BaselineError(f"command could not complete: {command!r}: {exc}") from exc


def _json_output(result: subprocess.CompletedProcess[str], command: list[str]) -> dict[str, Any]:
    if result.returncode != 0:
        raise BaselineError(
            f"command failed ({result.returncode}): {command!r}\n{result.stderr[-4000:]}"
        )
    try:
        data = json.loads(result.stdout)
    except (json.JSONDecodeError, TypeError) as exc:
        raise BaselineError(f"command returned invalid JSON: {command!r}: {result.stdout[:1000]!r}") from exc
    if not isinstance(data, dict):
        raise BaselineError(f"command JSON must be an object: {command!r}")
    return data


def _protocol(binary: Path, cpu: int | None) -> dict[str, Any]:
    args = ["--protocol"]
    result = _run_process(binary, args, cpu, timeout=30)
    data = _json_output(result, [str(binary), *args])
    if data.get("schema_version") != 2:
        raise BaselineError("benchmark --protocol must report schema_version 2")
    if data.get("instrumented") is not False:
        raise BaselineError("baseline requires protocol instrumented:false")
    if MODE not in data.get("modes", []):
        raise BaselineError(f"benchmark protocol does not advertise requested mode {MODE!r}")
    return data


def _profile(entry: dict[str, Any]) -> str:
    features = set(entry.get("required_features", []))
    if features <= {"mvp"}:
        return PROFILE_MVP
    return PROFILE_ALL


def _driver(entry: dict[str, Any]) -> str:
    family = entry["family"]
    if family == "scalar_host_call":
        return "host_tick"
    if family == "scalar_table_mutation":
        return "table_a"
    return "plain"


def _driver_args(fixture: Path, entry: dict[str, Any], input_value: int,
                 expected: int, stage: str, batch: int, profile: str,
                 driver: str) -> list[str]:
    return ["--scalar-driver", str(fixture), entry["id"], str(input_value),
            str(expected), stage, str(batch), MODE, profile, driver]


def _validate_timing(data: dict[str, Any], entry: dict[str, Any], input_value: int,
                     expected: int, stage: str, batch: int, profile: str,
                     driver: str) -> dict[str, Any]:
    identity = {
        "schema_version": 2,
        "case": entry["id"],
        "stage": stage,
        "input": f"i32:{input_value}",
        "expected": f"i32:{expected}",
        "result": f"i32:{expected}",
        "mode": MODE,
        "profile": profile,
        "driver": driver,
        "diagnostic": False,
    }
    if any(data.get(key) != wanted for key, wanted in identity.items()):
        raise BaselineError(f"observation identity/oracle mismatch for {entry['id']} {stage}: {data!r}")
    actual_batch = data.get("batch")
    if type(actual_batch) is not int or not 1 <= actual_batch <= 100_000:
        raise BaselineError(f"invalid timing batch for {entry['id']} {stage}: {actual_batch!r}")
    if batch and actual_batch != batch:
        raise BaselineError(f"batch mismatch for {entry['id']} {stage}: asked {batch}, got {actual_batch}")
    elapsed = data.get("ns_per_operation")
    if isinstance(elapsed, bool) or not isinstance(elapsed, (int, float)) or not math.isfinite(elapsed) or elapsed <= 0:
        raise BaselineError(f"invalid timing for {entry['id']} {stage}: {elapsed!r}")
    return data


def _call_timing(binary: Path, fixture: Path, entry: dict[str, Any], input_value: int,
                 expected: int, stage: str, batch: int, profile: str, driver: str,
                 cpu: int | None) -> dict[str, Any]:
    args = _driver_args(fixture, entry, input_value, expected, stage, batch, profile, driver)
    result = _run_process(binary, args, cpu)
    data = _json_output(result, [str(binary), *args])
    if data.get("status") == "unsupported":
        raise BaselineError(f"runtime reports scalar case unsupported: {entry['id']}: {data.get('reason', '')}")
    return _validate_timing(data, entry, input_value, expected, stage, batch, profile, driver)


def _call_probe(binary: Path, fixture: Path, entry: dict[str, Any], input_value: int,
                profile: str, driver: str, expected_trap: str, cpu: int | None) -> dict[str, Any]:
    args = _driver_args(fixture, entry, input_value, 0, "first_call", 1, profile, driver)
    result = _run_process(binary, args, cpu)
    data = _json_output(result, [str(binary), *args])
    if data.get("status") == "unsupported":
        raise BaselineError(f"runtime probe unavailable for {driver}: {data.get('reason', '')}")
    wanted = {
        "schema_version": 2, "case": entry["id"], "driver": driver,
        "diagnostic": True, "status": "trap_verified", "trap_kind": expected_trap,
        "mode": MODE, "profile": profile,
    }
    if any(data.get(key) != value for key, value in wanted.items()):
        raise BaselineError(f"diagnostic {driver} did not verify {expected_trap}: {data!r}")
    if not isinstance(data.get("message"), str) or not data["message"]:
        raise BaselineError(f"diagnostic {driver} omitted its actual trap message")
    if "ns_per_operation" in data or "batch" in data:
        raise BaselineError(f"diagnostic {driver} must remain outside timing samples")
    return data


def _call_table_mutation_probe(binary: Path, fixture: Path, entry: dict[str, Any],
                               input_value: int, expected_a: int, expected_b: int,
                               profile: str, cpu: int | None) -> dict[str, Any]:
    args = ["--scalar-table-probe", str(fixture), str(input_value),
            str(expected_a), str(expected_b), MODE, profile]
    result = _run_process(binary, args, cpu)
    data = _json_output(result, [str(binary), *args])
    wanted = {
        "schema_version": 2,
        "case": "scalar_table_mutation",
        "driver": "table_mutation",
        "diagnostic": True,
        "status": "mutation_verified",
        "input": f"i32:{input_value}",
        "target_a": f"i32:{expected_a}",
        "target_b": f"i32:{expected_b}",
        "target_a_after_restore": f"i32:{expected_a}",
        "mode": MODE,
        "profile": profile,
    }
    if any(data.get(key) != value for key, value in wanted.items()):
        raise BaselineError(f"table mutation/reset probe failed for {entry['id']}: {data!r}")
    if "ns_per_operation" in data or "batch" in data:
        raise BaselineError("table mutation/reset probe must remain outside timing samples")
    return data


def _encode_fixtures(encoder: Path, tool_dir: Path, output: Path, scalar: Any,
                     cpu: int | None) -> dict[str, dict[str, str]]:
    target = output / "fixtures" / "scalar"
    target.mkdir(parents=True, exist_ok=True)
    expected = scalar.WASM_SHA256
    fixtures: dict[str, dict[str, str]] = {}
    for wat_name, _ in scalar.FAMILIES.values():
        source = tool_dir / "fixtures" / "scalar" / wat_name
        dest = target / (Path(wat_name).stem + ".wasm")
        command = [str(encoder), "encode", str(source), "-o", str(dest)]
        result = _run_process(encoder, ["encode", str(source), "-o", str(dest)], cpu, timeout=180)
        if result.returncode != 0:
            raise BaselineError(f"fixture encoding failed ({result.returncode}): {command!r}\n{result.stderr[-2000:]}")
        if not dest.is_file():
            raise BaselineError(f"encoder did not create fixture: {dest}")
        digest = sha(dest)
        if digest != expected[wat_name]:
            raise BaselineError(f"encoded fixture hash mismatch for {wat_name}: {digest} != {expected[wat_name]}")
        fixtures[wat_name] = {"wat_sha256": sha(source), "wasm_sha256": digest,
                              "path": dest.relative_to(output).as_posix()}
    if len(fixtures) != 13:
        raise BaselineError(f"expected 13 distinct scalar fixtures, encoded {len(fixtures)}")
    return fixtures


def _run_wasmtime_matrix(probe_binary: Path | None, tool_dir: Path, output: Path,
                         entries: list[dict[str, Any]], scalar: Any,
                         cpu: int | None) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    by_family: dict[str, dict[str, Any]] = {}
    for entry in entries:
        by_family.setdefault(entry["family"], entry)
    if probe_binary is None:
        return [{"family": family, "status": "not_requested",
                 "reason": "--probe-binary was not supplied"} for family in by_family]
    for family, entry in by_family.items():
        if family == "scalar_host_call":
            rows.append({"family": family, "status": "unsupported",
                         "reason": "generic --capability cannot install the host.tick callback; no Wasmtime timing comparison is available"})
            continue
        input_value = entry["params"][0]["baseline"]
        expected = scalar.oracle(entry["id"], input_value)
        fixture = output / "fixtures/scalar" / Path(entry["source"]["path"]).with_suffix(".wasm").name
        args = ["--capability", str(fixture), f"i32:{input_value}", f"i32:{expected}",
                ",".join(entry["required_features"])]
        result = _run_process(probe_binary, args, cpu)
        data = _json_output(result, [str(probe_binary), *args])
        if data.get("schema_version") != 2 or data.get("status") not in {"supported", "unsupported"}:
            raise BaselineError(f"invalid Wasmtime capability response for {family}: {data!r}")
        if data["status"] == "supported" and data.get("compile_execute_checked") is not True:
            raise BaselineError(f"Wasmtime capability was not actually checked for {family}")
        if data["status"] == "unsupported" and not isinstance(data.get("reason"), str):
            raise BaselineError(f"Wasmtime unsupported capability lacks a reason for {family}")
        row = {"family": family, "status": data["status"],
               "probe_id": entry["id"], "input": input_value,
               "feature_profile": entry["required_features"],
               "response": data}
        if family == "scalar_table_mutation":
            row["custom_driver_capabilities"] = {
                "table_b": {"status": "unsupported", "reason": "generic --capability cannot call set_target(1)"},
                "probe_null": {"status": "unsupported", "reason": "generic --capability cannot invoke probe_null"},
                "probe_mismatch": {"status": "unsupported", "reason": "generic --capability cannot invoke probe_mismatch"},
            }
        rows.append(row)
    return rows


def _ensure_output_empty(output: Path) -> None:
    if output.exists() and (not output.is_dir() or any(output.iterdir())):
        raise BaselineError(f"refusing to overwrite nonempty output directory: {output}")
    output.mkdir(parents=True, exist_ok=True)


def run(args: argparse.Namespace, *, tool_dir: Path | None = None) -> tuple[int, dict[str, Any]]:
    """Run the suite and always return a report with only observed rows."""
    tool_dir = Path(tool_dir or getattr(args, "tool_dir", None) or DEFAULT_TOOL_DIR).resolve()
    root = _repo_root(tool_dir)
    output = Path(args.output).resolve()
    _ensure_output_empty(output)
    report: dict[str, Any] = {
        "schema_version": 1, "kind": "p008-scalar-scaling-baseline",
        "started_at_utc": dt.datetime.now(dt.timezone.utc).isoformat(),
        "status": "running", "passed": False,
        "settings": {"mode": MODE, "samples_per_stage": args.samples,
                      "stages": list(STAGES), "warm_call_calibration": "once per workload with batch=0, then fixed batch reused",
                      "instrumented": False},
        "observations": [], "calibrations": [], "correctness_probes": [],
        "runtime_capability_matrix": [], "wasmtime_capability_matrix": [],
        "failures": [],
    }
    cpu, allowed_cpus = _cpu()
    report["environment"] = {"platform": platform.platform(), "python": sys.version,
                             "cpu": platform.processor() or platform.machine(),
                             "selected_cpu": cpu, "allowed_cpus": allowed_cpus}
    binary = Path(args.binary).resolve()
    encoder = Path(args.encoder).resolve()
    probe_binary = Path(args.probe_binary).resolve() if args.probe_binary else None
    report["executables"] = {"binary": str(binary), "encoder": str(encoder),
                             "probe_binary": str(probe_binary) if probe_binary else None}
    before_sources: dict[str, str] = {}
    before_binary = before_encoder = before_probe = None
    scalar = None
    try:
        before_sources = source_hashes(root, tool_dir)
        before_binary = sha(binary)
        before_encoder = sha(encoder)
        before_probe = sha(probe_binary) if probe_binary else None
        report["source_hashes_before"] = before_sources
        report["source_set_sha256_before"] = source_set_hash(before_sources)
        report["binary_sha256_before"] = before_binary
        report["encoder_sha256_before"] = before_encoder
        if before_probe:
            report["probe_binary_sha256_before"] = before_probe

        scalar = _load_scalar(tool_dir)
        entries = scalar.entries()
        if len(entries) != 76:
            raise BaselineError(f"scalar registry must contain 76 entries, found {len(entries)}")
        if len({entry["id"] for entry in entries}) != len(entries):
            raise BaselineError("scalar registry contains duplicate workload IDs")
        fixture_hashes = _encode_fixtures(encoder, tool_dir, output, scalar, cpu)
        report["fixture_hashes"] = fixture_hashes
        report["entry_count"] = len(entries)
        report["oracle_expected_count"] = len(entries)
        protocol = _protocol(binary, cpu)
        report["protocol"] = protocol

        profile_success: dict[str, list[str]] = {}
        family_counts: dict[str, dict[str, Any]] = {}
        for entry in entries:
            wid = entry["id"]
            family = entry["family"]
            input_value = entry["params"][0]["baseline"]
            expected = scalar.oracle(wid, input_value)
            if type(expected) is not int:
                raise BaselineError(f"non-integer oracle result for {wid}: {expected!r}")
            profile = _profile(entry)
            driver = _driver(entry)
            fixture = output / "fixtures/scalar" / Path(entry["source"]["path"]).with_suffix(".wasm").name
            if sha(fixture) != entry["source"]["wasm_sha256"]:
                raise BaselineError(f"fixture hash changed after encoding for {wid}")
            actual_inputs = entry["params"][0]["baseline"]
            if actual_inputs != input_value:
                raise BaselineError(f"registry baseline input mismatch for {wid}")

            calibration = _call_timing(binary, fixture, entry, input_value, expected,
                                       "warm_call", 0, profile, driver, cpu)
            fixed_batch = calibration["batch"]
            report["calibrations"].append({"case": wid, "input": input_value,
                                           "expected": expected, "profile": profile,
                                           "driver": driver, "batch": fixed_batch,
                                           "ns_per_operation": calibration["ns_per_operation"]})
            profile_success.setdefault(profile, []).append(wid)
            family_counts.setdefault(family, {"entries": 0, "passed_entries": 0,
                                               "profile": profile, "drivers": sorted({driver})})
            family_counts[family]["entries"] += 1
            for stage in STAGES:
                batch = fixed_batch if stage == "warm_call" else 1
                for sample in range(args.samples):
                    observation = _call_timing(binary, fixture, entry, input_value, expected,
                                               stage, batch, profile, driver, cpu)
                    report["observations"].append({
                        "case": wid, "family": family, "input": input_value,
                        "expected": expected, "stage": stage, "sample": sample,
                        "batch": observation["batch"],
                        "ns_per_operation": observation["ns_per_operation"],
                        "mode": MODE, "profile": profile, "driver": driver,
                        "fixture_sha256": fixture_hashes[Path(entry["source"]["path"]).name]["wasm_sha256"],
                        "oracle_checked": True,
                    })
            family_counts[family]["passed_entries"] += 1

        # Table mutation and restoration are checked in the same Store and
        # excluded from timing samples.
        for entry in entries:
            if entry["family"] != "scalar_table_mutation":
                continue
            input_value = entry["params"][0]["baseline"]
            expected_a, expected_b = scalar.table_mutation_probe(input_value)
            profile = _profile(entry)
            fixture = output / "fixtures/scalar/table_controls.wasm"
            data = _call_table_mutation_probe(binary, fixture, entry, input_value,
                                              expected_a, expected_b, profile, cpu)
            report["correctness_probes"].append({
                "case": entry["id"], "driver": "table_mutation", "input": input_value,
                "expected_a": expected_a, "expected_b": expected_b,
                "restored_a": data["target_a_after_restore"],
                "status": "passed", "diagnostic": True,
                "timing_excluded": True,
                "note": "same-store target A/B mutation and A restoration are correctness-only",
            })

        table_entry = next(e for e in entries if e["family"] == "scalar_table_mutation")
        table_profile = _profile(table_entry)
        table_fixture = output / "fixtures/scalar/table_controls.wasm"
        for driver, expected_trap in (("probe_null", "NullReference"),
                                      ("probe_mismatch", "IndirectCallTypeMismatch")):
            data = _call_probe(binary, table_fixture, table_entry,
                               table_entry["params"][0]["baseline"], table_profile,
                               driver, expected_trap, cpu)
            report["correctness_probes"].append({"case": table_entry["id"],
                "driver": driver, "status": data["status"], "trap_kind": data["trap_kind"],
                "message": data["message"], "diagnostic": True, "timing_excluded": True})

        recursive = next(e for e in entries if e["family"] == "scalar_ordinary_recursion")
        recursive_fixture = output / "fixtures/scalar/ordinary_recursion.wasm"
        recursive_profile = _profile(recursive)
        for depth_input in (1024, 10000):
            data = _call_probe(binary, recursive_fixture, recursive, depth_input,
                               recursive_profile, "depth_probe", "ResourceLimit", cpu)
            report["correctness_probes"].append({"case": recursive["id"],
                "driver": "depth_probe", "input": depth_input,
                "status": data["status"], "trap_kind": data["trap_kind"],
                "message": data["message"], "diagnostic": True, "timing_excluded": True})

        # P008 explicitly contrasts constant-space tail-call behavior at a
        # deliberately low logical depth while keeping this diagnostic untimed.
        tail = next(e for e in entries if e["family"] == "scalar_tail_calls" and
                    e["params"][0]["baseline"] == 10000)
        tail_fixture = output / "fixtures/scalar/tail_calls.wasm"
        tail_input = 10000
        tail_expected = scalar.oracle(tail["id"], tail_input)
        tail_args = _driver_args(tail_fixture, tail, tail_input, tail_expected,
                                 "first_call", 1, _profile(tail), "tail_probe")
        tail_result = _run_process(binary, tail_args, cpu)
        tail_data = _json_output(tail_result, [str(binary), *tail_args])
        expected_identity = {"schema_version": 2, "case": tail["id"], "driver": "tail_probe",
                             "diagnostic": True, "status": "returned",
                             "input": f"i32:{tail_input}",
                             "expected": f"i32:{tail_expected}",
                             "result": f"i32:{tail_expected}", "mode": MODE,
                             "profile": _profile(tail)}
        if any(tail_data.get(k) != v for k, v in expected_identity.items()):
            raise BaselineError(f"low-depth 10000 tail-call probe did not return the oracle: {tail_data!r}")
        if "ns_per_operation" in tail_data or "batch" in tail_data:
            raise BaselineError("tail_probe must be diagnostic and excluded from timing rows")
        report["correctness_probes"].append({"case": tail["id"], "driver": "tail_probe",
            "input": tail_input, "expected": tail_expected,
            "result": tail_data["result"], "status": "passed", "diagnostic": True,
            "timing_excluded": True, "profile": _profile(tail)})

        report["runtime_capability_matrix"] = [
            {"profile": profile, "status": "supported_by_completed_oracle_checked_runs",
             "successful_entry_count": len(wids), "workload_ids": wids,
             "mode": MODE, "instrumented": False}
            for profile, wids in sorted(profile_success.items())
        ]
        report["runtime_family_matrix"] = [
            {"family": family, **values,
             "status": "supported" if values["entries"] == values["passed_entries"] else "incomplete"}
            for family, values in sorted(family_counts.items())
        ]
        report["wasmtime_capability_matrix"] = _run_wasmtime_matrix(
            probe_binary, tool_dir, output, entries, scalar, cpu)

        expected_rows = len(entries) * len(STAGES) * args.samples
        if len(report["observations"]) != expected_rows:
            raise BaselineError(f"incomplete timing coverage: {len(report['observations'])}/{expected_rows}")
        if sum(len(v) for v in profile_success.values()) != len(entries):
            raise BaselineError("not all 76 scalar inputs passed their runtime oracle preflight")
        if any(row["status"] != "supported" for row in report["runtime_family_matrix"]):
            raise BaselineError("runtime capability matrix includes an incomplete workload family")
        report["oracle_validation_count"] = len(report["observations"]) + len(report["calibrations"]) + len(report["correctness_probes"])
        report["expected_timing_rows"] = expected_rows
        report["source_hashes_after"] = source_hashes(root, tool_dir)
        report["source_set_sha256_after"] = source_set_hash(report["source_hashes_after"])
        report["source_stable"] = report["source_hashes_before"] == report["source_hashes_after"]
        report["binary_sha256_after"] = sha(binary)
        report["binary_stable"] = report["binary_sha256_before"] == report["binary_sha256_after"]
        report["encoder_sha256_after"] = sha(encoder)
        report["encoder_stable"] = report["encoder_sha256_before"] == report["encoder_sha256_after"]
        if probe_binary:
            report["probe_binary_sha256_after"] = sha(probe_binary)
            report["probe_binary_stable"] = report["probe_binary_sha256_before"] == report["probe_binary_sha256_after"]
        if not report["source_stable"]:
            raise BaselineError("benchmark source or fixture inputs changed during the run")
        if not report["binary_stable"] or not report["encoder_stable"] or report.get("probe_binary_stable") is False:
            raise BaselineError("an executable changed during the run")
        report["status"] = "passed"
        report["passed"] = True
        report["performance_claim"] = "none; integrity baseline only"
    except Exception as exc:
        report["failures"].append({"type": type(exc).__name__, "message": str(exc)})
        report["status"] = "failed"
        report["passed"] = False
        # Capture end identities after failures too. These are evidence, not
        # synthetic observations; observations remain only real validated rows.
        try:
            after = source_hashes(root, tool_dir)
            report["source_hashes_after"] = after
            report["source_set_sha256_after"] = source_set_hash(after)
            report["source_stable"] = bool(before_sources) and before_sources == after
        except Exception as hash_error:
            report["failures"].append({"type": type(hash_error).__name__, "message": f"final source hashing failed: {hash_error}"})
            report["source_stable"] = False
        for key, path, before in (("binary", binary, before_binary),
                                  ("encoder", encoder, before_encoder),
                                  ("probe_binary", probe_binary, before_probe)):
            if path is None or before is None:
                continue
            try:
                after_digest = sha(path)
                report[f"{key}_sha256_after"] = after_digest
                report[f"{key}_stable"] = before == after_digest
            except OSError as hash_error:
                report[f"{key}_stable"] = False
                report["failures"].append({"type": type(hash_error).__name__, "message": str(hash_error)})
    report["completed_at_utc"] = dt.datetime.now(dt.timezone.utc).isoformat()
    report["actual_observation_count"] = len(report["observations"])
    report["actual_calibration_count"] = len(report["calibrations"])
    report["actual_probe_count"] = len(report["correctness_probes"])
    report["samples_per_entry_stage"] = args.samples
    report["entry_count"] = report.get("entry_count", 0)
    report["performance_claim"] = report.get("performance_claim", "none; integrity baseline only")
    report_path = output / "report.json"
    report_path.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return (0 if report["passed"] else 1), report


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path, help="existing standalone wtime scalar-driver binary")
    parser.add_argument("--encoder", required=True, type=Path, help="existing conformance WAT encoder")
    parser.add_argument("--output", required=True, type=Path, help="new output directory; must be empty or absent")
    parser.add_argument("--samples", type=int, default=5)
    parser.add_argument("--probe-binary", type=Path, help="optional Wasmtime --capability harness executable")
    parser.add_argument("--tool-dir", type=Path, default=DEFAULT_TOOL_DIR,
                        help="directory containing scalar_workloads.py and fixtures (defaults to this script's directory)")
    args = parser.parse_args(argv)
    if args.samples <= 0 or args.samples > 1000:
        parser.error("--samples must be between 1 and 1000")
    code, report = run(args)
    print(json.dumps({"status": report["status"], "passed": report["passed"],
                      "report": str((Path(args.output).resolve() / "report.json")),
                      "entries": report.get("entry_count", 0),
                      "observations": report["actual_observation_count"],
                      "failures": len(report["failures"])}, sort_keys=True))
    return code


if __name__ == "__main__":
    raise SystemExit(main())
