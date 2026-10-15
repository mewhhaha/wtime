# P095 — Create the minimal optional numeric backend interface

Status: **deferred**

Outcome: **not_selected**

Kind: `foundation` · Branch: `optional` · Model: **Sol 6.1 Low**

## Prerequisites

[P057 — Define a small numeric register bytecode and eligibility rules](../05-register-executor/P057-register-ir-contract.md)

[P058 — Translate numeric stack code into direct slot operands](../05-register-executor/P058-stack-to-register-lowering.md)

[P060 — Implement safe parallel moves and register control transfers](../05-register-executor/P060-register-edge-moves.md)

[P094 — Decide the tiny native backend boundary and budget](P094-native-boundary-decision.md)

Conditional branch: `P094.outcome == go`. If its decision is declined, record this task as deferred/not_selected; do not claim it implemented.

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-exec-ir/Cargo.toml`
- `crates/wtime-exec-ir/src/lib.rs`
- `crates/wtime-native/Cargo.toml`
- `crates/wtime-native/src/lib.rs`
- `crates/wtime-runtime/Cargo.toml`
- `crates/wtime-runtime/src/execution/native.rs`
- `Cargo.toml`
- `crates/wtime-exec-ir/tests/verify.rs`
- `crates/wtime-runtime/tests/performance/p095.rs`
- `tools/checks/ci.sh`
- `tools/wtime-bench/Cargo.toml`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

VerifiedNumericProgram is bounded verified lowered IR, not a replacement Wasm validator. NativeProgram runs borrowed initialized numeric slots and returns explicit source Exit; immutable artifact binding preserves normal sealed Module provenance.

## Implementation recipe

1. Create the approved two tiny crates with optional runtime feature wiring and no dependency cycle; expose no Store/GC/raw-reference representation in native IR.
2. Define safe borrowed NumericState/AccountingState/Exit types plus read-only source-event/control metadata, and an IR verifier for slot bounds/targets/origins/work limits.
3. Translate only admitted eligible register programs into IR, bind artifacts to Module/policy identity internally and decline unsupported operations before execution.
4. Keep ABI raw pointers/private repr(C) wrappers confined to wtime-native; the runtime calls a safe checked execution method and never constructs executable mappings.
5. Change the core CI lane from cargo test --workspace to explicit original core/CLI package tests, so newly added optional native/IR workspace members do not enter the ordinary portable test/build closure. Keep explicit optional lanes for their tests.
6. Add native-executor forwarding to the standalone benchmark manifest; no absent placeholder native feature from P016 may count as a callable backend.

## Meaningful cases

- Invalid slot/control/source maps cannot produce VerifiedNumericProgram.
- Foreign Module/policy artifacts cannot be selected for execution.
- Default core compiles without native/IR crates and native interfaces contain no reference/host state.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime --features native-executor,execution-reference p095_ -- --nocapture
cargo test --locked -p wtime-native p095_ -- --nocapture
cargo test --locked -p wtime-exec-ir p095_ -- --nocapture
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
  "task": "P095",
  "tests_claimed": 0,
  "timing_evidence": false
}
```
