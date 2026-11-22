# Continuation Plan: Pure-Rust Wasm → x86-64 Native Compiler (`wtime-aot --backend x64`)

## 1. Current Verified State

### `/home/jacobt/src/wtime` (base `9b25bfd` + WIP `tools/wtime-aot/src/x64/`)
- **Goose `sudoku` regression fixed & verified**: hybrid `m32::direct`/`direct_mut` in `tools/wtime-aot/rt/src/lib.rs` and `RetAbi::Result` for single-`i32` returns in `tools/wtime-aot/src/codegen.rs`, plus `Module::new` global analysis sped up `32×` (`2,730 ms -> 85 ms`).
  - Paired CPU-pinned check vs `037d32c` (`aot-final`): `sudoku` `4.27 ms` vs `4.26 ms` (`1.002×`), `life` `6.59 ms` vs `6.52 ms` (`1.012×`), `sorting` `88.94 ms` vs `88.61 ms` (`1.004×`), exact stdout match.
- **Fast C99/Clang backend (`--backend c --c-opt auto`) complete & verified**:
  - `opt-palm` (1.5 MB / 2,760 fns): **6.27 s** cold (down from `387 s` Rust `-O3 ThinLTO`), **0.81 s** incremental (1 shard + relink), **0.86 ms** warm cached load.
  - `gdev-dev` (3.2 MB / 4,700 fns): **8.47 s** cold.
  - `opt-palm` CPU benchmark (`cpu-bench`, 100% SHA-256 parity across all 5 workloads):
    - `title`: `0.0555 ms` vs Wasmtime `0.0635 ms` (**1.14×**)
    - `colony`: `0.9710 ms` vs Wasmtime `1.1709 ms` (**1.21×**)
    - `maxed-carrying`: `3.2347 ms` vs Wasmtime `3.7069 ms` (**1.15×**, p95 **1.38×**)
    - `maxed-day-hitting`: `1.9006 ms` vs Wasmtime `2.5234 ms` (**1.33×**, p95 **2.15×**)
    - `piles-2000`: `0.9902 ms` vs Wasmtime `1.1773 ms` (**1.19×**)
  - **Why Clang hits a ~6.3 s cold floor**: `clang -O0` alone takes `1.80 s` (and runs `colony` at `18.03 ms`, ~15× slower than Wasmtime); `clang -O1` (`7.17 s`) is slower than outlined `-O2` (`6.27 s`) while running slower (`1.32 ms` vs `0.97 ms`). Our own frontend + `ld.lld` is only ~220 ms; 95% (`5.93 s`) is inside `clang`.
- **WIP pure-Rust x86-64 backend started in `tools/wtime-aot/src/x64/`** (not yet wired into `main.rs`, so existing `cargo test`, `cargo clippy -- -D warnings`, and `dependency_budget.py` remain 100% green).

### `/home/jacobt/src/pen`
- `pen run` (and unpacked `--game` runs) default to `wtime-aot --backend c --c-opt auto` via `GuestModule::new_run` / `load_run_module` in `crates/blot-host/src/{wtime_aot.rs,lib.rs}` and `crates/pen/src/main.rs`.
- `pen dev`, `test`, `bench`, `fuzz`, `diverge`, and unit tests remain on Wasmtime/Cranelift unless `PEN_WTIME_AOT=1`.
- Opt-out/overrides: `PEN_WTIME_AOT=0|1`, `PEN_WTIME_AOT_BACKEND=c|rust|x64`, `PEN_WTIME_AOT_C_OPT=auto|O2|O1|O0`. Cache key in `by-wasm/*.path` includes backend, `c_opt`, and `wtime-aot` binary size+mtime.
- Verified: `cargo test --offline -p blot-host` (30/30 passed), headless `opt-palm` colony 300 frames (`281 fps` vs `230 fps` Wasmtime, warm ready `1 ms` vs `12 ms`, exact `frames.trace` SHA-256 match), headless `pen run --project /home/jacobt/src/gdev --frames 30` (cold `8,123 ms`, warm `3 ms` vs `47 ms` Wasmtime, exact SHA-256 match).

