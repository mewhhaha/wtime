# wtime-aot: Ahead-of-time WebAssembly compiler (Rust, C99/Clang, and direct x86-64)

`wtime-aot` validates a WebAssembly module with wtime's own decoder and validator,
then compiles it ahead of time through one of three backends:

- **`--backend rust`** (default): translates Wasm into safe Rust crates (`#![forbid(unsafe_code)]`)
  compiled by `rustc`/LLVM — supports standalone Goose programs and `--blot-cdylib` shared libraries
  with peak LLVM `-O3` + ThinLTO throughput.
- **`--backend c`** (`--blot-cdylib`, `--c-opt auto|O2|O1|O0`): emits sharded C99 with a
  content-addressed per-shard object cache (`c-obj/`) compiled with Clang and linked with `ld.lld`
  (plus `-fcf-protection=return` CET SHSTK and calling-thread stack checks on Linux x86-64).
- **`--backend x64`** (`--blot-cdylib`, Linux x86-64 LP64): a self-contained pure-Rust
  (`#![forbid(unsafe_code)]`) SSA optimizer, register allocator, x86-64 assembler, and ELF64 `.so`
  linker with zero external compilers, zero subprocesses, and zero `libc`/`DT_NEEDED` runtime
  dependencies.

Nothing is JIT-compiled at run time.

## Backends and benchmarks

Measured on AMD Ryzen 7 7800X3D (8C/16T), Linux 7.3.5, Clang 23.1.5, Rust 1.99.0, and Node v26.5.0
with exact output/packet SHA-256 parity verified across every engine. Reproduce the dataset and all
four category charts with:

```sh
python3 -B tools/wtime-aot/bench.py --cold-rounds 5 -o tools/wtime-aot/charts/results.json
python3 -B tools/wtime-aot/chart.py tools/wtime-aot/charts/results.json -o tools/wtime-aot/charts
```

### Blot game runtime (`gdev`, 3.57 MiB, 4,935 functions)

![Blot game runtime on gdev](charts/runtime-gdev.svg)

### Representative Wasm kernels runtime

![Representative Wasm kernels runtime](charts/runtime-kernels.svg)

### Cold compilation time (empty cache, multi-core)

![Cold compilation time](charts/compile-cold.svg)

### Hot compilation, incremental rebuild, & warm load

![Hot compilation, incremental rebuild, and warm load](charts/compile-hot.svg)

## Usage

