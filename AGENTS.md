# Agent instructions

## Architecture and invariants

- The four core libraries (`wtime-types`, `wtime-decode`, `wtime-validate`,
  `wtime-runtime`) and `wtime-cli` must remain `#![forbid(unsafe_code)]` safe
  Rust with zero external production dependencies (`tools/checks/dependency_budget.py`).
- Public items in all four core library crates must remain documented
  (`#![warn(missing_docs)]`, verified with `RUSTDOCFLAGS="-D warnings" cargo doc`).
- Experimental interpreter tiers (`compact-executor`, `register-executor`) stay
  disabled by default and must preserve Wasm semantics, resource limits,
  source-level fuel, and cancellation contracts.
- `tools/wtime-aot` is a standalone opt-in AOT compiler excluded from the core
  workspace dependency graph. Keep `--backend x64`, `--backend c`, and
  `--backend rust` aligned on the Blot ABI 8 contract documented in
  `docs/wtime-aot/x64-abi.md`.

## Verification

- Pin Rust `1.99.0` per `rust-toolchain.toml`.
- Keep changes formatted (`cargo fmt --all -- --check`) and lint-clean
  (`cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`,
  plus `tools/wtime-aot` and `tools/wtime-aot/rt`).
- Run `WTIME_AOT_REQUIRE_NATIVE_TESTS=1 cargo test --locked --manifest-path tools/wtime-aot/Cargo.toml`
  when modifying `tools/wtime-aot` so native oracle/ABI/fault tests cannot
  silently skip.
- Run Python verification scripts with `python3 -B` to avoid writing `__pycache__`
  bytecode into the tree.
- Validate release-manifest evidence against real executed runs with
  `python3 -B tools/checks/release_audit.py` and
  `python3 -B tools/checks/opcode_coverage.py --require-complete`. Never commit
  stale or fabricated verification reports.