---

## 2. Goal: Own Pure-Rust Wasm → x86-64 Native Compiler (`--backend x64`)

Build a completely self-contained Wasm → x86-64 compiler + ELF64 `.so` emitter inside `tools/wtime-aot/src/x64/`:
- **Zero external dependencies, zero `clang`/`rustc`/`ld.lld` subprocesses, `#![forbid(unsafe_code)]`** (emitting machine bytes + ELF64 `.so` requires no `unsafe` in the compiler).
- **Zero dynamic relocations & zero `DT_NEEDED`**: all code/rodata/jump-table/call-indirect references use PC-relative `rel32` displacements; memory allocation/grow uses direct Linux x86-64 `syscall` (`mmap`/`mprotect`/`munmap`).
- **Target performance on `opt-palm` (1.5 MB / 2,760 fns, 8C/16T Zen 2)**:
  - **Stage 1 (first working end-to-end milestone):** ~**0.10 – 0.25 s** cold compile, `< 1 ms` warm `dlopen`, **100% SHA-256 bit-for-bit parity** on all 5 `opt-palm` workloads + `gdev-dev` (3.2 MB), ~0.85–1.05× Wasmtime runtime.
  - **Stage 2 (optimization hill-climb):** ~**0.25 – 0.60 s** cold compile (still faster than Cranelift's ~0.85 s), **1.15 – 1.35×+ faster than Wasmtime** at runtime.

---

## 3. Architecture & What Is Already Written (`tools/wtime-aot/src/x64/`)

### Register & Internal Calling Convention
- **Pinned registers** (never spilled, invariant across all internal Wasm calls):
  - `Rsp`: native stack pointer (16-byte aligned at every call boundary)
  - `R15`: `Ctx*` (Blot handle + globals + memory metadata + host callback + `ret_spill`)
  - `R14`: `mem_base` (fixed 4 GiB `PROT_NONE | MAP_NORESERVE` `mmap` base — never moves even on `memory.grow`)
  - `R13`: `mem_len` (current linear memory byte length — updated in-place by `memory.grow`)
  - `R12`: pinned hottest mutable scalar global (e.g. `$__stack_pointer`), synced with `Ctx` only at C ABI entry/exit and host callbacks
- **Scratch registers** (never allocated by regalloc): `R11`, `R10`, `Xmm15`, `Xmm14`.
- **Allocatable registers**:
  - 9 GP: caller-saved `Rax, Rcx, Rdx, Rsi, Rdi, R8, R9` + callee-saved `Rbx, Rbp`
  - 14 XMM: `Xmm0..Xmm13` (caller-saved in Stage 1)
- **Internal Wasm call ABI**:
  - GP args: `Rdi, Rsi, Rdx, Rcx, R8, R9`, overflow on stack `[rsp + k*8]`
  - FP/V128 args: `Xmm0..Xmm7`, overflow on stack
  - Trap signaling: `R11d` (`0` = ok, `1..21` = trap code); caller emits `test r11d, r11d; jnz .trap_prop`
  - Return values: GP in `Rax, Rdx, Rcx, Rsi, Rdi, R8, R9`, FP in `Xmm0..Xmm7`, overflow in `[r15 + OFF_RET_SPILL + k*8]`
- **Invariant**: every `I32` SSA value in a GP register has zeroed upper 32 bits (naturally preserved by all 32-bit x86-64 ops), so Wasm32 linear memory addressing needs no extra zero-extension.

### Existing WIP Files
1. `tools/wtime-aot/src/x64/asm.rs` (~545 LOC):
   - x86-64 + SSE2 + SSE4.1 (`roundss`/`roundsd`) + POPCNT + LZCNT/TZCNT encoder, local label fixups, RIP-relative relocations (`Func`, `Helper`, `Rodata`, `LocalRodata`), and rel32 `br_table` jump-table patching.
2. `tools/wtime-aot/src/x64/elf.rs` (~175 LOC):
   - Zero-relocation Linux x86-64 `ET_DYN` ELF64 `.so` writer: 3 `PT_LOAD` segments (`R`: headers + `.dynsym` + SysV `.hash` + `.dynstr` + `.rodata`; `RX`: `.text`; `RW`: `.dynamic`) + `PT_DYNAMIC` + `PT_GNU_STACK` (non-executable stack).
3. `tools/wtime-aot/src/x64/ir.rs` (~850 LOC):
   - Compact block-parameter SSA IR + single-pass Wasm → SSA lowering with `scan_frame_locals` pre-pass (only live modified locals become block/loop parameters).
   - Built-in Stage 1 optimizations: constant folding + algebraic identities + shift/mask strength reduction, `BranchIf` + compare/`i32.eqz`/const fusion (`CmpBranch`), intra-block redundant bounds-check elimination + pure load-run check hoisting (`BoundsCheck`), and reverse-pass DCE.
4. `tools/wtime-aot/src/x64/regalloc.rs` (~445 LOC):
   - RPO reachable block ordering, cross-block-only `BitSet` liveness fixed-point, clobber-aware linear-scan allocator (`Call`/`CallIndirect`/`MemCopy`/`MemFill`/`MemGrow`/`Trunc`/`Div`/`Rem`/variable shifts), spill-slot reuse, and 16-byte aligned frame layout.
5. `tools/wtime-aot/src/x64/lower.rs` (~725 LOC):
   - SSA IR → x86-64 lowering, parallel-move resolver, O(1) rel32 `call_indirect` dispatch, rel32 `br_table`, exact Wasm float `cmp` and `min`/`max` (`-0.0`/`+0.0`/NaN), and shared cold trap stubs.

---

## 4. Remaining Steps to Finish Stage 1

### Step 1A: Fix Known Edge Cases (and Address GPT 6.2 Sol Review Findings)
Before writing `mod.rs`, fix these already-spotted edge cases (plus anything flagged in review):
1. **`lower.rs` `emit_parallel_moves` stack-to-stack cycle bug**:
   - If a parallel-move cycle consists exclusively of memory slots (e.g. `Stack(0) <-> Stack(8)`), cycle-breaking saves `Stack(0)` into `SCRATCH_GP1` (`R11`), and the next move `Stack(0) <- Stack(8)` calls `emit_single_move` which ALSO uses `SCRATCH_GP1` (`R11`), clobbering the saved value!
   - Fix: use `SCRATCH_GP2` (`R10`) / `SCRATCH_FP2` (`Xmm14`) for memory-to-memory temporaries inside `emit_single_move`, OR reserve `R11` for cycle breaking and check that callers (`CallIndirect` stashing `idx` in `R10`) don't collide (e.g., stash `CallIndirect`'s `idx` in a dedicated stack temp or emit `idx -> R10` ONLY if no stack-to-stack cycle, or push `idx` / read `idx` from `[r15 + scratch]`!). Using `[r15 + OFF_CI_IDX]` or keeping `R10` free until after `emit_parallel_moves` is cleanest — wait: if `idx` might be overwritten by arg moves, save `idx` to `[r15 + OFF_SCRATCH_TMP]` before arg moves and load it into `R10` right after! Then `R10` & `Xmm14` are 100% free inside `emit_parallel_moves` for mem-to-mem copies!
