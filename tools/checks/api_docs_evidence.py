#!/usr/bin/env python3
"""Run documented embedding and CLI examples and emit docs/verification/api-docs.json."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

import hardening

ROOT = hardening.ROOT
DOCUMENTS = (
    'docs/embedding.md',
    'examples/README.md',
    'README.md',
    'examples/memory64-table64.wat',
    'examples/memory64-table64.wasm',
    'docs/architecture/api.md',
    'docs/architecture/host-suspension.md',
    'docs/architecture/scope.md',
)


def _scrub(text: str, root: Path = ROOT) -> str:
    resolved = str(root.resolve())
    raw = str(root)
    for prefix in dict.fromkeys((resolved, raw)):
        text = text.replace(prefix + '/', '').replace(prefix, '.')
    return text


def _display_path(path: Path, root: Path = ROOT) -> str:
    try:
        return path.resolve().relative_to(root.resolve()).as_posix()
    except ValueError:
        return str(path)


def collect(cli: Path, output: Path, *, target_dir: Path | None = None) -> bool:
    before = hardening.source_hash()
    env = dict(os.environ)
    if target_dir is not None:
        env['CARGO_TARGET_DIR'] = str(target_dir)
    cli_display = _display_path(cli)
    commands = [
        (['cargo', 'run', '--locked', '-p', 'wtime-runtime', '--example', 'embedding'], '20 + 22 = 42\n'),
        (['cargo', 'run', '--locked', '-p', 'wtime-runtime', '--example', 'suspension'], 'suspended with 7; resumed with 9\n'),
        ([cli_display, '--profile', 'all-phase3-core', 'run', 'examples/memory64-table64.wasm', 'memory_roundtrip'], 'I32(42)\n'),
        ([cli_display, '--profile', 'all-phase3-core', 'run', 'examples/memory64-table64.wasm', 'table_size'], 'I64(1)\n'),
    ]
    results = []
    all_passed = True
    for index, (display_cmd, expected_stdout) in enumerate(commands):
        actual_cmd = [str(cli.resolve()), *display_cmd[1:]] if index >= 2 else display_cmd
        proc = subprocess.run(actual_cmd, cwd=ROOT, env=env, text=True, capture_output=True)
        passed = proc.returncode == 0 and proc.stdout == expected_stdout
        all_passed = all_passed and passed
        results.append({
            'command': display_cmd,
            'exit_code': proc.returncode,
            'passed': passed,
            'stdout': _scrub(proc.stdout),
            'stderr': _scrub(proc.stderr),
        })
    documents = {
        rel: hashlib.sha256((ROOT / rel).read_bytes()).hexdigest()
        for rel in DOCUMENTS
    }
    stable = hardening.source_hash() == before
    record = {
        'source_sha256': before,
        'source_stable': stable,
        'executed': len(results),
        'passed': all_passed and stable,
        'results': results,
        'documents': documents,
    }
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(record, indent=2) + '\n')
    return record['passed']


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cli', type=Path, default=ROOT / 'target/debug/wtime-cli')
    parser.add_argument('--target-dir', type=Path, default=None)
    parser.add_argument('--report', type=Path, default=ROOT / 'docs/verification/api-docs.json')
    args = parser.parse_args()
    return 0 if collect(args.cli, args.report, target_dir=args.target_dir) else 1


if __name__ == '__main__':
    raise SystemExit(main())
