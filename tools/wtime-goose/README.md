# wtime-goose: Goose-compiled Wasm on wtime vs Wasmtime

`wtime-goose` runs direct WebAssembly modules produced by
[goose-to-wasm](https://github.com/mewhhaha/goose-to-wasm) on the wtime interpreter. It is a
port of goose-to-wasm's Wasmtime host (`host/rust`, `goose-wasmtime`), with the same
imports (`goose.*`, `env.gs_os_*`, `env.bench_*`, the WASI subset), program arguments,
stdin, output and exit codes, and the same timing JSON boundaries. That makes it possible
to run identical modules on both engines and compare them.

It is an isolated crate (own `[workspace]`), like `tools/wtime-bench`. `float.rs` and
`pytext.rs` are copied from goose-to-wasm.

## Results (2026-11-20)

The first comparison found several pathologies in wtime; later rounds made the Decoded tier
itself faster. Every run produced output byte-identical to Wasmtime and to the expected files.
When the Goose compiler itself runs inside wtime, the Wasm it emits matches the native
compiler's output byte for byte.

Guest execution time (`_start`), default builds of the Decoded tier. "Original" is wtime
`2e77f35`, "round 1" is `7451d25` (pathology fixes), `8f86fdb` added straight-line runs, and
"now" is `fa2128c` (lowered straight-line code, below). Ratios are medians of per-round
wtime/Wasmtime pairs; averages are geometric means.

| Workload | Wasmtime 50.0.1 | Original | Round 1 | `8f86fdb` | Now | Now / Wasmtime |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Ten example programs (average vs Wasmtime) | | 226× | | 27× | 27× (1.1–123×) | |
| `sorting` example | 175–299 ms | 277 s | 97 s | 38.6 s | 27.0 s | 104× (was 1,871×) |
| `life` example | 22 ms | 16.2 s | | 2.44 s | 2.69 s | 119× (was 736×) |
| `sudoku` example | 10–13 ms | 7.9 s | 2.6 s | 1.25 s | 1.78 s | 123× (was 889×) |
| `sieve` example | 0.6–0.7 ms | 598 ms | 173 ms | 83 ms | 84 ms | 112× (was 1,026×) |
| Compiler (memory32) compiling 4 examples (average vs Wasmtime) | | 318× | 114× | 70× | **57×** | |
| Compiler compiling `functions.goose` | 21–29 ms | 12.3 s | 4.9 s | 2.25 s | 2.11 s | 68× (was 476×) |
| Compiler type-checks its own sources (`--check`) | ≈2.5 s | 1,041 s | 379 s | 218–242 s | 164 s | ≈65× (was ≈410×) |
| Default **memory64** compiler compiling `functions.goose` | 33–35 ms | could not run | 5.4 s | 2.96 s | 3.27 s | 87× |
| Same memory64 compiler, `--check` on its own sources | | could not run | 442 s | 266–295 s | 244 s | |

The `fa2128c` run fell in a slower period of this shared VM (Wasmtime's own `sorting` took
299 ms instead of 175 ms), so compare ratios, or the `8f86fdb` build measured in the same
run: there it took 49.6 s on `sorting`, 3.74 s on `life`, 2.08 s on `sudoku` and 130 ms on
`sieve`, so the lowered code is 1.84×, 1.39×, 1.17× and 1.54× faster on them. The average over
all ten examples stays near 27× because the four smallest programs run for well under a
millisecond and are dominated by process start-up.

Preparation and memory for the 7.3 MB compiler module:

| | Wasmtime (uncached / cached) | Original | Now |
| --- | ---: | ---: | ---: |
| Preparation + instantiation | 5.4–7.5 s / 7 ms | 3.0–3.3 s | 0.20–0.29 s |
| Whole process compiling `vm.goose` | 6.1 s / 26 ms | 5.8 s | 0.69–0.87 s |
| Peak RSS (memory32 / memory64 compiler, `--check`) | 137 / 242 MiB uncached | 4,645 MiB / — | 410 / 448 MiB |

Host instructions (callgrind, the most stable metric on this VM): `sieve` 5.57 G originally,
2.80 G after round 1, 1.36 G at `8f86fdb`, 0.88 G now; the compiler compiling `vm.goose`
12.6 G after round 1, 6.3 G at `8f86fdb`, 5.0 G now, of which about 1.0 G is decoding and
validating the module. Full tables: [now](results/fa2128c/tables.md),
[`8f86fdb`](results/8f86fdb/tables.md), [round 1](results/after/tables.md) and
[original](results/before/tables.md). Raw samples, binary hashes and RSS are in
`results/*/*.json`. Benchmarks ran on a shared 4-vCPU VM partly concurrently with builds and
other benchmarks; self-check ranges span separate runs.

**What remains.** wtime is still an interpreter; Wasmtime compiles to native code, and wtime
excludes JIT compilation by design, so guest code stays roughly 10–120× slower on real
programs. A lowered dispatch still costs about 30 host instructions before its operation runs
(fetch, bounds, cancellation, fuel and operand-stack checks, an indirect jump), and operands
are 32-byte `Value`s on a `Vec`; grouping removes about a third of the dispatches. For short
compiler runs, decoding and validating the 7.3 MB module (≈200–260 ms) now dominates. The
experimental Compact and Register tiers do not help here: Compact admits nothing in these
modules (its module-wide preparation budget is 100,000 work units) and Register falls back to
Decoded for almost every frame.

### Against V8 (Deno)

V8 15.0 runs through Deno 2.9.6 with goose-to-wasm's own JavaScript host
(`scripts/run_wasm.mjs`, the same imports and timing JSON); `bench.py --deno D --v8-host H`
adds it as engine `v8`. V8 compiles lazily: Liftoff first, TurboFan for hot code, on
background threads. "Process" is the whole process, including Deno's own start-up
(about 50 ms). Ratios are wtime/V8, below 1 when wtime is faster.

One CPU per engine (`b2c2b04`, before parallel preparation; [tables](results/v8-b2c2b04/tables.md)):

| Example | V8 run | wtime run | wtime / V8 run | V8 process | wtime process | wtime / V8 process |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| squares, tour, functions, errors, maze, vm | 0.5–4.5 ms | 0.0–2.8 ms | 0.06–1.4× | 47–74 ms | 4.5–10 ms | **0.06–0.15×** |
| sieve | 7.6 ms | 65 ms | 10× | 56 ms | 69 ms | 1.24× |
| sudoku | 18 ms | 1.16 s | 65× | 68 ms | 1.16 s | 18× |
| life | 26 ms | 2.02 s | 80× | 72 ms | 2.03 s | 29× |
| sorting | 360 ms | 24.7 s | 69× | 435 ms | 24.7 s | 58× |

The Goose compiler as Wasm, three CPUs for every engine (`dc33870`, bodies prepared in
parallel; [tables](results/v8-dc33870/tables.md)):

| Case | V8 prep | wtime prep | V8 run | wtime run | V8 process | wtime process | wtime / V8 process |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| compile-squares | 19 ms | 101 ms | 109 ms | 27 ms | 225 ms | 149 ms | **0.65×** |
| compile64-vm | 21 ms | 130 ms | 610 ms | 582 ms | 762 ms | 721 ms | ≈1× (paired 1.07×) |
| compile-vm | 16 ms | 104 ms | 174 ms | 425 ms | 299 ms | 552 ms | 1.8× |
| compile-sudoku | 16 ms | 107 ms | 175 ms | 427 ms | 310 ms | 552 ms | 1.9× |
| compile64-functions | 24 ms | 121 ms | 751 ms | 3.08 s | 907 ms | 3.24 s | 3.4× |
| compile-functions | 16 ms | 100 ms | 234 ms | 1.93 s | 370 ms | 2.04 s | 5.5× |

wtime is ahead wherever start-up and preparation dominate: every short example finishes
in a sixth to a fifteenth of V8's (and uncached Wasmtime's) process time, and the compiler on
a small input beats V8 and uncached Wasmtime (Wasmtime with a warm compiled-module cache stays
faster). Where guest code runs long, V8's and Wasmtime's machine code is 10–80×
faster than wtime's interpreter; wtime's safe-Rust, JIT-free design rules out closing that
gap by generating code at run time.

