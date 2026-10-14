#!/usr/bin/env python3
"""Inert, source-bound build/compare/analyze/gate runner for compact P039/P040."""
from __future__ import annotations
import argparse, hashlib, json, os, shutil, subprocess, sys
sys.dont_write_bytecode = True
from pathlib import Path

TASKS = {"P039": ("simd", 27), "P040": ("scalar", 26)}
SCHEMAS = {"P039": "p039-source-bound-check-v1", "P040": "p040-source-bound-check-v1"}
RETAINED_GATE_SHA256 = "2f3c27018176523f7c6a488da2a32803ee93620b26191c005529607c2f906aa7"
ROOT = Path("/workspace/wtime")

class PhaseError(RuntimeError):
    pass

def need(ok: bool, message: str) -> None:
    if not ok:
        raise PhaseError(message)

def sha(path: Path) -> str:
    h = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()

def json_obj(path: Path, label: str) -> dict:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except Exception as exc:
        raise PhaseError(f"{label} is not valid JSON: {exc}") from exc
    need(isinstance(value, dict), f"{label} must be a JSON object")
    return value

def runtime_map(root: Path) -> dict[str, str]:
    src = root / "crates/wtime-runtime/src"
    need(src.is_dir(), f"runtime source directory missing: {src}")
    return {p.relative_to(root).as_posix(): sha(p) for p in sorted(src.rglob("*.rs"))}

def candidate_source_map(task: str, root: Path) -> dict[str, str]:
    bench = root / "tools/wtime-bench"
    if str(bench) not in sys.path:
        sys.path.insert(0, str(bench))
    import compare
    value = compare.sources(include_simd=(task == "P039"))
    need(isinstance(value, dict) and value, "compare.sources returned an empty source identity")
    return value

def phase_tool_map(root: Path) -> dict[str, str]:
    paths = [root / "tools/wtime-bench" / name for name in
             ("compare.py", "analyze.py", "gates.py", "baseline.py")]
    paths += [root / "tools/checks" / "build_budget.py"]
    need(all(path.is_file() for path in paths), "phase helper source is missing")
    return {path.relative_to(root).as_posix(): sha(path) for path in paths}

def validate_guard(source_before: dict[str, str], tools_before: dict[str, str],
                   artifact_before: dict[str, str], root: Path, task: str) -> None:
    need(candidate_source_map(task, root) == source_before,
         "compare.sources candidate identity changed during phase")
    need(phase_tool_map(root) == tools_before,
         "phase tool sources changed during phase")
    for name, digest in artifact_before.items():
        path = Path(name)
        need(path.is_file() and sha(path) == digest,
             f"phase input identity changed during phase: {path}")

def phase_artifacts(args, root: Path, helper: Path, check_path: Path,
                    build_record_path: Path | None, contract_path: Path | None) -> dict[str, str]:
    paths = [helper, check_path]
    if build_record_path is not None:
        record = json_obj(build_record_path, "candidate build record")
        paths.append(build_record_path)
        for key in ("binary_path",):
            if isinstance(record.get(key), str): paths.append(Path(record[key]).resolve(strict=True))
        for filename in ("helper.executed.py", "stdout.log", "stderr.log", "protocol.stdout.json", "protocol.stderr.log"):
            candidate = build_record_path.parent / filename
            if candidate.exists(): paths.append(candidate.resolve())
    if contract_path is not None: paths.append(contract_path)
    if args.gate_validator is not None: paths.append(args.gate_validator.resolve(strict=True))
    if args.phase in ("analyze", "gate"):
        comparison = args.comparison.resolve(strict=True) if args.comparison else root / "target/performance" / args.task / "comparison"
        report = comparison / "report.json"
        if report.is_file(): paths.append(report.resolve())
    paths.append(Path(__file__).resolve())
    return {str(path.resolve(strict=True)): sha(path.resolve(strict=True)) for path in paths}

