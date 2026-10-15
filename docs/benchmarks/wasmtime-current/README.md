# Current comparison with Wasmtime

The current Decoded interpreter prepares these modules faster than Wasmtime,
but Wasmtime executes the three loop workloads hundreds of times faster. Compact
and Register remain experimental options; this comparison changes no execution
policy.

These measurements use the unchanged runtime core from `b4ba161`, with explicit
tier selection added to the benchmark harness. Both sides receive the same five
MVP binaries used by the historical benchmark. Wasmtime remains pinned to
**49.0.2**, using Cranelift `Speed`. Each tier has two independently ordered runs,
with **30 complete pairs per workload/stage per run**, on logical CPU 0 and CPU 2
respectively. All runs passed their result and source-identity checks.

## Warm execution

Each range spans the two run medians. Ratios use the median of sample-paired
wtime/Wasmtime times; they are calculated separately from ratios of engine
medians. Values above one favor Wasmtime.

| Workload | Decoded time | Wasmtime time | Decoded / Wasmtime |
| --- | ---: | ---: | ---: |
| Return input + 1 | 0.205–0.213 µs | 0.111–0.114 µs | 1.84–1.87× |
| 10,000 integer iterations | 3.23–3.37 ms | 11.56–11.76 µs | 277–283× |
| 4,096 stores + 4,096 loads | 3.18–3.25 ms | 4.29–4.50 µs | 714–730× |
| 5,000 guest calls/branches | 3.59 ms | 8.85–9.09 µs | 396–403× |

| Workload | Decoded | Compact | Register eager | Register adaptive 3 |
| --- | ---: | ---: | ---: | ---: |
| Return input + 1 | 1.84–1.87× | 1.92–1.96× | 2.32–2.33× | 2.33–2.34× |
| Integer loop | 277–283× | 248–264× | 306–318× | 236–243× |
| Memory fill/scan | 714–730× | 716–732× | 727–734× | 583–608× |
| Calls/branches | 396–403× | 377–385× | 416–424× | 402–418× |

Each optional tier has its own matched Wasmtime measurements. The rows do not
establish a paired comparison between wtime tiers or a portfolio promotion.
These are requested execution policies, with their normal Decoded fallback
available for unsupported function shapes.

## Preparation and first use

Decoded module preparation took 7.47–17.16 µs for the four small modules, compared
with Wasmtime's 290–455 µs. The 1,000-function module took 5.22–5.52 ms in Decoded
and 74.6–79.2 ms in Wasmtime. Across these cases, the paired preparation ratios
correspond to roughly **14–43× faster preparation** in Decoded.

Including preparation through the first invocation, the trivial add took
10.96–11.08 µs in Decoded versus 281–282 µs in Wasmtime. Wasmtime still won the
three loop workloads at first use: Decoded's paired slowdown was 8.0–8.4× for
integers, 6.6–6.7× for memory, and 6.5–7.4× for calls/branches.

Preparation includes decoding, validation, and interpreter lowering for wtime,
and validation plus native code generation for Wasmtime. Engine initialization
and file I/O are excluded. `load_and_call` uses a fresh module; `first_call`
uses a fresh instance of a reused module. Adaptive entry counters and prepared
code are shared across those instances. Warm execution follows five warmup calls
and calibrated batches targeting 20 ms, capped at 100,000 invocations per batch.

## Evidence and reproduction

The [complete results](results.md) retain all stages, per-run medians and
quartiles, and 95% intervals from 10,000 bootstrap resamples of paired ratios.
The [analysis](analysis.json), [portable raw-report index](manifest.json),
[original measurement manifest](measurement-manifest.json),
[frozen sources](source-freeze.json), and [verification](verify.json) preserve
the input and build identities. The raw reports include all **10,080 timing
observations**, representing **5,040 pairs** across 168 run-specific groups.

The [provenance archive](provenance-records.tar.gz) retains frozen source files,
phase receipts, command logs, fixtures, and helpers. Its
[packaging validation](validation.json) checked exact copies and archive member
hashes. Native executables are retained here by identity only; this package does
not recreate the complete verifier CAS or installed build environment.
The final [read-only review](review.json) checked the published figures and
confirmed that runtime Rust and Cargo files were unchanged.

The [shared contract](../wasmtime-current-verification.json) records separate
freeze, build/verification, measurement, and analysis phases. P146 names this
informational follow-up; it does not extend or rewrite the closed 145-task
selection plan. Seven focused Python tests, three feature-specific builds, and
four real tier smoke checks passed before collection. No runtime sources,
production dependencies, or production lockfiles changed.

To reproduce the complete comparison from the repository root:

```sh
. /workspace/.setup/rust-env.sh
python3 -B tools/verify.py plan --contract docs/benchmarks/wasmtime-current-verification.json
python3 -B tools/verify.py close --contract docs/benchmarks/wasmtime-current-verification.json --output target/verification/P146-reproduction-0001 --execute
```

Use a new completion directory. A source-current passing phase can be reused;
use the verifier's global `--store target/verification-P146-fresh-0001` option
before `close` to collect independent new runs. Keep the new store inside the
repository. The contract requires logical CPUs 0 and 2 to be available. Runtime
and benchmark sources are frozen before building; each tier executable is
retained separately before measurement.
The contract builds offline, so its pinned dependencies must already be present
in the Cargo cache.

These are synthetic MVP workloads on a shared Intel Xeon cloud host. They do
not predict full applications, GC, SIMD, threads, or continuations. Affinity
does not ensure physical-core isolation. Fuel, epoch interruption, module
caching, and Wasmtime parallel compilation are disabled. Dynamic embedding API
overhead is included; Wasmtime reuses its output buffer, while wtime returns
owned results. The old comparison remains [historical](../README.md), and its
different host conditions do not establish a before/after speedup.
