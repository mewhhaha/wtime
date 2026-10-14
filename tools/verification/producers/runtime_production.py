#!/usr/bin/env python3
"""Build the current runtime through the small, source-bound protocol harness.

This is an opt-in candidate build lane. It deliberately omits the full
Wasmtime-backed benchmark main and does not replace candidate_production.py.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
from typing import Callable

ROOT = Path(__file__).resolve().parents[3]
BENCH = ROOT / "tools/wtime-bench"
if str(BENCH) not in sys.path:
    sys.path.insert(0, str(BENCH))

ENTRYPOINT = (
    "#![forbid(unsafe_code)]\n"
    "mod protocol;\n"
    "fn main() {\n"
    " let args = std::env::args().skip(1).collect::<Vec<_>>();\n"
    " if !protocol::try_main(&args) { std::process::exit(2); }\n"
    "}\n"
)
CORE_CRATES = ("wtime-runtime", "wtime-types", "wtime-decode", "wtime-validate")
MODES = {
    "decoded": {"harness_features": ["runtime-only"], "runtime_features": [],
                "cargo_features": ["runtime-only"]},
    "compact": {"harness_features": ["runtime-only"],
                "runtime_features": ["compact-executor"],
                "cargo_features": ["runtime-only", "compact-executor"]},
}
SCHEMA = "wtime-runtime-only-production-v1"


def sha_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sha(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def canonical_hash(value: object) -> str:
    return sha_bytes(json.dumps(value, sort_keys=True, separators=(",", ":"),
                                allow_nan=False).encode())


def write_new(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("x", encoding="utf-8") as stream:
        json.dump(value, stream, indent=2, sort_keys=True, allow_nan=False)
        stream.write("\n")


def _source_identity(root: Path) -> dict[str, str]:
    """Return the same projected identity that focused candidate reports use."""
    bench = root / "tools/wtime-bench"
    if str(bench) not in sys.path:
        sys.path.insert(0, str(bench))
    import focused_compare
    return focused_compare.sources()


def _core_source_map(root: Path) -> dict[str, str]:
    result: dict[str, str] = {}
    for crate in CORE_CRATES:
        source_dir = root / "crates" / crate / "src"
        if not source_dir.is_dir():
            raise FileNotFoundError(f"missing current core source directory: {source_dir}")
        for path in source_dir.rglob("*.rs"):
            if path.is_file():
                result[path.relative_to(root).as_posix()] = sha(path)
    if not all(any(name.startswith(f"crates/{crate}/src/") for name in result)
               for crate in CORE_CRATES):
        raise ValueError("runtime source identity omits one or more core crates")
    return dict(sorted(result.items()))


def _core_compilation_files(root: Path) -> dict[str, str]:
    result: dict[str, str] = {}
    for crate in CORE_CRATES:
        manifest = root / "crates" / crate / "Cargo.toml"
        if not manifest.is_file():
            raise FileNotFoundError(f"missing current core Cargo manifest: {manifest}")
        result[manifest.relative_to(root).as_posix()] = sha(manifest)
        source_dir = manifest.parent / "src"
        for path in source_dir.rglob("*.rs"):
            if path.is_file():
                result[path.relative_to(root).as_posix()] = sha(path)
    return dict(sorted(result.items()))


def _inside(path: Path, root: Path, label: str) -> Path:
    resolved = path.resolve()
    if not resolved.is_relative_to(root.resolve(strict=True)):
        raise ValueError(f"{label} escapes its authorized root: {resolved}")
    return resolved


def _write_if_changed(path: Path, data: bytes) -> bool:
    """Keep stable inputs' mtimes; atomically refresh changed wrapper inputs."""
    path.parent.mkdir(parents=True, exist_ok=True)
    if path.is_file() and path.read_bytes() == data:
        return False
    with tempfile.NamedTemporaryFile(prefix="." + path.name + ".", dir=path.parent,
                                     delete=False) as stream:
        temporary = Path(stream.name)
        stream.write(data)
        stream.flush()
        os.fsync(stream.fileno())
    temporary.replace(path)
    return True


