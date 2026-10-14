# T031 — Validate scalar numeric instruction families

Status: **done**  
Area: `wtime-validate`  
Work unit: one focused behavior/contract and its tests; split into child tasks before exceeding two focused patches.

## Prerequisites

[T028 — Implement operand/control stack validation](../02-validation/T028-validate-stack.md), [T008 — Create a complete instruction and section ledger](../00-foundation/T008-opcode-ledger.md)

Use the predecessor's interfaces and evidence. Do not reimplement a second parser, type registry, store or execution stack.

## Deliverable

Use reviewed scalar signatures for constants, integer operations, floats, comparisons, conversions and reinterpretations. Make unknown/reserved instructions impossible to type as defaults.

## Acceptance

All scalar families have valid and wrong-stack-type cases; conversions distinguish input/output widths and preserve raw-bit value shape.

Add focused tests for the changed behavior and relevant failure boundaries. Record the exact command, feature profile, corpus/source revision and executed assertion counts. Follow [the common definition of done](../EXECUTION.md#common-definition-of-done); a missing directive, zero-test run or unimplemented branch is not a pass.

## Boundaries

Follow [ARCHITECTURE.md](../ARCHITECTURE.md): no external core production dependency, no per-proposal Cargo-crate proliferation, no JS/browser implementation, and no invented semantics for draft TODOs. Coordinate changes to shared types/opcode/control contracts with other ready tasks. Runtime feature gates and implementation resource limits must remain explicit.

## Implementation instructions for Sol 6.1 Low

Read [the interface contracts](../IMPLEMENTATION_CONTRACTS.md) and the listed prerequisite outputs before coding. Use the predecessor's API; do not invent a second implementation to avoid a missing prerequisite.

### Files and ownership

Expected paths in `/workspace/wtime` (created by implementation, not by this plan):

- `crates/wtime-validate/src/function/numeric.rs`

Add focused tests in the relevant package's `tests/` directory, or its existing unit-test module. Name tests `t031_case_01`, `..._case_02`, etc. Reuse existing feature modules when present; coordinate edits to central dispatch/type tables.

### Ordered recipe

1. Add explicit scalar signatures grouped by input/output widths; use small local tables or matches.
2. Require no unknown opcode fallback and update the instruction ledger.
3. Test wrong input types as well as correct arity for every family.
4. Add the cases below before considering the behavior finished. Keep malformed, invalid, trapping, host-error and resource-limit outcomes distinct.
5. Wire the change into the existing entry point/dispatch and update its feature-policy or ownership evidence where relevant. Run the commands below and record exact outcomes.

### Required cases

- `t031_case_01`: i64 op with i32 operand
- `t031_case_02`: Conversion output width
- `t031_case_03`: Unknown scalar assignment remains rejected.

For binary fixtures, keep a readable WAT/hex source and provenance alongside the `.wasm` or inline bytes. Generate fixtures with the separate encoder/wasm-tools; do not add `wast` to core tests. Expected outputs must come from the locked spec/rule or an independent calculation, not the engine being tested.

### Verification commands

Run from `/workspace/wtime` after activating the configured Rust environment:

```sh
. /workspace/.setup/rust-env.sh
cargo test --locked -p wtime-validate t031_ -- --nocapture
cargo check --locked -p wtime-validate
cargo fmt --all --check
cargo clippy --locked -p wtime-validate --all-targets -- -D warnings
```

The filtered test invocation must execute the named cases with a positive count. If multiple packages are listed, at least each package with changed behavior must have an executed relevant test; a package's zero-match summary is not evidence for that package. Fix test placement/names rather than accepting a zero-test run.

### Handoff and scope limit

Record changed symbols/files, named test counts, corpus hash/profile if used, and remaining risks in Completion evidence. Stop at this task's interface; do not implement unrelated future features. If a prerequisite API is missing or pinned sources conflict, identify the exact missing contract/rule and keep the affected work blocked instead of guessing. Routine local coding choices do not require asking the user.

## Sources

- [MVP](https://github.com/WebAssembly/design/tree/6e43416ebc8becd6cdf5d91d9ae7d3712111df32)
- [Pinned integrated core specification](https://github.com/WebAssembly/spec/tree/7e9fb78c63d65018cc111422f657947b3df80d7b) is normative for finished core features; historical proposal repositories provide context.
- [Scope, source precedence and exclusions](../PROPOSALS.md).

## Completion evidence

- Changes: Validated numeric operand and conversion result widths through explicit scalar signatures; unknown opcode dispatch fails closed.
- Source/tests: `crates/wtime-validate/src/lib.rs`, `crates/wtime-validate/src/acceptance.rs`.
- Commands and outcomes: cargo test --locked -p wtime-validate: 83 unit + 4 integration + 3 compile-fail doctests = 90 passed, 0 failed (replayed 2026-10-11).
- Named tests: `t031_` matches 2 executed passing tests in the full relevant suite; zero-match summaries from other test binaries are not counted.
- Shared acceptance evidence: tests::t033_unknown_opcode.
- Quality checks: `cargo clippy -p wtime-validate --all-targets -- -D warnings` passed after the final source changes; `cargo fmt --all` applied. Runtime package quality checks are maintained by the runtime/release owner.
- Fixture/source digest: `322c98e589f396f77097a7acf1dabc666f72ccbba8936703cb115ed7befd9621` for `crates/wtime-validate/src/acceptance.rs` (readable inline binary construction).
- Formal/spec evidence: pinned integrated core `7e9fb78c63d65018cc111422f657947b3df80d7b`; `docs/architecture/validation-premise-audit.json` links inspected direct predicates to focused tests and exact locked corpus source/form offsets. Active proposal overlays remain explicit.
- Follow-up: final aggregate corpus source bindings are maintained by the conformance/release owners. This task has no pending source changes; T034 is reconciled separately.
