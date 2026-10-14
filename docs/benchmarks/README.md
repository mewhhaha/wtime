# Wasm performance against Wasmtime

**wtime prepares modules faster; Wasmtime executes these compute workloads much faster.**

Two independently ordered runs used Wasmtime 49.0.2, release builds, single-CPU affinity (CPU 0 then CPU 2), Cranelift `Speed`, and fuel/epoch/cache/parallel compilation disabled. Each run collected 21 samples per engine/stage. All results matched independent Python oracles. Ranges below are the two run medians, not minima/maxima of individual measurements.

Hardware: Intel Xeon Platinum 8573C, x86_64 Linux, container quota of four CPUs. Affinity does not guarantee an idle physical CPU. The memory case varied materially between runs; its full samples and quartiles are retained.

## Module preparation

Engine construction and file I/O are excluded; each sample uses a fresh initialized engine. wtime validates and lowers interpreter code, while Wasmtime validates and generates native code.

| Module | wtime | Wasmtime |
| --- | ---: | ---: |
| Tiny add | 14.20–14.38 µs | 708.92–976.45 µs |
| Integer loop | 19.65–20.65 µs | 1126.04–1273.12 µs |
| Memory fill/scan | 21.66–22.09 µs | 1174.37–1314.99 µs |
| Branches/calls | 22.64–23.00 µs | 1151.73–2730.47 µs |
| 1,000-function module | 7.72–8.19 ms | 133.68–141.38 ms |

## Repeated execution

One invocation on an already prepared instance. Dynamic embedding APIs and their marshalling/allocation are included; five warmup calls precede calibrated timing batches.

| Work per invocation | wtime | Wasmtime | wtime slowdown |
| --- | ---: | ---: | ---: |
| Input + 1 | 0.66–0.72 µs | 0.32–0.37 µs | 1.9–2.0× |
| 10,000 LCG iterations | 17.76–20.31 ms | 24.02–29.14 µs | 696.8–739.3× |
| 4,096 stores + 4,096 loads | 17.83–38.42 ms | 5.71–6.81 µs | 3120.5–5644.0× |
| 5,000 guest calls/branches | 15.66–22.25 ms | 16.59–22.43 µs | 944.1–991.9× |

## First-use latency

Already-loaded Wasm binary through its first completed invocation, including preparation, Store creation, instantiation and export lookup. Engine construction and file I/O are excluded.

| Workload | wtime | Wasmtime |
| --- | ---: | ---: |
| Tiny add | 23.64–25.56 µs | 777.60–1101.19 µs |
| Integer loop | 16.18–19.67 ms | 1.24–1.27 ms |
| Memory fill/scan | 32.28–38.09 ms | 1.26–1.73 ms |
| Branches/calls | 17.88–19.36 ms | 1.14–1.27 ms |

These synthetic workloads establish a baseline, not an application-wide speed ratio. Even including compilation, Wasmtime is faster for the three loop workloads here. The trivial add favors wtime on first use because it does very little guest computation.

Full preparation, instantiation, first-call and warmed-call samples, hashes, settings and workload sizes: [run 1](wasmtime-comparison.json) and [run 2](wasmtime-comparison-run2.json).

Reusable harness and timing boundaries: [tools/wtime-bench](../../tools/wtime-bench/README.md). To reproduce both runs after building the harness:

```sh
python3 tools/wtime-bench/run.py --cpu 0 --seed 20261011
python3 tools/wtime-bench/run.py --cpu 2 --seed 20261012 --output docs/benchmarks/wasmtime-comparison-run2.json
```

Production code, dependencies and the production workspace lockfile were unchanged. Wasmtime is isolated in the optional benchmark workspace.
