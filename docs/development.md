# CLI, Crate Layout, & Development

Detailed reference for running `wtime-cli`, understanding the workspace crates, and executing the full verification and release-audit suites.

---

## Workspace Crates

The core workspace (`crates/*`) enforces `#![forbid(unsafe_code)]`, `#![warn(missing_docs)]`, and **zero external production dependencies**:

| Crate | Role |
| --- | --- |
| `wtime-types` | Portable Wasm Core 3.0 + Phase 3–5 types, feature profiles, resource limits, and errors |
| `wtime-decode` | Bounded binary decoder with strict size/depth limits |
| `wtime-validate` | Spec validator producing sealed `ValidatedModule` values |
| `wtime-runtime` | Safe-Rust interpreter engine, canonical types, store, linker, non-moving mark/sweep GC, software SIMD, shared memory, fuel metering, and cooperative cancellation |
| `wtime-cli` | Thin CLI for `check`, `exports`, `hints`, and `run` |

Standalone tools excluded from the core workspace dependency graph:

| Tool | Role |
| --- | --- |
| [`tools/wtime-aot`](../tools/wtime-aot/README.md) | Opt-in ahead-of-time compiler (`--backend x64`, `--backend c`, `--backend rust`) |
| [`tools/wtime-goose`](../tools/wtime-goose/README.md) | Goose host & benchmark harness comparing `wtime`, `wtime-aot`, Wasmtime, and V8 |

---

## `wtime-cli` Reference

```sh
cargo run --locked -p wtime-cli -- --version
cargo run --locked -p wtime-cli -- check module.wasm
cargo run --locked -p wtime-cli -- exports module.wasm
cargo run --locked -p wtime-cli -- --fuel 100000 run module.wasm add i32:20 i32:22
```

Encode `.wat` text modules to binary `.wasm` using the conformance helper crate:

```sh
cargo run --locked -p wtime-conformance -- encode module.wat -o module.wasm
```

- **Feature profiles** (`--profile`): `mvp`, `stable-core`, and `all-phase3-core`.
- **Exit codes**:
  - `0` — success
  - `1` — usage, I/O, host, or link error
  - `2` — decode, validation, or disabled-feature error
  - `3` — guest trap
  - `4` — uncaught guest exception or suspension
  - `5` — resource limit or cancellation

Run `wtime-cli --help` for the complete flag list.

---

## Build, Test, & Verification Gates

Rust `1.99.0` is pinned in [`rust-toolchain.toml`](../rust-toolchain.toml).

### Core workspace checks

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

On a checkout that also built the pinned release-evidence binaries into `target/evidence/` (see [Release readiness](release-readiness.md)), run strict `python3 -B tools/checks/release_audit.py` without `--portable`.

### Standalone AOT compiler checks

Requires `clang` and `lld` on Linux x86-64:

```sh
cargo fmt --manifest-path tools/wtime-aot/Cargo.toml -- --check
cargo clippy --locked --manifest-path tools/wtime-aot/Cargo.toml --all-targets --all-features -- -D warnings
cargo test --locked --manifest-path tools/wtime-aot/rt/Cargo.toml
WTIME_AOT_REQUIRE_NATIVE_TESTS=1 cargo test --locked --manifest-path tools/wtime-aot/Cargo.toml
```

---

## `v0.1.0` Release Scope & Further Reading

- **Supported `v0.1.0` target**: `x86_64-unknown-linux-gnu` (native AArch64 release verification is planned for a follow-up release).
- **Spec scope**: WebAssembly Core 3.0 plus active Phase 3–5 proposals listed in [architecture/scope.md](architecture/scope.md). Browser/JS embedding APIs are excluded.
- **Additional docs**:
  - [Embedding guide](embedding.md) and [native examples](../examples/README.md)
  - [Embedding & ownership contract](architecture/api.md)
  - [Semantic invariants](architecture/hard_task_contracts.md) and [dependency policy](architecture/dependencies.md)
  - [Release readiness](release-readiness.md) and [release manifest](architecture/release-manifest.json)
  - [`wtime-aot` x64 ABI contract](wtime-aot/x64-abi.md)
