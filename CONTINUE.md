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
- `pen dev`, `test`, `bench`, `fuzz`, `diverge`, and unit tests stay on Wasmtime/Cranelift by default (preserving `dev_engine` deadlines and probes).
- Verified: `blot-host` tests (30/30), headless `opt-palm` colony 300 frames (`281 fps` vs `230 fps` Wasmtime, warm ready `1 ms` vs `12 ms`, exact `frames.trace` SHA-256 `6d550c35...`), headless `pen run --project /home/jacobt/src/gdev --frames 30` (cold `8,123 ms`, warm `3 ms` vs `47 ms`, exact SHA-256 `c2aadc5f...`).

---

## 2. Goal, Scope & Explicit Stage 1 Admission Contract (`--backend x64`)

Build a self-contained pure-Rust (`#![forbid(unsafe_code)]`) Wasm → Linux x86-64 ELF64 `.so` compiler inside `tools/wtime-aot/src/x64/`:
- **Dependencies**: the compiler crate (`tools/wtime-aot`) depends only on in-tree `wtime-validate` and `wtime-types` (zero third-party crates, zero `clang`/`rustc`/`ld.lld` subprocesses); the **generated `.so`** has zero `DT_NEEDED`, zero dynamic relocations, and zero `libc` dependency (uses direct Linux x86-64 `syscall` for `mmap`/`mprotect`/`munmap`).
- **Metering & safety policy**: `tools/wtime-aot` is an explicit opt-in AOT tool isolated outside the production safe-Rust core (`crates/*`, which remains metered with fuel and cancellation). `--backend x64` executes unmetered for `pen run` builds (`pen dev`, `pen test`, mods, and any epoch-deadline `dev_engine()` stay on Wasmtime even when `PEN_WTIME_AOT=1`, enforced by `!samples(engine)` in Pen's `GuestModule::new_with_aot`).

### 2.1 Target ISA Policy & Runtime CPUID Gate
- **Baseline required ISA (always emitted)**: Linux x86-64 + SSE2 + SSE4.1 (`roundss`/`roundsd`, `pextrq`/`pinsrq`) + POPCNT.
- **Optional ISA (`TargetIsa { has_abm_bmi1: bool }`)**:
  - Threaded explicitly into `lower::lower_ir_to_x64`.
  - When `has_abm_bmi1 == true`: `i32/i64.{clz,ctz}` emit single-instruction `lzcnt`/`tzcnt`.
  - When `has_abm_bmi1 == false`: `lower.rs` emits branchless portable `bsr`/`bsf` + `cmovz` (`63^31=32` / `127^63=64` for `clz`, `32`/`64` for `ctz`).
  - How `--target-cpu` resolves `TargetIsa` under `#![forbid(unsafe_code)]`:
    - `--target-cpu native` (default): `cfg!(target_arch = "x86_64") && std::is_x86_feature_detected!("lzcnt") && std::is_x86_feature_detected!("bmi1")`.
    - `--target-cpu x86-64-v2` / `generic`: `has_abm_bmi1 = false`.
    - `--target-cpu x86-64-v3` / `x86-64-v4` / `znver*` / `haswell`+: `has_abm_bmi1 = true`.
  - **Cache key**: `build.rs` and Pen's fast cache MUST key artifacts by the **resolved ISA profile** (e.g. `x64-v3` vs `x64-v2`), not raw `"native"`.
- **Mandatory `blot_aot_new` CPUID check** (preserving SysV callee-saved `Rbx` AND live entry arguments `Rdi, Rsi, Rdx, Rcx` across `cpuid`!):
  1. `eax = 0; cpuid` -> require `eax >= 1`; if `has_abm_bmi1`, also require `eax >= 7`.
  2. `eax = 1; cpuid` -> require `ECX` bit 19 (SSE4.1) AND bit 23 (POPCNT).
  3. If `has_abm_bmi1`:
     - `eax = 0x8000_0000; cpuid` -> require `eax >= 0x8000_0001`.
     - `eax = 0x8000_0001; cpuid` -> require `ECX` bit 5 (ABM / LZCNT).
     - `eax = 7; ecx = 0; cpuid` -> require `EBX` bit 3 (BMI1 / TZCNT).
  4. On any missing feature: set `*trap_out = TRAP_UNSUPPORTED_ISA (22)` (if non-null) and return `NULL`; `blot_aot_trap_message(22)` returns `"Unsupported CPU ISA"`.

### 2.2 Explicit Admission Contract (Enforced in `x64/module.rs::admit_x64_module`)
All Wasm bytes pass `wtime_validate::validate` first; `admit_x64_module` then enforces ALL constraints **before** any IR or codegen (including scanning **unreachable** instructions in every body):
- **Target**: `--blot-cdylib` only in Stage 1.
- **Memories**: `0` or `1` Wasm32 memory (`IndexType::I32`, non-shared, standard 64 KiB pages `page_size_log2 == 16`, `min <= max`, omitted `max` defaults to `65_536`, `max <= 65_536`). Memory64, multi-memory, shared memory, custom page sizes rejected.
- **Tables**:
  - Static 32-bit `funcref` tables (`element == nullable funcref`, no non-null `table_inits`, `limits.min <= 1_048_576`, initialized only by active element segments with constant `i32` offsets and `ref.func` / `ref.null` entries).
  - Mutable 32-bit `externref` tables (`element == nullable externref`, `min == max <= 65_536`, accessed only by `table.get`/`table.set`, no active element segments).
  - Canonicalized `(table, FuncType)` indirect-call dispatch tables with **aggregate** `.rodata` cap `sum(ci.span * 4) <= 64 MiB` (returns clean error if exceeded).
  - Passive/declarative element segments and table bulk/grow ops rejected.
- **Data segments**: active memory-0 segments with constant `i32` offsets only; passive segments rejected.
- **Imports**: function imports only (no imported memories/tables/globals/tags), with Blot ABI 8 compatible signatures (`()`, `(ref)`, `(i32)`, `(f32)`, `(ref, i32|i64|f32|f64|ref)` -> `()` or single `i32|i64|f32|f64|ref`; note Pen's `blot-host` further validates `blot:host/1::call_<param>_<res>` with `(externref, i32|f32) -> i32|f32` where `Unit` maps to `i32`).
- **Globals**: `i32`, `i64`, `f32`, `f64`, `v128`, or nullable `externref` with constant initializers (`const`, `v128.const`, `ref.null`, prior `global.get`, `i32`/`i64` `add`/`sub`/`mul`).
  - **Folding rule**: a global is folded into `global_consts[k] = Some(X64ConstVal)` iff it is immutable OR (`writes == 0 && !exported_mutable`). **Exported mutable globals are NEVER folded** so `blot_aot_set_global` / `Guest::restore` always work. Every global with `global_consts[k].is_none()` receives a checked `Ctx` storage slot.
- **Exports**:
  - Functions (all admitted; `blot_aot_call_export` dispatches any exported function — including re-exported imports — whose signature has `0..=1` scalar/ref param and `0..=1` scalar/ref result; `blot:allocate`/`blot:reset` require `(i32)->(i32)` and `blot:finish_invocation` requires `()->(i32)`).
  - Globals (all admitted, including folded constants).
  - Memory 0 (admitted — required for Pen's `blot:memory` export on array guests such as `opt-palm`!).
  - Exported tables/tags rejected.
- **Value types & opcodes** (checked across EVERY body including unreachable code):
  - Value types: `I32`, `I64`, `F32`, `F64`, `V128`, nullable `ExternRef`.
  - Opcodes: MVP (`0x00..=0x05`, `0x0b..=0x11`, `0x1a..=0x1c`, `0x20..=0x26`, `0x28..=0x44`, `0x45..=0xc4`, `0xd0`, `0xd1`), saturating truncs (`0xfc00..=0xfc07`), memory-0 `memory.copy`/`memory.fill` (`0xfc0a`/`0xfc0b`), minimal `v128` subset (`0xfd00`, `0xfd0b`, `0xfd0c`, `0xfd0f..=0xfd12`, `0xfd1d`, `0xfd1e`). All other opcodes rejected.
- **Resource/overflow caps** (return clean `Err`, never panic/wrap):
  - `MAX_LOCALS_PER_FN = 65_536`, `MAX_TOTAL_LOCALS = 1_000_000`, `MAX_FRAME_BYTES = 16 MiB`, `MAX_LIVENESS_BYTES = 64 MiB`, `ValRange.len: u32`, checked `i32::try_from` on every `rel32` displacement (total `.rodata + .text < 2 GiB`).

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
  - `HELPER_MEM_GROW` failure returns `mov eax, -1` (`0x0000_0000_ffff_ffff`, NOT `-1i64`!);
  - All 8 `I32` `HELPER_TRUNC_BASE` routines produce 32-bit zero-extended `Rax` (`mov eax, eax` / 32-bit `cvtt*2si eax`);
  - `wt_import{k}` trampolines zero-extend `[I32]` results (`mov eax, eax`).

### 3.2 Dedicated `mmap` Guest Stack (Exact Small-Stack & Reentry Safety Without `libc`)
Instead of guessing remaining pthread stack size via `rsp - 1 MiB`, `blot_aot_new` allocates a dedicated **8 MiB** guest stack with a **64 KiB `PROT_NONE` bottom guard** (anonymous demand-paged: unused pages consume zero physical RSS):
- Allocation: `mmap(NULL, 64 KiB + 8 MiB, PROT_NONE, MAP_PRIVATE | MAP_ANONYMOUS | MAP_NORESERVE, -1, 0)` followed by `mprotect(stack_base + 64 KiB, 8 MiB, PROT_READ | PROT_WRITE)`. On failure, `munmap` any prior allocations and return `TRAP_ALLOC_FAILED (12)`.
- `stack_top = stack_base + 64 KiB + 8 MiB` (16-byte aligned).
- `stack_floor = stack_base + 64 KiB + 4 KiB` (`4 KiB` explicit headroom for `call` return address + internal helper/wrapper frames — well above the `PROT_NONE` guard).
- **Stack switching protocol (100% reentrant, zero single-slot clobber)**:
  1. **Top-level C ABI entry** (`blot_aot_new` -> `wt_initialize`, `blot_aot_call_export`, `blot_aot_call_allocate`, `blot_aot_call_reset`, `blot_aot_call_finish_invocation`):
     - On host stack: save SysV callee-saved `Rbx, Rbp, R12..R15`, allocate a 16-byte-aligned local host frame, save host `MXCSR` into `[rsp + 0]` (`stmxcsr [rsp + 0]`), and load Wasm `MXCSR` (`0x1f80`); load `R15=ctx, R14=mem_base, R13=mem_len, R12=g_pinned`.
     - Switch to guest stack: store `ctx->host_rsp = rsp` (pointing at `[host_mxcsr, ...]`), save current host `rsp` into `[stack_top - 8]`, set `rsp = stack_top - 16` (16-byte aligned before `call`), `call target`, restore `rsp = [stack_top - 8]`.
     - On host stack: store `R12` back to `ctx->g_pinned` (on **both** normal return and trap), restore host `MXCSR` (`ldmxcsr [rsp + 0]`), free local host frame, pop callee-saved regs, return status. (On `blot_aot_new` `wt_initialize` failure, switching back to the host stack happens **before** `munmap`ing the guest stack, linear memory, and `Ctx`.)
  2. **Host callback (`HELPER_HOST_CALL`, invoked from `wt_import{k}`)**:
     - On guest stack: allocate a 32-byte guest frame saving previous `OFF_ACTIVE_GUEST_RSP`, previous `OFF_HOST_RSP`, and callee registers as needed; store current `guest_rsp` into `[r15 + OFF_ACTIVE_GUEST_RSP]`; publish `R12` to `ctx->g_pinned`.
     - Switch `rsp = [r15 + OFF_HOST_RSP]` (the active outer wrapper's 16-byte-aligned host stack frame), execute `ldmxcsr [rsp + 0]` to restore the outer host `MXCSR`, subtract a 32-byte host call frame (keeping 16-byte alignment before `call`), build `ActiveCall { ctx = R15, mem = R14 }` + `u64 res = 0` on the host stack, and call `ctx->host_fn(user_ctx, &ac, idx, tok, arg, &res)` via SysV ABI on the **real host stack** (full host stack available).
     - After `host_fn` returns: capture `res` and `eax` status, switch `rsp` back to saved `guest_rsp`, reload Wasm `MXCSR` (`0x1f80`), reload `R13 = ctx->mem_len` and `R12 = ctx->g_pinned`, restore saved `OFF_ACTIVE_GUEST_RSP` and `OFF_HOST_RSP`, return `Rax = res` and `R11d = 0 | TRAP_HOST (19)`.
  3. **Active reentrant call** (`blot_aot_active_allocate`, `blot_aot_active_reset`):
     - On host stack: save SysV callee-saved `Rbx, Rbp, R12..R15`, allocate a 16-byte-aligned local host frame, save host `MXCSR` at `[rsp + 0]`, load `R15 = ac->ctx`, `R14 = ac->mem`, `R13 = ctx->mem_len`, `R12 = ctx->g_pinned`, and load Wasm `MXCSR` (`0x1f80`).
     - Resume on the **same guest stack** right below suspended `guest_rsp = ([r15 + OFF_ACTIVE_GUEST_RSP] & !15) - 32`: check `new_guest_rsp >= ctx->stack_floor` (else restore host `MXCSR`, pop callee-saved regs, and trap `TRAP_CALL_STACK` immediately); save previous `ctx->host_rsp` and current host `rsp` in the 32-byte guest-stack link record, set `ctx->host_rsp = host_rsp`, switch `rsp = new_guest_rsp`, `call blot:allocate/reset`, restore `ctx->host_rsp` and host `rsp`, write back `R12` to `ctx->g_pinned` (normal AND trap), restore local host `MXCSR` (`ldmxcsr [rsp + 0]`), pop callee-saved regs, return status.
  4. **Pre-prologue function check** (already implemented in `lower.rs:1645-1662`): checks `lea r11, [rsp - (frame_size + 8*callee_saves)]; cmp r11, [r15 + OFF_STACK_FLOOR]; jb .early_stack_trap` **before** any push or frame subtraction.
  5. **`blot_aot_drop`**: `munmap`s guest stack (`64 KiB + 8 MiB`), guest memory (`mem_reserve_bytes`, if non-zero), and `Ctx` (`handle_alloc_bytes`).

### 3.3 `Ctx` (`BlotHandle`) Byte-for-Byte Layout
Every non-folded global (`global_consts[k].is_none()`) gets a slot at `global_offs[k]`:

| Offset | Size | Field | Notes |
|---:|---:|---|---|
| `0` | 8 | `mem_base: *mut u8` | Fixed virtual reservation base (`NULL` if 0 capacity) |
| `8` | 8 | `mem_len: usize` | Current accessible byte length (`pages * 65536`) |
| `16` | 8 | `mem_max_pages: u64` | Clamped max pages (`<= 65536`) |
| `24` | 8 | `mem_reserve_bytes: usize` | Exact reservation passed to `munmap` (`0` if no reservation) |
| `32` | 8 | `handle_alloc_bytes: usize` | Page-aligned `Ctx` size for `munmap` |
| `40` | 8 | `stack_floor: usize` | `guest_stack_base + 64 KiB + 4 KiB` (immutable after `new`) |
| `48` | 8 | `host_user_ctx: *mut c_void` | Passed as `host_fn` arg 0 |
| `56` | 8 | `host_fn: Option<BlotHostFn>` | Host callback pointer |
| `64` | 8 | `scratch_tmp: u64` | Internal scratch (`call_indirect` `idx` across arg moves) |
| `72` | 8 | `ret_spill_ptr: *mut u8` | Caller-owned overflow multi-value return buffer pointer |
| `80` | 8 | `guest_stack_base: *mut u8` | Base of `64 KiB + 8 MiB` `mmap` guest stack (for `munmap`) |
| `88` | 8 | `guest_stack_top: usize` | `guest_stack_base + 64 KiB + 8 MiB` |
| `96` | 8 | `host_rsp: usize` | Active outer host `rsp` (points to `[host_mxcsr_slot, ...]`) |
| `104` | 8 | `active_guest_rsp: usize` | Suspended guest `rsp` during `HELPER_HOST_CALL` (for `active_*`) |
| `112..` | var | Non-folded globals | 16-byte aligned base; 8 B each for scalars/refs, 16-byte aligned 16 B for `V128` |
| `..` | var | Mutable `externref` tables | 8-byte aligned, `len * 8` bytes each |

Total rounded up to 4096 bytes. `CtxLayout` & `RodataConsts` exact Rust fields (matching `lower.rs`):
- `CtxLayout { pinned_global: Option<u32>, global_offs: Vec<i32>, extern_tables: BTreeMap<u32, (i32, u32)>, ci_tables: Vec<CiTableLayout>, total_bytes: usize }`
- `CiTableLayout { table_len: u32, min_slot: u32, span: u32, rodata_off: u32 }`
- `RodataConsts { f32_abs: u32, f32_neg: u32, f64_abs: u32, f64_neg: u32, f32_qnan: u32, f64_qnan: u32, mxcsr_wasm: u32, ... }` — **Invariant**: `f32_abs`, `f32_neg`, `f64_abs`, `f64_neg` MUST each be **16-byte aligned AND 16 bytes wide** (four `u32` or two `u64` lanes) because `lower.rs` emits 128-bit `andps`/`xorps`/`andpd`/`xorpd` RIP-relative memory operands!

### 3.4 Memory Lifecycle & Linux Syscall Rules
- Linux syscalls use `(rax=nr, rdi, rsi, rdx, r10, r8, r9)`, clobber `rcx` & `r11`, and signal errors when `(rax as u64) >= (-4095i64 as u64)`. Keep syscall setup strictly isolated from SysV C call wrappers.
- **`blot_aot_new` memory limits** (exact match to `cgen.rs:4350-4355` and Pen `wtime_aot.rs:584-587`):
  - If module has **no memory**: `mem_base = NULL, mem_len = 0, mem_max_pages = 0, mem_reserve_bytes = 0` (no memory `mmap`).
  - If module has memory `(mem_min, declared_max)`:
    - `limit = if mem_limit_pages == 0 { 65_536 } else { mem_limit_pages }` (**critical**: `0` means unlimited in Blot ABI v1!).
    - If `mem_min > limit`: fail with `TRAP_MEM_LIMIT (11)`.
    - `max_p = min(declared_max, limit, 65_536)`.
    - If `mem_min > max_p`: fail with `TRAP_MEM_TOO_LARGE (9)`.
    - If `max_p == 0` (genuine zero-capacity memory): `mem_base = NULL, mem_len = 0, mem_max_pages = 0, mem_reserve_bytes = 0`.
    - Else: `mmap(NULL, max_p * 65536, PROT_NONE, MAP_PRIVATE | MAP_ANONYMOUS | MAP_NORESERVE, -1, 0)`; if `mem_min > 0`, `mprotect(mem_base, mem_min * 65536, PROT_READ | PROT_WRITE)`. Note: `MAP_NORESERVE` defers physical backing-page commit until first touch; on OOM under strict overcommit, `mprotect` or writes may fail — `mprotect` errors cleanly return `TRAP_ALLOC_FAILED (12)` (in `new`) or `-1` (in `grow`) and clean up all prior `mmap`s!
- **`memory.grow` vs `blot_aot_mem_grow`**:
  - Shared `grow` core checks `delta == 0` (returns `old_pages` immediately without `mprotect`), `delta <= max_pages && old_pages <= max_pages - delta`, runs `mprotect(mem_base + old_len, delta * 65536, PROT_READ | PROT_WRITE)`, and updates `ctx->mem_len` & `R13` **only after `mprotect` succeeds**.
  - `HELPER_MEM_GROW` (Wasm `memory.grow`, `Rdi = u32 delta`): returns zero-extended `0xffff_ffffu64` (`mov eax, -1`) on failure, or `old_pages` in `Eax` on success.
  - `blot_aot_mem_grow(h, delta: u64) -> i64`: validates full 64-bit `delta` (never truncates to `u32`!), returns signed `-1i64` (`0xffff_ffff_ffff_ffff`) on failure or `old_pages as i64` on success.

### 3.5 Exact Helper Clobber Contracts & Complete 18-Export Blot ABI 8 Table
- **Pure-guest helpers (`regalloc::HelperKind`)** preserve `Rbx, Rbp, R12..R15` (except `MemGrow` updating `R13`) and **all `Xmm0..Xmm13`**:
  - `HELPER_TRUNC_BASE + 0..15` (`HelperKind::Trunc`): `Xmm15 -> Rax` (32-bit zero-extended for `I32`, 64-bit for `I64`), `R11d = 0 | 6 | 7`; clobbers ONLY `Rax, R10, R11, Xmm14, Xmm15` (`gp_mask = MASK_RAX, xmm_mask = 0`).
  - `HELPER_MEM_GROW (16)` (`HelperKind::MemGrow`): `Rdi (u32) -> Rax (u32 zero-extended)`; clobbers `MASK_GP_CALLER, xmm_mask = 0`.
  - `HELPER_MEM_COPY (17)` & `HELPER_MEM_FILL (18)`: `(Rdi, Rsi, Rdx) -> R11d = 0 | TRAP_OOB`; overlap-safe forward (`rep movsb`) / backward loop (no `std`/`cld`); clobbers `MASK_GP_CALLER, xmm_mask = 0`.
- **`HELPER_HOST_CALL (19)`** (separate contract — NOT in `HelperKind`, invoked ONLY via `wt_import{k}` through `Op::Call` which already clobbers `MASK_GP_CALLER | MASK_XMM_CALLER`):
  - Inputs: `Edi = import_idx`, `Rsi = tok (u64)`, `Rdx = arg_bits (u64)`.
  - Output: `Rax = res_bits (u64)`, `R11d = 0 | TRAP_HOST (19)`.
  - Publishes `R12`, switches to host stack, restores host `MXCSR`, calls `host_fn`, switches back to guest stack, reloads `0x1f80` `MXCSR`, reloads `R13` & `R12`.
- **`wt_import{k}` param/result marshaling** (exact match to `cgen.rs:4097-4117`):
  - Params: `[] -> (0, 0)`, `[Ref] -> (rdi, 0)`, `[I32] -> (0, edi)`, `[F32] -> (0, movd xmm0)`, `[Ref, I32] -> (rdi, esi)`, `[Ref, I64] -> (rdi, rsi)`, `[Ref, F32] -> (rdi, movd xmm0)`, `[Ref, F64] -> (rdi, movq xmm0)`, `[Ref, Ref] -> (rdi, rsi)`.
  - Results: `[] -> ()`, `[I32] -> mov eax, eax`, `[I64]|[Ref] -> rax`, `[F32] -> movd xmm0, eax`, `[F64] -> movq xmm0, rax`.
- **Instantiation order (`wt_initialize`)**: (1) initialize all non-folded globals from `global_inits`; (2) validate ALL active element segment bounds against table lengths (trapping `TRAP_TABLE_OOB (3)` before any data write or `start`, matching Wasm spec); (3) copy active data segments in declaration order (checking `off + len <= mem_len`, trapping `TRAP_OOB (2)`, allowing overlapping segments in exact order); (4) if `module.start` is present, call `start` on the guest stack (`start` calling an import works cleanly under a C host; under Pen `AotGuest::start`, `active == None` so Pen cleanly rejects start-time imports with `stale_capability` — test both!).

| # | Export Symbol | Exact C Signature | Return / Status Contract & Null-Pointer Rules |
|---:|---|---|---|
| 1 | `blot_aot_abi_version` | `() -> u32` | Returns `1` |
| 2 | `blot_aot_trap_message` | `(u8 code, *mut *const u8 ptr, *mut usize len) -> i32` | `0` if `code in 1..=22` (writes non-null `ptr`/`len` via RIP-relative `.rodata` + static len, no `strlen`), else `1` |
| 3 | `blot_aot_import_count` | `() -> u32` | Returns `imports.len() as u32` |
| 4 | `blot_aot_import_name` | `(u32 idx, *mut *const u8 mp, *mut usize ml, *mut *const u8 np, *mut usize nl) -> i32` | `0` if `idx < count` (writes explicit `(ptr, len)` pairs to non-null outs), else `1` |
| 5 | `blot_aot_new` | `(*mut c_void user_ctx, Option<HostFn>, u64 mem_limit_pages, *mut u8 trap_out) -> *mut Ctx` | Returns handle or `NULL` (writing `*trap_out` if non-null); runs CPUID gate, allocates `Ctx` + guest stack + memory, runs `wt_initialize`, cleans up all `mmap`s on any failure |
| 6 | `blot_aot_drop` | `(*mut Ctx h) -> void` | No-op if `NULL`; `munmap`s guest stack, memory (if non-zero), and `Ctx` |
| 7 | `blot_aot_call_export` | `(*mut Ctx h, *const u8 np, usize nl, u64 arg_bits, *mut u64 res_out, *mut u8 trap_out) -> i32` | Matches `(nl, np)` (checks `nl` before reading `np`); zero-extends narrow `I32`/`F32` args and results; returns `0` ok (writes `*res_out` if non-null, `0` for void), `1` trap, `2` `TRAP_HOST`, `3` unknown/incompatible export |
| 8 | `blot_aot_call_allocate` | `(*mut Ctx h, i32 arg, *mut i32 res_out, *mut u8 trap_out) -> i32` | Calls `blot:allocate` if exported `(i32)->(i32)` (zero-extends `arg`, writes 32-bit `*res_out`); returns `0`/`1`/`2`/`3` |
| 9 | `blot_aot_call_reset` | `(*mut Ctx h, i32 arg, *mut i32 res_out, *mut u8 trap_out) -> i32` | Calls `blot:reset` if exported `(i32)->(i32)`; returns `0`/`1`/`2`/`3` |
| 10 | `blot_aot_call_finish_invocation` | `(*mut Ctx h, *mut i32 res_out, *mut u8 trap_out) -> i32` | Calls `blot:finish_invocation` if exported `()->(i32)`; returns `0`/`1`/`2`/`3` |
| 11 | `blot_aot_mem_data` | `(*mut Ctx h, *mut *mut u8 ptr_out, *mut usize len_out) -> void` | Writes `h->mem_base` / `h->mem_len` to non-null outs |
| 12 | `blot_aot_mem_grow` | `(*mut Ctx h, u64 delta) -> i64` | Full 64-bit `delta`; returns `old_pages as i64` or `-1i64` |
| 13 | `blot_aot_get_global` | `(*mut Ctx h, *const u8 np, usize nl, *mut u64 val_out, *mut u8 kind_out) -> i32` | Serves both `Ctx` and folded-const exports (`kind`: `0=I32, 1=I64, 2=F32, 3=F64, 4=Ref`; `V128` exceeds `u64` -> returns `1`); returns `0` found, `1` missing |
| 14 | `blot_aot_set_global` | `(*mut Ctx h, *const u8 np, usize nl, u64 val) -> i32` | Writes exported mutable scalar/ref global (`V128` -> returns `1`); returns `0` updated, `1` missing/immutable |
| 15 | `blot_aot_active_mem_data` | `(*mut ActiveCall ac, *mut *mut u8 ptr_out, *mut usize len_out) -> void` | Reads `ctx = ac->ctx`, writes `ac->mem` / `ctx->mem_len` to non-null outs |
| 16 | `blot_aot_active_allocate` | `(*mut ActiveCall ac, i32 arg, *mut i32 res_out, *mut u8 trap_out) -> i32` | Reentrant on suspended guest stack preserving `stack_floor`; syncs `R12` on ok & trap; returns `0`/`1`/`2`/`3` |
| 17 | `blot_aot_active_reset` | `(*mut ActiveCall ac, i32 arg, *mut i32 res_out, *mut u8 trap_out) -> i32` | Same as `active_allocate` for `blot:reset`; returns `0`/`1`/`2`/`3` |
| 18 | `blot_aot_active_get_global` | `(*mut ActiveCall ac, *const u8 np, usize nl, *mut u64 val_out, *mut u8 kind_out) -> i32` | Same as `blot_aot_get_global` on `ac->ctx`; returns `0`/`1` |

### 3.6 ELF64 Linking & Relocation Bases (`tools/wtime-aot/src/x64/elf.rs`)
- **Conforming ELF64 `ET_DYN`**: 3 page-separated `PT_LOAD` segments (`R`: Ehdr + Phdrs + `.dynsym` + SysV `.hash` + `.dynstr` + `.rodata`; `RX`: `.text`; `RW`: `.dynamic`) + `PT_DYNAMIC` + `PT_GNU_STACK` (`PF_R | PF_W`), plus an 8-entry Section Header Table (`.dynsym`, `.hash`, `.dynstr`, `.rodata`, `.text`, `.dynamic`, `.shstrtab`) so every `.dynsym` export carries real `st_shndx = 5` (`.text`).
- **Two explicit `rel32` displacement bases** (every conversion checked via `i32::try_from`):
  1. **Instruction relocations (`Reloc`)**: `(target_vaddr - (fn_vaddr + next_ip_off)) as i32` (relative to the end of the instruction).
  2. **Jump tables (`JumpTablePatch`) & `call_indirect` dispatch tables**: `(target_vaddr - table_base_vaddr) as i32` (relative to the first byte of the table in `.rodata`; `0` sentinel in `call_indirect` tables unambiguously marks empty/type-mismatched slots because `.text` starts strictly after `.rodata`).

### 3.7 Current WIP Files (`tools/wtime-aot/src/x64/`) — All Verified Zero-Error Typecheck
1. `module.rs`: `X64Module`, `X64ConstVal`, `admit_x64_module` enforcing §2.2 (including `blot:memory`, canonical `ci_sigs`, aggregate caps, non-folded exported mutable globals, unreachable-code opcode/type checks).
2. `asm.rs`: x86-64 + SSE2 + SSE4.1 + POPCNT + LZCNT/TZCNT + `bsr`/`bsf`/`cpuid`/`stmxcsr`/`ldmxcsr` encoder (with documented `mov_ri*(r, 0)` `RFLAGS` caveat).
3. `elf.rs`: Keyed `ExportDef<'a>`, exact `planned_rodata_len` check, non-empty `.text` & export extent validation, duplicate-name rejection, conforming 8-section header table.
4. `ir.rs`: Decoupled `lower_wasm_to_ir(decoded, &X64Module, func_index)`, fixed single-value block types (`-1..-5 | -16 | -17`), `u32` `ValRange.len` with checked bounds, constant folding, `CmpBranch` fusion, trap-order-preserving linear load-run check hoisting, DCE.
5. `regalloc.rs`: Unified `layout_args`/`layout_rets` (16-byte aligned `V128` + caller-owned overflow return buffer), unified `HelperKind`/`helper_clobbers`, checked `MAX_FRAME_BYTES`/`MAX_LIVENESS_BYTES`, RPO + cross-block `BitSet` liveness + clobber-aware linear scan.
6. `lower.rs`: Disjoint parallel-move scratches (`R11`/`Xmm15` cycle break, `R10`/`Xmm14` mem-to-mem), `TargetIsa` with branchless `bsr`/`bsf`+`cmovz` fallback, fixed shift `RCX` staging, fixed spilled `u64 -> fp` sticky-bit capture, unsigned 32-bit `call_indirect`/`table`/`br_table` bounds checks, checked pre-prologue stack check, caller-owned overflow returns.

---

## 4. Remaining Work — Stage 1 (First Complete Working Milestone)

### 4.1 Implement `tools/wtime-aot/src/x64/mod.rs`
1. Wire `admit_x64_module` -> `CtxLayout` -> `RodataConsts` -> emit shared runtime helpers (`HELPER_TRUNC_BASE + 0..15`, `HELPER_MEM_GROW`, `HELPER_MEM_COPY`, `HELPER_MEM_FILL`, `HELPER_HOST_CALL`), `wt_import{k}` trampolines, `wt_initialize`, and all 18 Blot ABI 8 C exports (§3.2–§3.5).
2. Compile all defined functions in parallel via bounded `std::thread::scope` (`--jobs N`, default `available_parallelism`) into an indexed `Vec<Option<Asm>>` for **100% deterministic** `.text`/`.rodata` output order.
3. Merge per-function `local_rodata` preserving 16-byte alignment, call `ElfLayout::plan`, resolve every `Reloc`, `JumpTablePatch`, and `call_indirect` table entry with checked `i32::try_from`, call `ElfLayout::emit`, and atomically publish (`tmp.{pid}.{seq}` -> `rename`).
4. Emit `WTIME_AOT_TIMINGS=1` phase breakdown: `validate`, `admit`, `ssa_opt`, `regalloc`, `emit`, `link`, `write`, `total`.

### 4.2 Wire CLI (`main.rs`, `build.rs`) & Pen Caching Rules
- Direct CLI: `wtime-aot module.wasm -o out.so --blot-cdylib --backend x64 [--target-cpu NAME]` writes the `.so` file directly to `-o` (atomic temp + rename). Reject `--backend x64` without `--blot-cdylib` in Stage 1.
- Cached CLI: `wtime-aot build module.wasm --cache DIR --blot-cdylib --backend x64 [--target-cpu NAME] [--jobs N]` ignores `--toolchain`/`--c-opt`, rejects `--train`, keys cache by `("wtime-aot build x64 1", wasm, generator_exe, resolved_isa_tag)`, and publishes `library.so` atomically via unique temp file + rename.
- Pen (`crates/blot-host/src/wtime_aot.rs`, `lib.rs`): passes `--toolchain` only for `rust`, `--c-opt` only for `c`, `--target-cpu` for `x64`, keys fast index `by-wasm/*.path` by `(digest, backend, profile/resolved-isa, bin_len, bin_mtime_s_ns)` with atomic temp+rename publication, keeps `dev_engine()` on Wasmtime via `!samples(engine)` even when `PEN_WTIME_AOT=1`, and eliminates all `&mut AotHostState` / `Box<AotHostState>` aliasing across FFI and `active_allocate` via raw-pointer `AotHostBox { in_trampoline: AtomicBool, state: UnsafeCell<AotHostState> }` with short non-FFI borrows. Keep explicit `PEN_WTIME_AOT_BACKEND=x64` errors surfaced diagnostically (never silently swallowed); only fallback to Wasmtime/C on admission/platform error if a future auto-default policy explicitly opts in.

### 4.3 Stage 1 Verification & Measurement Matrix
1. **Unit & differential tests** (`cargo test --locked --manifest-path tools/wtime-aot/Cargo.toml`, plus explicit Clippy & test entries in any task-owned verification contract):
   - **ELF/glibc smoke**: `dlopen`, `dlsym` all 18 exports + synthetic SysV hash collision, verify ASLR load + `/proc/self/maps` segment perms (`R`, `RX`, `RW`, non-executable stack).
   - **Opcode & lowering corner cases** (tested with **both** `has_abm_bmi1 = true` and `false`!):
     - Single-value blocks (`-1..-5 | -16 | -17`), implicit-else + loop-carried locals + unreachable blocks.
     - Shifts/rotates with `a` dying in `RCX` (reg & spilled count: `3 << 5 == 96`).
     - Spilled `f32.convert_i64_u(0x8000_0080_0000_0001)` and `f64.convert_i64_u(0x8000_0000_0000_0401)` exact rounding.
     - Parallel moves: 3-way GP/XMM cycles, pure stack-to-stack swaps (`Stack(0) <-> Stack(8)`), duplicate sources, mixed-width loop-edge phis.
     - Multi-value ABI: >6 GP args, 9th & 10th `v128` stack args non-overlapping, `24 i64` overflow returns, recursive multi-value calls.
     - All 16 float-to-int truncations (`NaN`, `±Inf`, `-0.0`, `-0.5`, `-1.0`, `INT_MIN/MAX`, `UINT_MAX`), `min`/`max` (`-0.0`/`+0.0`, arithmetic NaN per Wasm spec), unordered float cmp, `INT_MIN % -1 == 0`, `div_s` overflow/zero traps.
     - Live `Xmm0` value surviving across every `HelperKind` (`Trunc`, `MemGrow`, `MemCopy`, `MemFill`).
     - `call_indirect`: hit, OOB (`TRAP_TABLE_OOB`), null hole (`TRAP_INDIRECT_CALL`), signature mismatch (`TRAP_INDIRECT_CALL`), and imported function table target.
     - Memory & syscalls: `blot:memory` export, `mem_limit_pages == 0` (unlimited) via real `AotGuest::start(None)`, genuine 0-capacity vs 0-memory modules, `grow(0)`, `grow` > `u32::MAX` via `blot_aot_mem_grow`, `grow` failing `mprotect` leaves `mem_len` unchanged, `mmap`/`mprotect` failure cleanup in `new`, `offset > i32::MAX` store with spilled val, overlapping forward/backward `memory.copy`, load-run trap ordering (`load A; load B_oob; load A+off_oob`).
     - Dedicated stack, `MXCSR`, & reentry: recursive `TRAP_CALL_STACK` on a small 128 KiB host pthread (including large-frame callee and active callback reentry), non-default host `MXCSR` preserved across calls/traps/reentry, `active_allocate`/`active_reset` mutating `R12`/`R13`, nested host-callback rejection inside `active_allocate`, `start` calling an import (both C host pass and Pen `stale_capability` reject), `blot_aot_set_global` on exported mutable globals (including never-written-by-Wasm globals).
2. **Frozen Stage 1 benchmark & parity protocol**:
   - Inputs: `opt-palm` (`c958c20386207b3ea9713d5aae596edc264328ceaa87281b5cf6cc333db2b89b`) and `gdev-dev` (`25ebb19d8cb6adc2fc6dd9545e647baa88c29e0e021d7d274a511b4c6612f77b`), pinned CPU (`taskset`), fixed `--jobs`, recorded git revision & Wasmtime config.
   - Separate timings for: cache-miss CLI cold compile (with `WTIME_AOT_TIMINGS=1` phase breakdown), `dlopen`, `blot_aot_new` instantiation, and first frame call.
   - Stage 1 hypothesis: **~100 – 250 ms** cold compile on `opt-palm`, **~0.60 – 0.85×** Wasmtime runtime initially (before Stage 2 inlining/coalescing/LICM), **100% exact SHA-256 parity** across all 5 `opt-palm` workloads via `cpu-bench` (`.../scratch/pen-exp/cpu-bench/target/release/cpu-bench`) and headless `pen` (`colony.penrec` + `gdev` `pen run`).

---

## 5. Stage 2 — Runtime Optimization Hill-Climb (Dependency-Ordered & Numerically Gated)

### 5.1 Pass Ordering & Legality Rules
Every pass is evaluated independently (phase compile cost + paired 5-workload `opt-palm` runtime + exact parity) before combining into the Stage 2 candidate:
1. **Profile Stage 1 first**: measure compile-time tail on `gdev-dev` (`scan_frame_locals`, liveness, clobber scan, `O(S^2)` spill reuse, `O(M^2)` parallel moves) and hot functions/spills/bounds-check cost on `opt-palm`.
2. **Budgeted small-leaf inlining on IR** (runs **first** so all downstream passes optimize inlined bodies): non-recursive hot helpers (`<= 45` insns, bounded total code growth, keeping original functions for indirect/exported callers).
3. **CFG rebuild & verification -> true dominator tree (`Cooper–Harvey–Kennedy`) & effect analysis** (tracking global writes, memory writes, traps, and `memory.grow`/host callback effects).
4. **Dominator GVN / CSE + redundant `GlobalGet` elimination + conservative LICM** (respecting call/callback effects and trap precision).
5. **Loop & cross-block bounds-check elimination + addressing-mode folding**: dominator `checked_max` propagation, counted-loop induction-variable range hoisting, `[r14 + base + idx*scale + disp]` wrap-safe folding *(fallback to evaluate if explicit checks remain the dominant gap vs `--backend c`: bounded `SIGSEGV` guard-page experiment coordinated with host signal chaining)*.
6. **SSA cleanup (DCE/copy-prop) -> Register coalescing & ABI/loop-phi hinting + loop-depth weighted regalloc** (critical lever to cut loop-edge and call-boundary move traffic) + **callee-saved `Xmm8..Xmm13`** (internal convention preserved by generated callees and saved/restored around `host_fn` in `HELPER_HOST_CALL`; SysV C callers already treat XMMs as caller-saved).
7. **Cheap instruction-selection peepholes**:
   - `Cmp + Select -> cmp + cmovcc` (handling unordered FP `PF` cleanly!).
   - Constant signed/unsigned `div`/`rem` -> magic multiply + shift (preserving exact divide-by-zero & `INT_MIN / -1` trap semantics).
   - Inline small constant-length `memory.copy` / `memory.fill` (`<= 64 B`) with widened bounds check before any write, preserving overlap and `len == 0` semantics.
   - Memory-operand ALU (`add/sub/cmp r, [rsp+spill]`).
   - Direct-call trap-check elision (`!fn_can_trap` callees — keeping full `R11d` trap ABI for indirect, exported, and unchecked entrypoints!).
   - **2-phase native-frame stack-check elision**: parallel (SSA + regalloc) -> sequential acyclic max-native-frame propagation (`allocated_frame_bytes + callee_saves + max_callee_chain <= 4 KiB` leaf bound) -> parallel lowering.

### 5.2 Numeric Promotion Gate (Before Flipping `pen run` Default from `c` → `x64`)
Paired alternating measurements on pinned hardware against **both** Wasmtime/Cranelift and `--backend c --c-opt auto` (recording 95% confidence intervals across all 5 `opt-palm` workloads):
1. **Cold compile**: `opt-palm` cache-miss ready-to-call **<= 600 ms** (faster than Cranelift) and `gdev-dev` **<= 1.2 s**; warm load **<= 2 ms**.
2. **Runtime**: geometric mean **>= 1.15× faster than Wasmtime**, **zero regressions** on any of the 5 `opt-palm` workloads (`>= 1.05×` vs Wasmtime on every workload), and geometric mean within **<= 1.05× of `--backend c`** (or faster).
3. **Correctness**: 100% bit-for-bit SHA-256 replay parity across all 5 `opt-palm` workloads, headless `pen` (`colony.penrec` + `gdev`), full differential opcode/trap/reentry/small-stack test suite (with Wasm arithmetic NaN checked per spec on opcode tests), `dependency_budget.py` 0 violations, `clippy -D warnings` 0 warnings.

---

## 6. Review Log
- **Round 0 (`gpt-6.2-sol`)**: Caught 6 critical bugs in initial WIP files (compile/borrow errors, shift `RCX` clobber, spilled `u64->fp` bit-0 loss, parallel-move stack swap clobber, `v128`/return-spill ABI overlap, pre-push stack check) + important fixes. All 6 fixed in code and verified with 0-error isolated typecheck.
- **Round 1 (`Gemini 4 Neon`: 6.8/10, `gpt-6.2-sol`: 4.0/10)**: Caught `ir.rs` missing negative single-value block types (`-1..-5|-16|-17`), `blot:memory` export omission, inverted `mem_limit_pages == 0` rule, `HELPER_MEM_GROW`/`Trunc`/`wt_import` `I32` zero-extension contract, small-stack/reentry safety (solved cleanly via dedicated 8 MiB `mmap` guest stack + 64 KiB guard), non-reentrant `Ctx.saved_mxcsr` (moved to wrapper local host stack), Pen's `host_callback_trampoline` `&mut AotHostState` aliasing UB, `X64Module`/`X64ConstVal` decoupling, `TargetIsa` `bsr`/`bsf` fallback + CPUID max-leaf/`Rbx`/trap-22 rules, `elf.rs` keyed exports + section headers + `planned_rodata_len`, aggregate admission caps, complete 18-export Blot ABI 8 spec, and Stage 2 pass ordering + numeric promotion gate. All code fixes applied in `wtime` (`x64/{module,asm,elf,ir,regalloc,lower}.rs`) and `pen` (`crates/blot-host/src/wtime_aot.rs`), and integrated cleanly into §§1–5 above.
