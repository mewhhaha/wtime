# Changelog

All notable changes to `wtime` are documented in this file.

## [0.1.1] - 2026-11-24

Backwards-compatible performance and AOT-capability release across the safe-Rust
interpreter (`wtime-runtime`) and standalone AOT compiler (`tools/wtime-aot`).
Core crate public APIs, feature profiles, and the verified `x86_64-unknown-linux-gnu`
core target scope remain unchanged from `0.1.0`.

### Core interpreter (`crates/wtime-runtime`)
- **Opt-in lazy untyped-`u64` register tier (`--features register-executor`)**:
  remains disabled by default and preserves bit-exact Wasm semantics,
  deterministic source-level fuel, and cooperative cancellation while speeding
  up representative kernels by **1.82–4.07×** (`graph_bfs` `2.89×`,
  `parser_hash` `4.07×`, `image_filter` `1.82×`, `ecs_f64` `3.49×`) and lazily
  admitting `83.5%` of functions in a 5,030-fn application in `32 ms`.
- **Default `Decoded` tier fast paths**: fast `f32`/`f64`/16-bit memory paths
  and coalesced straight-line float execution while keeping stack frames compact
  under constrained `RLIMIT_AS` allocation-failure recovery tests.

### Ahead-of-time compiler (`tools/wtime-aot`, standalone tool)
- **`--backend x64` reach**: standalone static Linux x86-64 `ET_EXEC` output
  (zero `libc`/`DT_NEEDED`), `memory64`, and all 236 fixed-width 128-bit Wasm
  SIMD opcodes (`0xfd00..=0xfdff`).
- **`--backend x64` compilation & incremental cache**: dense tables, `FxHash`,
  parallel dependency-stratum inlining, and overlapped zero-copy `x64-inc-v7`
  incremental cache — bringing a 5,034-function (`3.90 MiB`) module to **152 ms**
  cold (**2.04×** faster than Wasmtime `49.0.3`), **61.5 ms** on a 1-function
  edit (**4.96×** faster), and **0.027 ms** warm `.so` load (**80×** faster),
  with tiny kernels compiling in **1.39–1.70 ms**.
- **`--backend x64` peak codegen (bit-exact IEEE-754)**: loop rotation, hot
  layout/align16, loop bounds-check elimination, `MemCopyConst` forwarding,
  boundary-clamp loop peeling, SSE2 `psadbw` byte-sum vectorization, `f64x2`
  SLP vectorization, and flag-preserving backedge moves — reaching a **2.35×**
  4-kernel geometric-mean speedup over Wasmtime `49.0.3` (edging out `--backend
  rust -O3` at **2.31×**).
- **`--backend aarch64`**: new pure-Rust (`#![forbid(unsafe_code)]`) ARMv8-A
  AOT backend emitting Linux AArch64 ELF64 Blot ABI 8 `.so` shared libraries
  and static `ET_EXEC` binaries with zero external compilers or cross-toolchains.

### Documentation & Benchmarks
- Redesigned [README.md](README.md) with architecture and mode trade-off
  visuals, refreshed [docs/benchmarks.md](docs/benchmarks.md), and regenerated
  all comparison charts under `tools/wtime-aot/charts/`.

## [0.1.0] - 2026-11-23

Initial `0.1.0` release of the safe-Rust WebAssembly core interpreter (`wtime-types`,
`wtime-decode`, `wtime-validate`, `wtime-runtime`, `wtime-cli`) and companion
ahead-of-time compiler tooling (`tools/wtime-aot`).

### Core interpreter (`crates/*`)
- **Zero-dependency safe Rust (`#![forbid(unsafe_code)]`)** across all four core
  libraries and `wtime-cli` (verified by `tools/checks/dependency_budget.py`).
- **Pinned WebAssembly Core 3.0 + Phase 3–5 scope**: explicit feature profiles
  (`mvp`, `stable-core`, `all-phase3-core`), non-moving mark/sweep GC, exception
  handling, threads/atomics with portable SC coordinator, stack-switching
  continuations, memory64, multi-memory, fixed-width and relaxed SIMD, wide
  arithmetic, custom page sizes, compact import sections, and specified custom
  descriptors.
- **Explicit resource safety**: bounded decoding/validation, fallible allocation
  checks for linear memories and guest-defined tables (verified under a 128 MiB
  `RLIMIT_AS` worker), explicit heap guest frames, source-level fuel metering,
  cancellation, and generation-checked handles.
- **Public API & CLI**: complete `rustdoc` coverage (`#![warn(missing_docs)]`)
  on all four core library crates, metadata-only `Module::exports`, `ExternalKind`
  re-export from `wtime-runtime`, and hardened `wtime-cli` with `--version`/`-V`,
  `-h`/`--help`, `--` separator, bounded file reads, and distinct documented exit
  codes (`0`–`5`).
- **Supported release target for `0.1.0`**: `x86_64-unknown-linux-gnu` (native
  AArch64 release verification is planned for a follow-up release).

### Ahead-of-time compiler (`tools/wtime-aot`, standalone tool)
- **`--backend rust`**: translates admitted Wasm modules into safe-Rust crates
  (`#![forbid(unsafe_code)]` for all generated Wasm code and `wtime-aot-rt`),
  supporting standalone Goose binaries and `--blot-cdylib` shared libraries with
  content-addressed caching, Rust 1.99.0 float-division NaN shielding, MXCSR
  isolation, and calling-thread stack checks.
- **`--backend c`**: sharded C99/Clang + `ld.lld` Blot ABI 8 `.so` compiler with
  per-shard SHA-256-verified caching (`c-obj/`), `-fcf-protection=return` CET
  shadow-stack hardening, calling-thread stack guards, MXCSR isolation, and exact
  byte-length import tables.
- **`--backend x64`**: self-contained pure-Rust (`#![forbid(unsafe_code)]`) Linux
  x86-64 SysV LP64 native Blot ABI 8 `.so` compiler with SSA optimization,
  register allocation, ELF64 emission, mandatory IR/regalloc verification by
  default, dedicated `mmap` guest stack with guard page, CPUID + `rdsspq` CET
  shadow-stack guard, and zero `libc`/`DT_NEEDED` dependencies.
