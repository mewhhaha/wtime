# T167 — Meet compile-time and dependency budgets

Status: **done**  
Area: `tooling`  
Work unit: one focused behavior/contract and its tests; split into child tasks before exceeding two focused patches.

## Prerequisites

[T166 — Audit panic, allocation, handle and thread failure paths](../14-integration-and-release/T166-runtime-hardening.md), [T009 — Measure build time and binary size](../00-foundation/T009-build-baseline.md), [T010 — Enforce the dependency and unsafe-code budget](../00-foundation/T010-dependency-policy.md)

Use the predecessor's interfaces and evidence. Do not reimplement a second parser, type registry, store or execution stack.

## Deliverable

Measure the full core/CLI after all proposals: clean and incremental times, RSS, binary size, dependency trees and codegen hotspots. Reduce unnecessary generics, cross-crate inlining and shared-type churn; record any budget adjustments transparently.

## Acceptance

Build numbers use the baseline protocol; default core still excludes tooling/proc-macro/oracle deps. Unmet budgets remain explicit optimization tasks, not unmeasured claims.

Add focused tests for the changed behavior and relevant failure boundaries. Record the exact command, feature profile, corpus/source revision and executed assertion counts. Follow [the common definition of done](../EXECUTION.md#common-definition-of-done); a missing directive, zero-test run or unimplemented branch is not a pass.

## Boundaries

Follow [ARCHITECTURE.md](../ARCHITECTURE.md): no external core production dependency, no per-proposal Cargo-crate proliferation, no JS/browser implementation, and no invented semantics for draft TODOs. Coordinate changes to shared types/opcode/control contracts with other ready tasks. Runtime feature gates and implementation resource limits must remain explicit.

## Implementation instructions for Sol 6.1 Low

Read [the interface contracts](../IMPLEMENTATION_CONTRACTS.md) and the listed prerequisite outputs before coding. Use the predecessor's API; do not invent a second implementation to avoid a missing prerequisite.

### Files and ownership

Expected paths in `/workspace/wtime` (created by implementation, not by this plan):

- `tools/checks/build_budget.py`
- `docs/architecture/build-final.json`

Add stdlib-only checker tests under `tools/checks/tests/test_fast_build_gate.py`, including an intentionally failing input. Do not test a checker solely against its own happy-path output.

### Ordered recipe

1. Run baseline measurement protocol on final default core/CLI graph with clean and warm caches distinguished.
2. Inspect generic/codegen/fan-out hot spots before changing crate boundaries.
3. Record dependency tree, binary size, peak RSS and any budget exception with rationale.
4. Add the cases below before considering the behavior finished. Keep malformed, invalid, trapping, host-error and resource-limit outcomes distinct.
5. Wire the change into the existing entry point/dispatch and update its feature-policy or ownership evidence where relevant. Run the commands below and record exact outcomes.

### Required cases

- `t167_case_01`: No unexpected production dependency
- `t167_case_02`: Touched leaf timing reproduced
- `t167_case_03`: Failed budget cannot be called pass.

For binary fixtures, keep a readable WAT/hex source and provenance alongside the `.wasm` or inline bytes. Generate fixtures with the separate encoder/wasm-tools; do not add `wast` to core tests. Expected outputs must come from the locked spec/rule or an independent calculation, not the engine being tested.

### Verification commands

Run from `/workspace/wtime` after activating the configured Rust environment:

```sh
. /workspace/.setup/rust-env.sh
python3 -m unittest discover -s tools/checks/tests -p 'test_fast_build_gate.py' -v
git diff --check
```

### Handoff and scope limit

Record changed symbols/files, named test counts, corpus hash/profile if used, and remaining risks in Completion evidence. Stop at this task's interface; do not implement unrelated future features. If a prerequisite API is missing or pinned sources conflict, identify the exact missing contract/rule and keep the affected work blocked instead of guessing. Routine local coding choices do not require asking the user.

## Sources

- [Frozen proposal scope](../PROPOSALS.md) and [architecture](../ARCHITECTURE.md).
- [Pinned integrated core specification](https://github.com/WebAssembly/spec/tree/7e9fb78c63d65018cc111422f657947b3df80d7b) is normative for finished core features; historical proposal repositories provide context.
- [Scope, source precedence and exclusions](../PROPOSALS.md).

## Completion evidence

- Changes: Zero external production dependencies and unsafe Rust; isolated build budgets measured on stable source with five jobs.
- Commands and outcomes: Clean 6.78s; unchanged 0.10s; leaf edit 0.26s; stripped CLI 1002024 bytes; all budgets pass.
- Evidence: `/workspace/wtime/docs/verification/build-final.json`; `/workspace/wtime/docs/verification/dependency-budget.txt`.
- Follow-up or normative ambiguity: No additional behavior claimed beyond the pinned core scope; final release also requires native aarch64 evidence.
