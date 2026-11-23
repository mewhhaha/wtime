# Changelog

All notable changes to `wtime` are documented in this file.

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
