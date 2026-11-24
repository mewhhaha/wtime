# wtime-aot: Ahead-of-time WebAssembly compiler (Rust, C99/Clang, direct x86-64, and AArch64)

`wtime-aot` is a standalone opt-in AOT compiler excluded from the core workspace
dependency graph. It validates each WebAssembly module with `wtime`'s decoder and
validator and compiles it ahead of time through one of four backends:

- **`--backend rust`** (default): translates admitted Wasm into safe-Rust crates
  (`#![forbid(unsafe_code)]` for all generated Wasm code and `wtime-aot-rt`)
  compiled by `rustc`/LLVM — supports standalone Goose executables and
  `--blot-cdylib` shared libraries (the latter adds a thin C-ABI FFI boundary
  for opaque instance/active-call handles, MXCSR isolation, and calling-thread
  stack-floor checks).
- **`--backend c`** (`--blot-cdylib`, `--c-opt auto|O2|O1|O0`): emits sharded
  C99 with a content-addressed, SHA-256-verified per-shard object cache
  (`c-obj/`) compiled by Clang/GCC and linked with `ld.lld`/`ld` (with
  `-fcf-protection=return` CET shadow-stack hardening, `-fstack-clash-protection`,
  MXCSR isolation, and calling-thread stack checks on Linux x86-64).
- **`--backend x64`** (`--blot-cdylib` or standalone static Linux x86-64
  `ET_EXEC`): a self-contained pure-Rust (`#![forbid(unsafe_code)]`) SSA
  optimizer, register allocator, independent IR/allocation verifier (enabled by
  default in `X64CompileOptions`, single-shot CLI, and debug builds; opt-in via
  `WTIME_AOT_VERIFY=1` for release `build --cache`), x86-64 assembler, and
  ELF64 linker with zero external compilers, zero subprocesses, and zero
  `libc`/`DT_NEEDED` runtime dependencies. Supports `memory32`, `memory64`, and
  all 236 fixed-width 128-bit Wasm SIMD opcodes. See
  [docs/wtime-aot/x64-abi.md](../../docs/wtime-aot/x64-abi.md) for the exact
  admission, ABI, stack, signal, and CET `rdsspq` contract.
- **`--backend aarch64`** (`--blot-cdylib` or standalone static Linux AArch64
  `ET_EXEC`): a self-contained pure-Rust (`#![forbid(unsafe_code)]`) ARMv8-A
  backend emitting Linux AArch64 ELF64 shared libraries (Blot ABI 8) and static
  executables with zero external compilers or cross-toolchains.

Nothing is JIT-compiled at run time.

## Backends and benchmarks

Measured on AMD Ryzen 7 7800X3D (8C/16T), Linux 7.3.5, Clang 23.1.5, Rust 1.99.0,
Wasmtime 49.0.3, and Node v26.5.0 (`2026-11-24`) with exact output parity
verified across every engine (`tools/wtime-aot/charts/results.json`; full tables
and methodology in [docs/benchmarks.md](../../docs/benchmarks.md)).

Reproduce the self-contained kernel dataset and charts anywhere with `clang`,
`rustc`, and `node`:

```sh
cargo build --locked --release --manifest-path tools/wtime-aot/Cargo.toml
python3 -B tools/wtime-aot/bench.py --kernels-only --cold-rounds 5 -o target/aot-kernels.json
python3 -B tools/wtime-aot/chart.py target/aot-kernels.json -o target/aot-charts
```

Pass `--wasmtime-driver PATH` and/or `--large-wasm PATH` to include Wasmtime
and a multi-thousand-function `.wasm` module in the cold, incremental, and warm
load benchmarks.

### Representative Wasm kernels runtime

![Representative Wasm kernels runtime](charts/runtime-kernels.svg)

### Cold compilation time (empty cache, multi-core)

![Cold compilation time](charts/compile-cold.svg)

### Incremental rebuild, warm load, & cache hit

![Incremental rebuild, warm load, and cache hit](charts/compile-hot.svg)

## Usage

