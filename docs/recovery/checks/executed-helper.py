"""Archive this recovery check and its inputs before executing any checks."""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time

sys.dont_write_bytecode = True
ROOT = Path('/workspace/wtime')
OUT = Path(__file__).resolve().parent


def sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def write(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open('x', encoding='utf-8') as stream:
        json.dump(value, stream, indent=2, sort_keys=True)
        stream.write('\n')


def main() -> int:
    archived = OUT / 'executed-helper.py'
    shutil.copy2(__file__, archived)
    write(OUT / 'invocation.json', {
        'argv': sys.argv, 'helper_sha256': sha(archived),
        'purpose': 'recovery verification; not performance or task-completion evidence',
    })
    selected = [
        'target/performance/P050/shared-finalization/attempt-0003/completion.json',
        'target/verification/runs/P050/analyze/attempt-0002/statistics.json',
        'target/performance/P050/experimental-retention/attempt-0001/receipt.json',
        'target/performance/P128/integration/attempt-0001/receipt.json',
        'target/performance/P128/integration/attempt-0001/before-core/receipt.json',
        'target/performance/P128/source-event-proof/attempt-0002/report.json',
    ]
    records = []
    for raw in selected:
        source = ROOT / raw
        dest = ROOT / 'docs/recovery/records' / raw
        if dest.exists():
            raise RuntimeError(f'refusing to replace {dest}')
        dest.parent.mkdir(parents=True, exist_ok=True)
        digest = sha(source)
        shutil.copy2(source, dest)
        if sha(dest) != digest or sha(source) != digest:
            raise RuntimeError(f'copy mismatch: {raw}')
        records.append({'source': raw, 'copy': dest.relative_to(ROOT).as_posix(),
                        'sha256': digest, 'size_bytes': dest.stat().st_size})
    write(ROOT / 'docs/recovery/record-manifest.json', {
        'schema_version': 1, 'records': records,
        'copied_original_bytes': True,
        'referenced_binaries_and_datasets_are_not_included': True,
    })
    inputs = {}
    for base in ['crates', 'tools/verification', 'tools/wtime-bench/tests']:
        for path in sorted((ROOT / base).rglob('*')):
            if path.is_file() and 'target' not in path.relative_to(ROOT / base).parts and path.suffix in {'.rs', '.py', '.wasm', '.wat', '.toml', '.lock'}:
                inputs[path.relative_to(ROOT).as_posix()] = sha(path)
    for raw in ['Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', 'tools/wtime-bench/scalar_p128_workloads.py', 'tools/wtime-bench/p128_scalar_scaling.py', 'tools/wtime-bench/execution_identity.py']:
        inputs[raw] = sha(ROOT / raw)
    write(OUT / 'source-inputs.json', inputs)
    env = dict(os.environ)
    env.update(CARGO_HOME='/workspace/.rust/cargo', RUSTUP_HOME='/workspace/.rust/rustup', CARGO_TARGET_DIR=str(ROOT / 'tools/wtime-bench/target'), PYTHONDONTWRITEBYTECODE='1')
    env['PATH'] = '/workspace/.rust/rustup/toolchains/1.99.0-x86_64-unknown-linux-gnu/bin:/workspace/.rust/cargo/bin:' + env.get('PATH', '')
    commands = [
        ('core-all-features', ['cargo', 'test', '--locked', '--all-features']),
        ('p128-tooling-and-identity', [sys.executable, '-B', '-m', 'unittest', '-v', 'tools.wtime-bench.tests.test_p128_scalar_workloads', 'tools.verification.tests.test_p128_scalar', 'tools.wtime-bench.tests.test_fast_execution_identity']),
        ('dependency-budget', [sys.executable, '-B', 'tools/checks/dependency_budget.py']),
    ]
    results = []
    for name, argv in commands:
        write(OUT / f'{name}.argv.json', {'argv': argv, 'cwd': str(ROOT), 'compiler': '/workspace/.rust/rustup/toolchains/1.99.0-x86_64-unknown-linux-gnu/bin', 'cargo_target_dir': env['CARGO_TARGET_DIR']})
        start = time.monotonic()
        with (OUT / f'{name}.stdout').open('xb') as stdout, (OUT / f'{name}.stderr').open('xb') as stderr:
            proc = subprocess.run(argv, cwd=ROOT, env=env, stdout=stdout, stderr=stderr)
        result = {'name': name, 'returncode': proc.returncode, 'elapsed_seconds': time.monotonic() - start,
                  'stdout_sha256': sha(OUT / f'{name}.stdout'), 'stderr_sha256': sha(OUT / f'{name}.stderr')}
        write(OUT / f'{name}.result.json', result)
        results.append(result)
        print(json.dumps(result), flush=True)
    changed = [raw for raw, digest in inputs.items() if not (ROOT / raw).is_file() or sha(ROOT / raw) != digest]
    write(OUT / 'result.json', {'schema_version': 1, 'commands': results, 'source_inputs_unchanged': not changed,
          'changed_inputs': changed, 'passed': all(row['returncode'] == 0 for row in results) and not changed})
    return 0 if all(row['returncode'] == 0 for row in results) and not changed else 1


if __name__ == '__main__':
    raise SystemExit(main())
