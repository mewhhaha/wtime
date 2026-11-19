# wtime-goose: Goose-compiled Wasm on wtime vs Wasmtime

`wtime-goose` runs direct WebAssembly modules produced by
[goose-to-wasm](https://github.com/mewhhaha/goose-to-wasm) on the wtime interpreter. It is a
port of goose-to-wasm's Wasmtime host (`host/rust`, `goose-wasmtime`), with the same
imports (`goose.*`, `env.gs_os_*`, `env.bench_*`, the WASI subset), program arguments,
stdin, output and exit codes, and the same timing JSON boundaries. That makes it possible
to run identical modules on both engines and compare them.

It is an isolated crate (own `[workspace]`), like `tools/wtime-bench`, and changes nothing
in the production runtime. `float.rs` and `pytext.rs` are copied from goose-to-wasm.

## Measured result (2026-11-19)

wtime runs Goose programs correctly. Every run on every tier produced byte-identical output to
Wasmtime and to the expected files. This includes the Goose compiler itself compiled to Wasm,
and the Wasm it emits from wtime matches the native compiler's output byte for byte. On guest
execution, though, wtime is **2–2,100× slower than Wasmtime**: a geometric mean of about
**220× on the ten example programs** and about **280× when the compiler compiles examples**.
wtime prepares modules **about 7–17× faster** than an uncached Cranelift compile, so
small, short programs finish sooner as whole processes on wtime. A warm Wasmtime cache removes
that advantage.

| Workload | Wasmtime 50.0.1 | wtime decoded | wtime / Wasmtime |
| --- | ---: | ---: | ---: |
| Compiler type-checks its own sources (`--check`), guest time | 2.35–2.80 s (3 runs) | 1,041 s (1 run) | ≈410× |
| Same, native compiler process | 1.77–2.08 s | | |
| Compiler emits `examples/functions.goose`, guest time | 28.2 ms | 12.7 s | 449× |
| `sorting` example, guest time | 145 ms | 302 s | 2,091× |
| `vm` example, whole process (uncached Cranelift) | 46 ms | 23 ms | 0.5× |
| Compiler module (7.3 MB) preparation: Cranelift on 1 CPU vs wtime decode+validate+lower | 6.5–7.1 s | 0.37–0.47 s | ≈0.06× |
| Compiler process emitting `vm.goose`, Wasmtime from warm cache | 21.7 ms | 6,265 ms | 289× |
| Peak RSS, compiler compiling `squares.goose` | 137 MiB | 4,645 MiB | 34× |

The experimental Compact and Register tiers (eager, and adaptive with a threshold of 3) were within
about ±20% of Decoded on these workloads. No tier changed the picture. Full tables are in
[results/tables.md](results/tables.md). Raw samples are in `results/*.json`.

### Observations

- **Interpreter dispatch dominates.** Stack samples while running `sorting` were all inside
  `Store::invoke_decoded`. On every instruction the loop clones the `Module` `Arc` (an atomic increment and
  decrement) and clones the `Instruction`, whose immediates are freed again afterwards
  (`crates/wtime-runtime/src/execution.rs`, around the `ops.get(frame.pc)...clone()` fetch).
  The host imports do not show up in the samples.
- **Linear memory is committed eagerly.** wtime backs memory with a zero-filled `Vec`. The
  fixed memory32 compiler declares 65,535 initial pages, so instantiation zeroes 4 GiB:
  about 2.3 s and 4.6 GiB RSS. Wasmtime reserves virtual memory and pays only for pages it
  touches.
- **The default memory64 compiler does not run on wtime.** goose-to-wasm's default build
  (`--wasm-memory64 --wasm-max-pages 262144`) grows linear memory past 12 GiB on even a tiny
  compile and relies on lazy commit. With eager zero-filled memory, wtime reached its
  configured 12 GiB budget and the guest reported `linear memory exhausted`. The comparison
  therefore uses goose-to-wasm's fixed memory32 compiler build. Wasmtime runs both builds.
- These ratios are larger than wtime's own published microbenchmarks
  ([`docs/benchmarks/wasmtime-current`](../../docs/benchmarks/wasmtime-current/README.md): 277–730×). Real compiler-generated code with calls,
  memory and branches is a harsher mix than those loops. `sorting` and `sieve` are the
  worst cases.

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
# The compiler itself as Wasm: the project's "fixed memory32" bundled-host build.
build/bin/goose selfhost/main.goose --stdlib stdlib --target wasm-direct -O2 --wasm-bundled-host \
  --wasm-initial-pages 65535 --wasm-max-pages 65535 \
  --wasm-region-profile bench/results/2026-11-19/lifetime-callers/training.profile \
  -o build/cmp/compiler32.wasm

# wtime (2e77f35 + this tool)
cd ../wtime/tools/wtime-goose
cargo build --release --features experimental-executors
python3 bench.py --cases cases/examples.json --wtime target/release/wtime-goose \
  --wasmtime ../../../goose-to-wasm/host/rust/target/release/goose-wasmtime \
  --tiers decoded,compact,register-adaptive=3,register --rounds 3 --cpu 1 -o results/examples.json
python3 bench.py --cases cases/compiler.json ... --cpu 2 -o results/compiler.json
python3 report.py results/*.json
```

`cases/*.json` use absolute paths from the measurement machine; adjust `module`, `expected`
and `cwd` for another checkout.

Run a module directly:

```sh
wtime-goose [--tier decoded|compact|register|register-adaptive=N] [--timing-json path] module.wasm [-- program arguments]
```

### Method and caveats

- Machine: 4-vCPU cloud VM (Intel Xeon @ 2.10 GHz), Linux 6.18, Rust 1.99.0, Clang 18.
  Each sample is a fresh process pinned with `taskset`. Engines alternate order by round.
  Examples ran on CPU 1 and compiler cases on CPU 2, concurrently. The single wtime
  self-check ran on CPU 3 at the same time. Expect some shared-host noise; ratios use
  per-round pairs.
- `prep` (`compile_instantiate_ns`) covers module preparation, import creation and
  instantiation. For Wasmtime that means an uncached Cranelift compile (`--no-cache`), unless the
  engine is labeled `wasmtime-cached`. `run` (`start_function_ns`) covers `_start` only.
- Three rounds per cell (five for the cached comparison). The wtime self-check is one sample
  because it takes 17 minutes.
- After these runs, the host gained a `compile_ns` field that splits wtime preparation from
  instantiation. The compile/instantiate split above was measured with that build. Nothing else
  changed.
