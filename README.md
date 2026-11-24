# wtime

**A safe-Rust, zero-dependency WebAssembly Core 3.0 runtime — with an optional ahead-of-time compiler when you want native speed.**

`#![forbid(unsafe_code)]` · **0** external production dependencies · **Wasm Core 3.0 + Phase 3–5** · **No JIT at runtime**

---

## Why `wtime`?

Most WebAssembly runtimes force a trade-off: ship a large JIT compiler with a broad `unsafe` surface, or settle for an incomplete interpreter.

`wtime` keeps those concerns cleanly separated:

- **Safe core runtime (`crates/*`)** — 100% safe Rust with zero third-party dependencies, full Core 3.0 + Phase 3–5 coverage (GC, exceptions, threads/atomics, memory64, relaxed SIMD, stack-switching continuations), deterministic fuel metering, cooperative cancellation, and recoverable OOM limits. No JIT and no native guest stack.
- **Opt-in AOT compiler (`tools/wtime-aot`)** — compiles validated modules *ahead of time* to native code while keeping execution JIT-free. Choose between a **pure-Rust x86-64 compiler** (no LLVM/Clang, ~200 ms cold builds, ~110 ms incremental rebuilds) or **C99/Clang** and **safe Rust** backends when peak LLVM optimization matters.

<br>

![wtime two-layer architecture](docs/assets/architecture.svg)

---

## Benchmarks vs. Wasmtime & V8

Every benchmark below verifies **byte-exact output parity** across all engines. Full tables, hardware specs, interpreter trade-offs, and reproduction commands live in **[docs/benchmarks.md](docs/benchmarks.md)**.

### At a Glance

| Workload | `wtime-aot x64` | `wtime-aot rust (-O3)` | Wasmtime | V8 |
| --- | ---: | ---: | ---: | ---: |
| **18 standalone & compiler apps** *(vs. competitor)* | — | **1.3–13× vs. WT** · **2.8–148× vs. V8** | 1× | 1× |
| **Wasm kernels** *(`graph_bfs` / `ecs_kernel`)* | **3.37 / 12.96 µs** | **1.31 / 6.17 µs** | 6.55 / 19.50 µs | 2.60 / 12.69 µs |
| **Cold compile** *(5,036-fn, 3.75 MiB app, 16T)* | **207 ms** *(1.4× vs. WT)* | — | 284 ms | — |
| **1-fn edit rebuild** *(5,036-fn app, warm cache)* | **111 ms** *(2.5× vs. WT)* | — | 279 ms | — |
| **Warm artifact load** *(`.so` vs. `.cwasm`)* | **0.032 ms** *(67× vs. WT)* | — | 2.13 ms | — |

<br>

### Execution Speed — Real Programs & Self-Hosting Compiler

Across 18 real workloads (algorithms plus a self-hosting 32- and 64-bit Wasm compiler), `wtime-aot --backend rust` runs **1.3–13× faster than Wasmtime** and **2.8–148× faster than V8** (and beats cached Wasmtime **1.01–2.94×** end-to-end; see [whole-process chart](docs/benchmarks.md#4-standalone-programs--self-hosting-compiler---backend-rust)).

![wtime-aot execution speedup vs Wasmtime and V8](tools/wtime-goose/charts/run.svg)

<br>

### Execution Speed — Self-Contained Wasm Kernels

Direct 5-engine comparison on CPU 0 (`AMD Ryzen 7 7800X3D`, Wasmtime `49.0.3`, Node `v26.5.0`). Pure-Rust `--backend x64` beats Wasmtime on 3 of 4 kernels (up to **1.9×**); `--backend rust (-O3)` beats both Wasmtime (**1.5–5.0×**) and V8 on every kernel.

![Execution time on Wasm kernels comparing wtime-aot backends, Wasmtime, and V8](tools/wtime-aot/charts/runtime-kernels.svg)

<br>

### Compilation Speed, Incremental Rebuilds, & Warm Load

On a **5,036-function (`3.75 MiB`)** Wasm application, pure-Rust `--backend x64` needs no external compiler or LLVM, compiles **1.4× faster cold** than Wasmtime Cranelift (**33×** faster than Clang `-O2`), rebuilds a 1-function edit **2.5× faster** via per-function incremental caching, and produces a **6.61 MiB** `.so` (vs. `7.51 MiB` `.cwasm`) that loads in **0.032 ms**.

![Cold compilation time comparing wtime-aot backends and Wasmtime](tools/wtime-aot/charts/compile-cold.svg)

<br>

![Incremental rebuild, warm load, and cache hit latency](tools/wtime-aot/charts/compile-hot.svg)

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

## Choosing a Mode

| Mode | Safety & Dependencies | Compile Speed | Runtime Speed | Best For |
| --- | --- | --- | --- | --- |
| **`wtime-runtime`** *(core)* | `#![forbid(unsafe_code)]`, **0 deps**, no JIT | Instant *(no codegen)* | Interpreter | Untrusted plugins, full Core 3.0 + Phase 3–5, fuel & cancellation |
| **`wtime-aot --backend x64`** | Pure-Rust compiler (`#![forbid(unsafe_code)]`), **0 LLVM/Clang** | **~207 ms** cold / **~111 ms** 1-fn edit | Matches/beats Cranelift | Fast edit-compile-run loops & zero-toolchain Linux x86-64 AOT |
| **`wtime-aot --backend c`** | Sharded C99 + Clang/`ld.lld` + CET hardening | ~6.7 s cold / ~820 ms 1-fn edit | Faster than Cranelift | Release `.so` builds where host Clang is available |
| **`wtime-aot --backend rust`** | Generated Wasm is `#![forbid(unsafe_code)]` | Seconds *(rustc/LLVM)* | **1.3–13×** vs. Wasmtime | Peak throughput & standalone sandboxed binaries |

---

## Documentation

- **Performance** — [Benchmarks & methodology](docs/benchmarks.md) · [Goose & interpreter benchmarks](tools/wtime-goose/README.md)
- **Using `wtime`** — [Embedding guide](docs/embedding.md) · [Native examples](examples/README.md) · [CLI, crates, & testing](docs/development.md)
- **AOT Compiler** — [`wtime-aot` guide & threat model](tools/wtime-aot/README.md) · [x64 ABI & safety contract](docs/wtime-aot/x64-abi.md)
- **Architecture & Release** — [Spec scope](docs/architecture/scope.md) · [API contract](docs/architecture/api.md) · [Release readiness (`v0.1.0`)](docs/release-readiness.md) · [Changelog](CHANGELOG.md) · [Security](SECURITY.md)
