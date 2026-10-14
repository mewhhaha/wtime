#!/usr/bin/env python3
"""Create or verify an immutable local archive of the original benchmark baseline."""
from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
REPORTS = (
    "docs/benchmarks/wasmtime-comparison.json",
    "docs/benchmarks/wasmtime-comparison-run2.json",
)
CASES = ("call_overhead", "integer_loop", "memory_fill_scan", "branch_calls", "prepare_large")
MODE_FEATURES = {
    "decoded": None,
    "reference": "execution-reference",
    "compact": "compact-executor",
    "register": "register-executor",
    "experimental": "experimental-executors",
    "native": "native-executor",
}


def _mode_config(root: Path, requested: str) -> tuple[str, list[str]]:
    """Resolve and validate a requested policy before invoking Cargo."""
    if requested not in (*MODE_FEATURES, "portable"):
        raise ValueError(f"unsupported execution mode: {requested}")
    resolved = "decoded" if requested == "portable" else requested
    if resolved == "native":
        raise ValueError("native execution mode is unavailable until P095 wires native-executor")
    feature = MODE_FEATURES[resolved]
    if feature is None:
        return resolved, []
    bench_manifest = root / "tools/wtime-bench/Cargo.toml"
    runtime_manifest = root / "crates/wtime-runtime/Cargo.toml"
    try:
        bench = tomllib.loads(bench_manifest.read_text(encoding="utf-8"))
        runtime = tomllib.loads(runtime_manifest.read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError) as exc:
        raise ValueError(f"cannot validate Cargo feature forwarding for {resolved}: {exc}") from exc
    bench_features = bench.get("features", {})
    runtime_features = runtime.get("features", {})
    forwards = _runtime_feature_forwards(bench_features, feature, resolved)
    required = {
        "experimental": {"compact-executor", "register-executor"},
    }.get(resolved, {feature})
    direct_experimental = resolved == "experimental" and "experimental-executors" in forwards
    if not direct_experimental and not required.issubset(forwards):
        missing = sorted(required - forwards)
        raise ValueError(
            f"execution mode {resolved!r} is unavailable: benchmark feature graph "
            f"does not forward required runtime feature(s) {missing!r}"
        )
    for forwarded in forwards:
        if forwarded not in runtime_features:
            raise ValueError(
                f"execution mode {resolved!r} is unavailable: runtime feature {forwarded!r} is not declared"
            )
    if direct_experimental:
        runtime_closure = _local_feature_closure(runtime_features, "experimental-executors", "runtime")
        if not required.issubset(runtime_closure):
            missing = sorted(required - runtime_closure)
            raise ValueError(
                "execution mode 'experimental' is unavailable: runtime feature graph "
                f"experimental-executors does not include {missing!r}"
            )
    return resolved, [feature]


def _feature_values(value: object) -> list[str]:
    """Cargo permits strings and arrays in feature declarations."""
    if isinstance(value, str):
        return [value]
    if isinstance(value, list) and all(isinstance(item, str) for item in value):
        return value
    return []


def _local_feature_closure(features: dict, root: str, owner: str) -> set[str]:
    """Resolve local Cargo feature references, rejecting reachable cycles."""
    if root not in features:
        return set()
    reached: set[str] = set()
    active: list[str] = []

    def visit(name: str) -> None:
        if name in active:
            cycle = " -> ".join((*active[active.index(name):], name))
            raise ValueError(f"cyclic {owner} Cargo feature graph: {cycle}")
        if name in reached:
            return
        active.append(name)
        reached.add(name)
        for item in _feature_values(features.get(name)):
            # `dep:` and dependency-qualified features are not local aliases.
            if item.startswith("dep:"):
                continue
            if "/" in item:
                continue
            if item in features:
                visit(item)
        active.pop()

    visit(root)
    return reached


