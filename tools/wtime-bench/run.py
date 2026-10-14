#!/usr/bin/env python3
"""Generate original MVP workloads, run paired benchmarks, preserve raw samples."""
import argparse
import csv
import datetime
import hashlib
import io
import json
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

def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--samples', type=int, default=21)
    p.add_argument('--seed', type=int, default=20261011)
    p.add_argument('--cpu', type=int)
    p.add_argument('--output', type=Path, default=ROOT/'docs/benchmarks/wasmtime-comparison.json')
    p.add_argument('--encoder', type=Path, default=ROOT/'target/debug/wtime-conformance')
    args = p.parse_args()
    if args.samples < 5 or args.seed <= 0: p.error('need at least 5 samples and a positive seed')
    allowed = sorted(os.sched_getaffinity(0)); cpu = args.cpu if args.cpu is not None else allowed[0]
    if cpu not in allowed: p.error('requested CPU is outside available affinity')
    FIXTURES.mkdir(exist_ok=True)
    fixtures = dict(WAT)
    # 1,000 separately compiled functions, each 16 multiply/xor pairs.
    functions = []
    for i in range(1000):
        export = '(export "run") ' if i == 0 else ''
        operations = ' '.join(f'i32.const {1664525 + j + i} i32.mul i32.const {1013904223 + j} i32.xor' for j in range(16))
        functions.append(f'(func {export}(param i32) (result i32) local.get 0 {operations})')
    fixtures['prepare_large'] = '(module\n' + '\n'.join(functions) + '\n)'
    inputs = {'call_overhead': 100, 'integer_loop': 10000, 'memory_fill_scan': 4096, 'branch_calls': 5000, 'prepare_large': 0}
    executable = HERE/'target/release/wtime-bench'
    tracked = [HERE/'Cargo.toml', HERE/'Cargo.lock', HERE/'src/main.rs', HERE/'run.py', ROOT/'Cargo.lock', ROOT/'Cargo.toml', *ROOT.glob('crates/**/*.rs'), *ROOT.glob('crates/**/Cargo.toml')]
    before_sources = {str(path.relative_to(ROOT)): sha(path) for path in sorted(set(tracked))}
    before_binary = sha(executable)
    raw = []; workload_info = {}
    for case, source in fixtures.items():
        wat = FIXTURES/(case+'.wat'); wasm = FIXTURES/(case+'.wasm'); wat.write_text(source+'\n')
        subprocess.run([str(args.encoder), 'encode', str(wat), '-o', str(wasm)], check=True, cwd=ROOT)
        expected = oracle(case, inputs[case]) if case != 'prepare_large' else 0
        command = [str(executable), str(wasm), case, str(inputs[case]), str(expected), str(args.samples), str(args.seed)]
        result = subprocess.run(command, cwd=ROOT, text=True, capture_output=True, timeout=180,
                                preexec_fn=lambda: os.sched_setaffinity(0, {cpu}))
        if result.returncode: raise RuntimeError(f'{case}: {result.stderr}')
        rows = list(csv.reader(io.StringIO(result.stdout)))
        expected_rows = args.samples * (2 if case == 'prepare_large' else 10)
        if len(rows) != expected_rows: raise RuntimeError(f'{case}: incomplete samples {len(rows)}/{expected_rows}')
        for row in rows:
            name, stage, engine, sample, batch, ns = row
            if name != case or float(ns) <= 0: raise RuntimeError('invalid timing output')
            raw.append(dict(workload=name, stage=stage, engine=engine, sample=int(sample), batch=int(batch), ns_per_operation=float(ns)))
        workload_info[case] = dict(input=inputs[case], expected_result=expected, wasm_bytes=wasm.stat().st_size,
                                   wasm_sha256=sha(wasm), wat_sha256=sha(wat), command=command)
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
    after_sources = {str(path.relative_to(ROOT)): sha(path) for path in sorted(set(tracked))}
    if before_sources != after_sources or before_binary != sha(executable): raise RuntimeError('implementation changed during benchmark')
    report = dict(passed=True, source_stable=True, recorded_at_utc=datetime.datetime.now(datetime.timezone.utc).isoformat(), wasm_correctness_checked=True, wasmtime_version='49.0.2', samples=args.samples, seed=args.seed,
                  hardware=dict(cpu_model=cpu_model, os=platform.platform(), cpu_affinity_available=allowed, selected_cpu=cpu,
                                cpu_quota=Path('/sys/fs/cgroup/cpu.max').read_text().strip()),
                  rustc=subprocess.check_output(['rustc', '-Vv'], text=True),
                  settings=dict(profile='release', wasmtime_features=['std', 'runtime', 'cranelift'], opcode_coverage=False, cranelift_optimization='speed', fuel=False, epoch_interruption=False,
                                wasmtime_cache=False, wasmtime_parallel_compilation=False, wtime_features='mvp',
                                call_api='dynamic values; reusable argument arrays and Wasmtime result buffer; wtime returns owned results',
                                preparation='fresh initialized engine per sample; file IO/engine initialization/module drop excluded',
                                instantiation='compiled module reused; fresh store creation/export lookup/drop excluded; no start functions',
                                first_call='single invocation on fresh instance, excludes preparation/instantiation',
                                load_and_call='fresh initialized engine; includes compilation/store/instantiation/export lookup/first invocation; excludes file IO and engine initialization',
                                warm_call='five warmup invocations, calibrated batches targeting 20 ms, deterministic alternating A/B order'),
                  benchmark_binary_sha256=before_binary, encoder_sha256=sha(args.encoder),
                  implementation_sources=after_sources,
                  workloads=workload_info, summary=summaries, raw_samples=raw,
                  limitations=['Synthetic MVP workloads, not a production application suite.', 'Cloud CPU affinity does not guarantee an otherwise idle physical core.', 'Preparation compares interpreter validation/lowering against optimized JIT machine-code generation.', 'Reported invocation includes dynamic embedding API overhead.'])
    args.output.parent.mkdir(parents=True, exist_ok=True); args.output.write_text(json.dumps(report, indent=2)+'\n')
    for row in summaries:
        print(f"{row['workload']:18} {row['stage']:12} wtime={row['wtime']['median_ns']/1000:10.3f} us wasmtime={row['wasmtime']['median_ns']/1000:10.3f} us ratio={row['wtime_over_wasmtime']:.2f}")

if __name__ == '__main__': main()
