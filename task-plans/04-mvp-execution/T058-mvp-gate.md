# T058 — Pass the complete selected MVP corpus

Status: **done**  
Area: `wtime-conformance`  
Work unit: one focused behavior/contract and its tests; split into child tasks before exceeding two focused patches.

## Prerequisites

[T057 — Add a small CLI for inspect, validate and invoke](../04-mvp-execution/T057-cli.md), [T013 — Implement spectest imports and module registration](../00-foundation/T013-spectest-host.md), [T034 — Run the complete MVP validation corpus](../02-validation/T034-validation-gate.md)

Use the predecessor's interfaces and evidence. Do not reimplement a second parser, type registry, store or execution stack.

## Deliverable

Connect all runtime actions/assertions and run the entire pinned MVP corpus including instantiation/link cases and nontrivial execution. Store counts and failure artifacts.

## Acceptance

No unexplained fail, unsupported directive or required skip remains. Report resource-limited cases distinctly; a zero-assertion run fails.

Add focused tests for the changed behavior and relevant failure boundaries. Record the exact command, feature profile, corpus/source revision and executed assertion counts. Follow [the common definition of done](../EXECUTION.md#common-definition-of-done); a missing directive, zero-test run or unimplemented branch is not a pass.

## Boundaries

Follow [ARCHITECTURE.md](../ARCHITECTURE.md): no external core production dependency, no per-proposal Cargo-crate proliferation, no JS/browser implementation, and no invented semantics for draft TODOs. Coordinate changes to shared types/opcode/control contracts with other ready tasks. Runtime feature gates and implementation resource limits must remain explicit.

## Implementation instructions for Sol 6.1 Low

Read [the interface contracts](../IMPLEMENTATION_CONTRACTS.md) and the listed prerequisite outputs before coding. Use the predecessor's API; do not invent a second implementation to avoid a missing prerequisite.

### Files and ownership

Expected paths in `/workspace/wtime` (created by implementation, not by this plan):

- `tools/wtime-conformance/src/adapter.rs`
- `tools/wtime-conformance/tests/mvp.rs`

Add focused tests in the relevant package's `tests/` directory, or its existing unit-test module. Name tests `t058_case_01`, `..._case_02`, etc. Reuse existing feature modules when present; coordinate edits to central dispatch/type tables.

### Ordered recipe

1. Run all locked MVP scripts through the real adapter and inventory every directive.
2. Resolve each failure to decoding/validation/link/execution rather than muting assertions.
3. Save counted report and minimal regressions for any corrected defects.
4. Add the cases below before considering the behavior finished. Keep malformed, invalid, trapping, host-error and resource-limit outcomes distinct.
5. Wire the change into the existing entry point/dispatch and update its feature-policy or ownership evidence where relevant. Run the commands below and record exact outcomes.

### Required cases

- `t058_case_01`: Full selected count executes
- `t058_case_02`: Instantiation trap not link error
- `t058_case_03`: Zero-test false pass rejected.

For binary fixtures, keep a readable WAT/hex source and provenance alongside the `.wasm` or inline bytes. Generate fixtures with the separate encoder/wasm-tools; do not add `wast` to core tests. Expected outputs must come from the locked spec/rule or an independent calculation, not the engine being tested.

### Verification commands

Run from `/workspace/wtime` after activating the configured Rust environment:

```sh
. /workspace/.setup/rust-env.sh
cargo test --locked -p wtime-conformance t058_ -- --nocapture
cargo check --locked -p wtime-conformance
cargo fmt --all --check
cargo clippy --locked -p wtime-conformance --all-targets -- -D warnings
cargo run --locked -p wtime-conformance -- run --profile mvp --suite mvp --filter all --report target/conformance/T058.json
```

The filtered test invocation must execute the named cases with a positive count. If multiple packages are listed, at least each package with changed behavior must have an executed relevant test; a package's zero-match summary is not evidence for that package. Fix test placement/names rather than accepting a zero-test run.

The corpus command must report every selected assertion as executed, with no unexplained failure, required skip or unsupported directive. A unit-test pass does not replace this gate.

### Handoff and scope limit

Record changed symbols/files, named test counts, corpus hash/profile if used, and remaining risks in Completion evidence. Stop at this task's interface; do not implement unrelated future features. If a prerequisite API is missing or pinned sources conflict, identify the exact missing contract/rule and keep the affected work blocked instead of guessing. Routine local coding choices do not require asking the user.

## Sources

- [MVP](https://github.com/WebAssembly/design/tree/6e43416ebc8becd6cdf5d91d9ae7d3712111df32)
- [Import/Export of Mutable Globals](https://github.com/WebAssembly/mutable-global/tree/89e5be9d69f2afac7243b6d2ff36b9c8723efb77)
- [Pinned integrated core specification](https://github.com/WebAssembly/spec/tree/7e9fb78c63d65018cc111422f657947b3df80d7b) is normative for finished core features; historical proposal repositories provide context.
- [Scope, source precedence and exclusions](../PROPOSALS.md).

## Completion evidence

- Changes: `tools/wtime-conformance/src/script.rs::AssertTrap/AssertUnlinkable`; `crates/wtime-runtime/src/linker.rs::Linker::instantiate`.
- Focused acceptance: `script::gate_acceptance_tests::t058_case_02`; `script::tests::t012_case_02`. All named cases executed successfully in the final workspace test run (304 passed; conformance 31 unit + 3 annotations + 7 interactions = 41 passed).
- Commands and outcomes: `cargo test --locked --workspace --all-features` and workspace/all-target/all-feature strict Clippy passed. Exact corpus replay commands, source revisions/file SHA-256 values, binary fingerprint, implementation source fingerprints, and byte-identical archived reports are recorded in `/workspace/wtime/docs/architecture/conformance-gate-evidence.json`.
- Assertion counts / fixture hashes: `final-mvp.json`: profile `mvp`, 18480/18480 executed, 0 failed/skipped/unsupported; report SHA-256 `62a48c4ed416a42f2a965a2be4b1584f271685a856c2bfd53bb21cd3970280a8`; compatibility assertions 60.
- Follow-up or normative ambiguity: Historical MVP executes 60 explicitly reported current-normative compatibility checks and enables mutable globals for the locked WG1.0 catalog; this is distinct from strict CLI mvp. Phase3 executes 2 explicitly reported nonempty-tag-result current stack-switching overlay checks; stable-core retains original invalid expectations. No assertions are skipped.
