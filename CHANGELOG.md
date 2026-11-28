# Changelog

All notable changes to `wtime` are documented in this file.

## [0.1.15] - 2026-11-28

`wtime-aot` GC-aware SSA optimization, live GC-ref register allocation across
bump-alloc safepoints, and `wtime-aot-host` generational copying GC +
`StoreSnapshot` binary heap snapshots. Core crates (`wtime-types`,
`wtime-decode`, `wtime-validate`, `wtime-runtime`) and `wtime-cli` remain
`#![forbid(unsafe_code)]` with zero external production dependencies, and
`tools/wtime-aot` remains `#![forbid(unsafe_code)]` with zero external
production dependencies.

### Ahead-of-time compiler (`tools/wtime-aot`)
- **GC-aware inlining, alias separation, and read-only GC GVN/CSE (`x64`)**:
  - Enabled inlining of Wasm-GC helpers (`StructGet`/`StructSet`/`StructNew`,
    `ArrayGet`/`ArraySet`/`ArrayLen`, `RefTest`/`RefCast`/`RefEq`/`I31*`).
  - Separated linear-memory and GC-heap alias classes so linear stores never
    clobber cached GC values and GC stores never clobber linear loads.
  - Added dominator-scoped GC GVN/CSE (`StructGet`/`ArrayLen`/immutable
    `ArrayGet`/`RefTest`/`RefEq`), `StructNew`/`StructSet` store-to-load
    forwarding, and redundant `RefCast`/`RefTest` elimination.
- **Register allocation of traced GC/Extern refs across bump-alloc safepoints (`x64` & `aarch64`)**:
  - Allowed traced GC/Extern refs live only across inline bump-allocation
    safepoints to live in callee-saved registers (`SafepointPlan::{gc_regs,
    ext_regs}`), spilling/reloading them exclusively in out-of-line cold GC
    stubs with zero fast-path overhead and clean OOM `rsp` restoration.
- **Generational card-marking write barriers (`x64` & `aarch64`, `STORE_ABI_VERSION = 2`)**:
  - Emitted inline Old-to-Young 512-byte card-marking write barriers on
    `StructSet`, `ArraySet`, `ArrayFill`, `ArrayCopy`, and `ArrayInitElem`
    against `StoreState::{gc_nursery_base, gc_arena_base, gc_card_table_base}`.

### Multi-module Store host runtime (`tools/wtime-aot/host`)
- **Generational Nursery + 512 B Card-Table Minor GC**:
  - Partitioned the 16 GiB virtual GC arena into `6.5 GiB` `OLD_A` + `6.5 GiB`
    `OLD_B` + `3 GiB` `NURSERY` + `26 MiB` `CARD_TABLE` (1 byte per 512 B Old
    card), supporting three concurrent 2 GiB arrays while preserving compact
    32-bit raw GC handles.
  - Added zero-syscall Young-Nursery Minor GC (`collect_minor_gc`) scanning
    only dirty Old cards via `old_card_first_obj` in 8-card `u64` chunks,
    preserving WeakMap/ephemeron/WeakRef semantics without copying Old Space
    or sweeping `extern_arena`, plus `Store::gc_minor_count()` /
    `Store::gc_major_count()`.
- **Binary `StoreSnapshot` (`capture_snapshot` / `restore_snapshot`)**:
  - Added shareable `StoreSnapshot` capturing compacted GC heap bytes,
    precomputed `gc_ref_offsets` and `func_ref_patches`, sparse non-zero 4 KiB
    linear/shared memory pages, instance `ctx_buf` tails, passive `elem_buf`s,
    dynamic tables, standalone globals, weak/ephemeron tables, and caller
    `OwnedRooted<EqRef>` roots, plus `Store::rebind_ephemeron` /
    `Store::rebind_weak_ref`.

## [0.1.13] - 2026-11-28

`wtime-aot` GC codegen & image-size optimizations plus `wtime-aot-host`
throughput, zero-copy mmap, and Wasmtime-parity release. Core crates
(`wtime-types`, `wtime-decode`, `wtime-validate`, `wtime-runtime`) and
`wtime-cli` remain `#![forbid(unsafe_code)]` with zero external production
dependencies, and `tools/wtime-aot` remains `#![forbid(unsafe_code)]` with zero
external production dependencies.

### Ahead-of-time compiler (`tools/wtime-aot`)
- **GC safepoint table deduplication (`x64` & `aarch64`)**:
  - Prefix-sharing sorted safepoint slot pool deduplication
    (`build_deduped_safepoint_slot_pool`), shrinking `.rodata` safepoint tables
    on large Wasm-GC images with zero ABI change.
- **x64 Wasm-GC codegen & `.text` shrinking**:
  - Moved slow `HELPER_GC_COLLECT` retry paths out-of-line into cold
    function-tail stubs (`ColdGcAllocStub`) so hot bump-allocation checks stay
    predicted fallthroughs.
  - Added static + RPO cross-block non-null analysis (`compute_gc_nonnull_info`)
    covering non-nullable params, globals, struct fields, array elements, call
    results, `br_on_null`/`br_on_non_null`, `i64.eqz` branch refinements, and
    dominating dereferences, plus zero-gap `sgp2_val` caching and fused
    single-use `(I32Eqz | I64Eqz | RefEq) + BranchIf` compare-branches.
  - Added 32-bit/narrow GC ops to `val_upper32_zero`, coalesced `>= 4`
    contiguous zero globals in `emit_store_init_instance` into `rep stosq`,
    dropped redundant `or $0` on struct headers, and reused zeroed scratch
    registers in `struct.new`/`array.new_fixed`.

### Multi-module Store host runtime (`tools/wtime-aot/host`)
- **Lock-free GC/type hot paths, warm semi-spaces, zero-copy mmap, and bulk UTF-16**:
  - Added lock-free canonical type/GC access, warm semi-space reuse across
    minor collections, zero-copy `WTMS0002` `mmap` deserialization, and bulk
    UTF-16 `ArrayRef` reads/writes.
