# Continuation Plan: Pure-Rust Wasm → x86-64 Native Compiler (`wtime-aot --backend x64`)

## 1. Verified Baseline & Motivation

### `/home/jacobt/src/wtime` (`main`)
- **Goose `sudoku` regression fixed & verified (`9b25bfd`)**: hybrid `m32::direct`/`direct_mut` in `tools/wtime-aot/rt/src/lib.rs`, `RetAbi::Result` for single-`i32` returns in `tools/wtime-aot/src/codegen.rs`, and `32×` faster `Module::new` global analysis (`2,730 ms -> 85 ms`).
  - Paired CPU-pinned check vs `037d32c` (`aot-final`, 9 rounds, exact stdout): `sudoku` `4.27 ms` vs `4.26 ms` (`1.002×`), `life` `6.59 ms` vs `6.52 ms` (`1.012×`), `sorting` `88.94 ms` vs `88.61 ms` (`1.004×`).
- **Fast C99/Clang backend (`--backend c --c-opt auto`) complete & verified (`9b25bfd`)**:
  - `opt-palm` (`1,507,839 B`, 2,760 defined fns, fixed 753-page memory, SHA-256 `c958c20386207b3ea9713d5aae596edc264328ceaa87281b5cf6cc333db2b89b`, Ryzen 7 3800X 8C/16T): **6.27 s** cold (down from `387 s` Rust `-O3 ThinLTO`), **0.81 s** incremental (1 shard + relink), **0.86 ms** warm cached load.
  - `gdev-dev` (`3,281,226 B`, 4,700+ fns, SHA-256 `25ebb19d8cb6adc2fc6dd9545e647baa88c29e0e021d7d274a511b4c6612f77b`): **8.47 s** cold.
  - `opt-palm` `cpu-bench` (100% SHA-256 parity across all 5 workloads):
    - `title`: `0.0555 ms` vs Wasmtime `0.0635 ms` (**1.14×**)
    - `colony`: `0.9710 ms` vs Wasmtime `1.1709 ms` (**1.21×**)
    - `maxed-carrying`: `3.2347 ms` vs Wasmtime `3.7069 ms` (**1.15×**, p95 **1.38×**)
    - `maxed-day-hitting`: `1.9006 ms` vs Wasmtime `2.5234 ms` (**1.33×**, p95 **2.15×**)
    - `piles-2000`: `0.9902 ms` vs Wasmtime `1.1773 ms` (**1.19×**)
  - **Why Clang hits a ~6.3 s cold floor**: `clang -O0` alone takes `1.80 s` (`colony` `18.03 ms`); `clang -O1` (`7.17 s`) is slower than outlined `-O2` (`6.27 s`) while running slower (`1.32 ms` vs `0.97 ms`). Our frontend + `ld.lld` takes ~220 ms; 95% (`5.93 s`) is inside `clang`.

### `/home/jacobt/src/pen`
- `pen run` (and direct unpacked `--game` runs) default to `wtime-aot --backend c --c-opt auto` via `GuestModule::new_run` / `load_run_module` in `crates/blot-host/src/{wtime_aot.rs,lib.rs}` and `crates/pen/src/main.rs`.
- `pen dev`, `test`, `bench`, `fuzz`, `diverge`, mods, and unit tests stay on Wasmtime/Cranelift by default (preserving epoch deadlines and stack sampling).
- Verified: `blot-host` tests (30/30 default + 5/5 core runtime tests under `PEN_WTIME_AOT=1`), headless `opt-palm` colony 300 frames (`281 fps` vs `230 fps` Wasmtime, warm ready `1 ms` vs `12 ms`, exact `frames.trace` SHA-256 `6d550c35...`), headless `pen run --project /home/jacobt/src/gdev --frames 30` (cold `8,123 ms`, warm `3 ms` vs `47 ms`, exact SHA-256 `c2aadc5f...`).

---

## 2. Goal, Scope & Explicit Stage 1 Admission Contract (`--backend x64`)

Build a self-contained pure-Rust (`#![forbid(unsafe_code)]`) Wasm → Linux x86-64 ELF64 `.so` compiler inside `tools/wtime-aot/src/x64/`:
- **Dependencies**: the compiler crate (`tools/wtime-aot`) depends only on in-tree `wtime-validate` and `wtime-types` (zero third-party crates, zero `clang`/`rustc`/`ld.lld` subprocesses); the **generated `.so`** has zero `DT_NEEDED`, zero dynamic relocations, and zero `libc` dependency (uses direct Linux x86-64 `syscall` for `mmap`/`mprotect`/`munmap`).
- **Metering & safety policy**: `tools/wtime-aot` is an explicit opt-in AOT tool isolated outside the production safe-Rust core (`crates/*`, which remains metered with fuel and cancellation). `--backend x64` executes unmetered for `pen run` builds. In Pen (`crates/blot-host/src/lib.rs`), **any** engine with `engine.get_epoch_interruption() == true` (`dev_engine()`, `ticking(...)`, or externally configured epoch engines) stays unconditionally on Wasmtime even when `PEN_WTIME_AOT=1` (lock-free, zero deadlock risk), and `Guest::set_deadline` panics explicitly if called on an `AotGuest` rather than silently ignoring the request.

### 2.1 Target ISA Policy & Runtime CPUID Gate
- **Baseline required ISA (always emitted)**: Linux x86-64 + SSE2 + SSE4.1 (`roundss`/`roundsd`, `pextrq`/`pinsrq`) + POPCNT.
- **Optional ISA (`TargetIsa { has_abm_bmi1: bool }`)**:
  - Threaded explicitly into `lower::lower_ir_to_x64`.
  - When `has_abm_bmi1 == true`: `i32/i64.{clz,ctz}` emit single-instruction `lzcnt`/`tzcnt`.
  - When `has_abm_bmi1 == false`: `lower.rs` emits branchless portable `bsr`/`bsf` + `cmovz` (`63^31=32` / `127^63=64` for `clz`, `32`/`64` for `ctz`).
- **Single canonical CPU-to-profile mapping** (shared identically between `tools/wtime-aot/src/build.rs` and Pen `crates/blot-host/src/wtime_aot.rs::resolved_x64_isa_tag`; note `x64-v2` and `x64-v3` are **internal compiler feature profiles**, not full psABI ISA levels):
  - `native` (default): `#[cfg(target_arch = "x86_64")]` checks `std::is_x86_feature_detected!("lzcnt") && std::is_x86_feature_detected!("bmi1")` -> `x64-v3` (`has_abm_bmi1 = true`) else `x64-v2` (`has_abm_bmi1 = false`); non-`x86_64` hosts return a clean error.
  - `x86-64-v2` | `generic`: `x64-v2` (`has_abm_bmi1 = false`).
  - `x86-64-v3` | `x86-64-v4` | `znver*` | `haswell` | `broadwell` | `skylake` | `skylake-avx512` | `cascadelake` | `cooperlake` | `cannonlake` | `icelake-client` | `icelake-server` | `tigerlake` | `rocketlake` | `alderlake` | `raptorlake` | `sapphirerapids` | `emeraldrapids` | `graniterapids`: `x64-v3` (`has_abm_bmi1 = true`).
  - Any other `--target-cpu` / `PEN_WTIME_AOT_TARGET_CPU` value: **rejected with an explicit error** (never silently mapped to a raw string).
  - **Cache key**: `build.rs` and Pen's fast cache key artifacts by the **resolved internal profile** (`x64-v2` vs `x64-v3`), never raw `"native"`.
- **Mandatory `blot_aot_new` CPUID check** (executed on host stack AFTER saving SysV callee-saved `Rbx` AND live entry arguments `Rdi, Rsi, Rdx, Rcx`):
  1. `eax = 0; cpuid` -> require `eax >= 1`; if `has_abm_bmi1`, also require `eax >= 7`.
  2. `eax = 1; cpuid` -> require `ECX` bit 19 (SSE4.1) AND bit 23 (POPCNT).
  3. If `has_abm_bmi1`:
     - `eax = 0x8000_0000; cpuid` -> require `eax >= 0x8000_0001`.
     - `eax = 0x8000_0001; cpuid` -> require `ECX` bit 5 (ABM / LZCNT).
     - `eax = 7; ecx = 0; cpuid` -> require `EBX` bit 3 (BMI1 / TZCNT).
  4. On any missing feature: restore `Rbx`/callee-saved regs, write `*trap_out = TRAP_UNSUPPORTED_ISA (22)` (if non-null), and return `NULL`; `blot_aot_trap_message(22)` returns `"Unsupported CPU ISA"`.

