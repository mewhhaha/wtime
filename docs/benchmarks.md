# Benchmarks & Methodology

All numbers in this document and the root [README](../README.md) come from real runs with **byte-exact output parity** verified across every compared engine.

---

## 1. Test Environment (`wtime-aot` vs. Wasmtime & V8)

Recorded in [`tools/wtime-aot/charts/results.json`](../tools/wtime-aot/charts/results.json) (captured `2026-11-25`):

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
| `graph_bfs` | **0.015 µs** *(439× vs WT)* | 6.55 µs | **1.15 µs** *(5.72× vs WT)* | 6.59 µs | 2.64 µs | `04aa4f2359a193ae` |
| `parser_hash` | **3.35 µs** *(1.00× vs WT)* | **3.33 µs** | 3.36 µs *(1.00× vs WT)* | 3.36 µs | 3.45 µs | `0501be7dc7425cf0` |
| `image_filter` | **0.285 µs** *(18.8× vs WT)* | 4.10 µs | 3.59 µs *(1.49× vs WT)* | 5.35 µs | 4.65 µs | `79b786ce0d4a3fc0` |
| `ecs_kernel` | **4.88 µs** *(4.00× vs WT)* | 10.49 µs | **6.41 µs** *(3.04× vs WT)* | 19.49 µs | 13.13 µs | `9f9dd941bb465bbb` |

Across all 4 kernels, pure-Rust `--backend x64` achieves a **13.5×** geometric-mean speedup over Wasmtime `49.0.3` (**5.98×** over `--backend rust (-O3)` at **2.26×**, and **10.8×** over Clang `-O2`), driven by general sparse equality-filtered counted-loop narrowing (`graph_bfs`: narrows `for v in init..limit { if v == k1 || v == k2 { ... } }` to visit only matching indices in ascending order — **0.015 µs**, **439×** faster than Wasmtime), AVX2 16-byte/iter 3-tap blur vectorization + `Select` boundary-clamp peeling (`image_filter`: **0.285 µs**, **18.8×** faster than Wasmtime and **12.6×** faster than Rust `-O3`), hoisted & deduplicated Granlund–Montgomery magic-constant loop division with callee-saved constant promotion, small-constant multiply strength reduction (`lea` / shift-add-sub), redundant `movl` elision via `val_upper32_zero`, 3-operand VEX scalar/SIMD FP + `vmovdqu`/`vpshufd` encoding, and bit-exact 256-bit `f64x4` / 128-bit `f64x2` SLP (`ecs_kernel`: **4.88 µs**, **4.00×** faster than Wasmtime, **2.15×** faster than Clang `-O2`, and **1.32×** faster than Rust `-O3`). On standalone Linux `ET_EXEC` workloads, both `--backend x64` and `--backend aarch64` provide `8 GiB + 64 KiB` guard-page bounds-check elimination (`memory32` only, zero signal handlers in `--blot-cdylib`), full WASI/Goose/`env` direct-syscall import trampolines, and built-in pure-assembly 128-bit Ryū shortest round-trip float formatting.

---

## 3. Compilation, Incremental Rebuilds, & Warm Load

Measured across 5 fresh-cache rounds (cold & 1-function edit) and 7 warm rounds on a real **5.51 MiB, 4,410-function** SIMD-enabled WebAssembly application (`--jobs 0`, 16 threads) plus the 4 self-contained kernels. (`--backend c` declines this app cleanly because it uses 128-bit Wasm SIMD.)

- **Cold compile boundary** (identical across engines): fresh process reads `.wasm` from disk, compiles multi-core, and writes the compiled artifact (`.so` + `.sha256` for `wtime-aot`; serialized `.cwasm` for Wasmtime).
- **1-function edit boundary**: inserts a `nop` into one function body with a warmed cache (`x64-inc` parallel function cache + companion `.vidx` Merkle-verified incremental validation index + overlapped `.bin` preload + reachable-cone `ModuleEffects` + parallel `.text` merge & relocation patching for `x64`; full recompile for Wasmtime).
- **Warm artifact load boundary**: fresh process maps and validates the compiled artifact (`dlopen(..., RTLD_NOW)` + `dlsym` for `.so`; `Module::deserialize_file` for `.cwasm`).

![Cold compilation time comparing wtime-aot backends and Wasmtime](../tools/wtime-aot/charts/compile-cold.svg)

![Incremental rebuild, warm load, and cache hit latency](../tools/wtime-aot/charts/compile-hot.svg)

| Metric (`4,410`-fn, `5.51 MiB` SIMD app) | `wtime-aot x64` | Wasmtime `49.0.3` |
| --- | ---: | ---: |
| Cold compile (empty cache, 16T) | **147.7 ms** *(2.49× vs WT)* | 367.3 ms |
| Incremental 1-fn edit (warm cache, 16T) | **32.6 ms** *(12.9× vs WT)* | 420.2 ms |
| Warm fresh-process load (`.so` vs `.cwasm`) | **0.033 ms** *(55× vs WT)* | 1.80 ms |
| Warm CLI `wtime-aot build --cache` hit | **9.60 ms** | — |
| Compiled artifact size | **7.58 MiB** (`.so`) | 8.97 MiB (`.cwasm`) |

On tiny 0.3–0.5 KiB kernels, `wtime-aot x64` cold CLI builds take **1.43–1.79 ms** (warm cache hits **0.78–0.91 ms**), matching Wasmtime (**1.46–1.83 ms**), compared to **436–524 ms** for Clang `-O2` and **1.80–1.85 s** for Rust `-O3`.

On `--backend aarch64` (2,000-function module, 16T), multi-core cold builds complete in **12.26 ms**, 1-function incremental edits (`a64-inc` + `.vidx`) drop to **7.94 ms** with **100% byte-for-byte ELF parity** against cold builds, and warm CLI cache hits complete in **2.05 ms**.

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
- **Compute-heavy steady state**: long-running CPU loops run roughly **10–120×** slower than native JIT/AOT code in the default decoded interpreter. Enabling the opt-in experimental `--features register-executor` lazy untyped-`u64` register tier (12-byte packed `RegOp`, contiguous dense-memory fast path for active memories `<= 16 MiB` with fallible promotion and clean sparse demotion past `16 MiB`, zero-`Arc` intra-instance calls, fused integer/float/branch superinstructions, and native `call_indirect`/`memory.copy`/`memory.fill`/`memory.grow` lowering) speeds up the 4 kernel workloads by **3.92–4.61×** (`ecs_f64` `4.61×`, `parser_hash` `4.41×`, `graph_bfs` `4.08×`, `image_filter` `3.92×`) and 8-page (`512 KiB`) multi-page scatter/gather by **3.32×** (`137.7 µs` vs. `457.0 µs`) while preserving bit-exact Wasm semantics, source-level fuel, and cancellation — and lazily admitting **99.7%** (`4,569 / 4,582`) of functions in the 4.65 MiB app in `40.5 ms`. When native throughput matters, compile hot modules ahead of time with `tools/wtime-aot`.

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