def _runtime_feature_forwards(features: dict, root: str, mode: str) -> set[str]:
    """Resolve a benchmark feature's local aliases to runtime feature enables."""
    if root not in features:
        raise ValueError(f"execution mode {mode!r} is unavailable: benchmark feature {root!r} is not declared")
    active: list[str] = []
    visited: set[str] = set()
    runtime: set[str] = set()

    def visit(name: str) -> None:
        if name in active:
            cycle = " -> ".join((*active[active.index(name):], name))
            raise ValueError(f"cyclic benchmark Cargo feature graph for mode {mode!r}: {cycle}")
        if name in visited:
            return
        active.append(name)
        visited.add(name)
        for item in _feature_values(features.get(name)):
            if item.startswith("wtime-runtime/"):
                runtime.add(item.split("/", 1)[1].removesuffix("?"))
            elif "/" not in item and item in features:
                visit(item)
        active.pop()

    visit(root)
    return runtime


def sha(path: Path) -> str:
    digest = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def _source_paths(root: Path) -> list[Path]:
    paths = [
        root / "Cargo.toml", root / "Cargo.lock", root / "rust-toolchain.toml",
        root / "LICENSE-APACHE", root / "LICENSE-MIT",
        root / "tools/wtime-bench/Cargo.toml", root / "tools/wtime-bench/Cargo.lock",
        root / "tools/wtime-bench/README.md", root / "tools/wtime-bench/run.py",
        root / "tools/wtime-bench/baseline.py",
        root / "tools/wtime-bench/workloads.json",
        root / ".cargo/config.toml", root / ".cargo/config",
        root / "tools/wtime-bench/.cargo/config.toml",
        root / "tools/wtime-bench/.cargo/config",
    ]
    bench_root = root / "tools/wtime-bench"
    paths.extend(path for path in bench_root.rglob("*.py")
                 if "target" not in path.relative_to(bench_root).parts
                 and "tests" not in path.relative_to(bench_root).parts)
    paths.extend((root / "crates").rglob("Cargo.toml"))
    paths.extend((root / "crates").rglob("*.rs"))
    paths.extend((root / "tools/wtime-conformance").rglob("Cargo.toml"))
    paths.extend((root / "tools/wtime-conformance").rglob("*.rs"))
    paths.extend((root / "tools/wtime-bench/src").rglob("*.rs"))
    paths.extend((bench_root / "fixtures/scalar").rglob("*.wat"))
    paths.extend((bench_root / "fixtures/gc").rglob("*.wat"))
    paths.extend((bench_root / "fixtures/gc").rglob("*.wasm"))
    paths.extend((bench_root / "fixtures/exceptions").rglob("*.wat"))
    paths.extend((bench_root / "fixtures/representative").glob("*.wat"))
    paths.extend((bench_root / "fixtures/representative").glob("*.wasm"))
    paths.extend((bench_root / "fixtures/integration_p113").glob("*.wat"))
    paths.extend((bench_root / "fixtures/integration_p113").glob("*.wasm"))
    paths.extend((bench_root / "fixtures/integration_p113").glob("*.json"))
    paths.extend(root / "docs/benchmarks/performance" / name for name in
                 ("P033-contract.json", "P034-contract.json",
                  "P034-measurement-contract.json", "P034-registry-preservation.json"))
    runtime_gc = root / "crates/wtime-runtime/tests/fixtures/performance/gc"
    paths.extend(runtime_gc.glob("*.wat"))
    paths.extend(runtime_gc.glob("*.wasm"))
    return sorted(set(p for p in paths if p.is_file()))


def _git_commit(root: Path) -> str | None:
    try:
        value = subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=root, text=True, stderr=subprocess.DEVNULL
        ).strip()
        return value or None
    except (OSError, subprocess.CalledProcessError):
        return None


def _record(files: dict[str, dict], archive: Path, rel: str, role: str,
            source_path: str | None = None) -> None:
    path = archive / rel
    files[rel] = {
        "sha256": sha(path), "size_bytes": path.stat().st_size, "role": role,
        **({"source_path": source_path} if source_path else {}),
    }