def check_inputs(task: str, path: Path, root: Path) -> tuple[dict, dict[str, str]]:
    selection, count = TASKS[task]
    proof = json_obj(path, "source-bound check")
    need(proof.get("schema") == SCHEMAS[task] and proof.get("phase") == "check",
         f"check result must use {SCHEMAS[task]}/check")
    need(proof.get("passed") is True and proof.get("status") == "passed" and proof.get("source_stable") is True,
         "source-bound check did not pass stably")
    mapping = proof.get("runtime_source_map")
    need(isinstance(mapping, dict) and proof.get("runtime_source_count") == count and len(mapping) == count,
         f"check must bind exactly {count} runtime files")
    need(all(isinstance(k, str) and isinstance(v, str) and len(v) == 64 for k, v in mapping.items()),
         "check runtime source map is malformed")
    current = runtime_map(root)
    need(current == mapping, "live runtime sources differ from source-bound check")
    builds = proof.get("builds")
    need(isinstance(builds, dict) and {"default", "optional"} <= set(builds),
         "source-bound check lacks default/optional build reports")
    for name in ("default", "optional"):
        ref = builds[name]
        need(isinstance(ref, dict) and isinstance(ref.get("path"), str), f"{name} build reference missing")
        report_path = Path(ref["path"]).resolve(strict=True)
        need(sha(report_path) == ref.get("sha256") and report_path.stat().st_size == ref.get("bytes"),
             f"{name} build reference is stale")
        report = json_obj(report_path, f"{name} build report")
        need(report.get("source_stable") is True and report.get("budget_pass") is True,
             f"{name} build report did not pass")
        expected_mode = "default" if name == "default" else "optional-feature"
        expected_features = [] if name == "default" else ["wtime-runtime/compact-executor"]
        need(report.get("mode") == expected_mode and report.get("features") == expected_features,
             f"{name} build report has unexpected mode/features")
    return proof, mapping

def next_attempt(base: Path) -> Path:
    base.mkdir(parents=True, exist_ok=True)
    for index in range(1, 10000):
        path = base / f"attempt-{index:04d}"
        try:
            path.mkdir(exist_ok=False)
            return path
        except FileExistsError:
            continue
    raise PhaseError(f"attempt directory limit reached: {base}")

