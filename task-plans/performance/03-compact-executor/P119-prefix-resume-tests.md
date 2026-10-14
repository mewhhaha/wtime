# P119 — Cover resume and resume_throw after compact exits

Status: **done**

Outcome: **contract_only**

Kind: `foundation` · Branch: `portable` · Model: **gpt-6.1-sol low**

## Prerequisites

[P115 — Integrate canonical handoff and reference-local admission](P115-canonical-prefix-handoff.md)
[P118 — Cover suspend and bind transfer from compact prefixes](P118-prefix-suspend-bind-tests.md)
[P022 — Add adversarial state and interaction tier fixtures](../01-execution-contracts/P022-state-differential-fixtures.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C5, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

Owned paths:

- `crates/wtime-runtime/src/continuation.rs` — bounded shared cleanup of saved resume fuel when resumed invocation termination abandons the segment.
- `crates/wtime-runtime/tests/performance/p119.rs`
- `crates/wtime-runtime/tests/fixtures/performance/compact/`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

Tests prove resume/resume_throw preserve exact source context, pc, roots, fuel and unwind state after the compact executor has handed off to decoded semantics. A bounded shared cleanup clears armed `Store.resume_fuel` when the resumed invocation terminates during initial exception unwind, including a catch targeting the function label that returns with empty frames; the rule applies equally to Decoded, Reference and Compact execution.

## Implementation recipe

1. Reuse P118 continuation fixtures and extend them to inspect the resumed source boundary.
2. Check resume and resume_throw context identity, original pc, root visibility and source fuel before and after resumed execution.
3. In the shared continuation path, clear armed `Store.resume_fuel` when the resumed invocation terminates during its initial exception unwind; apply the same cleanup when a catch targets the function label and returns with empty frames. Keep the cleanup bounded and identical for Decoded, Reference and Compact policies.
4. Regress the stale-fuel failure: after an unhandled exception consumes a captured segment and ends its invocation, the next independent invocation must reach consumed-handle validation without inheriting stale resume fuel. Also verify the empty-frame function-label catch cannot leave saved fuel armed.
5. Cover caught and unhandled resume_throw plus nested unwind; compare against decoded reference and assert no compact re-entry.
6. Check completion/failure byte and root release.

## Meaningful cases

- Resume returns the expected value with exact context/pc/root state and fuel.
- resume_throw is caught at the original handler with exact unwind and fuel.
- Unhandled resume_throw and nested unwind release continuation roots/bytes once.
- An unhandled exception ending the resumed invocation clears `Store.resume_fuel`, so the next independent invocation checks the consumed handle without stale fuel preempting it.
- A catch targeting the function label and returning with empty frames also clears saved resume fuel.

## Verification

Use the shared verifier. Planning and development receipts are task-local; no command has been run for this task. Full acceptance is contract-only and makes no independent speed claim.

`python3 -B tools/verify.py plan --contract docs/benchmarks/performance/P119-verification.json`
`python3 -B tools/verify.py dev --contract docs/benchmarks/performance/P119-verification.json --group resume --execute`

## Measurement and decision

Selection: `integration`. Explicit execution setting: `compact`. Decision rule: `integrity`. The deliverable is a bounded foundation for P044; it does not establish a runtime speedup or authorize promotion. Preserve exact fixture/source identities and executed named-case counts.

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass. Do not fabricate run results.

Update this file and TASKS.json with evidence only after execution. Executed evidence is recorded below.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/P119/shared-finalization/attempt-0001/evidence.json": "e54f1478abd30e998f4598d1d66d1356099a799e9fb2aa56bb2bb0dc94dafa87"
  },
  "evidence": [
    {
      "bytes": 1077,
      "path": "/workspace/wtime/target/performance/P119/shared-finalization/attempt-0001/evidence.json",
      "sha256": "e54f1478abd30e998f4598d1d66d1356099a799e9fb2aa56bb2bb0dc94dafa87"
    }
  ],
  "outcome": "contract_only",
  "status": "done",
  "task": "P119"
}
```