2. **`lower.rs` `V128` overflow stack args / `ret_spill`**:
   - In `regalloc.rs` & `lower.rs`, overflow stack arguments currently advance `stack_slot += 8`; ensure `V128` overflow args reserve 16 bytes (16-byte aligned) in both caller and callee if ever encountered.
3. **`lower.rs` `Op::MemGrow` when `module.fixed32.is_some()`**:
   - Ensure `MemGrow` returns `old_pages` when `delta == 0` and `-1` when `delta > 0` if `fixed32` or `max_pages == min_pages` (our `rt_mem_grow` checking `old_pages + delta <= max_pages` handles this automatically!).
4. **`ir.rs` `Op::I32BinImm` / `Op::I64BinImm` with `IntBinOp::Sub` and `i32::MIN`**:
   - Already handled in `lower.rs` (`imm.checked_neg()`), verify `I64BinImm` sign-extension of `imm32` for `And`/`Or`/`Xor`/`Add`/`Sub`/`Mul` matches x86-64 `REX.W` imm32 sign-extension (it does because `ir.rs` checks `i32::try_from(cb)`!).
5. **`asm.rs` `mov_rr(false, d, d)`**:
   - Deliberately DOES emit 32-bit `mov d32, d32` (zero-extends upper 32 bits of `d`, needed by `I32WrapI64`/`I64ExtendI32U`), while `mov_rr(true, d, d)` is a no-op. Keep this invariant documented and tested.
