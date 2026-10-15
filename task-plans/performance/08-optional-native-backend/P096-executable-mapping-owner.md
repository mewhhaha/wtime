# P096 — Implement one audited W^X mapping owner for x86 Linux

Status: **deferred**

Outcome: **not_selected**

Kind: `foundation` · Branch: `optional` · Model: **Sol 6.1 Low**

## Prerequisites

[P094 — Decide the tiny native backend boundary and budget](P094-native-boundary-decision.md)

[P095 — Create the minimal optional numeric backend interface](P095-native-ir-and-abi.md)

Conditional branch: `P094.outcome == go`. If its decision is declined, record this task as deferred/not_selected; do not claim it implemented.

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-native/src/platform/linux.rs`
- `crates/wtime-native/src/code_memory.rs`
- `crates/wtime-native/tests/code_memory.rs`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

Private CodeMemory owns fallible RW allocation -> bounded patch -> RX seal -> lifetime-safe execute -> unmap. It is never RWX, executable bytes are not caller supplied, and unsupported targets decline.

## Implementation recipe

1. Implement the smallest required Linux mapping/protection/unmap boundary with checked sizes/page rounding and documented unsafe preconditions; follow the ADR dependency cap.
2. Expose writable access only before sealing and no mutation after RX publication; retain an owned handle during execution and release mapping on every failure/drop.
3. Account mapped/code bytes before publication and support injected platform failures; omit signal handlers and guard-page trapping.
4. Execute one locally generated constant-return stub through the private checked ABI on native x86_64 Linux, record actual target evidence and leave other targets unsupported.

## Meaningful cases

- Permissions are RW during creation and RX during execution, never RWX.
- Allocation/protection/seal failure releases ownership/bytes once.
- Dropping cache ownership cannot unmap code held by an active execution handle.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-native code_memory -- --nocapture
python3 tools/checks/dependency_budget.py
```

## Measurement and decision

Selection: `scalar`. Explicit execution setting: `native`. Decision rule: `integrity`.

This is a tooling/contract deliverable. Require correct nonzero checks and bounded/source-bound output; do not invent a runtime speedup requirement. If it changes production work, record cold/build/retention deltas under [MEASUREMENT.md](../MEASUREMENT.md).

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. Executed evidence is recorded below.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/final-optional-dispositions-0002/proof.json": "5a2943b1b7f0e4fedf44a2d4e96153937683fc97d45e079e55c0ef43a5e3b97f"
  },
  "evidence": [
    {
      "bytes": 38990,
      "path": "/workspace/wtime/target/performance/final-optional-dispositions-0002/proof.json",
      "sha256": "5a2943b1b7f0e4fedf44a2d4e96153937683fc97d45e079e55c0ef43a5e3b97f"
    }
  ],
  "implementation_claimed": false,
  "outcome": "not_selected",
  "reason": "Conditional research is not selected for the current portfolio; this record claims no implementation, test execution, or speed result.",
  "schema": "wtime-task-disposition-record-v1",
  "status": "deferred",
  "task": "P096",
  "tests_claimed": 0,
  "timing_evidence": false
}
```
