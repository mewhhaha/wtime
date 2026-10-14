# P120 — Cover switch transfer and compact resource boundaries

Status: **done**

Outcome: **contract_only**

Kind: `foundation` · Branch: `portable` · Model: **gpt-6.1-sol low**

## Prerequisites

[P115 — Integrate canonical handoff and reference-local admission](P115-canonical-prefix-handoff.md)
[P116 — Cover GC locals, host reentry and root lifetime](P116-prefix-gc-reentry-tests.md)
[P117 — Cover exception transfer from compact source prefixes](P117-prefix-eh-transfer-tests.md)
[P118 — Cover suspend and bind transfer from compact prefixes](P118-prefix-suspend-bind-tests.md)
[P119 — Cover resume and resume_throw after compact exits](P119-prefix-resume-tests.md)
[P022 — Add adversarial state and interaction tier fixtures](../01-execution-contracts/P022-state-differential-fixtures.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C5, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

Owned paths:

- `crates/wtime-runtime/tests/performance/p120.rs`
- `crates/wtime-runtime/tests/fixtures/performance/compact/`
- `docs/benchmarks/performance/`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

The final interaction contract covers switch context transfer, stale/foreign refs, frame/byte boundary cleanup and explicitCompact across all eight P022 cases, with positive-path proof and fallback reporting.

## Implementation recipe

1. Integrate under the shared performance target only after serial P116–P119 registration changes; retain one owner for tests/performance.rs registration.
2. Exercise switch after a positive prefix and compare exact module/type/function context, pc, roots, fuel and cleanup with decoded execution.
3. Test stale/foreign references and frame/continuation-byte boundary refusal after prefix entry; verify failure happens at the same original source point and resources return to zero.
4. Run explicitCompact over all eight P022 interaction cases, prove at least one eligible positive path per intended family, and report fallback/decline reasons for unsupported cases.
5. Retain source/fixture hashes, executed counts and a concise integration result table; do not claim a speedup from these contract checks.

## Meaningful cases

- Switch transfers exact context and roots after prefix exit.
- Stale/foreign refs and frame/byte boundaries reject with decoded-equivalent source/fuel and complete cleanup.
- ExplicitCompact runs all eight P022 cases with positive compact coverage and visible fallbacks.

## Verification

Use the shared verifier. Planning and development receipts are task-local; no command has been run for this task. Full acceptance is contract-only and makes no independent speed claim.

`python3 -B tools/verify.py plan --contract docs/benchmarks/performance/P120-verification.json`
`python3 -B tools/verify.py dev --contract docs/benchmarks/performance/P120-verification.json --group integration-interactions --execute`

## Measurement and decision

Selection: `integration`. Explicit execution setting: `compact`. Decision rule: `integrity`. The deliverable is a bounded foundation for P044; it does not establish a runtime speedup or authorize promotion. Preserve exact fixture/source identities and executed named-case counts.

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass. Do not fabricate run results.

Update this file and TASKS.json with evidence only after execution. Executed evidence is recorded below.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/P120/shared-finalization/attempt-0001/evidence.json": "d809be4525e351101b1753f3ab39feb626bb08ce7b66560c23ac65a43d029215"
  },
  "evidence": [
    {
      "bytes": 1077,
      "path": "/workspace/wtime/target/performance/P120/shared-finalization/attempt-0001/evidence.json",
      "sha256": "d809be4525e351101b1753f3ab39feb626bb08ce7b66560c23ac65a43d029215"
    }
  ],
  "outcome": "contract_only",
  "status": "done",
  "task": "P120"
}
```