For modules from untrusted sources, use `wtime-aot build` (see
[Uploaded modules](#uploaded-modules)):

```sh
# Standalone sandboxed Goose program (--backend rust)
program=$(target/release/wtime-aot build upload.wasm --cache /srv/aot)
$program --allow-files /srv/job/42 --time-limit 10 -- program arguments < input

# Blot shared library with the direct pure-Rust x86-64 backend (Linux x86-64 LP64)
lib=$(target/release/wtime-aot build game.wasm --cache /srv/aot --blot-cdylib --backend x64)

# Blot shared library with the sharded C99/Clang backend
lib=$(target/release/wtime-aot build game.wasm --cache /srv/aot --blot-cdylib --backend c --c-opt auto)
```

To generate a crate by hand:

```sh
cargo build --release                                       # the generator
target/release/wtime-aot module.wasm -o out/ [--name NAME] [--module-path PATH]
cd out && cargo +1.97.0 build --release                     # the program (see Toolchain)
out/target/release/aot-module [--timing-json path] [-- program arguments]
```

The generated program runs a direct Goose module (goose-to-wasm) the way
`tools/wtime-goose` runs it on the interpreter: the same imports, standard
output, exit codes and timing JSON. `tools/wtime-goose/bench.py --aot-dir`
benchmarks such programs as engine `wtime-aot`.

## Output

The generated directory is a Cargo workspace: `core` holds `Ctx` (hot
fields `stack_floor`, memory metadata and mutable globals laid out first
under `#[repr(C)]`), `Instance` (owning `Ctx` and the disjoint linear
memory buffer), the host imports (`HostView` zero-allocation bridge), and
the trait `Fns<M>`; the crates `p0`, `p1`, ... hold the functions
balanced by Longest-Processing-Time-first bin-packing (~6 MiB of source
each, up to 12 parts), generic over `M` and calling each other through
`Fns<M>`; the program crate implements `Fns` for its marker, which
monomorphizes every function there with direct calls and thin LTO (`codegen-units = 1`
for modules up to 4 MiB, `16` for larger modules). Parallel balanced part
crates and thin LTO build the 32-bit and 64-bit Goose compilers in under
one minute each on 8 cores.

## Translation

- **Functions** take `(ctx: &mut Ctx, mb, [preserved_globals...], params...)`,
  keeping linear memory (`mb`) disjoint (`noalias`) from `Ctx`. The operand
  stack becomes typed `let` bindings, and a `local.get` copies the local, so
  later `local.set`s cannot change values already on the stack.
- **Global analyses:**
  - Effectively constant globals (unwritten globals, or globals whose every
    reachable `global.set` writes back their own value) are evaluated at
    generation time and inlined as literals.
  - A greatest-fixed-point structured-control-flow analysis proves which
    globals each function preserves across normal exits (including
    loop-header invariants). Module-wide preserved globals (up to 4) are
    passed by value in registers to functions that transitively use them;
    per-function preservation omits redundant caller reloads and dead
    return stores.
  - Non-returning host imports (`goose.abort`, `goose.exit`, `proc_exit`)
    compile as `#[cold] #[inline(never)] -> Trap`, avoiding dead global
    stores/reloads on abort paths.
- **Control flow:**
  - Blocks become labeled Rust blocks. A block's results are variables
    declared before it.
  - Loops become labeled `loop`s whose parameters are variables reassigned
    before `continue`.
  - An `if` without `else` passes its parameters through as results.
  - A branch assigns its target's values through a tuple, so that values
    can be exchanged, then `break`s, `continue`s or returns.
  - Code after an unconditional branch is skipped up to its `else` or `end`.
- **Traps** are `Err(Trap)`, a one-byte code, returned through every call.
  So are host failures, whose text the host keeps.
- **Calls** check the native stack left, as Wasmtime's do: a function
  whose frame comes within an eighth of the stack's end (4 MiB at least)
  traps with `ResourceLimit: call depth exhausted` instead of overflowing
  it. Small acyclic call subtrees (bounded locals, instructions and call
  depth, never in a recursive SCC or `goose.reserve` cycle) elide the
  redundant prologue check. How deep recursion goes depends on the frames
  (the default stack is 1 GiB, `--stack-limit-mib`).
- **Numeric operators** keep Wasm semantics:
  - Division and remainder trap on zero, and signed division traps on
    overflow.
  - `min`/`max` return NaN if either operand is NaN, and order -0 below +0.
  - Trapping truncation checks for NaN and range against exact bounds;
    saturating truncation is Rust's `as`.
  - Shifts and rotations take their count modulo the width.
  - `nearest` rounds ties to even.
- **Memory:**
  - A 32-bit memory (`Mem32Meta` + `alloc32::<CAP32>()`) is one lazily
    zeroed fixed-capacity array `&mut [u8; CAP32]` (`max_pages * 65536`),
    passed disjointly from `Ctx`. Accesses go through zero-panic `m32`
    helpers (`first_chunk` for zero-offset `>= 4`-byte accesses so loops
    auto-vectorize, `last_chunk` for offset/narrow accesses, direct `.get`
    for bytes); fixed-size memories fold `ml` to `CAP32` as a constant.
  - Generated functions cache the active direct memory length `ml` in a
    local with a dominating range guard (`MIN_BYTES <= ml <= cap`), and
    refresh `ml` only after `memory.grow` or calls that transitively may
    grow memory.
  - A chunked 64-bit memory (`Outer` + `Vec<u8>`) allocates a lazily zeroed
    base up to the OS grant (64 GiB at most), then 4 GiB chunks on demand
    without moving existing bytes. Fast-path misses use register-returned
    `#[cold]` slow paths (`Option<u64>` / `bool`, zero `sret`). Non-growing
    loops and blocks specialize under `if mf { <flat m64> } else { <chunked m64> }`
    while `len == fast` (all bytes reside in the base).
  - A *flat* 64-bit memory (`--flat-memory64`; `wtime-aot build` chooses
    it when the machine grants the declared maximum in one allocation,
    and keys the cache by the choice) allocates a fixed-capacity array
    `Box<[u8; CAP64]>` (`alloc64::<CAP64>()`) passed in a single register
    and accessed via `m64f` (single hoisted `cmpq (ml - offset - N), addr;
    ja trap` even across `memory.grow` PHI nodes). Where the maximum cannot
    be allocated when the program starts, it fails with `ResourceLimit:
    memory allocation failed`.

## Supported

- One memory, 32- or 64-bit, with 64 KiB pages and not shared.
- Function imports served by the Goose host (`wtime_aot_rt::goose`).
- Numeric globals and active data segments, with constant initializers
  that may use the extended constant operators (integer add, sub, mul).
- Tables only as far as instantiation goes: an active element segment is
  bounds checked against its table and traps if it does not fit, as the
  interpreter does. No instruction may use a table: no `call_indirect`,
  `br_table` or table instructions.
- Sign extension, saturating truncation, `memory.copy` and `memory.fill`.
- The SIMD instructions that treat `v128` as 16 bytes: load, store, const,
  and i64x2 splat, extract_lane and replace_lane.

The generator refuses anything else in live code with an error naming the
construct. Dead code (after an unconditional branch) is skipped unread,
except that exception-handling instructions are refused there too, since
skipping relies on knowing where each block ends.

## Host parity with wtime-goose

The program reports what `wtime-goose` reports for the same module: the
output written before a failure (also when the start function fails), the
exit code, and the failure text (`Trap: integer divide by zero`,
`ResourceLimit: call depth exhausted`, `unsupported direct host import:
env.x`, ...). One difference is deliberate: files and environment variables are denied
until the operator grants them (see Uploaded modules), where `wtime-goose`
grants the process's own; `--allow-files / --allow-env '*'` gives a
compiled program the same reach. Like `wtime-goose`, the host reads and writes only the memory
exported as `memory`, and only once instantiation, start function included,
has completed. Two differences remain: an access past the end of memory
reports `Trap: out of bounds` where the interpreter may report
`Trap: address overflow`, and recursion stops where the native stack does
rather than at the interpreter's 4,000,000 calls (see Calls).

## Not covered

- **Fuel and cancellation:** a compiled program runs unmetered; only the
  process's time limit (`--time-limit`) or the OS ends it early. Use the
  interpreter where fuel or in-process cancellation matters.
- **The interpreter's other resource limits:** only the stack and memory
  pages (2^20, as under wtime-goose) are enforced.

## Uploaded modules

The uploader hands over the Wasm, and nothing else. `wtime-aot build
module.wasm --cache DIR` validates it, generates the crates and compiles
them against this runtime crate with the operator's toolchain
(`--toolchain`, default `stable`; see Toolchain), then prints the path of
the program. The program is cached under the SHA-256 of everything that
determines it: the Wasm, the module name the program reports (the file's
own name unless `--module-path`), the generator's binary, the runtime
crate's sources, the toolchain's `rustc -vV` and the target CPU with
the features it resolves to; a cached program is reused, a change to any
input builds a new one. Programs are built for the machine's own CPU
(`--target-cpu`, default `native`, as a JIT compiles for its host): a
cache shared between machines keeps a program per CPU.

`--train PROGRAM-ARGUMENTS...` (last on the command line) makes the build
profile-guided: an instrumented program is built and run once with those
arguments (its own flags, such as `--allow-files`, then `--` and the
guest's arguments, chosen by the operator), in the same sandbox and under
the same `--time-limit` as any run, from the current directory; rustc then
builds the program again with the profile that run wrote (`-C
profile-use`, merged by the toolchain's `llvm-profdata`: `rustup component
add llvm-tools`). The profile steers code layout and inlining, never what
the program does. Both programs are cached, the profile-guided one under a
key that also covers the profile, so the same training gives the same
program. A training run that exits with an error still profiles the code
it ran; one ended by a signal or the time limit fails the build.

```sh
program=$(target/release/wtime-aot build compiler.wasm --cache /srv/aot \
  --train --allow-files /srv/sample -- /srv/sample/input.goose -o /dev/null)
```

What makes an uploaded module safe to run:

- **Only trusted code is compiled.** The generated source is a function of
  the Wasm made by this generator: the module's names reach it only as
  escaped string literals and comments, its data only as a data file. The
  generated crates contain no `unsafe` (`#![forbid(unsafe_code)]` in every
  crate, the runtime's included), no build scripts and no dependencies but
  the runtime crate. Cargo runs `--offline` in an environment reduced to
  `PATH`, `HOME`, `RUSTUP_HOME` and `CARGO_HOME`, so no `RUSTFLAGS`,
  wrappers or registries from the caller apply.
- **The module reaches only its host imports.** Its memory accesses are
  bounds checked, its calls and branches stay inside its own functions, and
  a trap ends it. The host gives it standard input and output, its
  arguments and the clock. Files and environment variables it gets only
  as the operator grants them:
  - `--allow-files DIR` (repeatable): the module may read, write and test
    files under DIR. A path is checked by its text first (no `..`; made
    absolute against the working directory, it must lie under a granted
    directory), so nothing outside is even looked at; then it is resolved
    through links and checked again. A link that cannot be resolved is
    refused, and writes do not follow a link in the last component.
    Anything else looks to the module like a file that does not exist (or
    cannot be written).
  - The module's `goose.reserve` may not call back into the host (into
    another reservation or file read): host buffers cannot pile up, and the
    host cannot recurse.
  - `--allow-env NAME` (repeatable, `*` for all): the variables it may
    read; others look unset.
- **The operator bounds its resources:**
  - `--time-limit SECONDS` ends the process (exit 1, `ResourceLimit: time
    limit`, its buffered output dropped).
  - `--memory-limit-pages N` lowers the memory limit below the default 2^20
    pages (64 GiB).
  - `--output-limit BYTES` (default 256 MiB) bounds what the module writes to
    standard output, standard error and files together, and the size of a
    file it may read; output is buffered until the program ends, so this
    also bounds the host's memory.
  - `--stack-limit-mib N` (default 1024, from 16 to 65536) sets the native
    stack, and with it what a runaway recursion costs before it traps
    (about 900 MiB by default). Calls trap with `ResourceLimit: call depth exhausted` when a
    frame comes within an eighth of the stack's end (4 MiB at least), so
    neither deep recursion nor large frames can overflow it.

  A compiled program cannot be metered by fuel or cancelled from inside:
  run it as a process of its own and use the OS (a cgroup) for anything
  finer, such as CPU or a hard bound on resident memory.
- **The build is bounded too.** The generator refuses modules over 256 MiB
  or 100 million instructions, more than 1,000,000 locals in all, blocks
  nested deeper than 512 (rustc's own stack gives out near 1,000) and
  generated source over 1 GiB, and its work is linear in the module
  (nesting, operand stacks and constant expressions cannot make it grow
  faster). rustc's time is not linear: a valid 28 KB module of 2,000 locals
  chained through 50 loops takes over a minute. `wtime-aot build
  --time-limit SECONDS` (default 3600) ends a build, rustc included; run
  builds under a cgroup to bound their memory. Each build has its own
  target directory, removed afterwards, success or not. A build refuses to
  run where an ancestor of the cache has a `.cargo` configuration, which
  could name a program for Cargo to run.

Not guarded against:

- Another local process swapping links inside a granted directory between
  the check and the access.
- Cargo configuration in `CARGO_HOME` (the operator's own) applies to
  builds, `target.<triple>.rustflags` there overriding `--target-cpu`; keep
  the cache under directories only the operator can write.
- rustc's memory grows faster than linearly with large, call-dense
  functions (a 200 KB module of one 10,000-call function takes 8 GB): run
  builds under a cgroup.
- rustc or LLVM miscompiling the generated code: pin `--toolchain` to a
  version you have checked (see Toolchain); the default, `stable`, moves.

## Toolchain

Build generated crates with a toolchain whose LLVM keeps float division
exact. rustc 1.99.0, the version this repository pins, folds
`0.0 / (b as f64)` to `0.0` when it can see that `b` is 0 or 1
(`fdiv 0.0, uitofp i1` becomes 0.0 at opt-level 1 and above). If `b` is 0,
the result should be NaN. Generated code reaches that pattern whenever a
comparison result is converted to a float and divides, and differential
fuzzing found such a program. rustc 1.97.0 compiles it correctly. The
generator itself, and the runtime crate's tests, are unaffected.
