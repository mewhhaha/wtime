# T004 — Define feature dependencies and resource limits

Status: **done**  
Area: `wtime-types`  
Work unit: one focused behavior/contract and its tests; split into child tasks before exceeding two focused patches.

## Prerequisites

[T003 — Create the minimal Rust workspace](../00-foundation/T003-workspace.md)

Use the predecessor's interfaces and evidence. Do not reimplement a second parser, type registry, store or execution stack.

## Deliverable

Define a runtime FeatureSet, stable and all_phase3_core profiles, dependency closure and independently configured policy limits. Add u64 counts/address widths and page-size metadata now. Keep proposal toggles out of Cargo's feature matrix.

## Acceptance

Reject contradictory profiles; enabling descriptors requires GC/function-reference prerequisites and stack switching requires its tag/type prerequisites. Limits have separate diagnostics from malformed/invalid Wasm.

Add focused tests for the changed behavior and relevant failure boundaries. Record the exact command, feature profile, corpus/source revision and executed assertion counts. Follow [the common definition of done](../EXECUTION.md#common-definition-of-done); a missing directive, zero-test run or unimplemented branch is not a pass.

## Boundaries

Follow [ARCHITECTURE.md](../ARCHITECTURE.md): no external core production dependency, no per-proposal Cargo-crate proliferation, no JS/browser implementation, and no invented semantics for draft TODOs. Coordinate changes to shared types/opcode/control contracts with other ready tasks. Runtime feature gates and implementation resource limits must remain explicit.

## Implementation instructions for Sol 6.1 Low

Read [the interface contracts](../IMPLEMENTATION_CONTRACTS.md) and the listed prerequisite outputs before coding. Use the predecessor's API; do not invent a second implementation to avoid a missing prerequisite.

### Files and ownership

Expected paths in `/workspace/wtime` (created by implementation, not by this plan):

- `crates/wtime-types/src/features.rs`

Add focused tests in the relevant package's `tests/` directory, or its existing unit-test module. Name tests `t004_case_01`, `..._case_02`, etc. Reuse existing feature modules when present; coordinate edits to central dispatch/type tables.

### Ordered recipe

1. Enumerate a bit/field for each included semantic feature and profiles MVP/stable-core/all-phase3-core.
2. Implement profile construction plus dependency validation; keep resource limits as separate fields with checked conversions.
3. Expose named diagnostics for disabled features and policy exhaustion; do not gate semantic modules using Cargo features.
4. Add the cases below before considering the behavior finished. Keep malformed, invalid, trapping, host-error and resource-limit outcomes distinct.
5. Wire the change into the existing entry point/dispatch and update its feature-policy or ownership evidence where relevant. Run the commands below and record exact outcomes.

### Required cases

- `t004_case_01`: Descriptors without GC prerequisites fail configuration
- `t004_case_02`: Stable profile excludes active drafts
- `t004_case_03`: Changing a resource budget never makes an invalid module valid.

For binary fixtures, keep a readable WAT/hex source and provenance alongside the `.wasm` or inline bytes. Generate fixtures with the separate encoder/wasm-tools; do not add `wast` to core tests. Expected outputs must come from the locked spec/rule or an independent calculation, not the engine being tested.

### Verification commands

Run from `/workspace/wtime` after activating the configured Rust environment:

```sh
. /workspace/.setup/rust-env.sh
cargo test --locked -p wtime-types t004_ -- --nocapture
cargo check --locked -p wtime-types
cargo fmt --all --check
cargo clippy --locked -p wtime-types --all-targets -- -D warnings
```

The filtered test invocation must execute the named cases with a positive count. If multiple packages are listed, at least each package with changed behavior must have an executed relevant test; a package's zero-match summary is not evidence for that package. Fix test placement/names rather than accepting a zero-test run.

### Handoff and scope limit

Record changed symbols/files, named test counts, corpus hash/profile if used, and remaining risks in Completion evidence. Stop at this task's interface; do not implement unrelated future features. If a prerequisite API is missing or pinned sources conflict, identify the exact missing contract/rule and keep the affected work blocked instead of guessing. Routine local coding choices do not require asking the user.

## Sources

- [Frozen proposal scope](../PROPOSALS.md) and [architecture](../ARCHITECTURE.md).
- [Pinned integrated core specification](https://github.com/WebAssembly/spec/tree/7e9fb78c63d65018cc111422f657947b3df80d7b) is normative for finished core features; historical proposal repositories provide context.
- [Scope, source precedence and exclusions](../PROPOSALS.md).

## Completion evidence

- Changes: FeatureSet validates dependencies and profiles; ResourceLimits separates semantic page limits, per-memory bytes and aggregate Store policy.
- Commands and outcomes: 13 shared-type tests, 28 feature-profile integration tests, and 2 validator policy tests passed in cargo test --workspace --locked.
- Assertion counts / fixture hashes: docs/verification/feature-profiles.json; features.rs and validator/tests/policy.rs.
- Follow-up or normative ambiguity: No open implementation gap within this task; later integration gates remain separately tracked.