- **Wasmtime API parity & idle RSS reclamation**:
  - Decoupled `OptLevel` from `Engine::precompile_compatibility_hash()` so
    `OptLevel::Speed` / `enable_compiler(false)` engines deserialize
    `OptLevel::SpeedAndSize` precompiled modules.
  - Fixed `Module::image_range()` to report the actual mapped `SoMapping::Mmap`
    `base .. base + image_len` range.
  - Added `Store::reclaim_inactive_gc_space(&mut self)` (`madvise(MADV_DONTNEED)`
    on the inactive GC semi-space) so embedders can reclaim physical RSS pages
    from idle isolates without disturbing live objects.

## [0.1.12] - 2026-11-27

Full AArch64 `StoreCdylib`, Wasm GC, Wasm Threads, 8 GiB GC semi-space, and
cross-architecture `wtime-aot-host` CI release. Core crates (`wtime-types`,
`wtime-decode`, `wtime-validate`, `wtime-runtime`) and `wtime-cli` remain
`#![forbid(unsafe_code)]` with zero external production dependencies, and
`tools/wtime-aot` remains `#![forbid(unsafe_code)]` with zero external
production dependencies.

### Ahead-of-time compiler (`tools/wtime-aot`)
- **Full AArch64 `StoreCdylib` + Wasm GC + Wasm Threads (`--backend aarch64`)**:
  - Exported `compile_wasm_to_aarch64_so` supporting `X64Target::StoreCdylib`
    with `wt_store_get_desc` (96-byte `StoreModuleDesc`), `WTST0001` metadata,
    sorted `SafepointHeader`/`SafepointEntry` `.rodata` tables, and unified
    `(FP, LR)` GC stack walking matching `x86_64`.
  - Lowered all Wasm GC instructions (`struct.*`, `array.*`, `i31.*`,
    `ref.test`/`ref.cast`, `ref.eq`, `any.convert_extern`/`extern.convert_any`),
    all Wasm Threads atomics (`dmb ish`, `ldar*`, `stlr*`, `ldaxr*`/`stlxr*`
    RMW/cmpxchg, `atomic.wait`/`notify`), and multi-module imports, exports,
    tables, memories, globals, tags, and exceptions on AArch64.
  - Preserved callee-saved `X19..X28` and full 128-bit `Q8..Q15` across
    deduplicated Store host trampolines and bumped AArch64 incremental cache
    `FORMAT_VERSION` to `16`.
  - Added AArch64 `StoreCdylib` codegen, `llvm-objdump` disassembly, and
    `A64Vm` verification tests.

### Multi-module Store host runtime (`tools/wtime-aot/host`)
- **Native AArch64 support**:
  - Dispatched `Module` native compilation per `target_arch` (`x86_64` /
    `aarch64`), included `std::env::consts::ARCH` in compatibility/version
    digests, enlarged `SigJmpBuf` (`#[repr(C, align(16))]` 320 bytes), and
    wired AArch64 `uc_mcontext.pc` + `SIGTRAP` fault handling.
- **8 GiB GC semi-space reserve (`>2 GiB` `ArrayRef` support)**:
  - Expanded each Cheney semi-space reserve from `1 GiB` to `8 GiB` (`16 GiB`
    `MAP_NORESERVE` arena) using `(off >> 2)` 32-bit raw handles (bit 31 flips
    across Space A ↔ Space B), `MADV_DONTNEED` on semi-space flip, and
    lazy-zero allocation for large zero-initialized arrays.
  - Added `large_byte_array_2gib_plus_16_survives_gc` integration test.

### CI (`.github/workflows/ci.yml`)
- Added `tools/wtime-aot/host` `cargo fmt --check`, `cargo clippy --locked --all-targets -- -D warnings`,
  and `cargo test --locked` to both `ubuntu-24.04` (`x86_64`) and
  `ubuntu-24.04-arm` (`aarch64`) CI runners, and fixed `tools/wtime-aot`
  formatting drift.

## [0.1.11] - 2026-11-27

Multi-module Wasm GC, Threads/atomics, and host runtime release for `wtime-aot`
(`--backend x64`) and the new standalone `wtime-aot-host` crate — enabling full
multi-module Wasm GC language runtimes (such as Scriptw) to compile ahead of
time and execute natively with zero LLVM, zero V8, and zero Cranelift. Core
crates (`wtime-types`, `wtime-decode`, `wtime-validate`, `wtime-runtime`) and
`wtime-cli` remain `#![forbid(unsafe_code)]` with zero external production
dependencies.

### Ahead-of-time compiler (`tools/wtime-aot`)
- **Wasm Threads & atomics on `--backend x64`**:
  - Added x64 lowering and native encoding for `atomic.fence`, atomic
    loads/stores, `rmw.add/sub/and/or/xor/xchg`, `cmpxchg`, and
    `memory.atomic.wait32/wait64/notify` via Store runtime callbacks.
- **Multi-module `StoreCdylib` ABI (`WTST0001`)**:
  - Added `X64Target::StoreCdylib` emitting shared `.so` images with a single
    `wt_store_get_desc` ELF descriptor, compact `WTST0001` binary metadata,
    cross-module `call`/`call_indirect`/`call_ref`, shared/private multi-memory,
    cross-module exception tags, and `wt_store_init_instance`.
  - Preserved callee-saved Wasm `Xmm8..Xmm13` and `MXCSR` across
    `emit_store_host_trampoline` on both normal and trap/exception return paths.
  - Raised `StoreCdylib`-only local/signature caps (`STORE_MAX_TOTAL_LOCALS`,
    `STORE_MAX_TOTAL_SIG_ARITY`) for large JS-to-WasmGC modules while keeping
    `BlotCdylib` and `StandaloneExec` validation limits unchanged.
- **Library API (`tools/wtime-aot/src/lib.rs`)**:
  - Exposed `wtime-aot` as a safe-Rust (`#![forbid(unsafe_code)]`) library
    crate alongside the `wtime-aot` CLI binary.

### Multi-module Store host runtime (`tools/wtime-aot/host`)
- Added standalone `wtime-aot-host` crate (excluded from the zero-dependency
  core workspace) implementing `Engine`, `Config`, `Module`, `Store`, `Linker`,
  `Instance`, `Memory`, `SharedMemory`, `Table`, `Global`, `Func`, `Tag`,
  `RootScope`, `Rooted`, `OwnedRooted`, and `InterruptHandle`.
