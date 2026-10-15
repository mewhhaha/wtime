# P098 — Add checked branches and bounded native numeric loops

Status: **deferred**

Outcome: **not_selected**

Kind: `foundation` · Branch: `optional` · Model: **Sol 6.1 Low**

## Prerequisites

[P060 — Implement safe parallel moves and register control transfers](../05-register-executor/P060-register-edge-moves.md)

[P097 — Emit a tiny verified x86_64 integer block subset](P097-x86-integer-block-emitter.md)

Conditional branch: `P094.outcome == go`. If its decision is declined, record this task as deferred/not_selected; do not claim it implemented.

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-native/src/x86/control.rs`
- `crates/wtime-native/src/x86/encode.rs`
- `crates/wtime-native/tests/x86_control.rs`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

Native CFG emission adds verified conditional/unconditional edges and parallel slot moves; all jumps target generated instruction boundaries and exits return explicit source state.

## Implementation recipe

1. Emit basic blocks from verified CFG with fixed relocation widths, checked displacement arithmetic and no branches into immediates or external memory.
2. Implement parallel edge moves through the verified slot/scratch plan; keep original structural event/cursor metadata available for P099.
3. Bound emitted size/work and decline oversized/out-of-range graphs rather than adding a general branch-relaxation compiler.
4. Run loop/nested-branch/br_table-subset controls through native code and assert constant Rust/native stack depth independent of guest iterations.

## Meaningful cases

- Taken/untaken branches and swap/rotate edge moves preserve values.
- Relocation overflow/interior target corruption is rejected.
- Long numeric loops use bounded native stack and unsupported control returns portable fallback.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-native p098_ -- --nocapture
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
  "task": "P098",
  "tests_claimed": 0,
  "timing_evidence": false
}
```
