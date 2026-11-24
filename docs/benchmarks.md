# Benchmarks & Methodology

All numbers in this document and the root [README](../README.md) come from real runs with **byte-exact output parity** verified across every compared engine.

---

## 1. Test Environment (`wtime-aot` vs. Wasmtime & V8)

Recorded in [`tools/wtime-aot/charts/results.json`](../tools/wtime-aot/charts/results.json) (captured `2026-11-24`):

| Component | Version / Spec |
| --- | --- |
| CPU | AMD Ryzen 7 7800X3D (8 cores / 16 threads) |
| OS | Linux `7.3.5` (`x86_64-unknown-linux-gnu`) |
| Rust toolchain | `rustc 1.99.0` |
| C compiler & linker | `clang 23.1.5` + `ld.lld` |
| Wasmtime | `49.0.3` (Cranelift `OptLevel::Speed`, `parallel_compilation(true)`) |
| V8 | Node.js `v26.5.0` (Liftoff + TurboFan, warmed before timing) |

---

## 2. Execution Time — Self-Contained Wasm Kernels

7 paired rounds per kernel, alternating engine order every round, pinned to CPU 0 (`taskset -c 0`). Lower is better.

![Execution time on Wasm kernels comparing wtime-aot backends, Wasmtime, and V8](../tools/wtime-aot/charts/runtime-kernels.svg)

| Kernel | `wtime-aot x64` | `wtime-aot c (-O2)` | `wtime-aot rust (-O3)` | Wasmtime `49.0.3` | V8 (`Node v26.5`) | Parity hash |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| `graph_bfs` | **1.17 µs** *(5.75× vs WT)* | 6.67 µs | **1.21 µs** *(5.54× vs WT)* | 6.73 µs | 2.65 µs | `04aa4f2359a193ae` |
| `parser_hash` | **3.52 µs** *(1.00× vs WT)* | **3.45 µs** | **3.47 µs** *(1.02× vs WT)* | 3.52 µs | 3.54 µs | `0501be7dc7425cf0` |
| `image_filter` | **3.63 µs** *(1.57× vs WT)* | **4.45 µs** *(1.28× vs WT)* | **3.52 µs** *(1.62× vs WT)* | 5.71 µs | 4.54 µs | `79b786ce0d4a3fc0` |
| `ecs_kernel` | **6.61 µs** *(3.36× vs WT)* | **11.49 µs** *(1.93× vs WT)* | **7.10 µs** *(3.12× vs WT)* | 22.17 µs | 14.15 µs | `9f9dd941bb465bbb` |

Across all 4 kernels, pure-Rust `--backend x64` achieves a **2.35×** geometric-mean speedup over Wasmtime `49.0.3`, edging out `--backend rust (-O3)` (**2.31×**).

---

## 3. Compilation, Incremental Rebuilds, & Warm Load

Measured across 5 fresh-cache rounds (cold & 1-function edit) and 7 warm rounds on a real **3.90 MiB, 5,034-function** WebAssembly application (`--jobs 0`, 16 threads) plus the 4 self-contained kernels.

- **Cold compile boundary** (identical across engines): fresh process reads `.wasm` from disk, compiles multi-core, and writes the compiled artifact (`.so` + `.sha256` for `wtime-aot`; serialized `.cwasm` for Wasmtime).
- **1-function edit boundary**: inserts a `nop` into one function body with a warmed cache (`x64-inc-v7` parallel function cache for `x64`, `c-obj/` shard cache for `c`; full recompile for Wasmtime).
- **Warm artifact load boundary**: fresh process maps and validates the compiled artifact (`dlopen(..., RTLD_NOW)` + `dlsym` for `.so`; `Module::deserialize_file` for `.cwasm`).

![Cold compilation time comparing wtime-aot backends and Wasmtime](../tools/wtime-aot/charts/compile-cold.svg)

![Incremental rebuild, warm load, and cache hit latency](../tools/wtime-aot/charts/compile-hot.svg)

