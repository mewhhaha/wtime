# `wtime-aot --backend x64` Architecture & Blot ABI 8 Contract

This document specifies the target envelope, admission rules, internal ABI,
runtime stack/memory model, and security/CET guarantees of `tools/wtime-aot`'s
direct Linux x86-64 backend (`--backend x64 --blot-cdylib`).

## 1. Target Platform & ISA Policy

- **Supported target**: **Linux x86-64 SysV LP64 only**
  (`target_os = "linux"`, `target_arch = "x86_64"`, `target_pointer_width = "64"`;
  Linux x32 ILP32 is explicitly excluded).
- **Zero external build/runtime dependencies**:
  - The compiler crate (`tools/wtime-aot`) is `#![forbid(unsafe_code)]` and
    depends only on in-tree `wtime-types` and `wtime-validate` (zero external
    crates, zero `clang`/`rustc`/`ld.lld` subprocesses).
  - The emitted ELF64 `.so` has zero `DT_NEEDED`, zero dynamic relocations, and
    zero `libc` dependency (uses direct Linux x86-64 `mmap`/`mprotect`/`munmap`
    syscalls).
- **Required baseline ISA**: SSE2 + SSE4.1 (`roundss`/`roundsd`, `pextrq`/`pinsrq`)
  + POPCNT.
- **Optional ISA (`TargetIsa { has_abm_bmi1 }`)**:
  - `x64-v3` (`has_abm_bmi1 = true`): emits `lzcnt`/`tzcnt` for `clz`/`ctz`.
  - `x64-v2` (`has_abm_bmi1 = false`): emits branchless `bsr`/`bsf` + `cmovz`.
  - `--target-cpu native` probes host features; `generic` / `x86-64-v2` select
    `x64-v2`; `x86-64-v3`, `x86-64-v4`, `znver<digits>`, and listed Intel
    Haswell+ profiles select `x64-v3`; unrecognized CPU names are rejected.
  - Cache keys bind the resolved profile (`x64-v2` vs `x64-v3`), never raw
    `"native"`.
- **Mandatory runtime CPUID & CET shadow-stack (`SHSTK`) gate**:
  - `blot_aot_new` probes CPUID leaves `0`, `1`, `0x8000_0000`/`0x8000_0001`
    (unsigned comparison), and `7` to verify baseline/optional ISA features and
    records `CPUID.(EAX=7, ECX=0):ECX[bit 7]` (`CET_SS`) in `Ctx`.
  - Because generated `.so` code switches `rsp` to a dedicated 8 MiB guest stack
    without emitting `GNU_PROPERTY_X86_FEATURE_1_SHSTK`, **both `blot_aot_new`
    and every top-level `blot_aot_call_*` wrapper** check `rdsspq rax` whenever
    `cpu_has_cet_ss != 0`. If hardware shadow stacks are active on the calling
    thread (`rax != 0`), the call fails closed with `TRAP_UNSUPPORTED_ISA (22)`
    (`"Unsupported CPU ISA"`) without switching `rsp`. (Callers on CET-active
    threads should use `--backend c`, which compiles and links with
    `-fcf-protection=return` and verifies `GNU_PROPERTY_X86_FEATURE_1_SHSTK` on
    the resulting `.so`.)

## 2. Admission Contract & Compiler Caps

All inputs pass bounded file acquisition (`<= 64 MiB` for `--backend x64`,
deadline-checked in 64 KiB chunks) and `wtime-validate` under Stage 1's feature
profile (MVP + `mutable_globals`, `saturating_float_to_int`, `sign_extension`,
`multi_value`, `reference_types`, `bulk_memory`, `simd`, `extended_const`;
`function_references: false`) with `ResourceLimits::default()`.

Admitted constructs:
- Plain final function types (no GC subtypes or descriptors).
- `0` or `1` non-shared Wasm32 memory with standard 64 KiB pages (`max <= 65_536`).
- Static 32-bit nullable `funcref` tables (`min <= 1_048_576`, aggregate
  `<= 2_097_152` slots) and mutable 32-bit nullable `externref` tables
  (`min == max <= 65_536`).
- Active memory-0 data segments with constant `i32` offsets.
- Plain function imports with Blot ABI-compatible signatures and globals of
  `i32`, `i64`, `f32`, `f64`, `v128`, or nullable `externref`.
- MVP opcodes, saturating float-to-int truncations, `memory.copy`/`memory.fill`,
  and the 16-byte `v128` load/store/const/splat/extract/replace subset.

