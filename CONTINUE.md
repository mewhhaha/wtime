# Continuation Plan: Pure-Rust Wasm → x86-64 Native Compiler (`wtime-aot --backend x64`)

## 1. Verified Baseline & Motivation

### `/home/jacobt/src/wtime` (`main`)
- **Goose `sudoku` regression fixed & verified (`9b25bfd`)**: hybrid `m32::direct`/`direct_mut` in `tools/wtime-aot/rt/src/lib.rs`, `RetAbi::Result` for single-`i32` returns in `tools/wtime-aot/src/codegen.rs`, and `32×` faster `Module::new` global analysis (`2,730 ms -> 85 ms`).
  - Paired CPU-pinned check vs `037d32c` (`aot-final`, 9 rounds, exact stdout): `sudoku` `4.27 ms` vs `4.26 ms` (`1.002×`), `life` `6.59 ms` vs `6.52 ms` (`1.012×`), `sorting` `88.94 ms` vs `88.61 ms` (`1.004×`).
- **Fast C99/Clang backend (`--backend c --c-opt auto`) complete & verified (`9b25bfd`)**:
  - `opt-palm` (1.5 MB / 2,760 fns, Ryzen 7 3800X 8C/16T): **6.27 s** cold (down from `387 s` Rust `-O3 ThinLTO`), **0.81 s** incremental (1 shard + relink), **0.86 ms** warm load.
  - `gdev-dev` (3.2 MB / 4,700 fns): **8.47 s** cold.
  - `opt-palm` `cpu-bench` (100% SHA-256 parity across all 5 workloads):
    - `title`: `0.0555 ms` vs Wasmtime `0.0635 ms` (**1.14×**)
    - `colony`: `0.9710 ms` vs Wasmtime `1.1709 ms` (**1.21×**)
    - `maxed-carrying`: `3.2347 ms` vs Wasmtime `3.7069 ms` (**1.15×**, p95 **1.38×**)
    - `maxed-day-hitting`: `1.9006 ms` vs Wasmtime `2.5234 ms` (**1.33×**, p95 **2.15×**)
    - `piles-2000`: `0.9902 ms` vs Wasmtime `1.1773 ms` (**1.19×**)
  - **Why Clang hits a ~6.3 s cold floor**: `clang -O0` alone takes `1.80 s` (and runs `colony` at `18.03 ms`); `clang -O1` (`7.17 s`) is slower than outlined `-O2` (`6.27 s`) while running slower (`1.32 ms` vs `0.97 ms`). Our frontend + `ld.lld` takes ~220 ms; 95% (`5.93 s`) is inside `clang`.

### `/home/jacobt/src/pen` (`c3719fb`)
- `pen run` (and direct unpacked `--game` runs) default to `wtime-aot --backend c --c-opt auto` via `GuestModule::new_run` / `load_run_module` in `crates/blot-host/src/{wtime_aot.rs,lib.rs}` and `crates/pen/src/main.rs`.
- `pen dev`, `test`, `bench`, `fuzz`, `diverge`, and unit tests remain on Wasmtime/Cranelift unless `PEN_WTIME_AOT=1`.
- Env overrides: `PEN_WTIME_AOT=0|1`, `PEN_WTIME_AOT_BACKEND=c|rust|x64`, `PEN_WTIME_AOT_C_OPT=auto|O2|O1|O0`. Cache key `by-wasm/*.path` includes wasm SHA-256, backend, `c_opt`, and `wtime-aot` binary size + mtime.
- Verified: `blot-host` tests (30/30), headless `opt-palm` colony 300 frames (`281 fps` vs `230 fps` Wasmtime, warm ready `1 ms` vs `12 ms`, exact `frames.trace` SHA-256), headless `pen run --project /home/jacobt/src/gdev --frames 30` (cold `8,123 ms`, warm `3 ms` vs `47 ms`, exact SHA-256).

---

## 2. Goal, Scope & Explicit Admission Contract (`--backend x64`)