def _copy_identity_sources(root: Path, output: Path,
                           sources: dict[str, str]) -> dict[str, dict[str, object]]:
    archived: dict[str, dict[str, object]] = {}
    for relative, expected in sorted(sources.items()):
        source = _inside(root / relative, root, "source input")
        if not source.is_file() or sha(source) != expected:
            raise ValueError(f"focused source identity is stale or missing: {relative}")
        destination = _inside(output / "source" / relative, output, "source archive output")
        destination.parent.mkdir(parents=True, exist_ok=True)
        with destination.open("xb") as stream:
            stream.write(source.read_bytes())
        if sha(destination) != expected:
            raise ValueError(f"source changed while archiving: {relative}")
        archived[relative] = {"source_path": str(source), "path": str(destination),
                              "sha256": expected, "bytes": destination.stat().st_size}
    return archived


def _sync_harness(root: Path, workspace: Path, output: Path,
                  source_identity: dict[str, str]) -> dict[str, dict[str, object]]:
    """Copy the protocol and every sibling driver except the full benchmark main."""
    source_dir = root / "tools/wtime-bench/src"
    files = sorted(source_dir.rglob("*.rs"))
    if not files or not any(path.name == "protocol.rs" for path in files):
        raise ValueError("benchmark Rust protocol sources are missing")
    compiled: dict[str, dict[str, object]] = {}
    copied_names: set[str] = set()
    for source in files:
        relative = source.relative_to(root).as_posix()
        if source.name == "main.rs":
            continue
        expected = source_identity.get(relative)
        if not expected or sha(source) != expected:
            raise ValueError(f"benchmark source is not bound by the focused identity: {relative}")
        destination = workspace / "src" / source.relative_to(source_dir)
        destination = _inside(destination, workspace, "compiled harness output")
        _write_if_changed(destination, source.read_bytes())
        copied_names.add(destination.relative_to(workspace / "src").as_posix())
        archive = output / "source" / relative
        compiled[relative] = {"source_path": str(source), "workspace_path": str(destination),
                              "archive_path": str(archive), "sha256": expected}
    destination_main = workspace / "src/main.rs"
    _write_if_changed(destination_main, ENTRYPOINT.encode())
    compiled["generated:src/main.rs"] = {
        "source_path": None, "workspace_path": str(destination_main),
        "archive_path": str(output / "compiled/src/main.rs"),
        "sha256": sha(destination_main),
    }
    archive_main = Path(compiled["generated:src/main.rs"]["archive_path"])
    archive_main.parent.mkdir(parents=True, exist_ok=True)
    archive_main.write_bytes(destination_main.read_bytes())
    # Remove stale Rust modules so membership cannot silently differ from this attempt.
    src_root = workspace / "src"
    for stale in sorted(src_root.rglob("*.rs")):
        if stale.relative_to(src_root).as_posix() not in copied_names | {"main.rs"}:
            stale.unlink()
    return compiled


def _package_manifest(root: Path, mode: str) -> bytes:
    features = MODES[mode]["cargo_features"]
    feature_lines = ["default = []", "runtime-only = []"]
    if "compact-executor" in features:
        feature_lines.append('compact-executor = ["wtime-runtime/compact-executor"]')
    return ("[package]\nname = \"wtime-production-protocol\"\nversion = \"0.1.0\"\n"
            "edition = \"2024\"\npublish = false\n\n[workspace]\n\n[features]\n"
            + "\n".join(feature_lines) + "\n\n[dependencies]\n"
            + "wtime-runtime = { path = " + json.dumps(str(root / "crates/wtime-runtime"))
            + ", default-features = false }\n"
            + "wtime-types = { path = " + json.dumps(str(root / "crates/wtime-types"))
            + ", default-features = false }\n").encode()