### Ahead of time (wtime-aot)

![wtime-aot's speed-up over Wasmtime and V8 running each program](charts/run.svg)

![wtime-aot's speed-up over cached Wasmtime and V8 for the whole process](charts/process.svg)

Each bar is the median, over paired rounds, of the other engine's time divided by
wtime-aot's: right of parity wtime-aot is faster. `python3 chart.py <reports> -o charts`
redraws both from the latest results.

[`tools/wtime-aot`](../wtime-aot/README.md) closes the gap before run time instead: it
translates a module to safe Rust crates (bounds-checked memory, Wasm's numeric
semantics, no `unsafe`) that rustc compiles once; `wtime-aot build` does so for
uploaded modules through a content-keyed cache, for the machine's own CPU, and the
programs run sandboxed (files and variables only as granted; time, memory, output
and stack limits). Every output below matches Wasmtime's, and the AOT-compiled Goose
compilers emit Wasm byte-identical to the native (32-bit) and interpreter-run
(64-bit) compilers'. Same machine, one CPU per engine, programs built by `wtime-aot build` (`0c142ba`, Goose
compiler from goose-to-wasm `45eb0c7`; [tables](results/aot-0c142ba/tables.md)); the VM is
noisy, V8 most of all, and the examples ran on another CPU at the same time.
Run is the guest's `_start`, process the whole process.

| Example | Wasmtime run | V8 run | wtime-aot run | aot / Wasmtime run | cached-Wasmtime process | V8 process | wtime-aot process |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| squares | 0.02 ms | 0.28 ms | 0.00 ms | **0.11×** | 4.4 ms | 45.5 ms | **3.7 ms** |
| tour | 0.08 ms | 3.5 ms | 0.06 ms | **0.64×** | 4.9 ms | 56.2 ms | **3.9 ms** |
| functions | 0.07 ms | 2.9 ms | 0.04 ms | **0.65×** | 4.6 ms | 48.3 ms | **3.7 ms** |
| errors | 0.04 ms | 1.3 ms | 0.02 ms | **0.54×** | 4.3 ms | 42.7 ms | **3.5 ms** |
| sieve | 0.73 ms | 7.4 ms | 0.59 ms | **0.81×** | 4.9 ms | 52.4 ms | **4.3 ms** |
| sorting | 196 ms | 638 ms | 133 ms | **0.69×** | 215 ms | 686 ms | **137 ms** |
| life | 22.9 ms | 26.8 ms | 5.2 ms | **0.23×** | 28.0 ms | 75.2 ms | **9.3 ms** |
| maze | 0.07 ms | 1.9 ms | 0.05 ms | **0.63×** | 4.5 ms | 47.1 ms | **3.7 ms** |
| sudoku | 9.7 ms | 15.2 ms | 8.0 ms | **0.81×** | 14.2 ms | 62.4 ms | **11.6 ms** |
| vm | 0.06 ms | 1.9 ms | 0.04 ms | **0.64×** | 4.6 ms | 50.9 ms | **4.0 ms** |

| Case | Wasmtime run | V8 run | wtime-aot run | aot / Wasmtime run | cached-Wasmtime process | V8 process | wtime-aot process |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| compile-squares | 0.90 ms | 71.4 ms | 0.95 ms | **0.99×** | 12.3 ms | 196 ms | **5.0 ms** |
| compile-vm | 5.1 ms | 207 ms | 4.7 ms | **0.95×** | 16.7 ms | 355 ms | **9.4 ms** |
| compile-sudoku | 4.0 ms | 199 ms | 4.0 ms | **0.96×** | 15.7 ms | 344 ms | **8.6 ms** |
| compile-functions | 19.7 ms | 334 ms | 20.8 ms | 1.13× | 33.8 ms | 484 ms | **25.3 ms** |
| compile64-vm | 8.3 ms | 917 ms | 10.3 ms | 1.28× | 20.1 ms | 1,050 ms | **15.2 ms** |
| compile64-functions | 32.0 ms | 1,200 ms | 38.2 ms | 1.19× | 45.9 ms | 1,342 ms | **43.4 ms** |
| compile64-8g-vm | 8.2 ms | 329 ms | 8.0 ms | **0.96×** | 21.7 ms | 483 ms | **12.5 ms** |
| compile64-8g-functions | 32.8 ms | 486 ms | 31.2 ms | **0.95×** | 50.0 ms | 631 ms | **36.5 ms** |

Compiled ahead of time, every program's whole process is faster than both cached Wasmtime
and V8, every example runs faster than Wasmtime's code, and the 32-bit Goose compiler runs
level with it or ahead. Its rounds spread widely here (compile-functions: 18.6–22.5 ms
against Wasmtime's 18.4–20.9 ms); measured alone, 11 paired rounds put the four compiler
cases at 0.87–0.92× Wasmtime's run ([pgo.md](results/aot-0c142ba/pgo.md)). The 64-bit
compiler runs ahead too when its memory fits one allocation: `compile64-8g-*` is the same
compiler built with an 8 GiB maximum. Built with its default 16 GiB maximum it grows its
memory to all 16 GiB (touching little of it), which this 15.7 GiB machine will not grant as
one allocation, and safe Rust cannot reserve address space without committing it as
Wasmtime does; wtime-aot then keeps the memory in a base and 4 GiB chunks, and the chunk
path that every access must be able to take costs `compile64-*` 1.2–1.3× Wasmtime's run
(on sorting built for memory64, 48% more instructions than flat). On a machine with more
memory the 16 GiB build is flat as well.

What closed the gap, measured with callgrind on the compiler:

- **Constant memory length** (`5747518`): a 32-bit memory whose minimum and maximum are
  equal, as the release compiler's 65,535 pages are, checks each access against a constant,
  one compare with an immediate; 12–14% fewer instructions.
- **Globals in locals** (`5747518`, bounded in `2f9b4a4`): LLVM cannot tell the memory from
  the instance, which is passed to every callee, so it reloaded globals after every store;
  3% fewer.
- **Upstream** (`45eb0c7`): the compiler's Wasm is 43–46% smaller (32-bit 7.26 → 4.12 MB,
  64-bit 8.94 → 4.79 MB) and builds 1.7× (32-bit, 652 → 383 s, measured on its 16-page
  build) and 3.7× (64-bit, 1,899 → 520 s) faster ahead of time.
- Earlier: memory bytes and length in locals, constant-length copies and fills inline,
  calls check only the native stack, and builds target the host CPU.

Tried and left out, each measured: ThinLTO (no faster: the functions are generic, so all of
them are compiled in the program crate, where cross-unit inlining already applies); one
codegen unit (rustc needed more than 11 GB); a higher LLVM inline threshold (0.4% fewer
instructions); traps by unwinding instead of `Result` (result tests after calls are under
1% of instructions); skipping the stack check in small functions on short acyclic call
chains (half the compiler's functions, no change in instructions: the hot ones do not
qualify); profile-guided builds (`wtime-aot build --train`, kept as an option: 0.97–1.02×
the plain build, [pgo.md](results/aot-0c142ba/pgo.md)); three other shapes of the 64-bit
access (all slower); and, for the chunked memory, a cold-path hint on the slow accesses (no
change), an inline byte-wise slow load (17% more instructions on sorting built for memory64)
and a call-free lookup in the chunk (9% more). Whatever form it takes, a load that can
continue after missing the base costs that much; only one that traps instead is cheap, which
needs every byte in one allocation. The whole process of a small program is 0.16 ms above
the cost of starting `/bin/true` here, so startup has little left to give.

## Fixes

All of these are in `crates/wtime-runtime`, are safe Rust, and add no dependencies.

1. **Lazily allocated linear memory** (`src/linear.rs`). An unshared memory is now a table of
   64 KiB pages, allocated on the first write and read as zeros until then. Before, memory was a
   zero-filled `Vec`. The fixed memory32 compiler declares 4 GiB of initial memory, which cost
   2.3 s and 4.6 GiB of RSS at instantiation. The memory64 compiler grows its memory to about
   17 GB while touching a few hundred MB, so it failed with `linear memory exhausted`.
   A contiguous lazily-zeroed buffer was tried first. It depends on the kernel accepting a single
   14–17 GiB allocation, which on this VM was unreliable, and it has to copy on regrowth. Pages
   need no large allocation at all.
2. **No per-instruction cloning in Decoded dispatch.** Every instruction used to clone the
   module `Arc` and the `Instruction` (with heap-allocated immediates), then drop both.
   Instructions are now borrowed from a module handle that is cached per instance and
   revalidated after instructions that can run host code.
3. **No allocation per memory load.** Loads read into a fixed buffer through `Mem::read_into`
   instead of a `Vec` per access.
4. **Bulk memory without per-chunk buffers.** `memory.fill` built a 64 KiB stack array on
   every call, even for tiny fills. `memory.copy` allocated a `Vec` per 64 KiB chunk. Fill now
   writes the pages directly and skips zero-filling pages that were never written. Copy reuses
   one buffer. Cancellation is still checked every 64 KiB.
5. **Host calls root only references.** Each host call copied the whole operand stack, the
   arguments and every frame's locals into a root set. Collection only marks
   `Value::Ref`, so only those are kept now.
6. **Cheaper calls.** A function's `FuncType` is shared (`Arc`), so a frame's copy of its
   function no longer allocates two vectors per call.
7. **Smaller dispatch costs.** Numeric operators rewrite the stack in place.
   `local.get/set/tee` have their own match arms. Fuel resumption is checked before it is taken.
8. **`Module::function_imports()`** is a new public API listing function imports with their
   type index and signature, for `Store::create_module_host_func`. This host used to decode the
   module a second time just to find its imports.

Verification, run on both the baseline and this change:

- **Unit and integration tests:** `cargo test -p wtime-runtime` passed in five feature
  configurations (243 / 294 / 446 / 246 / 452 tests), and `cargo test` for the workspace passed
  (372).
- **New tests:** 7 for the paged memory, including zero-length accesses at the end of memory.
  The spec suite found that case.
- **Conformance:** the pinned spec corpus executed 62,968 core3 cases, 64,297 phase3 cases and
  15 interaction cases with 0 failures, identical to the baseline. Phase3 also passed on the
  Compact, Register and Reference tiers (64,297 each, 0 failures).
  [Summary](results/after/conformance/summary.json)
- **Lints and checks:** clippy reports no findings that the baseline doesn't already have. The
  production dependency/unsafe budget has 0 violations. `tools/checks` has 2 failing
  release-audit tests, the same 2 as on the baseline.
- **Test expectation change:** p027 now expects unshared `memory.copy`/`memory.fill` to take the
  direct slice path (`bulk_slice_chunks`). Table copies keep their previous expectation.

These changes were made directly. They did not go through the `tools/verify.py` task-contract
flow that the repository uses for its performance plan. The `ReadVec` performance counter still
marks the decoded load path, because existing tests use it to recognize that path, even though
loads no longer allocate.

### Later rounds (2026-11-20)

Each batch went through the gate below and two independent reviews (1–4 rounds each);
their findings are fixed in the commits listed.

- **Straight-line runs** (`execution/straight.rs`, `32f8093`): after the loop head has charged an
  instruction, consecutive locals, globals, constants, numeric operators, loads/stores,
  drop/select and in-frame `block/loop/if/else/end/br/br_if/br_table` run in a tight loop. Each
  instruction is still charged exactly once before it executes, with `next_source`'s
  cancellation, fuel and operand-stack checks; an instruction that would leave the frame is
  handed back already charged. Disabled in `execution-reference` builds.
- **Calls, globals, validation** (`db077fa`): locals are allocated at full capacity at the call
  site and zeroed in runs; numeric `global.set` and memory 0 resolve directly; the validator no
  longer SipHashes its unknown-operand set on every pop.
- **Fast paths proposed in review** (`8f86fdb`): fixed-width loads/stores within one page;
  run state (pc, fuel, cancellation) kept in locals; a validator fast path for the commonest
  instructions; inline one-byte LEB128 decoding.
- **Memory backing hardening** (`0a6c7eb`, `0ee73cb`, `a9ec1cd`, review findings): page
  allocation is fallible and reported as `ResourceLimit` instead of aborting; the last page is
  sized to the memory's length (budget bypass with tiny custom-page-size memories); writes are
  all-or-nothing and zero writes keep unwritten pages unallocated; shared conversion copies only
  written pages within the length; the allocation-failure hardening check covers page writes.
- **Control map** (`d063d22`): 8 bytes per instruction instead of 24 (−53 MiB RSS for the
  compiler module).
- **Host** (`70d4aaa`): `wtime-goose` skips teardown before exit.
- **Lowered straight-line code** (`execution/lowered.rs`; `fa97dc8`, `0af0cfc`, `8bff02b`,
  `cfd7912`, `fa2128c`): once a body has run four times its length in instructions, it is
  lowered to one 16-byte op per instruction with operands resolved, and instructions are
  grouped into the fewest dispatches: pairs (`local.set`+`local.get`, `global.get`+`local.set`,
  `i32.add`+load, …), expressions over locals that keep their operands off the operand stack
  (`local.get`+const+binop+`local.set`, `local.get`+load+`local.set`, two locals into a binop
  or a store, …) and comparisons closed by `if`/`br_if`. Every instruction is still charged
  before it runs, with the operand-stack height it would see, and only a group's first
  instruction can be a branch target. A false `if` without `else` charges its `end` in place
  instead of pushing and popping a label; unmetered runs use a copy of the loop without the
  fuel test. Cold bodies run from their source instructions as before, so short programs do
  not pay for lowering. Differential tests compare each group with the same code separated by
  `nop`s, and the reviewers' fuzzers compared fuel, stack-limit, counter and coverage sweeps
  against the previous commit and the reference executor (several hundred thousand runs per
  round, no differences).

## Reproduce

```sh
# goose-to-wasm (45eb0c7; results before 2026-11-21 used b4f46ec and the
# 2026-11-19/lifetime-callers profile): build the Goose compiler, its Wasmtime host and the modules.
cd goose-to-wasm
python3 scripts/bootstrap.py                       # build/bin/goose
(cd host/rust && cargo build --locked --release)   # goose-wasmtime, Wasmtime 50.0.1
mkdir -p build/cmp
for e in squares tour functions errors sieve sorting life maze sudoku vm; do
  python3 scripts/goose_wasm.py build examples/$e.goose -O2 -o build/cmp/$e.wasm
done
# The compiler itself as Wasm: the project's "fixed memory32" bundled-host build with
# its release layout profile (the default 16-page memory exhausts a region compiling itself) ...
build/bin/goose selfhost/main.goose --stdlib stdlib --target wasm-direct -O2 --wasm-bundled-host \
  --wasm-initial-pages 65535 --wasm-max-pages 65535 \
  --wasm-region-profile bench/results/2026-11-21/shared-emitters/training.profile \
  -o build/cmp/compiler32.wasm
# ... and its default memory64 build.
build/bin/goose selfhost/main.goose --stdlib stdlib --target wasm-direct -O2 \
  --wasm-memory64 --wasm-max-pages 262144 -o build/cmp/compiler64.wasm

# wtime
cd ../wtime/tools/wtime-goose
cargo build --release          # Decoded tier, as shipped; add --features experimental-executors for others
python3 bench.py --cases cases/examples.json --wtime target/release/wtime-goose \
  --wtime-before path/to/baseline/wtime-goose \
  --wasmtime ../../../goose-to-wasm/host/rust/target/release/goose-wasmtime \
  --rounds 3 --cpu 1 -o results/after/examples.json
python3 bench.py --cases cases/compiler.json ... --wasmtime-cache DIR --cpu 2 -o results/after/compiler.json
python3 report.py results/after/*.json
```

`cases/*.json` use absolute paths from the measurement machine; adjust `module`, `expected`
and `cwd` for another checkout. Run a module directly:

```sh
wtime-goose [--tier decoded|compact|register|register-adaptive=N] [--timing-json path] module.wasm [-- program arguments]
```

Build with `--features diagnostics` and set `WTIME_GOOSE_STATS=1` to print the Compact and
Register tiers' dispatch and fallback counters.

### Method and caveats

- **Machine:** a 4-vCPU cloud VM (Intel Xeon @ 2.10 GHz), Linux 6.18, Rust 1.99.0, Clang 18.
  Each sample is a fresh process pinned with `taskset`, and engine order alternates by round.
- **Concurrent runs:** examples ran on CPU 1, compiler cases on CPU 2 and the self-checks on
  CPU 3, all at the same time. Single-process timings on this VM vary by about ±20%, so ratios
  use per-round pairs. Changes during development were judged by callgrind instruction counts.
- **Rounds:** three per cell. Each self-check is a single sample: wtime takes several minutes;
  Wasmtime's ≈2.5 s is from the earlier run.
- **Timing boundaries:** `prep` (`compile_instantiate_ns`) covers module preparation, import
  creation and instantiation. For Wasmtime that is an uncached Cranelift compile on one CPU,
  unless the engine is labeled `wasmtime-cached`. `run` (`start_function_ns`) covers `_start`.
- **RSS:** peak RSS comes from `wait4`. Values below about 34 MiB are a floor inherited from the
  Python driver at fork time.