### 2.2 Explicit Admission Contract (Enforced in `x64/module.rs::admit_x64_module`)
All Wasm bytes pass `wtime_validate::validate` first; `admit_x64_module` then enforces ALL constraints **before** any IR or codegen (including scanning **unreachable** instructions in every body):
- **Target**: `--blot-cdylib` only in Stage 1.
- **Type section**: plain final function types only (`!group.explicit`, `CompositeType::Func`, `st.final_ == true`, `st.supertype.is_none()`, `st.descriptor.is_none()`, `st.describes.is_none()`). Rejects GC recursive groups, structs/arrays/conts, subtyping, non-final `(sub (func))` types, and descriptors so structural `FuncType` equality matches exact Wasm canonical type identity.
- **Memories**: `0` or `1` Wasm32 memory (`IndexType::I32`, non-shared, standard 64 KiB pages `page_size_log2 == 16`, `min <= max`, omitted `max` defaults to `65_536`, `max <= 65_536`). Memory64, multi-memory, shared memory, custom page sizes rejected.
- **Tables**:
  - Static 32-bit `funcref` tables (`element == nullable funcref`, explicit `table_inits` allowed ONLY if matching `ref.null func`, per-table `limits.min <= 1_048_576`, aggregate funcref slots `<= 2_097_152` checked BEFORE allocation, initialized only by active element segments with constant `i32` offsets and `ref.func`/`ref.null` entries).
  - Mutable 32-bit `externref` tables (`element == nullable externref`, explicit `table_inits` allowed ONLY if matching `ref.null extern`, `min == max <= 65_536`, aggregate extern slots `<= 65_536`, accessed only by `table.get`/`table.set`, no active element segments).
  - Canonicalized `(table, FuncType)` indirect-call dispatch tables with **aggregate** `.rodata` cap `sum(ci.span * 4) <= 64 MiB`.
  - Passive/declarative element segments, non-null table initializers, and table bulk/grow ops rejected.