def _metadata_summary(metadata: dict, root: Path, workspace: Path,
                      mode: str) -> dict:
    packages = metadata.get("packages")
    if not isinstance(packages, list):
        raise ValueError("Cargo metadata has no packages array")
    by_name = {package.get("name"): package for package in packages}
    expected_names = set(CORE_CRATES) | {"wtime-production-protocol"}
    if len(packages) != len(expected_names) or set(by_name) != expected_names:
        raise ValueError("runtime-only wrapper must resolve exactly five local packages")
    if any(package.get("source") is not None for package in packages):
        raise ValueError("runtime-only wrapper resolved an external dependency")
    resolution = {}
    for crate in CORE_CRATES:
        expected = (root / "crates" / crate / "Cargo.toml").resolve(strict=True)
        actual = Path(by_name[crate].get("manifest_path", "")).resolve(strict=True)
        if actual != expected:
            raise ValueError(f"Cargo resolved {crate} outside the current core: {actual}")
        resolution[crate] = str(actual)
    wrapper_manifest = (workspace / "Cargo.toml").resolve(strict=True)
    actual_wrapper = Path(by_name["wtime-production-protocol"].get("manifest_path", "")).resolve(strict=True)
    if actual_wrapper != wrapper_manifest:
        raise ValueError("Cargo metadata package path differs from stable wrapper manifest")
    nodes = (metadata.get("resolve") or {}).get("nodes")
    if not isinstance(nodes, list) or len(nodes) != len(expected_names):
        raise ValueError("Cargo resolution graph must contain exactly five local package nodes")
    node_ids = {node.get("id") for node in nodes}
    if node_ids != {package.get("id") for package in packages}:
        raise ValueError("Cargo resolution graph contains an unreported package")
    node_by_name = {package["name"]: next(node for node in nodes if node.get("id") == package["id"])
                    for package in packages}
    expected_runtime = MODES[mode]["runtime_features"]
    expected_wrapper = MODES[mode]["cargo_features"]
    if sorted(node_by_name["wtime-runtime"].get("features", [])) != expected_runtime:
        raise ValueError("resolved runtime features differ from the requested production mode")
    if sorted(node_by_name["wtime-production-protocol"].get("features", [])) != sorted(expected_wrapper):
        raise ValueError("resolved wrapper features differ from the requested production mode")
    return {"packages": sorted(expected_names), "external_dependencies": 0,
            "resolution": resolution, "wrapper_manifest": str(wrapper_manifest),
            "resolve_sha256": canonical_hash(nodes),
            "runtime_features": expected_runtime, "wrapper_features": expected_wrapper}


def _verify_compiled_inputs(compiled: dict[str, dict[str, object]]) -> None:
    for name, item in compiled.items():
        expected = item["sha256"]
        for key in ("archive_path", "workspace_path"):
            path = Path(str(item[key]))
            if not path.is_file() or sha(path) != expected:
                raise ValueError(f"compiled input {name} changed at {key}")


def _cache_compiled_inputs(compiled: dict[str, dict[str, object]]) -> dict:
    return {name: {key: value for key, value in item.items() if key != "archive_path"}
            for name, item in compiled.items()}


def _root_metadata_summary(metadata: dict, root: Path) -> dict:
    packages = metadata.get("packages")
    if not isinstance(packages, list):
        raise ValueError("root Cargo metadata has no packages array")
    by_name = {package.get("name"): package for package in packages}
    resolution = {}
    for crate in CORE_CRATES:
        package = by_name.get(crate)
        if not isinstance(package, dict) or package.get("source") is not None:
            raise ValueError(f"root Cargo metadata does not resolve local core crate {crate}")
        expected = (root / "crates" / crate / "Cargo.toml").resolve(strict=True)
        actual = Path(package.get("manifest_path", "")).resolve(strict=True)
        if actual != expected:
            raise ValueError(f"root Cargo metadata resolved {crate} to the wrong path: {actual}")
        resolution[crate] = str(actual)
    return {"workspace_root": str(Path(metadata.get("workspace_root", "")).resolve(strict=True)),
            "core_resolution": resolution,
            "metadata_sha256": canonical_hash(metadata)}


def _run_logged(argv: list[str], cwd: Path, output: Path, label: str,
                runner: Callable, env: dict[str, str] | None = None) -> tuple[int, Path, Path]:
    command_path = output / f"{label}.command.json"
    stdout_path, stderr_path = output / f"{label}.stdout", output / f"{label}.stderr"
    write_new(command_path, {"argv": argv, "cwd": str(cwd),
                             "environment_sha256": _environment_hash(env or {})})
    # Reserve streams before launch so every attempted command has its artifacts.
    with stdout_path.open("xb"):
        pass
    with stderr_path.open("xb"):
        pass
    completed = runner(argv, cwd=cwd, env=env, text=True, capture_output=True, check=False)
    stdout_path.write_text(completed.stdout or "", encoding="utf-8")
    stderr_path.write_text(completed.stderr or "", encoding="utf-8")
    write_new(output / f"{label}.status.json", {"exit_code": completed.returncode})
    return completed.returncode, stdout_path, stderr_path


