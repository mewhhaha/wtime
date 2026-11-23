# wtime

A WebAssembly interpreter written in Rust. The project targets core WebAssembly
and the proposals at Phase 3 or above in its [pinned scope](docs/architecture/scope.md).
Browser and JavaScript embedding APIs are excluded.

Implementation and conformance work is in progress. Source code for a feature
is not a claim of full conformance. The task graph and acceptance evidence live
in the sibling `../tasks` directory, with a tracked backup in
[`task-plans`](task-plans/); official test runs produce JSON reports
with separate failed and unsupported counts.

The selected portable performance portfolio is complete. See the
[measured results](docs/benchmarks/performance/README.md) and
[execution policy](docs/architecture/performance-policy.md). Native AArch64
release verification remains outstanding; conditional research branches are
explicitly deferred.

Modules can also be compiled ahead of time with [`tools/wtime-aot`](tools/wtime-aot/README.md),
which supports three backends: safe Rust crates (`--backend rust`), sharded C99/Clang
(`--backend c`, `--blot-cdylib`), and a self-contained pure-Rust Linux x86-64 native
compiler (`--backend x64`, `--blot-cdylib`, zero external compilers or `libc`).

### Benchmarks

Each category is shown separately below (details in
[`tools/wtime-goose`](tools/wtime-goose/README.md#ahead-of-time-wtime-aot) and
[`tools/wtime-aot`](tools/wtime-aot/README.md#backends-and-benchmarks); reproduce the
`wtime-aot` charts with `python3 -B tools/wtime-aot/bench.py` and `tools/wtime-aot/chart.py`):

#### Standalone programs & Goose compiler — runtime (`--backend rust`)

![wtime-aot's speed-up over Wasmtime and V8 running each program](tools/wtime-goose/charts/run.svg)

#### Standalone programs & Goose compiler — whole process (`--backend rust`)

![wtime-aot's speed-up over cached Wasmtime and V8 for the whole process](tools/wtime-goose/charts/process.svg)

#### Blot game runtime — `gdev` (`3.57 MiB`, 4,935 functions)

![Blot game runtime on gdev comparing wtime-aot x64, wtime-aot c -O2, and Wasmtime](tools/wtime-aot/charts/runtime-gdev.svg)

#### Representative Wasm kernels — runtime

![Representative Wasm kernels runtime comparing wtime-aot x64, c -O2, rust -O3, and V8](tools/wtime-aot/charts/runtime-kernels.svg)

#### Cold compilation time (empty cache, multi-core)

![Cold compilation time comparing wtime-aot x64, c -O2, rust -O3, and Wasmtime](tools/wtime-aot/charts/compile-cold.svg)

#### Hot compilation, incremental rebuild, & warm cache-hit load

![Hot compilation, incremental 1-function rebuild, and warm load time comparing wtime-aot backends and Wasmtime](tools/wtime-aot/charts/compile-hot.svg)

Summary on real 4,935-function `gdev` (`3.57 MiB` Wasm; AMD Ryzen 7 7800X3D 8C/16T, Linux 7.3.5,
Clang 23.1.5, Rust 1.99.0; exact 120-frame / 242-file SHA-256 parity across all three engines):

| Metric (`gdev`) | `wtime-aot x64` | `wtime-aot c (-O2)` | Wasmtime |
| --- | ---: | ---: | ---: |
| Frame median / p95 (CPU 0) | **0.061 ms / 0.066 ms** | 0.056 ms / 0.060 ms | 0.062 ms / 0.068 ms |
| Snapshot median (CPU 0) | **0.153 ms** | 0.142 ms | 0.154 ms |
| Cold compile (16T, empty cache) | **221 ms** | 7.72 s | 916 ms |
| Hot 1-fn edit rebuild (16T) | **205 ms** | 848 ms *(warm `c-obj/`)* | 802 ms |
| Warm fresh-process load (`pen`) | **7.63 ms** | 9.58 ms | 16.0 ms *(`.cwasm`)* |
| Warm `build --cache` CLI hit | **46.4 ms** | 335 ms | — |
| Compiled artifact size | **6.47 MiB** (`.so`) | 8.51 MiB (`.so`) | 13.94 MiB (`.cwasm`) |

After a fresh clone, restore the plans if the sibling directory is absent:

```sh
if [ ! -e ../tasks ]; then
  cp -a task-plans ../tasks
fi
python3 -B ../tasks/check_plan.py
python3 -B ../tasks/performance/check_plan.py
```

Keep an existing sibling directory: it may contain newer work. The backup
retains the plans' original relative links and is intended to be restored to
`../tasks`. See the [recovery record](docs/recovery/README.md).

## Build and test

Rust 1.99.0 is pinned. In the prepared cloud environment first run
`source /workspace/.setup/rust-env.sh`.

```sh
cargo test --locked
cargo test --locked -p wtime-conformance
cargo clippy --locked --all-targets -- -D warnings
python3 -m unittest discover -s tools/checks/tests -v
python3 tools/checks/dependency_budget.py
```

The four production libraries and CLI use no external crates. `wast` is pinned
only in the separate conformance tool, which is excluded from default builds.
The implementation uses safe Rust, explicit guest frames, software SIMD, and a
nonmoving garbage collector. There is no JIT or native guest stack execution.

## Run a binary module

```sh
cargo run --locked -p wtime-cli -- check module.wasm
cargo run --locked -p wtime-cli -- exports module.wasm
cargo run --locked -p wtime-cli -- --fuel 100000 run module.wasm add i32:20 i32:22
```

The CLI takes binary Wasm. Generate one with the separate tool:

```sh
cargo run --locked -p wtime-conformance -- encode module.wat -o module.wasm
```

`--profile` accepts `mvp`, `stable-core`, and `all-phase3-core`. Disabled features,
invalid modules, guest traps, guest exceptions, and host/resource failures are
separate outcomes. Fuel and other resource budgets belong to engine policy.

## Architecture

`wtime-types` defines portable types and errors; `wtime-decode` reads bounded
binary structures; `wtime-validate` seals checked modules; `wtime-runtime`
instantiates and interprets them. `wtime-cli` is a thin embedding example.

Run the [native examples](examples/README.md) or read the [embedding guide](docs/embedding.md).
Read the [embedding contract](docs/architecture/api.md),
[semantic invariants](docs/architecture/hard_task_contracts.md), and
[dependency policy](docs/architecture/dependencies.md) before extending the engine.
