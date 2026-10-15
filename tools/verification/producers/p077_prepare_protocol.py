"""Prepare an isolated source-bound Cargo crate for P077 protocol tests."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[3]
BENCH = ROOT / "tools/wtime-bench/src"
PROTOCOL = BENCH / "protocol.rs"
SHARED_MODEL = ROOT / "tools/wtime-bench/shared_bulk_workloads.py"
QUEUE_MODEL = ROOT / "tools/wtime-bench/wait_queue_workloads.py"
SHARED_WAT = ROOT / "tools/wtime-bench/fixtures/threads/shared_bulk_copy.wat"
SHARED_WASM = ROOT / "tools/wtime-bench/fixtures/threads/shared_bulk_copy.wasm"
QUEUE_WAT = ROOT / "tools/wtime-bench/fixtures/threads/wait_queue_cohort.wat"
QUEUE_WASM = ROOT / "tools/wtime-bench/fixtures/threads/wait_queue_cohort.wasm"
COMPILE_TIME_FIXTURES = (
    ROOT / "tools/wtime-bench/fixtures/continuations/depth_0.wasm",
    ROOT / "tools/wtime-bench/fixtures/p135/counter.wasm",
    ROOT / "tools/wtime-bench/fixtures/p135/late_bad_result.wasm",
    ROOT / "tools/wtime-bench/fixtures/threads/shared_bulk_copy.wasm",
    ROOT / "crates/wtime-runtime/tests/fixtures/performance/p138/uniform66.wasm",
)


def sources() -> tuple[Path, ...]:
    # protocol.rs imports every production driver module. Hash the complete
    # source set, including modules outside the P077 route, because Cargo
    # compiles the same module graph in this isolated test crate.
    return tuple(sorted(BENCH.rglob("*.rs"))) + (
        SHARED_MODEL, QUEUE_MODEL, SHARED_WAT, SHARED_WASM, QUEUE_WAT, QUEUE_WASM,
        *COMPILE_TIME_FIXTURES,
    )


def sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def write(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("x", encoding="utf-8") as stream:
        json.dump(value, stream, indent=2, sort_keys=True, ensure_ascii=False, allow_nan=False)
        stream.write("\n")


def prepare(output: Path) -> dict:
    output = output.resolve()
    verification_root = (ROOT / "target/verification").resolve()
    if not output.is_relative_to(verification_root) or output.exists():
        raise ValueError("P077 protocol output must be a fresh target/verification child")
    bound_sources = sources()
    for source in bound_sources:
        source.resolve(strict=True).relative_to(ROOT)
    before = {str(path.relative_to(ROOT)): sha(path) for path in bound_sources}
    output.mkdir(parents=True, exist_ok=False)
    (output / "src").mkdir()
    (output / "src/main.rs").write_text(
        '#![forbid(unsafe_code)]\n#[path = "' + str(PROTOCOL) + '"]\nmod protocol;\nfn main() {}\n',
        encoding="utf-8",
    )
    (output / "Cargo.toml").write_text(
        '''[package]
name = "wtime-p077-protocol-tests"
version = "0.1.0"
edition = "2024"
[workspace]
[features]
default = []
runtime-only = []
legacy-api = []
performance-counters = ["wtime-runtime/performance-counters"]
execution-reference = ["wtime-runtime/execution-reference"]
compact-executor = ["wtime-runtime/compact-executor"]
register-executor = ["wtime-runtime/register-executor"]
experimental-executors = ["compact-executor", "register-executor"]
[dependencies]
wtime-runtime = { path = "'''+str(ROOT / "crates/wtime-runtime") + '''" }
wtime-types = { path = "'''+str(ROOT / "crates/wtime-types") + '''" }
''',
        encoding="utf-8",
    )
    helper_copy = output / "prepare.executed.py"
    shutil.copyfile(__file__, helper_copy)
    argv = ["cargo", "generate-lockfile", "--offline", "--manifest-path", str(output / "Cargo.toml")]
    write(output / "lock.command.json", {"argv": argv, "cwd": str(ROOT)})
    with (output / "lock.stdout").open("x") as stdout, (output / "lock.stderr").open("x") as stderr:
        result = subprocess.run(argv, cwd=ROOT, stdout=stdout, stderr=stderr, check=False)
    write(output / "lock.result.json", {"exit_code": result.returncode})
    if result.returncode:
        raise RuntimeError(f"P077 offline lockfile generation failed: {result.returncode}")
    after = {str(path.relative_to(ROOT)): sha(path) for path in bound_sources}
    if before != after:
        raise RuntimeError("P077 protocol source changed during preparation")
    report = {
        "schema": "wtime-p077-protocol-preparation-v1",
        "status": "passed",
        "source_stable": True,
        "sources": before,
        "manifest": {"path": str(output / "Cargo.toml"), "sha256": sha(output / "Cargo.toml")},
        "lockfile": {"path": str(output / "Cargo.lock"), "sha256": sha(output / "Cargo.lock")},
        "helper": {"path": str(helper_copy), "sha256": sha(helper_copy)},
        "feature": "runtime-only",
        "instrumented": False,
        "timing_evidence": False,
    }
    write(output / "prepared.json", report)
    return report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    report = prepare(args.output)
    print(json.dumps({"status": report["status"], "source_stable": report["source_stable"]}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
