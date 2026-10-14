# T101 — Pass the fixed-width SIMD conformance corpus

Status: **done**  
Area: `wtime-conformance`  
Work unit: one focused behavior/contract and its tests; split into child tasks before exceeding two focused patches.

## Prerequisites

[T095 — Execute vector constants and memory operations](../07-vector-proposals/T095-simd-memory.md), [T096 — Execute vector lane, shuffle and bitwise operations](../07-vector-proposals/T096-simd-permute.md), [T098 — Execute SIMD saturating, narrowing and widening operations](../07-vector-proposals/T098-simd-int-special.md), [T099 — Execute SIMD floating operations](../07-vector-proposals/T099-simd-floats.md), [T100 — Execute SIMD numeric conversions and demotion/promotion](../07-vector-proposals/T100-simd-convert.md), [T013 — Implement spectest imports and module registration](../00-foundation/T013-spectest-host.md)

Use the predecessor's interfaces and evidence. Do not reimplement a second parser, type registry, store or execution stack.

## Deliverable

Run every selected SIMD script plus opcode-ledger coverage. Exercise at least one host architecture without native SIMD optimization and memory-proposal combinations.

## Acceptance

All SIMD assertions execute; NaN cases use spec patterns and missing handlers are counted as failures.

Add focused tests for the changed behavior and relevant failure boundaries. Record the exact command, feature profile, corpus/source revision and executed assertion counts. Follow [the common definition of done](../EXECUTION.md#common-definition-of-done); a missing directive, zero-test run or unimplemented branch is not a pass.

## Boundaries

Follow [ARCHITECTURE.md](../ARCHITECTURE.md): no external core production dependency, no per-proposal Cargo-crate proliferation, no JS/browser implementation, and no invented semantics for draft TODOs. Coordinate changes to shared types/opcode/control contracts with other ready tasks. Runtime feature gates and implementation resource limits must remain explicit.

## Implementation instructions for Sol 6.1 Low

Read [the interface contracts](../IMPLEMENTATION_CONTRACTS.md) and the listed prerequisite outputs before coding. Use the predecessor's API; do not invent a second implementation to avoid a missing prerequisite.

### Files and ownership

Expected paths in `/workspace/wtime` (created by implementation, not by this plan):

- `tools/wtime-conformance/tests/simd.rs`

Add focused tests in the relevant package's `tests/` directory, or its existing unit-test module. Name tests `t101_case_01`, `..._case_02`, etc. Reuse existing feature modules when present; coordinate edits to central dispatch/type tables.

### Ordered recipe

1. Run complete selected SIMD scripts and compare counts to ledger ownership.
2. Add memory64/multimemory addressing fixtures without depending on host SIMD intrinsics.
3. Check NaNs with patterns rather than a particular payload.
4. Add the cases below before considering the behavior finished. Keep malformed, invalid, trapping, host-error and resource-limit outcomes distinct.
5. Wire the change into the existing entry point/dispatch and update its feature-policy or ownership evidence where relevant. Run the commands below and record exact outcomes.

### Required cases

- `t101_case_01`: All assigned instructions executed
- `t101_case_02`: One intentionally wrong lane result fails
- `t101_case_03`: Software-only path passes.

For binary fixtures, keep a readable WAT/hex source and provenance alongside the `.wasm` or inline bytes. Generate fixtures with the separate encoder/wasm-tools; do not add `wast` to core tests. Expected outputs must come from the locked spec/rule or an independent calculation, not the engine being tested.

### Verification commands

Run from `/workspace/wtime` after activating the configured Rust environment:

```sh
. /workspace/.setup/rust-env.sh
cargo test --locked -p wtime-conformance t101_ -- --nocapture
cargo check --locked -p wtime-conformance
cargo fmt --all --check
cargo clippy --locked -p wtime-conformance --all-targets -- -D warnings
cargo run --locked -p wtime-conformance -- run --profile stable-core --suite fixed-width_simd --filter all --report target/conformance/T101.json
```

The filtered test invocation must execute the named cases with a positive count. If multiple packages are listed, at least each package with changed behavior must have an executed relevant test; a package's zero-match summary is not evidence for that package. Fix test placement/names rather than accepting a zero-test run.

The corpus command must report every selected assertion as executed, with no unexplained failure, required skip or unsupported directive. A unit-test pass does not replace this gate.

### Handoff and scope limit

Record changed symbols/files, named test counts, corpus hash/profile if used, and remaining risks in Completion evidence. Stop at this task's interface; do not implement unrelated future features. If a prerequisite API is missing or pinned sources conflict, identify the exact missing contract/rule and keep the affected work blocked instead of guessing. Routine local coding choices do not require asking the user.

## Sources

- [Fixed-width SIMD](https://github.com/webassembly/simd/tree/a78b98a6899c9e91a13095e560767af6e99d98fd)
- [Pinned integrated core specification](https://github.com/WebAssembly/spec/tree/7e9fb78c63d65018cc111422f657947b3df80d7b) is normative for finished core features; historical proposal repositories provide context.
- [Scope, source precedence and exclusions](../PROPOSALS.md).

## Completion evidence

- Changes: `crates/wtime-runtime/src/simd.rs::execute`; `tools/wtime-conformance/src/script.rs::vector_matches`.
- Focused acceptance: `script::gate_acceptance_tests::t101_case_02`; `simd::tests::t093_simd_wrapping_saturation_and_signed_comparison`; `simd::tests::t094_simd_extension_uses_upper_half`; `simd::tests::t095_simd_float_min_signed_zero_and_nan`. All named cases executed successfully in the final workspace test run (304 passed; conformance 31 unit + 3 annotations + 7 interactions = 41 passed).
- Commands and outcomes: `cargo test --locked --workspace --all-features` and workspace/all-target/all-feature strict Clippy passed. Exact corpus replay commands, source revisions/file SHA-256 values, binary fingerprint, implementation source fingerprints, and byte-identical archived reports are recorded in `/workspace/wtime/docs/architecture/conformance-gate-evidence.json`.
- Assertion counts / fixture hashes: `final-fixed-width_simd.json`: profile `all-phase3-core`, 25515/25515 executed, 0 failed/skipped/unsupported; report SHA-256 `657803ddfd1819263590b177edcbdfd6ff70c164fff89b6b27a7231b7c867d52`; compatibility assertions 0. `final-core3.json`: profile `stable-core`, 62968/62968 executed, 0 failed/skipped/unsupported; report SHA-256 `2d1b1e665cf253f58ab3ffe305e6f77d60f2776283db2d9cb7bd0509d6bb1a4c`; compatibility assertions 0. `final-interactions.json`: profile `all-phase3-core`, 15/15 executed, 0 failed/skipped/unsupported; report SHA-256 `7b5dbeb1fc0a0c502eece8d9e5e975574f410ae53c4d1d735d986449c1547119`; compatibility assertions 0.
- Follow-up or normative ambiguity: None for this locked scope; unsupported directives and zero-test selection remain errors.
