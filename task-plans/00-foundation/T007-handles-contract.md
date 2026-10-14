# T007 — Define store handles, roots and cross-store rules

Status: **done**  
Area: `wtime-runtime`  
Work unit: one focused behavior/contract and its tests; split into child tasks before exceeding two focused patches.

## Prerequisites

[T006 — Define portable value and module type shapes](../00-foundation/T006-type-shapes.md), [T002 — Write crate contracts and semantic invariants](../00-foundation/T002-crate-contracts.md)

Use the predecessor's interfaces and evidence. Do not reimplement a second parser, type registry, store or execution stack.

## Deliverable

Specify EngineId/StoreId ownership, generational handles, external-reference rooting and host borrowing. Store-local objects stay local; only explicit shared memories and immutable modules cross workers. Bound generation exhaustion instead of wrapping identities.

## Acceptance

Design examples reject stale and foreign handles, preserve ref.eq identity and keep host-retained refs rooted without requiring conservative stack scanning.

Add focused tests for the changed behavior and relevant failure boundaries. Record the exact command, feature profile, corpus/source revision and executed assertion counts. Follow [the common definition of done](../EXECUTION.md#common-definition-of-done); a missing directive, zero-test run or unimplemented branch is not a pass.

## Boundaries

Follow [ARCHITECTURE.md](../ARCHITECTURE.md): no external core production dependency, no per-proposal Cargo-crate proliferation, no JS/browser implementation, and no invented semantics for draft TODOs. Coordinate changes to shared types/opcode/control contracts with other ready tasks. Runtime feature gates and implementation resource limits must remain explicit.

## Implementation instructions for Sol 6.1 Low

Read [the interface contracts](../IMPLEMENTATION_CONTRACTS.md) and the listed prerequisite outputs before coding. Use the predecessor's API; do not invent a second implementation to avoid a missing prerequisite.

### Files and ownership

Expected paths in `/workspace/wtime` (created by implementation, not by this plan):

- `docs/architecture/handles.md`

This is a contract/documentation task: its acceptance checklist replaces an invented runtime test. Link concrete source rules and make the output usable by successor tasks.

### Ordered recipe

1. Specify private slot/generation/StoreId fields for runtime handles and Engine-bound canonical type IDs.
2. Specify root token creation/release and how Caller borrows end before reentry.
3. Choose generation retirement on exhaustion, not wraparound; document local versus shareable object kinds.
4. Add the cases below before considering the behavior finished. Keep malformed, invalid, trapping, host-error and resource-limit outcomes distinct.
5. Wire the change into the existing entry point/dispatch and update its feature-policy or ownership evidence where relevant. Run the commands below and record exact outcomes.

### Required cases

- `t007_case_01`: Foreign Store handle rejected before access
- `t007_case_02`: Cloned legitimate handle retains identity
- `t007_case_03`: No integer-to-reference public API.

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

- [MVP](https://github.com/WebAssembly/design/tree/6e43416ebc8becd6cdf5d91d9ae7d3712111df32)
- [Reference Types](https://github.com/WebAssembly/reference-types/tree/c7a3558bb5cbacb9c8879a91e08d0ac7276b740c)
- [Garbage collection](https://github.com/WebAssembly/gc/tree/756060f5816c7e2159f4817fbdee76cf52f9c923)
- [Pinned integrated core specification](https://github.com/WebAssembly/spec/tree/7e9fb78c63d65018cc111422f657947b3df80d7b) is normative for finished core features; historical proposal repositories provide context.
- [Scope, source precedence and exclusions](../PROPOSALS.md).

## Completion evidence

- Changes: Opaque Store-owned generational handles, explicit roots, generation retirement and checked shared-memory capabilities; ownership contract documented.
- Commands and outcomes: Runtime tests passed (108 total native tests), including foreign references, generation retirement, host retention, shared aliases and detached continuation collection.
- Assertion counts / fixture hashes: docs/architecture/handles.md; docs/verification/native-x86_64.json.
- Follow-up or normative ambiguity: No open implementation gap within this task; later integration gates remain separately tracked.
