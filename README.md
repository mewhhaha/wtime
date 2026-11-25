# wtime

**A safe-Rust, zero-dependency WebAssembly Core 3.0 runtime — with an optional ahead-of-time compiler when you want native speed.**

`#![forbid(unsafe_code)]` · **0** external production dependencies · **Wasm Core 3.0 + Phase 3–5** · **No JIT at runtime**

---

## Why `wtime`?

Most WebAssembly runtimes force a trade-off: ship a large JIT compiler with a broad `unsafe` surface, or settle for an incomplete interpreter.

`wtime` keeps those concerns cleanly separated:

- **Safe core runtime (`crates/*`)** — 100% safe Rust with zero third-party dependencies, full Core 3.0 + Phase 3–5 coverage (GC, exceptions, threads/atomics, memory64, relaxed SIMD, stack-switching continuations), deterministic fuel metering, cooperative cancellation, and recoverable OOM limits. No JIT and no native guest stack.
- **Opt-in AOT compiler (`tools/wtime-aot`)** — compiles validated modules *ahead of time* to native code while keeping execution JIT-free. Choose between **pure-Rust native backends** (`--backend x64` with full 236-opcode SIMD + all 20 `relaxed_simd` opcodes, `wide-arithmetic`, `multi-memory`, `exception_handling` (`try_table`/`throw`/`throw_ref`/`exnref`), O(1)-stack `tail_call` `return_call`/`return_call_indirect`, AVX2 runtime-gated loops / 3-tap blur / `f64x4` SLP / `F64AxpyLoop` auto-vectorization, `LoadScaled`/`AddScaled` SIB addressing, diamond load forwarding, sparse equality-filtered loop narrowing, 1-branch `memory64` bounds checks, `.delta` incremental sidecars, and standalone static Linux executables — ~151.2 ms cold builds, ~24.2 ms 1-fn incremental rebuilds, ~9.97 ms warm hits on a 5.79 MiB app — plus `--backend aarch64` for ARMv8-A with multi-core parallel builds, `a64-inc` byte-exact incremental caching, standalone guard-page BCE + Ryū/WASI parity, and full 236+20 NEON SIMD + `tail_call` + `wide-arithmetic` + `multi-memory` + `exception_handling` parity, zero LLVM/Clang) or **C99/Clang** and **safe Rust** backends when peak LLVM optimization matters.

<br>

![wtime two-layer architecture](docs/assets/architecture.svg)

---

## Benchmarks vs. Wasmtime & V8

Every benchmark below verifies **byte-exact output parity** across all engines. Full tables, hardware specs, interpreter trade-offs, and reproduction commands live in **[docs/benchmarks.md](docs/benchmarks.md)**.

### At a Glance

![At a glance speedup and size reduction vs Wasmtime](tools/wtime-aot/charts/summary.svg)

<br>

### Execution Speed — Real Programs & Self-Hosting Compiler

