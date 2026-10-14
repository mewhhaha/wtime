# T165 — Compare deterministic semantics to independent references

Status: **done**  
Area: `tooling`  
Work unit: one focused behavior/contract and its tests; split into child tasks before exceeding two focused patches.

## Prerequisites

[T058 — Pass the complete selected MVP corpus](../04-mvp-execution/T058-mvp-gate.md)

Use the predecessor's interfaces and evidence. Do not reimplement a second parser, type registry, store or execution stack.

## Deliverable

Run generated small valid programs against a pinned external spec interpreter or other independent engine; leave the oracle outside core/default builds. Compare exact integer/ref outcomes and permitted float/relaxed result sets; use standalone expected vectors for proposals the oracle lacks. Begin with the completed MVP rather than waiting for all proposals. Extend generators and reference coverage incrementally; no oracle is linked into the runtime.

## Acceptance

Wrong result injection is detected; oracle capability gaps are reported separately. Samples cover state mutation/traps, not just add constants. T166 reruns the expanded comparison set after the full feature interaction gate.

Add focused tests for the changed behavior and relevant failure boundaries. Record the exact command, feature profile, corpus/source revision and executed assertion counts. Follow [the common definition of done](../EXECUTION.md#common-definition-of-done); a missing directive, zero-test run or unimplemented branch is not a pass.

## Boundaries

Follow [ARCHITECTURE.md](../ARCHITECTURE.md): no external core production dependency, no per-proposal Cargo-crate proliferation, no JS/browser implementation, and no invented semantics for draft TODOs. Coordinate changes to shared types/opcode/control contracts with other ready tasks. Runtime feature gates and implementation resource limits must remain explicit.

## Implementation instructions for Sol 6.1 Low

Read [the interface contracts](../IMPLEMENTATION_CONTRACTS.md) and the listed prerequisite outputs before coding. Use the predecessor's API; do not invent a second implementation to avoid a missing prerequisite.

### Files and ownership

Expected paths in `/workspace/wtime` (created by implementation, not by this plan):

- `tools/checks/differential.py`

Add stdlib-only checker tests under `tools/checks/tests/test_differential_execution.py`, including an intentionally failing input. Do not test a checker solely against its own happy-path output.

### Ordered recipe

1. Choose a pinned external oracle with documented capability profile and input/output protocol.
2. Generate small valid stateful programs and compare permitted results/traps.
3. Begin at MVP; add proposal vectors without using the oracle to define uncertain semantics.
4. Add the cases below before considering the behavior finished. Keep malformed, invalid, trapping, host-error and resource-limit outcomes distinct.
5. Wire the change into the existing entry point/dispatch and update its feature-policy or ownership evidence where relevant. Run the commands below and record exact outcomes.

### Required cases

- `t165_case_01`: Injected wrong result detected
- `t165_case_02`: NaN/relaxed allowed sets
- `t165_case_03`: Memory/table side effects compared.

For binary fixtures, keep a readable WAT/hex source and provenance alongside the `.wasm` or inline bytes. Generate fixtures with the separate encoder/wasm-tools; do not add `wast` to core tests. Expected outputs must come from the locked spec/rule or an independent calculation, not the engine being tested.

### Verification commands

Run from `/workspace/wtime` after activating the configured Rust environment:

```sh
. /workspace/.setup/rust-env.sh
python3 -m unittest discover -s tools/checks/tests -p 'test_differential_execution.py' -v
git diff --check
```

### Handoff and scope limit

Record changed symbols/files, named test counts, corpus hash/profile if used, and remaining risks in Completion evidence. Stop at this task's interface; do not implement unrelated future features. If a prerequisite API is missing or pinned sources conflict, identify the exact missing contract/rule and keep the affected work blocked instead of guessing. Routine local coding choices do not require asking the user.

## Sources

- [Frozen proposal scope](../PROPOSALS.md) and [architecture](../ARCHITECTURE.md).
- [Pinned integrated core specification](https://github.com/WebAssembly/spec/tree/7e9fb78c63d65018cc111422f657947b3df80d7b) is normative for finished core features; historical proposal repositories provide context.
- [Scope, source precedence and exclusions](../PROPOSALS.md).

## Completion evidence

- Changes: Differential vectors exercise arithmetic, memory/table mutation, traps, SIMD and wide/custom-page cases with independent expected results.
- Commands and outcomes: 100 vectors, 80 Node comparisons, 20 explicitly unsupported by Node, 0 differences. Wrong-result injection checker passes.
- Evidence: `/workspace/wtime/docs/verification/differential.json`.
- Follow-up or normative ambiguity: No additional behavior claimed beyond the pinned core scope; final release also requires native aarch64 evidence.