Unsupported modules are rejected cleanly before codegen with CLI exit code `4`
(`X64_ADMISSION_REJECTED_EXIT_CODE`), distinct from exit code `1` for I/O,
budget/deadline, or internal errors. Compiler bounds enforce `MAX_EXPORTS = 65_536`,
`MAX_GLOBALS = 65_536`, `MAX_CTX_BYTES = 16 MiB`, `MAX_BLOCKS_PER_FN = 262_144`,
`MAX_IR_VALS_PER_FN = 1_000_000`, `MAX_LIVENESS_BYTES = 16 MiB` per function /
`256 MiB` concurrent across workers, `MAX_TOTAL_CODE_BYTES = 512 MiB`,
`MAX_TOTAL_RODATA_BYTES = 128 MiB`, and mandatory IR + register-allocation
verification (`verify: true` by default).

## 3. Runtime Stack, Memory & Blot ABI 8 Contract

### Register Convention
- Pinned: `Rsp` (16-byte aligned guest stack), `R15` (`Ctx*`), `R14` (`mem_base`,
  `NULL` for 0-capacity memories), `R13` (`mem_len`), `R12` (optional hottest
  mutable `i32`/`i64` global, synced at every C/host boundary).
- Scratch: `R11` / `Xmm15` (trap code `R11d = 0 | 1..22`, cycle break),
  `R10` / `Xmm14` (secondary scratch).
- Allocatable: GP `Rax, Rcx, Rdx, Rsi, Rdi, R8, R9` (caller-saved) + `Rbx, Rbp`
  (callee-saved); XMM `Xmm0..Xmm7` (caller-saved) + `Xmm8..Xmm13` (callee-saved).

### Dedicated Guest Stack & Signals
- `blot_aot_new` reserves a dedicated `64 KiB PROT_NONE guard + 8 MiB RW` guest
  stack via `mmap`/`mprotect` with `stack_floor = guard_end + 128 KiB`.
- Every non-elided function executes a wrap-proof prospective stack check before
  frame allocation; elided acyclic subtrees (`<= 8 KiB`) have their worst-case
  stack usage reserved in retaining callers.
- The 128 KiB headroom covers runtime helpers + up to 2 nested async signals
  (`<= 16 KiB` kernel xstate + `<= 32 KiB` handler frame each). Embedders with
  deeper/larger async signal handlers or CPUs where `AT_MINSIGSTKSZ > 16 KiB`
  must configure `SA_ONSTACK` + `sigaltstack`.

### Blot ABI 8 Exports (18 Symbols)
All three Blot backends (`x64`, `c`, `rust`) expose the same 18 C-ABI symbols:
- Metadata: `blot_aot_abi_version` (returns `1`), `blot_aot_trap_message`,
  `blot_aot_import_count`, `blot_aot_import_name` (exact byte lengths, preserving
  embedded NULs).
- Lifecycle: `blot_aot_new`, `blot_aot_drop` (no-op on `NULL` or while `in_call != 0`).
- Calls: `blot_aot_call_export`, `blot_aot_call_allocate`, `blot_aot_call_reset`,
  `blot_aot_call_finish_invocation` (return `0` ok, `1` Wasm trap, `2` host error /
  invalid reentry with `*trap_out = 19`, `3` missing/incompatible export or `NULL`
  handle on `call_export`).
- Memory & globals: `blot_aot_mem_data` (zeroes outputs on `NULL` handle or
  while `in_call != 0`), `blot_aot_mem_grow` (returns `-1` on `NULL` handle,
  `in_call != 0`, or limit failure), `blot_aot_get_global` (returns `1` on
  `NULL` handle or while `in_call != 0`), `blot_aot_set_global` (returns `1` on
  `NULL` handle or while `in_call != 0`).
- **Handle lifetime & serialization**: a `BlotHandle` is single-threaded and
  must not be called concurrently from multiple host threads. During a
  synchronous `host_fn` callback, the handle is marked `in_call != 0`; all
  handle-taking APIs fail closed before borrowing guest state, and callbacks
  must access guest memory, globals, or re-entrant exports exclusively through
  the `ActiveCall*` pointer supplied to `host_fn` (valid only until `host_fn`
  returns). Any pointer returned by `blot_aot_mem_data` or
  `blot_aot_active_mem_data` is valid only until the next `blot_aot_mem_grow`,
  guest call, or `blot_aot_drop`.
- Active callback APIs (valid only inside `host_fn`): `blot_aot_active_mem_data`,
  `blot_aot_active_allocate`, `blot_aot_active_reset`,
  `blot_aot_active_get_global`. Synchronous callback reentry is capped at
  `MAX_ACTIVE_REENTRY_DEPTH = 16` on `--backend x64` (`TRAP_CALL_STACK = 8`).
- **MXCSR isolation**: all three backends save host MXCSR on entry, execute Wasm
  under canonical `0x1f80` (subnormals preserved, round-to-nearest-even), restore
  host MXCSR around `host_fn` callbacks, and restore host MXCSR on normal or
  trapping return.
- **No cross-frame unwinding**: panics, C++ exceptions, and `longjmp` must never
  unwind across generated frames.
