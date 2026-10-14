# T169 — Complete the proposal coverage and release audit

Status: **in_progress**  
Area: `documentation`  
Work unit: one focused behavior/contract and its tests; split into child tasks before exceeding two focused patches.

## Prerequisites

[T161 — Pass all active Phase 3–5 core proposal corpora](../14-integration-and-release/T161-phase3-gate.md), [T162 — Verify complete feature enable/disable behavior](../14-integration-and-release/T162-feature-profile-tests.md), [T166 — Audit panic, allocation, handle and thread failure paths](../14-integration-and-release/T166-runtime-hardening.md), [T167 — Meet compile-time and dependency budgets](../14-integration-and-release/T167-fast-build-gate.md), [T168 — Document embedding, feature scope and compatibility](../14-integration-and-release/T168-public-api-docs.md)

Use the predecessor's interfaces and evidence. Do not reimplement a second parser, type registry, store or execution stack.

## Deliverable

Reconcile every instruction, encoding, validation rule and relevant test file with implementation evidence. Record licenses, toolchain/MSRV, build budgets, security invariants and remaining source ambiguities. Version the supported snapshot and establish deliberate proposal refresh policy.

## Acceptance

Completion means no unspecified core claim, missing required task/test or unexplained failure; publish a reproducible readiness report and explicitly scoped limitations.

Add focused tests for the changed behavior and relevant failure boundaries. Record the exact command, feature profile, corpus/source revision and executed assertion counts. Follow [the common definition of done](../EXECUTION.md#common-definition-of-done); a missing directive, zero-test run or unimplemented branch is not a pass.

## Boundaries

Follow [ARCHITECTURE.md](../ARCHITECTURE.md): no external core production dependency, no per-proposal Cargo-crate proliferation, no JS/browser implementation, and no invented semantics for draft TODOs. Coordinate changes to shared types/opcode/control contracts with other ready tasks. Runtime feature gates and implementation resource limits must remain explicit.

## Implementation instructions for Sol 6.1 Low

Read [the interface contracts](../IMPLEMENTATION_CONTRACTS.md) and the listed prerequisite outputs before coding. Use the predecessor's API; do not invent a second implementation to avoid a missing prerequisite.

### Files and ownership

Expected paths in `/workspace/wtime` (created by implementation, not by this plan):

- `docs/release-readiness.md`
- `docs/architecture/release-manifest.json`

This is a contract/documentation task: its acceptance checklist replaces an invented runtime test. Link concrete source rules and make the output usable by successor tasks.

### Ordered recipe

1. Join task evidence, opcode/rule coverage, corpus reports, source locks and build budgets into a release manifest.
2. Check every required task is done and every required assertion executes; list undefined/excluded behavior precisely.
3. Verify licenses, documented target/toolchain support and reproducer commands.
4. Add the cases below before considering the behavior finished. Keep malformed, invalid, trapping, host-error and resource-limit outcomes distinct.
5. Wire the change into the existing entry point/dispatch and update its feature-policy or ownership evidence where relevant. Run the commands below and record exact outcomes.

### Required cases

- `t169_case_01`: Missing evidence blocks release
- `t169_case_02`: Source/test hashes match
- `t169_case_03`: No unsupported behavior claimed complete.

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

- Changes: Fail-closed release checker joins pinned sources/corpus, task dependency/status closure, opcode and formal-rule evidence, current source/executable fingerprints, test counts, licenses, excluded scope and measured budgets. Producer reports are indexed by SHA-256 in docs/architecture/release-manifest.json.
- Commands and outcomes: Frozen Rust workspace 304 passed; Python checker suite 58 passed; fmt and strict all-target/all-feature Clippy pass. Full frozen conformance: Core3 62968, Phase3 64297, historical MVP 18480, interaction corpus 15; zero failures/skips/unsupported. Historical compatibility60 and Phase3 overlay2 are explicit actual current-rule assertions.
- Evidence: `/workspace/wtime/docs/release-readiness.md`, `/workspace/wtime/docs/architecture/release-manifest.json`, `/workspace/wtime/docs/verification/`; opcode/rule completeness is checked separately with `python3 tools/checks/opcode_coverage.py --require-complete`.
- External blocker: T166 requires native aarch64 execution against the same frozen core source digest. This x86_64 workspace cannot produce that evidence. CI configures the native ARM lane, but it has not run here; release audit remains nonzero until an actual successful ARM report is supplied. No full release readiness is claimed.