- Implemented a precise Cheney semi-space copying garbage collector with Wasm
  safepoint stack walking across reentrant Host↔Wasm calls, `WeakAnyRef`,
  `GcEphemeron`, WeakMap reference tables, `ExternRef` finalization, and
  recoverable `GcHeapOutOfMemory` (`StoreLimits`).
- Added end-to-end native test suite (`tools/wtime-aot/host/tests/store_suite.rs`,
  9 tests covering multi-module linking, reentrant GC stack walking,
  weak/ephemeron chains, cross-module exceptions, shared memory atomics,
  recoverable GC OOM, preemptive `InterruptHandle`, and `Xmm8..Xmm13` C-ABI
  preservation).

## [0.1.10] - 2026-11-27

AOT vectorization, SIMD lowering, and binary-heap optimization release for
`wtime-aot` (`--backend x64` SSE2/AVX2/AVX-512 and `--backend aarch64` NEON):
pure nested-loop outer SIMD vectorization (`Op::SimdOuterLoop`), constant-immediate
SIMD lane shifts, `pmovzxbd`/`vpmovzxbd` zero-extension fusion with DOM-GVN
deduplication, wrap-safe variable-stride 4× counted-loop unrolling, and algebraic
binary-heap `sift_down` reassociation + hole-sift store sinking — sweeping all 12
benchmark kernels across both suites against Wasmtime `49.0.3`, Node V8 `v26.5.0`,
and Bun JSC `1.4.5` with 100% bit-exact output parity. Core crates remain
`#![forbid(unsafe_code)]` with zero external production dependencies.

### Ahead-of-time compiler (`tools/wtime-aot`)
- **Pure nested-loop outer SIMD vectorization (`Op::SimdOuterLoop`)**:
  - Added `vectorize_pure_outer_loops` (disabled under `--spectre-mitigations`)
    converting pure counted outer loops with early-exit `f32` inner loops into
    predicated multi-lane SIMD execution with exact partial-lane tail masking,
    horizontal `i32` wrapping accumulation, and dynamic-bound guard fallbacks.
  - Lowered `Op::SimdOuterLoop` natively across 16-wide AVX-512 (`zmm` + `k1`
    opmask), 8-wide AVX2 (`ymm`), 4-wide SSE2 (`xmm`), and 4-wide AArch64 NEON
    using NaN-exact quiet unordered FP comparisons (`vcmpnlt_uqps` / `vcmpnle_uqps`)
    and full `Xmm8..Xmm13` callee-save & `vzeroupper` accounting.
  - Remapped `Op::SimdOuterLoop` / `Op::SlpWideChain` `args: ValRange` across
    counted-loop BCE versioning passes, refreshed `def_op` across multi-loop
    rewrites, and tightened raw-`Val` escape checks.
- **Immediate SIMD shifts, `vpmovzxbd` fusion, & variable-stride 4× unrolling**:
  - Lowered constant-immediate SIMD shifts (`0xfd8b..=0xfd8d`, `0xfdab..=0xfdad`,
    `0xfdcb`, `0xfdcd`) to single immediate `psll*/psrl*/psra*` / `vpsll*/vpsrl*/vpsra*`
    instructions with exact modulo masking and shift-0 elision.
  - Added `V128_UN_I32X4_EXTEND_LOW_I8X16_U` (`pmovzxbd`/`vpmovzxbd` on x64,
    `2× uxtl` on AArch64), DOM-GVN `GvnKey::V128Un` deduplication, and 4-lane
    `i8x16.extract_lane_u` + `i32x4.splat/replace_lane` chain fusion.
  - Extended `unroll_counted_loops_4x` with wrap-safe 4-guard variable-stride
    `UnrollStep::VarInc(Val)` dynamic-stride unrolling.
- **Binary-heap `sift_down` reassociation & hole-sift store sinking**:
  - Added `optimize_sift_down_loops` rewriting BCE-versioned `i32`/`i64`
    max-heap and min-heap sift loops into a rotated 2-child hot loop (parallel
    `a[left]`/`a[left+1]` loads, speculative `2*left+1`/`2*left+3` indices,
    1 `cmp` + 3 `cmov`s, 1 store per level, cold 1-child leaf exit, and a
    0-swap-guarded sunk exit store with single-predecessor branch-fact checking).
- **Incremental cache & tests**:
  - Bumped incremental cache `FORMAT_VERSION` (`x64`: `19 -> 20`, `aarch64`:
    `14 -> 15`).
  - Added comprehensive IR, disassembly, native oracle, AArch64 NEON, and
    soundness-audit regression tests (`x64_final_stretch_p1_*`, `p2_*`, `p3_*`,
    `x64_final_stretch_audit_regressions`, and
    `aarch64_final_stretch_outer_simd_pmovzxbd_and_sift_down_parity`).

## [0.1.9] - 2026-11-27

Scalar codegen and loop-optimization release for `wtime-aot` (`--backend x64`,
with shared Stage 2 IR passes benefiting `aarch64`): inductive join/loop-header
`u32` bounds propagation, branchless 2-pointer merge-loop versioning &
paired-`Store` diamond if-conversion, x86-64 `ZF` flag reuse & pre-`cmp`
backedge move hoisting for macro-fusion, phi-edge peer coalescing, single-use
loop-constant rematerialization, and zero-gap spill-slot register forwarding.
Core crates remain `#![forbid(unsafe_code)]` with zero external production
dependencies.

### Ahead-of-time compiler (`tools/wtime-aot`)
- **Inductive join/loop-header BCE & branchless merge-loop if-conversion**:
  - Added `refine_join_param_bounds`, `unwrap_min_u_src` (requiring proven
    `<=u imm` fallback arms on `I32Imm` selects), and dominator-chain
    `extract_term_i32_cmp` inside DOM-GVN/BCE to prove non-circular `u32` upper
    bounds across multi-predecessor joins and loop headers.
  - Extended `if_convert_trivial_diamonds` to merge identical-address GP
    `Op::Store` diamonds into adjacent `CmpSelect`s + a single `Op::Store`
    sharing one `cmp` via `live_flags`.
  - Added `static_u32_upper_bound` and `if_convert_two_ptr_merge_loops` to
    version guarded 2-pointer merge loops (proving `k <= hi - 2` under a
    preheader `i_init <u mid && j_init <u hi` guard) into a branchless
    `cmp` + `cmov` + single `Op::Store` + macro-fused `jb` backedge fast loop
    while preserving the untouched slow loop as the exact fallback for
    adversarial/wrapping inputs.
