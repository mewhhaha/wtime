# P115 — Integrate canonical handoff and reference-local admission

Status: **done**

Outcome: **contract_only**

Kind: `foundation` · Branch: `portable` · Model: **gpt-6.1-sol low**

## Prerequisites

[P114 — Verify bounded straight-line prefix and exit certificates](P114-prefix-exit-certificate.md)
[P039 — Lower float/vector constants and barrier adapters](P039-compact-float-vector-adapters.md)
[P041 — Resolve direct guest call bindings once per instance](P041-instance-call-plans.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C5, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

Owned paths:

- `crates/wtime-runtime/src/execution/compact/`
- `crates/wtime-runtime/tests/performance/p115.rs`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

PreparedModule may carry bounded eligible prefixes. At the already-charged original boundary, compact state is cleared and the original prepared (Instruction, pc) is recovered once via prepared_instruction before one-way decoded execution; validated ref parameters/locals are admitted only through canonical Values and root-aware state.

## Implementation recipe

1. Trace current compact dispatch, frame lifecycle, fuel charging and prepared_instruction lookup. Keep cross-cutting dispatch edits with the integrator.
2. Attach immutable admitted prefix plans to PreparedModule with bounded aggregate work/bytes; unsupported functions decline without guest work. Admit reference params/locals only when canonical Value conversion and root visibility are validated.
3. At the first exit, clear frame.compact after the boundary is charged, resolve original (Instruction, pc) using prepared_instruction exactly once, materialize exact decoded operands/locals/fuel, and continue decoded without compact re-entry.
4. Keep branch-over-barrier semantics explicit: compact prefix falls through to decoded suffix, including the branch instruction and subsequent original control.
5. Instrument actual optimized prefix entry and fallback/exit reason for diagnostics; preserve the default Decoded policy.

## Meaningful cases

- Fuel at the boundary is charged exactly once and decoded suffix resumes at the original pc.
- A side effect immediately after exit occurs once, with exact state and no compact re-entry.
- A branch over a barrier executes decoded suffix/control and matches reference state/fuel.

## Verification

Use the shared verifier. Planning and development receipts are task-local; no command has been run for this task. Full acceptance is contract-only and makes no independent speed claim.

`python3 -B tools/verify.py plan --contract docs/benchmarks/performance/P115-verification.json`
`python3 -B tools/verify.py dev --contract docs/benchmarks/performance/P115-verification.json --group handoff --execute`

## Measurement and decision

Selection: `integration`. Explicit execution setting: `compact`. Decision rule: `integrity`. The deliverable is a bounded foundation for P044; it does not establish a runtime speedup or authorize promotion. Preserve exact fixture/source identities and executed named-case counts.

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass. Do not fabricate run results.

Update this file and TASKS.json with evidence only after execution. Executed evidence is recorded below.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/P115/shared-finalization/attempt-0001/evidence.json": "9d7aa4ba3477ead4db45ffcf2bcbf4425eebb0884827285a44cabdb29df1c1ea"
  },
  "evidence": [
    {
      "bytes": 1119,
      "path": "/workspace/wtime/target/performance/P115/shared-finalization/attempt-0001/evidence.json",
      "sha256": "9d7aa4ba3477ead4db45ffcf2bcbf4425eebb0884827285a44cabdb29df1c1ea"
    }
  ],
  "outcome": "contract_only",
  "status": "done",
  "task": "P115"
}
```
