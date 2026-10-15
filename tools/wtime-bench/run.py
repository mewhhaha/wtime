#!/usr/bin/env python3
"""Generate original MVP workloads, run paired benchmarks, preserve raw samples."""
import argparse
import csv
import datetime
import hashlib
import io
import json
import math
import os
from pathlib import Path
import platform
import statistics
import subprocess

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
FIXTURES = HERE / 'fixtures'
WAT = {
'call_overhead': '(module (func (export "run") (param i32) (result i32) local.get 0 i32.const 1 i32.add))',
'integer_loop': '''(module (func (export "run") (param $n i32) (result i32)
 (local $i i32) (local $acc i32) (local.set $acc (i32.const 1))
 (block $done (loop $loop
  (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
  (local.set $acc (i32.add (i32.mul (local.get $acc) (i32.const 1664525)) (i32.const 1013904223)))
  (local.set $i (i32.add (local.get $i) (i32.const 1))) (br $loop))) (local.get $acc)))''',
'memory_fill_scan': '''(module (memory 1 1)
 (func (export "run") (param $n i32) (result i32) (local $i i32) (local $acc i32)
 (block $done (loop $fill
  (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
  (i32.store (i32.mul (local.get $i) (i32.const 4)) (i32.mul (local.get $i) (i32.const 3)))
  (local.set $i (i32.add (local.get $i) (i32.const 1))) (br $fill)))
 (local.set $i (i32.const 0))
 (block $done (loop $scan
  (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
  (local.set $acc (i32.add (local.get $acc) (i32.load (i32.mul (local.get $i) (i32.const 4)))))
  (local.set $i (i32.add (local.get $i) (i32.const 1))) (br $scan))) (local.get $acc)))''',
'branch_calls': '''(module
 (func $step (param $x i32) (result i32)
  (if (result i32) (i32.eqz (i32.and (local.get $x) (i32.const 1)))
   (then (i32.add (i32.mul (local.get $x) (i32.const 3)) (i32.const 1)))
   (else (i32.xor (local.get $x) (i32.const 21845)))))
 (func (export "run") (param $n i32) (result i32) (local $i i32) (local $acc i32)
 (local.set $acc (i32.const 1))
 (block $done (loop $loop
  (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
  (local.set $acc (call $step (i32.xor (local.get $acc) (local.get $i))))
  (local.set $i (i32.add (local.get $i) (i32.const 1))) (br $loop))) (local.get $acc)))''',
}

def signed(value):
    value &= 0xffffffff
    return value - 0x100000000 if value & 0x80000000 else value

