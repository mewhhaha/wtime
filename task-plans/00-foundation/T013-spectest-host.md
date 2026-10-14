# T013 — Implement spectest imports and module registration

Status: **done**  
Area: `wtime-conformance`  
Work unit: one focused behavior/contract and its tests; split into child tasks before exceeding two focused patches.

## Prerequisites

[T012 — Implement spec-script parsing and outcome accounting](../00-foundation/T012-script-runner.md), [T007 — Define store handles, roots and cross-store rules](../00-foundation/T007-handles-contract.md)

Use the predecessor's interfaces and evidence. Do not reimplement a second parser, type registry, store or execution stack.

## Deliverable

Define spec fixture memories/tables/globals/functions and named-module registration across one Store. Use typed host APIs and explicit expected output; preserve references and exports across script steps.

## Acceptance

Multi-module scripts import the right instance and mutable global identity; spectest values and failure classes match the pinned test runner.

Add focused tests for the changed behavior and relevant failure boundaries. Record the exact command, feature profile, corpus/source revision and executed assertion counts. Follow [the common definition of done](../EXECUTION.md#common-definition-of-done); a missing directive, zero-test run or unimplemented branch is not a pass.

## Boundaries

Follow [ARCHITECTURE.md](../ARCHITECTURE.md): no external core production dependency, no per-proposal Cargo-crate proliferation, no JS/browser implementation, and no invented semantics for draft TODOs. Coordinate changes to shared types/opcode/control contracts with other ready tasks. Runtime feature gates and implementation resource limits must remain explicit.

## Implementation instructions for Sol 6.1 Low

Read [the interface contracts](../IMPLEMENTATION_CONTRACTS.md) and the listed prerequisite outputs before coding. Use the predecessor's API; do not invent a second implementation to avoid a missing prerequisite.

### Files and ownership

Expected paths in `/workspace/wtime` (created by implementation, not by this plan):

- `tools/wtime-conformance/src/spectest.rs`
- `tools/wtime-conformance/src/adapter.rs`

Add focused tests in the relevant package's `tests/` directory, or its existing unit-test module. Name tests `t013_case_01`, `..._case_02`, etc. Reuse existing feature modules when present; coordinate edits to central dispatch/type tables.

### Ordered recipe

1. Create the standard host fixture definitions with signatures/limits from the locked corpus.
2. Implement named module registration and export resolution in the test adapter.
3. Route print outputs to a captured sink and preserve actual external handle identities between directives.
4. Add the cases below before considering the behavior finished. Keep malformed, invalid, trapping, host-error and resource-limit outcomes distinct.
5. Wire the change into the existing entry point/dispatch and update its feature-policy or ownership evidence where relevant. Run the commands below and record exact outcomes.

### Required cases

- `t013_case_01`: Second module sees first mutable global
- `t013_case_02`: Wrong spectest signature is a link error
- `t013_case_03`: Print assertions use captured values.

For binary fixtures, keep a readable WAT/hex source and provenance alongside the `.wasm` or inline bytes. Generate fixtures with the separate encoder/wasm-tools; do not add `wast` to core tests. Expected outputs must come from the locked spec/rule or an independent calculation, not the engine being tested.

### Verification commands

Run from `/workspace/wtime` after activating the configured Rust environment:

```sh
. /workspace/.setup/rust-env.sh
cargo test --locked -p wtime-conformance t013_ -- --nocapture
cargo check --locked -p wtime-conformance
cargo fmt --all --check
cargo clippy --locked -p wtime-conformance --all-targets -- -D warnings
```

The filtered test invocation must execute the named cases with a positive count. If multiple packages are listed, at least each package with changed behavior must have an executed relevant test; a package's zero-match summary is not evidence for that package. Fix test placement/names rather than accepting a zero-test run.

### Handoff and scope limit

Record changed symbols/files, named test counts, corpus hash/profile if used, and remaining risks in Completion evidence. Stop at this task's interface; do not implement unrelated future features. If a prerequisite API is missing or pinned sources conflict, identify the exact missing contract/rule and keep the affected work blocked instead of guessing. Routine local coding choices do not require asking the user.

## Sources

- [MVP](https://github.com/WebAssembly/design/tree/6e43416ebc8becd6cdf5d91d9ae7d3712111df32)
- [Import/Export of Mutable Globals](https://github.com/WebAssembly/mutable-global/tree/89e5be9d69f2afac7243b6d2ff36b9c8723efb77)
- [Reference Types](https://github.com/WebAssembly/reference-types/tree/c7a3558bb5cbacb9c8879a91e08d0ac7276b740c)
- [Pinned integrated core specification](https://github.com/WebAssembly/spec/tree/7e9fb78c63d65018cc111422f657947b3df80d7b) is normative for finished core features; historical proposal repositories provide context.
- [Scope, source precedence and exclusions](../PROPOSALS.md).

## Completion evidence

- Changes: Typed spectest imports capture prints and preserve named module state; registration replaces bindings and shared workers import explicit shared memory.
- Commands and outcomes: All named t013 cases and shared worker tests passed; interaction corpus executes 10/10 with no failures.
- Assertion counts / fixture hashes: tools/wtime-conformance/src/spectest.rs and script.rs; target/conformance/T163.json.
- Follow-up or normative ambiguity: No open implementation gap within this task; later integration gates remain separately tracked.