Build a self-contained, zero-dependency, pure-Rust (`#![forbid(unsafe_code)]`) Wasm → Linux x86-64 ELF64 `.so` compiler inside `tools/wtime-aot/src/x64/`:
- **No external toolchain**: zero `clang`, `rustc`, `ld.lld`, or `libc` dependencies; emits machine code and a zero-relocation, zero-`DT_NEEDED` ELF64 `ET_DYN` shared library directly.
- **Scope & metering policy**: `tools/wtime-aot` is an explicit opt-in AOT tool isolated outside the production safe-Rust core workspace (`crates/*` remains safe, zero-dep, metered with fuel/cancellation). Like `--backend rust` and `--backend c`, `--backend x64` executes unmetered for Blot/Pen `pen run` builds (`pen dev` keeps Wasmtime deadlines/probes). Before ever promoting `--backend x64` beyond `pen run`, any metered/cancellable host mode must remain on Wasmtime or add explicit epoch/fuel checks.
- **Target platform**: Linux x86-64.
  - Required baseline ISA: SSE2 + SSE4.1 (`roundss`/`roundsd`, `pextrq`/`pinsrq`) + POPCNT.
  - Optional ISA: ABM (`lzcnt`) + BMI1 (`tzcnt`) when enabled, with portable `bsr`/`bsf` + `cmovz` fallback when targeting generic x86-64-v2.
  - **Runtime CPUID gate**: `blot_aot_new` MUST execute `cpuid` (leaf `1` ECX bits 19 & 23 for SSE4.1 & POPCNT; if `lzcnt`/`tzcnt` emitted, leaf `0x80000001` ECX bit 5 & leaf `7` subleaf `0` EBX bit 3) and fail cleanly with a trap code rather than silently executing `lzcnt`/`tzcnt` as `bsr`/`bsf` on unsupported CPUs.

### Explicit Stage 1 Admission Contract (Checked Before Emission)
All inputs pass `wtime_validate::validate` first, then an explicit `admit_x64_module(decoded)` check rejects unsupported shapes cleanly with actionable error messages before codegen:
- **Target**: `--blot-cdylib` only in Stage 1 (Goose standalone CLI binary is optional follow-up).
- **Memories**: `0` or `1` Wasm32 memory (`IndexType::I32`, declared `min <= max <= 65536` pages). Wasm64 / multi-memory / shared memory rejected.
- **Tables**:
  - Static `funcref` tables used by `call_indirect` (initialized only by active element segments with constant offsets; no `table.grow`/`table.copy`/`table.init`/`elem.drop`; declared initial length capped at `<= 1_048_576` slots before building dense metadata to prevent OOM on adversarial modules).
  - Mutable `externref` tables (`table.get`/`table.set` only, declared `min == max <= 65_536` slots).
- **Imports**: function imports only, matching Blot ABI 8 host callback signatures (`()`, `(ref)`, `(i32)`, `(f32)`, `(ref, i32|i64|f32|f64|ref)` -> `()` or single scalar/ref). Imported memories, tables, or globals rejected.
- **Globals**: numeric (`i32`, `i64`, `f32`, `f64`, `v128`) and nullable `externref` globals with constant initializers (`const`, `ref.null`, prior `global.get`, `i32`/`i64` `add`/`sub`/`mul`).
- **Exports**: functions (Blot ABI 8 entrypoints + arbitrary internal functions) and globals; `blot_aot_call_export` dispatches exported functions whose param/result shapes fit Blot ABI 8 (`0..=1` scalar/ref param, `0..=1` scalar/ref result).
- **Opcodes**: MVP + sign-ext + sat-trunc + `memory.copy`/`memory.fill` + minimal `v128` subset (`load`/`store`/`const`/`i{8,16,32,64}x* splat`/`i64x2 extract/replace_lane`) + `ref.null`/`ref.is_null` + `table.get`/`table.set` on `externref`. Exceptions, GC, continuations, atomics, tail calls, passive data/elem ops rejected.
- **Size guards**: `ValRange.len` is `u32`; total module `.rodata + .text` verified `< 2 GiB` (every `rel32` displacement checked via `i32::try_from` before emission).

### Performance Targets (`opt-palm` 1.5 MB / 2,760 fns, Ryzen 7 3800X 8C/16T)
- **Stage 1 (first complete end-to-end milestone)**:
  - Cold compile: **~100 – 250 ms** (using a lean `X64Module` analysis pass — see §4.2, avoiding C-only global/outline passes in `Module::new`), warm `dlopen`: **< 1 ms**, **100% SHA-256 bit-for-bit parity** on all 5 `opt-palm` workloads + `gdev-dev` (3.2 MB), runtime ~0.85–1.05× Wasmtime.
