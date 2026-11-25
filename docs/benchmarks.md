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
| `graph_bfs` | **0.015 µs** *(433× vs WT)* | 6.47 µs | **1.12 µs** *(5.80× vs WT)* | 6.49 µs | 2.56 µs | `04aa4f2359a193ae` |
| `parser_hash` | **3.31 µs** *(1.00× vs WT)* | 3.31 µs | 3.31 µs *(1.00× vs WT)* | 3.32 µs | 3.41 µs | `0501be7dc7425cf0` |
| `image_filter` | **0.415 µs** *(15.9× vs WT)* | 6.68 µs | 3.53 µs *(1.87× vs WT)* | 6.61 µs | 6.63 µs | `79b786ce0d4a3fc0` |
| `ecs_kernel` | **5.00 µs** *(3.97× vs WT)* | 10.34 µs | **6.24 µs** *(3.18× vs WT)* | 19.87 µs | 12.78 µs | `9f9dd941bb465bbb` |
| `matmul_f64` | **42.03 µs** *(3.24× vs WT)* | 130.15 µs | **79.13 µs** *(1.72× vs WT)* | 136.36 µs | 116.92 µs | `d439b84aa68434e6` |
| `heapsort_i32` | **32.29 µs** *(1.005× vs WT; 1.64× vs Rust)* | 54.68 µs | 52.92 µs *(0.61× vs WT)* | 32.43 µs | **31.18 µs** | `2fa61a2b53989045` |