- **x86-64 `ZF` flag reuse, pre-`cmp` macro-fusion, & zero-gap spill forwarding**:
  - Added `LiveFlags::IntZero` on flag-setting `add`/`sub`/`and`/`or`/`xor`
    instructions (never `imul` or `lea`), reused exclusively for `BranchIf` /
    `Eq 0` / `Ne 0` checks.
  - Added `will_reuse_int_flags` and `can_hoist_moves_before_cmp` to hoist
    non-aliasing backedge moves ahead of `cmp`/`test` for x86 `cmp/add/test +
    jcc` macro-fusion.
  - Added intra-block zero-gap spill-slot register forwarding (`just_spilled`,
    guarded by exact code-offset equality and `is64 || is_upper32_zero(v)`).
- **Register coalescing, constant rematerialization, & incremental cache**:
  - Added `add_phi_peer` phi-edge coalescing priority and single-use
    non-hoisted `ConstI32` rematerialization in `regalloc.rs`.
  - Bumped incremental cache `FORMAT_VERSION` (`x64`: `18 -> 19`, `aarch64`:
    `13 -> 14`) and added IR/oracle parity and soundness regression tests
    (`x64_scalar_gap_bce_merge_ifconv_and_flags_parity`,
    `x64_scalar_gap_audit_regressions`).

## [0.1.8] - 2026-11-27

SIMD, vectorization, loop optimization, and virtual-memory bounds-checking
performance release across both `wtime-aot` (native `x64` and `aarch64`) and
the safe-Rust `wtime-runtime` `register-executor`: EVEX/ZMM SLP widening,
`xmm16..xmm31` register expansion, hardware FMA, dynamic/relational/inclusive
software BCE, 4× counted loop unrolling, opt-in `--guard-pages` for
`--blot-cdylib` (`x64` and `aarch64`), and unboxed `[u64; 2]` v128 + Wasm 3.0
relaxed FMA execution in `register-executor`. Core crates remain
`#![forbid(unsafe_code)]` with zero external production dependencies.

### Ahead-of-time compiler (`tools/wtime-aot`)
- **AVX-512 (`x64-v4`) EVEX/ZMM SLP widening, `xmm16..xmm31`, hardware FMA, & fast SIMD lowering**:
  - Added `Op::SlpWideChain` (`slp_fuse_v128_wide`) widening adjacent 128-bit
    SIMD chains to 256-bit YMM (`x64-v3`) and 512-bit ZMM (`x64-v4`) with
    `k1` opmask NaN canonicalization, cold out-of-line 128-bit overlap
    fallbacks, hoisted `vzeroupper`, and exact-order NEON lowering on
    `aarch64`.
  - Unlocked `xmm16..xmm31` on `x64-v4` (AVX-512VL EVEX), lowered `v128` via
    3-address VEX/EVEX with memory-operand folding, broadcasts, single-insn
    `vcvtudq2ps`, and fast `i8x16.shuffle` paths, and emitted hardware FMA
    (`vfmadd231*`/`vfnmadd231*` on `x64`, `fmla`/`fmls` on `aarch64`) for
    Wasm 3.0 `relaxed_madd`/`relaxed_nmadd`.
- **Stage 2 pure-software BCE & 4× counted loop unrolling**:
  - Added `version_dynamic_counted_loops` dynamic loop-versioned software BCE
    (covering both strict `<`/`>` and inclusive `<=`/`>=` signed/unsigned bounds
    with wrap-safe preheader guards; disabled under `--spectre-mitigations`),
    dominating-guard relational `LtU`/`LeU` propagation and branch folding, and
    `Op::Select` `live_flags` reuse.
  - Added `Term::BranchIf` countdown loop rotation and 4× counted loop
    unrolling (`unroll_counted_loops_4x`) for exact constant multiples
    (`T >= 8 && T % 4 == 0`, inc/dec) and dynamic loops (`LtU`/`LtS` inc and
    `i -= 1 != 0` countdown) with single-step parallel escaping-value
    substitution.
- **Opt-in `--guard-pages` for `--blot-cdylib` (`--backend x64` and `--backend aarch64`)**:
  - Default `--blot-cdylib` builds remain signal-handler-free. Opt-in
    `--guard-pages` reserves `8 GiB + 64 KiB` `PROT_NONE` for `memory32`
    (including `(memory 0 0)`), emits `DT_FLAGS_1 = DF_1_NODELETE` and a
    64-byte writable `GuardState` tail in `PT_LOAD 2`, and installs a
    thread-safe one-time chained `SA_SIGINFO | SA_ONSTACK | SA_NODEFER`
    `SIGSEGV` handler scoped strictly to `[GuestCodeStart, GuestCodeEnd)`,
    per-invocation `Ctx` recovery slots, and `[mem_base, mem_base + 8 GiB + 64 KiB)`.
  - Preserves host callee-saved registers (`MXCSR`/`vzeroupper` on `x64`,
    `D8..D15` on `aarch64`) and pinned globals on trap unwind; saves/zeroes/restores
    guard slots across `HELPER_HOST_CALL` and nested `blot_aot_active_*`
    reentry; chains non-guest faults cleanly (including `si_code <= 0`
    software `SIGSEGV` under `SIG_IGN` and `SIG_DFL` via `tgkill`); fixes
    coalesced `Op::BoundsCheck` + DCE interaction (`bounds_check_covered_by_subsequent_mem_op`);
    rejects `--guard-pages` on `--backend c`/`rust` or combined with
    `--spectre-mitigations` (exit code `2`); and isolates build and incremental
    cache keys.

