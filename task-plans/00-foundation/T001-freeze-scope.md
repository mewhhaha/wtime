# T001 — Freeze the specification and proposal test revisions

Status: **done**  
Area: `planning`  
Work unit: one focused behavior/contract and its tests; split into child tasks before exceeding two focused patches.

## Prerequisites

None. This is the first ready task.

Use the predecessor's interfaces and evidence. Do not reimplement a second parser, type registry, store or execution stack.

## Deliverable

Record the catalog/core-spec commits and each active core proposal revision. Extract normative encodings, validation and execution rules into a feature ledger; identify explainer-only rules and unresolved TODOs rather than inventing semantics. Preserve the user's JS/browser exclusions.

## Acceptance

Every included entry has an authoritative revision and a test-source strategy; no Phase 0–2 feature enters scope implicitly.

Add focused tests for the changed behavior and relevant failure boundaries. Record the exact command, feature profile, corpus/source revision and executed assertion counts. Follow [the common definition of done](../EXECUTION.md#common-definition-of-done); a missing directive, zero-test run or unimplemented branch is not a pass.

## Boundaries

Follow [ARCHITECTURE.md](../ARCHITECTURE.md): no external core production dependency, no per-proposal Cargo-crate proliferation, no JS/browser implementation, and no invented semantics for draft TODOs. Coordinate changes to shared types/opcode/control contracts with other ready tasks. Runtime feature gates and implementation resource limits must remain explicit.

## Implementation instructions for Sol 6.1 Low

Read [the interface contracts](../IMPLEMENTATION_CONTRACTS.md) and the listed prerequisite outputs before coding. Use the predecessor's API; do not invent a second implementation to avoid a missing prerequisite.

### Files and ownership

Expected paths in `/workspace/wtime` (created by implementation, not by this plan):

- `docs/architecture/spec-lock.json`
- `docs/architecture/scope.md`

This is a contract/documentation task: its acceptance checklist replaces an invented runtime test. Link concrete source rules and make the output usable by successor tasks.

### Ordered recipe

1. Start from SOURCES.json, fetch exact commit objects and identify normative core/proposal documents and test directories.
2. Produce docs/architecture/spec-lock.json with commit, directory, checksum, feature ID and unresolved clauses for every included entry.
3. List all proposal conflicts/TODOs with the rule owner; explicitly exclude the five JS/browser entries and lower phases.
4. Add the cases below before considering the behavior finished. Keep malformed, invalid, trapping, host-error and resource-limit outcomes distinct.
5. Wire the change into the existing entry point/dispatch and update its feature-policy or ownership evidence where relevant. Run the commands below and record exact outcomes.

### Required cases

- `t001_case_01`: Re-fetch produces identical corpus identity
- `t001_case_02`: Every included row has normative text and test strategy
- `t001_case_03`: No moving branch URL is used for acceptance.

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

- [Frozen proposal scope](../PROPOSALS.md) and [architecture](../ARCHITECTURE.md).
- [Pinned integrated core specification](https://github.com/WebAssembly/spec/tree/7e9fb78c63d65018cc111422f657947b3df80d7b) is normative for finished core features; historical proposal repositories provide context.
- [Scope, source precedence and exclusions](../PROPOSALS.md).

## Completion evidence

- Changes: docs/architecture/spec-lock.json and scope.md freeze 26 source repositories (24 included proposals plus catalog and core), with five explicit JS/browser exclusions.
- Commands and outcomes: Source audit fetched exact commit objects and checked root tree hashes; JSON parse and 26-source/5-exclusion assertions passed.
- Assertion counts / fixture hashes: 26 immutable source records; source and corpus identities recorded in manifest.
- Follow-up or normative ambiguity: No open issue within this task; feature gates remain separate tasks.