Across 18 real workloads (algorithms plus a self-hosting 32- and 64-bit Wasm compiler), `wtime-aot --backend rust` runs **1.3–13× faster than Wasmtime** and **2.8–148× faster than V8** (and beats cached Wasmtime **1.01–2.94×** end-to-end; see [whole-process chart](docs/benchmarks.md#4-standalone-programs--self-hosting-compiler---backend-rust)).

![wtime-aot execution speedup vs Wasmtime and V8](tools/wtime-goose/charts/run.svg)

<br>

### Execution Speed — Self-Contained Wasm Kernels

Direct 5-engine comparison across 6 kernels on CPU 0 (`AMD Ryzen 7 7800X3D`, Wasmtime `49.0.3`, Node `v26.5.0`). Pure-Rust `--backend x64` achieves a **6.22×** 6-kernel geometric-mean speedup over Wasmtime (**434×** on `graph_bfs` via sparse equality-filtered counted-loop narrowing, **13.7×** on `image_filter` via AVX2 blur vectorization, **4.10×** on `ecs_kernel` via hoisted magic division + VEX/SLP, **3.17×** on `matmul_f64` via bit-exact AVX2 AXPY auto-vectorization — beating Rust/LLVM `-O3` by **1.78×** on `matmul_f64`, **1.30×** on `ecs_kernel`, **1.22×** on `heapsort_i32` (`43.11 µs` vs. `52.69 µs`), and **3.55×** geomean).

![Execution time on Wasm kernels comparing wtime-aot backends, Wasmtime, and V8](tools/wtime-aot/charts/runtime-kernels.svg)

<br>

### Compilation Speed, Incremental Rebuilds, & Warm Load

On a **4,480-function (`5.79 MiB`)** SIMD-enabled Wasm application, pure-Rust `--backend x64` needs no external compiler or LLVM, compiles **2.64× faster cold** than Wasmtime Cranelift (`151.2 ms` vs. `399.8 ms`), rebuilds a 1-function edit **16.6× faster** (**24.2 ms**) via `.delta` incremental sidecars + `.vidx` incremental validation + parallel `.text` linking, hits warm CLI caches in **9.97 ms**, compiles small kernels in **1.40–1.77 ms** (faster than Wasmtime at `1.56–1.81 ms`), and produces a **7.75 MiB** `.so` (vs. `9.34 MiB` `.cwasm`, **17.1%** smaller) that loads in **0.031 ms** (**59×** faster).

![Cold compilation time comparing wtime-aot backends and Wasmtime](tools/wtime-aot/charts/compile-cold.svg)

<br>

![Incremental rebuild, warm load, and cache hit latency](tools/wtime-aot/charts/compile-hot.svg)

---

## Choosing a Mode

![Choosing a wtime execution mode: compile speed vs. runtime speed](docs/assets/modes.svg)

- **`wtime-runtime`** *(core)* — untrusted plugins, full Core 3.0 + Phase 3–5, deterministic fuel, cooperative cancellation, zero deps (plus opt-in `--features register-executor` lazy register tier with contiguous dense-memory fast path `<= 16 MiB`, native 256-opcode `simd` + `relaxed_simd`, O(1)-frame `tail_call` + `wide-arithmetic`, **4.09–4.67×** faster on compute kernels with `99.8%` function admission).
- **`wtime-aot --backend x64`** / **`--backend aarch64`** — fast edit-compile-run loops and zero-toolchain Linux x86-64 / ARMv8-A AOT (`.so` and static executables, `#![forbid(unsafe_code)]` compiler, no LLVM/Clang).
- **`wtime-aot --backend c`** — sharded Clang `-O2` + `ld.lld` `.so` builds with CET shadow-stack hardening.
- **`wtime-aot --backend rust`** — peak LLVM `-O3` throughput and standalone sandboxed binaries (`#![forbid(unsafe_code)]` guest codegen).

---

## Quick Start

### Run with the safe-Rust interpreter

```sh
cargo run --locked -p wtime-cli -- check module.wasm
cargo run --locked -p wtime-cli -- --fuel 100000 run module.wasm add i32:20 i32:22
```

Or embed `wtime-runtime` directly in Rust — see **[docs/embedding.md](docs/embedding.md)** and **[examples/](examples/README.md)**.

### Compile ahead of time (`wtime-aot`)

```sh
cargo build --locked --release --manifest-path tools/wtime-aot/Cargo.toml

# Pure-Rust x86-64 shared library (Linux x86-64, zero external compiler deps)
tools/wtime-aot/target/release/wtime-aot build module.wasm --cache /tmp/aot --blot-cdylib --backend x64

# Standalone sandboxed binary via safe-Rust codegen (--backend rust)
tools/wtime-aot/target/release/wtime-aot build module.wasm --cache /tmp/aot
```

---

## Documentation

- **Performance** — [Benchmarks & methodology](docs/benchmarks.md) · [Goose & interpreter benchmarks](tools/wtime-goose/README.md)
- **Using `wtime`** — [Embedding guide](docs/embedding.md) · [Native examples](examples/README.md) · [CLI, crates, & testing](docs/development.md)
- **AOT Compiler** — [`wtime-aot` guide & threat model](tools/wtime-aot/README.md) · [x64 ABI & safety contract](docs/wtime-aot/x64-abi.md)
- **Architecture & Release** — [Spec scope](docs/architecture/scope.md) · [API contract](docs/architecture/api.md) · [Release readiness (`v0.1.3`)](docs/release-readiness.md) · [Changelog](CHANGELOG.md) · [Security](SECURITY.md)
