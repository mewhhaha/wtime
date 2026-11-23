# wtime-goose: Goose-compiled Wasm on wtime vs Wasmtime

`wtime-goose` runs direct WebAssembly modules produced by
[goose-to-wasm](https://github.com/mewhhaha/goose-to-wasm) on the wtime interpreter. It is a
port of goose-to-wasm's Wasmtime host (`host/rust`, `goose-wasmtime`), with the same
imports (`goose.*`, `env.gs_os_*`, `env.bench_*`, the WASI subset), program arguments,
stdin, output and exit codes, and the same timing JSON boundaries. That makes it possible
to run identical modules on both engines and compare them.

It is an isolated crate (own `[workspace]`) outside the core workspace. `float.rs` and
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
compilers emit Wasm byte-identical to the native (32-bit) and Wasmtime-run
(64-bit) compilers'. Measured sequentially on one CPU per engine (`--cpu 1`, 5 paired
rounds, AMD Ryzen 7 3800X, Linux 7.3.4, Rust 1.97.1, Wasmtime 50.0.1, Deno 2.9.8 / V8 15.0)
with `wtime-aot build` at `037d32c` and Goose compiler from goose-to-wasm `766898a`
([tables](results/aot-037d32c/tables.md); earlier `0c142ba` baseline in
[results/aot-0c142ba](results/aot-0c142ba/tables.md)). `bench.py --no-thp` applies
`prctl(PR_SET_THP_DISABLE, 1)` equally to every engine so the host kernel's
`transparent_hugepage=always` default behaves like standard `madvise` Linux instead of
faulting 2 MiB huge pages on every sparse anonymous region write (without `--no-thp`,
short runs that touch sparse 4–16 GiB memories pay ~24 ms of kernel huge-page zeroing,
including Wasmtime on `compile-squares`).
Run is the guest's `_start`, process the whole process.

| Example | Wasmtime run | V8 run | wtime-aot run | aot / Wasmtime run | cached-Wasmtime process | V8 process | wtime-aot process |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| squares | 0.02 ms | 0.24 ms | 0.00 ms | **0.08×** | 4.6 ms | 53.7 ms | **3.7 ms** |
| tour | 0.07 ms | 2.8 ms | 0.04 ms | **0.56×** | 4.7 ms | 56.9 ms | **4.6 ms** |
| functions | 0.06 ms | 2.6 ms | 0.03 ms | **0.50×** | 5.1 ms | 59.7 ms | **4.0 ms** |
| errors | 0.04 ms | 1.3 ms | 0.02 ms | **0.34×** | 4.9 ms | 54.7 ms | **4.4 ms** |
| sieve | 0.71 ms | 4.7 ms | 0.48 ms | **0.70×** | 5.2 ms | 61.4 ms | **5.0 ms** |
| sorting | 172.5 ms | 278.0 ms | 100.9 ms | **0.59×** | 179.9 ms | 339.4 ms | **106.0 ms** |
| life | 24.4 ms | 27.2 ms | 5.8 ms | **0.24×** | 28.5 ms | 77.9 ms | **10.1 ms** |
| maze | 0.07 ms | 1.5 ms | 0.03 ms | **0.48×** | 4.9 ms | 52.1 ms | **4.3 ms** |
| sudoku | 10.0 ms | 16.4 ms | 4.4 ms | **0.45×** | 15.5 ms | 69.1 ms | **8.7 ms** |
| vm | 0.06 ms | 1.4 ms | 0.02 ms | **0.30×** | 4.7 ms | 50.8 ms | **4.2 ms** |

| Case | Wasmtime run | V8 run | wtime-aot run | aot / Wasmtime run | cached-Wasmtime process | V8 process | wtime-aot process |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| compile-squares | 0.69 ms | 59.2 ms | 0.69 ms | 1.04× | 10.9 ms | 186.0 ms | **5.6 ms** |
| compile-vm | 5.1 ms | 163.5 ms | 3.5 ms | **0.70×** | 15.7 ms | 320.0 ms | **8.0 ms** |
| compile-sudoku | 4.0 ms | 159.5 ms | 2.7 ms | **0.68×** | 14.6 ms | 318.4 ms | **7.7 ms** |
| compile-functions | 22.5 ms | 273.7 ms | 13.7 ms | **0.62×** | 32.9 ms | 423.0 ms | **19.3 ms** |
| compile64-vm | 7.3 ms | 796.6 ms | 5.4 ms | **0.73×** | 18.4 ms | 951.7 ms | **10.2 ms** |
| compile64-functions | 32.0 ms | 1,062.0 ms | 22.6 ms | **0.70×** | 42.8 ms | 1,207.3 ms | **27.7 ms** |
| compile64-8g-vm | 7.3 ms | 283.5 ms | 5.5 ms | **0.74×** | 17.7 ms | 436.8 ms | **10.0 ms** |
| compile64-8g-functions | 33.1 ms | 426.5 ms | 22.7 ms | **0.69×** | 44.3 ms | 587.6 ms | **28.0 ms** |