def write_new(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("x", encoding="utf-8") as stream:
        stream.write(text)

def run(argv: list[str], cwd: Path, env: dict[str, str]) -> tuple[subprocess.CompletedProcess, str]:
    result = subprocess.run(argv, cwd=cwd, env=env, text=True, capture_output=True, check=False)
    return result, "\n".join(argv)

def normal_ref(path: Path) -> dict:
    path = path.resolve(strict=True)
    return {"path": str(path), "sha256": sha(path), "bytes": path.stat().st_size}

def copy_exclusive(src: Path, dst: Path) -> Path:
    dst.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(src, dst)
    need(sha(src) == sha(dst), f"copy changed bytes: {src}")
    return dst.resolve()

def protocol_output(path: Path, root: Path, attempt: Path) -> tuple[list[str], int, str, str]:
    cmd = [str(path), "--protocol"]
    proc = subprocess.run(cmd, cwd=root, text=True, capture_output=True, check=False)
    write_new(attempt / "protocol.stdout.json", proc.stdout)
    write_new(attempt / "protocol.stderr.log", proc.stderr)
    return cmd, proc.returncode, proc.stdout, proc.stderr

def execute_phase(args) -> dict:
    task = args.task
    selection, _count = TASKS[task]
    root = args.workspace_root.resolve(strict=True)
    need((root / "Cargo.toml").is_file(), "workspace root is not wtime")
    check_path = args.check_result.resolve(strict=True)
    check, runtime_before = check_inputs(task, check_path, root)
    check_ref = normal_ref(check_path)
    out_root = root / "target/performance" / task
    attempt = next_attempt(out_root / "production" / args.phase)
    helper_copy = copy_exclusive(Path(__file__).resolve(), attempt / "helper.executed.py")
    build_record_path = args.build_record.resolve(strict=True) if args.build_record else None
    contract_path = args.contract.resolve(strict=True) if args.contract else None
    gate_validator_path = args.gate_validator.resolve(strict=True) if args.gate_validator else None
    snapshots = {"check_result": copy_exclusive(check_path, attempt / "inputs/check-result.json")}
    if build_record_path is not None:
        snapshots["candidate_build_record"] = copy_exclusive(build_record_path, attempt / "inputs/build-record.json")
    if contract_path is not None:
        snapshots["contract"] = copy_exclusive(contract_path, attempt / "inputs/contract.json")
    gate_validator_copy = None
    if gate_validator_path is not None:
        need(sha(gate_validator_path) == RETAINED_GATE_SHA256,
             "gate validator is not the reviewed retained source-bound extension")
        gate_validator_copy = copy_exclusive(gate_validator_path, attempt / "inputs/gates.py")
        snapshots["gate_validator"] = gate_validator_copy
    snapshots = {key: normal_ref(value) for key, value in snapshots.items()}
    full_sources_before = candidate_source_map(task, root)
    tools_before = phase_tool_map(root)
    artifacts_before = phase_artifacts(args, root, helper_copy, check_path, build_record_path, contract_path)
    for snapshot in snapshots.values():
        artifacts_before[snapshot["path"]] = snapshot["sha256"]
    env = os.environ.copy()
    env["PYTHONDONTWRITEBYTECODE"] = "1"
    record = {"schema": "wtime-production-command-v1", "task": task, "phase": args.phase,
              "check_result": check_ref, "helper_sha256": sha(helper_copy),
              "runtime_source_map": runtime_before, "candidate_source_map": full_sources_before,
              "phase_tool_sha256": tools_before, "input_snapshots": snapshots,
              "source_stable": False, "cwd": str(root), "command": [], "exit_code": None,
              "binary_path": None, "binary_sha256": None, "protocol_command": [],
              "protocol_exit": None, "stdout_sha256": None, "stderr_sha256": None,
              "attempt_dir": str(attempt.resolve())}
    stdout = stderr = ""
    try:
        if args.phase == "build":
            command = ["cargo", "build", "--locked", "--release", "--manifest-path",
                       "tools/wtime-bench/Cargo.toml", "--no-default-features", "--features",
                       "compact-executor", "-j4"]
            wrapped = ["bash", "-lc", ". /workspace/.setup/rust-env.sh && exec " + " ".join(command)]
            validate_guard(full_sources_before, tools_before, artifacts_before, root, task)
            proc, _ = run(wrapped, root, env)
            stdout, stderr = proc.stdout, proc.stderr
            record["command"] = command
            record["execution_command"] = wrapped
            record["exit_code"] = proc.returncode
            validate_guard(full_sources_before, tools_before, artifacts_before, root, task)
            need(proc.returncode == 0, f"candidate Cargo build failed ({proc.returncode})")
            built = root / "tools/wtime-bench/target/release/wtime-bench"
            need(built.is_file(), f"release binary missing: {built}")
            retained = copy_exclusive(built, attempt / "binary/wtime-bench")
            need(os.access(retained, os.X_OK), "retained candidate binary is not executable")
            validate_guard(full_sources_before, tools_before, artifacts_before, root, task)
            pcmd, pexit, protocol_stdout, protocol_stderr = protocol_output(retained, root, attempt)
            record.update(binary_path=str(retained), binary_sha256=sha(retained),
                          protocol_command=pcmd, protocol_exit=pexit,
                          build_reports=check["builds"],
                          protocol_stdout_sha256=sha(attempt / "protocol.stdout.json"))
            need(pexit == 0, f"candidate --protocol failed ({pexit})")
            try:
                protocol = json.loads(protocol_stdout)
            except Exception as exc:
                raise PhaseError(f"candidate protocol is not JSON: {exc}") from exc
            need(protocol.get("instrumented") is False, "candidate protocol is instrumented")
            need("compact" in protocol.get("modes", []), "candidate protocol does not advertise compact")
            record["protocol"] = protocol
            validate_guard(full_sources_before, tools_before, artifacts_before, root, task)
        elif args.phase == "compare":
            need(args.build_record is not None, "compare requires --build-record")
            build_record_path = args.build_record.resolve(strict=True)
            br = json_obj(build_record_path, "candidate build record")
            need(br.get("schema") == "wtime-production-command-v1" and br.get("task") == task
                 and br.get("phase") == "build" and br.get("exit_code") == 0
                 and br.get("source_stable") is True, "build record is not a successful source-stable candidate build")
            binary = Path(br["binary_path"]).resolve(strict=True)
            need(sha(binary) == br.get("binary_sha256"), "retained candidate binary hash mismatch")
            need(br.get("runtime_source_map") == runtime_before, "candidate build runtime identity differs from check")
            comparison = out_root / "comparison"
            need(not comparison.exists(), f"comparison output already exists: {comparison}")
            command = ["python3", "-B", "tools/wtime-bench/compare.py", "--baseline",
                       f"target/performance/{task}-before", "--candidate", str(binary),
                       "--selection", selection, "--output", str(comparison), "--pairs", "30",
                       "--candidate-mode", "compact"]
            validate_guard(full_sources_before, tools_before, artifacts_before, root, task)
            proc, _ = run(command, root, env)
            stdout, stderr = proc.stdout, proc.stderr
            record.update(command=command, exit_code=proc.returncode)
            validate_guard(full_sources_before, tools_before, artifacts_before, root, task)
            record.update(binary_path=str(binary), binary_sha256=sha(binary),
                          build_record=normal_ref(build_record_path), comparison_output=str(comparison))
            need(proc.returncode == 0, f"paired comparison failed ({proc.returncode})")
            report_path = comparison / "report.json"
            report = json_obj(report_path, "comparison report")
            need(report.get("passed") is True and report.get("source_stable") is True
                 and report.get("selection") == selection, "comparison report failed or mismatches selection")
            metadata = report.get("metadata", {})
            need(metadata.get("candidate_sources") == full_sources_before,
                 "comparison report candidate source map differs from producer guard")
            settings = metadata.get("settings", {})
            need(settings.get("candidate_mode") == "compact" and settings.get("instrumented") is False,
                 "comparison was not uninstrumented compact mode")
        elif args.phase == "analyze":
            comparison = args.comparison.resolve(strict=True) if args.comparison else out_root / "comparison"
            report = json_obj(comparison / "report.json", "comparison report")
            need(report.get("passed") is True and report.get("source_stable") is True,
                 "analysis requires passed source-stable comparison")
            analysis_path = comparison / "analysis.json"
            need(not analysis_path.exists(), f"analysis output already exists: {analysis_path}")
            command = ["python3", "-B", "tools/wtime-bench/analyze.py", "--input",
                       str(comparison), "--output", str(analysis_path)]
            validate_guard(full_sources_before, tools_before, artifacts_before, root, task)
            proc, _ = run(command, root, env)
            stdout, stderr = proc.stdout, proc.stderr
            record.update(command=command, exit_code=proc.returncode)
            validate_guard(full_sources_before, tools_before, artifacts_before, root, task)
            record.update(comparison_output=str(comparison),
                          analysis_output=str(analysis_path))
            need(proc.returncode == 0 and analysis_path.is_file(), f"analysis failed ({proc.returncode})")
        else:
            need(args.build_record is not None, "gate requires --build-record")
            build_record_path = args.build_record.resolve(strict=True)
            br = json_obj(build_record_path, "candidate build record")
            need(br.get("schema") == "wtime-production-command-v1" and br.get("task") == task
                 and br.get("phase") == "build" and br.get("exit_code") == 0
                 and br.get("source_stable") is True and br.get("runtime_source_map") == runtime_before,
                 "gate build record is not successful/current")
            binary = Path(br["binary_path"]).resolve(strict=True)
            need(sha(binary) == br.get("binary_sha256"), "candidate binary identity changed")
            comparison = args.comparison.resolve(strict=True) if args.comparison else out_root / "comparison"
            need(contract_path is not None, "gate requires --contract")
            need(gate_validator_copy is not None, "gate requires --gate-validator; never use the live gate implicitly")
            contract = contract_path
            task_row = next((row for row in json_obj(root.parent / "tasks/performance/TASKS.json", "task registry")["tasks"]
                             if row.get("id") == task), None)
            need(task_row is not None, "task absent from TASKS.json")
            decision = out_root / "decision.json"
            if decision.exists():
                copy_exclusive(decision, attempt / "previous-decision.json")
            builds = check["builds"]
            command = ["python3", "-B", str(gate_validator_copy), "--task", task,
                       "--comparison", str(comparison), "--build", builds["default"]["path"],
                       "--optional-build", builds["optional"]["path"],
                       "--candidate-build-record", str(build_record_path), "--contract", str(contract),
                       "--output", str(decision)]
            env["WTIME_GATE_WORKSPACE_ROOT"] = str(root)
            record["execution_env"] = {"WTIME_GATE_WORKSPACE_ROOT": str(root)}
            record["gate_validator"] = normal_ref(gate_validator_copy)
            validate_guard(full_sources_before, tools_before, artifacts_before, root, task)
            proc, _ = run(command, root, env)
            stdout, stderr = proc.stdout, proc.stderr
            record.update(command=command, exit_code=proc.returncode)
            validate_guard(full_sources_before, tools_before, artifacts_before, root, task)
            record.update(binary_path=str(binary),
                          binary_sha256=sha(binary), build_record=normal_ref(build_record_path),
                          comparison_output=str(comparison), decision_output=str(decision))
            need(proc.returncode in (0, 1) and decision.is_file(),
                 f"gate failed before recording a decision ({proc.returncode})")
        need(runtime_map(root) == runtime_before, "runtime sources changed during phase")
        validate_guard(full_sources_before, tools_before, artifacts_before, root, task)
        record["source_stable"] = True
    finally:
        write_new(attempt / "stdout.log", stdout)
        write_new(attempt / "stderr.log", stderr)
        record["stdout_sha256"] = sha(attempt / "stdout.log")
        record["stderr_sha256"] = sha(attempt / "stderr.log")
        record["source_after"] = runtime_map(root)
        try:
            record["candidate_source_map_after"] = candidate_source_map(task, root)
            record["phase_tool_sha256_after"] = phase_tool_map(root)
            record["artifact_sha256_after"] = {name: sha(Path(name)) for name in artifacts_before}
            full_stable = (record["candidate_source_map_after"] == full_sources_before
                           and record["phase_tool_sha256_after"] == tools_before
                           and record["artifact_sha256_after"] == artifacts_before)
        except Exception:
            full_stable = False
        record["source_stable"] = (record.get("source_stable") is True
                                   and record["source_after"] == runtime_before and full_stable)
        write_new(attempt / "record.json", json.dumps(record, indent=2, sort_keys=True) + "\n")
    need(record["source_stable"], "runtime sources changed during phase; record retained as failed")
    return record

def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--task", choices=sorted(TASKS), required=True)
    parser.add_argument("--phase", choices=("build", "compare", "analyze", "gate"), required=True)
    parser.add_argument("--check-result", type=Path, required=True)
    parser.add_argument("--workspace-root", type=Path, default=ROOT)
    parser.add_argument("--candidate-build-record", "--build-record", dest="build_record", type=Path)
    parser.add_argument("--comparison", type=Path)
    parser.add_argument("--contract", type=Path)
    parser.add_argument("--gate-validator", type=Path,
                        help="retained external gates.py copy used only in gate phase")
    parser.add_argument("--execute", action="store_true")
    args = parser.parse_args(argv)
    if not args.execute:
        print("inert: --execute required; no files, commands, builds, or measurements created")
        return 0
    try:
        result = execute_phase(args)
        print(json.dumps(result, sort_keys=True))
        return 0
    except (PhaseError, OSError, subprocess.SubprocessError, KeyError, ValueError) as exc:
        print(f"production phase failed: {exc}", file=sys.stderr)
        return 2

if __name__ == "__main__":
    raise SystemExit(main())