For untrusted modules, use `wtime-aot build` (see [Uploaded modules](#uploaded-modules)):

```sh
# Standalone sandboxed Goose program (--backend rust)
program=$(tools/wtime-aot/target/release/wtime-aot build upload.wasm --cache /srv/aot)
$program --allow-files /srv/job/42 --time-limit 10 -- program arguments < input

# Blot shared library with the direct pure-Rust x86-64 or AArch64 backend (zero external compiler deps)
lib=$(tools/wtime-aot/target/release/wtime-aot build game.wasm --cache /srv/aot --blot-cdylib --backend x64)
lib_arm64=$(tools/wtime-aot/target/release/wtime-aot build game.wasm --cache /srv/aot --blot-cdylib --backend aarch64)

# Standalone static Linux executable with pure-Rust x86-64 or AArch64 backend
bin=$(tools/wtime-aot/target/release/wtime-aot build prog.wasm --cache /srv/aot --backend x64)

# Blot shared library with the sharded C99/Clang backend
lib=$(tools/wtime-aot/target/release/wtime-aot build game.wasm --cache /srv/aot --blot-cdylib --backend c --c-opt auto)
```

To generate a Rust crate by hand:

```sh
cargo build --locked --release --manifest-path tools/wtime-aot/Cargo.toml
tools/wtime-aot/target/release/wtime-aot module.wasm -o out/ [--name NAME] [--module-path PATH]
cd out && cargo build --release
out/target/release/aot-module [--timing-json path] [-- program arguments]
```

## Supported Features by Target

- **Goose standalone (`--backend rust`)**:
  - One non-shared 32- or 64-bit memory with 64 KiB pages.
  - Direct Goose host imports (`wtime_aot_rt::goose`), numeric globals, active
    data segments, extended-const initializers, active element bounds checks
    (no table instructions or `call_indirect`), sign extension, saturating
    truncations, `memory.copy`/`memory.fill`, and 16-byte `v128` load/store/const
    + `i64x2` splat/extract/replace.
- **Blot ABI 8 shared libraries (`--blot-cdylib` across `x64`, `aarch64`, `c`, `rust`)** and **pure-Rust static Linux executables (`--backend x64` / `--backend aarch64`)**:
  - One non-shared Wasm32 memory (`max <= 65_536` pages; plus `memory64` and all
    236 fixed-width 128-bit Wasm SIMD opcodes on `--backend x64`), static 32-bit
    nullable `funcref` tables with `call_indirect`, mutable 32-bit nullable
    `externref` tables with `table.get`/`table.set`, `br_table`, multi-value
    blocks/functions, Blot host function imports (including byte-exact embedded
    NUL names), exported globals (including `externref`), and all 18 Blot ABI 8
    C exports with identical MXCSR (`0x1f80`) isolation, `in_call` reentry/drop
    guards, and null-handle defensive behavior.

Unsupported constructs are rejected before codegen (`--backend x64` uses exit
code `4` for admission rejections vs. exit code `1` for I/O, timeout, or
internal errors).

## Not covered

- **Fuel and in-process cancellation**: compiled artifacts run unmetered; use
  the safe-Rust core interpreter (`wtime-runtime`) where fuel or in-process
  cancellation is required, or isolate AOT binaries with OS cgroups and
  `--time-limit`.
- **GC, exception handling, threads/atomics, continuations, multi-memory, custom
  page sizes**: handled by the core interpreter (`wtime-runtime`), not
  `wtime-aot`.

## Uploaded modules & Threat Model

`wtime-aot build module.wasm --cache DIR` validates the input, generates the
artifact, and caches it under a SHA-256 key binding all compiler, runtime,
toolchain, target-CPU, and active Cargo-config inputs (with owner-private `0700`
directories and immutable SHA-256 sidecars).

Per-backend input and build bounds:
- **Max input `.wasm` size**: `64 MiB` for `--backend x64` and
  `--backend aarch64`; `256 MiB` for `--backend c` and `--backend rust` (inputs
  must be regular files, opened non-blockingly on Unix to reject FIFOs/devices
  immediately, and read in deadline-checked 64 KiB chunks before full
  buffering).
- **Build timeout**: `--time-limit SECONDS` (default `3600`) bounds regular-file
  reads, validation, codegen, and child compiler/linker processes (run builds on
  untrusted inputs inside an OS memory cgroup as well).
- **Source & compiler bounds**: `--backend rust`/`c` cap generated source at
  `1 GiB` (cooperatively enforced across parallel C workers), nesting at `512`,
  instructions at `100M`, and locals at `1M`. `--backend x64` enforces tighter
  per-function IR, liveness (`16 MiB` per function / `256 MiB` concurrent), and
  emitted `.text`/`.rodata` caps (see [x64-abi.md](../../docs/wtime-aot/x64-abi.md)).
- **Goose runtime sandbox (`wtime-aot-rt`)**: `--allow-files DIR`,
  `--allow-env NAME`, `--time-limit SECONDS` (bounds stalled stdin as well as
  guest execution), `--memory-limit-pages N`, `--output-limit BYTES` (default
  `256 MiB`), and `--stack-limit-mib N`.
- **Toolchain trust boundary**:
  - `--backend rust` runs `cargo --offline` with a sanitized environment
    (`PATH`, `HOME`, `RUSTUP_HOME`, `CARGO_HOME`) and snapshots `wtime-aot-rt`
    into a private per-build workspace; `--toolchain` defaults to `1.99.0`
    (matching `rust-toolchain.toml`). Both `f32.div` and `f64.div` shield zero
    divisors via `wtime_aot_rt::f32_div`/`f64_div` so Rust 1.99.0/LLVM cannot
    mis-fold `0.0 / (cmp as f64)` from `NaN` to `0.0`.
  - `--backend c` runs the operator's host C compiler (`clang`/`gcc`) and linker
    with a sanitized environment and verifies compiler/linker/header identity
    before publishing to `c-obj/`; host C compiler soundness is part of the
    build trust boundary.
  - `--backend x64` invokes no external compilers or linkers at all, includes an
    independent IR and register-allocation verifier (`WTIME_AOT_VERIFY=1` for
    release `build --cache`; enabled by default in single-shot CLI and debug
    builds), and fails closed (`TRAP_UNSUPPORTED_ISA = 22`) on CPUs missing
    SSE4.1/POPCNT (or ABM/BMI1 for `x64-v3`) or on threads with active Linux
    CET hardware shadow stacks (`rdsspq != 0`).
  - `--backend aarch64` likewise invokes no external compilers or linkers and
    emits Linux AArch64 ELF64 Blot ABI 8 `.so` libraries and static `ET_EXEC`
    executables directly in pure safe Rust.