Compiled ahead of time, every program's whole process is faster than both cached Wasmtime
(1.01–2.94×) and V8 (3.2–94×), every example runs faster than Wasmtime (1.43–13×) and V8
(2.76–127×), and every non-trivial Goose compiler workload beats Wasmtime by 1.34–1.62× and
V8 by 18–148× across both 32-bit and 64-bit memories (`compile-squares` ties uncached
Wasmtime's 0.69 ms run while beating cached Wasmtime's 0.83 ms run and 10.9 ms process at
5.6 ms). Compiling the Goose compiler's own sources (`selfhost/main.goose`, 3,075,247 bytes
emitted, byte-identical to Wasmtime), `wtime-aot` finishes in 1.62 s (32-bit) and 2.45 s
(64-bit), 1.43× faster than Wasmtime (2.32 s / 3.50 s) and faster than the native
`build/bin/goose` (2.59 s). On machines with enough virtual/commit capacity (such as this
host), the default 16 GiB memory64 compiler uses a flat allocation (`CAP64`); on smaller
machines where a 16 GiB single allocation is refused, `wtime-aot` falls back transparently
to a base plus 4 GiB chunks with a non-growing fast-region check (`if mf`) when memory stays
within the base.

What closed the gap and pulled ahead:

- **Disjoint `noalias` memory and `#[repr(C)]` `Ctx`** (`037d32c`): passing linear memory
  bytes (`&mut [u8; CAP32]` / `&mut [u8; CAP64]` / `&mut [u8]`) separately from the
  instance context lets LLVM prove guest memory stores never alias globals, tables, or
  runtime state, eliminating redundant reloads across calls and stores.
- **Whole-module global analyses** (`037d32c`): constant-initialized globals (including
  immutable region bases and tables) are folded to immediates; a greatest-fixed-point
  preservation analysis passes up to 4 callee-preserved globals (such as the shadow stack
  pointer) in registers and elides redundant writes across calls.
- **Hybrid zero-panic memory32 and fixed-capacity `CAP64` memory64** (`037d32c`): constant-
  length 32-bit memories use fixed-size array refs with zero-panic bounds checks; flat
  64-bit memories use a fixed-capacity buffer with dominating `ml` length guards refreshed
  only after calls that may grow memory (`fn_may_grow`).
- **Tarjan SCC acyclic stack-check elision & targeted leaf inlining** (`037d32c`): functions
  on acyclic subtrees with bounded frame depth skip per-call native stack checks, and small
  non-looping leaves inline across callers without bloating loop bodies.
- **Zero-copy host scanning & parallel balanced builds** (`037d32c`): `gs_find_any` and
  `gs_find_pair` scan guest memory slices directly; LPT-balanced parallel part crates with
  thin LTO build `compiler32` in 41.4 s and `compiler64` in 58.9 s on this 8-core host.
- **Earlier wins**: constant memory length (`5747518`, 12–14% fewer instructions), globals in
  locals (`5747518`/`2f9b4a4`), upstream smaller compiler Wasm (`45eb0c7`), inline
  constant-length copies/fills, native-stack-only checks, and `-C target-cpu=native`.

Tried and left out earlier, each measured: one codegen unit (rustc needed more than 11 GB);
a higher LLVM inline threshold (0.4% fewer instructions); traps by unwinding instead of
`Result` (result tests after calls are under 1% of instructions); profile-guided builds
(`wtime-aot build --train`, kept as an option: 0.97–1.02× the plain build,
[pgo.md](results/aot-0c142ba/pgo.md)); and for chunked memory64, inline byte-wise slow loads
and call-free chunk lookups (both slower than cold register-returned slow paths). The whole
process of a small program sits within ~0.2 ms of `/bin/true`, leaving startup near its floor.

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

These changes were made directly against the Goose benchmark suite rather than through automated microbenchmark gating;
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
