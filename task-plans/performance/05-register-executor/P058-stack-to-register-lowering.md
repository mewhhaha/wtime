# P058 — Translate numeric stack code into direct slot operands

Status: **todo**

Outcome: **pending**

Kind: `foundation` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P037 — Map compact instructions to source events and control targets](../03-compact-executor/P037-compact-source-maps.md)

[P040 — Lower structured control to resolved jump and result plans](../03-compact-executor/P040-compact-control-plans.md)

[P057 — Define a small numeric register bytecode and eligibility rules](P057-register-ir-contract.md)

[P129 — Add bounded source-visit selection to execution diagnostics](../01-execution-contracts/P129-source-visit-diagnostics.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/execution/register/lower.rs`
- `crates/wtime-runtime/tests/performance/p058.rs`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

The first lowering pass assigns fixed local slots and source stack-height slots to numeric values, translating pop/push to explicit operand/destination indices and recording block-entry stack layouts. Its decoded-prefix comparison uses P129 bounded source-visit selection to inspect every dynamic loop prefix at a chosen original function/PC occurrence.

## Implementation recipe

1. Walk validated structured control with an abstract typed stack; assign locals first and each temporary by checked stack height, preserving parameter/result block shapes.
2. Emit explicit source operations and branch result move requests, using conservative stable slots across joins rather than building global SSA.
3. Eliminate only redundant physical stack moves when the source event/materialization recipe is retained; decline ambiguous/unimplemented control effects.
4. Compare translated slot states to decoded source prefixes on deterministic numeric/control fixtures and record lowering work/retained bytes.

## Meaningful cases

- Local.get/set/tee and binary destinations preserve numeric dataflow.
- Loop/multivalue join layouts retain values across backedges.
- Every source prefix has the right stack values/types/heights and malformed joins decline.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime --features register-executor,execution-reference p058_ -- --nocapture
```

## Measurement and decision

Selection: `scalar`. Explicit execution setting: `register`. Decision rule: `integrity`.

This is a tooling/contract deliverable. Require correct nonzero checks and bounded/source-bound output; do not invent a runtime speedup requirement. If it changes production work, record cold/build/retention deltas under [MEASUREMENT.md](../MEASUREMENT.md).

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. No commands in this planning file have been executed as implementation evidence.
