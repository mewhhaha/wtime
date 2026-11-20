# wtime-aot: WebAssembly compiled ahead of time to safe Rust

`wtime-aot` translates a WebAssembly module into a Rust crate. rustc and LLVM
then compile it to native code. wtime's own decoder and validator check the
module first. Nothing is generated or run at run time, and neither the
generator, the runtime crate nor the generated code uses `unsafe`.

```sh
cargo build --release                                       # the generator
target/release/wtime-aot module.wasm -o out/ [--name NAME] [--module-path PATH]
cd out && cargo build --release                             # the program
out/target/release/aot-module [--timing-json path] [-- program arguments]
```

The generated program runs a direct Goose module (goose-to-wasm) the way
`tools/wtime-goose` runs it on the interpreter: the same imports, standard
output, exit codes and timing JSON. `tools/wtime-goose/bench.py --aot-dir`
benchmarks such programs as engine `wtime-aot`.

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
  - A 64-bit memory (`Memory`) is a vector as long as the memory, with its
    capacity reserved up to 4 GiB. Growing it writes the new zero bytes.

## Supported

- One memory, 32- or 64-bit, with 64 KiB pages and not shared.
- Function imports served by the Goose host (`wtime_aot_rt::goose`).
- Numeric globals with constant initializers, and active data segments.
- The MVP instruction set without tables: no `call_indirect` or `br_table`.
- Sign extension, saturating truncation, `memory.copy` and `memory.fill`.
- The SIMD instructions that treat `v128` as 16 bytes: load, store, const,
  and i64x2 splat, extract_lane and replace_lane.

The generator refuses anything else with an error naming the construct.

## Not covered

- **Fuel and cancellation:** a compiled program runs unmetered and runs to
  completion. Use the interpreter where fuel or cancellation matters.
- **The interpreter's resource limits, other than call depth.**