def verify_archive(path: Path) -> dict:
    """Verify every archived byte against the manifest and return that manifest."""
    path = Path(path)
    manifest_path = path / "manifest.json"
    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise ValueError(f"invalid or missing archive manifest: {manifest_path}") from exc
    if manifest.get("schema") != 1 or not isinstance(manifest.get("files"), dict):
        raise ValueError("unsupported archive manifest schema")
    listed = set(manifest["files"])
    present = {
        item.relative_to(path).as_posix()
        for item in path.rglob("*") if item.is_file() and item != manifest_path
    }
    if present != listed:
        missing = sorted(listed - present)
        extra = sorted(present - listed)
        raise ValueError(f"archive contents differ from manifest (missing={missing}, extra={extra})")
    for rel, record in manifest["files"].items():
        candidate = path / rel
        if not candidate.is_file():
            raise ValueError(f"archive file missing: {rel}")
        if candidate.stat().st_size != record.get("size_bytes") or sha(candidate) != record.get("sha256"):
            raise ValueError(f"archive hash mismatch: {rel}")
        if "mode_bits" in record and stat.S_IMODE(candidate.stat().st_mode) != record["mode_bits"]:
            raise ValueError(f"archive mode mismatch: {rel}")
    return manifest


def _copy_file(source: Path, destination: Path) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, destination)


def _ensure_builds(root: Path, resolved_mode: str = "decoded",
                   cargo_features: list[str] | None = None) -> tuple[Path, Path, str]:
    bench = root / "tools/wtime-bench/target/release/wtime-bench"
    encoder = root / "target/debug/wtime-conformance"
    commands = []
    # Remove a stale or manually replaced executable so a successful no-op
    # Cargo invocation cannot accidentally archive it as the selected build.
    bench.unlink(missing_ok=True)
    command = ["cargo", "build", "--locked", "--release", "--manifest-path",
               "tools/wtime-bench/Cargo.toml", "-j", "4", "--no-default-features"]
    if cargo_features:
        command.extend(["--features", ",".join(cargo_features)])
    subprocess.run(command, cwd=root, check=True)
    commands.append(" ".join(command))
    if not encoder.is_file():
        command = ["cargo", "build", "--locked", "-p", "wtime-conformance"]
        subprocess.run(command, cwd=root, check=True)
        commands.append(" ".join(command))
    return bench, encoder, "; ".join(commands) + ("; pre-existing debug encoder reused" if not commands[1:] else "")


def _protocol_modes(bench: Path) -> tuple[dict, list[str]]:
    result = subprocess.run([str(bench), "--protocol"], text=True, capture_output=True, check=True, timeout=30)
    try:
        protocol = json.loads(result.stdout)
        modes = protocol["modes"]
    except (json.JSONDecodeError, KeyError, TypeError) as exc:
        raise RuntimeError(f"benchmark --protocol returned invalid mode metadata: {result.stdout!r}") from exc
    if not isinstance(modes, list) or any(not isinstance(mode, str) for mode in modes):
        raise RuntimeError("benchmark --protocol modes must be a list of strings")
    return protocol, modes


def _require_protocol_mode(protocol: dict, resolved_mode: str) -> list[str]:
    modes = protocol.get("modes")
    if not isinstance(modes, list) or any(not isinstance(mode, str) for mode in modes):
        raise RuntimeError("benchmark --protocol modes must be a list of strings")
    if resolved_mode not in modes:
        raise RuntimeError(
            f"requested execution mode {resolved_mode!r} is absent from benchmark --protocol modes {modes!r}"
        )
    return modes


