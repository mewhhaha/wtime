#!/usr/bin/env python3
"""Build and attest an uninstrumented production benchmark binary."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
from typing import Callable

ROOT = Path(__file__).resolve().parents[3]
BENCH = ROOT / "tools/wtime-bench"
if str(BENCH) not in sys.path:
    sys.path.insert(0, str(BENCH))

CORE_CRATES = ("wtime-runtime", "wtime-types", "wtime-decode", "wtime-validate")
MODE_FEATURES = {
    "decoded": [],
    "compact": ["compact-executor"],
}


def sha(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def write_new(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("x", encoding="utf-8") as stream:
        json.dump(value, stream, indent=2, sort_keys=True, allow_nan=False)
        stream.write("\n")


def _source_identity(root: Path) -> dict[str, str]:
    # Keep the complete transitive benchmark/runtime map, including analysis-independent
    # harness inputs. candidate_gate checks the runtime subset against paired metadata.
    import compare
    return compare.sources(include_memory=True, include_simd=True,
                           include_representative=True, include_integration=True)


def _core_source_map(root: Path) -> dict[str, str]:
    result: dict[str, str] = {}
    for crate in CORE_CRATES:
        source_dir = root / "crates" / crate / "src"
        for path in source_dir.rglob("*.rs"):
            if path.is_file():
                result[path.relative_to(root).as_posix()] = sha(path)
    if not all(any(name.startswith(f"crates/{crate}/src/") for name in result)
               for crate in CORE_CRATES):
        raise ValueError("production source identity omits one or more core crates")
    return dict(sorted(result.items()))


def _run_logged(argv: list[str], cwd: Path, directory: Path, label: str,
                runner: Callable = subprocess.run) -> tuple[int, Path, Path]:
    write_new(directory / f"{label}.command.json", {"argv": argv, "cwd": str(cwd)})
    completed = runner(argv, cwd=cwd, text=True, capture_output=True, check=False)
    stdout_path, stderr_path = directory / f"{label}.stdout", directory / f"{label}.stderr"
    with stdout_path.open("xb") as stream:
        stream.write((completed.stdout or "").encode())
    with stderr_path.open("xb") as stream:
        stream.write((completed.stderr or "").encode())
    write_new(directory / f"{label}.status.json", {"exit_code": completed.returncode})
    return completed.returncode, stdout_path, stderr_path


def build_production(output: Path, mode: str, *, root: Path = ROOT,
                     runner: Callable = subprocess.run) -> dict:
    """Build `mode`, probe its real protocol, and emit the gate record schema."""
    if mode not in MODE_FEATURES:
        raise ValueError(f"unsupported production mode: {mode}")
    root = root.resolve(strict=True)
    output = output.resolve()
    directory = output
    directory.mkdir(parents=True, exist_ok=False)

    helper_bytes = Path(__file__).resolve().read_bytes()
    helper_path = directory / "helper.executed.py"
    with helper_path.open("xb") as stream:
        stream.write(helper_bytes)

    before_sources = _source_identity(root)
    before_core = _core_source_map(root)
    features = MODE_FEATURES[mode]
    command = ["cargo", "build", "--locked", "--release", "--no-default-features",
               "-j", "4", "--manifest-path", "tools/wtime-bench/Cargo.toml"]
    if features:
        command.extend(["--features", ",".join(features)])
    command.extend(["--bin", "wtime-bench"])
    build_exit, stdout, stderr = _run_logged(command, root, directory, "build", runner)
    if build_exit:
        raise RuntimeError(f"production build failed with {build_exit}; see {directory}")

    target_dir = Path(os.environ.get("CARGO_TARGET_DIR", str(root / "tools/wtime-bench/target")))
    if not target_dir.is_absolute():
        target_dir = root / target_dir
    built_binary = target_dir / "release/wtime-bench"
    if not built_binary.is_file():
        raise FileNotFoundError(f"Cargo succeeded without producing {built_binary}")
    binary_dir = directory / "bin"
    binary_dir.mkdir()
    binary = binary_dir / "wtime-bench"
    shutil.copy2(built_binary, binary)

    protocol_command = [str(binary), "--protocol"]
    protocol_exit, protocol_stdout, protocol_stderr = _run_logged(
        protocol_command, root, directory, "protocol", runner)
    if protocol_exit:
        raise RuntimeError(f"production protocol probe failed with {protocol_exit}; see {directory}")
    protocol_bytes = protocol_stdout.read_bytes()
    protocol = json.loads(protocol_bytes)
    if (protocol.get("schema_version") != 2 or protocol.get("engine") != "wtime"
            or protocol.get("instrumented") is not False
            or mode not in protocol.get("modes", [])):
        raise ValueError(f"production protocol does not prove uninstrumented {mode} support")

    after_sources = _source_identity(root)
    after_core = _core_source_map(root)
    if before_sources != after_sources or before_core != after_core:
        raise ValueError("candidate sources changed during production build/protocol probe")

    # gates._validate_candidate_build_record consumes these names and hashes.
    with (directory / "stdout.log").open("xb") as stream:
        stream.write(stdout.read_bytes())
    with (directory / "stderr.log").open("xb") as stream:
        stream.write(stderr.read_bytes())
    # Preserve the actual protocol bytes at the path used by the shared gate.
    with (directory / "protocol.stdout.json").open("xb") as stream:
        stream.write(protocol_bytes)
    record = {
        "schema": "wtime-production-command-v1",
        "phase": "build",
        "exit_code": 0,
        "source_stable": True,
        "cwd": str(root),
        "command": command,
        "binary_path": str(binary),
        "binary_sha256": sha(binary),
        "runtime_source_map": {name: digest for name, digest in before_core.items()
                               if name.startswith("crates/wtime-runtime/src/")},
        "core_source_map": before_core,
        "source_identities": before_sources,
        "feature_flags": features,
        "protocol_command": protocol_command,
        "protocol_exit": 0,
        "protocol_stdout": str(protocol_stdout),
        "protocol_stderr": str(protocol_stderr),
        "protocol_sha256": sha(directory / "protocol.stdout.json"),
        "helper_sha256": sha(helper_path),
        "stdout_sha256": sha(directory / "stdout.log"),
        "stderr_sha256": sha(directory / "stderr.log"),
    }
    write_new(directory / "build-record.json", record)
    return record


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mode", choices=sorted(MODE_FEATURES), required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    record = build_production(args.output, args.mode)
    print(json.dumps({"status": "passed", "mode": args.mode,
                      "binary_sha256": record["binary_sha256"]}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