6. **`regalloc.rs` `free_spills8` reuse order**:
   - Verify whether reusing a spill slot evicted from an earlier interval `[ev_start, ev_end]` can ever collide if the slot holds multiple past intervals (track `max_end` per slot, or track per-slot interval list if needed; note `ev_start < iv.start` so checking against ALL intervals on that slot is trivially exact and still O(slots)).
7. **`regalloc.rs` `Call` / `CallIndirect` with `args` & `rets` on dead/unused return values + `Call` clobber check**:
   - Double-check `clob_end` when a value is defined in a block before a loop and last used AT a call inside the loop (`live_out` extends `last_use` to `block_end`, so `clob_end > call_pos`, correctly marking it clobbered!). Verify case where a block has a `Call` at the very last instruction and a value is ONLY used by the block's terminator (`pos_term = pos_use + 2 > pos_use` -> correctly clobbered!).

### Step 1B: Implement `tools/wtime-aot/src/x64/mod.rs`
Implement the module driver, runtime helpers, and Blot ABI 8 exports:
1. **`CtxLayout` & constants**:
   - `OFF_MEM_BASE = 0`, `OFF_MEM_LEN = 8`, `OFF_MEM_MAX_PAGES = 16`, `OFF_MEM_RESERVE = 24`, `OFF_HANDLE_ALLOC = 32`, `OFF_STACK_FLOOR = 40`, `OFF_HOST_USER_CTX = 48`, `OFF_HOST_FN = 56`, `OFF_SCRATCH_TMP = 64`, `OFF_RET_SPILL = 72` (16×8 = 128 bytes), globals starting at `200` (8 bytes each; 16-byte aligned for `V128`), then mutable `externref` tables (`len * 8` bytes each), total rounded up to `4096`.
   - Select `pinned_global: Option<u32>` = highest static read+write count mutable non-const `I32`/`I64` global (pin in `R12`).
   - Build `ci_tables` descriptors (table len, min_slot, span, `.rodata` offset).
2. **`RodataConsts` & `.rodata` builder**:
   - 16-byte aligned FP masks (`f32_abs`, `f32_neg`, `f64_abs`, `f64_neg`, `f32_qnan`, `f64_qnan`) and all float/double truncation boundary constants.
   - Trap message C strings (`1..=21`), import module & name strings, active data segment bytes (`WT_DATA`), and placeholder slots for `call_indirect` `i32` offset tables.
