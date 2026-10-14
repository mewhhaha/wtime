#!/usr/bin/env python3
"""Build and attest an isolated Register entry-batch adapter for a frozen core."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import time
import tomllib

SCHEMA = "wtime.source-bound-register-entry-adapter.v1"
KIND = "source-bound-register-entry-adapter"
FEATURES = ["register-executor", "runtime-only"]
REQUIRED_CRATES = (
    "wtime-runtime", "wtime-types", "wtime-decode", "wtime-validate",
)
PROTOCOL_SOURCE = "src/protocol.rs"
DRIVER_SOURCE = "src/entry_batch_driver.rs"
ENTRYPOINT = '''#![forbid(unsafe_code)]
mod protocol;

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if !protocol::try_main(&args) {
        eprintln!("unsupported entry-batch command");
        std::process::exit(2);
    }
}
'''


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def _json_bytes(value: object) -> bytes:
    return (json.dumps(value, sort_keys=True, indent=2) + "\n").encode()


def _inside(path: Path, root: Path) -> bool:
    try:
        path.resolve().relative_to(root.resolve())
        return True
    except ValueError:
        return False


def _safe_rel(value: str) -> str:
    candidate = Path(value)
    if candidate.is_absolute() or not value or any(part in ("", ".", "..") for part in value.split("/")):
        raise ValueError(f"unsafe relative path in manifest: {value!r}")
    return candidate.as_posix()


def _load_archive(archive: Path) -> tuple[dict, dict[str, dict]]:
    supplied = Path(archive)
    if supplied.is_symlink():
        raise ValueError("core archive path must not be a symlink")
    archive = supplied.resolve(strict=True)
    manifest_path = archive / "manifest.json"
    if manifest_path.is_symlink() or not _inside(manifest_path, archive):
        raise ValueError("core archive manifest is a symlink or escapes archive root")
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    files = manifest.get("files")
    if manifest.get("schema") != 1 or not isinstance(files, dict):
        raise ValueError("unsupported core archive manifest")
    present = set()
    for p in archive.rglob("*"):
        if p.is_symlink():
            raise ValueError(f"core archive contains a symlink member: {p.relative_to(archive)}")
        if not _inside(p, archive):
            raise ValueError(f"core archive member escapes archive root: {p.relative_to(archive)}")
        if p.is_file() and p != manifest_path:
            present.add(p.relative_to(archive).as_posix())
    if present != set(files):
        raise ValueError("core archive files differ from its manifest")
    for rel, record in files.items():
        rel = _safe_rel(rel)
        p = archive / rel
        if not isinstance(record, dict) or p.stat().st_size != record.get("size_bytes"):
            raise ValueError(f"core archive size mismatch: {rel}")
        if sha256_file(p) != record.get("sha256"):
            raise ValueError(f"core archive hash mismatch: {rel}")
    if manifest.get("mode") != "register":
        raise ValueError("core archive is not Register mode")
    policy = manifest.get("execution_policy", {})
    if policy.get("resolved_mode") != "register" or "register" not in policy.get("protocol_modes", []):
        raise ValueError("core archive manifest does not attest Register support")
    source = {rel: rec for rel, rec in files.items() if rec.get("role") == "source"}
    required = {f"source/crates/{name}/Cargo.toml" for name in REQUIRED_CRATES}
    if "source/Cargo.toml" not in source or not required.issubset(source):
        raise ValueError("core archive does not contain the complete runtime workspace manifests")
    for crate in REQUIRED_CRATES:
        prefix = f"source/crates/{crate}/"
        if not any(rel.startswith(prefix) and rel.endswith(".rs") for rel in source):
            raise ValueError(f"core archive has no Rust sources for {crate}")
    runtime_manifest = tomllib.loads((archive / "source/crates/wtime-runtime/Cargo.toml").read_text())
    if "register-executor" not in runtime_manifest.get("features", {}):
        raise ValueError("archived runtime does not declare register-executor")
    runtime_source = "\n".join(
        (archive / rel).read_text(encoding="utf-8", errors="replace")
        for rel in sorted(source) if rel.startswith("source/crates/wtime-runtime/") and rel.endswith(".rs")
    )
    if "with_execution_policy" not in runtime_source or "with_register_entry_threshold" not in runtime_source:
        raise ValueError("archived runtime lacks required P134 Register constructors")
    return manifest, source


def _harness_inputs(harness: Path) -> dict[str, Path]:
    harness = Path(harness).resolve(strict=True)
    src = harness / "src"
    if src.is_symlink() or not _inside(src, harness):
        raise ValueError("harness src directory is a symlink or escapes harness root")
    protocol = src / "protocol.rs"
    driver = src / "entry_batch_driver.rs"
    if not protocol.is_file() or not driver.is_file():
        raise ValueError("harness lacks P135 protocol.rs or entry_batch_driver.rs")
    # Copy every direct Rust leaf except the production entry point. This is a
    # superset of the current protocol module closure and remains hash-bound.
    leaves = sorted(src.glob("*.rs"))
    for path in leaves:
        if path.is_symlink() or not _inside(path, harness):
            raise ValueError(f"harness Rust source is a symlink or escapes harness root: {path.name}")
    inputs = {f"src/{p.name}": p for p in leaves if p.name != "main.rs"}
    for required in (PROTOCOL_SOURCE, DRIVER_SOURCE):
        if required not in inputs:
            raise ValueError(f"missing required harness source {required}")
    return inputs


def _workspace_manifest() -> str:
    lines = [
        "[package]", 'name = "wtime-register-entry-adapter"', 'version = "0.1.0"',
        'edition = "2024"', 'license = "MIT OR Apache-2.0"', "publish = false", "",
        "[workspace]", "resolver = \"3\"", "",
        "[features]", "default = []", "runtime-only = []",
        'register-executor = ["wtime-runtime/register-executor"]',
        "execution-reference = []", "performance-counters = []", "legacy-api = []",
        "compact-executor = []", "experimental-executors = []", "",
        "[dependencies]",
        'wtime-runtime = { path = "../source/crates/wtime-runtime" }',
        'wtime-types = { path = "../source/crates/wtime-types" }', "",
    ]
    return "\n".join(lines)


def _run(argv: list[str], cwd: Path, log: dict, label: str) -> subprocess.CompletedProcess:
    entry = {"label": label, "argv": argv, "cwd": str(cwd), "returncode": None,
             "stdout": "", "stderr": ""}
    log["commands"].append(entry)
    log_path = log["path"]
    log_path.write_bytes(_json_bytes({k: v for k, v in log.items() if k != "path"}))
    result = subprocess.run(argv, cwd=cwd, text=True, capture_output=True, check=False)
    entry.update(returncode=result.returncode, stdout=result.stdout, stderr=result.stderr)
    log_path.write_bytes(_json_bytes({k: v for k, v in log.items() if k != "path"}))
    if result.returncode:
        raise RuntimeError(f"{label} failed ({result.returncode}):\n{result.stdout}\n{result.stderr}")
    return result


def _command_log(stage: Path) -> dict:
    commands_dir = stage / "commands"
    commands_dir.mkdir(parents=True)
    helper_copy = commands_dir / "entry_batch_adapter.py"
    helper_copy.write_bytes(Path(__file__).read_bytes())
    helper_copy.chmod(0o644)
    log = {"schema": "wtime-entry-adapter-commands.v1", "helper_sha256": sha256_file(helper_copy),
           "commands": [], "path": commands_dir / "commands.json"}
    log["path"].write_bytes(_json_bytes({k: v for k, v in log.items() if k != "path"}))
    return log


def _copy_input(source: Path, target: Path) -> None:
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, target)


def _refresh_rust_mtimes(root: Path) -> None:
    now = time.time_ns()
    for path in root.rglob("*.rs"):
        os.utime(path, ns=(now, now))


def _file_map(root: Path) -> dict[str, dict]:
    result = {}
    for path in sorted(p for p in root.rglob("*") if p.is_file() and p.name != "adapter.json"):
        rel = path.relative_to(root).as_posix()
        result[rel] = {"sha256": sha256_file(path), "size_bytes": path.stat().st_size,
                       "mode_bits": stat.S_IMODE(path.stat().st_mode)}
    return result


def _verify_generated_workspace(output: Path, manifest: dict) -> None:
    workspace = output / "workspace"
    cargo_manifest = workspace / "Cargo.toml"
    generated_main = workspace / "src/main.rs"
    lock = workspace / "Cargo.lock"
    if not cargo_manifest.is_file() or cargo_manifest.read_bytes() != _workspace_manifest().encode():
        raise ValueError("generated adapter Cargo.toml differs from the required isolated manifest")
    if not generated_main.is_file() or generated_main.read_text(encoding="utf-8") != ENTRYPOINT:
        raise ValueError("generated adapter entrypoint differs from the required dispatcher")
    if sha256_file(cargo_manifest) != manifest.get("generated_manifest_sha256"):
        raise ValueError("generated adapter manifest changed")
    if not lock.is_file() or sha256_file(lock) != manifest.get("isolated_lock_sha256"):
        raise ValueError("isolated adapter lockfile changed")
    parsed = tomllib.loads(cargo_manifest.read_text(encoding="utf-8"))
    if (parsed.get("package", {}).get("name") != "wtime-register-entry-adapter"
            or set(parsed.get("dependencies", {})) != {"wtime-runtime", "wtime-types"}
            or parsed.get("features", {}).get("runtime-only") != []
            or parsed.get("features", {}).get("register-executor") != ["wtime-runtime/register-executor"]):
        raise ValueError("adapter package/features/dependencies are invalid")
    if (manifest.get("features") != FEATURES or manifest.get("profile") != "release"
            or manifest.get("instrumented") is not False or manifest.get("wasmtime_dependency") is not False
            or manifest.get("feature_profile") != "mvp" or manifest.get("source_fuel") is not None):
        raise ValueError("adapter feature/profile/instrumentation identity is invalid")


def _verify_command_log(output: Path, manifest: dict) -> dict[str, dict]:
    command_log_path = output / _safe_rel(manifest.get("commands", ""))
    command_log = json.loads(command_log_path.read_text(encoding="utf-8"))
    if command_log.get("schema") != "wtime-entry-adapter-commands.v1":
        raise ValueError("unsupported adapter command log schema")
    helper = output / "commands/entry_batch_adapter.py"
    if command_log.get("helper_sha256") != sha256_file(helper):
        raise ValueError("executed adapter helper source does not match its command log")
    entries = command_log.get("commands")
    labels_expected = {"cargo-metadata", "cargo-generate-lockfile", "cargo-build", "rustc-version", "cargo-version"}
    if not isinstance(entries, list) or len(entries) != len(labels_expected):
        raise ValueError("adapter command log has missing or duplicate commands")
    labels = [entry.get("label") for entry in entries if isinstance(entry, dict)]
    if len(labels) != len(entries) or len(set(labels)) != len(labels) or set(labels) != labels_expected:
        raise ValueError("adapter command log has missing, duplicate, or unknown labels")
    commands = {entry["label"]: entry for entry in entries}
    for label, entry in commands.items():
        if (type(entry.get("returncode")) is not int or entry["returncode"] != 0
                or not isinstance(entry.get("argv"), list) or not entry["argv"]
                or not isinstance(entry.get("cwd"), str)
                or not isinstance(entry.get("stdout"), str) or not isinstance(entry.get("stderr"), str)):
            raise ValueError(f"adapter command did not succeed or lacks exact logs: {label}")
    metadata_argv = commands["cargo-metadata"]["argv"]
    if (len(metadata_argv) != 7 or metadata_argv[1:] != ["metadata", "--format-version", "1", "--no-deps",
                                                         "--manifest-path", metadata_argv[-1]]):
        raise ValueError("adapter Cargo metadata command is invalid")
    cargo, manifest_path = metadata_argv[0], metadata_argv[-1]
    expected_generate = [cargo, "generate-lockfile", "--offline", "--manifest-path", manifest_path]
    expected_build = [cargo, "build", "--release", "--locked", "--offline", "-j", "4",
                      "--manifest-path", manifest_path, "--no-default-features", "--features",
                      ",".join(FEATURES), "--message-format=json-render-diagnostics"]
    if commands["cargo-generate-lockfile"]["argv"] != expected_generate:
        raise ValueError("adapter lockfile generation flags are invalid")
    if commands["cargo-build"]["argv"] != expected_build:
        raise ValueError("adapter build flags differ from locked offline Register release build")
    if (len(commands["rustc-version"]["argv"]) != 2
            or commands["rustc-version"]["argv"][-1] != "-Vv"
            or len(commands["cargo-version"]["argv"]) != 2
            or commands["cargo-version"]["argv"] != [cargo, "-V"]):
        raise ValueError("adapter toolchain version commands are invalid")
    toolchain = manifest.get("toolchain", {})
    if (commands["rustc-version"]["stdout"] != toolchain.get("rustc_vv")
            or commands["cargo-version"]["stdout"] != toolchain.get("cargo_v")):
        raise ValueError("adapter toolchain output differs from its manifest")
    return commands


def _source_snapshot(archive: Path, harness: Path, stage: Path) -> tuple[dict, dict, dict]:
    archive_manifest, archive_sources = _load_archive(archive)
    harness_inputs = _harness_inputs(harness)
    core_map = {}
    for rel, record in sorted(archive_sources.items()):
        target_rel = rel.removeprefix("source/")
        target = stage / "source" / target_rel
        _copy_input(archive / rel, target)
        core_map[target_rel] = {"archive_path": rel, "sha256": record["sha256"],
                                "size_bytes": record["size_bytes"]}
    overlay_map = {}
    for rel, source in sorted(harness_inputs.items()):
        target = stage / "workspace" / rel
        _copy_input(source, target)
        overlay_map[rel] = {"sha256": sha256_file(source), "size_bytes": source.stat().st_size}
    main = stage / "workspace/src/main.rs"
    main.parent.mkdir(parents=True, exist_ok=True)
    main.write_text(ENTRYPOINT, encoding="utf-8")
    overlay_map["src/main.rs"] = {"sha256": sha256_file(main), "size_bytes": main.stat().st_size,
                                  "generated": True}
    manifest = (stage / "workspace/Cargo.toml")
    manifest.parent.mkdir(parents=True, exist_ok=True)
    manifest.write_text(_workspace_manifest(), encoding="utf-8")
    _refresh_rust_mtimes(stage / "source")
    _refresh_rust_mtimes(stage / "workspace/src")
    return archive_manifest, core_map, overlay_map


def _current_source_paths(repo: Path) -> dict[str, Path]:
    """Return the complete live Cargo workspace source closure for core members."""
    repo = Path(repo).resolve(strict=True)
    root_manifest = repo / "Cargo.toml"
    if not root_manifest.is_file():
        raise ValueError("current repository lacks its root Cargo.toml")
    workspace = tomllib.loads(root_manifest.read_text(encoding="utf-8")).get("workspace", {})
    members = set(workspace.get("members", []))
    required_members = {"crates/*", "tools/wtime-conformance"}
    if not required_members.issubset(members):
        raise ValueError("current repository workspace does not expose required core members")
    paths: set[Path] = {
        root_manifest,
        repo / "Cargo.lock",
        repo / "rust-toolchain.toml",
        repo / "LICENSE-APACHE",
        repo / "LICENSE-MIT",
    }
    paths.update(p for p in (repo / ".cargo").glob("config*") if p.is_file())
    for base in (repo / "crates", repo / "tools/wtime-conformance"):
        if not base.is_dir():
            raise ValueError(f"current workspace member directory is missing: {base.relative_to(repo)}")
        for path in base.rglob("*"):
            if not path.is_file() or any(part in {"target", ".git"} for part in path.relative_to(repo).parts):
                continue
            resolved = path.resolve(strict=True)
            if not _inside(resolved, repo):
                raise ValueError(f"workspace source escapes current repository: {path}")
            paths.add(path)
    result = {}
    for path in sorted(paths):
        if not path.is_file():
            continue
        rel = path.relative_to(repo).as_posix()
        if any(part in {"target", ".git"} for part in Path(rel).parts):
            continue
        result[rel] = path
    if "Cargo.toml" not in result or "Cargo.lock" not in result:
        raise ValueError("current repository snapshot lacks root Cargo files")
    for crate in REQUIRED_CRATES:
        if f"crates/{crate}/Cargo.toml" not in result:
            raise ValueError(f"current workspace manifest missing for {crate}")
        if not any(p.startswith(f"crates/{crate}/") and p.endswith(".rs") for p in result):
            raise ValueError(f"current workspace has no Rust sources for {crate}")
    return result


def _validate_current_core(source_root: Path) -> None:
    runtime_manifest = tomllib.loads((source_root / "crates/wtime-runtime/Cargo.toml").read_text(encoding="utf-8"))
    if "register-executor" not in runtime_manifest.get("features", {}):
        raise ValueError("current runtime does not declare register-executor")
    runtime_source = "\n".join(
        path.read_text(encoding="utf-8", errors="replace")
        for path in sorted((source_root / "crates/wtime-runtime").rglob("*.rs"))
    )
    if "with_execution_policy" not in runtime_source or "with_register_entry_threshold" not in runtime_source:
        raise ValueError("current runtime lacks required Register constructors")


def _git_head(repo: Path) -> str | None:
    try:
        value = subprocess.check_output(["git", "-C", str(repo), "rev-parse", "HEAD"],
                                        text=True, stderr=subprocess.DEVNULL).strip()
        return value or None
    except (OSError, subprocess.CalledProcessError):
        return None


def _copy_current_source(repo: Path, harness: Path, stage: Path) -> tuple[dict, dict]:
    source_paths = _current_source_paths(repo)
    _validate_current_core(repo)
    core_map = {}
    for rel, source in source_paths.items():
        target = stage / "source" / rel
        _copy_input(source, target)
        core_map[rel] = {"source_path": rel, "sha256": sha256_file(source),
                         "size_bytes": source.stat().st_size}
    overlay_map = {}
    for rel, source in sorted(_harness_inputs(harness).items()):
        _copy_input(source, stage / "workspace" / rel)
        overlay_map[rel] = {"sha256": sha256_file(source), "size_bytes": source.stat().st_size}
    main = stage / "workspace/src/main.rs"
    main.parent.mkdir(parents=True, exist_ok=True)
    main.write_text(ENTRYPOINT, encoding="utf-8")
    overlay_map["src/main.rs"] = {"sha256": sha256_file(main), "size_bytes": main.stat().st_size,
                                  "generated": True}
    manifest = stage / "workspace/Cargo.toml"
    manifest.parent.mkdir(parents=True, exist_ok=True)
    manifest.write_text(_workspace_manifest(), encoding="utf-8")
    _refresh_rust_mtimes(stage / "source")
    _refresh_rust_mtimes(stage / "workspace/src")
    return core_map, overlay_map


def _build_workspace(stage: Path, output: Path, log: dict, core_map: dict, overlay_map: dict,
                     schema: str, kind: str, source_identity: dict, validate_inputs) -> dict:
    workspace = stage / "workspace"
    cargo = os.environ.get("CARGO", "cargo")
    metadata_argv = [cargo, "metadata", "--format-version", "1", "--no-deps", "--manifest-path", str(workspace / "Cargo.toml")]
    metadata_result = _run(metadata_argv, workspace, log, "cargo-metadata")
    metadata = json.loads(metadata_result.stdout)
    target_directory = Path(metadata["target_directory"]).resolve()
    generate_argv = [cargo, "generate-lockfile", "--offline", "--manifest-path", str(workspace / "Cargo.toml")]
    _run(generate_argv, workspace, log, "cargo-generate-lockfile")
    _refresh_rust_mtimes(stage / "source")
    _refresh_rust_mtimes(workspace / "src")
    build_argv = [cargo, "build", "--release", "--locked", "--offline", "-j", "4",
                  "--manifest-path", str(workspace / "Cargo.toml"), "--no-default-features",
                  "--features", ",".join(FEATURES), "--message-format=json-render-diagnostics"]
    build_result = _run(build_argv, workspace, log, "cargo-build")
    cargo_executable, artifact = _read_artifact(build_result.stdout, "wtime-register-entry-adapter")
    if not cargo_executable.is_file():
        raise ValueError("Cargo compiler-artifact executable is missing")
    binary = stage / "binary/wtime-register-entry-adapter"
    _copy_input(cargo_executable, binary)
    binary.chmod(binary.stat().st_mode | 0o111)
    rustc = _run([os.environ.get("RUSTC", "rustc"), "-Vv"], workspace, log, "rustc-version")
    cargo_version = _run([cargo, "-V"], workspace, log, "cargo-version")
    validate_inputs()
    target_rel = None
    try:
        target_rel = target_directory.relative_to(stage).as_posix()
    except ValueError:
        target_rel = None
    final_target = output / target_directory.relative_to(stage) if _inside(target_directory, stage) else target_directory
    final_executable = (output / cargo_executable.relative_to(stage)
                        if _inside(cargo_executable, stage) else cargo_executable)
    leaf_map = {k: v for k, v in overlay_map.items()
                if k not in (PROTOCOL_SOURCE, DRIVER_SOURCE, "src/main.rs")}
    return {
        "schema": schema, "kind": kind, "mode": "register", **source_identity,
        "copied_core_sources": dict(sorted(core_map.items())),
        "core_source_map": {rel: rec["sha256"] for rel, rec in sorted(core_map.items())},
        "protocol_source": PROTOCOL_SOURCE, "protocol_sha256": overlay_map[PROTOCOL_SOURCE]["sha256"],
        "entry_driver_source": DRIVER_SOURCE, "entry_batch_driver_sha256": overlay_map[DRIVER_SOURCE]["sha256"],
        "protocol_leaf_sources": dict(sorted(leaf_map.items())),
        "shared_leaf_map": {rel: rec["sha256"] for rel, rec in sorted(leaf_map.items())},
        "overlay_sources": dict(sorted(overlay_map.items())),
        "features": FEATURES, "profile": "release", "instrumented": False,
        "wasmtime_dependency": False, "feature_profile": "mvp", "source_fuel": None,
        "generated_manifest_sha256": sha256_file(workspace / "Cargo.toml"),
        "isolated_lock_sha256": sha256_file(workspace / "Cargo.lock"),
        "cargo_metadata_target_directory": str(final_target),
        "cargo_metadata_target_directory_relative": target_rel,
        "cargo_executable_path": str(final_executable.resolve()),
        "cargo_executable_sha256": sha256_file(cargo_executable),
        "executable": "binary/wtime-register-entry-adapter",
        "binary_path": str((output / "binary/wtime-register-entry-adapter").resolve()),
        "executable_sha256": sha256_file(binary), "cargo_artifact": artifact,
        "toolchain": {"rustc_vv": rustc.stdout, "cargo_v": cargo_version.stdout},
        "source_roots": {"core": str((output / "source").resolve()),
                         "harness": str((output / "workspace/src").resolve()),
                         "workspace": str((output / "workspace").resolve())},
        "commands": "commands/commands.json", "files": _file_map(stage),
    }


def _read_artifact(stdout: str, package_name: str) -> tuple[Path, dict]:
    artifacts = []
    for line in stdout.splitlines():
        try:
            item = json.loads(line)
        except json.JSONDecodeError:
            continue
        if item.get("reason") == "compiler-artifact" and item.get("target", {}).get("name") == package_name:
            executable = item.get("executable")
            if executable:
                artifacts.append((Path(executable), item))
    if len(artifacts) != 1:
        raise ValueError(f"Cargo emitted {len(artifacts)} adapter compiler-artifact records")
    return artifacts[0]


def build(archive: Path, harness: Path, output: Path) -> dict:
    """Verify inputs, snapshot exact sources, build and return the adapter manifest."""
    archive, harness, output = Path(archive).resolve(strict=True), Path(harness).resolve(strict=True), Path(output).resolve()
    if output == archive or output == harness or _inside(output, archive) or _inside(output, harness):
        raise ValueError("adapter output must not overlap archive or harness inputs")
    _load_archive(archive)
    _harness_inputs(harness)
    if output.exists():
        if any(output.iterdir()):
            return verify(output, archive, harness)
        output.rmdir()
    output.parent.mkdir(parents=True, exist_ok=True)
    stage = Path(tempfile.mkdtemp(prefix=f".{output.name}.staging-", dir=output.parent)).resolve()
    log = _command_log(stage)
    try:
        archive_manifest, core_map, overlay_map = _source_snapshot(archive, harness, stage)
        archive_manifest_sha = sha256_file(archive / "manifest.json")
        def validate_inputs() -> None:
            if sha256_file(archive / "manifest.json") != archive_manifest_sha:
                raise ValueError("core archive manifest changed during adapter build")
            _load_archive(archive)
            for rel, record in core_map.items():
                if sha256_file(archive / record["archive_path"]) != record["sha256"]:
                    raise ValueError(f"core archive source changed during adapter build: {rel}")
            for rel, record in overlay_map.items():
                if not record.get("generated") and sha256_file(harness / rel) != record["sha256"]:
                    raise ValueError(f"harness source changed during adapter build: {rel}")
        manifest = _build_workspace(
            stage, output, log, core_map, overlay_map, SCHEMA, KIND,
            {"archive": str(archive), "archive_manifest_sha256": archive_manifest_sha,
             "archive_mode": archive_manifest["mode"]}, validate_inputs,
        )
        (stage / "adapter.json").write_bytes(_json_bytes(manifest))
        if output.exists():
            raise FileExistsError(f"refusing to replace existing adapter output {output}")
        stage.rename(output)
        return verify(output, archive, harness)
    except BaseException:
        # Keep the unique staging tree: command logs and the exact executed
        # helper are provenance for failed attempts too.
        raise


CURRENT_SCHEMA = "wtime.source-bound-register-entry-candidate.v1"
CURRENT_KIND = "source-bound-register-entry-candidate"


def build_current(repo: Path, harness: Path, output: Path) -> dict:
    """Build the same tiny Register harness against a byte-bound live workspace snapshot."""
    repo, harness, output = Path(repo).resolve(strict=True), Path(harness).resolve(strict=True), Path(output).resolve()
    protected = (repo / "crates", repo / "tools/wtime-conformance")
    if (output == repo or _inside(repo, output) or output == harness or _inside(output, harness)
            or any(output == path or _inside(output, path) for path in protected)):
        raise ValueError("candidate output must not overlap repository inputs or harness")
    expected = _current_source_paths(repo)
    _validate_current_core(repo)
    initial_source_map = {rel: sha256_file(path) for rel, path in expected.items()}
    initial_head = _git_head(repo)
    initial_harness = {rel: sha256_file(path) for rel, path in _harness_inputs(harness).items()}
    if output.exists():
        if any(output.iterdir()):
            return verify_current(output, repo, harness)
        output.rmdir()
    output.parent.mkdir(parents=True, exist_ok=True)
    stage = Path(tempfile.mkdtemp(prefix=f".{output.name}.staging-", dir=output.parent)).resolve()
    log = _command_log(stage)
    try:
        core_map, overlay_map = _copy_current_source(repo, harness, stage)
        copied_map = {rel: rec["sha256"] for rel, rec in core_map.items()}
        if copied_map != initial_source_map:
            raise ValueError("current workspace source membership/content changed during snapshot")

        def validate_inputs() -> None:
            current = _current_source_paths(repo)
            current_map = {rel: sha256_file(path) for rel, path in current.items()}
            if current_map != initial_source_map:
                raise ValueError("current workspace source membership/content changed during candidate build")
            if _git_head(repo) != initial_head:
                raise ValueError("current repository HEAD changed during candidate build")
            live_harness = {rel: sha256_file(path) for rel, path in _harness_inputs(harness).items()}
            if live_harness != initial_harness:
                raise ValueError("current protocol/driver/leaf source changed during candidate build")

        validate_inputs()
        manifest = _build_workspace(
            stage, output, log, core_map, overlay_map, CURRENT_SCHEMA, CURRENT_KIND,
            {"source_origin": "current-workspace", "repository_root": str(repo),
             "repository_head": initial_head, "current_source_map": dict(sorted(initial_source_map.items()))},
            validate_inputs,
        )
        (stage / "adapter.json").write_bytes(_json_bytes(manifest))
        if output.exists():
            raise FileExistsError(f"refusing to replace existing candidate output {output}")
        stage.rename(output)
        return verify_current(output, repo, harness)
    except BaseException:
        # Keep the unique staging tree: command logs and the exact executed
        # helper are provenance for failed attempts too.
        raise


def verify_current(output: Path, repo: Path, harness: Path) -> dict:
    """Verify the candidate's live workspace snapshot, harness, commands and binary."""
    output, repo, harness = Path(output).resolve(strict=True), Path(repo).resolve(strict=True), Path(harness).resolve(strict=True)
    protected = (repo / "crates", repo / "tools/wtime-conformance")
    if (output == repo or _inside(repo, output) or output == harness or _inside(output, harness)
            or any(output == path or _inside(output, path) for path in protected)):
        raise ValueError("candidate output overlaps repository inputs or harness")
    manifest = json.loads((output / "adapter.json").read_text(encoding="utf-8"))
    if (manifest.get("schema") != CURRENT_SCHEMA or manifest.get("kind") != CURRENT_KIND
            or manifest.get("source_origin") != "current-workspace" or manifest.get("mode") != "register"):
        raise ValueError("unsupported or foreign current-candidate schema/kind/source origin")
    if manifest.get("repository_root") != str(repo):
        raise ValueError("candidate is bound to a different repository root")
    recorded_head = manifest.get("repository_head")
    if recorded_head is not None and (not isinstance(recorded_head, str)
            or not re.fullmatch(r"(?:[0-9a-f]{40}|[0-9a-f]{64})", recorded_head)):
        raise ValueError("candidate repository HEAD provenance is malformed")
    current_paths = _current_source_paths(repo)
    current_map = {rel: sha256_file(path) for rel, path in current_paths.items()}
    if current_map != manifest.get("current_source_map"):
        raise ValueError("current candidate source membership or hashes are stale")
    copied = manifest.get("copied_core_sources", {})
    if {rel: rec.get("sha256") for rel, rec in copied.items()} != current_map:
        raise ValueError("candidate copied-core map differs from live workspace source map")
    _validate_current_core(output / "source")
    for rel, rec in copied.items():
        target = output / "source" / _safe_rel(rel)
        if not target.is_file() or target.stat().st_size != rec.get("size_bytes") or sha256_file(target) != rec.get("sha256"):
            raise ValueError(f"candidate copied source is missing or dirty: {rel}")
        if rec.get("source_path") != rel or sha256_file(repo / rel) != rec["sha256"]:
            raise ValueError(f"candidate current source input changed: {rel}")
    current_harness = _harness_inputs(harness)
    expected_overlay = {rel: sha256_file(path) for rel, path in current_harness.items()}
    recorded_overlay = {rel: rec.get("sha256") for rel, rec in manifest.get("overlay_sources", {}).items()
                        if not rec.get("generated")}
    if expected_overlay != recorded_overlay:
        raise ValueError("candidate protocol/driver/leaf overlay is stale or incomplete")
    _verify_generated_workspace(output, manifest)
    for rel, rec in manifest.get("overlay_sources", {}).items():
        target = output / "workspace" / _safe_rel(rel)
        if not target.is_file() or sha256_file(target) != rec.get("sha256"):
            raise ValueError(f"candidate overlay source is missing or dirty: {rel}")
    if manifest.get("core_source_map") != current_map:
        raise ValueError("candidate core source hash map is stale")
    if manifest.get("protocol_sha256") != expected_overlay.get(PROTOCOL_SOURCE):
        raise ValueError("candidate protocol hash is stale")
    if manifest.get("entry_batch_driver_sha256") != expected_overlay.get(DRIVER_SOURCE):
        raise ValueError("candidate entry-batch driver hash is stale")
    expected_leaves = {rel: sha for rel, sha in expected_overlay.items()
                       if rel not in (PROTOCOL_SOURCE, DRIVER_SOURCE)}
    if manifest.get("shared_leaf_map") != expected_leaves:
        raise ValueError("candidate shared leaf hash map is stale")
    workspace_manifest = output / "workspace/Cargo.toml"
    lock = output / "workspace/Cargo.lock"
    if (sha256_file(workspace_manifest) != manifest.get("generated_manifest_sha256")
            or sha256_file(lock) != manifest.get("isolated_lock_sha256")):
        raise ValueError("candidate generated manifest or lockfile changed")
    parsed = tomllib.loads(workspace_manifest.read_text(encoding="utf-8"))
    if (parsed.get("package", {}).get("name") != "wtime-register-entry-adapter"
            or set(parsed.get("dependencies", {})) != {"wtime-runtime", "wtime-types"}
            or parsed.get("features", {}).get("register-executor") != ["wtime-runtime/register-executor"]):
        raise ValueError("candidate workspace package/features/dependencies are invalid")
    commands = _verify_command_log(output, manifest)
    argv = commands["cargo-build"].get("argv", [])
    if ("--release" not in argv or "--locked" not in argv or "--offline" not in argv
            or "--no-default-features" not in argv or "CARGO_TARGET_DIR" in argv
            or argv[argv.index("--features") + 1] != ",".join(FEATURES)):
        raise ValueError("candidate Cargo build flags are invalid")
    metadata = json.loads(commands["cargo-metadata"].get("stdout", ""))
    raw_target = Path(metadata.get("target_directory", "")).resolve()
    relative_target = manifest.get("cargo_metadata_target_directory_relative")
    if relative_target is not None:
        if (output / relative_target).resolve() != Path(manifest["cargo_metadata_target_directory"]).resolve():
            raise ValueError("candidate target directory identity is inconsistent")
        raw_expected = (Path(commands["cargo-metadata"]["cwd"]).resolve().parent / relative_target).resolve()
    else:
        raw_expected = Path(manifest["cargo_metadata_target_directory"]).resolve()
    if raw_target != raw_expected:
        raise ValueError("candidate Cargo metadata target directory is stale")
    artifacts = []
    for line in commands["cargo-build"].get("stdout", "").splitlines():
        try:
            item = json.loads(line)
        except json.JSONDecodeError:
            continue
        if item.get("reason") == "compiler-artifact" and item.get("target", {}).get("name") == "wtime-register-entry-adapter":
            artifacts.append(item)
    if len(artifacts) != 1 or artifacts[0].get("executable") != manifest.get("cargo_artifact", {}).get("executable"):
        raise ValueError("candidate Cargo artifact record is inconsistent")
    cargo_executable = Path(manifest.get("cargo_executable_path", "")).resolve()
    if (not cargo_executable.is_file() or sha256_file(cargo_executable) != manifest.get("cargo_executable_sha256")
            or manifest.get("cargo_executable_sha256") != manifest.get("executable_sha256")):
        raise ValueError("candidate executable artifact is stale")
    if _file_map(output) != manifest.get("files"):
        raise ValueError("candidate output contains missing, extra, or dirty files")
    expected_roots = {"core": str((output / "source").resolve()),
                      "harness": str((output / "workspace/src").resolve()),
                      "workspace": str((output / "workspace").resolve())}
    if manifest.get("source_roots") != expected_roots:
        raise ValueError("candidate source roots are inconsistent")
    binary = output / _safe_rel(manifest.get("executable", ""))
    if not binary.is_file() or not os.access(binary, os.X_OK) or sha256_file(binary) != manifest.get("executable_sha256"):
        raise ValueError("candidate retained executable is missing, non-executable, or stale")
    if manifest.get("binary_path") != str(binary.resolve()):
        raise ValueError("candidate binary path is inconsistent")
    return manifest


