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
| `graph_bfs` | **0.016 µs** *(411× vs WT)* | 6.55 µs | **1.12 µs** *(5.85× vs WT)* | 6.58 µs | 2.68 µs | `04aa4f2359a193ae` |
| `parser_hash` | **3.46 µs** *(1.15× vs WT)* | 3.51 µs | 3.51 µs *(1.13× vs WT)* | 3.97 µs | 3.48 µs | `0501be7dc7425cf0` |
| `image_filter` | **0.259 µs** *(19.4× vs WT)* | 4.01 µs | 3.38 µs *(1.49× vs WT)* | 5.04 µs | 4.78 µs | `79b786ce0d4a3fc0` |
| `ecs_kernel` | **4.81 µs** *(4.16× vs WT)* | 10.56 µs | **6.38 µs** *(3.14× vs WT)* | 20.02 µs | 13.09 µs | `9f9dd941bb465bbb` |
| `matmul_f64` | **63.91 µs** *(4.07× vs WT)* | 229.33 µs | **128.80 µs** *(2.02× vs WT)* | 259.95 µs | 205.14 µs | `d439b84aa68434e6` |
| `heapsort_i32` | **61.52 µs** *(0.88× vs WT; 1.04× vs Rust)* | 66.09 µs | 63.86 µs *(0.85× vs WT)* | 54.20 µs | **53.73 µs** | `2fa61a2b53989045` |

Across all 6 kernels, pure-Rust `--backend x64` achieves a **7.18×** geometric-mean speedup over Wasmtime `49.0.3` (**5.36×** over V8, **6.15×** over Clang `-O2`, and **3.70×** over `--backend rust (-O3)` at **1.94×** vs. Wasmtime), beating Rust `-O3` on all 6 kernels. Key optimizations include sparse equality-filtered counted-loop narrowing (`graph_bfs`: **0.016 µs**, **411×** faster than Wasmtime), AVX2 16-byte/iter 3-tap blur vectorization + `Select` boundary-clamp peeling (`image_filter`: **0.259 µs**, **19.4×** faster than Wasmtime and **13.05×** faster than Rust `-O3`), hoisted & deduplicated Granlund–Montgomery magic-constant loop division + VEX/SLP (`ecs_kernel`: **4.81 µs**, **4.16×** faster than Wasmtime and **1.32×** faster than Rust `-O3`), bit-exact 2×-unrolled AVX2/SSE2/NEON `F64AxpyLoop` auto-vectorization with runtime bounds & alias guards (`matmul_f64`: **63.91 µs**, **4.07×** faster than Wasmtime, **3.59×** faster than Clang `-O2`, and **2.02×** faster than Rust `-O3`), `LoadScaled`/`AddScaled` SIB addressing + diamond load forwarding + multi-exit loop versioning + intra-block `LiveFlags` reuse (`heapsort_i32`: **61.52 µs**, beating Rust `-O3` at `63.86 µs` and Clang `-O2` at `66.09 µs`), backward cold-trap block sinking, Euclidean power-of-two remainder folding + bounded 2-round innermost constant-trip (`1..=4`) loop unrolling, and 1-branch wrap-proof `memory64` bounds checks. On `heapsort_i32`, all three Blot `.so` backends (`x64` `61.5 µs`, `rust` `63.9 µs`, `c` `66.1 µs`) intentionally enforce explicit software bounds checks with zero process-wide signal handlers, whereas Wasmtime (`54.2 µs`) and V8 (`53.7 µs`) elide bounds checks via `SIGSEGV` guard pages. On standalone Linux `ET_EXEC` workloads, both `--backend x64` and `--backend aarch64` provide `8 GiB + 64 KiB` guard-page bounds-check elimination (`memory32` only, zero signal handlers in `--blot-cdylib`), full WASI/Goose/`env::gs_os_*`/`env::gs_find_*` direct-syscall import trampolines with `openat2` (`RESOLVE_BENEATH | O_NOFOLLOW`) sandboxing, built-in pure-assembly 128-bit Ryū shortest round-trip float formatting, and Euclidean pow2-rem folding + constant-trip unrolling — cutting standalone Goose `suite2/life.wasm` to **3.08 ms** median (**2.47 ms** uncontended, beating Rust `-O3`'s `3.22 ms`) and running the real 5.38 MiB `memory64` self-hosting `compiler-64.wasm` natively with byte-exact output parity.

---

## 3. Compilation, Incremental Rebuilds, & Warm Load

Measured across 5 fresh-cache rounds (cold & 1-function edit) and 7 warm rounds on a real **5.90 MiB, 4,513-function** SIMD-enabled WebAssembly application (`--jobs 0`, 16 threads) plus the 6 self-contained kernels. (`--backend c` declines this app cleanly because it uses 128-bit Wasm SIMD.)

- **Cold compile boundary** (identical across engines): fresh process reads `.wasm` from disk, compiles multi-core, and writes the compiled artifact (`.so` + `.sha256` for `wtime-aot`; serialized `.cwasm` for Wasmtime).
- **1-function edit boundary**: inserts a `nop` into one function body with a warmed cache (`x64-inc` parallel function cache + compact `.delta` incremental sidecars + 64 KiB Merkle chunking + companion `.vidx` Merkle-verified incremental validation index + overlapped `.bin`/`.delta` preload + filtered inlining strata + zero-alloc SCCs + parallel `.text` merge & relocation patching for `x64`; full recompile for Wasmtime).
- **Warm artifact load boundary**: fresh process maps and validates the compiled artifact (`dlopen(..., RTLD_NOW)` + `dlsym` for `.so`; `Module::deserialize_file` for `.cwasm`).

![Cold compilation time comparing wtime-aot backends and Wasmtime](../tools/wtime-aot/charts/compile-cold.svg)

![Incremental rebuild, warm load, and cache hit latency](../tools/wtime-aot/charts/compile-hot.svg)

| Metric (`4,513`-fn, `5.90 MiB` SIMD app) | `wtime-aot x64` | Wasmtime `49.0.3` |
| --- | ---: | ---: |
| Cold compile (empty cache, 16T, paired) | **263.7 ms** *(2.31× vs WT; 123.5 ms uncontended)* | 609.5 ms |
| Incremental 1-fn edit (warm cache, 16T) | **34.2 ms** *(16.3× vs WT)* | 556.4 ms |
| Warm fresh-process load (`.so` vs `.cwasm`) | **0.031 ms** *(57× vs WT)* | 1.78 ms |
| Warm CLI `wtime-aot build --cache` hit | **13.99 ms** | — |
| Compiled artifact size | **7.86 MiB** (`.so`) | 9.49 MiB (`.cwasm`) |

On tiny 0.3–0.5 KiB kernels, `wtime-aot x64` cold CLI builds take **1.52–2.09 ms** (warm cache hits **0.74–1.06 ms**), compared to **1.49–3.31 ms** for Wasmtime, **393–632 ms** for Clang `-O2`, and **1.76–2.23 s** for Rust `-O3`.

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