3. **Shared `.text` runtime helpers (`HelperId`)**:
   - `0..16`: All 16 `TruncOp` helpers (`Xmm15 -> Rax`, trap in `R11d`, clobbers only `Rax, R10, R11, Xmm14, Xmm15`), matching exact boundary semantics in `cgen.rs` lines 302–394.
   - `HELPER_MEM_GROW (16)`: checks `delta <= max_pages - old_pages`, invokes Linux `SYS_mprotect (10)` on `mem_base + old_len` for `delta * 65536` bytes with `PROT_READ | PROT_WRITE`, updates `R13` and `[R15 + OFF_MEM_LEN]`, returns `old_pages` (or `-1`).
   - `HELPER_MEM_COPY (17)`: bounds checks `dst + len <= R13` and `src + len <= R13`, overlap-safe forward (`rep movsb`) / backward loop copy, returns `R11d = 0 | TRAP_OOB`.
   - `HELPER_MEM_FILL (18)`: bounds check `dst + len <= R13`, `rep stosb`, returns `R11d = 0 | TRAP_OOB`.
   - `HELPER_HOST_CALL (19)`: checks `ctx->host_fn != NULL`, flushes `R12` to `Ctx`, builds `ActiveCall { ctx = R15, mem = R14 }` + `u64 res = 0` on 16-byte-aligned stack, calls `ctx->host_fn` via SysV ABI, reloads `R13 = ctx->mem_len` and `R12 = ctx->g_pinned`, returns `res` in `Rax` and `R11d = 0 | TRAP_HOST`.
   - `wt_import{k}` trampolines (matching `cgen.rs` lines 4094–4126).
   - `wt_initialize`: validates active element segments against table lengths, copies active data segments from `.rodata` (`WT_DATA`) into `mem_base` with OOB checks, and invokes `module.start` if present.
4. **All 18 Blot ABI 8 C exports** (SysV ABI entry/exit saving/restoring `Rbx, Rbp, R12..R15` and syncing `R12` with `Ctx`):
   - `blot_aot_abi_version`, `blot_aot_trap_message`, `blot_aot_import_count`, `blot_aot_import_name`
   - `blot_aot_new` (allocates `Ctx` via `SYS_mmap (9)`, reserves `max_pages * 65536` `PROT_NONE | MAP_PRIVATE | MAP_ANONYMOUS | MAP_NORESERVE` via `SYS_mmap`, `SYS_mprotect`s initial `min_pages * 65536` as `PROT_READ | PROT_WRITE`, initializes non-const globals, runs `wt_initialize`; on failure `SYS_munmap`s and sets `*trap_out`)
   - `blot_aot_drop` (`SYS_munmap` memory + handle)
   - `blot_aot_call_export`, `blot_aot_call_allocate`, `blot_aot_call_reset`, `blot_aot_call_finish_invocation` (sets `ctx->stack_floor = rsp - (7 << 20)` on top-level entry)
   - `blot_aot_mem_data`, `blot_aot_mem_grow`
   - `blot_aot_get_global`, `blot_aot_set_global`
   - `blot_aot_active_mem_data`, `blot_aot_active_allocate`, `blot_aot_active_reset`, `blot_aot_active_get_global`
5. **Parallel compile & zero-relocation linking**:
   - Compile all defined Wasm functions in parallel with `std::thread::scope` (`lower_wasm_to_ir` -> `allocate` -> `lower_ir_to_x64`).
   - Merge all functions' `local_rodata` into global `.rodata`, plan `ElfLayout`, compute exact `fn_vaddrs` & `helper_vaddrs`, patch every `Reloc` in `.text`, patch every `JumpTablePatch` in `.rodata`, fill every `call_indirect` table entry `(target_vaddr - table_base_vaddr) as i32`, call `ElfLayout::emit`, and atomically write `out_so`.