def _mode_smoke(bench: Path, fixture: Path, mode: str, output: Path) -> dict:
    command = [str(bench), "--single-case", str(fixture), "call_overhead", "7", "8",
               "warm_call", "1", mode]
    result = subprocess.run(command, text=True, capture_output=True, check=True, timeout=180)
    try:
        report = json.loads(result.stdout)
    except json.JSONDecodeError as exc:
        raise RuntimeError(f"mode smoke returned invalid JSON: {result.stdout!r}") from exc
    if (report.get("case") != "call_overhead" or report.get("input") != 7
            or report.get("expected") != 8 or report.get("result") != 8
            or report.get("mode") != mode or report.get("stage") != "warm_call"):
        raise RuntimeError(f"mode smoke did not verify requested policy and oracle: {report!r}")
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(report, sort_keys=True, indent=2) + "\n", encoding="utf-8")
    return {"command": command, "returncode": result.returncode, "correctness_checked": True,
            "case": report["case"], "stage": report["stage"], "input": 7,
            "expected": 8, "result": 8, "mode": mode,
            "fixture_sha256": sha(fixture), "binary_sha256": sha(bench),
            "report_sha256": sha(output)}


def create_archive(output: Path, root: Path = ROOT, mode: str | None = None) -> dict:
    output = Path(output).resolve()
    root = Path(root).resolve()
    requested_mode = mode or "decoded"
    resolved_mode, cargo_features = _mode_config(root, requested_mode)
    if output.exists() and any(output.iterdir()):
        manifest = verify_archive(output)
        recorded = manifest.get("execution_policy", {})
        if mode is not None and (recorded.get("requested_mode") != requested_mode
                                 or recorded.get("resolved_mode") != resolved_mode):
            raise ValueError(f"existing archive is bound to mode {recorded.get('requested_mode')!r}; refusing to reuse for {requested_mode!r}")
        return manifest
    output.parent.mkdir(parents=True, exist_ok=True)
    bench, encoder, build_note = _ensure_builds(root, resolved_mode, cargo_features)
    staging = Path(tempfile.mkdtemp(prefix=f".{output.name}.staging-", dir=output.parent))
    try:
        files: dict[str, dict] = {}
        for source in _source_paths(root):
            relative = source.relative_to(root).as_posix()
            rel = f"source/{relative}"
            _copy_file(source, staging / rel)
            _record(files, staging, rel, "source", relative)
        for source, rel, role in (
            (bench, "binary/wtime-bench", "release_benchmark_executable"),
            (encoder, "binary/wtime-conformance", "fixture_encoder_executable"),
        ):
            _copy_file(source, staging / rel)
            _record(files, staging, rel, role, source.relative_to(root).as_posix())
        protocol, available_modes = _protocol_modes(bench)
        _require_protocol_mode(protocol, resolved_mode)
        for report in REPORTS:
            source = root / report
            if not source.is_file():
                raise FileNotFoundError(f"historical report missing: {source}")
            rel = f"historical_reports/{Path(report).name}"
            _copy_file(source, staging / rel)
            _record(files, staging, rel, "historical_report", report)

        # The original runner is the oracle-backed smoke test and preserves its historical report format.
        report_path = staging / "runs/historical-format.json"
        report_path.parent.mkdir(parents=True)
        command = [sys.executable, str(root / "tools/wtime-bench/run.py"), "--samples", "5",
                   "--output", str(report_path), "--encoder", str(encoder)]
        result = subprocess.run(command, cwd=root, text=True, capture_output=True, timeout=1800)
        if result.returncode:
            raise RuntimeError(f"historical-format benchmark failed ({result.returncode}):\n{result.stdout}\n{result.stderr}")
        report = json.loads(report_path.read_text(encoding="utf-8"))
        if not report.get("passed") or not report.get("wasm_correctness_checked"):
            raise RuntimeError("historical-format report did not pass correctness checks")
        expected_cases = set(CASES)
        if set(report.get("workloads", {})) != expected_cases:
            raise RuntimeError("historical-format report workload set is incomplete")
        for case in CASES[:-1]:
            if report["workloads"][case]["expected_result"] is None:
                raise RuntimeError(f"missing oracle value for {case}")
        raw = report.get("raw_samples", [])
        counts = {case: sum(row.get("workload") == case for row in raw) for case in CASES}
        if any(count <= 0 for count in counts.values()):
            raise RuntimeError(f"smoke run produced no samples: {counts}")
        _record(files, staging, "runs/historical-format.json", "fresh_historical_format_run")
        # Archive deterministic fixture bytes written by the runner.
        for case in CASES:
            for extension in ("wat", "wasm"):
                source = root / f"tools/wtime-bench/fixtures/{case}.{extension}"
                rel = f"fixtures/{case}.{extension}"
                _copy_file(source, staging / rel)
                _record(files, staging, rel, "deterministic_fixture")
        mode_smoke = _mode_smoke(bench, staging / "fixtures/call_overhead.wasm", resolved_mode,
                                 staging / "runs/mode-smoke.json")
        _record(files, staging, "runs/mode-smoke.json", "mode_aware_single_case_run")
        manifest = {
            "schema": 1,
            "created_at_utc": dt.datetime.now(dt.timezone.utc).isoformat(),
            "repository_commit": _git_commit(root),
            "repository_commit_note": "checkout has no commit; frozen from the supplied source tree" if _git_commit(root) is None else None,
            "mode": resolved_mode,
            "execution_policy": {"requested_mode": requested_mode, "resolved_mode": resolved_mode,
                                 "cargo_features": cargo_features, "protocol_modes": available_modes,
                                 "protocol_sha256": sha(root / "tools/wtime-bench/src/protocol.rs")},
            "build": {"benchmark_command": "cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml -j 4 --no-default-features" +
                      ((" --features " + ",".join(cargo_features)) if cargo_features else ""),
                      "build_observation": build_note, "rustc": subprocess.check_output(["rustc", "-Vv"], cwd=root, text=True),
                      "release_profile": True, "features": cargo_features},
            "smoke": {"command": command, "returncode": result.returncode, "correctness_checked": True,
                      "sample_count": 5, "workloads": counts, "prepare_large": "prepare only",
                      "historical_policy": "decoded (historical-format runner uses Engine::new)"},
            "mode_smoke": mode_smoke,
            "files": dict(sorted(files.items())),
        }
        (staging / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
        if output.exists():
            if any(output.iterdir()):
                raise FileExistsError(f"refusing to replace nonempty archive: {output}")
            output.rmdir()
        staging.rename(output)
        return verify_archive(output)
    except BaseException:
        shutil.rmtree(staging, ignore_errors=True)
        raise


def _adapter_manifest(path: Path) -> dict:
    manifest_path = path / "manifest.json"
    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise ValueError(f"invalid or missing adapter manifest: {manifest_path}") from exc
    if manifest.get("schema") != 1 or manifest.get("kind") != "legacy-api-adapter":
        raise ValueError("unsupported adapter archive manifest")
    verify_archive(path)
    if manifest.get("binary") != "binary/wtime-bench":
        raise ValueError("adapter archive does not identify its executable path")
    executable = path / "binary/wtime-bench"
    if not executable.is_file() or not os.access(executable, os.X_OK):
        raise ValueError("adapter benchmark binary is not executable")
    if manifest.get("binary_sha256") != sha(executable):
        raise ValueError("adapter benchmark binary identity mismatch")
    return manifest


def verify_adapter(path: Path, baseline: Path, protocol: Path) -> dict:
    """Verify an adapter archive and that it is bound to the requested inputs."""
    path = Path(path).resolve()
    baseline = Path(baseline).resolve()
    protocol = Path(protocol).resolve()
    frozen = verify_archive(baseline)
    manifest = _adapter_manifest(path)
    baseline_manifest_sha = sha(baseline / "manifest.json")
    protocol_sha = sha(protocol)
    if Path(manifest.get("baseline", "")).resolve() != baseline:
        raise ValueError("adapter archive is bound to a different baseline path")
    if manifest.get("baseline_manifest_sha256") != baseline_manifest_sha:
        raise ValueError("adapter archive baseline manifest identity mismatch")
    if manifest.get("protocol_sha256") != protocol_sha:
        raise ValueError("adapter archive protocol identity mismatch")
    harness = protocol.parent.parent
    recorded_harness = manifest.get("shared_harness_identities", {})
    current_harness = {}
    for rel in recorded_harness:
        source = harness / rel
        if not source.is_file():
            raise ValueError(f"shared harness input missing: {rel}")
        current_harness[rel] = sha(source)
    if current_harness != recorded_harness:
        raise ValueError("adapter archive shared harness identity mismatch")
    if manifest.get("frozen_source_identities") != {
        name: record["sha256"] for name, record in frozen["files"].items()
        if record.get("role") == "source"
    }:
        raise ValueError("adapter archive frozen source identity mismatch")
    return manifest


def _adapter_files(archive: Path) -> dict[str, dict]:
    files = {}
    for item in sorted(archive.rglob("*")):
        if not item.is_file() or item == archive / "manifest.json":
            continue
        rel = item.relative_to(archive).as_posix()
        if rel == "binary/wtime-bench":
            role = "legacy_adapter_executable"
        elif rel == "src/protocol.rs":
            role = "shared_measurement_protocol"
        elif rel == "src/main.rs":
            role = "adapter_entrypoint"
        elif rel == "Cargo.lock":
            role = "isolated_lockfile"
        else:
            role = "adapter_source_or_manifest"
        files[rel] = {"sha256": sha(item), "size_bytes": item.stat().st_size,
                      "mode_bits": stat.S_IMODE(item.stat().st_mode), "role": role}
    return files


def create_adapter(harness: Path, baseline: Path, output: Path) -> dict:
    """Build a separately identified protocol adapter against the frozen core."""
    harness = Path(harness).resolve()
    baseline = Path(baseline).resolve()
    output = Path(output).resolve()
    protocol = harness / "src/protocol.rs"
    if not protocol.is_file():
        raise FileNotFoundError(f"shared protocol module missing: {protocol}")
    frozen = verify_archive(baseline)
    baseline_manifest_sha = sha(baseline / "manifest.json")
    protocol_sha = sha(protocol)
    if output.exists() and any(output.iterdir()):
        return verify_adapter(output, baseline, protocol)
    output.parent.mkdir(parents=True, exist_ok=True)
    staging = Path(tempfile.mkdtemp(prefix=f".{output.name}.staging-", dir=output.parent))
    build_area = Path(tempfile.mkdtemp(prefix=f".{output.name}.build-", dir=output.parent))
    try:
        runtime = (baseline / "source/crates/wtime-runtime").resolve()
        types = (baseline / "source/crates/wtime-types").resolve()
        cargo_toml = f'''[package]
name = "wtime-bench-legacy"
version = "0.1.0"
edition = "2024"
license = "MIT OR Apache-2.0"
publish = false

[workspace]

[features]
default = ["legacy-api", "runtime-only"]
legacy-api = []
runtime-only = []

[dependencies]
wtime-runtime = {{ path = {json.dumps(str(runtime))} }}
wtime-types = {{ path = {json.dumps(str(types))} }}
'''
        (staging / "Cargo.toml").write_text(cargo_toml, encoding="utf-8")
        (staging / "src").mkdir()
        (staging / "src/protocol.rs").write_bytes(protocol.read_bytes())
        for leaf in ("scalar_driver.rs", "memory_driver.rs", "representative_driver.rs"):
            source = harness / "src" / leaf
            if source.is_file():
                (staging / "src" / leaf).write_bytes(source.read_bytes())
        (staging / "src/main.rs").write_text('''#![forbid(unsafe_code)]
mod protocol;

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if !protocol::try_main(&args) {
        eprintln!("unsupported legacy benchmark command");
        std::process::exit(2);
    }
}
''', encoding="utf-8")
        target = build_area / "target"
        env = dict(os.environ, CARGO_TARGET_DIR=str(target))
        manifest_path = staging / "Cargo.toml"
        subprocess.run(["cargo", "generate-lockfile", "--manifest-path", str(manifest_path)],
                       check=True, cwd=harness, env=env)
        subprocess.run(["cargo", "build", "--locked", "--release", "--manifest-path",
                        str(manifest_path), "-j", "4"], check=True, cwd=harness, env=env)
        executable = target / "release/wtime-bench-legacy"
        if not executable.is_file():
            raise RuntimeError("adapter release executable was not produced")
        _copy_file(executable, staging / "binary/wtime-bench")
        source_identities = {
            name: record["sha256"] for name, record in frozen["files"].items()
            if record.get("role") == "source"
        }
        shared_identities = {}
        for rel in ("Cargo.toml", "Cargo.lock", "src/main.rs", "src/protocol.rs", "run.py",
                    "src/scalar_driver.rs", "src/memory_driver.rs", "src/representative_driver.rs"):
            source = harness / rel
            if source.is_file():
                shared_identities[rel] = sha(source)
        files = _adapter_files(staging)
        manifest = {
            "schema": 1,
            "kind": "legacy-api-adapter",
            "created_at_utc": dt.datetime.now(dt.timezone.utc).isoformat(),
            "baseline": str(baseline),
            "binary": "binary/wtime-bench",
            "baseline_manifest_sha256": baseline_manifest_sha,
            "protocol_sha256": protocol_sha,
            "frozen_source_identities": dict(sorted(source_identities.items())),
            "shared_harness_identities": dict(sorted(shared_identities.items())),
            "binary_sha256": files["binary/wtime-bench"]["sha256"],
            "build": {"command": "cargo build --locked --release --manifest-path <adapter>/Cargo.toml -j 4",
                      "rustc": subprocess.check_output(["rustc", "-Vv"], cwd=harness, text=True),
                      "features": ["legacy-api", "runtime-only"], "wasmtime_oracle": False,
                      "dependencies": {"wtime-runtime": str(runtime), "wtime-types": str(types)}},
            "files": dict(sorted(files.items())),
        }
        (staging / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
        if output.exists():
            if any(output.iterdir()):
                raise FileExistsError(f"refusing to replace nonempty adapter archive: {output}")
            output.rmdir()
        staging.rename(output)
        return verify_adapter(output, baseline, protocol)
    except BaseException:
        shutil.rmtree(staging, ignore_errors=True)
        raise
    finally:
        shutil.rmtree(build_area, ignore_errors=True)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--adapt-harness", type=Path, metavar="HARNESS")
    parser.add_argument("--baseline", type=Path)
    parser.add_argument("--mode", choices=(*MODE_FEATURES, "portable"))
    args = parser.parse_args()
    try:
        if args.adapt_harness:
            if args.mode not in (None, "decoded"):
                parser.error("legacy API adapters support only --mode decoded")
            if not args.baseline:
                parser.error("--adapt-harness requires --baseline ARCHIVE")
            manifest = create_adapter(args.adapt_harness, args.baseline, args.output)
            print(f"verified adapter: {Path(args.output).resolve()}")
            print(f"binary sha256: {manifest['binary_sha256']}")
            return 0
        if args.baseline:
            parser.error("--baseline requires --adapt-harness HARNESS")
        manifest = create_archive(args.output, mode=args.mode)
    except (OSError, ValueError, RuntimeError, subprocess.CalledProcessError) as exc:
        print(f"baseline archive failed: {exc}", file=sys.stderr)
        return 1
    print(f"verified archive: {Path(args.output).resolve()}")
    print(f"files: {len(manifest['files'])}; workloads: {len(manifest['smoke']['workloads'])}; correctness: passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
