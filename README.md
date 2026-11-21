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

Modules can also be compiled ahead of time to safe Rust with
[`tools/wtime-aot`](tools/wtime-aot/README.md). Against Wasmtime and V8 on Goose
programs and the Goose compiler ([details](tools/wtime-goose/README.md#ahead-of-time-wtime-aot)):

![wtime-aot's speed-up over Wasmtime and V8 running each program](tools/wtime-goose/charts/run.svg)

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