def _environment_hash(env: dict[str, str]) -> str:
    selected = {name: value for name, value in env.items()
                if name.startswith("CARGO_") or name in {"RUSTFLAGS", "RUSTDOCFLAGS"}}
    return canonical_hash(selected)


def _tool_identity(root: Path, output: Path, runner: Callable,
                   env: dict[str, str], suffix: str) -> dict:
    result = {}
    for name in ("rustc", "cargo"):
        executable = shutil.which(name, path=env.get("PATH"))
        if not executable:
            raise FileNotFoundError(f"{name} is not available in PATH")
        exit_code, stdout, stderr = _run_logged([name, "-Vv"], root, output,
                                                 f"{name}-version-{suffix}", runner, env)
        if exit_code:
            raise RuntimeError(f"{name} -Vv failed with {exit_code}")
        result[name] = {"path": str(Path(executable).resolve()),
                        "sha256": sha(Path(executable).resolve()),
                        "version": stdout.read_text(encoding="utf-8"),
                        "version_sha256": sha(stdout), "stderr_sha256": sha(stderr)}
    return result


def _json_command(argv: list[str], root: Path, output: Path, label: str,
                  runner: Callable, env: dict[str, str]) -> tuple[dict, Path]:
    exit_code, stdout, _stderr = _run_logged(argv, root, output, label, runner, env)
    if exit_code:
        raise RuntimeError(f"{label} failed with {exit_code}; see {output}")
    value = json.loads(stdout.read_text(encoding="utf-8"))
    return value, stdout


