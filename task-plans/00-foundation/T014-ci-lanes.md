# T014 — Create focused validation and build lanes

Status: **done**  
Area: `tooling`  
Work unit: one focused behavior/contract and its tests; split into child tasks before exceeding two focused patches.

## Prerequisites

[T009 — Measure build time and binary size](../00-foundation/T009-build-baseline.md), [T010 — Enforce the dependency and unsafe-code budget](../00-foundation/T010-dependency-policy.md), [T012 — Implement spec-script parsing and outcome accounting](../00-foundation/T012-script-runner.md)

Use the predecessor's interfaces and evidence. Do not reimplement a second parser, type registry, store or execution stack.

## Deliverable

Define fast core check/unit-test, feature-specific conformance, full nightly corpus, build-budget and optional oracle lanes. Avoid rebuilding every feature permutation; use runtime feature profiles. Add Miri for safe core tests only where supported, as a separate tooling lane.

## Acceptance

Fast lane excludes wast/oracles; an intentional feature test failure makes its lane fail. CI reports skipped and unsupported tooling explicitly.

Add focused tests for the changed behavior and relevant failure boundaries. Record the exact command, feature profile, corpus/source revision and executed assertion counts. Follow [the common definition of done](../EXECUTION.md#common-definition-of-done); a missing directive, zero-test run or unimplemented branch is not a pass.

## Boundaries

Follow [ARCHITECTURE.md](../ARCHITECTURE.md): no external core production dependency, no per-proposal Cargo-crate proliferation, no JS/browser implementation, and no invented semantics for draft TODOs. Coordinate changes to shared types/opcode/control contracts with other ready tasks. Runtime feature gates and implementation resource limits must remain explicit.

## Implementation instructions for Sol 6.1 Low

Read [the interface contracts](../IMPLEMENTATION_CONTRACTS.md) and the listed prerequisite outputs before coding. Use the predecessor's API; do not invent a second implementation to avoid a missing prerequisite.

### Files and ownership

Expected paths in `/workspace/wtime` (created by implementation, not by this plan):

- `.github/workflows/ci.yml`
- `tools/checks/ci.sh`

Add stdlib-only checker tests under `tools/checks/tests/test_ci_lanes.py`, including an intentionally failing input. Do not test a checker solely against its own happy-path output.

### Ordered recipe

1. Create separate default-core, feature-corpus, full-corpus and build-budget commands/jobs.
2. Cache toolchain/dependencies by lock hashes, never by mutable branch names alone.
3. Upload failing fixture/report artifacts and require explicit positive assertion counts.
4. Add the cases below before considering the behavior finished. Keep malformed, invalid, trapping, host-error and resource-limit outcomes distinct.
5. Wire the change into the existing entry point/dispatch and update its feature-policy or ownership evidence where relevant. Run the commands below and record exact outcomes.

### Required cases

- `t014_case_01`: Core job does not compile wast
- `t014_case_02`: Feature failure propagates nonzero
- `t014_case_03`: Unknown/skipped required cases remain visible.

For binary fixtures, keep a readable WAT/hex source and provenance alongside the `.wasm` or inline bytes. Generate fixtures with the separate encoder/wasm-tools; do not add `wast` to core tests. Expected outputs must come from the locked spec/rule or an independent calculation, not the engine being tested.

### Verification commands

Run from `/workspace/wtime` after activating the configured Rust environment:

```sh
. /workspace/.setup/rust-env.sh
python3 -m unittest discover -s tools/checks/tests -p 'test_ci_lanes.py' -v
git diff --check
```

### Handoff and scope limit

Record changed symbols/files, named test counts, corpus hash/profile if used, and remaining risks in Completion evidence. Stop at this task's interface; do not implement unrelated future features. If a prerequisite API is missing or pinned sources conflict, identify the exact missing contract/rule and keep the affected work blocked instead of guessing. Routine local coding choices do not require asking the user.

## Sources

- [Frozen proposal scope](../PROPOSALS.md) and [architecture](../ARCHITECTURE.md).
- [Pinned integrated core specification](https://github.com/WebAssembly/spec/tree/7e9fb78c63d65018cc111422f657947b3df80d7b) is normative for finished core features; historical proposal repositories provide context.
- [Scope, source precedence and exclusions](../PROPOSALS.md).

## Completion evidence

- Changes: Separate core, focused/full conformance, budget, optional Miri/oracle lanes; native CI artifacts compare matching source digests.
- Commands and outcomes: Three lane checker tests pass including intentional exit failure and unknown/optional status.
- Assertion counts / fixture hashes: tools/checks/ci.sh; .github/workflows/ci.yml. Native aarch64 execution remains a T166 release prerequisite, not claimed run.
- Follow-up or normative ambiguity: No open implementation gap within this task; later integration gates remain separately tracked.
