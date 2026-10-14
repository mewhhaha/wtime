# P113 — Add integration-prefix benchmark and paired baseline support

Status: **done**

Outcome: **contract_only**

Kind: `foundation` · Branch: `portable` · Model: **gpt-6.1-sol low**

## Prerequisites

[P043 — Resolve memory bindings and specialize checked scalar accesses](P043-compact-memory-plans.md)
[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)
[P022 — Add adversarial state and interaction tier fixtures](../01-execution-contracts/P022-state-differential-fixtures.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C5, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

Owned paths:

- `tools/wtime-bench/`
- `tools/wtime-bench/tests/test_p113*.py`
- `tools/wtime-bench/fixtures/`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

A contract-only harness foundation adds toolmodel, native driver and paired adapter for six source-bound compact integration rows, with fixtures encoded and validated before the P044 baseline. It makes no standalone speed claim.

## Implementation recipe

1. Inspect the existing workload schema, native driver, paired adapter, comparison runner and fixture validator; extend their current abstractions rather than adding a parallel harness.
2. Add primary rows integration_gc_prefix, integration_eh_prefix and integration_cont_prefix, plus protected controls integration_numeric_control, integration_ref_entry_control and integration_cont_entry_control. Bind each row to exact module/input/result/state oracles and explicit compact/reference policy.
3. Keep integration_numeric_control on the existing whole-function Compact path and prove it executes no prefix-optimized steps. The ref-entry and continuation-entry controls must prove that no prefix-optimized steps execute. Do not report any of these control rows as optimized prefix work.
4. Use input n=32 and assert n+32 with wrapping arithmetic. GC and EH rows write the full memory word n+32. Continuation row records payload n+32, then outside timed region resumes with 7 and verifies word 7, deliberate double-resume trap and final bytes=0.
5. Encode the exact fixture bytes and validate them against the pinned proposal/tool configuration before any P044 baseline. Record hashes, validation commands, nonzero rows and expected state digests.
6. Keep the existing five paired stages and analysis schema; run baseline/candidate as distinct binaries with identical cases and retain all paired observations. Full verification builds production and runs six independent probes. Do not execute measurement as part of implementing this foundation until root explicitly schedules it.

## Meaningful cases

- GC prefix writes and reads the complete memory word and returns n+32.
- EH prefix writes the complete memory word and returns n+32 while transfer follows decoded semantics.
- Continuation payload is n+32; after timing resume returns 7, writes word 7, second resume traps, and continuation bytes return to zero.
- integration_numeric_control retains the existing whole-function Compact path while proving zero prefix-optimized steps; ref-entry and continuation-entry controls prove zero prefix-optimized steps.

## Verification

Use the shared verifier. Planning and development receipts are task-local; no command has been run for this task. Full acceptance is contract-only and makes no independent speed claim.

`python3 -B tools/verify.py plan --contract docs/benchmarks/performance/P113-verification.json`
`python3 -B tools/verify.py dev --contract docs/benchmarks/performance/P113-verification.json --group model --execute` — five named Python tests.
`python3 -B tools/verify.py dev --contract docs/benchmarks/performance/P113-verification.json --group adapter --execute` — four named Python tests.
`python3 -B tools/verify.py dev --contract docs/benchmarks/performance/P113-verification.json --group harness --execute` — locked incremental Cargo check.
`python3 -B tools/verify.py close --contract docs/benchmarks/performance/P113-verification.json --output target/performance/P113/finalization/attempt-0001 --execute` — full verification builds production and runs six independent probes.

## Measurement and decision

Selection: `integration`. Explicit execution setting: `compact`. Decision rule: `integrity`. The deliverable is a bounded foundation for P044; it does not establish a runtime speedup or authorize promotion. Preserve exact fixture/source identities and executed named-case counts.

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass. Do not fabricate run results.

Update this file and TASKS.json with evidence only after execution. Executed evidence is recorded below.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/P113/shared-finalization/attempt-0002/evidence.json": "b8c0d94adf6c6b1232bbe0730d8c23ab71cd000eff1727648d2ef8700b1da365"
  },
  "evidence": [
    {
      "bytes": 1108,
      "path": "/workspace/wtime/target/performance/P113/shared-finalization/attempt-0002/evidence.json",
      "sha256": "b8c0d94adf6c6b1232bbe0730d8c23ab71cd000eff1727648d2ef8700b1da365"
    }
  ],
  "outcome": "contract_only",
  "status": "done",
  "task": "P113"
}
```
