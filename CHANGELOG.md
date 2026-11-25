# Changelog

All notable changes to `wtime` are documented in this file.

## [0.1.3] - 2026-11-25

Major Wasm 3.0 AOT feature-completeness (`wide-arithmetic`, `multi-memory`,
`exception_handling`), branchy/array & real-workload codegen (`Op::LoadScaled`,
`Op::AddScaled`, diamond load forwarding, post-inlining `const64` + constant-divisor
loop specialization), compile-pipeline acceleration, and native 256-opcode SIMD +
`relaxed_simd` + differential fuzzing in the opt-in safe-Rust register interpreter.
Core crate `#![forbid(unsafe_code)]`, zero external production dependencies, public
APIs, and `x86_64-unknown-linux-gnu` release scope remain unchanged.

### Core interpreter (`crates/wtime-runtime`)
- **Opt-in lazy register tier (`--features register-executor`)**:
  - Full native **`simd`** (all 236 fixed-width 128-bit opcodes `0xfd00..=0xfdff`)
    and **`relaxed_simd`** (all 20 opcodes `0xfd 0x100..=0xfd 0x113`) support using
    variable 1-or-2 `u64` register slots per value while preserving the 12-byte
    `RegOp` packing, exact logical `active_local_slots` and `max_stack_height`
    accounting, hot inline `v128` load/store/arith/splat/shuffle/select paths, and
    reusable zero-alloc-per-op scratch delegation for complete `256/256` parity.
  - Added full 256-opcode parity + fuel-prefix sweep tests and a deterministic
    differential fuzzer (80 random valid Wasm modules × 4 fuel budgets = 320 runs
    verified bit-exact against `Decoded`).
  - Raises lazy admission on the 4,454-function SIMD app to **99.8%** (`4,443 /
    4,454` functions in `27.9 ms`) and reaches **4.09–4.67×** over the decoded
    tier across the 4 benchmark kernels (`ecs_f64` `4.67×`, `parser_hash` `4.23×`,
    `image_filter` `4.14×`, `graph_bfs` `4.09×`).

### Ahead-of-time compiler (`tools/wtime-aot`, standalone tool)
- **Wasm 3.0 backend features on BOTH `--backend x64` and `--backend aarch64`**:
  - **`wide-arithmetic`** (`i64.add128`, `i64.sub128`, `i64.mul_wide_s`,
    `i64.mul_wide_u` — `0xfc13..=0xfc16`).
  - **`multi-memory`** (up to 64 extra memories with independent `memory32`/
    `memory64` descriptors, active data segments, `load`/`store`/`v128.load*_lane`/
    `store*_lane`, `memory.size`, `memory.grow`, `memory.fill`, and cross-memory
    `memory.copy`).
  - **`exception_handling`** (`TagSection`, `try_table` with `catch`/`catch_ref`/
    `catch_all`/`catch_all_ref`, `throw`, `throw_ref`, `exnref`) backed by a
    demand-paged 4 MiB RW exception slab with zero overhead for modules without
    tags/EH; guest-only `INTERNAL_EXN_SIGNAL (0xff)` maps to `TRAP_UNREACHABLE (1)`
    at Blot ABI 8 outer boundaries (`1..=22` unchanged) and `throw_ref(null)` traps
    with `TRAP_INDIRECT_CALL (4)` matching `TrapCode::NullReference`.