- **Stage 2 (runtime optimization hill-climb)**:
  - Cold compile: **<= 600 ms** (beating Cranelift's ~850 ms), runtime **1.15 – 1.35×+ faster than Wasmtime** and competitive with/beating `--backend c` across all 5 `opt-palm` workloads.

---

## 3. Exact ABI, Layout & Compiler Architecture

### 3.1 Register Partition & Internal Wasm Calling Convention
- **Pinned registers** (never allocated by regalloc; invariant across all internal Wasm calls):
  - `Rsp`: native stack pointer (16-byte aligned before every `call`, so `rsp % 16 == 8` on callee entry)
  - `R15`: `Ctx*` (the `BlotHandle` itself — zero extra indirection)
  - `R14`: `mem_base` (fixed virtual `mmap` base — invariant across `memory.grow`)
  - `R13`: `mem_len` (current linear memory byte length — updated in-place by `memory.grow` after successful `mprotect`)
  - `R12`: pinned hottest mutable non-const scalar (`I32`/`I64`) global (typically `$__stack_pointer`), synced with `Ctx` at every C ABI boundary, host callback, and active reentrant call/trap
- **Scratch registers** (never allocated by regalloc; strictly disjoint roles):
  - `R11` / `Xmm15`: primary scratch, `call` trap-code return (`R11d`), and **parallel-move cycle-breaking temporary**
  - `R10` / `Xmm14`: secondary scratch and **parallel-move memory-to-memory copy temporary**
- **Allocatable registers**:
  - 9 GP: caller-saved `Rax, Rcx, Rdx, Rsi, Rdi, R8, R9` (7) + callee-saved `Rbx, Rbp` (2)
  - 14 XMM: `Xmm0..Xmm13` (all caller-saved in Stage 1)
- **Unified Typed Internal Call ABI** (single source of truth in `regalloc::layout_args` & `regalloc::layout_rets`, shared by caller, callee, and frame allocator):
  - **Arguments**: first 6 GP in `Rdi, Rsi, Rdx, Rcx, R8, R9`; first 8 FP/V128 in `Xmm0..Xmm7`; overflow args placed in caller `[rsp + off]` with natural alignment (`8` bytes for scalars, `16` bytes for `V128`), rounded up to 16 bytes.
  - **Trap signaling**: normal return sets `R11d = 0`; trap return sets `R11d = trap_code (1..21)`. Callers test `test r11d, r11d; jnz .trap_prop`.
  - **Return values**: first 7 GP in `Rax, Rdx, Rcx, Rsi, Rdi, R8, R9`; first 8 FP/V128 in `Xmm0..Xmm7`.
  - **Overflow multi-value returns (arbitrary arity, reentrant-safe)**: caller reserves a 16-byte-aligned return buffer `[rsp + ret_buf_off]` in its own stack frame (sized by `layout_rets`) and stores its address into `[r15 + OFF_RET_SPILL_PTR]` immediately before `call`. Any callee whose own signature has overflow returns saves `[r15 + OFF_RET_SPILL_PTR]` into its frame slot `saved_ret_ptr_off` in its prologue and writes overflow results through that saved pointer *before* `emit_parallel_moves` writes register results. Zero fixed return-count limit; impossible to overwrite `Ctx` globals.
- **Value representation invariant**: every `I32` SSA value in a GP register has zeroed upper 32 bits (preserved by all 32-bit x86-64 ops, `mov_rr(false, d, d)`, and explicit 32-bit moves on C ABI entry).

### 3.2 `Ctx` (`BlotHandle`) & `ActiveCall` Byte-for-Byte Layout
`BlotHandle` and `Ctx` are the exact same `mmap`-allocated struct (`h == ctx`):

| Offset | Size | Field | Notes |
|---:|---:|---|---|
| `0` | 8 | `mem_base: *mut u8` | Fixed virtual reservation base (`NULL` if 0 capacity) |
| `8` | 8 | `mem_len: usize` | Current accessible byte length (`pages * 65536`) |
| `16` | 8 | `mem_max_pages: u64` | `min(declared_max, mem_limit_pages, 65536)` |
| `24` | 8 | `mem_reserve_bytes: usize` | Exact size passed to `munmap` (`mem_max_pages * 65536`) |
| `32` | 8 | `handle_alloc_bytes: usize` | Page-aligned `Ctx` allocation size for `munmap` |
| `40` | 8 | `stack_floor: usize` | Active native stack guard address (shared across active reentry) |
| `48` | 8 | `host_user_ctx: *mut c_void` | Passed to `host_fn` arg 0 |
| `56` | 8 | `host_fn: Option<BlotHostFn>` | Host callback function pointer |
| `64` | 8 | `scratch_tmp: u64` | Internal scratch (e.g. `call_indirect` `idx` across arg moves) |
| `72` | 8 | `ret_spill_ptr: *mut u8` | Caller-owned overflow return buffer pointer |
| `80` | 4 | `saved_mxcsr: u32` | Host `MXCSR` saved on top-level entry / restored around host callbacks |
| `84` | 4 | `in_active_call: u32` | Reentry depth / flag (`0` = idle; `>0` = inside top-level call) |
| `88` | 8 | `reserved` | Padding so globals start 16-byte aligned at offset `96` |
| `96..` | var | Mutable non-const globals | `8` bytes each for `I32`/`I64`/`F32`/`F64`/`Ref`; `16`-byte aligned `16` bytes for `V128` |
| `..` | var | Mutable `externref` tables | `len * 8` bytes each, 8-byte aligned |

Total size rounded up to 4096 bytes.
`ActiveCall` is a 16-byte stack struct `{ ctx: *mut Ctx ([rsp+0]), mem: *mut u8 ([rsp+8]) }` built inside `HELPER_HOST_CALL`.

### 3.3 Native Stack Guard, `MXCSR`, and Linux Syscall Hygiene
- **Prospective pre-prologue stack check**: every Stage 1 function checks `lea r11, [rsp - (frame_size + 8 * used_callee_gp.len())]; cmp r11, [r15 + OFF_STACK_FLOOR]; jb .early_stack_trap` **before** pushing any callee-saved register or subtracting `rsp`. `.early_stack_trap` immediately emits `mov r11d, TRAP_CALL_STACK; ret` with zero pops.
- **Safe `stack_floor` initialization**:
  - Top-level C ABI entrypoints (`blot_aot_new` before `wt_initialize`, `blot_aot_call_export`, `blot_aot_call_allocate`, `blot_aot_call_reset`, `blot_aot_call_finish_invocation`) set `stack_floor = rsp.saturating_sub(budget)` where `budget` is a conservative default **`1 MiB`** (leaving >= `1 MiB` on Rust's default `2 MiB` worker threads and >= `9 MiB` on Pen's `10 MiB` guest threads (`GUEST_STACK_BYTES = 8 MiB` + `2 MiB`), plus >= `64 KiB` headroom reserved for runtime helpers and host callbacks).
  - Active reentrant calls (`blot_aot_active_allocate`, `blot_aot_active_reset`) **preserve** the existing `ctx->stack_floor` instead of recomputing a deeper floor!
  - *(Note: `blot_aot_abi_version() == 1` has no stack-size parameter; if a future ABI v2 adds an explicit thread-stack limit parameter or host stack query, wire it into `stack_floor` directly. Stage 2 leaf check elision must be gated on actual allocated `frame_size + callee_saves` and bounded acyclic call-chain bytes, never on Wasm local count.)*
- **`MXCSR` ownership**:
  - On top-level C ABI entry (when entering guest code), `stmxcsr [r15 + OFF_SAVED_MXCSR]` and `ldmxcsr [rip + RO_MXCSR_WASM]` (`0x1f80`: round-to-nearest-even, all exceptions masked, FTZ=0, DAZ=0). Restore `ldmxcsr [r15 + OFF_SAVED_MXCSR]` on every exit (normal and trap).
  - Around `ctx->host_fn` inside `HELPER_HOST_CALL`: restore host `MXCSR` before calling `host_fn`, and reload `0x1f80` on return (before resuming guest code).
- **Linux x86-64 `syscall` ABI**:
  - Keep syscall emission isolated from SysV C call marshaling: Linux syscalls take `(rax=nr, rdi, rsi, rdx, r10, r8, r9)` and clobber `rcx` and `r11`. Check error via `cmp rax, -4095; jae .err`.
  - Memory lifecycle:
    - `blot_aot_new`: compute `max_p = min(declared_max, limit, 65536)` (returning `TRAP_MEM_LIMIT` / `TRAP_MEM_TOO_LARGE` matching `cgen.rs:4350-4355`). If `max_p == 0`, set `mem_base = NULL, mem_len = 0, mem_reserve = 0` (no `mmap`). Otherwise `mmap(NULL, max_p * 65536, PROT_NONE, MAP_PRIVATE | MAP_ANONYMOUS | MAP_NORESERVE, -1, 0)`; if `min_p > 0`, `mprotect(mem_base, min_p * 65536, PROT_READ | PROT_WRITE)` and on failure `munmap` both memory and `Ctx`.
    - `memory.grow(delta)` / `blot_aot_mem_grow(h, delta: u64)`: handle `delta == 0` explicitly (return `old_pages` without `mprotect`); check `delta <= max_pages && old_pages <= max_pages - delta`; invoke `mprotect(mem_base + old_len, delta * 65536, PROT_READ | PROT_WRITE)` and update `ctx->mem_len` & `R13` **only after `mprotect` succeeds** (else return `-1`).
    - `blot_aot_drop(h)`: if `h != NULL`, `munmap(h->mem_base, h->mem_reserve_bytes)` when non-zero, then `munmap(h, h->handle_alloc_bytes)`.

### 3.4 Single Source of Truth for Shared Helpers (`regalloc::HelperKind`)
All internal runtime helpers preserve `Rbx, Rbp, R12..R15` (except `MemGrow` updating `R13` in-place) and **all 14 allocatable XMM registers (`Xmm0..Xmm13`)** (helpers use only `Xmm14`/`Xmm15`):
- **`HELPER_TRUNC_BASE + 0..15` (`HelperKind::Trunc`)**: input in `Xmm15`, output int in `Rax` (`I32` zero-extended or `I64`), trap code in `R11d` (`0`, `6`=`TRAP_INT_OVERFLOW`, `7`=`TRAP_INVALID_CONV`); clobbers ONLY `Rax, R10, R11, Xmm14, Xmm15` (`gp_mask = MASK_RAX`, `xmm_mask = 0`). Exact IEEE boundary checks matching `cgen.rs:302-394`.
- **`HELPER_MEM_GROW (16)` (`HelperKind::MemGrow`)**: `Rdi = delta (u32)` -> `Rax = old_pages | -1`, updates `R13` & `ctx->mem_len`; clobbers `MASK_GP_CALLER`, `xmm_mask = 0`.
- **`HELPER_MEM_COPY (17)` (`HelperKind::MemCopy`)**: `Rdi = dst (u32), Rsi = src (u32), Rdx = len (u32)` -> `R11d = 0 | TRAP_OOB`; overlap-safe forward (`rep movsb`) / backward loop without `std`/`cld`; clobbers `MASK_GP_CALLER`, `xmm_mask = 0`.
- **`HELPER_MEM_FILL (18)` (`HelperKind::MemFill`)**: `Rdi = dst (u32), Rsi = val (u8), Rdx = len (u32)` -> `R11d = 0 | TRAP_OOB`; `rep stosb`; clobbers `MASK_GP_CALLER`, `xmm_mask = 0`.
- **`HELPER_HOST_CALL (19)`**: called only from `wt_import{k}` trampolines (`Op::Call`, which clobbers `MASK_GP_CALLER | MASK_XMM_CALLER`): checks `ctx->host_fn != NULL`, publishes `R12` to `Ctx`, restores host `MXCSR`, builds `ActiveCall { R15, R14 }` + `u64 res = 0` on 16-byte-aligned stack, calls `host_fn(user_ctx, &ac, idx, tok, arg, &res)` via SysV ABI, reloads `0x1f80` `MXCSR`, reloads `R13 = ctx->mem_len` and `R12 = ctx->g_pinned` (even if `host_fn` failed or invoked reentrant `active_allocate`/`active_reset`!), and returns `res` in `Rax`, `R11d = 0 | TRAP_HOST`.

### 3.5 Status of Existing WIP Files (`tools/wtime-aot/src/x64/`)
All 6 critical bugs from the initial `gpt-6.2-sol` audit have been fixed directly in the WIP files:
1. `asm.rs`: x86-64 + SSE2 + SSE4.1 + POPCNT + LZCNT/TZCNT + `bsr`/`bsf`/`cpuid`/`stmxcsr`/`ldmxcsr` encoder, local labels, RIP-relative relocations, rel32 jump-table patching.
2. `elf.rs`: Zero-relocation Linux ELF64 `ET_DYN` writer (`3 PT_LOAD` + `PT_DYNAMIC` + `PT_GNU_STACK`, SysV `.hash`, `.dynsym`, `.dynstr`), unified single export list shared between `plan` and `emit`, extent validation.
3. `ir.rs`: Block-parameter SSA IR + single-pass Wasm → SSA lowering (`lower_wasm_to_ir(decoded, module, func_index)` with flat `body.locals`), `u32` `ValRange.len`, constant folding + algebraic identities, `CmpBranch` fusion, trap-order-preserving linear load-run check hoisting (stops at any different-address load), DCE.
4. `regalloc.rs`: Unified `layout_args`/`layout_rets` (16-byte aligned `V128`, caller-owned overflow return buffer), unified `HelperKind`/`helper_clobbers`, RPO ordering, cross-block-only `BitSet` liveness, clobber-aware linear scan, eviction-safe spill reuse, 16-byte frame alignment.
5. `lower.rs`: Disjoint scratch parallel-move resolver (`R11`/`Xmm15` cycle break, `R10`/`Xmm14` mem-to-mem), clobber-free variable shifts/rotates, fixed spilled `u64 -> f32/f64` sticky-bit capture, unsigned 32-bit `call_indirect`/`table`/`br_table` bounds checks, pre-prologue prospective stack guard check, caller-owned overflow returns, exact float `cmp` & `min`/`max`.

---

## 4. Remaining Work — Stage 1 (First Complete Working Milestone)

### 4.1 Implement `tools/wtime-aot/src/x64/mod.rs`
1. **Admission & lean module analysis (`X64Module`)**:
   - Validate with `validate_module`, run `admit_x64_module(decoded)`.
   - Compute only what `--backend x64` actually needs: resolved function types, constant globals (`eval_constant` pass), hottest mutable scalar global for `R12` pinning, static `funcref` table dispatch entries (`ci_sigs` / `ci_targets` with dense cap `<= 1_048_576`), mutable `externref` table sizes, exported functions & globals. **Do NOT run C-only analyses** (global-preservation fixpoint, callee-read/write sets, `fn_no_stack_check` Wasm heuristics) so `validate + decode + analyze` stays ~20–30 ms instead of ~100 ms!
2. **`.rodata` & `RodataConsts`**:
   - 16-byte aligned FP masks (`f32_abs`, `f32_neg`, `f64_abs`, `f64_neg`, `f32_qnan`, `f64_qnan`), `RO_MXCSR_WASM (0x1f80u32)`, all 16 truncation boundary constants, trap strings (`1..=21` matching `cgen.rs:4273-4297`), import module/name strings, active data segment bytes (`WT_DATA`), and placeholder `i32` slots for `call_indirect` dispatch tables (using `0` sentinel for null/type-mismatch holes, valid targets strictly non-zero since `.text` follows `.rodata`!).
3. **Runtime helpers & `wt_import{k}` & `wt_initialize`**:
   - Emit `HELPER_TRUNC_BASE + 0..15`, `HELPER_MEM_GROW`, `HELPER_MEM_COPY`, `HELPER_MEM_FILL`, `HELPER_HOST_CALL`, and `wt_import{k}` trampolines.
   - Emit `wt_initialize` (validating active element segment bounds against table lengths, copying active data segments from `WT_DATA` with OOB checks, and calling `module.start` with `R12`/`R13` live — supporting a start function that calls host imports!).
4. **All 18 Blot ABI 8 C exports** (exact signatures, `blot_aot_abi_version() == 1`, return codes `0=ok, 1=trap, 2=host_err, 3=missing`, nullable output pointers checked before writing, SysV callee-saved `Rbx, Rbp, R12..R15` preserved, `MXCSR` & `R12` synced on normal return AND trap):
   - `blot_aot_abi_version`, `blot_aot_trap_message`, `blot_aot_import_count`, `blot_aot_import_name`
   - `blot_aot_new` (CPUID gate + `mmap` `Ctx` + `mmap` memory reservation + `mprotect` initial pages + init globals + `wt_initialize`)
   - `blot_aot_drop` (`munmap` memory + `Ctx`)
   - `blot_aot_call_export`, `blot_aot_call_allocate`, `blot_aot_call_reset`, `blot_aot_call_finish_invocation`
   - `blot_aot_mem_data`, `blot_aot_mem_grow` (`uint64_t` delta)
   - `blot_aot_get_global`, `blot_aot_set_global`
   - `blot_aot_active_mem_data`, `blot_aot_active_allocate`, `blot_aot_active_reset`, `blot_aot_active_get_global`
5. **Parallel compile & deterministic zero-relocation link**:
   - Bounded worker pool via `std::thread::scope` (default `available_parallelism`, overridable with `--jobs N`) compiling defined functions into an indexed `Vec<Option<Asm>>` so `.text` / `.rodata` layout is **100% deterministic** regardless of thread scheduling.
   - Merge each function's `local_rodata` (preserving 16-byte alignment!) into global `.rodata`, call `ElfLayout::plan`, compute exact helper/export/function vaddrs, verify every `rel32` via `i32::try_from`, patch `.text` relocs + `br_table` jump tables + `call_indirect` tables, call `ElfLayout::emit`, and atomically write via temp file + rename.
   - Emit structured `WTIME_AOT_TIMINGS=1` phase timings: `validate`, `analyze`, `ssa`, `regalloc`, `emit`, `link`, `write`, `total`.

### 4.2 Wire CLI, Cache & Verification Suite
1. Add `mod x64;` to `tools/wtime-aot/src/main.rs` and `Backend::X64` (`--backend x64`) to `main.rs` and `build.rs` (cache key: `"wtime-aot build x64 1"` + wasm bytes + generator exe bytes + `target_cpu`).
2. **Stage 1 Unit & Differential Tests** (all runnable via `cargo test --locked --manifest-path tools/wtime-aot/Cargo.toml`):
   - **ELF / glibc smoke test**: `dlopen` generated `.so`, `dlsym` all 18 exports (plus synthetic hash-collision test), verify `/proc/self/maps` segment permissions (`R`, `RX`, `RW`, non-executable stack).
   - **Opcode & corner-case C/Rust execution test**:
     - Shifts/rotates with `a` dying in `RCX` (both reg and spilled counts) — e.g. `3 << 5 == 96`.
     - Spilled `f32.convert_i64_u(0x8000_0080_0000_0001)` and `f64.convert_i64_u(0x8000_0000_0000_0401)` exact IEEE rounding.
     - Parallel moves: 3-way GP/XMM cycles, pure stack-to-stack swaps (`Stack(0) <-> Stack(8)`), duplicate sources, mixed `i32`/`i64`/`f32`/`f64`/`v128` loop-edge phis.
     - Multi-value calls: >6 GP args, >8 FP/`v128` args (9th & 10th `v128` non-overlapping!), >7 GP + >16 overflow returns (`24 i64`s), recursive multi-value returns.
     - All 16 float-to-int truncations (`NaN`, `±Inf`, `-0.0`, `-0.5`, `-1.0`, `INT_MIN`, `INT_MAX`, `UINT_MAX`), `min`/`max` (`-0.0` vs `+0.0`, NaN — checking canonical/arithmetic NaN per Wasm spec), unordered float comparisons, `INT_MIN % -1 == 0`, `div_s` overflow/zero traps.
     - Live FP value in `Xmm0` surviving across every helper (`Trunc`, `MemGrow`, `MemCopy`, `MemFill`).
     - `call_indirect` hit, OOB (`TRAP_TABLE_OOB`), null hole (`TRAP_INDIRECT_CALL`), type mismatch (`TRAP_INDIRECT_CALL`), and imported function table target.
     - Memory: `offset > i32::MAX` load/store with spilled value, `grow(0)`, zero-capacity memory, `grow` exceeding host `mem_limit_pages`, overlapping forward/backward `memory.copy`, load-run trap order (`load A; load B_oob; load A+off_oob`).
     - Host boundary: `MXCSR` non-default rounding/FTZ restored across calls, reentrant `active_allocate`/`active_reset` mutating `R12`/`R13`, `start` function calling an import, recursive `TRAP_CALL_STACK` before touching guard page.
3. **Stage 1 Full Workload & Parity Gates**:
   - `python3 -B tools/checks/dependency_budget.py` (0 violations), `cargo clippy --locked --all-targets -- -D warnings` (0 warnings), `cargo test --locked`.
   - Measure cold compile (cache-miss CLI + ready-to-call with pinned CPU/jobs/revision) on `opt-palm` (1.5 MB) and `gdev-dev` (3.2 MB).
   - Run `cpu-bench` across all 5 `opt-palm` workloads (`title`, `colony`, `maxed-carrying`, `maxed-day-hitting`, `piles-2000`) — **100% exact SHA-256 parity** against Wasmtime.
   - Run headless `pen` with `PEN_WTIME_AOT_BACKEND=x64` on `opt-palm` `colony.penrec` and live `gdev` `pen run` — **100% exact `frames.trace` SHA-256 parity**.

---

## 5. Stage 2 — Runtime Optimization Hill-Climb (Beat Wasmtime at <= 600 ms Cold)

Every Stage 2 optimization must be measured on pinned hardware against **both** Wasmtime/Cranelift and `--backend c` (recording phase compile times + paired 5-workload `opt-palm` runtimes + exact SHA-256 parity):

1. **Profile Stage 1 first** (compile-time tail on `gdev-dev` & large functions for `scan_frame_locals`/liveness/clobber-scan/parallel-moves + runtime hot functions & spill counts on `opt-palm`).
2. **Cheap high-ROI codegen wins** (~0–20 ms compile cost):
   - Fold `Cmp` + `Select` into `cmp + cmovcc` without materializing 0/1 booleans.
   - Strength-reduce constant signed/unsigned `div`/`rem` to magic multiply + shift.
   - Inline small constant-length `memory.copy` / `memory.fill` (`<= 64` bytes) without helper calls.
   - Memory-operand ALU (`add/sub/cmp r, [rsp+spill]`) to cut spill-reload instructions.
   - Elide `test r11d; jnz` at callsites and `xor r11d, r11d` in proven non-trapping callees; elide leaf stack checks using actual allocated native frame bytes + bounded call depth.
   - Callee-saved `Xmm8..Xmm13` (preserving them across `HELPER_HOST_CALL` and C ABI entry/exit!) + loop-depth weighted regalloc spill heuristics.
3. **Hot-budgeted IR optimizations** (budgeted to keep cold compile `<= 600 ms`):
   - **Small-leaf inlining** on IR (`<= 45` insns, non-recursive, total code-growth cap) to eliminate call overhead on Blot's hottest tiny helpers (`f5/f6/f3/f2/f7/f10`).
   - **CFG-dominance GVN / CSE + redundant `GlobalGet` elimination**: compute true dominator tree from the actual CFG (`Cooper–Harvey–Kennedy`, <1 ms) and track side effects across calls/callbacks (global writes & memory mutation).
   - **Loop-invariant code motion (LICM)** + cross-block bounds-check hoisting along dominator paths.
4. **Qualification Gate Before Flipping `pen run` Default (`c` -> `x64`)**:
   - Paired measurements against **both** `--backend c` and Wasmtime/Cranelift (fixed Wasm, CPU affinity, worker count, Wasmtime config):
     - Cold compile `<= 600 ms` on `opt-palm` (`<` Cranelift), warm load `< 2 ms`.
     - Runtime beats Wasmtime across all 5 `opt-palm` workloads and meets/beats `--backend c` target (or clearly documents tradeoff).
     - 100% SHA-256 replay parity on all 5 `opt-palm` workloads + headless `pen` (`colony` & `gdev`), plus full differential opcode/trap/callback test pass.

---

## 6. Review Log
- **Round 0 (`gpt-6.2-sol`)**: Identified 6 critical bugs in initial WIP files (compile/borrow errors, shift `RCX` clobber, spilled `u64 -> fp` bit-0 loss, stack-swap parallel-move clobber, `v128`/return-spill ABI overlap, pre-push stack check & small-stack headroom) + important fixes (load-run trap ordering, `ValRange` `u32`, unified helper clobbers, CPUID/MXCSR hygiene, ELF single export list, unsigned `table_len` compare, lean x64 analysis pass, qualification vs both C and Wasmtime). All 6 critical bugs and core code fixes have been applied to `tools/wtime-aot/src/x64/{elf,ir,regalloc,lower}.rs` and integrated into §§2–5 above.