def verify(output: Path, archive: Path, harness: Path) -> dict:
    """Verify adapter files and all current archive/protocol/driver bindings."""
    output, archive, harness = Path(output).resolve(strict=True), Path(archive).resolve(strict=True), Path(harness).resolve(strict=True)
    if output == archive or output == harness or _inside(output, archive) or _inside(output, harness):
        raise ValueError("adapter output overlaps archive or harness inputs")
    manifest = json.loads((output / "adapter.json").read_text(encoding="utf-8"))
    if manifest.get("schema") != SCHEMA or manifest.get("kind") != KIND or manifest.get("mode") != "register":
        raise ValueError("unsupported or foreign adapter schema/kind/mode")
    if manifest.get("archive") != str(archive) or manifest.get("archive_manifest_sha256") != sha256_file(archive / "manifest.json"):
        raise ValueError("adapter is bound to a different or stale archive")
    archived_manifest, source_map = _load_archive(archive)
    expected_core = {rel.removeprefix("source/"): rec["sha256"] for rel, rec in source_map.items()}
    recorded_core = {rel: rec.get("sha256") for rel, rec in manifest.get("copied_core_sources", {}).items()}
    if recorded_core != expected_core:
        raise ValueError("adapter copied-core source membership or hashes differ from archive")
    for rel, rec in manifest["copied_core_sources"].items():
        expected_archive_path = f"source/{_safe_rel(rel)}"
        archive_path = _safe_rel(rec.get("archive_path", ""))
        if archive_path != expected_archive_path:
            raise ValueError(f"adapter archive source path is foreign or unsafe: {rel}")
        p = output / "source" / _safe_rel(rel)
        if not p.is_file() or sha256_file(p) != rec["sha256"]:
            raise ValueError(f"copied core source missing or dirty: {rel}")
        if sha256_file(archive / archive_path) != rec["sha256"]:
            raise ValueError(f"source archive input changed: {rel}")
    current_harness = _harness_inputs(harness)
    expected_overlay = {rel: sha256_file(path) for rel, path in current_harness.items()}
    recorded_overlay = {rel: rec.get("sha256") for rel, rec in manifest.get("overlay_sources", {}).items()
                        if not rec.get("generated")}
    if recorded_overlay != expected_overlay:
        raise ValueError("adapter protocol/driver/leaf overlay is stale or incomplete")
    _verify_generated_workspace(output, manifest)
    for rel, rec in manifest["overlay_sources"].items():
        p = output / "workspace" / _safe_rel(rel)
        if not p.is_file() or sha256_file(p) != rec["sha256"]:
            raise ValueError(f"overlay source missing or dirty: {rel}")
    if manifest.get("features") != FEATURES or manifest.get("instrumented") is not False or manifest.get("wasmtime_dependency") is not False:
        raise ValueError("adapter feature/instrumentation/dependency identity is invalid")
    if manifest.get("feature_profile") != "mvp" or manifest.get("source_fuel") is not None:
        raise ValueError("adapter feature profile or source-fuel setting is invalid")
    workspace_manifest = output / "workspace/Cargo.toml"
    lock = output / "workspace/Cargo.lock"
    if sha256_file(workspace_manifest) != manifest.get("generated_manifest_sha256"):
        raise ValueError("generated adapter manifest changed")
    if sha256_file(lock) != manifest.get("isolated_lock_sha256"):
        raise ValueError("isolated adapter lockfile changed")
    parsed = tomllib.loads(workspace_manifest.read_text())
    deps = parsed.get("dependencies", {})
    if "wasmtime" in deps or set(deps) != {"wtime-runtime", "wtime-types"}:
        raise ValueError("adapter workspace has foreign or extra dependencies")
    if (parsed.get("package", {}).get("name") != "wtime-register-entry-adapter"
            or parsed.get("features", {}).get("runtime-only") != []
            or parsed.get("features", {}).get("register-executor") != ["wtime-runtime/register-executor"]):
        raise ValueError("adapter package identity or feature forwarding is invalid")
    commands = _verify_command_log(output, manifest)
    build_argv = commands["cargo-build"].get("argv", [])
    if ("--release" not in build_argv or "--locked" not in build_argv or "--offline" not in build_argv
            or "--no-default-features" not in build_argv or "CARGO_TARGET_DIR" in build_argv
            or build_argv[build_argv.index("--features") + 1] != ",".join(FEATURES)):
        raise ValueError("adapter build command does not match the required offline Register configuration")
    metadata_output = commands["cargo-metadata"].get("stdout", "")
    metadata = json.loads(metadata_output)
    raw_target = Path(metadata.get("target_directory", "")).resolve()
    if manifest.get("cargo_metadata_target_directory_relative") is None:
        expected_raw_target = Path(manifest.get("cargo_metadata_target_directory", "")).resolve()
    else:
        recorded_cwd = Path(commands["cargo-metadata"].get("cwd", "")).resolve()
        expected_raw_target = (recorded_cwd.parent / manifest["cargo_metadata_target_directory_relative"]).resolve()
        if (output / manifest["cargo_metadata_target_directory_relative"]).resolve() != Path(
                manifest.get("cargo_metadata_target_directory", "")).resolve():
            raise ValueError("recorded Cargo target directory is not contained at its expected workspace path")
    if raw_target != expected_raw_target:
        raise ValueError("recorded Cargo target directory differs from metadata output")
    artifact_lines = []
    for line in commands["cargo-build"].get("stdout", "").splitlines():
        try:
            item = json.loads(line)
        except json.JSONDecodeError:
            continue
        if item.get("reason") == "compiler-artifact" and item.get("target", {}).get("name") == "wtime-register-entry-adapter":
            artifact_lines.append(item)
    if len(artifact_lines) != 1 or artifact_lines[0].get("executable") != manifest.get("cargo_artifact", {}).get("executable"):
        raise ValueError("Cargo artifact record does not match adapter manifest")
    cargo_executable = Path(manifest.get("cargo_executable_path", "")).resolve()
    if (not cargo_executable.is_file() or sha256_file(cargo_executable) != manifest.get("cargo_executable_sha256")
            or manifest.get("cargo_executable_sha256") != manifest.get("executable_sha256")):
        raise ValueError("actual Cargo executable path/hash differs from the retained adapter executable")
    expected_files = manifest.get("files", {})
    actual_files = _file_map(output)
    if expected_files != actual_files:
        raise ValueError("adapter contains missing, extra, or dirty files")
    binary = output / _safe_rel(manifest.get("executable", ""))
    if not binary.is_file() or not os.access(binary, os.X_OK) or sha256_file(binary) != manifest.get("executable_sha256"):
        raise ValueError("adapter executable is missing, non-executable, or stale")
    if manifest.get("schema") != SCHEMA or manifest.get("kind") == "legacy-api-adapter":
        raise ValueError("legacy adapter identity is forbidden")
    return manifest


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    build_cmd = sub.add_parser("build")
    build_cmd.add_argument("--core-archive", required=True, type=Path)
    build_cmd.add_argument("--harness", required=True, type=Path)
    build_cmd.add_argument("--output", required=True, type=Path)
    verify_cmd = sub.add_parser("verify")
    verify_cmd.add_argument("--output", required=True, type=Path)
    verify_cmd.add_argument("--core-archive", required=True, type=Path)
    verify_cmd.add_argument("--harness", required=True, type=Path)
    build_current_cmd = sub.add_parser("build-current")
    build_current_cmd.add_argument("--repo", required=True, type=Path)
    build_current_cmd.add_argument("--harness", required=True, type=Path)
    build_current_cmd.add_argument("--output", required=True, type=Path)
    verify_current_cmd = sub.add_parser("verify-current")
    verify_current_cmd.add_argument("--output", required=True, type=Path)
    verify_current_cmd.add_argument("--repo", required=True, type=Path)
    verify_current_cmd.add_argument("--harness", required=True, type=Path)
    args = parser.parse_args(argv)
    try:
        if args.command == "build":
            result = build(args.core_archive, args.harness, args.output)
        elif args.command == "verify":
            result = verify(args.output, args.core_archive, args.harness)
        elif args.command == "build-current":
            result = build_current(args.repo, args.harness, args.output)
        else:
            result = verify_current(args.output, args.repo, args.harness)
        print(json.dumps({"schema": result["schema"], "kind": result["kind"],
                          "executable_sha256": result["executable_sha256"],
                          "copied_core_source_count": len(result.get("core_source_map", result["copied_core_sources"])),
                          "overlay_source_count": len(result["overlay_sources"])}))
        return 0
    except (OSError, ValueError, KeyError, RuntimeError, json.JSONDecodeError) as exc:
        print(f"entry-batch adapter {args.command} failed: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