### Core libraries (`wtime-runtime`)
- **Unboxed `v128`, Wasm 3.0 relaxed FMA, dense `mem0` hoisting, & fused superinstructions (`register-executor`)**:
  - Unboxed hot `v128` SIMD operations and Wasm 3.0 relaxed FMA
    (`0xfa05..=0xfa08`) directly into `[u64; 2]` `RegOp` slots (plus fused
    two-rounding `F64x2MulAdd`, safe SWAR `i8x16`/`i16x8` arithmetic,
    `i64x2.mul`, `i32x4` cmp/shift, `bitmask`/`any_true`, and guarded
    `SimdExtract32`), preserving `#![forbid(unsafe_code)]`, zero external
    production dependencies, `size_of::<RegOp>() <= 12`, and exact fuel,
    cancellation, and NaN parity.
  - Hoisted dense `mem0` slice access inside `dispatch_inner` (with safe reload
    across grow/calls/instance switches), fixed sparse zero-length
    `memory.copy`/`memory.fill` OOB bounds checks at offset `65536`, added 10
    fused scalar superinstructions, and switched canonical Wasm 3.0 relaxed FMA
    to single-rounding `f32::mul_add` / `f64::mul_add`.

## [0.1.7] - 2026-11-26

Major execution-performance release across both `wtime-aot` (native `x64` and
`aarch64`) and the safe-Rust core runtime (`wtime-decode`, `wtime-validate`,
`wtime-runtime`): branchless Spectre SLH, value-range BCE, VEX memory folding,
dual global pinning, SSA loop live-range splitting & address rematerialization,
AVX-512 (`x86-64-v4`) EVEX encoding & general counted-loop auto-vectorization,
native `WTPGO1` profile-guided optimization (`--pgo-gen` / `--pgo-use`) with
speculative devirtualization & hot-first `.text` layout, cold module prep &
fast `register-executor` upgrades, and multi-pass soundness audit fixes. Core
crates remain `#![forbid(unsafe_code)]` with zero external production
dependencies.

### Ahead-of-time compiler (`tools/wtime-aot`)
- **Branchless Spectre v1 SLH, value-range BCE, VEX memory folding, dual global pinning, & AArch64 loop alignment**:
  - Upgraded `--spectre-mitigations` on `x64` (`cmov` mask), `aarch64`
    (`csel` mask), and `c` (branchless mask) to Speculative Load Hardening
    (SLH) on linear-memory bounds checks while preserving full barriers on
    control-flow/table dispatch and shared bulk helpers.
  - Added SSA value-range bounds-check elimination (BCE) for monotonic/bounded
    indices (`u32::from(u8/u16)` masks, shifts, proven bounds), x64 VEX/AVX2
    memory-operand folding for loads into FP/SIMD binary operations, dual hot
    Wasm global pinning (`R12` + `R13`) on `x64`, and 16-byte loop-header
    alignment on `aarch64`.
- **SSA loop live-range splitting & address rematerialization**:
  - Added SSA `Op::Copy` loop live-range splitting and `RematAddr` address
    rematerialization across both `--backend x64` and `--backend aarch64` to
    eliminate hot-loop register spills and redundant address reloads.
- **AVX-512 (`x86-64-v4`) EVEX backend & general counted-loop auto-vectorization**:
  - Added native EVEX (`512-bit` ZMM) instruction encoding and `--target-cpu`
    `x64-v4` / `native` AVX-512F+VL+BW+DQ dispatch on `x64` (with `vzeroupper`
    cleanup and compressed `disp8*64` addressing).
  - Added general counted-loop auto-vectorization lowering (`Op::VecElemLoop`
    for elementwise array/scalar kernels and `Op::VecReduceLoop` for sum and
    dot-product reductions) across `x64` (`v2`/`v3`/`v4`) and `aarch64` (NEON)
    with full trap-before-write & overlap/wrap guards.
- **Native `WTPGO1` profile-guided optimization (`--pgo-gen` / `--pgo-use`)**:
  - Added two-pass native PGO (`--pgo-gen <profile>` / `--pgo-use <profile>`)
    with binary `WTPGO1` profile emission and loading across `x64` and
    `aarch64`, bound into build and incremental cache keys.
  - Added per-callsite Boyer–Moore majority indirect-call target profiling,
    speculative `call_indirect` devirtualization + inlining of dominant direct
    targets (preserving signature checks and trap behavior), and hot-first
    `.text` function layout ordering.
- **AOT soundness & CI fixes**:
  - Hardened `VecElemLoop`/`VecReduceLoop` constant-fold, wrap-around, and
    aliasing checks; fixed `RematAddr` / `Copy` liveness & parallel-move
    coalescing edge cases; guarded `x64` EVEX GNU `objdump` disassembly unit
    check to `#[cfg(target_arch = "x86_64")]` for native ARM64 CI; and fixed
    PGO profile validation and indirect-call speculative guard emission.

### Core libraries (`wtime-decode`, `wtime-validate`, `wtime-runtime`)
- **Cold module decode, validation, & instantiation speedups**:
  - Reduced per-function allocation overhead in `wtime-decode` and
    `wtime-validate` with flat `local_runs` representation, lazy `ControlMap`
    construction, and a 64 MiB dense memory-initial-data cap (falling back
    cleanly without oversized eager allocations).
- **Experimental `register-executor` fast-tier upgrades** (opt-in, disabled by
  default):
  - Added pooled register banks to avoid per-call heap allocations, fused
    hot-loop superinstructions, `FastType::Ref` admission with full GC root
    scanning and survival across allocations/collections, and zero-frame
    `try_invoke_leaf` dispatch for leaf calls.
  - Hardened `register-executor` and validator bounds/resource checks,
    preserving exact Wasm semantics, fuel accounting, and cancellation
    contracts across all tiers.

## [0.1.6] - 2026-11-26

Multi-tenant & side-channel hardening sweep (all opt-in with zero overhead on
default local execution), full 236-opcode fixed-width Wasm SIMD128 support in
`wtime-aot --backend c`, and multi-round compiler/runtime soundness and
resource-cap hardening across `wtime-runtime`, `wtime-aot` (`x64`, `aarch64`,
`c`), and `wtime-aot-rt`. Core crates remain `#![forbid(unsafe_code)]` with
zero external production dependencies.

### Core libraries & CLI (`wtime-types`, `wtime-runtime`, `wtime-cli`)
- **Opt-in untrusted-worker profile**:
  - Added `FeatureSet::untrusted_worker()` (stable core with `threads: false`
    to disallow concurrent atomic counter-thread timers),
    `ResourceLimits::untrusted_worker()` (16 MiB module cap, 64 MiB memory,
    32 MiB GC, 16 MiB continuations, 65,536 tables/objects, 1,024 call depth,
    16,384 stack height, 10M default fuel, `compile_threads = 1`),
    `Config::untrusted_worker()`, and `wtime-cli --profile untrusted-worker`
    (preserving explicit `--fuel` overrides).
  - Documented explicit architectural-vs-OS isolation boundaries on `Config`
    and `Store` and added [`docs/architecture/security-model.md`](docs/architecture/security-model.md).
