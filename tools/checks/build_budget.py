#!/usr/bin/env python3
"""Measure clean, incremental, runtime-leaf, and release build budgets."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[2]
LIMITS = {"clean_seconds": 60.0, "leaf_seconds": 5.0, "stripped_bytes": 5 * 1024 * 1024}
SKIP_DIRS = {".git", "target", "node_modules", "__pycache__"}


def cargo_metadata(*, features: str | None = None) -> dict:
    command = ["cargo", "metadata", "--locked", "--format-version=1"]
    if features:
        command.extend(["--features", features])
    return json.loads(subprocess.check_output(command, cwd=ROOT, text=True))


def _workspace_files(metadata: dict) -> list[Path]:
    """Return project files that can affect workspace compilation."""
    paths = {ROOT / "Cargo.toml", ROOT / "Cargo.lock"}
    for path in (ROOT / "rust-toolchain.toml", ROOT / "rust-toolchain", ROOT / ".cargo"):
        if path.is_file():
            paths.add(path)
        elif path.is_dir():
            paths.update(child for child in path.rglob("*") if child.is_file())
    for package in metadata.get("packages", []):
        if package.get("source") is not None:
            continue
        manifest = Path(package["manifest_path"]).resolve()
        try:
            package_root = manifest.parent
            package_root.relative_to(ROOT)
        except ValueError:
            continue
        for current, dirs, files in os.walk(package_root):
            dirs[:] = sorted(d for d in dirs if d not in SKIP_DIRS)
            for name in files:
                path = Path(current) / name
                if path.is_file():
                    paths.add(path)
    return sorted(path for path in paths if path.is_file())


def source_inputs(metadata: dict | None = None) -> dict[str, str]:
    metadata = metadata or cargo_metadata()
    result = {}
    for path in _workspace_files(metadata):
        try:
            relative = path.relative_to(ROOT).as_posix()
        except ValueError:
            continue
        result[relative] = hashlib.sha256(path.read_bytes()).hexdigest()
    return result


def hash_inputs(inputs: dict[str, str]) -> str:
    digest = hashlib.sha256()
    for name, checksum in sorted(inputs.items()):
        digest.update(name.encode() + b"\0" + checksum.encode() + b"\0")
    return digest.hexdigest()


def core_source_hash(root: Path | None = None) -> str:
    """Match hardening.py and release_audit.py core source digest."""
    base = root or ROOT
    paths = sorted([base / "Cargo.lock", base / "Cargo.toml",
                    *base.glob("crates/**/*.rs"), *base.glob("crates/**/Cargo.toml")])
    digest = hashlib.sha256()
    for path in paths:
        digest.update(str(path.relative_to(base)).encode() + b"\0" + path.read_bytes() + b"\0")
    return digest.hexdigest()


def source_hash() -> str:
    return hash_inputs(source_inputs())


def _stage_workspace(metadata: dict, expected_inputs: dict[str, str], destination: Path) -> Path:
    """Copy exactly the measured local compilation inputs into an isolated tree."""
    files = _workspace_files(metadata)
    relative = {path.relative_to(ROOT).as_posix(): path for path in files}
    if set(relative) != set(expected_inputs):
        raise ValueError("workspace input set changed while staging build measurement")
    destination.mkdir(parents=True, exist_ok=False)
    for name, expected in sorted(expected_inputs.items()):
        source = relative[name]
        if hashlib.sha256(source.read_bytes()).hexdigest() != expected:
            raise ValueError(f"workspace source drift before staging: {name}")
        target = destination / name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, target)
        if hashlib.sha256(target.read_bytes()).hexdigest() != expected:
            raise ValueError(f"workspace source changed during staging: {name}")
    if source_inputs(metadata) != expected_inputs:
        raise ValueError("workspace source drift while staging build measurement")
    return destination


def _staged_inputs(root: Path, expected_inputs: dict[str, str]) -> dict[str, str]:
    return {name: hashlib.sha256((root / name).read_bytes()).hexdigest()
            for name in expected_inputs}


def validate_feature_spec(spec: str, metadata: dict) -> tuple[str, str]:
    if spec.count("/") != 1:
        raise ValueError("--features must be PACKAGE/FEATURE")
    package_name, feature = spec.split("/", 1)
    if not package_name or not feature:
        raise ValueError("--features must be PACKAGE/FEATURE")
    members = set(metadata.get("workspace_members", []))
    matches = [package for package in metadata.get("packages", [])
               if package.get("name") == package_name and package.get("id") in members]
    if len(matches) != 1:
        raise ValueError(f"unknown workspace package {package_name!r}")
    if feature not in matches[0].get("features", {}):
        raise ValueError(f"unknown feature {feature!r} for package {package_name!r}")
    return package_name, feature


def _scrub_pkg_id(pkg_id: str) -> str:
    for prefix in dict.fromkeys((f"path+file://{ROOT.resolve().as_posix()}/", f"path+file://{ROOT.as_posix()}/")):
        pkg_id = pkg_id.replace(prefix, "path+file://./")
    return pkg_id


def _resolved_graph(metadata: dict) -> list[dict]:
    packages = {item["id"]: item for item in metadata.get("packages", [])}
    graph = metadata.get("resolve") or {}
    rows = []
    for node in graph.get("nodes", []):
        rows.append({
            "package": _scrub_pkg_id(node["id"]),
            "features": sorted(node.get("features", [])),
            "dependencies": sorted(
                [
                    {"package": _scrub_pkg_id(dependency["pkg"]), "name": dependency["name"],
                     "kinds": [{"kind": kind.get("kind"), "target": kind.get("target")}
                               for kind in dependency.get("dep_kinds", [])]}
                    for dependency in node.get("deps", [])
                ],
                key=lambda item: (item["package"], item["name"]),
            ),
            "name": packages.get(node["id"], {}).get("name"),
            "version": packages.get(node["id"], {}).get("version"),
        })
    return sorted(rows, key=lambda row: row["package"])


def _default_dependency_result(metadata: dict) -> tuple[bool, list[str]]:
    # Keep the production closure and unsafe scanner owned by dependency_budget.py.
    sys.path.insert(0, str(ROOT / "tools" / "checks"))
    import dependency_budget
    sources = {
        str(path.relative_to(ROOT)): path.read_text(errors="replace")
        for path in (ROOT / "crates").rglob("*.rs")
    }
    violations = dependency_budget.violations(metadata, sources)
    return not violations, violations


def run(command, *, env, cwd=ROOT, rss_path=None):
    start = time.monotonic()
    wrapper = (
        "import pathlib,resource,subprocess,sys; "
        "p=subprocess.run(sys.argv[2:]); "
        "pathlib.Path(sys.argv[1]).write_text(str(resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss)); "
        "sys.exit(p.returncode)"
    )
    measured = command if rss_path is None else [sys.executable, "-c", wrapper, str(rss_path), *command]
    result = subprocess.run(measured, cwd=cwd, env=env, check=False)
    elapsed = time.monotonic() - start
    if result.returncode:
        raise subprocess.CalledProcessError(result.returncode, command)
    return elapsed


def _build_command(jobs: int, feature_spec: str | None) -> list[str]:
    command = ["cargo", "build", "--locked", "--offline", "-j", str(jobs)]
    if feature_spec:
        command.extend(["--features", feature_spec])
    return command


def _runtime_leaf(command: list[str], env: dict[str, str], leaf: Path | None = None,
                  cwd: Path | None = None) -> float:
    cwd = cwd or ROOT
    leaf = leaf or cwd / "crates/wtime-cli/src/main.rs"
    stat = leaf.stat()
    contents = leaf.read_bytes()
    try:
        leaf.write_bytes(contents + b"\n// Build measurement touch.\n")
        return run(command, env=env, cwd=cwd)
    finally:
        # Restore exact bytes and timestamps even when writing/building fails.
        leaf.write_bytes(contents)
        os.utime(leaf, ns=(stat.st_atime_ns, stat.st_mtime_ns))


def check_budget(record: dict) -> bool:
    budgets = record.get("budgets", LIMITS)
    limits_pass = all(
        isinstance(record.get(key), (int, float))
        and not isinstance(record.get(key), bool)
        and record[key] <= limit
        for key, limit in budgets.items()
    )
    return (record.get("source_stable") is True and limits_pass
            and record.get("default_dependency_closure_pass") is True)


def measure(output: Path, feature_spec: str | None = None) -> dict:
    # Fetch, metadata, and all hashing happen outside measured build intervals.
    subprocess.run(["cargo", "fetch", "--locked"], cwd=ROOT, check=True)
    default_metadata = cargo_metadata()
    package_feature_metadata = cargo_metadata(features=feature_spec) if feature_spec else default_metadata
    if feature_spec:
        validate_feature_spec(feature_spec, default_metadata)
    before_inputs = source_inputs(default_metadata)
    before_hash = hash_inputs(before_inputs)
    before_core_hash = core_source_hash(ROOT)
    default_dependencies_ok, dependency_errors = _default_dependency_result(default_metadata)
    mode = "optional-feature" if feature_spec else "default"

    with tempfile.TemporaryDirectory(prefix="wtime-build-") as temporary:
        temporary_root = Path(temporary)
        build_root = _stage_workspace(default_metadata, before_inputs, temporary_root / "workspace")
        cargo_target = temporary_root / "cargo-target"
        cargo_target.mkdir()
        if source_inputs(default_metadata) != before_inputs:
            raise ValueError("workspace source drift before build measurement")
        env = dict(os.environ, CARGO_TARGET_DIR=str(cargo_target))
        jobs = min(os.cpu_count() or 1, 5)
        command = _build_command(jobs, feature_spec)
        record = {
            "schema_version": 2,
            "mode": mode,
            "features": [feature_spec] if feature_spec else [],
            "feature_package": feature_spec.split("/", 1)[0] if feature_spec else None,
            "feature_name": feature_spec.split("/", 1)[1] if feature_spec else None,
            "cpu": platform.processor() or platform.machine(),
            "logical_cpus": os.cpu_count(),
            "jobs": jobs,
            "build_command": command,
            "cargo_version": subprocess.check_output(["cargo", "-V"], text=True).strip(),
            "rustc": subprocess.check_output(["rustc", "-Vv"], text=True).strip(),
            "compile_environment": {
                name: os.environ[name]
                for name in ("CARGO_BUILD_TARGET", "CARGO_ENCODED_RUSTFLAGS", "RUSTFLAGS",
                             "CARGO_PROFILE_DEV_DEBUG", "CARGO_PROFILE_RELEASE_LTO",
                             "CARGO_PROFILE_RELEASE_CODEGEN_UNITS", "CARGO_INCREMENTAL")
                if name in os.environ
            },
            "lock_sha256": hashlib.sha256((ROOT / "Cargo.lock").read_bytes()).hexdigest(),
            "target_initially_empty": not any(cargo_target.iterdir()),
            "isolated_workspace": build_root.name,
            "isolated_workspace_source_sha256": before_hash,
            "budgets": dict(LIMITS),
            "source_inputs": before_inputs,
            "source_sha256": before_hash,
            "core_source_sha256": before_core_hash,
            "resolved_graph": _resolved_graph(package_feature_metadata),
            "default_dependency_closure_pass": default_dependencies_ok,
            "default_dependency_violations": dependency_errors,
        }
        clean_rss = Path(temporary) / "clean-rss"
        record["clean_seconds"] = run(command, env=env, cwd=build_root, rss_path=clean_rss)
        record["clean_peak_rss_kib"] = int(clean_rss.read_text().strip())
        record["unchanged_seconds"] = run(command, env=env, cwd=build_root)
        record["leaf_seconds"] = _runtime_leaf(command, env, cwd=build_root)
        record["runtime_leaf_seconds"] = _runtime_leaf(
            command, env, build_root / "crates/wtime-runtime/src/numeric.rs", cwd=build_root)
        record["release_seconds"] = run(command + ["--release"], env=env, cwd=build_root)
        executable = cargo_target / "release/wtime-cli"
        stripped = temporary_root / "stripped-cli"
        shutil.copy2(executable, stripped)
        subprocess.run(["strip", str(stripped)], check=True)
        record["stripped_bytes"] = stripped.stat().st_size
        rss = Path(temporary) / "rss"
        run(command, env=env, cwd=build_root, rss_path=rss)
        record["unchanged_peak_rss_kib"] = int(rss.read_text().strip())
        isolated_after = _staged_inputs(build_root, before_inputs)

    after_inputs = source_inputs(default_metadata)
    after_core_hash = core_source_hash(ROOT)
    record["isolated_source_inputs_after"] = isolated_after
    record["source_stable"] = (
        before_inputs == after_inputs == isolated_after
        and before_core_hash == after_core_hash
    )
    record["source_inputs_after"] = after_inputs
    record["source_sha256_after"] = hash_inputs(after_inputs)
    record["core_source_sha256_after"] = after_core_hash
    record["budget_pass"] = check_budget(record)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(record, indent=2, sort_keys=True) + "\n")
    return record


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "docs/verification/build-final.json")
    parser.add_argument("--features", metavar="PACKAGE/FEATURE")
    args = parser.parse_args(argv)
    try:
        record = measure(args.output, args.features)
    except (OSError, ValueError, subprocess.SubprocessError, json.JSONDecodeError) as error:
        print(f"build_budget.py: {error}", file=sys.stderr)
        return getattr(error, "returncode", 2) or 2
    print(json.dumps({k: record[k] for k in ("mode", "features", "clean_seconds", "unchanged_seconds", "leaf_seconds", "runtime_leaf_seconds", "release_seconds", "stripped_bytes", "clean_peak_rss_kib", "source_stable", "budget_pass") if k in record}, indent=2))
    return 0 if record["budget_pass"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
