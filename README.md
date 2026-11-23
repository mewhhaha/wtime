# wtime

A safe-Rust, zero-dependency WebAssembly core interpreter (`wtime-types`,
`wtime-decode`, `wtime-validate`, `wtime-runtime`, `wtime-cli`) plus a
standalone opt-in ahead-of-time compiler (`tools/wtime-aot`).

## Scope and Release Status (`v0.1.0`)

- **Core interpreter (`crates/*`)**: targets WebAssembly Core 3.0 and the active
  Phase 3–5 proposals listed in [docs/architecture/scope.md](docs/architecture/scope.md)
  (threads/atomics, GC, exception handling, stack-switching continuations,
  memory64, multi-memory, fixed-width and relaxed SIMD, wide arithmetic, custom
  page sizes, compact import sections, and specified custom descriptors).
  Browser and JavaScript embedding APIs are excluded.
- **Core safety invariants**: all four core libraries and `wtime-cli` use
  `#![forbid(unsafe_code)]`, zero third-party production dependencies, explicit
  heap guest frames, software SIMD, a non-moving mark/sweep GC, source-level
  fuel metering, and cooperative cancellation. The core interpreter performs no
  JIT compilation and no native guest stack execution.
- **Supported `v0.1.0` target**: `x86_64-unknown-linux-gnu`. Native AArch64
  release verification is planned for a follow-up release. See
  [docs/release-readiness.md](docs/release-readiness.md) and
  [docs/architecture/release-manifest.json](docs/architecture/release-manifest.json)
  for reproducible verification gates.
- **Ahead-of-time compiler (`tools/wtime-aot`, standalone tool)**: excluded from
  the core workspace dependency graph, offering three opt-in AOT backends:
  - `--backend rust`: safe-Rust crate generation (`#![forbid(unsafe_code)]` for
    all generated Wasm code and `wtime-aot-rt`; thin C-ABI FFI wrapper for
    `--blot-cdylib`).
  - `--backend c`: sharded C99/Clang + `ld.lld` Blot ABI 8 `.so` compiler with
    content-addressed per-shard caching and `-fcf-protection=return` CET
    shadow-stack hardening.
  - `--backend x64`: self-contained pure-Rust (`#![forbid(unsafe_code)]`) Linux
    x86-64 SysV LP64 native Blot ABI 8 `.so` compiler with zero external
    compilers or `libc` dependencies (contract in
    [docs/wtime-aot/x64-abi.md](docs/wtime-aot/x64-abi.md)).

## Benchmarks (`tools/wtime-goose` & `tools/wtime-aot`)

