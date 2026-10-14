# P114 — Verify bounded straight-line prefix and exit certificates

Status: **done**

Outcome: **contract_only**

Kind: `foundation` · Branch: `portable` · Model: **gpt-6.1-sol low**

## Prerequisites

[P113 — Add integration-prefix benchmark and paired baseline support](P113-integration-prefix-benchmark-foundation.md)
[P037 — Map compact instructions to source events and control targets](P037-compact-source-maps.md)
[P043 — Resolve memory bindings and specialize checked scalar accesses](P043-compact-memory-plans.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C5, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

Owned paths:

- `crates/wtime-runtime/src/execution/compact/prefix.rs`
- `crates/wtime-runtime/tests/performance/p114_private.rs`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

A private bounded certificate admits a straight-line numeric/local/numeric-call prefix over canonical Values, records exact original source operations and one exit before any control, memory or complex operation, and publishes atomically or declines.

## Implementation recipe

1. Read current CompactFunction/SourceMap/effect certificate and prepared instruction APIs; add the leaf prefix module without broadening compact whole-function control certificates.
2. Represent prefix source identity, engine identity, bounded operation sequence, canonical local/ref admission, exact original exit (Instruction, pc), and work/capacity accounting. Do not add a whole EH/prompt CFG certificate.
3. Refuse malformed source/op/engine identity, over-limit work/capacity, unsupported references/effects and any exit that cannot resolve through validated prepared source. Reserve and validate before single publication.
4. Make prefix boundary an explicit one-way exit: a branch-over-barrier runs the decoded suffix; no re-entry to compact occurs after the first barrier.
5. Keep all internals private and add corruption constructors only in private tests.

## Meaningful cases

- Corrupt source identity, operation sequence and engine identity are rejected.
- Work and actual capacity limits refuse safely with no partial publication, including checked arithmetic overflow.
- Exit is before control/memory/complex operation; decoded suffix remains the sole path after exit.

## Verification

Use the shared verifier. Planning and development receipts are task-local; no command has been run for this task. Full acceptance is contract-only and makes no independent speed claim.

`python3 -B tools/verify.py plan --contract docs/benchmarks/performance/P114-verification.json`
`python3 -B tools/verify.py dev --contract docs/benchmarks/performance/P114-verification.json --group certificate --execute`

## Measurement and decision

Selection: `integration`. Explicit execution setting: `compact`. Decision rule: `integrity`. The deliverable is a bounded foundation for P044; it does not establish a runtime speedup or authorize promotion. Preserve exact fixture/source identities and executed named-case counts.

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass. Do not fabricate run results.

Update this file and TASKS.json with evidence only after execution. Executed evidence is recorded below.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/P114/shared-finalization/attempt-0001/evidence.json": "f9c61ec1a4c1b4774b30241f888864bd47668ebbcbb78d718a9ddf28c4d41c42"
  },
  "evidence": [
    {
      "bytes": 1096,
      "path": "/workspace/wtime/target/performance/P114/shared-finalization/attempt-0001/evidence.json",
      "sha256": "f9c61ec1a4c1b4774b30241f888864bd47668ebbcbb78d718a9ddf28c4d41c42"
    }
  ],
  "outcome": "contract_only",
  "status": "done",
  "task": "P114"
}
```
