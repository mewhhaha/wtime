# T032 — Validate memory, table and segment rules

Status: **done**  
Area: `wtime-validate`  
Work unit: one focused behavior/contract and its tests; split into child tasks before exceeding two focused patches.

## Prerequisites

[T028 — Implement operand/control stack validation](../02-validation/T028-validate-stack.md), [T027 — Validate baseline constant expressions](../02-validation/T027-validate-const.md), [T021 — Decode data and element segment forms](../01-binary-decoding/T021-decode-segments.md)

Use the predecessor's interfaces and evidence. Do not reimplement a second parser, type registry, store or execution stack.

## Deliverable

Check alignment immediates, index spaces, address types, limits, segment targets and passive/data-count constraints. Validate offsets against types, not current runtime sizes.

## Acceptance

Over-aligned scalar operations and invalid segment modes reject; out-of-bounds runtime offsets remain valid modules that trap on instantiation.

Add focused tests for the changed behavior and relevant failure boundaries. Record the exact command, feature profile, corpus/source revision and executed assertion counts. Follow [the common definition of done](../EXECUTION.md#common-definition-of-done); a missing directive, zero-test run or unimplemented branch is not a pass.

## Boundaries

Follow [ARCHITECTURE.md](../ARCHITECTURE.md): no external core production dependency, no per-proposal Cargo-crate proliferation, no JS/browser implementation, and no invented semantics for draft TODOs. Coordinate changes to shared types/opcode/control contracts with other ready tasks. Runtime feature gates and implementation resource limits must remain explicit.

## Implementation instructions for Sol 6.1 Low

Read [the interface contracts](../IMPLEMENTATION_CONTRACTS.md) and the listed prerequisite outputs before coding. Use the predecessor's API; do not invent a second implementation to avoid a missing prerequisite.

### Files and ownership

Expected paths in `/workspace/wtime` (created by implementation, not by this plan):

- `crates/wtime-validate/src/function/memory.rs`

Add focused tests in the relevant package's `tests/` directory, or its existing unit-test module. Name tests `t032_case_01`, `..._case_02`, etc. Reuse existing feature modules when present; coordinate edits to central dispatch/type tables.

### Ordered recipe

1. Resolve each memory/table/segment target and check operand types and alignment exponent.
2. Validate active offsets structurally, leaving current-instance OOB to execution.
3. Check data-count/passive/declarative obligations for segment-indexed instructions.
4. Add the cases below before considering the behavior finished. Keep malformed, invalid, trapping, host-error and resource-limit outcomes distinct.
5. Wire the change into the existing entry point/dispatch and update its feature-policy or ownership evidence where relevant. Run the commands below and record exact outcomes.

### Required cases

- `t032_case_01`: Alignment exponent too large
- `t032_case_02`: Unknown segment index
- `t032_case_03`: Statically valid initializer that traps at instantiation.

For binary fixtures, keep a readable WAT/hex source and provenance alongside the `.wasm` or inline bytes. Generate fixtures with the separate encoder/wasm-tools; do not add `wast` to core tests. Expected outputs must come from the locked spec/rule or an independent calculation, not the engine being tested.

### Verification commands

Run from `/workspace/wtime` after activating the configured Rust environment:

```sh
. /workspace/.setup/rust-env.sh
cargo test --locked -p wtime-validate t032_ -- --nocapture
cargo check --locked -p wtime-validate
cargo fmt --all --check
cargo clippy --locked -p wtime-validate --all-targets -- -D warnings
```

The filtered test invocation must execute the named cases with a positive count. If multiple packages are listed, at least each package with changed behavior must have an executed relevant test; a package's zero-match summary is not evidence for that package. Fix test placement/names rather than accepting a zero-test run.

### Handoff and scope limit

Record changed symbols/files, named test counts, corpus hash/profile if used, and remaining risks in Completion evidence. Stop at this task's interface; do not implement unrelated future features. If a prerequisite API is missing or pinned sources conflict, identify the exact missing contract/rule and keep the affected work blocked instead of guessing. Routine local coding choices do not require asking the user.

## Sources

- [MVP](https://github.com/WebAssembly/design/tree/6e43416ebc8becd6cdf5d91d9ae7d3712111df32)
- [Bulk memory operations](https://github.com/WebAssembly/bulk-memory-operations/tree/dcaa1b6791401c29b67e8cd7929ec80949f1f849)
- [Reference Types](https://github.com/WebAssembly/reference-types/tree/c7a3558bb5cbacb9c8879a91e08d0ac7276b740c)
- [Pinned integrated core specification](https://github.com/WebAssembly/spec/tree/7e9fb78c63d65018cc111422f657947b3df80d7b) is normative for finished core features; historical proposal repositories provide context.
- [Scope, source precedence and exclusions](../PROPOSALS.md).

## Completion evidence

- Changes: Validated natural memory alignment and segment indices; statically legal out-of-bounds active data remains an instantiation trap rather than a validation error.
- Source/tests: `crates/wtime-validate/src/lib.rs`, `crates/wtime-validate/src/acceptance.rs`.
- Commands and outcomes: cargo test --locked -p wtime-validate: 83 unit + 4 integration + 3 compile-fail doctests = 90 passed, 0 failed (replayed 2026-10-11).
- Named tests: `t032_` matches 3 executed passing tests in the full relevant suite; zero-match summaries from other test binaries are not counted.
- Shared acceptance evidence: wtime-runtime::tests::t043_earlier_data_writes_survive_later_segment_trap.
- Quality checks: `cargo clippy -p wtime-validate --all-targets -- -D warnings` passed after the final source changes; `cargo fmt --all` applied. Runtime package quality checks are maintained by the runtime/release owner.
- Fixture/source digest: `322c98e589f396f77097a7acf1dabc666f72ccbba8936703cb115ed7befd9621` for `crates/wtime-validate/src/acceptance.rs` (readable inline binary construction).
- Formal/spec evidence: pinned integrated core `7e9fb78c63d65018cc111422f657947b3df80d7b`; `docs/architecture/validation-premise-audit.json` links inspected direct predicates to focused tests and exact locked corpus source/form offsets. Active proposal overlays remain explicit.
- Follow-up: final aggregate corpus source bindings are maintained by the conformance/release owners. This task has no pending source changes; T034 is reconciled separately.