- **GC, continuation, and `register-executor` resource-cap hardening**:
  - Borrowed `&SubType` in `gc_type` instead of cloning `StructType::fields`
    per struct access; eliminated `max_gc_bytes` double-charging and temporary
    allocations in in-place `array.init_data`/`array.init_elem`, using fallible
    `try_reserve_exact` and validated zero-alloc `copy_fields` for `array.copy`.
  - Filtered continuation capture/bind/install root vectors to traceable
    reference values only, pre-checked GC object and byte budgets before
    allocating, and switched to fallible reservations.
  - Bounded `register-executor` fast-tier per-function `BlockMeta`,
    `stack_types`, `static_labels`, and `CompileLabel` prefix copies with
    fallible reservations (cleanly declining to `Decoded` on limit) and
    declined uncached multi-instruction bodies when zero fuel remains.

### Ahead-of-time compiler (`tools/wtime-aot`) & runtime (`tools/wtime-aot/rt`)
- **Full 236-opcode fixed-width Wasm SIMD128 in `--backend c`**:
  - Implemented all remaining 227 standard Wasm SIMD128 opcodes (including
    `f32x4.convert_i32x4_u` / `0xfdfb`) with packed vector helpers,
    rounding-mode isolation, and interpreter-oracle parity across all 236
    opcodes.
- **Opt-in clock policies (`--no-clock`, `--clock-coarsen-ns N`)**:
  - Added `--no-clock` and `--clock-coarsen-ns N` across standalone `x64`,
    standalone `aarch64`, and Goose (`wtime-aot-rt`, `ClockPolicy` /
    `Policy::clock`): returns WASI `EACCES` (`2`) / `0` when denied and
    truncates timestamps to multiples of `N` ns when coarsened.
  - Standalone `x64`/`aarch64` ELFs omit `clock_gettime` syscalls completely
    under `--no-clock` when `--timing-json` is absent, and Goose skips internal
    `Instant::now()` / `SystemTime::now()` calls when `--timing-json` and
    `--time-limit` are absent.
- **Opt-in Spectre v1 hardening (`--spectre-mitigations`)**:
  - Added `--spectre-mitigations` to `wtime-aot` and `wtime-aot build`, bound
    into all top-level build and incremental cache keys (`x64-inc` v12,
    `a64-inc` v7) and fail-closed on `--backend rust`.
  - Emits `lfence` (`x64`), `dsb nsh; isb` (`aarch64`), and `WT_SPEC_BARRIER()`
    (`c`) after linear memory, extra-memory, `memory64`, table, `call_indirect`,
    `call_ref`, `br_table`, and all 9 shared native bulk memory/table helper
    bounds checks (`memory.copy`/`fill`/`init`, extra-memory `copy`/`fill`,
    `table.init`/`copy`/`grow`/`fill`), disabling guard-page elision and Stage 2
    loop BCE/vectorization.
  - Scrubs committed linear memory, extra memories, active dynamic tables, used
    exception-slab bytes, and `Ctx` (followed by a barrier) before `munmap`/`free`
    in `blot_aot_drop` and failing `blot_aot_new` cleanup across `x64`,
    `aarch64`, and `c`.
  - Published exact Linux `seccomp-bpf` syscall tables and measured native
    `x86_64` Spectre overhead benchmarks in `docs/architecture/security-model.md`
    and `docs/wtime-aot/x64-abi.md`.
- **Multi-round AOT & Goose sandbox hardening**:
  - `x64`: handled `CallCatch`/`CallIndirectCatch` effects in LICM, GVN/BCE,
    and load forwarding; unified multi-memory growth tracking; fixed extra-memory
    `Op::StoreMem` spilled/rematerialized pointer double-basing by `R14`; fixed
    SIMD spill-aliasing (`prep_bin_swap`, `emit_ucmp`, `i8x16.lt_u`, `extmul`,
    `i64x2.mul`); guarded `memory64` page shifts, strict `u64` parsing, and
    `readlinkat` truncation.
  - `aarch64` & `c`: bounded AArch64 `regalloc` backward liveness memory/work
    with `CompileBudget` LIFO reservations; preserved store/table-set address
    scratch and `AddScaled` operands across large spills; fixed `v128.bitselect`,
    `pmin`/`pmax`, and shuffle spill-aliasing; rolled back all mappings on
    `blot_aot_new` failure; enforced `!in_call` in C Blot ABI 8; hardened cache
    reads and staged build cleanup.
  - `wtime-aot-rt` Goose host: charged `goose.write` output limits before
    reading guest memory with fallible reservations, bounded stdin reads with
    EINTR-safe `INPUT_LIMIT`, and hardened file opens via verified parent-dirfd
    paths (`O_NOFOLLOW|O_NONBLOCK|O_NOCTTY`) and post-open allowlist checks.

## [0.1.5] - 2026-11-25

Wasm 3.0 `table64` & `function-references` (`call_ref`, `return_call_ref`,
`ref.as_non_null`, `br_on_null`, `br_on_non_null`, explicit table initializers,
typed concrete function references) across BOTH `--backend x64` and
`--backend aarch64`, major branchy-loop & register-allocation codegen wins
(`heapsort_i32` **1.54×** faster at **`32.29 µs`**, now beating Wasmtime `49.0.3`
on all 6 of 6 benchmark kernels even with explicit software bounds checks),
full AArch64 regalloc/trace-layout/prologue parity, native ARM64 CI fixes, and
automated dual-arch Linux `x86_64` + `aarch64` GitHub Release publishing. Core
crate `#![forbid(unsafe_code)]`, zero external production dependencies, and
public APIs remain unchanged.

