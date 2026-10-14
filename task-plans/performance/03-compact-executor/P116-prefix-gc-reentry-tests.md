# P116 — Cover GC locals, host reentry and root lifetime

Status: **done**

Outcome: **contract_only**

Kind: `foundation` · Branch: `portable` · Model: **gpt-6.1-sol low**

## Prerequisites

[P115 — Integrate canonical handoff and reference-local admission](P115-canonical-prefix-handoff.md)
[P022 — Add adversarial state and interaction tier fixtures](../01-execution-contracts/P022-state-differential-fixtures.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C5, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

Owned paths:

- `crates/wtime-runtime/tests/performance/p116.rs`
- `crates/wtime-runtime/tests/fixtures/performance/compact/`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

Tests prove compact-active root visibility and exact one-way handoff through collection and nested host invocation; tests and durable fixtures only, with no new runtime mechanism.

## Implementation recipe

1. Reuse P022 state digests and current host collection/reentry fixture helpers; reference P115's `gc-active-local` fixture and add only minimal encoded fixtures with provenance.
2. Keep a root only in a local while compact prefix is active, invoke numeric zero-argument hostcollect, and prove collection retains and later observes it.
3. Exercise nested invoke plus release, foreign/stale references, and low-fuel exit/failure cleanup. Compare roots, memory, handles and fuel against decoded reference.
4. Assert positive prefix execution before each handoff case and exact fallback/exit reason.

## Meaningful cases

- Root-only-local survives host reentrant collection while compact is active.
- Numeric zero-argument hostcollect and nested invoke preserve state and return values.
- Released, foreign and stale refs plus low-fuel exit fail/clean up identically to decoded execution.

## Verification

Use the shared verifier. Planning and development receipts are task-local; no command has been run for this task. Full acceptance is contract-only and makes no independent speed claim.

`python3 -B tools/verify.py plan --contract docs/benchmarks/performance/P116-verification.json`
`python3 -B tools/verify.py dev --contract docs/benchmarks/performance/P116-verification.json --group gc-reentry --execute`

## Measurement and decision

Selection: `integration`. Explicit execution setting: `compact`. Decision rule: `integrity`. The deliverable is a bounded foundation for P044; it does not establish a runtime speedup or authorize promotion. Preserve exact fixture/source identities and executed named-case counts.

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass. Do not fabricate run results.

Update this file and TASKS.json with evidence only after execution. Executed evidence is recorded below.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/P116/shared-finalization/attempt-0001/evidence.json": "c8829d54600d03a08937c6118ec7dc0f62b4e6552fd53e9e6636a98d679366d5"
  },
  "evidence": [
    {
      "bytes": 1069,
      "path": "/workspace/wtime/target/performance/P116/shared-finalization/attempt-0001/evidence.json",
      "sha256": "c8829d54600d03a08937c6118ec7dc0f62b4e6552fd53e9e6636a98d679366d5"
    }
  ],
  "outcome": "contract_only",
  "status": "done",
  "task": "P116"
}
```
