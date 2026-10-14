# P117 — Cover exception transfer from compact source prefixes

Status: **done**

Outcome: **contract_only**

Kind: `foundation` · Branch: `portable` · Model: **gpt-6.1-sol low**

## Prerequisites

[P115 — Integrate canonical handoff and reference-local admission](P115-canonical-prefix-handoff.md)
[P022 — Add adversarial state and interaction tier fixtures](../01-execution-contracts/P022-state-differential-fixtures.md)
[P040 — Lower structured control to resolved jump and result plans](P040-compact-control-plans.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C5, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

Owned paths:

- `crates/wtime-runtime/tests/performance/p117.rs`
- `crates/wtime-runtime/tests/fixtures/performance/compact/`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

Tests prove exact source-prefix transfer into existing decoded EH semantics for try_table, throw, unhandled throw and nested unwind; no EH CFG is certified or newly optimized.

## Implementation recipe

1. Use validated try_table/throw fixtures and existing exception helper contracts; encode and validate fixture bytes before execution.
2. Trigger EH only after positive prefix work. Check charged throw/source pc and preserve exact logical frame/context during decoded transfer.
3. Cover caught and unhandled paths plus nested guest frames; compare result/trap, state digest, fuel and cleanup with decoded execution.
4. Retain original failing fixture drafts and report executed named case counts.

## Meaningful cases

- try_table catches a throw after a compact prefix with exact source/fuel.
- Unhandled throw leaves identical trap and frame cleanup.
- Nested unwind preserves caller state and does not re-enter compact execution.

## Verification

Use the shared verifier. Planning and development receipts are task-local; no command has been run for this task. Full acceptance is contract-only and makes no independent speed claim.

`python3 -B tools/verify.py plan --contract docs/benchmarks/performance/P117-verification.json`
`python3 -B tools/verify.py dev --contract docs/benchmarks/performance/P117-verification.json --group exception-transfer --execute`

## Measurement and decision

Selection: `integration`. Explicit execution setting: `compact`. Decision rule: `integrity`. The deliverable is a bounded foundation for P044; it does not establish a runtime speedup or authorize promotion. Preserve exact fixture/source identities and executed named-case counts.

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass. Do not fabricate run results.

Update this file and TASKS.json with evidence only after execution. Executed evidence is recorded below.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/P117/shared-finalization/attempt-0001/evidence.json": "169ef5455d4159da786ddc87e777543386d1eeda75928238a5ffeb07d8a19e28"
  },
  "evidence": [
    {
      "bytes": 1077,
      "path": "/workspace/wtime/target/performance/P117/shared-finalization/attempt-0001/evidence.json",
      "sha256": "169ef5455d4159da786ddc87e777543386d1eeda75928238a5ffeb07d8a19e28"
    }
  ],
  "outcome": "contract_only",
  "status": "done",
  "task": "P117"
}
```