### Ahead-of-time compiler (`tools/wtime-aot`, standalone tool)
- **Wasm 3.0 `table64` and `function-references` on BOTH `--backend x64` and `--backend aarch64`**:
  - Enabled `function_references` in the shared AOT validation profile and
    admitted typed concrete function references `(ref $t)` / `(ref null $t)`,
    non-nullable `funcref`/`externref`/`exnref`, explicit table initializers
    (`0x40 0x00`), and 64-bit `table64` limits (`0x04`/`0x05`) across static
    and dynamic tables, active element segments (`i64.const` offsets), globals,
    locals, params, and block signatures.
  - Lowered `call_ref` (`0x14`), `return_call_ref` (`0x15`, O(1) tail-call
    frame reuse), `call_ref` inside `try_table` (preserving `catch`/`catch_all`
    unwind while trapping `TRAP_INDIRECT_CALL` on null without catching),
    `ref.as_non_null` (`0xd4`), `br_on_null` (`0xd5`), `br_on_non_null`
    (`0xd6`), and full 64-bit `table64` indexing/bounds checks (`call_indirect`,
    `return_call_indirect`, `table.get`/`set`/`size`/`grow`/`fill`/`init`/
    `copy` including 64↔32 mixed table copies) with zero new IR op variants.
  - Bumped incremental cache versions (`x64-inc` v10, `a64-inc` v4→v5) and
    added a 17-case differential oracle (`oracle_wasm3_table64_and_function_references`
    + `a64_wasm3_table64_funcref` + native ARM64 `ET_EXEC` check in
    `tests/aarch64_native.rs`) verifying high-32-bit `0x1_0000_0000` OOB traps,
    100,000-deep `return_call_ref` tail recursion, and `call_ref` + `try_table`.
- **Branchy-loop & Goose codegen sweep (`--backend x64` & shared SSA IR)**:
  - Extended `Op::AddScaled` with signed `disp: i32` (folding `(i << 1) + 1`
    into a single 1-cycle `lea` / AArch64 shifted `add`+`add`), raised inline
    `MemCopyConst`/`MemFillConst` cap from `64 -> 128` bytes with constant-zero
    AVX2/SSE2/NEON fast paths, upgraded `forward_loop_carried_scaled_loads` to
    split `Term::CmpBranch` preheaders, added late `thread_empty_branch_trampolines`
    + 1-hop `resolve_merge` diamond linearization, and added vote-counted peer
    coalescing with `fv`-first `CmpSelect` peer priority.
  - Added Goose slice-unpack/wrap folding, redundant no-op `GlobalSet` elision,
    narrow-store mask elision, nested mask folding, and `share_const_div_rem`.
  - Cut `heapsort_i32` from `49.0 µs` to **`32.29 µs`** (**`1.64×`** faster
    than Rust `-O3` at `52.92 µs`, **`1.69×`** faster than Clang `-O2` at
    `54.68 µs`, and beating Wasmtime `49.0.3` at `32.43 µs` — winning **6/6**
    kernels vs. Wasmtime at **`6.69×`** geomean, **`5.18×`** vs. V8,
    **`6.50×`** vs. Clang `-O2`, **`3.67×`** vs. Rust `-O3`) and swept all 8
    standalone Goose `suite2` workloads over Rust `-O3`.
  - On the `5.91 MiB / 4,523`-function `gdev` SIMD app: **`151.3 ms`** cold
    compile vs. Wasmtime `377.1 ms` (**`2.49×`**), **`23.2 ms`** 1-fn edit vs.
    `369.7 ms` (**`15.9×`**), **`0.034 ms`** warm `.so` load vs. `1.84 ms`
    (**`54×`**), **`8.31 ms`** warm CLI cache hit, and **`15.4%`** smaller
    (`8.03 MiB` vs. `9.49 MiB`).
- **AArch64 regalloc, layout, prologue, & full 18-export Blot ABI 8 parity (`tools/wtime-aot/src/aarch64`)**:
  - Added `A64Loc::Remat` GP constant rematerialization, 2-pass hot-fallthrough
    trace block layout with triangle-`if` diamond linearization and cold trap
    sinking, loop-depth-weighted spill eviction, ABI register hints +
    vote-counted coalescing peer hints, `Term::BranchIf` (`cbz`) and
    `Term::CmpBranch` (`b.cond` invert) fallthrough inversion, constant-zero
    NEON `movi` `MemFillConst` fast path, and zero-frame leaf prologue
    stack-check elision with prospective `frame_size` stack checking.
  - Aligned `--backend aarch64 --blot-cdylib` 1:1 with the Blot ABI 8 Library
    Protocol v1 contract (`blot_aot_abi_version() == 1`, `mem_limit_pages` in
    pages, full `HELPER_HOST_CALL` + import trampolines with `Q8..Q15`
    preservation and `ActiveCall*` callback reentry, plus all 18 exports
    including `import_name`, `call_allocate`/`reset`/`finish_invocation`,
    `mem_data`, `mem_grow`, `get_global`/`set_global`, and `active_*`).
- **CI & automated dual-arch GitHub Releases**:
  - Fixed ARM64 test compilation (`x64/tests_oracle.rs`, `x64/runtime.rs`,
    `cgen.rs`), ARM64 C backend `-mcpu=` CPU selection (avoiding Clang's
    `-march=native` rejection on AArch64), x64 `abi18` test driver pure-SysV
    assembly stubs (fixing red-zone corruption under Clang without default
    `-fstack-protector`), native ARM64 Blot ABI 8 integration harness, and
    `conformance` checkout `fetch-depth: 0` in `.github/workflows/ci.yml`
    (`fail-fast: false` on `wtime-aot` matrix).
  - Added `.github/workflows/release.yml` to build, strip, checksum
    (`SHA256SUMS.txt`), and publish both `x86_64-unknown-linux-gnu` and
    `aarch64-unknown-linux-gnu` tarballs for `wtime-aot` and `wtime-cli` on
    `v*` tags.

## [0.1.4] - 2026-11-25

Full passive & declarative bulk memory/table support (`memory.init`, `data.drop`,
`table.init`, `elem.drop`, `table.copy`, `table.grow`, `table.size`, `table.fill`,
`ref.func`, dynamic `funcref`/`externref` tables & dynamic `call_indirect`),
standalone Goose compiler OS/search sandbox imports (`env::gs_os_*`, `env::gs_find_*`,
`openat2` + `RESOLVE_BENEATH` + `O_NOFOLLOW`), Euclidean power-of-two remainder
folding & constant-trip loop unrolling, cold-compile pipeline speedups, native
ARM64 CI verification (`ubuntu-24.04-arm`), and hardened `register-executor`
admission & differential fuzzing. Core crate `#![forbid(unsafe_code)]`, zero
external production dependencies, public APIs, and `x86_64-unknown-linux-gnu`
release scope remain unchanged.

