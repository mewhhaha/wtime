# wtime-aot: WebAssembly compiled ahead of time to safe Rust

`wtime-aot` translates a WebAssembly module into a Rust crate. rustc and LLVM
then compile it to native code. wtime's own decoder and validator check the
module first. Nothing is generated or run at run time, and neither the
generator, the runtime crate nor the generated code uses `unsafe`.

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

The generated directory is a Cargo workspace: `core` holds the instance,
the host imports and the trait `Fns<M>`, which declares every function;
the crates `p0`, `p1`, ... hold the functions, about 12 MiB of source each,
generic over `M` and calling each other through `Fns<M>`; the program crate
implements `Fns` for its marker, which monomorphizes every function there
with direct calls. rustc's front end is sequential within a crate, so the
parts build in parallel and each in a fraction of the memory: the 64-bit
Goose compiler needed more than 13.5 GB as one crate and builds in 6 GB
(about 9 minutes on four cores) as nine.

## Translation

- **Functions** become methods of `Instance`. The operand stack becomes typed
  `let` bindings, and a `local.get` copies the local, so later `local.set`s
  cannot change values already on the stack.
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
- **Calls** count their depth. Recursion deeper than 1,000,000 calls traps
  instead of overflowing the program's 8 GiB native stack.
- **Numeric operators** keep Wasm semantics:
  - Division and remainder trap on zero, and signed division traps on
    overflow.
  - `min`/`max` return NaN if either operand is NaN, and order -0 below +0.
  - Trapping truncation checks for NaN and range against exact bounds;
    saturating truncation is Rust's `as`.
  - Shifts and rotations take their count modulo the width.
  - `nearest` rounds ties to even.
- **Memory:**
  - A 32-bit memory (`Memory32`) is one lazily zeroed allocation spanning
    every address a 32-bit access can form (8 GiB of address space, touched
    only where the program writes). An access checks a single bound, the
    current length; `memory.grow` moves the length.
  - A 64-bit memory (`Memory`) is a base, one lazily zeroed allocation as
    large as the OS grants up to the memory's maximum (64 GiB at most),
    then 4 GiB chunks, each lazily zeroed and allocated when `memory.grow`
    reaches it. Growing never moves or touches existing bytes, so a memory
    can grow to many GiB (the 64-bit Goose compiler grows to about 17 GB)
    while the program writes little of it. An access ending within the
    base checks one bound, as for a 32-bit memory; others take a cold path
    through the chunks.

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
env.x`, ...). Like `wtime-goose`, the host reads and writes only the memory
exported as `memory`, and only once instantiation, start function included,
has completed. Two differences remain: an access past the end of memory
reports `Trap: out of bounds` where the interpreter may report
`Trap: address overflow`, and the call depth limit is lower (see Calls).

## Not covered

- **Fuel and cancellation:** a compiled program runs unmetered and runs to
  completion. Use the interpreter where fuel or cancellation matters.
- **The interpreter's resource limits, other than call depth.**

## Toolchain

Build generated crates with a toolchain whose LLVM keeps float division
exact. rustc 1.99.0, the version this repository pins, folds
`0.0 / (b as f64)` to `0.0` when it can see that `b` is 0 or 1
(`fdiv 0.0, uitofp i1` becomes 0.0 at opt-level 1 and above). If `b` is 0,
the result should be NaN. Generated code reaches that pattern whenever a
comparison result is converted to a float and divides, and differential
fuzzing found such a program. rustc 1.97.0 compiles it correctly. The
generator itself, and the runtime crate's tests, are unaffected.
