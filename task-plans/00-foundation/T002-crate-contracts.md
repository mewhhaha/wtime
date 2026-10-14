# T002 — Write crate contracts and semantic invariants

Status: **done**  
Area: `planning`  
Work unit: one focused behavior/contract and its tests; split into child tasks before exceeding two focused patches.

## Prerequisites

[T001 — Freeze the specification and proposal test revisions](../00-foundation/T001-freeze-scope.md)

Use the predecessor's interfaces and evidence. Do not reimplement a second parser, type registry, store or execution stack.

## Deliverable

Specify ownership of decoded versus validated versus executable modules; Engine-bound type identities; Store lifetimes; reference roots; thread sharing; continuation boundaries and host callbacks. Fix the dependency DAG before concurrent implementation.

## Acceptance

Review a module's lifetime, an imported GC type, a suspended continuation and an atomic wait against the contracts; each has an owner and cleanup path.

Add focused tests for the changed behavior and relevant failure boundaries. Record the exact command, feature profile, corpus/source revision and executed assertion counts. Follow [the common definition of done](../EXECUTION.md#common-definition-of-done); a missing directive, zero-test run or unimplemented branch is not a pass.

## Boundaries

Follow [ARCHITECTURE.md](../ARCHITECTURE.md): no external core production dependency, no per-proposal Cargo-crate proliferation, no JS/browser implementation, and no invented semantics for draft TODOs. Coordinate changes to shared types/opcode/control contracts with other ready tasks. Runtime feature gates and implementation resource limits must remain explicit.

## Implementation instructions for Sol 6.1 Low

Read [the interface contracts](../IMPLEMENTATION_CONTRACTS.md) and the listed prerequisite outputs before coding. Use the predecessor's API; do not invent a second implementation to avoid a missing prerequisite.

### Files and ownership

Expected paths in `/workspace/wtime` (created by implementation, not by this plan):

- `docs/architecture/interfaces.md`
- `docs/architecture/ownership.md`

This is a contract/documentation task: its acceptance checklist replaces an invented runtime test. Link concrete source rules and make the output usable by successor tasks.

### Ordered recipe

1. Copy the shared choices from IMPLEMENTATION_CONTRACTS.md into repository architecture docs.
2. Write concrete Rust signatures and ownership diagrams for decode/validate/Engine/Store/Linker/Caller; specify which crate constructs each opaque type.
3. Review one mutable-global import, one GC ref, one host reentry and one suspended stack against those signatures before allowing consumers.
4. Add the cases below before considering the behavior finished. Keep malformed, invalid, trapping, host-error and resource-limit outcomes distinct.
5. Wire the change into the existing entry point/dispatch and update its feature-policy or ownership evidence where relevant. Run the commands below and record exact outcomes.

### Required cases

- `t002_case_01`: No dependency cycle
- `t002_case_02`: No public unchecked ValidatedModule or raw-reference constructor
- `t002_case_03`: Every retained buffer/handle/root has one documented owner.

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

- Changes: docs/architecture/api.md, implementation_contracts.md and hard_task_contracts.md record concrete interfaces, dependencies, frame ownership and semantic invariants.
- Commands and outcomes: Reviewed interfaces against compiling workspace and actual public API.
- Assertion counts / fixture hashes: Documentation contract task; executable evidence belongs to consuming tasks.
- Follow-up or normative ambiguity: No open issue within this task; feature gates remain separate tasks.