| Metric (`5,034`-fn, `3.90 MiB` app) | `wtime-aot x64` | `wtime-aot c (-O2)` | Wasmtime `49.0.3` |
| --- | ---: | ---: | ---: |
| Cold compile (empty cache, 16T) | **152 ms** *(2.04× vs WT, 102× vs c)* | 15.45 s | 309 ms |
| Incremental 1-fn edit (warm cache, 16T) | **61.5 ms** *(4.96× vs WT, 13.4× vs c)* | 825 ms | 305 ms |
| Warm fresh-process load (`.so` vs `.cwasm`) | **0.027 ms** *(80× vs WT)* | **0.188 ms** | 2.16 ms |
| Warm CLI `wtime-aot build --cache` hit | **22.1 ms** | 323 ms | — |
| Compiled artifact size | **6.74 MiB** (`.so`) | 9.24 MiB (`.so`) | 7.69 MiB (`.cwasm`) |

On tiny 0.3–0.5 KiB kernels, `wtime-aot x64` cold CLI builds take **1.39–1.70 ms** (warm cache hits **0.81–0.93 ms**), beating Wasmtime (**1.56–1.92 ms**) on all 4 kernels, compared to **441–470 ms** for Clang `-O2` and **1.90–2.01 s** for Rust `-O3`.

---

## 4. Standalone Programs & Self-Hosting Compiler (`--backend rust`)

18 real programs — 10 standalone algorithms (`sorting`, `life`, `sudoku`, `sieve`, `maze`, `vm`, etc.) plus 8 self-hosting Goose compiler workloads across both `memory32` and `memory64` — measured on single-CPU paired rounds (`AMD Ryzen 7 3800X`, Linux `7.3.4`, Wasmtime `50.0.1`, Deno `2.9.8` / V8 `15.0`). Full per-program tables and methodology live in [`tools/wtime-goose/README.md`](../tools/wtime-goose/README.md#ahead-of-time-wtime-aot).

- **Execution (`_start`)**: `wtime-aot --backend rust` beats Wasmtime by **1.3–13×** and V8 by **2.8–148×** across all non-trivial workloads.
- **Whole process (start, load, run, exit)**: beats **cached Wasmtime** by **1.01–2.94×** and **V8** by **3.2–94×**.

![wtime-aot execution speedup vs Wasmtime and V8](../tools/wtime-goose/charts/run.svg)

![wtime-aot whole-process speedup vs cached Wasmtime and V8](../tools/wtime-goose/charts/process.svg)

---

## 5. Core Interpreter (`wtime-runtime`) Performance Profile

The core interpreter (`crates/*`) intentionally excludes JIT compilation and native stack execution to keep `#![forbid(unsafe_code)]`, zero dependencies, deterministic source-level fuel, and full Core 3.0 + Phase 3–5 semantics:

- **Fast cold startup & small inputs**: on short programs and uncached cold loads, `wtime-runtime` finishes in **0.06–0.15×** of V8's process time and avoids multi-second uncached JIT compilation (preparing a `7.3 MiB` compiler module in `0.20–0.29 s` vs. `5.4–7.5 s` for uncached Wasmtime).
- **Compute-heavy steady state**: long-running CPU loops run roughly **10–120×** slower than native JIT/AOT code in the default decoded interpreter. Enabling the opt-in experimental `--features register-executor` lazy untyped-`u64` register tier speeds up the 4 kernel workloads by **1.82–4.07×** (`graph_bfs` `2.89×`, `parser_hash` `4.07×`, `image_filter` `1.82×`, `ecs_f64` `3.49×`) while preserving bit-exact Wasm semantics, source-level fuel, and cancellation (and lazily admitting `83.5%` of functions in the 5k-fn app in `32 ms`). When native throughput matters, compile hot modules ahead of time with `tools/wtime-aot`.

See [`tools/wtime-goose/README.md`](../tools/wtime-goose/README.md#results-2026-11-20) for the full interpreter benchmark tables.

---

## 6. Reproducing Locally

Anyone with Rust `1.99.0`, Clang, and Node.js can reproduce the self-contained kernel benchmarks and charts:

```sh
cargo build --locked --release --manifest-path tools/wtime-aot/Cargo.toml
python3 -B tools/wtime-aot/bench.py --kernels-only --cold-rounds 5 -o target/aot-kernels.json
python3 -B tools/wtime-aot/chart.py target/aot-kernels.json -o target/aot-charts
```

Optional flags:
- `--wasmtime-driver PATH`: include Wasmtime in kernel runtime, cold compile, and large-module benchmarks.
- `--large-wasm PATH`: include a multi-thousand-function `.wasm` module for cold compile, 1-function incremental edit, and warm artifact load comparisons.
