# wtime-goose: Goose-compiled Wasm on wtime vs Wasmtime

`wtime-goose` runs direct WebAssembly modules produced by
[goose-to-wasm](https://github.com/mewhhaha/goose-to-wasm) on the wtime interpreter. It is a
port of goose-to-wasm's Wasmtime host (`host/rust`, `goose-wasmtime`), with the same
imports (`goose.*`, `env.gs_os_*`, `env.bench_*`, the WASI subset), program arguments,
stdin, output and exit codes, and the same timing JSON boundaries. That makes it possible
to run identical modules on both engines and compare them.

It is an isolated crate (own `[workspace]`), like `tools/wtime-bench`. `float.rs` and
`pytext.rs` are copied from goose-to-wasm.

## Results (2026-11-19)

The first comparison found several pathologies in wtime. They are fixed in the runtime now
(see [Fixes](#fixes)). Every run on both builds produced output byte-identical to Wasmtime and
to the expected files. When the Goose compiler itself runs inside wtime, the Wasm it emits
matches the native compiler's output byte for byte.

Guest execution time (`_start`). Ratios are medians of per-round wtime/Wasmtime pairs;
averages are geometric means. "Before" is wtime `2e77f35`; "after" is this change. Both are
default builds of the Decoded tier.

| Workload | Wasmtime 50.0.1 | wtime before | wtime after | Speedup | After / Wasmtime |
| --- | ---: | ---: | ---: | ---: | ---: |
| Ten example programs (average) | | | | **5.1×** (2.2–14×) | 43× (was 226×) |
| `sorting` example | 152 ms | 281 s | 97 s | 2.9× | 639× (was 1,871×) |
| `sudoku` example | 9.2 ms | 8.2 s | 2.6 s | 3.1× | 284× (was 889×) |
| `tour` example | 0.1 ms | 10.1 ms | 1.1 ms | 9.2× | 11× (was 101×) |
| Compiler (memory32) compiling 4 examples (average) | | | | **2.6×** | 114× (was 318×) |
| Compiler compiling `functions.goose` | 24.4 ms | 12.3 s | 4.9 s | 2.5× | 200× (was 476×) |
| Compiler type-checks its own sources (`--check`) | ≈2.5 s | 1,041 s | 379 s | 2.7× | ≈150× (was ≈410×) |
| Compiler, default **memory64** build, compiling `functions.goose` | 36.8 ms | could not run | 5.4 s | — | 142× |
| Same memory64 compiler, `--check` on its own sources | | could not run | 442 s | — | |

Preparation and memory for the 7.3 MB compiler module:

| | Wasmtime (uncached / cached) | wtime before | wtime after |
| --- | ---: | ---: | ---: |
| Preparation + instantiation | 6.1–6.8 s / 7 ms | 3.0–3.3 s | 0.37–0.44 s |
| Whole process compiling `vm.goose` | 6.1 s / 26 ms | 5.8 s | 1.6 s |
| Peak RSS (memory32 / memory64 compiler) | 137 / 242 MiB uncached | 4,645 MiB / — | 330 / 395 MiB |

Full tables: [after](results/after/tables.md) and [before](results/before/tables.md). Raw
samples, binary hashes and RSS are in `results/*/*.json`.

**What remains.** wtime is still an interpreter. Wasmtime compiles to native code, and wtime
excludes JIT compilation by design. Guest code therefore stays roughly 10–600× slower. The
remaining cost is spread across the Decoded dispatch loop: about 140 host instructions per Wasm
instruction, down from about 280. That is design-level overhead (Result-returning
stack operations, 32-byte `Value`s, label bookkeeping), not a pathology, and lowering it means
a better execution tier. The existing experimental Compact and Register tiers do not help
here. Compact admits nothing in these modules, because its module-wide preparation budget is
100,000 work units. Register falls back to Decoded for almost every frame: on `sieve` it
recorded 9,711 fallback frames against 504 register dispatches.

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

## Reproduce

```sh
# goose-to-wasm (b4f46ec): build the Goose compiler, its Wasmtime host and the modules.
cd goose-to-wasm
python3 scripts/bootstrap.py                       # build/bin/goose
(cd host/rust && cargo build --locked --release)   # goose-wasmtime, Wasmtime 50.0.1
mkdir -p build/cmp
for e in squares tour functions errors sieve sorting life maze sudoku vm; do
  python3 scripts/goose_wasm.py build examples/$e.goose -O2 -o build/cmp/$e.wasm
done
# The compiler itself as Wasm: the project's "fixed memory32" bundled-host build ...
build/bin/goose selfhost/main.goose --stdlib stdlib --target wasm-direct -O2 --wasm-bundled-host \
  --wasm-initial-pages 65535 --wasm-max-pages 65535 \
  --wasm-region-profile bench/results/2026-11-19/lifetime-callers/training.profile \
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
