# T166 — Audit panic, allocation, handle and thread failure paths

Status: **in_progress**  
Area: `core`  
Work unit: one focused behavior/contract and its tests; split into child tasks before exceeding two focused patches.

## Prerequisites

[T164 — Fuzz parser and validator with reproducible mutations](../14-integration-and-release/T164-fuzz-decode-validate.md), [T165 — Compare deterministic semantics to independent references](../14-integration-and-release/T165-differential-execution.md), [T163 — Exercise proposal interaction scenarios](../14-integration-and-release/T163-interaction-tests.md), [T136 — Pass Threads and combined-memory conformance](../10-threads-and-atomics/T136-threads-gate.md), [T147 — Pass stack-switching conformance and lifecycle stress](../11-stack-switching/T147-cont-gate.md), [T157 — Pass custom-descriptor core conformance](../12-custom-descriptors/T157-desc-gate.md)

Use the predecessor's interfaces and evidence. Do not reimplement a second parser, type registry, store or execution stack.

## Deliverable

Audit resource/cancellation propagation, stale handles, generation exhaustion, root safepoints, lock ordering, host reentry and fallible large allocations. Run bounded stress/Miri where supported; justify every remaining unsafe block. Rerun mutation/differential harnesses on the complete all_phase3_core profile after T163, and verify native x86_64/aarch64 builds and semantic smoke cases in CI.

## Acceptance

Each reviewed failure path has a reproducer/test; no safe public API can create an invalid handle, Rust data race or unrooted live reference. Record final complete-profile results and host-target results separately; a cross-compile alone does not count as native execution.

Add focused tests for the changed behavior and relevant failure boundaries. Record the exact command, feature profile, corpus/source revision and executed assertion counts. Follow [the common definition of done](../EXECUTION.md#common-definition-of-done); a missing directive, zero-test run or unimplemented branch is not a pass.

## Boundaries

Follow [ARCHITECTURE.md](../ARCHITECTURE.md): no external core production dependency, no per-proposal Cargo-crate proliferation, no JS/browser implementation, and no invented semantics for draft TODOs. Coordinate changes to shared types/opcode/control contracts with other ready tasks. Runtime feature gates and implementation resource limits must remain explicit.

## Implementation instructions for Sol 6.1 Low

Read [the interface contracts](../IMPLEMENTATION_CONTRACTS.md) and the listed prerequisite outputs before coding. Use the predecessor's API; do not invent a second implementation to avoid a missing prerequisite.

### Files and ownership

Expected paths in `/workspace/wtime` (created by implementation, not by this plan):

- `docs/architecture/hardening.md`
- `tools/checks/hardening.py`

Add stdlib-only checker tests under `tools/checks/tests/test_runtime_hardening.py`, including an intentionally failing input. Do not test a checker solely against its own happy-path output.

### Ordered recipe

1. Review all public handles, borrow/safepoint boundaries, overflow/alloc errors and lock/state transitions against contracts.
2. Rerun complete-profile fuzz/differential/stress and native target smoke cases.
3. Document every remaining unsafe block or unresolved resource failure with a focused follow-up.
4. Add the cases below before considering the behavior finished. Keep malformed, invalid, trapping, host-error and resource-limit outcomes distinct.
5. Wire the change into the existing entry point/dispatch and update its feature-policy or ownership evidence where relevant. Run the commands below and record exact outcomes.

### Required cases

- `t166_case_01`: Stale handle and cancel cleanup
- `t166_case_02`: Failed allocation during construction
- `t166_case_03`: Native x86_64/aarch64 smoke evidence.

For binary fixtures, keep a readable WAT/hex source and provenance alongside the `.wasm` or inline bytes. Generate fixtures with the separate encoder/wasm-tools; do not add `wast` to core tests. Expected outputs must come from the locked spec/rule or an independent calculation, not the engine being tested.

### Verification commands

Run from `/workspace/wtime` after activating the configured Rust environment:

```sh
. /workspace/.setup/rust-env.sh
python3 -m unittest discover -s tools/checks/tests -p 'test_runtime_hardening.py' -v
git diff --check
```

### Handoff and scope limit

Record changed symbols/files, named test counts, corpus hash/profile if used, and remaining risks in Completion evidence. Stop at this task's interface; do not implement unrelated future features. If a prerequisite API is missing or pinned sources conflict, identify the exact missing contract/rule and keep the affected work blocked instead of guessing. Routine local coding choices do not require asking the user.

## Sources

- [Frozen proposal scope](../PROPOSALS.md) and [architecture](../ARCHITECTURE.md).
- [Pinned integrated core specification](https://github.com/WebAssembly/spec/tree/7e9fb78c63d65018cc111422f657947b3df80d7b) is normative for finished core features; historical proposal repositories provide context.
- [Scope, source precedence and exclusions](../PROPOSALS.md).

## Completion evidence

- Changes: Safe generation handles, bounded allocation/mark/capture, rooted reference lifetimes, transactional private construction, shared-memory synchronization, cancellation and failure cleanup audited. Actual allocator failure runs in a constrained separate process.
- Commands and outcomes: Runtime native x86_64 134 tests pass. Full-profile fuzz 1000 inputs has zero failures; differential 100 vectors has zero differences (80 Node comparisons, 20 explicit capability gaps). Allocation worker forces 512 MiB failure under 128 MiB address-space limit, then successfully creates/accesses one-page memory under a one-object budget.
- Evidence: `/workspace/wtime/docs/verification/native-x86_64.json`, `allocation-failure.json`, `fuzz.json`, `differential.json`; final source bindings match frozen core SHA e5a3a4eeba54a547ec8102acb6d03bb823de92643b7b34fc435817fcf0a3c75f.
- External blocker: Native aarch64 execution is unavailable in this x86_64 workspace. `.github/workflows/ci.yml` configures native Ubuntu ARM tests and compares source digests, but an unrun job is not evidence. Run the native hardening producer on aarch64 and provide its actual report before completing T166/T169.