Across all 6 kernels, pure-Rust `--backend x64` beats Wasmtime `49.0.3` on **all 6 kernels** at a **6.69×** geometric-mean speedup (**5.18×** over V8, **6.50×** over Clang `-O2`, and **3.67×** over `--backend rust (-O3)` at **1.82×** vs. Wasmtime). Key optimizations include sparse equality-filtered counted-loop narrowing (`graph_bfs`: **0.015 µs**, **433×** faster than Wasmtime), AVX2 16-byte/iter 3-tap blur vectorization + `Select` boundary-clamp peeling (`image_filter`: **0.415 µs**, **15.9×** faster than Wasmtime and **8.51×** faster than Rust `-O3`), hoisted & deduplicated Granlund–Montgomery magic-constant loop division + VEX/SLP (`ecs_kernel`: **5.00 µs**, **3.97×** faster than Wasmtime and **1.25×** faster than Rust `-O3`), bit-exact 2×-unrolled AVX2/SSE2/NEON `F64AxpyLoop` auto-vectorization with runtime bounds & alias guards (`matmul_f64`: **42.03 µs**, **3.24×** faster than Wasmtime, **3.10×** faster than Clang `-O2`, and **1.88×** faster than Rust `-O3`), and `LoadScaled`/`AddScaled` (with signed displacement folding) + preheader-splitting loop-carried load forwarding + late empty-trampoline threading & diamond linearization + vote-counted peer coalescing (`heapsort_i32`: **32.29 µs**, **1.64×** faster than Rust `-O3`, **1.69×** faster than Clang `-O2`, and faster than Wasmtime's `32.43 µs` despite enforcing explicit software bounds checks with zero process-wide signal handlers in `--blot-cdylib`). On standalone Linux `ET_EXEC` workloads, both `--backend x64` and `--backend aarch64` provide `8 GiB + 64 KiB` guard-page bounds-check elimination (`memory32` only, zero signal handlers in `--blot-cdylib`), full WASI/Goose/`env::gs_os_*`/`env::gs_find_*` direct-syscall import trampolines with `openat2` (`RESOLVE_BENEATH | O_NOFOLLOW`) sandboxing, built-in pure-assembly 128-bit Ryū shortest round-trip float formatting, and Euclidean pow2-rem folding + constant-trip unrolling — sweeping all 8 standalone Goose `suite2` workloads over Rust `-O3` and running the real 5.38 MiB `memory64` self-hosting `compiler-64.wasm` natively with byte-exact output parity.

---

## 3. Compilation, Incremental Rebuilds, & Warm Load

Measured across 5 fresh-cache rounds (cold & 1-function edit) and 7 warm rounds on a real **5.91 MiB, 4,523-function** SIMD-enabled WebAssembly application (`--jobs 0`, 16 threads) plus the 6 self-contained kernels. (`--backend c` declines this app cleanly because it uses 128-bit Wasm SIMD.)

- **Cold compile boundary** (identical across engines): fresh process reads `.wasm` from disk, compiles multi-core, and writes the compiled artifact (`.so` + `.sha256` for `wtime-aot`; serialized `.cwasm` for Wasmtime).
- **1-function edit boundary**: inserts a `nop` into one function body with a warmed cache (`x64-inc` parallel function cache + compact `.delta` incremental sidecars + 64 KiB Merkle chunking + companion `.vidx` Merkle-verified incremental validation index + overlapped `.bin`/`.delta` preload + filtered inlining strata + zero-alloc SCCs + parallel `.text` merge & relocation patching for `x64`; full recompile for Wasmtime).
- **Warm artifact load boundary**: fresh process maps and validates the compiled artifact (`dlopen(..., RTLD_NOW)` + `dlsym` for `.so`; `Module::deserialize_file` for `.cwasm`).

![Cold compilation time comparing wtime-aot backends and Wasmtime](../tools/wtime-aot/charts/compile-cold.svg)

![Incremental rebuild, warm load, and cache hit latency](../tools/wtime-aot/charts/compile-hot.svg)

| Metric (`4,523`-fn, `5.91 MiB` SIMD app) | `wtime-aot x64` | Wasmtime `49.0.3` |
| --- | ---: | ---: |
| Cold compile (empty cache, 16T) | **151.3 ms** *(2.49× vs WT)* | 377.1 ms |
| Incremental 1-fn edit (warm cache, 16T) | **23.2 ms** *(15.9× vs WT)* | 369.7 ms |
| Warm fresh-process load (`.so` vs `.cwasm`) | **0.034 ms** *(54× vs WT)* | 1.84 ms |
| Warm CLI `wtime-aot build --cache` hit | **8.31 ms** | — |
| Compiled artifact size | **8.03 MiB** (`.so`) | 9.49 MiB (`.cwasm`) |

On tiny 0.3–0.5 KiB kernels, `wtime-aot x64` cold CLI builds take **1.41–1.72 ms** (faster than Wasmtime's **1.51–1.94 ms** on all 6 kernels; warm cache hits **0.81–0.93 ms**), compared to **406–417 ms** for Clang `-O2` and **1.80–1.85 s** for Rust `-O3`.

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
- **Compute-heavy steady state**: long-running CPU loops run roughly **10–120×** slower than native JIT/AOT code in the default decoded interpreter. Enabling the opt-in experimental `--features register-executor` lazy register tier (12-byte packed `RegOp`, variable 1-or-2 `u64` slots for full native 236-opcode `simd` + 20-opcode `relaxed_simd` parity, contiguous dense-memory fast path for active memories `<= 16 MiB` with fallible promotion and clean sparse demotion past `16 MiB`, zero-`Arc` intra-instance calls, O(1)-frame `tail_call` `return_call`/`return_call_indirect`, `wide-arithmetic` `0xfc13..=0xfc16`, fused integer/float/branch superinstructions, and native `call_indirect`/`memory.copy`/`memory.fill`/`memory.grow` lowering) speeds up the 4 kernel workloads by **4.09–4.67×** (`ecs_f64` `4.67×`, `parser_hash` `4.23×`, `image_filter` `4.14×`, `graph_bfs` `4.09×`) and 8-page (`512 KiB`) multi-page scatter/gather by **3.32×** (`137.7 µs` vs. `457.0 µs`) while preserving bit-exact Wasm semantics, source-level fuel, and cancellation — and lazily admitting **99.8%** (`4,443 / 4,454`) of functions in the 5.72 MiB SIMD app in `27.9 ms`. When native throughput matters, compile hot modules ahead of time with `tools/wtime-aot`.

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
