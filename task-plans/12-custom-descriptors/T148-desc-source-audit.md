# T148 — Resolve the pinned custom-descriptor core contract

Status: **done**  
Area: `planning`  
Work unit: one focused behavior/contract and its tests; split into child tasks before exceeding two focused patches.

## Prerequisites

[T001 — Freeze the specification and proposal test revisions](../00-foundation/T001-freeze-scope.md)

Use the predecessor's interfaces and evidence. Do not reimplement a second parser, type registry, store or execution stack.

## Deliverable

Extract core descriptor/describes/exact/import/allocation/cast rules and tests, separating JS prototype APIs and prose TODOs. The current field-deduplication section is unspecified: record a future source-refresh decision, not invented encodings.

## Acceptance

Every specified core rule has an implementable source and owner; excluded JS behaviors are listed. Any genuine normative conflict blocks only the affected tasks and is escalated as a decision.

Add focused tests for the changed behavior and relevant failure boundaries. Record the exact command, feature profile, corpus/source revision and executed assertion counts. Follow [the common definition of done](../EXECUTION.md#common-definition-of-done); a missing directive, zero-test run or unimplemented branch is not a pass.

## Boundaries

Follow [ARCHITECTURE.md](../ARCHITECTURE.md): no external core production dependency, no per-proposal Cargo-crate proliferation, no JS/browser implementation, and no invented semantics for draft TODOs. Coordinate changes to shared types/opcode/control contracts with other ready tasks. Runtime feature gates and implementation resource limits must remain explicit.

## Implementation instructions for Sol 6.1 Low

Read [the interface contracts](../IMPLEMENTATION_CONTRACTS.md) and the listed prerequisite outputs before coding. Use the predecessor's API; do not invent a second implementation to avoid a missing prerequisite.

### Files and ownership

Expected paths in `/workspace/wtime` (created by implementation, not by this plan):

- `docs/architecture/descriptors.md`

This is a contract/documentation task: its acceptance checklist replaces an invented runtime test. Link concrete source rules and make the output usable by successor tasks.

### Ordered recipe

1. Extract core clauses into a rule matrix: syntax, paired links, exact types/imports, constructors and casts.
2. Mark JS prototypes excluded and field-deduplication TODO unspecified; do not infer shared-everything support from examples.
3. Cross-reference each normative rule to T149–T157 and record unresolved source contradictions.
4. Add the cases below before considering the behavior finished. Keep malformed, invalid, trapping, host-error and resource-limit outcomes distinct.
5. Wire the change into the existing entry point/dispatch and update its feature-policy or ownership evidence where relevant. Run the commands below and record exact outcomes.

### Required cases

- `t148_case_01`: Every specified core clause has an owner
- `t148_case_02`: No TODO treated as encoding
- `t148_case_03`: Source revision matches lock manifest.

For binary fixtures, keep a readable WAT/hex source and provenance alongside the `.wasm` or inline bytes. Generate fixtures with the separate encoder/wasm-tools; do not add `wast` to core tests. Expected outputs must come from the locked spec/rule or an independent calculation, not the engine being tested.

### Verification commands

Run from `/workspace/wtime` after activating the configured Rust environment:

```sh
. /workspace/.setup/rust-env.sh
git diff --check
```

### Handoff and scope limit

Record changed symbols/files, named test counts, corpus hash/profile if used, and remaining risks in Completion evidence. Stop at this task's interface; do not implement unrelated future features. If a prerequisite API is missing or pinned sources conflict, identify the exact missing contract/rule and keep the affected work blocked instead of guessing. Routine local coding choices do not require asking the user.

## Sources

- [Custom Descriptors and JS Interop](https://github.com/WebAssembly/custom-descriptors/tree/4aa477a4c1ca064fe3592b3a0307509e9059d0f9)
- [Pinned integrated core specification](https://github.com/WebAssembly/spec/tree/7e9fb78c63d65018cc111422f657947b3df80d7b) is normative for finished core features; historical proposal repositories provide context.
- [Scope, source precedence and exclusions](../PROPOSALS.md).

## Completion evidence

- Changes: docs/architecture/descriptors.md audits actual pinned descriptor proposal rules, exact types/imports and excluded JS behavior; field-dedup TODO remains unspecified.
- Commands and outcomes: Source audit against custom-descriptors commit4aa477a4c1ca064fe3592b3a0307509e9059d0f9 and concrete test inventory; JSON/git diff checks passed.
- Assertion counts / fixture hashes: Normative delta identified as proposals/custom-descriptors/Overview.md, separate from inherited core3 baseline.
- Follow-up or normative ambiguity: No open issue within this task; feature gates remain separate tasks.