def build(output: Path, mode: str, *, root: Path = ROOT,
          runner: Callable = subprocess.run) -> dict:
    if mode not in MODES:
        raise ValueError(f"unsupported runtime-only production mode: {mode}")
    root = root.resolve(strict=True)
    output = _inside(output, root, "production output")
    if output.exists():
        raise FileExistsError(f"production output must be exclusive: {output}")
    output.mkdir(parents=True, exist_ok=False)
    sources_before = _source_identity(root)
    core_before = _core_source_map(root)
    core_compile_before = _core_compilation_files(root)
    archived_sources = _copy_identity_sources(root, output, sources_before)
    source_after_archive = _source_identity(root)
    if source_after_archive != sources_before:
        raise ValueError("candidate sources changed while archiving focused inputs")

    # Stable per-mode workspace/target: Cargo sees the same paths across edits,
    # while _write_if_changed refreshes only changed wrapper inputs.
    cache_root = _inside(root / "target/verification/build-caches/runtime-protocol" / mode,
                         root, "build cache")
    workspace = cache_root / "workspace"
    workspace.mkdir(parents=True, exist_ok=True)
    (workspace / "src").mkdir(exist_ok=True)
    compiled_harness = _sync_harness(root, workspace, output, sources_before)
    package_manifest = _package_manifest(root, mode)
    _write_if_changed(workspace / "Cargo.toml", package_manifest)
    generated_manifest = output / "compiled/Cargo.toml"
    generated_manifest.parent.mkdir(parents=True, exist_ok=True)
    generated_manifest.write_bytes((workspace / "Cargo.toml").read_bytes())
    generated_compiled = {"generated:Cargo.toml": {
        "source_path": None, "workspace_path": str(workspace / "Cargo.toml"),
        "archive_path": str(generated_manifest),
        "sha256": sha(workspace / "Cargo.toml")}}
    generated_compiled.update(compiled_harness)

    helper_path = output / "helper.executed.py"
    helper_bytes = Path(__file__).resolve().read_bytes()
    with helper_path.open("xb") as stream:
        stream.write(helper_bytes)

    env = dict(os.environ)
    target_dir = workspace / "target"
    target_dir.mkdir(exist_ok=True)
    env["CARGO_TARGET_DIR"] = str(target_dir)
    tools_before = _tool_identity(root, output, runner, env, "before")
    toolchain_identity = {name: {"path": row["path"], "sha256": row["sha256"],
                                 "version_sha256": row["version_sha256"]}
                          for name, row in tools_before.items()}
    root_metadata_command = ["cargo", "metadata", "--locked", "--offline",
                             "--format-version", "1", "--manifest-path", str(root / "Cargo.toml")]
    root_metadata_before, root_metadata_stdout = _json_command(
        root_metadata_command, root, output, "root-metadata-before", runner, env)
    root_metadata_summary = _root_metadata_summary(root_metadata_before, root)
    manifest_path = workspace / "Cargo.toml"
    lock_path = workspace / "Cargo.lock"
    if not lock_path.is_file():
        lock_command = ["cargo", "generate-lockfile", "--offline", "--manifest-path", str(manifest_path)]
        lock_exit, _out, _err = _run_logged(lock_command, root, output, "lock", runner, env)
        if lock_exit:
            raise RuntimeError(f"offline lock generation failed with {lock_exit}")
        if not lock_path.is_file():
            raise FileNotFoundError("Cargo did not write the runtime-only lockfile")
    output_lock = output / "compiled/Cargo.lock"
    output_lock.write_bytes(lock_path.read_bytes())
    generated_compiled["generated:Cargo.lock"] = {
        "source_path": None, "workspace_path": str(lock_path),
        "archive_path": str(output_lock), "sha256": sha(lock_path)}
    output_manifest = output / "source-manifest.json"
    write_new(output_manifest, {"source_identities": sources_before,
                                "archived_sources": archived_sources,
                                "core_source_map": core_before,
                                "core_compilation_map": core_compile_before,
                                "compiled_inputs": generated_compiled})
    key_inputs = {"mode": mode, "sources": sources_before, "core": core_compile_before,
                  "compiled": _cache_compiled_inputs(generated_compiled), "features": MODES[mode],
                  "tools": toolchain_identity, "environment_sha256": _environment_hash(env),
                  "target": str(target_dir), "profile": "release", "jobs": 4}
    cache_key = canonical_hash(key_inputs)
    build_features = ",".join(MODES[mode]["cargo_features"])
    metadata_command = ["cargo", "metadata", "--locked", "--offline", "--no-default-features",
                       "--features", build_features, "--format-version", "1",
                       "--manifest-path", str(manifest_path)]
    metadata_before, metadata_stdout = _json_command(metadata_command, root, output,
                                                      "metadata-before", runner, env)
    summary = _metadata_summary(metadata_before, root, workspace, mode)
    build_command = ["cargo", "build", "--locked", "--offline", "--release",
                     "--no-default-features", "-j4", "--features", build_features,
                     "--manifest-path", str(manifest_path), "--bin", "wtime-production-protocol"]
    _verify_compiled_inputs(generated_compiled)
    build_exit, build_stdout, build_stderr = _run_logged(build_command, root, output,
                                                         "production-build", runner, env)
    _verify_compiled_inputs(generated_compiled)
    if build_exit:
        raise RuntimeError(f"runtime-only production build failed with {build_exit}")
    built_binary = target_dir / "release/wtime-production-protocol"
    if not built_binary.is_file():
        raise FileNotFoundError(f"Cargo succeeded without producing {built_binary}")
    binary = output / "bin/wtime-bench"
    binary.parent.mkdir()
    shutil.copy2(built_binary, binary)
    protocol_command = [str(binary), "--protocol"]
    protocol_exit, protocol_stdout, protocol_stderr = _run_logged(
        protocol_command, root, output, "protocol", runner, env)
    if protocol_exit:
        raise RuntimeError(f"runtime-only protocol probe failed with {protocol_exit}")
    protocol_bytes = protocol_stdout.read_bytes()
    protocol_path = output / "protocol.stdout.json"
    protocol_path.write_bytes(protocol_bytes)
    protocol = json.loads(protocol_bytes)
    if (protocol.get("schema_version") != 2 or protocol.get("engine") != "wtime"
            or protocol.get("instrumented") is not False or mode not in protocol.get("modes", [])):
        raise ValueError(f"runtime-only protocol does not prove uninstrumented {mode} support")

    metadata_after, metadata_after_stdout = _json_command(metadata_command, root, output,
                                                           "metadata-after", runner, env)
    root_metadata_after, root_metadata_after_stdout = _json_command(
        root_metadata_command, root, output, "root-metadata-after", runner, env)
    tools_after = _tool_identity(root, output, runner, env, "after")
    if (canonical_hash(metadata_before) != canonical_hash(metadata_after)
            or canonical_hash(root_metadata_before) != canonical_hash(root_metadata_after)
            or tools_before != tools_after):
        raise ValueError("Cargo metadata resolution changed during production build")
    sources_after = _source_identity(root)
    core_after = _core_source_map(root)
    core_compile_after = _core_compilation_files(root)
    if (sources_before != sources_after or core_before != core_after
            or core_compile_before != core_compile_after):
        raise ValueError("current candidate sources changed during runtime-only build/probe")
    stdout_log = output / "stdout.log"
    stderr_log = output / "stderr.log"
    stdout_log.write_bytes(build_stdout.read_bytes())
    stderr_log.write_bytes(build_stderr.read_bytes())

    cache_manifest = {
        "schema": "wtime-runtime-protocol-cache-v1", "input_key": cache_key,
        "mode": mode, "workspace": str(workspace), "target": str(target_dir),
        "source_identities": sources_before, "core_source_map": core_before,
        "core_compilation_map": core_compile_before,
        "compiled_inputs": _cache_compiled_inputs(generated_compiled), "features": MODES[mode],
        "cargo_metadata_sha256": canonical_hash(metadata_before),
        "cargo_lock_sha256": sha(lock_path), "tools": tools_before,
    }
    cache_manifest_path = output / "cache-manifest.json"
    write_new(cache_manifest_path, cache_manifest)
    record = {
        "schema": SCHEMA, "mode": mode, "phase": "build", "exit_code": 0,
        "source_stable": True, "cwd": str(root), "input_key": cache_key,
        "command": build_command,
        "feature_flags": MODES[mode]["runtime_features"],
        "runtime_features": MODES[mode]["runtime_features"],
        "harness_features": MODES[mode]["harness_features"],
        "cargo_features": MODES[mode]["cargo_features"],
        "instrumented": False,
        "runtime_source_map": {name: digest for name, digest in core_before.items()
                                if name.startswith("crates/wtime-runtime/src/")},
        "core_source_map": core_before,
        "source_identities": sources_before,
        "compiled_inputs": generated_compiled,
        "archived_sources": archived_sources,
        "source_manifest": {"path": str(output_manifest), "sha256": sha(output_manifest)},
        "core_resolution": summary["resolution"],
        "metadata": {"path": str(metadata_stdout), "sha256": sha(metadata_stdout),
                     "after_path": str(metadata_after_stdout),
                     "after_sha256": sha(metadata_after_stdout),
                     "summary": summary},
        "toolchain": {"before": tools_before, "after": tools_after,
                      "root_metadata_before": {"path": str(root_metadata_stdout),
                                               "sha256": sha(root_metadata_stdout),
                                               "summary": root_metadata_summary},
                      "root_metadata_after": {"path": str(root_metadata_after_stdout),
                                              "sha256": sha(root_metadata_after_stdout),
                                              "summary": _root_metadata_summary(root_metadata_after, root)}},
        "cache_manifest": {"path": str(cache_manifest_path), "sha256": sha(cache_manifest_path)},
        "workspace_manifest": {"path": str(generated_manifest),
                               "workspace_path": str(manifest_path),
                               "sha256": sha(generated_manifest)},
        "workspace_lock": {"path": str(output_lock), "workspace_path": str(lock_path),
                           "sha256": sha(output_lock)},
        "compiled_manifest": {"path": str(generated_manifest), "sha256": sha(generated_manifest)},
        "binary_path": str(binary), "binary_sha256": sha(binary),
        "helper_path": str(helper_path), "helper_sha256": sha(helper_path),
        "protocol_command": protocol_command, "protocol_exit": 0,
        "protocol_stdout": str(protocol_stdout), "protocol_stderr": str(protocol_stderr),
        "protocol_sha256": sha(protocol_path), "protocol": protocol,
        "stdout_sha256": sha(build_stdout), "stderr_sha256": sha(build_stderr),
        "stdout_log": {"path": str(stdout_log), "sha256": sha(stdout_log)},
        "stderr_log": {"path": str(stderr_log), "sha256": sha(stderr_log)},
        "logs": {path.name: {"path": str(path), "sha256": sha(path)}
                 for path in sorted(output.iterdir())
                 if path.is_file() and (path.name.endswith((".stdout", ".stderr", ".command.json", ".status.json")))},
    }
    record_path = output / "build-record.json"
    write_new(record_path, record)
    return record


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mode", choices=sorted(MODES), required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    record = build(args.output, args.mode)
    print(json.dumps({"status": "passed", "mode": args.mode,
                      "input_key": record["input_key"],
                      "binary_sha256": record["binary_sha256"]}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