def oracle(case, n):
    if case == 'call_overhead': return signed(n + 1)
    if case == 'memory_fill_scan': return signed(3 * n * (n - 1) // 2)
    value = 1
    for i in range(n):
        if case == 'integer_loop': value = (value * 1664525 + 1013904223) & 0xffffffff
        else:
            value ^= i
            value = ((value * 3 + 1) if value & 1 == 0 else (value ^ 21845)) & 0xffffffff
    return signed(value)

def sha(path): return hashlib.sha256(Path(path).read_bytes()).hexdigest()

def percentile(values, fraction):
    values = sorted(values)
    index = fraction * (len(values) - 1)
    low = int(index); high = min(low + 1, len(values) - 1)
    return values[low] + (values[high] - values[low]) * (index - low)

def build_fixture_sources():
    fixtures = dict(WAT)
    # 1,000 separately compiled functions, each 16 multiply/xor pairs.
    functions = []
    for i in range(1000):
        export = '(export "run") ' if i == 0 else ''
        operations = ' '.join(f'i32.const {1664525 + j + i} i32.mul i32.const {1013904223 + j} i32.xor' for j in range(16))
        functions.append(f'(func {export}(param i32) (result i32) local.get 0 {operations})')
    fixtures['prepare_large'] = '(module\n' + '\n'.join(functions) + '\n)'
    return fixtures

INPUTS = {'call_overhead': 100, 'integer_loop': 10000, 'memory_fill_scan': 4096,
          'branch_calls': 5000, 'prepare_large': 0}
STAGES = ('prepare', 'instantiate', 'first_call', 'warm_call', 'load_and_call')
ENGINES = ('wtime', 'wasmtime')

def tier_arguments(mode, register_entry_threshold=None):
    if mode not in ('decoded', 'compact', 'register'):
        raise ValueError('mode must be decoded, compact, or register')
    threshold = register_entry_threshold
    if threshold is not None:
        if mode != 'register':
            raise ValueError('register entry threshold requires register mode')
        if isinstance(threshold, bool):
            raise ValueError('register entry threshold must be a positive u64')
        try:
            threshold = int(threshold)
        except (TypeError, ValueError) as exc:
            raise ValueError('register entry threshold must be a positive u64') from exc
        if threshold < 1 or threshold > (1 << 64) - 1:
            raise ValueError('register entry threshold must be a positive u64')
        if str(threshold) != str(register_entry_threshold).strip():
            raise ValueError('register entry threshold must be a positive integer')
        return [mode, str(threshold)]
    return [mode]

def source_map(executable):
    paths = [ROOT/'Cargo.toml', ROOT/'Cargo.lock', ROOT/'rust-toolchain.toml',
             ROOT/'.cargo/config.toml', ROOT/'.cargo/config',
             HERE/'.cargo/config.toml', HERE/'.cargo/config',
             HERE/'Cargo.toml', HERE/'Cargo.lock', HERE/'run.py']
    paths += list((HERE/'src').rglob('*.rs'))
    paths += list((ROOT/'crates').rglob('*.rs'))
    paths += list((ROOT/'crates').rglob('Cargo.toml'))
    return {str(path.resolve().relative_to(ROOT)): sha(path)
            for path in sorted(set(paths)) if path.is_file()}

def write_fixtures(fixtures, fixture_dir, encoder):
    root = ROOT.resolve()
    directory = Path(fixture_dir).resolve()
    if not directory.is_relative_to(root):
        raise ValueError('fixture directory must be contained in the repository')
    if directory == FIXTURES.resolve() or FIXTURES.resolve() in directory.parents:
        raise ValueError('fixture directory cannot be inside the shared fixtures tree')
    if directory.exists():
        raise ValueError('fixture directory must be new and empty')
    directory.mkdir(parents=True)
    encoder = Path(encoder).resolve(strict=True)
    records = {}
    for case, source in fixtures.items():
        wat = directory/(case+'.wat')
        wasm = directory/(case+'.wasm')
        wat.write_text(source+'\n')
        subprocess.run([str(encoder), 'encode', str(wat), '-o', str(wasm)],
                       check=True, cwd=ROOT)
        expected = 0 if case == 'prepare_large' else oracle(case, INPUTS[case])
        records[case] = {'input': INPUTS[case], 'expected_result': expected,
                         'wasm_bytes': wasm.stat().st_size,
                         'wasm_sha256': sha(wasm), 'wat_sha256': sha(wat),
                         'wat_path': str(wat), 'wasm_path': str(wasm)}
    return directory, records, encoder

def verify_fixture_identity(directory, records):
    directory = Path(directory).resolve(strict=True)
    expected = set()
    for record in records.values():
        for path_key, hash_key in (('wat_path', 'wat_sha256'), ('wasm_path', 'wasm_sha256')):
            path = Path(record[path_key]).resolve(strict=True)
            if not path.is_relative_to(directory):
                raise RuntimeError(f'fixture path escaped its directory: {path}')
            expected.add(path)
            if sha(path) != record[hash_key]:
                raise RuntimeError(f'fixture changed after preparation: {path}')
        wasm = Path(record['wasm_path']).resolve(strict=True)
        if wasm.stat().st_size != record['wasm_bytes']:
            raise RuntimeError(f'fixture size changed after preparation: {wasm}')
    actual = {path.resolve() for path in directory.rglob('*') if path.is_file()}
    if actual != expected:
        raise RuntimeError('fixture directory membership changed after preparation')

def parse_timings(output, workload, samples):
    expected = {(workload, stage, engine, sample)
                for stage in (('prepare',) if workload == 'prepare_large' else STAGES)
                for engine in ENGINES for sample in range(samples)}
    rows = []
    seen = set()
    for row in csv.reader(io.StringIO(output)):
        if len(row) != 6:
            raise ValueError('timing row must have six CSV fields')
        name, stage, engine, sample_text, batch_text, ns_text = row
        try:
            sample, batch, ns = int(sample_text), int(batch_text), float(ns_text)
        except ValueError as exc:
            raise ValueError('timing row has malformed numeric fields') from exc
        key = (name, stage, engine, sample)
        if key not in expected:
            raise ValueError(f'unexpected timing key: {key!r}')
        if key in seen:
            raise ValueError(f'duplicate timing key: {key!r}')
        if batch < 1 or not math.isfinite(ns) or ns <= 0:
            raise ValueError('timing batch and duration must be positive and finite')
        if stage != 'warm_call' and batch != 1:
            raise ValueError('cold-stage timing batch must equal one')
        seen.add(key)
        rows.append({'workload': name, 'stage': stage, 'engine': engine,
                     'sample': sample, 'batch': batch, 'ns_per_operation': ns})
    missing = expected - seen
    if missing:
        raise ValueError(f'incomplete timing output: missing {len(missing)} rows')
    return rows

def write_exclusive(path, content):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open('x', encoding='utf-8') as stream:
        stream.write(content)

def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--samples', type=int, default=21)
    p.add_argument('--seed', type=int, default=20261011)
    p.add_argument('--cpu', type=int)
    p.add_argument('--output', type=Path, default=ROOT/'docs/benchmarks/wasmtime-comparison.json')
    p.add_argument('--encoder', type=Path, default=ROOT/'target/debug/wtime-conformance')
    p.add_argument('--executable', type=Path, default=HERE/'target/release/wtime-bench')
    p.add_argument('--mode', choices=('decoded', 'compact', 'register'), default='decoded')
    p.add_argument('--register-entry-threshold', type=int)
    p.add_argument('--fixture-dir', type=Path)
    p.add_argument('--fixture-only', action='store_true',
                   help='encode fixtures and write a provenance manifest without timing')
    args = p.parse_args()
    try:
        mode_args = tier_arguments(args.mode, args.register_entry_threshold)
    except ValueError as exc:
        p.error(str(exc))
    if args.samples < 5 or args.seed <= 0: p.error('need at least 5 samples and a positive seed')
    if args.output.exists(): p.error('--output must be a new path')
    if not args.fixture_only:
        try:
            allowed = sorted(os.sched_getaffinity(0))
        except AttributeError:
            p.error('CPU affinity is unavailable on this platform')
        cpu = args.cpu if args.cpu is not None else allowed[0]
        if cpu not in allowed: p.error('requested CPU is outside available affinity')
    executable = args.executable.resolve(strict=not args.fixture_only)
    try:
        encoder = args.encoder.resolve(strict=True)
    except OSError as exc:
        p.error(f'cannot resolve encoder: {exc}')
    before_sources = source_map(executable) if executable.exists() else source_map(None)
    before_encoder = sha(encoder)
    before_binary = sha(executable) if executable.exists() else None
    fixtures = build_fixture_sources()
    fixture_dir = args.fixture_dir or args.output.parent/(args.output.stem+'-fixtures')
    try:
        fixture_dir, workload_info, encoder = write_fixtures(fixtures, fixture_dir, args.encoder)
    except (OSError, ValueError, subprocess.CalledProcessError) as exc:
        p.error(str(exc))
    verify_fixture_identity(fixture_dir, workload_info)
    if source_map(executable) != before_sources or sha(encoder) != before_encoder:
        raise RuntimeError('implementation sources or encoder changed during fixture preparation')
    if args.fixture_only:
        report = {'schema_version': 1, 'passed': True, 'fixture_only': True,
                  'mode': args.mode, 'register_entry_threshold': args.register_entry_threshold,
                  'fixture_directory': str(fixture_dir), 'encoder': str(encoder),
                  'encoder_sha256': before_encoder, 'implementation_sources': before_sources,
                  'benchmark_binary': {'path': str(executable), 'sha256': before_binary},
                  'workloads': workload_info}
        if sha(encoder) != before_encoder:
            raise RuntimeError('encoder changed during fixture preparation')
        write_exclusive(args.output, json.dumps(report, indent=2)+'\n')
        print(f"prepared {len(workload_info)} fixtures: {args.output}")
        return
    if before_binary is None:
        p.error(f'benchmark executable does not exist: {executable}')
    raw = []
    for case, record in workload_info.items():
        wasm = Path(record['wasm_path'])
        command = [str(executable), str(wasm), case, str(record['input']),
                   str(record['expected_result']), str(args.samples), str(args.seed), *mode_args]
        result = subprocess.run(command, cwd=ROOT, text=True, capture_output=True, timeout=180,
                                preexec_fn=lambda: os.sched_setaffinity(0, {cpu}))
        if result.returncode: raise RuntimeError(f'{case}: {result.stderr}')
        rows = parse_timings(result.stdout, case, args.samples)
        raw.extend(rows)
        record['command'] = command
        print(f'{case}: {len(rows)} timings; correctness checks passed', flush=True)
    summaries = []
    for case in fixtures:
        for stage in ['prepare', 'instantiate', 'first_call', 'warm_call', 'load_and_call']:
            group = [r for r in raw if r['workload'] == case and r['stage'] == stage]
            if not group: continue
            item = dict(workload=case, stage=stage)
            for engine in ['wtime', 'wasmtime']:
                values = [r['ns_per_operation'] for r in group if r['engine'] == engine]
                item[engine] = dict(median_ns=statistics.median(values), p25_ns=percentile(values, .25), p75_ns=percentile(values, .75), samples=len(values))
            item['wtime_over_wasmtime'] = item['wtime']['median_ns'] / item['wasmtime']['median_ns']
            summaries.append(item)
    cpu_model = next((line.split(':', 1)[1].strip() for line in Path('/proc/cpuinfo').read_text().splitlines() if line.startswith('model name')), 'unknown')
    after_sources = source_map(executable)
    if (before_sources != after_sources or before_binary != sha(executable)
            or before_encoder != sha(encoder)):
        raise RuntimeError('implementation, executable, or encoder changed during benchmark')
    verify_fixture_identity(fixture_dir, workload_info)
    report = dict(passed=True, source_stable=True, recorded_at_utc=datetime.datetime.now(datetime.timezone.utc).isoformat(), wasm_correctness_checked=True, wasmtime_version='49.0.2', samples=args.samples, seed=args.seed,
                  hardware=dict(cpu_model=cpu_model, os=platform.platform(), cpu_affinity_available=allowed, selected_cpu=cpu,
                                cpu_quota=Path('/sys/fs/cgroup/cpu.max').read_text().strip()),
                  rustc=subprocess.check_output(['rustc', '-Vv'], text=True),
                  settings=dict(profile='release', wasmtime_features=['std', 'runtime', 'cranelift'], opcode_coverage=False, cranelift_optimization='speed', fuel=False, epoch_interruption=False,
                                wasmtime_cache=False, wasmtime_parallel_compilation=False, wtime_features='mvp',
                                wtime_mode=args.mode, mode=args.mode,
                                register_entry_threshold=args.register_entry_threshold,
                                call_api='dynamic values; reusable argument arrays and Wasmtime result buffer; wtime returns owned results',
                                preparation='fresh initialized engine per sample; file IO/engine initialization/module drop excluded',
                                instantiation='compiled module reused; fresh store creation/export lookup/drop excluded; no start functions',
                                first_call=('single invocation on fresh instance using a reused compiled module; excludes preparation/instantiation; Register adaptive counters and compilation state are shared across instances'
                                            if args.mode == 'register' else
                                            'single invocation on fresh instance, excludes preparation/instantiation'),
                                load_and_call='fresh initialized engine; includes compilation/store/instantiation/export lookup/first invocation; excludes file IO and engine initialization',
                                warm_call=('five warmup invocations; Register adaptive counters and compilation state include warmup; calibrated batches targeting 20 ms, deterministic alternating A/B order'
                                           if args.mode == 'register' else
                                           'five warmup invocations, calibrated batches targeting 20 ms, deterministic alternating A/B order')),
                  benchmark_binary={'path': str(executable), 'sha256': before_binary},
                  encoder={'path': str(encoder), 'sha256': before_encoder},
                  fixture_directory=str(fixture_dir),
                  implementation_sources=after_sources,
                  workloads=workload_info, summary=summaries, raw_samples=raw,
                  limitations=['Synthetic MVP workloads, not a production application suite.', 'Cloud CPU affinity does not guarantee an otherwise idle physical core.', 'Preparation compares interpreter validation/lowering against optimized JIT machine-code generation.', 'Reported invocation includes dynamic embedding API overhead.', 'Register adaptive first-call observations reuse a compiled module, and warm-call observations include five adaptive training calls.'])
    write_exclusive(args.output, json.dumps(report, indent=2)+'\n')
    for row in summaries:
        print(f"{row['workload']:18} {row['stage']:12} wtime={row['wtime']['median_ns']/1000:10.3f} us wasmtime={row['wasmtime']['median_ns']/1000:10.3f} us ratio={row['wtime_over_wasmtime']:.2f}")

if __name__ == '__main__': main()