- **Branchy/array & real-workload x64 + AArch64 codegen**:
  - `Op::LoadScaled`/`Op::StoreScaled` SIB addressing, general `Op::AddScaled`
    (`lea dst, [base + idx * {2,4,8}]` on x64 and `add rd, rn, rm, lsl #k` on
    AArch64, folding `i32.wrap_i64` directly into 32-bit scaled address ops),
    diamond load forwarding, multi-exit/inductive counted-loop versioning,
    intra-block `LiveFlags`/`A64LiveFlags` reuse, and 32-byte AVX2 loop alignment.
  - Post-inlining `const64` + single-constant `Bin -> BinImm` / `Cmp -> CmpImm`
    strength reduction with LICM retry on reduced divisions, zero-clobber 5-cycle
    power-of-two signed `RemS` (`a - ((a + bias) & -(2^k))`), and bounded
    constant-divisor loop specialization inlining.
  - Cuts `heapsort_i32` from `55.76 µs` to **`43.11 µs`** — beating both
    `--backend rust (-O3)` (`52.69 µs`, **1.22×**) and `--backend c (-O2)`
    (`54.87 µs`, **1.27×**) — yields a **6.22×** 6-kernel geomean speedup over
    Wasmtime `49.0.3` (**4.83×** vs. V8, **3.55×** vs. Rust `-O3`), and speeds up
    real standalone Goose workloads (`sorting` **1.47×** to `43.89 ms`, beating
    Rust `-O3`'s `50.61 ms` by 15%; `life` **1.74×** to `6.59 ms`).
- **Compile & hot-reload pipeline (`wtime-decode`, `wtime-validate`, `tools/wtime-aot`)**:
  - Single-thread fast path `< 64 KiB` in decode/validate, work-stealing parallel
    SHA-256 Merkle chunking (64 KiB chunks in `WTX64INC-ROOT-2`), parallel non-code
    section hashing, zero-clone `pre_vidx`, lazy `compiled` / sparse overlays,
    early `DecodedModule` drop, and skipped stale sweeps on fresh caches.
  - On a **5.79 MiB, 4,480-function** SIMD application (16T): **151.2 ms** cold
    (**2.64×** faster than Wasmtime `49.0.3` at `399.8 ms`), **24.2 ms** on a
    1-function edit (**16.6×** faster than Wasmtime at `400.6 ms`), **9.97 ms**
    warm CLI cache hit, **0.031 ms** warm `.so` load (**59×** faster), and
    **17.1%** smaller artifact (`7.75 MiB` vs. `9.34 MiB`).

## [0.1.2] - 2026-11-25

Major pure-Rust AOT codegen, compile-pipeline, Wasm 3.0 backend, and opt-in
register-interpreter release across `wtime-runtime` and `tools/wtime-aot`. Core
crate `#![forbid(unsafe_code)]`, zero external production dependencies, public
APIs, and `x86_64-unknown-linux-gnu` release scope remain unchanged.

### Core interpreter (`crates/wtime-runtime`)
- **Opt-in lazy register tier (`--features register-executor`)**:
  - Contiguous dense-memory fast path for active memories `<= 16 MiB` (with
    fallible promotion and clean sparse demotion past `16 MiB`), bringing the 4
    kernel workloads to **3.92–4.61×** over the decoded tier and 8-page scatter/
    gather to **3.32×**, while lazily admitting `99.7%` of a multi-thousand-fn
    application.
  - Native O(1)-frame **`tail_call`** (`return_call`, `return_call_indirect`)
    reusing active frames in place with instruction-exact fuel parity,
    cooperative cancellation on infinite tail recursion, and clean host/fallback
    materialization, plus native **`wide-arithmetic`** (`i64.add128`,
    `i64.sub128`, `i64.mul_wide_s`, `i64.mul_wide_u`).

### Ahead-of-time compiler (`tools/wtime-aot`, standalone tool)
- **Wasm 3.0 backend features on BOTH `--backend x64` and `--backend aarch64`**:
  - Full **`relaxed_simd`** support (all 20 opcodes `0xfd 0x100..=0x113`,
    bit-exact with `wtime-runtime`).
  - Full **`tail_call`** support (`return_call`, `return_call_indirect`) with
    true O(1)-stack frame reuse (`jmp`/`b`/`br`) whenever `callee_arg_stack <=
    caller_param_stack`, overflow multi-value return-buffer forwarding, and
    clean growing-arity fallback.
- **Compile & incremental cache pipeline (`--backend x64`)**:
  - Compact `.delta` incremental sidecars (`WTX64DLT`, v1) avoiding full base
    `.bin` rewrites on small edits (`wait_inc` `4.46 ms -> 0.00 ms`, byte-exact
    `.so` output), O(1) epoch-stamped `def_stamp` in `optimize_ir`, filtered
    inlining strata, zero-alloc pair-lookup SCCs, and `Stage2Scratch` capacity
    capping after LPT giant functions.
  - On a **5.72 MiB, 4,449-function** SIMD application (16T): **134.0 ms** cold
    (**2.77×** faster than Wasmtime `49.0.3` at `371.6 ms`), **24.4 ms** on a
    1-function edit (**14.0×** faster than Wasmtime at `342.2 ms`), **8.39 ms**
    warm CLI cache hit, **0.030 ms** warm `.so` load (**61×** faster), and
    **16.2%** smaller artifact (`7.74 MiB` vs `9.23 MiB`).
- **Peak x64 & AArch64 codegen (strictly IEEE-754 bit-exact, zero FMA)**:
  - Sparse equality-filtered counted-loop narrowing, AVX2 3-tap blur + `f64x4`
    SLP, hoisted/deduplicated magic-constant loop division, bit-exact 2×-unrolled
    AVX2/SSE2/NEON `F64AxpyLoop` auto-vectorization (`matmul_f64` **40.62 µs**,
    **3.42×** faster than Wasmtime and **1.90×** faster than Rust `-O3`),
    backward cold-trap block sinking, and 1-branch wrap-proof `memory64` bounds
    checks.
  - `--backend aarch64` parity: multi-core builds, `a64-inc` incremental cache,
    standalone guard-page BCE + Ryū/WASI parity, `val_upper32_zero` redundant-
    `uxtw` elision, shifted-`ADD`/`SUB` small-constant multiply strength
    reduction, and cold-block sinking.
- **Expanded benchmarks**: added `matmul_f64` and `heapsort_i32` fixtures to
  the 5-engine suite (`tools/wtime-aot/bench.py`, **6.25×** 6-kernel geomean
  vs. Wasmtime `49.0.3`) and regenerated all SVG charts.

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