Each category is shown separately below (methodology in
[tools/wtime-goose/README.md](tools/wtime-goose/README.md#ahead-of-time-wtime-aot)
and [tools/wtime-aot/README.md](tools/wtime-aot/README.md#backends-and-benchmarks)).
Anyone can reproduce the self-contained kernel benchmarks locally with:

```sh
cargo build --locked --release --manifest-path tools/wtime-aot/Cargo.toml
python3 -B tools/wtime-aot/bench.py --kernels-only -o target/aot-kernels.json
python3 -B tools/wtime-aot/chart.py target/aot-kernels.json -o target/aot-charts
```

*(The `gdev` charts below additionally benchmark an external Blot game module
passed via `--pen`, `--gdev-wasm`, and `--gdev-pkg`.)*

### Standalone programs & Goose compiler — runtime (`--backend rust`)

![wtime-aot's speed-up over Wasmtime and V8 running each program](tools/wtime-goose/charts/run.svg)

### Standalone programs & Goose compiler — whole process (`--backend rust`)

![wtime-aot's speed-up over cached Wasmtime and V8 for the whole process](tools/wtime-goose/charts/process.svg)

### Blot game runtime — `gdev` (`3.57 MiB`)

![Blot game runtime on gdev comparing wtime-aot x64, wtime-aot c -O2, and Wasmtime](tools/wtime-aot/charts/runtime-gdev.svg)

### Representative Wasm kernels — runtime

![Representative Wasm kernels runtime comparing wtime-aot x64, c -O2, rust -O3, and V8](tools/wtime-aot/charts/runtime-kernels.svg)

### Cold compilation time (empty cache, multi-core)

![Cold compilation time comparing wtime-aot x64, c -O2, rust -O3, and Wasmtime](tools/wtime-aot/charts/compile-cold.svg)

### Hot compilation, incremental rebuild, & warm cache-hit load

![Hot compilation, incremental 1-function rebuild, and warm load time comparing wtime-aot backends and Wasmtime](tools/wtime-aot/charts/compile-hot.svg)

Summary on `gdev` (`3.57 MiB` Wasm; AMD Ryzen 7 7800X3D 8C/16T, Linux 7.3.5,
Clang 23.1.5, Rust 1.99.0; exact 120-frame / 242-file SHA-256 parity across all
three engines; `--backend x64` with full IR & register-allocation verification
enabled by default):

| Metric (`gdev`) | `wtime-aot x64` | `wtime-aot c (-O2)` | Wasmtime |
| --- | ---: | ---: | ---: |
| Frame median / p95 (CPU 0) | **0.091 ms / 0.124 ms** | **0.060 ms / 0.116 ms** | 0.090 ms / 0.109 ms |
| Snapshot median (CPU 0) | **0.168 ms** | **0.155 ms** | 0.164 ms |
| Cold compile (16T, empty cache) | **237 ms** | 7.03 s | 790 ms |
| Hot 1-fn edit rebuild (16T) | **232 ms** | 846 ms *(warm `c-obj/`)* | 799 ms |
| Warm fresh-process load (`pen packets`) | **6.0 ms** | 9.0 ms | 17.0 ms *(`.cwasm`)* |
| Warm `build --cache` CLI hit | **47.6 ms** | 314 ms | — |
| Compiled artifact size | **6.59 MiB** (`.so`) | 8.67 MiB (`.so`) | 14.22 MiB (`.cwasm`) |

## Build and test

Rust `1.99.0` is pinned via `rust-toolchain.toml`.

```sh
cargo test --locked --workspace --all-features
cargo test --locked --doc --workspace
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --locked --no-deps --workspace --all-features
python3 -B tools/checks/dependency_budget.py
python3 -B -m unittest discover -s tools/checks/tests -v
python3 -B tools/checks/opcode_coverage.py --require-complete
python3 -B tools/checks/release_audit.py --portable
```

*(On a checkout that also built the pinned release-evidence binaries into
`target/evidence/` per [docs/release-readiness.md](docs/release-readiness.md),
run strict `python3 -B tools/checks/release_audit.py` without `--portable`.)*

To test the standalone AOT compiler (requires `clang` and `lld` on Linux x86-64):

```sh
cargo test --locked --manifest-path tools/wtime-aot/rt/Cargo.toml
WTIME_AOT_REQUIRE_NATIVE_TESTS=1 cargo test --locked --manifest-path tools/wtime-aot/Cargo.toml
```

## Run a binary module

```sh
cargo run --locked -p wtime-cli -- --version
cargo run --locked -p wtime-cli -- check module.wasm
cargo run --locked -p wtime-cli -- exports module.wasm
cargo run --locked -p wtime-cli -- --fuel 100000 run module.wasm add i32:20 i32:22
```

Generate a binary `.wasm` from `.wat` with the separate conformance tool:

```sh
cargo run --locked -p wtime-conformance -- encode module.wat -o module.wasm
```

`--profile` accepts `mvp`, `stable-core`, and `all-phase3-core`. CLI exit codes
distinguish outcomes explicitly (`0` success, `1` usage/I/O/host/link error,
`2` decode/validation/disabled-feature error, `3` guest trap, `4` uncaught guest
exception or suspension, `5` resource limit or cancellation; run
`wtime-cli --help`).

## Architecture & Documentation

- `wtime-types`: portable types, feature profiles, resource limits, and errors.
- `wtime-decode`: bounded binary decoder.
- `wtime-validate`: validator producing sealed `ValidatedModule` values.
- `wtime-runtime`: interpreter engine, canonical types, store, linker, GC, and
  shared memory.
- `wtime-cli`: thin CLI for `check`, `exports`, `hints`, and `run`.

Further reading:
- [Embedding guide](docs/embedding.md) and [native examples](examples/README.md)
- [Embedding & ownership contract](docs/architecture/api.md)
- [Semantic invariants](docs/architecture/hard_task_contracts.md) and
  [dependency policy](docs/architecture/dependencies.md)
- [Release readiness](docs/release-readiness.md), [Changelog](CHANGELOG.md), and
  [Security policy](SECURITY.md)
