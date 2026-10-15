"""Prepare a small source-bound Cargo crate for P072 protocol unit tests."""
from __future__ import annotations
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[3]
PROTOCOL = ROOT / 'tools/wtime-bench/src/protocol.rs'
DRIVER = ROOT / 'tools/wtime-bench/src/continuation_live_driver.rs'
FIXTURE = ROOT / 'tools/wtime-bench/fixtures/continuations/depth_0.wasm'
MODEL = ROOT / 'tools/wtime-bench/continuation_live_workloads.py'
SOURCES = (PROTOCOL, DRIVER, FIXTURE, MODEL)

def sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()

def write(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open('x', encoding='utf-8') as stream:
        json.dump(value, stream, indent=2, sort_keys=True, ensure_ascii=False, allow_nan=False)
        stream.write('\n')

def prepare(output: Path) -> dict:
    output = output.resolve()
    verification_root = (ROOT / 'target/verification').resolve()
    if not output.is_relative_to(verification_root) or output.exists():
        raise ValueError('P072 protocol output must be a fresh target/verification child')
    for source in SOURCES:
        source.resolve(strict=True).relative_to(ROOT)
    before = {str(p.relative_to(ROOT)): sha(p) for p in SOURCES}
    output.mkdir(parents=True, exist_ok=False)
    (output / 'src').mkdir()
    (output / 'src/main.rs').write_text('''#![forbid(unsafe_code)]
#[path = "'''+str(PROTOCOL)+'''"]
mod protocol;
fn main() {}
''', encoding='utf-8')
    (output / 'Cargo.toml').write_text('''[package]
name = "wtime-p072-protocol-tests"
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
wtime-runtime = { path = "'''+str(ROOT / 'crates/wtime-runtime')+'''" }
wtime-types = { path = "'''+str(ROOT / 'crates/wtime-types')+'''" }
''', encoding='utf-8')
    helper_copy = output / 'prepare.executed.py'
    shutil.copyfile(__file__, helper_copy)
    prep_argv = ['cargo', 'generate-lockfile', '--offline', '--manifest-path', str(output / 'Cargo.toml')]
    write(output / 'lock.command.json', {'argv': prep_argv, 'cwd': str(ROOT)})
    with (output / 'lock.stdout').open('x') as stdout, (output / 'lock.stderr').open('x') as stderr:
        result = subprocess.run(prep_argv, cwd=ROOT, stdout=stdout, stderr=stderr, check=False)
    write(output / 'lock.result.json', {'exit_code': result.returncode})
    if result.returncode:
        raise RuntimeError(f'P072 offline lockfile generation failed: {result.returncode}')
    after = {str(p.relative_to(ROOT)): sha(p) for p in SOURCES}
    if before != after:
        raise RuntimeError('P072 protocol source changed during preparation')
    report = {'schema': 'wtime-p072-protocol-preparation-v1', 'status': 'passed',
              'source_stable': True, 'sources': before,
              'manifest': {'path': str(output / 'Cargo.toml'), 'sha256': sha(output / 'Cargo.toml')},
              'lockfile': {'path': str(output / 'Cargo.lock'), 'sha256': sha(output / 'Cargo.lock')},
              'helper': {'path': str(helper_copy), 'sha256': sha(helper_copy)},
              'feature': 'runtime-only', 'instrumented': False, 'timing_evidence': False}
    write(output / 'prepared.json', report)
    return report

def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    report = prepare(args.output)
    print(json.dumps({'status': report['status'], 'source_stable': report['source_stable']}))
    return 0

if __name__ == '__main__':
    raise SystemExit(main())
