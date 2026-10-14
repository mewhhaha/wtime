# P118 — Cover suspend and bind transfer from compact prefixes

Status: **done**

Outcome: **contract_only**

Kind: `foundation` · Branch: `portable` · Model: **gpt-6.1-sol low**

## Prerequisites

[P115 — Integrate canonical handoff and reference-local admission](P115-canonical-prefix-handoff.md)
[P022 — Add adversarial state and interaction tier fixtures](../01-execution-contracts/P022-state-differential-fixtures.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C5, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

Owned paths:

- `crates/wtime-runtime/tests/performance/p118.rs`
- `crates/wtime-runtime/tests/fixtures/performance/compact/`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

Tests prove exact contexts, roots, continuation bytes and one-shot failure cleanup across suspend/bind barriers after compact prefixes, using current semantic helpers. Rejected bind validation before mutation preserves the original ready capability; successful bind/resume consumes it, and completed capabilities cannot be reused.

## Implementation recipe

1. Build on the cross-module suspend/bind P022 cases, extending only the state needed to prove an active prefix and one-way exit; retain P118's own validated packet fixture.
2. Compare exact function/type/module contexts, pc, roots and continuation-byte accounting at suspend and bind.
3. Exercise host failure cleanup and rejected bind before mutation. A rejected bind preserves the original ready capability, roots, and byte accounting; successful bind/resume consumes it exactly once, and completed capabilities cannot be reused. Host failure cleanup releases provisional bytes/roots once without consuming a capability before a successful bind. Ensure no compact instruction reference escapes without retained immutable payload.
4. Assert positive prefix count and identical decoded/reference result, error and cleanup.

## Meaningful cases

- Cross-module suspend preserves exact contexts, pc and roots.
- Bind/resume preserves byte accounting and canonical refs.
- Rejected bind before mutation preserves the original ready capability and byte accounting; successful bind/resume consumes it, and completed capabilities reject reuse.
- Host failure cleanup releases provisional bytes/roots once; rejection before bind mutation preserves the original ready capability.

## Verification

Use the shared verifier. Planning and development receipts are task-local; no command has been run for this task. Full acceptance is contract-only and makes no independent speed claim.

`python3 -B tools/verify.py plan --contract docs/benchmarks/performance/P118-verification.json`
`python3 -B tools/verify.py dev --contract docs/benchmarks/performance/P118-verification.json --group suspend-bind --execute`

## Measurement and decision

Selection: `integration`. Explicit execution setting: `compact`. Decision rule: `integrity`. The deliverable is a bounded foundation for P044; it does not establish a runtime speedup or authorize promotion. Preserve exact fixture/source identities and executed named-case counts.

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass. Do not fabricate run results.

Update this file and TASKS.json with evidence only after execution. Executed evidence is recorded below.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/P118/shared-finalization/attempt-0001/evidence.json": "d75ae50c5ad8610f801f3ce244fdb08a15f4efd72bb1ead684134b0348a0d0a8"
  },
  "evidence": [
    {
      "bytes": 1077,
      "path": "/workspace/wtime/target/performance/P118/shared-finalization/attempt-0001/evidence.json",
      "sha256": "d75ae50c5ad8610f801f3ce244fdb08a15f4efd72bb1ead684134b0348a0d0a8"
    }
  ],
  "outcome": "contract_only",
  "status": "done",
  "task": "P118"
}
```
