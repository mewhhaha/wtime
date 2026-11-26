# wtime

**A safe-Rust, zero-dependency WebAssembly Core 3.0 runtime — with an optional ahead-of-time compiler when you want native speed.**

`#![forbid(unsafe_code)]` · **0** external production dependencies · **Wasm Core 3.0 + Phase 3–5** · **No JIT at runtime**

---

## Why `wtime`?

Most WebAssembly runtimes force a trade-off: ship a large JIT compiler with a broad `unsafe` surface, or settle for an incomplete interpreter.

`wtime` keeps those concerns cleanly separated:

- **Safe core runtime (`crates/*`)** — 100% safe Rust with zero third-party dependencies, full Core 3.0 + Phase 3–5 coverage (GC, exceptions, threads/atomics, memory64, relaxed SIMD, stack-switching continuations), deterministic fuel metering, cooperative cancellation, and recoverable OOM limits. No JIT and no native guest stack.
- **Opt-in AOT compiler (`tools/wtime-aot`)** — compiles validated modules *ahead of time* to native code while keeping execution JIT-free. Choose between **pure-Rust native backends** (`--backend x64` with full 236-opcode SIMD + all 20 `relaxed_simd` opcodes, `table64`, `function-references` (`call_ref`/`return_call_ref`/`ref.as_non_null`/`br_on_null`/`br_on_non_null`/explicit table initializers), passive/declarative bulk memory & table ops + dynamic `call_indirect`, `wide-arithmetic`, `multi-memory`, `exception_handling` (`try_table`/`throw`/`throw_ref`/`exnref`), O(1)-stack `tail_call` `return_call`/`return_call_indirect`/`return_call_ref`, AVX2 runtime-gated loops / 3-tap blur / `f64x4` SLP / `F64AxpyLoop` auto-vectorization, `LoadScaled`/`AddScaled` SIB addressing, preheader-split loop-carried load forwarding, vote-counted peer coalescing, Euclidean pow2-rem folding + constant-trip loop unrolling, sparse equality-filtered loop narrowing, 1-branch `memory64` bounds checks, `.delta` incremental sidecars, and standalone static Linux executables with WASI/Goose OS+search sandboxing — plus `--backend aarch64` for ARMv8-A with multi-core parallel builds, constant rematerialization + 2-pass hot-fallthrough trace layout + vote-counted coalescing + branch inversion + leaf stack-check elision, `a64-inc` byte-exact incremental caching, standalone guard-page BCE + Ryū/WASI/Goose-sandbox parity, native `ubuntu-24.04-arm` CI testing, and full 236+20 NEON SIMD + `table64` + `function-references` + bulk memory/table + `tail_call` + `wide-arithmetic` + `multi-memory` + `exception_handling` parity, zero LLVM/Clang) or **C99/Clang** and **safe Rust** backends when peak LLVM optimization matters.

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

Direct 5-engine comparison across 6 kernels on CPU 0 (`AMD Ryzen 7 7800X3D`, Wasmtime `49.0.3`, Node `v26.5.0`). Pure-Rust `--backend x64` beats Wasmtime on **all 6 kernels** (even on `heapsort_i32` with explicit software bounds checks and zero signal handlers) at a **6.69×** 6-kernel geometric-mean speedup (**433×** on `graph_bfs`, **15.9×** on `image_filter`, **3.97×** on `ecs_kernel`, **3.24×** on `matmul_f64`, **1.005×** on `heapsort_i32`, **1.003×** on `parser_hash`), and achieves **5.18×** geomean over V8, **6.50×** over Clang `-O2`, and **3.67×** over Rust/LLVM `-O3` (**1.88×** on `matmul_f64`, **1.64×** on `heapsort_i32`, **1.25×** on `ecs_kernel`).

![Execution time on Wasm kernels comparing wtime-aot backends, Wasmtime, and V8](tools/wtime-aot/charts/runtime-kernels.svg)

<br>

### Compilation Speed, Incremental Rebuilds, & Warm Load

On a **4,523-function (`5.91 MiB`)** SIMD-enabled Wasm application, pure-Rust `--backend x64` needs no external compiler or LLVM, compiles **2.49× faster cold** than Wasmtime Cranelift (`151.3 ms` vs. `377.1 ms`), rebuilds a 1-function edit **15.9× faster** (**23.2 ms** vs. `369.7 ms`) via `.delta` incremental sidecars + `.vidx` incremental validation + parallel `.text` linking, hits warm CLI caches in **8.31 ms**, compiles small kernels in **1.41–1.72 ms** (faster than Wasmtime on all 6 kernels), and produces an **8.03 MiB** `.so` (vs. `9.49 MiB` `.cwasm`, **15.4%** smaller) that loads in **0.034 ms** (**54×** faster).

![Cold compilation time comparing wtime-aot backends and Wasmtime](tools/wtime-aot/charts/compile-cold.svg)

<br>

![Incremental rebuild, warm load, and cache hit latency](tools/wtime-aot/charts/compile-hot.svg)

---

## Choosing a Mode

![Choosing a wtime execution mode: compile speed vs. runtime speed](docs/assets/modes.svg)

- **`wtime-runtime`** *(core)* — untrusted plugins, full Core 3.0 + Phase 3–5, deterministic fuel, cooperative cancellation, zero deps (plus opt-in `--features register-executor` lazy register tier with contiguous dense-memory fast path `<= 16 MiB`, native 256-opcode `simd` + `relaxed_simd`, bulk memory/table ops, O(1)-frame `tail_call` + `wide-arithmetic`, **4.09–4.67×** faster on compute kernels with `99.98%` function admission).
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
- **Architecture & Release** — [Spec scope](docs/architecture/scope.md) · [API contract](docs/architecture/api.md) · [Security model](docs/architecture/security-model.md) · [Release readiness (`v0.1.6`)](docs/release-readiness.md) · [Changelog](CHANGELOG.md) · [Security](SECURITY.md)