### Step 1C: Wire CLI, Cache, Tests & Verification
1. Add `mod x64;` to `tools/wtime-aot/src/main.rs` and `Backend::X64` (`--backend x64`) to `main.rs` & `build.rs` (cache key: `"wtime-aot build x64 1"` + wasm + generator exe bytes).
2. Add comprehensive end-to-end unit tests in `x64/mod.rs` (loading the generated `.so` via a tiny C/Rust test driver — same pattern as `cgen::tests` — testing every numeric op, FMA-off IEEE parity, `-0.0`/`+0.0`/NaN `min`/`max`, all 16 truncations + traps, `div`/`rem` traps & `INT_MIN % -1 == 0`, `br_table`, `call_indirect` hit/OOB/mismatch, `memory.grow`/`copy`/`fill` including overlapping copy, host callbacks with reentrant `active_allocate`, stack-overflow trap, multi-value returns, and parallel-move register/stack cycles).
3. Run verification gates:
   - `python3 -B tools/checks/dependency_budget.py` (0 violations)
   - `cargo clippy --locked --manifest-path tools/wtime-aot/Cargo.toml --all-targets -- -D warnings` (0 warnings)
   - `cargo test --locked --manifest-path tools/wtime-aot/Cargo.toml`
   - Compile `opt-palm` (1.5 MB) and `gdev-dev` (3.2 MB) with `WTIME_AOT_TIMINGS=1 wtime-aot build ... --backend x64 --blot-cdylib`.
   - Run `cpu-bench` on all 5 `opt-palm` workloads (`title`, `colony`, `maxed-carrying`, `maxed-day-hitting`, `piles-2000`) and verify **100% bit-for-bit SHA-256 parity** against Wasmtime!
   - Run headless `pen` with `PEN_WTIME_AOT_BACKEND=x64` on `opt-palm` `colony.penrec` and `gdev` `pen run` and verify exact `frames.trace` SHA-256 match.

---

## 5. Stage 2: Runtime Optimization Hill-Climb (Target: Beat Wasmtime at <0.6s Cold)