### Core interpreter (`crates/wtime-runtime`)
- **Opt-in lazy register tier (`--features register-executor`)**:
  - Raised per-function admission ceilings (`MAX_FN_INSNS = 262_144`,
    `MAX_FN_SLOTS = 32_768`) while preserving exact `u16` slot packing and
    runtime `max_total_locals` / `max_stack_height` limits, raising lazy
    admission on the 4,513-function SIMD app (`gdev`) to **99.98%**
    (`4,512 / 4,513` functions).
  - Added native fast-register lowering for `memory.init` (`0xfc08`),
    `data.drop` (`0xfc09`), `table.init` (`0xfc0c`), `elem.drop` (`0xfc0d`),
    `table.copy` (`0xfc0e`), and `table.size` (`0xfc10`) while keeping
    `size_of::<RegOp>() <= 12`.
  - Added a 60-seed deterministic differential fuzzer (`fast_register_hardened_features_differential_fuzzer`)
    covering `exception_handling` (`try_table`, `throw`, `catch`, `catch_all`),
    `multi-memory` + `memory64`, passive `memory.init`/`data.drop`, `tail_call`
    (`return_call`/`return_call_indirect`), bulk table ops (`table.size`/
    `table.init`/`elem.drop`/`table.copy`/`table.grow`/`table.fill`), and
    5,000-local functions across 5 fuel budgets verified bit-exact against `Decoded`.

### Ahead-of-time compiler (`tools/wtime-aot`, standalone tool)
- **Passive/declarative bulk memory & table ops on BOTH `--backend x64` and `--backend aarch64`**:
  - Full support for `memory.init` (`0xfc08`), `data.drop` (`0xfc09`),
    `table.init` (`0xfc0c`), `elem.drop` (`0xfc0d`), `table.copy` (`0xfc0e`),
    `table.grow` (`0xfc0f`), `table.size` (`0xfc10`), `table.fill` (`0xfc11`),
    `ref.func` (`0xd2`), `ref.null func` (`0xd0 0x70`), `funcref` block/value/global
    types, growable `funcref`/`externref` tables (`<= 131_072` slots/table,
    demand-paged `mmap` backing), and dynamic `call_indirect` /
    `return_call_indirect` / `call_indirect` inside `try_table` — with **zero
    overhead** on static modules (static `funcref` tables stay in `.rodata`
    `ci_tables`, fixed `externref` tables stay inline in `Ctx`, and unused bulk
    helpers emit 1-byte `ret` stubs).
- **Standalone self-hosting Goose compiler imports & sandbox on BOTH `--backend x64` and `--backend aarch64`**:
  - Added direct-syscall `env::gs_os_{arg_count,arg,getenv,file_exists,is_dir,
    read_file,write_file,append_file,resource_dir}` and `env::gs_find_{any,pair}`
    imports, `goose.reserve` host-to-guest allocation callbacks with re-entrancy
    guards, 256 GiB `MAP_NORESERVE` default cap for standalone `memory64`, and
    sandbox CLI flags (`--allow-files`, `--allow-env`, `--output-limit`,
    `--memory-limit-pages`, `--timing-json`, `--time-limit`, `--stack-limit-mib`,
    `--`) enforced via Linux `openat2` (`RESOLVE_BENEATH | O_NOFOLLOW`).
  - Compiles and executes the real 5.38 MiB `memory64` self-hosting Goose
    compiler (`compiler-64.wasm`) with byte-for-byte identical Wasm output on
    both `examples/sorting.goose` and self-compiling `selfhost/main.goose`
    (with `0` `<unknown>` AArch64 encodings under `llvm-objdump`).
- **Loop codegen & cold-compile pipeline**:
  - Added signed Euclidean power-of-two remainder folding (`r = v rem_s 2^k;
    (r < 0) ? r + 2^k : r` -> `v & (2^k - 1)`), zero-allocation epoch-stamped
    `bool01 & odd_mask` elision, immediate `Loc::Remat` folding in
    `emit_int_cmp_flags_reusing`, constant `Term::CmpBranch` folding, and
    bounded 2-round innermost constant-trip (`1..=4`) loop unrolling with
    escaping SSA value rewrite — cutting `suite2/life.wasm` by **>2×**
    (`6.59 ms` -> **`3.08 ms`** median / `2.47 ms` uncontended, beating Rust `-O3`)
    and raising the 6-kernel `--backend x64` geomean speedup over Wasmtime
    `49.0.3` to **`7.18×`** (**`5.36×`** vs. V8, **`6.15×`** vs. Clang `-O2`,
    **`3.70×`** vs. Rust `-O3`, beating Rust `-O3` on all 6 kernels).
  - Eliminated per-function heap allocations in `fold_euclidean_pow2_rem` and
    `unroll_small_constant_loops`, added instruction-weighted chunking for
    parallel admission, and tuned parallel validation threads — cutting cold
    compile on the 5.90 MiB / 4,513-function `gdev` app to **`123.5 ms`**
    uncontended (`263.7 ms` vs. Wasmtime `609.5 ms` = **`2.31×`** in paired
    5-round `bench.py`, **`34.2 ms`** vs. `556.4 ms` = **`16.3×`** on 1-fn edit,
    **`0.031 ms`** vs. `1.78 ms` = **`57×`** warm `.so` load, **`17.1%`** smaller).
- **Native ARM64 CI verification**:
  - Gated x86-64-only native `dlopen` oracle/fault tests on `target_arch = "x86_64"`,
    added `tools/wtime-aot/tests/aarch64_native.rs` covering native ARM64 Blot
    ABI 8 `dlopen` + standalone static `ET_EXEC` WASI/Goose/sandbox execution,
    and expanded `.github/workflows/ci.yml`'s `wtime-aot` job to the
    `[ubuntu-24.04, ubuntu-24.04-arm]` matrix with `WTIME_AOT_REQUIRE_NATIVE_TESTS=1`.

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
