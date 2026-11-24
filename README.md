# wtime

**A safe-Rust, zero-dependency WebAssembly Core 3.0 runtime — with an optional ahead-of-time compiler when you want native speed.**

`#![forbid(unsafe_code)]` · **0** external production dependencies · **Wasm Core 3.0 + Phase 3–5** · **No JIT at runtime**

---

## Why `wtime`?

Most WebAssembly runtimes force a trade-off: ship a large JIT compiler with a broad `unsafe` surface, or settle for an incomplete interpreter.

`wtime` keeps those concerns cleanly separated:

- **Safe core runtime (`crates/*`)** — 100% safe Rust with zero third-party dependencies, full Core 3.0 + Phase 3–5 coverage (GC, exceptions, threads/atomics, memory64, relaxed SIMD, stack-switching continuations), deterministic fuel metering, cooperative cancellation, and recoverable OOM limits. No JIT and no native guest stack.
- **Opt-in AOT compiler (`tools/wtime-aot`)** — compiles validated modules *ahead of time* to native code while keeping execution JIT-free. Choose between **pure-Rust native backends** (`--backend x64` with full 236-opcode SIMD, AVX2 runtime-gated loops, `memory64`, and standalone static Linux executables — ~141.5 ms cold builds, ~62.1 ms incremental rebuilds — plus `--backend aarch64` for ARMv8-A with multi-core parallel builds and 236-opcode NEON parity, zero LLVM/Clang) or **C99/Clang** and **safe Rust** backends when peak LLVM optimization matters.

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

Direct 5-engine comparison on CPU 0 (`AMD Ryzen 7 7800X3D`, Wasmtime `49.0.3`, Node `v26.5.0`). Pure-Rust `--backend x64` beats **both** Wasmtime and V8 on **all 4 kernels** (**2.27×** geomean over Wasmtime, up to **5.78×** faster — edging out Rust/LLVM `-O3` at **2.24×** geomean and beating Clang `-O2` by **1.97×** on `ecs_kernel`).

![Execution time on Wasm kernels comparing wtime-aot backends, Wasmtime, and V8](tools/wtime-aot/charts/runtime-kernels.svg)

<br>

### Compilation Speed, Incremental Rebuilds, & Warm Load

On a **5,072-function (`3.96 MiB`)** Wasm application, pure-Rust `--backend x64` needs no external compiler or LLVM, compiles **2.17× faster cold** than Wasmtime Cranelift (`141.5 ms` vs. `306.5 ms`; **114×** faster than Clang `-O2`), rebuilds a 1-function edit **5.97× faster** (**62.1 ms**) via parallel per-function incremental caching, compiles small kernels in **1.42–2.15 ms**, and produces a **6.81 MiB** `.so` (vs. `7.76 MiB` `.cwasm`) that loads in **0.036 ms** (**60×** faster).

![Cold compilation time comparing wtime-aot backends and Wasmtime](tools/wtime-aot/charts/compile-cold.svg)

<br>

![Incremental rebuild, warm load, and cache hit latency](tools/wtime-aot/charts/compile-hot.svg)

---

## Choosing a Mode

![Choosing a wtime execution mode: compile speed vs. runtime speed](docs/assets/modes.svg)

- **`wtime-runtime`** *(core)* — untrusted plugins, full Core 3.0 + Phase 3–5, deterministic fuel, cooperative cancellation, zero deps (plus opt-in `--features register-executor` lazy register tier, **3.68–4.22×** faster on compute kernels with `99.8%` function admission).
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
- **Architecture & Release** — [Spec scope](docs/architecture/scope.md) · [API contract](docs/architecture/api.md) · [Release readiness (`v0.1.1`)](docs/release-readiness.md) · [Changelog](CHANGELOG.md) · [Security](SECURITY.md)
