# T168 — Document embedding, feature scope and compatibility

Status: **done**  
Area: `documentation`  
Work unit: one focused behavior/contract and its tests; split into child tasks before exceeding two focused patches.

## Prerequisites

[T163 — Exercise proposal interaction scenarios](../14-integration-and-release/T163-interaction-tests.md), [T167 — Meet compile-time and dependency budgets](../14-integration-and-release/T167-fast-build-gate.md)

Use the predecessor's interfaces and evidence. Do not reimplement a second parser, type registry, store or execution stack.

## Deliverable

Document Engine/Module/Store/Linker APIs, typed hosts, limits, concurrency, reference rooting, exceptions and continuation ownership. Publish the pinned proposal matrix and excluded JS/browser/Phase0–2 scope with examples.

## Acceptance

Native examples build/run; memory64/table64 and continuation examples are accurate; no WASI/JIT/JS functionality is advertised.

Add focused tests for the changed behavior and relevant failure boundaries. Record the exact command, feature profile, corpus/source revision and executed assertion counts. Follow [the common definition of done](../EXECUTION.md#common-definition-of-done); a missing directive, zero-test run or unimplemented branch is not a pass.

## Boundaries

Follow [ARCHITECTURE.md](../ARCHITECTURE.md): no external core production dependency, no per-proposal Cargo-crate proliferation, no JS/browser implementation, and no invented semantics for draft TODOs. Coordinate changes to shared types/opcode/control contracts with other ready tasks. Runtime feature gates and implementation resource limits must remain explicit.

## Implementation instructions for Sol 6.1 Low

Read [the interface contracts](../IMPLEMENTATION_CONTRACTS.md) and the listed prerequisite outputs before coding. Use the predecessor's API; do not invent a second implementation to avoid a missing prerequisite.

### Files and ownership

Expected paths in `/workspace/wtime` (created by implementation, not by this plan):

- `README.md`
- `docs/embedding.md`
- `examples/`

This is a contract/documentation task: its acceptance checklist replaces an invented runtime test. Link concrete source rules and make the output usable by successor tasks.

### Ordered recipe

1. Write runnable examples for Engine/Store/imports, resource policy and reference ownership.
2. Add core feature matrix/pins and examples for memory64/table64 and continuations.
3. Test examples against current public APIs without pulling test tools into core dependencies.
4. Add the cases below before considering the behavior finished. Keep malformed, invalid, trapping, host-error and resource-limit outcomes distinct.
5. Wire the change into the existing entry point/dispatch and update its feature-policy or ownership evidence where relevant. Run the commands below and record exact outcomes.

### Required cases

- `t168_case_01`: Embedding example compiles/runs
- `t168_case_02`: No implied WASI/JS/JIT support
- `t168_case_03`: Incorrect root ownership not shown in docs.

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

- Changes: Embedding documentation covers module-bound hosts/tags, limits, roots, exceptions, suspension and concurrency. Original memory64/table64 and continuation examples are runnable; pinned scope excludes JS/browser/WASI/JIT.
- Commands and outcomes: Four example invocations pass (42, suspension7->9, memory42, table1). Document and fixture hashes recorded.
- Evidence: `/workspace/wtime/docs/verification/api-docs.json`.
- Follow-up or normative ambiguity: No additional behavior claimed beyond the pinned core scope; final release also requires native aarch64 evidence.