Once Stage 1 has 100% SHA-256 parity and ~0.1–0.25 s cold compile, climb runtime systematically (measuring `opt-palm` 5 workloads + compile time after each change):
1. **Small-leaf inlining directly on Wasm/IR** (`<= 45` insns, non-recursive — inlines Blot's hot `f5/f6/f3/f2/f7/f10` helpers at zero C-parser cost).
2. **Dominator-tree GVN / CSE + redundant `GlobalGet` elimination** (structured Wasm blocks give dominator order for free; eliminates repeated address/bounds/global expressions inside loops).
3. **Simple LICM** (hoist loop-invariant `Const*`, non-mutated `GlobalGet`, and pure arithmetic out of loops).
4. **Callee-saved XMM registers (`Xmm8..Xmm13`) + loop-depth weighted regalloc** (keep loop-carried floats and inner-loop values in registers across helper calls; sync `Xmm8..Xmm13` in `HELPER_HOST_CALL`).
5. **Instruction selection & peepholes**:
   - Fold `Cmp` + `Select` into `cmp + cmovcc` without materializing boolean `0/1`.
   - Strength-reduce constant signed/unsigned `div`/`rem` to magic multiply + shift (`mul`/`imul`).
   - Inline small constant-length `memory.copy` / `memory.fill` (`<= 64` bytes) as unrolled 8/16-byte `mov`/`movdqu` without calling helpers.
   - Elide `test r11d, r11d; jnz` at callsites and `xor r11d, r11d` in callees proven non-trapping (`!fn_can_trap`).
   - Fuse `add(base, const)` / `[r14 + idx*scale + disp]` addressing where wrap-safe, and support `mem` right-hand operands (`add r, [rsp+spill]`, `cmp r, [rsp+spill]`) to reduce spill-reload pressure.
   - Multi-global pinning (`R12` + optional callee-saved reg for 2nd hottest preserved global).
6. **Cross-block bounds-check elimination** (dominator-tree `checked_max` propagation + loop-invariant upper-bound hoisting for counted loops).
7. **Flip `pen run` default** in `/home/jacobt/src/pen/crates/blot-host/src/wtime_aot.rs` from `c` to `x64` once `--backend x64` beats Wasmtime across all 5 `opt-palm` workloads with 100% parity!

---

## 6. External Review Findings (`gpt-6.2-sol`, `codex exec`)

Reviewed `CONTINUE.md` and `tools/wtime-aot/src/x64/{asm.rs,elf.rs,ir.rs,regalloc.rs,lower.rs}` (including an isolated Rust type-check and numeric counterexample verification).

### Critical Bugs (Must Fix Before Stage 1 End-to-End Wiring)
1. **8 compile/borrow errors in WIP `x64` files** (`ir.rs:519,1059`, `lower.rs:220,262,1371`):
   - `Module` has no `bodies` field and `DecodedModule`'s `FunctionBody.locals` is already a flat `Vec<ValType>` — pass `DecodedModule` / `&FunctionBody` into `lower_wasm_to_ir`.
   - Pass `&mut f` explicitly to `emit_range` in `Term::for_each_use` (`ir.rs:519`).
   - Read `self.ty(v)` into a local before mutably borrowing `self.asm` in the 3 FP load/store helpers (`lower.rs:220,262,1371`).
2. **Shift/rotate clobbers left operand when `a` is in `RCX`** (`lower.rs:508`, `regalloc.rs:223,385`):
   - `regalloc` only protects values live *across* the shift, so an operand `a` dying at the shift can live in `RCX`. Loading `b` into `RCX` then overwrites `a` before `a` reaches `dst` (e.g., `a=3` in `RCX`, `b=5` in `RDX`, `dst=RAX` computes `5 << 5 = 160` instead of `3 << 5 = 96`).
   - Fix: marshal `(d <- a, rcx <- b)` via parallel assignment (or move `a` out of `RCX` first) and test both register and spilled counts.
3. **Spilled `u64 -> f32/f64` conversion loses bit 0 before rounding** (`lower.rs:1106,1148`):
   - When `a` is spilled, `ar` is loaded into `R11`. The high-bit path shifts `R11` right by 1 and then reads `ar` (`R11`) again to extract the sticky bit, corrupting rounding (verified wrong at `f32`: `0x8000_0080_0000_0001` and `f64`: `0x8000_0000_0000_0401`).
   - Fix: copy `ar` (or extract `ar & 1`) into `R10` *before* shifting `R11`, and test with forced spills.
4. **Parallel-move stack-to-stack cycle clobber (GP & XMM)** (`lower.rs:80,130,355`):
   - Both cycle-breaking and memory-to-memory copies use `R11`/`Xmm15`, so a pure memory cycle (`Stack(0) <-> Stack(8)`) overwrites the saved cycle value.
   - Fix: reserve independent scratch temporaries (`R11`/`Xmm15` for cycle breaking vs `R10`/`Xmm14` for mem-to-mem copies), stash `CallIndirect`'s `idx` in a frame/Ctx slot instead of `R10` across arg moves, and test 3-way cycles, duplicate sources, and mixed-width spills.
5. **Typed ABI layout overlap for `v128` overflow args and >16 multi-value returns** (`regalloc.rs:174,544`, `lower.rs:367,1625,1714`):
   - Overflow `v128` stack args advance by 8 bytes instead of 16 bytes (overlapping adjacent args and overflowing the reserved outgoing arg area).
   - Fixed 16-slot `Ctx.ret_spill` overflows into globals if a function returns >7 GP + 16 overflow values (e.g., 24 `i64`s).
   - Fix: share one typed byte-offset layout (with explicit size & alignment) between `regalloc.rs` and `lower.rs` for both args and results, and use a caller-owned return buffer (or enforce an explicit checked admission limit before emission).
6. **Stack guard order & small-thread stack safety** (`lower.rs:1701`, `ir.rs:1074`, `codegen.rs:2231`):
   - Prologue pushes callee-saved registers *before* checking `stack_floor`, and `rsp - 7 MiB` can undershoot a smaller pthread stack. Also `fn_no_stack_check` estimates Wasm locals/insns rather than actual allocated native frame bytes.
   - Fix: compute/check prospective `rsp - total_frame` *before* any push, start with checks on every frame using a real stack bound (or conservative headroom accounting for helper/host calls and active reentry), and revisit leaf check elision using real allocated frame bytes + call depth.

### Important Fixes & Verification Checks
- **IR load-run hoisting & `ValRange.len` overflow** (`ir.rs:616,635,1130,2055`):
  - Lookahead can cross a different-address load (`load A; load B_oob; load A+off_oob`), changing which instruction traps (and is $O(N^2)$ on long runs of distinct addresses). Stop lookahead at any other potentially trapping load (or preserve exact order on failed combined check).
  - Widen `ValRange.len` (e.g. `u32`) or reject oversized ranges explicitly so large operand + carried-local lists cannot truncate `u16`.
- **Single source of truth for helper clobber masks** (`regalloc.rs:212`):
  - Align `Trunc` helper comment vs mask vs implementation (ensure `Xmm0` is NOT clobbered by `Trunc`, `MemCopy`, `MemFill`, or `MemGrow`, and test live FP values in `Xmm0` across every helper).
- **CPU feature gate & `MXCSR` hygiene** (`asm.rs:361,531,866,1061`):
  - Gate SSE4.1, POPCNT, LZCNT, and BMI1/TZCNT before execution (on unsupported CPUs `lzcnt`/`tzcnt` silently execute as `bsr`/`bsf` with wrong results!), or provide `bsr`/`bsf` fallback.
  - Save/restore `MXCSR` across guest/host boundaries so host rounding mode / FTZ / DAZ / unmasked FP exceptions cannot alter Wasm semantics.
- **ELF export single-table & checked `rel32`** (`elf.rs:42,85,138,185`):
  - Derive `.dynsym`, `.dynstr`, and `.hash` from one shared export descriptor slice (avoid independent name slices in `plan` vs `emit`), verify merged `.rodata` alignment and every `rel32` conversion, and add a glibc `dlopen`/`dlsym` smoke test covering all 18 exports, hash collisions, and segment permissions.
- **Stage 1 admission & Blot ABI 8 boundary** (`codegen.rs:2032`, `lower.rs:396`):
  - Validate/cap supported memory, table, reference, import, and export shapes up front; fix `cmp_ri(true, SCRATCH_GP2, table_len as i32)` sign-extension (use unsigned 32-bit compare or cap table length).
  - Match all 18 Blot ABI 8 signatures, status codes (`0/1/2/3`), nullable output pointers, `uint64_t` grow delta, `R12`/`R13` publish+reload across host callbacks and active returns (including traps), and Linux syscall `R10`/`RCX`/`R11` clobber rules (`mprotect` before updating `mem_len`, zero-capacity & `grow(0)` edge cases).
- **Stage 2 budgeting & qualification**:
  - Instrument phase timings (`validate`, `analyze`, `ssa`, `regalloc`, `emit`, `link`, `load`); audit which `Module::new` analyses `--backend x64` actually needs so analysis doesn't eat the 100–250 ms Stage 1 budget.
  - Compute true CFG dominance before GVN/LICM, budget inlining by measured hotness/growth, preserve any callee-saved XMMs across host calls, and qualify against **both** `--backend c` and Wasmtime (runtime + cold/ready + exact replay parity + differential trap tests) before changing `pen run`'s default backend.

### Confirmed Sound by Inspection
- `asm.rs` REX/ModRM/SIB/RIP-relative/SSE instruction encodings.
- `elf.rs` 3 page-separated `PT_LOAD` segments, writable `PT_DYNAMIC`, non-executable `PT_GNU_STACK`, and SysV `.hash` chaining.
- `ir.rs` structured block/implicit-else/unreachable lowering.
- `regalloc.rs` cross-block back-edge liveness, strict-interior clobber checks, spill-slot reuse, and scalar call alignment.
- `lower.rs` signed-zero `min`/`max`, unordered float comparisons, `INT_MIN % -1 == 0`, and widened Wasm32 offset arithmetic.