- **Data segments**: active memory-0 segments with constant `i32` offsets only; passive segments rejected.
- **Imports**: plain function imports only (`ImportKind::Func`; `ImportKind::ExactFunc` and imported memories/tables/globals/tags rejected), with Blot ABI-compatible signatures (`()`, `(ref)`, `(i32)`, `(f32)`, `(ref, i32|i64|f32|f64|ref)` -> `()` or single `i32|i64|f32|f64|ref`; Pen's `validate_aot_wasm` further enforces exact `blot:host/1::call_<param>_<res>` Wasm signatures).
- **Globals**: up to `MAX_GLOBALS = 65_536` globals of `i32`, `i64`, `f32`, `f64`, `v128`, or nullable `externref` with constant initializers (`const`, `v128.const`, `ref.null`, prior `global.get`, `i32`/`i64` `add`/`sub`/`mul`).
  - **Folding rule**: a global is folded into `global_consts[k] = Some(X64ConstVal)` iff it is immutable OR (`writes == 0 && !exported_mutable`). **Exported mutable globals are NEVER folded** so `blot_aot_set_global` / `Guest::restore` always work. Every global with `global_consts[k].is_none()` receives a checked `Ctx` storage slot.
- **Exports**:
  - Functions (all admitted; `blot_aot_call_export` dispatches any exported function — including re-exported imports — whose signature has `0..=1` scalar/ref param and `0..=1` scalar/ref result; `blot:allocate`/`blot:reset` require `(i32)->(i32)` and `blot:finish_invocation` requires `()->(i32)`).
  - Globals (all admitted, including folded constants and folded `RefNull`).
  - Memory 0 (admitted — required for Pen's `blot:memory` export on array guests such as `opt-palm`).
  - Exported tables/tags rejected.
- **Value types, block types, & opcodes** (checked across EVERY body including unreachable code):
  - Value types: `I32`, `I64`, `F32`, `F64`, `V128`, nullable `ExternRef`. (`funcref` values/blocks are **rejected**, including `Immediate::Block(-16)` and `ref.null func` even in dead code.)
  - Block types: `-64` (empty), `-1` (`i32`), `-2` (`i64`), `-3` (`f32`), `-4` (`f64`), `-5` (`v128`), `-17` (nullable `externref`), `Immediate::BlockType(vt)` (checked via `check_val_type`), or valid function type index `>= 0`.
  - Opcodes: MVP (`0x00..=0x05`, `0x0b..=0x11`, `0x1a..=0x1c`, `0x20..=0x26`, `0x28..=0x44`, `0x45..=0xc4`, `0xd0` with `HeapType::Extern`, `0xd1`), saturating truncs (`0xfc00..=0xfc07`), memory-0 `memory.copy`/`memory.fill` (`0xfc0a`/`0xfc0b`), minimal `v128` subset (`0xfd00`, `0xfd0b`, `0xfd0c`, `0xfd0f..=0xfd12`, `0xfd1d`, `0xfd1e`). All other opcodes rejected.
- **Resource & peak-work caps** (checked BEFORE large allocations; return clean `Err`, never panic/wrap):
  - `MAX_FUNCREF_TABLE_SLOTS = 1_048_576`, `MAX_TOTAL_FUNCREF_SLOTS = 2_097_152`, `MAX_EXTERN_TABLE_SLOTS = 65_536`, `MAX_TOTAL_EXTERN_SLOTS = 65_536`, `MAX_GLOBALS = 65_536`, `MAX_CTX_BYTES = 16 MiB`, `MAX_AGGREGATE_CI_TABLE_BYTES = 64 MiB`.
  - `MAX_LOCALS_PER_FN = 65_536`, `MAX_TOTAL_LOCALS = 1_000_000`, `MAX_FRAME_SCAN_WRITES = 4_000_000`, `MAX_IF_SNAPSHOT_ENTRIES = 4_000_000`, `MAX_LIVENESS_BYTES = 64 MiB`, `MAX_FRAME_BYTES = 4 MiB` (well below the 8 MiB guest stack).
  - Checked `i32` arithmetic in `layout_args`/`layout_rets`/spill allocation, `ValRange.len: u32` with checked pool push, checked `align_up` in `elf.rs`, and `seg1_end <= i32::MAX` (`< 2 GiB` total `.rodata + .text`) with checked `i32::try_from` on every `rel32`.

---

## 3. Exact ABI, Runtime Stack, Memory & ELF64 Specification

### 3.1 Register Partition & Unified Internal Wasm ABI
- **Pinned registers** (never allocated by regalloc; invariant across internal Wasm calls):
  - `Rsp`: active guest stack pointer (16-byte aligned before every `call`, so `rsp % 16 == 8` on callee entry)
  - `R15`: `Ctx*` (`BlotHandle == Ctx`)
  - `R14`: `mem_base` (fixed virtual `mmap` base — invariant across `memory.grow`; `NULL` if 0 capacity)
  - `R13`: `mem_len` (current accessible byte length — updated in-place by `memory.grow` after `mprotect` succeeds)
  - `R12`: pinned hottest mutable non-const `I32`/`I64` global (typically `$__stack_pointer`), synced with `Ctx` at every C ABI boundary, host callback, and active reentrant call/trap
- **Scratch registers** (never allocated by regalloc; strictly disjoint roles):
  - `R11` / `Xmm15`: primary scratch, internal trap return (`R11d = 0 | 1..22`), parallel-move **cycle-breaking** temporary
  - `R10` / `Xmm14`: secondary scratch, parallel-move **memory-to-memory copy** temporary
- **Allocatable registers**:
  - 9 GP: caller-saved `Rax, Rcx, Rdx, Rsi, Rdi, R8, R9` (7) + callee-saved `Rbx, Rbp` (2)
  - 14 XMM: `Xmm0..Xmm13` (caller-saved in Stage 1)
- **Unified Typed Internal Call ABI** (single source of truth in `regalloc::{layout_args, layout_rets}`):
  - GP args: `Rdi, Rsi, Rdx, Rcx, R8, R9`; FP/V128 args: `Xmm0..Xmm7`; overflow args on caller `[rsp + off]` (8-byte aligned scalars, 16-byte aligned 16-byte `V128`, total rounded to 16 B).
  - GP results: `Rax, Rdx, Rcx, Rsi, Rdi, R8, R9`; FP/V128 results: `Xmm0..Xmm7`; overflow results written to caller-owned 16-byte-aligned stack buffer `[rsp + ret_buf_off]` whose address caller stores into `[r15 + OFF_RET_SPILL_PTR]` immediately before `call`. Any callee with overflow returns saves `[r15 + OFF_RET_SPILL_PTR]` in its own frame slot `saved_ret_ptr_off` in its prologue and writes overflow results before `emit_parallel_moves` writes register results.
- **`I32` zero-extension invariant**: every `I32` SSA value in a GP register has zero upper 32 bits. Enforced explicitly at **every** producer boundary:
  - All 32-bit x86-64 ops and `mov_rr(false, d, d)`;
  - C ABI entry wrappers (`mov edi, esi` / `mov edi, arg_bits_lo`);
  - `HELPER_MEM_GROW` failure returns `mov eax, -1` (`0x0000_0000_ffff_ffff`, NOT `-1i64`);
  - All 8 `I32` `HELPER_TRUNC_BASE` routines produce 32-bit zero-extended `Rax` (`mov eax, eax` / 32-bit `cvtt*2si eax`);
  - `wt_import{k}` trampolines zero-extend `[I32]` results (`mov eax, eax`).

### 3.2 Dedicated `mmap` Guest Stack & Reentry Protocol (Without `libc`)
`blot_aot_new` allocates a dedicated **8 MiB** guest stack with a **64 KiB `PROT_NONE` bottom guard** (anonymous demand-paged: unused pages consume zero physical RSS):
- Allocation: `mmap(NULL, 64 KiB + 8 MiB, PROT_NONE, MAP_PRIVATE | MAP_ANONYMOUS | MAP_NORESERVE, -1, 0)` followed by `mprotect(stack_base + 64 KiB, 8 MiB, PROT_READ | PROT_WRITE)`. On failure, `munmap` any prior allocations and return `TRAP_ALLOC_FAILED (12)`.
- `stack_top = stack_base + 64 KiB + 8 MiB` (16-byte aligned).
- `stack_floor = stack_base + 64 KiB + 64 KiB` (**64 KiB headroom** above the `PROT_NONE` guard — accommodating Linux x86-64 AVX-512 `xstate` signal frames (~3–4 KiB) + normal async signal handler stack + internal helper call frames without requiring `sigaltstack`, while still leaving `8 MiB - 64 KiB` of usable guest stack).
- **Exact stack-switching, reentry, & `MXCSR` protocol**:
  1. **Top-level C ABI entry** (`blot_aot_call_export`, `blot_aot_call_allocate`, `blot_aot_call_reset`, `blot_aot_call_finish_invocation`):
     - On host stack: check `h != NULL` and **`ctx->in_call == 0`** BEFORE touching the guest stack! If `ctx->in_call != 0` (illegal same-handle top-level reentry from inside a host callback), leave `*res_out` untouched, write `*trap_out = TRAP_HOST (19)` (if non-null), and return `2` immediately without clobbering `[stack_top - 8]`. (`blot_aot_active_*` is the only supported synchronous reentry path.)
     - Save SysV callee-saved `Rbx, Rbp, R12..R15`, allocate a 16-byte-aligned local host frame saving host `MXCSR` at `[rsp + 0]` (`stmxcsr [rsp + 0]`) plus `res_out`/`trap_out`/`h` pointers in local slots; set `ctx->in_call = 1`; load Wasm `MXCSR` (`0x1f80`); load `R15=ctx, R14=mem_base, R13=mem_len, R12=g_pinned`.
     - Switch to guest stack: store `ctx->host_rsp = rsp` (pointing at `[host_mxcsr, ...]`), save current host `rsp` into `[stack_top - 8]`, set `rsp = stack_top - 16` (16-byte aligned before `call`), `call target`, restore `rsp = [stack_top - 8]`.
     - On host stack: store `R12` back to `ctx->g_pinned` (on **both** normal return and trap), clear `ctx->in_call = 0`, restore host `MXCSR` (`ldmxcsr [rsp + 0]`), write `*res_out` ONLY on `R11d == 0` (leaving `*res_out` untouched on trap!) or `*trap_out` on `R11d != 0`, free local host frame, pop callee-saved regs, return `0`/`1`/`2`.
  2. **`blot_aot_new` -> `wt_initialize` ordering**:
     - Runs CPUID gate (§2.1), allocates `Ctx`, guest stack, and linear memory (§3.4), saves SysV callee-saved regs & host `MXCSR` at `[rsp + 0]`, sets `ctx->in_call = 1`, `ctx->host_rsp = rsp`, switches to `[stack_top - 16]`, loads `0x1f80`, `R15=ctx, R14=mem_base, R13=mem_len`, and calls `wt_initialize`.
     - Inside `wt_initialize`: (1) write all non-folded global initializers into `Ctx`, (2) **then** load `R12 = ctx->g_pinned` (so `R12` is initialized before `start`!), (3) check active element bounds (`TRAP_TABLE_OOB`), (4) copy active data segments (`TRAP_OOB`), (5) call `start` if present.
     - Upon return to `blot_aot_new` on the host stack: write back `R12 -> ctx->g_pinned`, clear `ctx->in_call = 0`, restore host `MXCSR`. On failure (`R11d != 0`), AFTER restoring the host stack, `munmap` guest stack, linear memory (if non-zero), and `Ctx`, write `*trap_out = R11b` (if non-null), and return `NULL`.
  3. **Host callback (`HELPER_HOST_CALL`, invoked from `wt_import{k}`)**:
     - First check `cmp qword [r15 + OFF_HOST_FN], 0; je .trap_host` -> return `R11d = TRAP_HOST (19)` immediately if `host_fn == NULL` (exact match to `cgen.rs:4086`).
     - On guest stack: allocate a 32-byte guest frame saving previous `OFF_ACTIVE_GUEST_RSP` and previous `OFF_HOST_RSP`; store current `guest_rsp` into `[r15 + OFF_ACTIVE_GUEST_RSP]`; publish `R12` to `ctx->g_pinned`.
     - Switch `rsp = [r15 + OFF_HOST_RSP]` (the active outer wrapper's 16-byte-aligned host stack frame), execute `ldmxcsr [rsp + 0]` to restore the outer host `MXCSR`, subtract a 32-byte host call frame (keeping 16-byte alignment before `call`), build `ActiveCall { ctx = R15, mem = R14 }` + `u64 res = 0` on the host stack, and call `ctx->host_fn(user_ctx, &ac, idx, tok, arg, &res)` via SysV ABI on the **real host stack**.
     - After `host_fn` returns: capture `res` and `eax` status, switch `rsp` back to saved `guest_rsp`, reload Wasm `MXCSR` (`0x1f80`), reload `R13 = ctx->mem_len` and `R12 = ctx->g_pinned`, restore saved `OFF_ACTIVE_GUEST_RSP` and `OFF_HOST_RSP`, return `Rax = res` and `R11d = 0 | TRAP_HOST (19)`.
  4. **Active reentrant call** (`blot_aot_active_allocate`, `blot_aot_active_reset`):
     - On host stack: save SysV callee-saved `Rbx, Rbp, R12..R15`, allocate 16-byte-aligned local host frame saving host `MXCSR` at `[rsp + 0]` + `res_out`/`trap_out`, load `R15 = ac->ctx`, `R14 = ac->mem`.
     - Validate `ctx->in_call != 0` AND wrap-proof bounds on `suspended = [r15 + OFF_ACTIVE_GUEST_RSP]`: require `suspended >= ctx->stack_floor + 64 && suspended <= ctx->guest_stack_top` (else restore host `MXCSR`, pop regs, write `*trap_out = TRAP_CALL_STACK (10)`, leave `*res_out` untouched, return `1`!).
     - Compute `new_guest_rsp = (suspended & !15) - 32` (guaranteed `>= ctx->stack_floor` with zero unsigned wrap), save previous `ctx->host_rsp` and current host `rsp` in that 32-byte link record, set `ctx->host_rsp = host_rsp`, load `R13 = ctx->mem_len`, `R12 = ctx->g_pinned`, load Wasm `MXCSR` (`0x1f80`), switch `rsp = new_guest_rsp`, `call blot:allocate/reset`, restore `ctx->host_rsp` and host `rsp`, write back `R12` to `ctx->g_pinned` (normal AND trap), restore local host `MXCSR` (`ldmxcsr [rsp + 0]`), write `*res_out` only on success, pop callee-saved regs, return status.
  5. **Wrap-proof pre-prologue function check** (implemented in `lower.rs:1671-1686`): computes `mov r11, [r15 + OFF_STACK_FLOOR]; add r11, (frame_size + 8*callee_saves); cmp rsp, r11; jb .early_stack_trap` **before** any push or frame subtraction (cannot wrap unsigned because user-space `stack_floor < 2^47` and `total <= 4 MiB + 16`).
  6. **`blot_aot_drop`**: no-op if `NULL`; `munmap`s guest stack (`64 KiB + 8 MiB`), guest memory (`mem_reserve_bytes`, if non-zero), and `Ctx` (`handle_alloc_bytes`).

### 3.3 `Ctx` (`BlotHandle`) Byte-for-Byte Layout
Every non-folded global (`global_consts[k].is_none()`) gets a checked `i32` slot at `global_offs[k]` (bounded by `MAX_CTX_BYTES = 16 MiB`):

| Offset | Size | Field | Notes |
|---:|---:|---|---|
| `0` | 8 | `mem_base: *mut u8` | Fixed virtual reservation base (`NULL` if 0 capacity) |
| `8` | 8 | `mem_len: usize` | Current accessible byte length (`pages * 65536`) |
| `16` | 8 | `mem_max_pages: u64` | Clamped max pages (`<= 65536`) |
| `24` | 8 | `mem_reserve_bytes: usize` | Exact reservation passed to `munmap` (`0` if no reservation) |
| `32` | 8 | `handle_alloc_bytes: usize` | Page-aligned `Ctx` size for `munmap` |
| `40` | 8 | `stack_floor: usize` | `guest_stack_base + 64 KiB + 64 KiB` (immutable after `new`) |
| `48` | 8 | `host_user_ctx: *mut c_void` | Passed as `host_fn` arg 0 |
| `56` | 8 | `host_fn: Option<BlotHostFn>` | Host callback pointer (`NULL` -> `TRAP_HOST`) |
| `64` | 8 | `scratch_tmp: u64` | Internal scratch (`call_indirect` `idx` across arg moves) |
| `72` | 8 | `ret_spill_ptr: *mut u8` | Caller-owned overflow multi-value return buffer pointer |
| `80` | 8 | `guest_stack_base: *mut u8` | Base of `64 KiB + 8 MiB` `mmap` guest stack (for `munmap`) |
| `88` | 8 | `guest_stack_top: usize` | `guest_stack_base + 64 KiB + 8 MiB` |
| `96` | 8 | `host_rsp: usize` | Active outer host `rsp` (points to `[host_mxcsr_slot, ...]`) |
| `104` | 8 | `active_guest_rsp: usize` | Suspended guest `rsp` during `HELPER_HOST_CALL` (for `active_*`) |
| `112` | 1 (+15 pad) | `in_call: u8` | Reentrancy guard (`1` while top-level call / `wt_initialize` active; rejects illegal top-level reentry) |
| `128..` | var | Non-folded globals | 16-byte aligned base; 8 B each for scalars/refs, 16-byte aligned 16 B for `V128` |
| `..` | var | Mutable `externref` tables | 8-byte aligned, `len * 8` bytes each |

Total rounded up to 4096 bytes. `CtxLayout` & `RodataConsts` exact Rust fields (matching `lower.rs`):
- `CtxLayout { pinned_global: Option<u32>, global_offs: Vec<i32>, extern_tables: BTreeMap<u32, (i32, u32)>, ci_tables: Vec<CiTableLayout>, total_bytes: usize }`
- `CiTableLayout { table_len: u32, min_slot: u32, span: u32, rodata_off: u32 }`
- `RodataConsts { f32_abs: u32, f32_neg: u32, f64_abs: u32, f64_neg: u32, f32_qnan: u32, f64_qnan: u32, mxcsr_wasm: u32, ... }` — **Invariant**: `f32_abs`, `f32_neg`, `f64_abs`, `f64_neg` MUST each be **16-byte aligned AND 16 bytes wide** (four `u32` or two `u64` lanes) because `lower.rs` emits 128-bit `andps`/`xorps`/`andpd`/`xorpd` RIP-relative memory operands.

### 3.4 Memory Lifecycle & Linux Syscall Rules
- Linux syscalls use `(rax=nr, rdi, rsi, rdx, r10, r8, r9)`, clobber `rcx` & `r11`, and signal errors when `(rax as u64) >= (-4095i64 as u64)`. Keep syscall setup strictly isolated from SysV C call wrappers.
- **`blot_aot_new` memory limits** (exact match to `cgen.rs:4350-4355` and Pen `wtime_aot.rs:930-954`):
  - If module has **no memory**: `mem_base = NULL, mem_len = 0, mem_max_pages = 0, mem_reserve_bytes = 0` (no memory `mmap`).
  - If module has memory `(mem_min, declared_max)`:
    - `limit = if mem_limit_pages == 0 { 65_536 } else { mem_limit_pages }` (`0` means unlimited in library protocol v1; Pen enforces sub-page limits `< 64 KiB` on growable memories BEFORE calling `blot_aot_new` so `start` never runs and `memory.grow` cannot bypass a sub-page sandbox cap).
    - If `mem_min > limit`: fail with `TRAP_MEM_LIMIT (11)`.
    - `max_p = min(declared_max, limit, 65_536)`.
    - If `mem_min > max_p`: fail with `TRAP_MEM_TOO_LARGE (9)`.
    - If `max_p == 0` (genuine zero-capacity memory): `mem_base = NULL, mem_len = 0, mem_max_pages = 0, mem_reserve_bytes = 0`.
    - Else: `mmap(NULL, max_p * 65536, PROT_NONE, MAP_PRIVATE | MAP_ANONYMOUS | MAP_NORESERVE, -1, 0)`; if `mem_min > 0`, `mprotect(mem_base, mem_min * 65536, PROT_READ | PROT_WRITE)`. `mprotect` errors cleanly return `TRAP_ALLOC_FAILED (12)` (in `new`, unmapping all prior allocations) or `-1` (in `grow`, leaving `mem_len` unchanged).
- **`memory.grow` vs `blot_aot_mem_grow`**:
  - Shared `grow` core checks `delta == 0` (returns `old_pages` immediately without `mprotect`), `delta <= max_pages && old_pages <= max_pages - delta`, runs `mprotect(mem_base + old_len, delta * 65536, PROT_READ | PROT_WRITE)`, and updates `ctx->mem_len` & `R13` **only after `mprotect` succeeds**.
  - `HELPER_MEM_GROW` (Wasm `memory.grow`, `Rdi = u32 delta`): returns zero-extended `0xffff_ffffu64` (`mov eax, -1`) on failure, or `old_pages` in `Eax` on success.
  - `blot_aot_mem_grow(h, delta: u64) -> i64`: **MUST save/restore SysV callee-saved `R13` (and `R15` if loaded)** before calling the shared grow core; validates full 64-bit `delta` (never truncates to `u32`), returns signed `-1i64` (`0xffff_ffff_ffff_ffff`) on failure or `old_pages as i64` on success.

### 3.5 Exact Helper Clobber Contracts & Complete 18-Export Library Protocol v1 (Blot Guest ABI 8)
- **Pure-guest helpers (`regalloc::HelperKind`)** preserve `Rbx, Rbp, R12..R15` (except `MemGrow` updating `R13`) and **all `Xmm0..Xmm13`**:
  - `HELPER_TRUNC_BASE + 0..15` (`HelperKind::Trunc`): `Xmm15 -> Rax` (32-bit zero-extended for `I32`, 64-bit for `I64`), `R11d = 0 | 6 | 7`; clobbers ONLY `Rax, R10, R11, Xmm14, Xmm15` (`gp_mask = MASK_RAX, xmm_mask = 0`).
  - `HELPER_MEM_GROW (16)` (`HelperKind::MemGrow`): `Rdi (u32) -> Rax (u32 zero-extended)`; clobbers `MASK_GP_CALLER, xmm_mask = 0`.
  - `HELPER_MEM_COPY (17)` & `HELPER_MEM_FILL (18)`: `(Rdi, Rsi, Rdx) -> R11d = 0 | TRAP_OOB`. **Mandatory zero-length OOB & null-`R14` order**: perform 64-bit unsigned bounds checks (`(dst as u64) + (len as u64) > R13` and for copy `(src as u64) + (len as u64) > R13`) **FIRST** (trapping `TRAP_OOB` even when `len == 0` if `off > mem_len`), **THEN** early-return `R11d = 0` on `len == 0` **BEFORE** dereferencing `R14` (`NULL` for 0-capacity memories); overlap-safe forward (`rep movsb`) / backward loop (no `std`/`cld`); clobbers `MASK_GP_CALLER, xmm_mask = 0`.
- **`HELPER_HOST_CALL (19)`** (NOT in `HelperKind`, invoked ONLY via `wt_import{k}` through `Op::Call` which already clobbers `MASK_GP_CALLER | MASK_XMM_CALLER`):
  - Inputs: `Edi = import_idx`, `Rsi = tok (u64)`, `Rdx = arg_bits (u64)`.
  - Output: `Rax = res_bits (u64)`, `R11d = 0 | TRAP_HOST (19)`.
  - Checks `ctx->host_fn == NULL -> TRAP_HOST`, publishes `R12`, switches to host stack, restores host `MXCSR` from `[rsp + 0]`, calls `host_fn`, switches back to guest stack, reloads `0x1f80` `MXCSR`, reloads `R13` & `R12`.
- **`wt_import{k}` param/result marshaling** (exact match to `cgen.rs:4097-4117`, with explicit non-clobbering move order — move `Rsi -> Rdx` BEFORE `Rdi -> Rsi`, and load `Edi = import_idx` LAST!):
  - Params: `[] -> (0, 0)`, `[Ref] -> (rsi=rdi, rdx=0)`, `[I32] -> (rsi=0, edx=edi)`, `[F32] -> (rsi=0, movd edx, xmm0)`, `[Ref, I32] -> (edx=esi, rsi=rdi)`, `[Ref, I64] -> (rdx=rsi, rsi=rdi)`, `[Ref, F32] -> (movd edx, xmm0, rsi=rdi)`, `[Ref, F64] -> (movq rdx, xmm0, rsi=rdi)`, `[Ref, Ref] -> (rdx=rsi, rsi=rdi)`.
  - Results: `[] -> ()`, `[I32] -> mov eax, eax`, `[I64]|[Ref] -> rax`, `[F32] -> movd xmm0, eax`, `[F64] -> movq xmm0, rax`.
- **Instantiation order (`wt_initialize`)**: (1) initialize all non-folded globals from `global_inits`, then load `R12 = ctx->g_pinned`; (2) validate ALL active element segment bounds (`(base as u64) + (elem_len as u64) > (table_len as u64)` checked even when `elem_len == 0` -> `TRAP_TABLE_OOB (3)` before any data write or `start`); (3) for each active data segment in declaration order, check `(off as u64) + (len as u64) > R13` BEFORE `len == 0` skip, then if `len > 0` copy bytes from `.rodata` to `[R14 + off]` (trapping `TRAP_OOB (2)`, allowing overlapping segments in order); (4) if `module.start` is present, call `start` on the guest stack.
- **Explicit parity & documented differences vs `cgen.rs`**:
  - Library protocol version is `1` (`blot_aot_abi_version() == 1`), hosting Blot guest ABI `8`.
  - Trap codes `1..=21` carry byte-for-byte identical messages to `cgen.rs:4299-4323`.
  - **Three intentional differences**: (a) trap code `22` (`"Unsupported CPU ISA"`) added for the CPUID gate; (b) 0-capacity memories report `mem_base = NULL, mem_len = 0` (vs C's 1-byte static dummy — safe because Pen checks `n == 0 || p.is_null()` and all `len == 0` operations return before touching `R14`); (c) `blot_aot_import_name` and `blot_aot_trap_message` emit explicit `(ptr, len)` pairs pointing to non-null `.rodata` addresses even for empty strings (no `strlen`).
  - **Out-pointer contract across all 18 exports**: every optional out pointer (`ptr`, `len`, `res_out`, `trap_out`, `val_out`, `kind_out`) is checked for `NULL` before writing; on error/trap/missing, result out-pointers (`res_out`, `val_out`, `kind_out`, `mp/ml/np/nl`) are left **untouched**.

| # | Export Symbol | Exact C Signature | Return / Status Contract |
|---:|---|---|---|
| 1 | `blot_aot_abi_version` | `() -> u32` | Returns `1` (library protocol version 1) |
| 2 | `blot_aot_trap_message` | `(u8 code, *mut *const u8 ptr, *mut usize len) -> i32` | `0` if `code in 1..=22` (writes non-null `.rodata` `ptr`/`len`), else `1` (outs untouched) |
| 3 | `blot_aot_import_count` | `() -> u32` | Returns `imports.len() as u32` |
| 4 | `blot_aot_import_name` | `(u32 idx, *mut *const u8 mp, *mut usize ml, *mut *const u8 np, *mut usize nl) -> i32` | `0` if `idx < count` (writes non-null `(ptr, len)` pairs), else `1` (outs untouched) |
| 5 | `blot_aot_new` | `(*mut c_void user_ctx, Option<HostFn>, u64 mem_limit_pages, *mut u8 trap_out) -> *mut Ctx` | Returns handle or `NULL` (writing `*trap_out` if non-null); runs CPUID gate, allocates `Ctx` + guest stack + memory, runs `wt_initialize`, unmaps all allocations on failure |
| 6 | `blot_aot_drop` | `(*mut Ctx h) -> void` | No-op if `NULL`; `munmap`s guest stack, memory (if non-zero), and `Ctx` |
| 7 | `blot_aot_call_export` | `(*mut Ctx h, *const u8 np, usize nl, u64 arg_bits, *mut u64 res_out, *mut u8 trap_out) -> i32` | Rejects `in_call != 0` (`2`, `TRAP_HOST`); matches `(nl, np)` (checks `nl` before reading `np`); zero-extends narrow `I32`/`F32` args & results; `0` ok (`*res_out` = val or `0` for void), `1` trap, `2` `TRAP_HOST`, `3` unknown/incompatible |
| 8 | `blot_aot_call_allocate` | `(*mut Ctx h, i32 arg, *mut i32 res_out, *mut u8 trap_out) -> i32` | Rejects `in_call != 0`; calls `blot:allocate` if `(i32)->(i32)` (zero-extends `arg`, writes 32-bit `*res_out` on ok); returns `0`/`1`/`2`/`3` |
| 9 | `blot_aot_call_reset` | `(*mut Ctx h, i32 arg, *mut i32 res_out, *mut u8 trap_out) -> i32` | Rejects `in_call != 0`; calls `blot:reset` if `(i32)->(i32)`; returns `0`/`1`/`2`/`3` |
| 10 | `blot_aot_call_finish_invocation` | `(*mut Ctx h, *mut i32 res_out, *mut u8 trap_out) -> i32` | Rejects `in_call != 0`; calls `blot:finish_invocation` if `()->(i32)`; returns `0`/`1`/`2`/`3` |
| 11 | `blot_aot_mem_data` | `(*mut Ctx h, *mut *mut u8 ptr_out, *mut usize len_out) -> void` | Writes `h->mem_base` / `h->mem_len` to non-null outs |
| 12 | `blot_aot_mem_grow` | `(*mut Ctx h, u64 delta) -> i64` | Preserves SysV `R13`; full 64-bit `delta`; returns `old_pages as i64` or `-1i64` |
| 13 | `blot_aot_get_global` | `(*mut Ctx h, *const u8 np, usize nl, *mut u64 val_out, *mut u8 kind_out) -> i32` | Serves `Ctx`, folded scalar consts, AND folded `RefNull` (`val=0, kind=4`); `kind`: `0=I32, 1=I64, 2=F32, 3=F64, 4=Ref`; `V128` -> `1`; `0` found, `1` missing |
| 14 | `blot_aot_set_global` | `(*mut Ctx h, *const u8 np, usize nl, u64 val) -> i32` | Writes exported mutable scalar/ref global (`V128` -> `1`); returns `0` updated, `1` missing/immutable |
| 15 | `blot_aot_active_mem_data` | `(*mut ActiveCall ac, *mut *mut u8 ptr_out, *mut usize len_out) -> void` | Reads `ctx = ac->ctx`, writes `ac->mem` / `ctx->mem_len` to non-null outs |
| 16 | `blot_aot_active_allocate` | `(*mut ActiveCall ac, i32 arg, *mut i32 res_out, *mut u8 trap_out) -> i32` | Validates `in_call != 0` and wrap-proof `active_guest_rsp` bounds; syncs `R12` on ok & trap; returns `0`/`1`/`2`/`3` |
| 17 | `blot_aot_active_reset` | `(*mut ActiveCall ac, i32 arg, *mut i32 res_out, *mut u8 trap_out) -> i32` | Same as `active_allocate` for `blot:reset`; returns `0`/`1`/`2`/`3` |
| 18 | `blot_aot_active_get_global` | `(*mut ActiveCall ac, *const u8 np, usize nl, *mut u64 val_out, *mut u8 kind_out) -> i32` | Same as `blot_aot_get_global` on `ac->ctx`; returns `0`/`1` |

### 3.6 ELF64 Linking & Relocation Bases (`tools/wtime-aot/src/x64/elf.rs`)
- **Conforming ELF64 `ET_DYN`**: 3 page-separated `PT_LOAD` segments (`R`: Ehdr + Phdrs + `.dynsym` + SysV `.hash` + `.dynstr` + `.rodata`; `RX`: `.text`; `RW`: `.dynamic`) + `PT_DYNAMIC` + `PT_GNU_STACK` (`PF_R | PF_W`), plus an 8-entry Section Header Table (`.dynsym`, `.hash`, `.dynstr`, `.rodata`, `.text`, `.dynamic`, `.shstrtab`) so every `.dynsym` export carries real `st_shndx = 5` (`.text`), checked `align_up`/size arithmetic, and `seg1_end <= i32::MAX`.
- **Two explicit `rel32` displacement bases** (every conversion checked via `i32::try_from`):
  1. **Instruction relocations (`Reloc`)**: `(target_vaddr - (fn_vaddr + next_ip_off)) as i32` (relative to the end of the instruction).
  2. **Jump tables (`JumpTablePatch`) & `call_indirect` dispatch tables**: `(target_vaddr - table_base_vaddr) as i32` (relative to the first byte of the table in `.rodata`; `0` sentinel in `call_indirect` tables unambiguously marks empty/type-mismatched slots because `.text` starts strictly after `.rodata`).

### 3.7 Current WIP Files (`tools/wtime-aot/src/x64/`) — All Verified Zero-Warning Clippy (`-D warnings`, `#![forbid(unsafe_code)]`)
1. `module.rs`: `X64Module`, `X64ConstVal`, `admit_x64_module` enforcing §2.2 (plain final `Func` types, `ImportKind::Func` only, `blot:memory`, explicit `ref.null` table init check, canonical `ci_sigs`, aggregate caps checked before allocation, non-folded exported mutable globals, unreachable-code opcode/type/block-type checks, `active_elem_checks` & `active_data_segs`).
2. `asm.rs`: x86-64 + SSE2 + SSE4.1 + POPCNT + LZCNT/TZCNT + `bsr`/`bsf`/`cpuid`/`stmxcsr`/`ldmxcsr` encoder (audited zero encoding bugs; documented `mov_ri*(r, 0)` `RFLAGS` caveat).
3. `elf.rs`: Keyed `ExportDef<'a>`, exact `planned_rodata_len` check, checked `align_up` & `seg1_end <= i32::MAX`, non-empty `.text` & export extent validation, duplicate-name rejection, conforming 8-section header table.
4. `ir.rs`: Decoupled `lower_wasm_to_ir(decoded, &X64Module, func_index)`, exact admitted single-value block types (`-1..-5 | -17`), bounded `scan_frame_locals` & `if` local snapshots (`MAX_FRAME_SCAN_WRITES`, `MAX_IF_SNAPSHOT_ENTRIES`, move on `else`), `u32` `ValRange.len` with checked bounds, constant folding, `CmpBranch` fusion, trap-order-preserving linear load-run check hoisting, DCE.
5. `regalloc.rs`: Checked `layout_args`/`layout_rets` (16-byte aligned `V128` + caller-owned overflow return buffer), unified `HelperKind`/`helper_clobbers`, checked `MAX_FRAME_BYTES (4 MiB)`/`MAX_LIVENESS_BYTES (64 MiB)` & checked spill accumulation, RPO + tight `live_in`/`live_out` cross-block `BitSet` liveness (no false def-pos drag across earlier clobbers) + clobber-aware linear scan.
6. `lower.rs`: Disjoint parallel-move scratches (`R11`/`Xmm15` cycle break, `R10`/`Xmm14` mem-to-mem), `TargetIsa` with branchless `bsr`/`bsf`+`cmovz` fallback, fixed shift `RCX` staging, fixed spilled `u64 -> fp` sticky-bit capture, unsigned 32-bit `call_indirect`/`table`/`br_table` bounds checks, wrap-proof pre-prologue stack check (`stack_floor + total`), caller-owned overflow returns.

---

## 4. Remaining Work — Stage 1 (First Complete Working Milestone)

### 4.1 Implement `tools/wtime-aot/src/x64/mod.rs`
1. Wire `admit_x64_module` -> `CtxLayout` (checked `i32` offsets <= `MAX_CTX_BYTES`) -> `RodataConsts` -> emit shared runtime helpers (`HELPER_TRUNC_BASE + 0..15`, `HELPER_MEM_GROW`, `HELPER_MEM_COPY`, `HELPER_MEM_FILL`, `HELPER_HOST_CALL`), `wt_import{k}` trampolines, `wt_initialize`, and all 18 library protocol v1 C exports (§3.2–§3.5).
2. Compile all defined functions in parallel via bounded `std::thread::scope` (`--jobs N`, default `available_parallelism`) into an indexed `Vec<Option<Asm>>` for **100% deterministic** `.text`/`.rodata` output order.
3. Merge per-function `local_rodata` preserving 16-byte alignment, call `ElfLayout::plan`, resolve every `Reloc`, `JumpTablePatch`, and `call_indirect` table entry with checked `i32::try_from`, call `ElfLayout::emit`, and atomically publish (`tmp.{pid}.{seq}` -> `rename`).
4. Emit `WTIME_AOT_TIMINGS=1` phase breakdown: `validate`, `admit`, `ssa_opt`, `regalloc`, `emit`, `link`, `write`, `total`.

### 4.2 Wire CLI (`main.rs`, `build.rs`) & Pen Integration (Already Hardened in `pen`)
- Direct CLI (`main.rs`): `wtime-aot module.wasm -o out.so --blot-cdylib --backend x64 [--target-cpu NAME]` writes the `.so` directly to `-o` (atomic temp + rename). Reject `--backend x64` without `--blot-cdylib` in Stage 1.
- Cached CLI (`build.rs`): `wtime-aot build module.wasm --cache DIR --blot-cdylib --backend x64 [--target-cpu NAME] [--jobs N]` uses the shared canonical ISA resolver (§2.1), ignores `--toolchain`/`--c-opt`, rejects `--train`, keys cache by `("wtime-aot build x64 1", wasm, generator_exe, resolved_isa_profile)`, and publishes `library.so` atomically via unique temp file + rename.
- Pen (`crates/blot-host/src/{wtime_aot.rs,lib.rs}`, verified at `pen` `main`):
  - Exact Wasm ABI signature validation (`validate_aot_wasm`) before cache lookup or compilation (imports, entrypoints, constants, `blot:memory` as Memory, `blot:allocate`/`blot:reset` as `i32->i32`, ownership hooks, `blot:finish_invocation`, plus memory `(min, max)` extraction).
  - Pre-`blot_aot_new` sub-page limit enforcement: rejects initial memory `> lim.memory` AND growable memories under `< 64 KiB` caps BEFORE `start` runs.
  - Lock-free `!engine.get_epoch_interruption()` isolation + observable `Guest::set_deadline` panic on `AotGuest`.
  - Safe UTF-8 / null-checked `read_aot_str` for import names; scoped `with_mem_mut` (no `&'static mut [u8]`); raw-pointer `AotHostBox { in_trampoline: AtomicBool, state: UnsafeCell<AotHostState> }` with short non-FFI borrows; cleared/taken `pending_error` across `call_allocate`; canonical ISA profile tag + key-bound canonical-prefix-checked atomic `by-wasm/*.path` fast cache.
  - Keep explicit `PEN_WTIME_AOT_BACKEND=x64` errors surfaced diagnostically (never silently swallowed); only consider automatic fallback on admission/platform error if a future default policy explicitly opts in.

### 4.3 Stage 1 Verification & Measurement Matrix
1. **Unit, ABI, & differential tests** (`cargo test --locked --manifest-path tools/wtime-aot/Cargo.toml`):
   - **Full 18-export behavioral parity & ELF/glibc verification**: `dlopen`, `dlsym` all 18 exports + synthetic SysV hash collision, verify ASLR load + `/proc/self/maps` segment perms (`R`, `RX`, `RW`, non-executable stack), AND behaviorally test all 18 exports (exact output widths, untouched outputs on error/trap/missing, null optional outputs, exact trap messages 1–21 + code 22, folded `RefNull` global read, non-null `.rodata` ptr for empty strings).
   - **Admission acceptance vs rejection suite** (including dead/unreachable code):
     - Accepted: single-value blocks `-1..-5 | -17`, explicit `ref.null` matching table init, `blot:memory` export, equivalent plain final `Func` types in `call_indirect`.
     - Rejected: `-16` (`funcref`) block type (live & dead), `ref.null func` (live & dead), Wasm GC non-final `(sub (func))` (regression vs final `(func)`!), descriptor types, `ImportKind::ExactFunc`, non-null table init, aggregate table/ctx/locals/frame-scan/if-snapshot cap overflows.
   - **Opcode & lowering corner cases** (tested with **both** `has_abm_bmi1 = true` and `false`!):
     - Implicit-else + loop-carried locals + unreachable blocks; shifts/rotates with `a` dying in `RCX` (reg & spilled count); spilled `f32.convert_i64_u(0x8000_0080_0000_0001)` & `f64.convert_i64_u(0x8000_0000_0000_0401)` exact rounding.
     - Parallel moves: 3-way GP/XMM cycles, pure stack-to-stack swaps (`Stack(0) <-> Stack(8)`), duplicate sources, randomized mixed-width call/return/spill layouts.
     - Multi-value ABI: >6 GP args, 9th & 10th `v128` stack args non-overlapping, `24 i64` overflow returns, recursive multi-value calls.
     - All 16 float-to-int truncations including adjacent representable boundaries (`I32F64S` on `-2147483648.5` -> `INT32_MIN` vs `-2147483649.0` -> trap `6`; `I32*U`/`I64*U` on `-0.5` -> `0` vs `-1.0` -> trap `6`), subnormals, signaling & quiet NaNs, `±Inf`, `-0.0`, `min`/`max` (`-0.0`/`+0.0`, arithmetic NaN per Wasm spec), unordered float cmp, `INT_MIN % -1 == 0`, `div_s` overflow/zero traps.
     - `externref` table get/set/OOB traps; live `Xmm0` surviving across every `HelperKind` (`Trunc`, `MemGrow`, `MemCopy`, `MemFill`).
     - `call_indirect`: hit, OOB (`TRAP_TABLE_OOB`), null hole (`TRAP_INDIRECT_CALL`), signature mismatch (`TRAP_INDIRECT_CALL`), and imported function table target.
   - **Memory, syscalls, stack, signals, CPUID, & reentry**:
     - `blot:memory` export, `mem_limit_pages == 0` (unlimited) via real `AotGuest::start(None)`, Pen sub-page `Limits { memory: 0 }` rejecting growable memory before `start` (both `start` growth & ordinary call growth), genuine 0-capacity vs 0-memory modules, `grow(0)`, `grow > u32::MAX` via `blot_aot_mem_grow` (verifying caller `R13` preserved!), `grow` failing `mprotect` leaves `mem_len` unchanged, `mmap`/`mprotect` failure cleanup in `new`, `offset > i32::MAX` store with spilled val, overlapping forward/backward `memory.copy`, `memory.copy`/`memory.fill` with `len == 0` BOTH in-bounds on 0-capacity (`R14 == NULL`) AND OOB (`dst = mem_len + 1, len = 0` -> `TRAP_OOB`), load-run trap ordering.
     - Dedicated stack, signals, `MXCSR`, CPUID, & reentry: recursive `TRAP_CALL_STACK` on a 128 KiB host pthread (including large-frame callee & `active_*` reentry), Linux `SIGUSR1` async signal delivered on the switched guest stack near `stack_floor`, non-default host `MXCSR` preserved across calls/traps/reentry, `host_fn == None` calling an import -> `TRAP_HOST (19)`, same-handle top-level reentry rejected (`2`, `TRAP_HOST`) without stack corruption, `active_allocate`/`active_reset` mutating `R12`/`R13`, nested host-callback rejection inside `active_allocate`, `start` calling an import (C host pass + Pen `stale_capability` reject), `blot_aot_set_global` on exported mutable globals, and masked-CPUID failure test verifying `RBX`/live-arg preservation & trap `22`.
2. **Frozen Stage 1 benchmark & parity protocol**:
   - Inputs: `opt-palm` (`c958c20386207b3ea9713d5aae596edc264328ceaa87281b5cf6cc333db2b89b`) and `gdev-dev` (`25ebb19d8cb6adc2fc6dd9545e647baa88c29e0e021d7d274a511b4c6612f77b`), pinned CPU (`taskset`), fixed `--jobs`, recorded git revision & Wasmtime config.
   - Separate timings for: cache-miss CLI cold compile (with `WTIME_AOT_TIMINGS=1` phase breakdown), `dlopen`, `blot_aot_new` instantiation, and first frame call.
   - Stage 1 hypothesis: **~100 – 250 ms** cold compile on `opt-palm`, **~0.60 – 0.85×** Wasmtime runtime initially (before Stage 2 inlining/coalescing/LICM), **100% exact SHA-256 parity** across all 5 `opt-palm` workloads via `cpu-bench` and headless `pen` (`colony.penrec` + `gdev` `pen run`).

---

## 5. Stage 2 — Runtime Optimization Hill-Climb (Dependency-Ordered & Numerically Gated)

### 5.1 Pass Ordering & Strict Legality Rules
Every pass is evaluated independently (phase compile cost + paired 5-workload `opt-palm` runtime + exact parity) before combining into the Stage 2 candidate. An **IR/SSA verifier (`verify_ir`)** runs after every CFG-changing pass (inlining, LICM, DCE) in test builds to verify block-param arity/types, dominance, and effect summary consistency.
1. **Profile Stage 1 first**: measure compile-time tail on `gdev-dev` (`scan_frame_locals`, liveness, clobber scan, `O(S^2)` spill reuse, `O(M^2)` parallel moves) and hot functions/spills/bounds-check cost on `opt-palm`.
2. **Budgeted small-leaf inlining on IR** (runs **first** so all downstream passes optimize inlined bodies): non-recursive hot helpers (`<= 45` insns, bounded total code growth, keeping original functions for indirect/exported callers).
3. **CFG rebuild & verification -> true dominator tree (`Cooper–Harvey–Kennedy`) & effect analysis** (tracking global writes, memory writes, traps, and `memory.grow`/host callback effects).
4. **Dominator GVN / CSE + redundant `GlobalGet` elimination + conservative LICM** (respecting call/callback effects and exact trap precision).
5. **Loop & cross-block bounds-check elimination + addressing-mode folding**: dominator `checked_max` propagation, counted-loop induction-variable range hoisting (ONLY with formal proof that zero-trip loops cannot turn a non-executing body into a hoisted trap AND no earlier store/global-write/call/trap can be misordered relative to the hoisted trap), `[r14 + base + idx*scale + disp]` wrap-safe folding *(fallback to evaluate if explicit checks remain the dominant gap vs `--backend c`: bounded `SIGSEGV` guard-page experiment coordinated with host signal chaining)*.
6. **SSA cleanup (DCE/copy-prop) -> Register coalescing & ABI/loop-phi hinting + loop-depth weighted regalloc** (cutting loop-edge and call-boundary move traffic) + **callee-saved `Xmm8..Xmm13`** (internal convention preserved by generated callees and saved/restored around `host_fn` in `HELPER_HOST_CALL`; SysV C callers already treat XMMs as caller-saved).
7. **Cheap instruction-selection peepholes**:
   - `Cmp + Select -> cmp + cmovcc` (handling unordered FP `PF` cleanly).
   - Constant signed/unsigned `div`/`rem` -> magic multiply + shift (preserving exact divide-by-zero & `INT_MIN / -1` trap semantics).
   - Inline small constant-length `memory.copy` / `memory.fill` (`<= 64 B`) with widened bounds check before any write, preserving overlap and `len == 0` OOB semantics.
   - Memory-operand ALU (`add/sub/cmp r, [rsp+spill]`).
8. **2-phase native-frame stack-check elision -> THEN direct-call trap-check elision** (strictly coupled order!):
   - **Step 8a (Stack-check elision)**: parallel (SSA + regalloc) -> sequential acyclic max-native-frame propagation computing each candidate's **exact worst-case transitive native stack use** (`own_frame_size + 8*callee_saves + 8 /*ret addr*/ + max(callee_subtree_bytes, max_helper_frame_bytes)`). A function's prologue check may ONLY be elided if: (i) it is non-recursive and has **zero** `wt_import`/`HELPER_HOST_CALL` or indirect calls in its transitive call graph, (ii) its exact subtree native stack use is `<= 8 KiB` (far below the 64 KiB floor headroom), AND (iii) every caller that retains a stack check reserves `own_frame + 8*callee_saves + max_elided_subtree_bytes` against `stack_floor`!
   - **Step 8b (Direct-call trap-check elision)**: runs strictly **after** Step 8a; a callee qualifies for `!fn_can_trap` ONLY if `!callee.needs_stack_check` AND it has zero trapping Wasm ops and zero trapping callees in its transitive call graph (keeping full `R11d` trap ABI for indirect, exported, and unchecked entrypoints).

### 5.2 Frozen Promotion Decision Procedure (Before Flipping `pen run` Default from `c` → `x64`)
- **Measurement protocol**: pinned CPU (`taskset`), fixed `--jobs`, cold-compile measured from an empty cache directory with exact end-to-end ready-to-call timer (`CLI spawn -> .so rename -> dlopen -> blot_aot_new`), warm-load measured from a populated cache (`dlopen + blot_aot_new`), runtime measured via `N = 15` alternating pairs (`A/B`, `B/A` balanced) after 2 discarded warmup iterations per workload; estimator = median of paired ratios with 10,000-resample bootstrap 95% confidence intervals. **The conservative 95% CI bound MUST clear every threshold**:
  1. **Compile & load**: `opt-palm` cold ready-to-call upper 95% CI **<= 600 ms**; `gdev-dev` cold ready-to-call upper 95% CI **<= 1.2 s**; warm load upper 95% CI **<= 2 ms**.
  2. **Runtime vs Wasmtime**: geometric mean speedup across the 5 `opt-palm` workloads lower 95% CI **>= 1.15×**, every single `opt-palm` workload lower 95% CI **>= 1.05×**, p95 frame-time ratio `p95_x64 / p95_wasmtime <= 1.00` on every workload, and `gdev-dev` headless 30-frame runtime control upper 95% CI **<= 1.00×** Wasmtime.
  3. **Runtime vs `--backend c --c-opt auto`**: geometric mean time ratio across the 5 `opt-palm` workloads upper 95% CI **`time_x64 / time_c <= 1.05`** (within 5% of Clang `-O2`, or faster).
  4. **Correctness**: 100% bit-for-bit SHA-256 replay parity across all 5 `opt-palm` workloads, headless `pen` (`colony.penrec` + `gdev`), full Stage 1 + Stage 2 differential/ABI/signal/CPUID/reentry test suite, `dependency_budget.py` 0 violations, `clippy -D warnings` 0 warnings.

---

## 6. Review Log
- **Round 0 (`gpt-6.2-sol`)**: Caught 6 critical bugs in initial WIP files (compile/borrow errors, shift `RCX` clobber, spilled `u64->fp` bit-0 loss, parallel-move stack swap clobber, `v128`/return-spill ABI overlap, pre-push stack check). All 6 fixed in code and verified with 0-error isolated typecheck.
- **Round 1 (`Gemini 4 Neon`: 6.8/10, `gpt-6.2-sol`: 4.0/10)**: Caught `ir.rs` missing negative single-value block types, `blot:memory` export omission, inverted `mem_limit_pages == 0` rule, `I32` zero-extension boundaries, small-stack/reentry safety (dedicated 8 MiB `mmap` guest stack + guard), non-reentrant `MXCSR`, Pen's `&mut AotHostState` aliasing UB, `X64Module` decoupling, `TargetIsa` `bsr`/`bsf` fallback + CPUID rules, `elf.rs` keyed exports + section headers, aggregate caps, complete 18-export spec, and Stage 2 pass ordering + promotion gate. All fixed and verified.
- **Round 2 (`Gemini 4 Neon`: 8.2/10, `gpt-6.2-sol`: 4.5/10)**: Caught Wasm GC non-final `SubType` & `ExactFunc` admission hole (`module.rs`), Pen AB-BA lock order in `is_ticking_engine`/`dev_engine` (replaced lock-free with `engine.get_epoch_interruption()` + observable `set_deadline` panic), exact Wasm signature checks in `validate_aot_wasm`, null/UTF-8 safe `read_aot_str`, scoped `with_mem_mut`, pre-`blot_aot_new` sub-page `Limits` rejection for growable memories, `pending_error` leak on `call_allocate`, aggregate table/globals/`Ctx`/frame-scan/`if`-snapshot caps, `MAX_FRAME_BYTES = 4 MiB` + wrap-proof stack check, `regalloc.rs` tight `live_in`/`live_out` liveness, checked `layout_args`/`layout_rets`/spill/`elf.rs` math, 64 KiB signal-safe stack floor headroom, `in_call` same-handle reentry guard, `wt_initialize` `R12` init order, `blot_aot_mem_grow` `R13` save, null `host_fn` check, wrap-proof `active_guest_rsp` validation, zero-length OOB & null-`R14` ordering, canonical ISA tag sync + key-bound fast cache, Stage 2 stack-check -> trap-check coupling, and exact paired bootstrap CI promotion protocol. All fixed in `wtime` (`x64/*.rs`, `CONTINUE.md`) and `pen` (`crates/blot-host/src/{wtime_aot.rs,lib.rs}`).
