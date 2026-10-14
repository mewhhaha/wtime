# T034 — Run the complete MVP validation corpus

Status: **done**  
Area: `wtime-conformance`  
Work unit: one focused behavior/contract and its tests; split into child tasks before exceeding two focused patches.

## Prerequisites

[T033 — Seal the validated-module contract](../02-validation/T033-validated-api.md), [T012 — Implement spec-script parsing and outcome accounting](../00-foundation/T012-script-runner.md)

Use the predecessor's interfaces and evidence. Do not reimplement a second parser, type registry, store or execution stack.

## Deliverable

Wire malformed/invalid assertions through own decoder/validator. Import every relevant normative validation test, add adversarial depth/size cases and retain replay artifacts.

## Acceptance

All selected MVP decode/validation assertions execute with exact outcome accounting; any unexplained failure blocks the gate.

Add focused tests for the changed behavior and relevant failure boundaries. Record the exact command, feature profile, corpus/source revision and executed assertion counts. Follow [the common definition of done](../EXECUTION.md#common-definition-of-done); a missing directive, zero-test run or unimplemented branch is not a pass.

## Boundaries

Follow [ARCHITECTURE.md](../ARCHITECTURE.md): no external core production dependency, no per-proposal Cargo-crate proliferation, no JS/browser implementation, and no invented semantics for draft TODOs. Coordinate changes to shared types/opcode/control contracts with other ready tasks. Runtime feature gates and implementation resource limits must remain explicit.

## Implementation instructions for Sol 6.1 Low

Read [the interface contracts](../IMPLEMENTATION_CONTRACTS.md) and the listed prerequisite outputs before coding. Use the predecessor's API; do not invent a second implementation to avoid a missing prerequisite.

### Files and ownership

Expected paths in `/workspace/wtime` (created by implementation, not by this plan):

- `tools/wtime-conformance/src/adapter.rs`

Add focused tests in the relevant package's `tests/` directory, or its existing unit-test module. Name tests `t034_case_01`, `..._case_02`, etc. Reuse existing feature modules when present; coordinate edits to central dispatch/type tables.

### Ordered recipe

1. Wire malformed and invalid assertions to the actual decoder/validator.
2. Run locked MVP validation suites and add deep nesting/size-limit cases.
3. Record selected versus executed assertion totals and save any minimal failing input.
4. Add the cases below before considering the behavior finished. Keep malformed, invalid, trapping, host-error and resource-limit outcomes distinct.
5. Wire the change into the existing entry point/dispatch and update its feature-policy or ownership evidence where relevant. Run the commands below and record exact outcomes.

### Required cases

- `t034_case_01`: assert_malformed decode failure
- `t034_case_02`: assert_invalid type failure
- `t034_case_03`: No empty or filtered suite success.

For binary fixtures, keep a readable WAT/hex source and provenance alongside the `.wasm` or inline bytes. Generate fixtures with the separate encoder/wasm-tools; do not add `wast` to core tests. Expected outputs must come from the locked spec/rule or an independent calculation, not the engine being tested.

### Verification commands

Run from `/workspace/wtime` after activating the configured Rust environment:

```sh
. /workspace/.setup/rust-env.sh
cargo test --locked -p wtime-conformance t034_ -- --nocapture
cargo check --locked -p wtime-conformance
cargo fmt --all --check
cargo clippy --locked -p wtime-conformance --all-targets -- -D warnings
```

The filtered test invocation must execute the named cases with a positive count. If multiple packages are listed, at least each package with changed behavior must have an executed relevant test; a package's zero-match summary is not evidence for that package. Fix test placement/names rather than accepting a zero-test run.

### Handoff and scope limit

Record changed symbols/files, named test counts, corpus hash/profile if used, and remaining risks in Completion evidence. Stop at this task's interface; do not implement unrelated future features. If a prerequisite API is missing or pinned sources conflict, identify the exact missing contract/rule and keep the affected work blocked instead of guessing. Routine local coding choices do not require asking the user.

## Sources

- [MVP](https://github.com/WebAssembly/design/tree/6e43416ebc8becd6cdf5d91d9ae7d3712111df32)
- [Pinned integrated core specification](https://github.com/WebAssembly/spec/tree/7e9fb78c63d65018cc111422f657947b3df80d7b) is normative for finished core features; historical proposal repositories provide context.
- [Scope, source precedence and exclusions](../PROPOSALS.md).

## Completion evidence

- Changes: `tools/wtime-conformance/src/script.rs::AssertMalformed/AssertInvalid`; `tools/wtime-conformance/src/adapter.rs::Engine::compile`.
- Focused acceptance: `script::tests::t034_decode_and_validation_stages`; `corpus::tests::t034_filtered_selection_fails`; `script::tests::t012_case_02`. All named cases executed successfully in the final workspace test run (304 passed; conformance 31 unit + 3 annotations + 7 interactions = 41 passed).
- Commands and outcomes: `cargo test --locked --workspace --all-features` and workspace/all-target/all-feature strict Clippy passed. Exact corpus replay commands, source revisions/file SHA-256 values, binary fingerprint, implementation source fingerprints, and byte-identical archived reports are recorded in `/workspace/wtime/docs/architecture/conformance-gate-evidence.json`.
- Assertion counts / fixture hashes: `final-mvp.json`: profile `mvp`, 18480/18480 executed, 0 failed/skipped/unsupported; report SHA-256 `62a48c4ed416a42f2a965a2be4b1584f271685a856c2bfd53bb21cd3970280a8`; compatibility assertions 60.
- Follow-up or normative ambiguity: Historical MVP executes 60 explicitly reported current-normative compatibility checks and enables mutable globals for the locked WG1.0 catalog; this is distinct from strict CLI mvp. Phase3 executes 2 explicitly reported nonempty-tag-result current stack-switching overlay checks; stable-core retains original invalid expectations. No assertions are skipped.
