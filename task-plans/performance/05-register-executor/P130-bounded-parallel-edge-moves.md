# P130 — Prepare bounded parallel register edge moves

Status: **done**

Outcome: **contract_only**

Kind: `foundation` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P057 — Define a small numeric register bytecode and eligibility rules](P057-register-ir-contract.md)

[P058 — Translate numeric stack code into direct slot operands](P058-stack-to-register-lowering.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/execution/register/control.rs`
- `crates/wtime-runtime/tests/performance/p130_private.rs`
- `crates/wtime-runtime/tests/performance/p130.rs`
- `crates/wtime-runtime/tests/performance.rs`

P130 owns checked immutable edge-move plan preparation and private library unit tests. The planner is compiled only with `register-executor`; it is safe Rust, adds no dependencies or default-core code, exposes no public planner API, and does not select register execution. P061 consumes plans for its initial Jump/Branch subset and must preserve exact source fuel, cancellation and operand-limit materialization for that subset. P060 remains the full control-transfer candidate and performance gate. One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; keep its registration hunk bounded.

## Input/output contract

A bounded planner accepts typed source/destination slot moves and returns an immutable deterministic schedule that has the same result as simultaneous assignment. Numeric types describe values at the original source boundary: reads of one original source slot must agree on type, but target slots may be reused with another numeric type after the edge. It rejects out-of-range original slots, duplicate destinations, conflicting types for reads of the same original source slot, and work or byte budget excess before publication. It removes self-moves, schedules acyclic moves without overwriting a still-needed source, and breaks cycles with a symbolic Scratch value whose Save step records the NumericType. Save occurs before scratch-dependent reads; the one scratch value is reused across disconnected cycles, including mixed-type cycles. All four numeric types fit the safe u64 value representation; no scratch slot index is synthesized. Failure publishes no partial plan. The planner and its private unit tests compile only with register-executor, remain safe Rust, and do not expose a public planner API or select register execution.

## Implementation recipe

1. Define private checked move-request and immutable move-plan types using bounded slot indices and source-boundary NumericType values; validate original source/destination slot bounds, unique destinations, and consistent numeric types for reads of each original source slot before producing output. Permit destination slot type reuse across the edge.
2. Remove identity moves and construct a deterministic schedule for acyclic components, selecting only moves whose destination is no longer needed as a source; preserve simultaneous-assignment semantics for arbitrary input order.
3. Detect cycles and break each with one symbolic Scratch value; emit Save with the saved NumericType before scratch-dependent reads, then reuse that scratch value across disconnected cycles including mixed-type cycles. All four numeric types fit the safe u64 value representation, so do not synthesize a scratch slot index or reject a scratch type; account schedule steps and encoded bytes against shared work/byte limits with checked arithmetic.
4. Reserve and validate the complete bounded output before allocating/publishing it; every limit or arithmetic failure returns a decline without an observable partial plan.
5. Add the four private named planner unit cases and compare bounded generated schedules with a test-only simultaneous-assignment oracle; add a separate public integration policy guard proving register execution remains unavailable when the feature is disabled.

## Meaningful cases

- p130_identity_and_acyclic_moves_preserve_simultaneous_results: empty and self-only requests produce an empty plan, while acyclic chains and fanout preserve every original source value.
- p130_two_and_three_slot_cycles_use_bounded_scratch: swaps and rotations match simultaneous assignment; mixed-type disconnected cycles reuse one symbolic scratch, and the real P058 edge_values fixture shifts i32/i64 values into earlier temporary heights where a slot changes from i32 to i64 after its old value is read.
- p130_invalid_requests_and_budget_failures_publish_nothing: duplicate destinations, conflicting NumericTypes for reads of the same old source slot, out-of-bounds slots, arithmetic overflow, and work/byte limits reject transactionally, while destination type reuse is permitted.
- p130_bounded_generated_graphs_match_simultaneous_oracle: 4096 deterministic bounded all-i32 move graphs with bit-distinct source values produce exactly the test-only simultaneous-assignment result.
- p130_register_policy_stays_unavailable_without_execution: with `register-executor` disabled, the default engine remains Decoded and requesting Register returns DisabledFeature; this test does not expose or exercise the planner API.

The first four cases are private library unit tests under the optional feature, executed by the package test command. The public policy guard is an integration test compiled without the optional feature.

## Before editing

Verify the original immutable P001 archive, then create a fresh immutable decoded P130 immediate-predecessor archive before changing runtime sources. Do not reuse or overwrite a different archive. These archival commands were run before any P130 runtime edits; no implementation-test execution is implied.

```sh
python3 -B tools/wtime-bench/baseline.py --output target/performance/baseline
python3 -B tools/wtime-bench/baseline.py --output target/performance/P130-before --mode decoded
```

## Verification

Activate Rust as described in EXECUTION.md. Run private planner semantics with the optional feature; run the policy guard and default build without it. Private cases are library unit tests, not `--test performance` cases, and the planner must remain unavailable in the default build.

```sh
cargo test --locked -p wtime-runtime --features register-executor,execution-reference p130_ -- --nocapture
cargo test --locked -p wtime-runtime --test performance p130_register_policy_stays_unavailable_without_execution -- --nocapture
cargo check --locked -p wtime-runtime
cargo check --locked -p wtime-runtime --features register-executor
```

## Measurement and decision

Selection: `scalar`. Explicit execution setting: `register`. Decision rule: `integrity`.

This immutable planner foundation makes no speed claim and does not enable a runtime selector. P061 owns initial Jump/Branch operations consuming these plans and must prove exact source state for its eligible subset. P060 remains responsible for the full portable control-transfer candidate gate: broader block/loop/if/end/br_table/return behavior, exact source-event and logical-height semantics, source-prefix interruption, fuel/cancellation/operand-limit materialization, optimized/fallback accounting, paired measurement, build budgets and promotion decision.

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes for all four private cases, the public policy guard, and both feature configurations. Keep status pending until implementation and checks are executed; this task-plan edit is not implementation evidence.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/P130/shared-finalization/attempt-0001/evidence.json": "ecb263457e4a29db3cf4b774e6040b9e6f027a3f580506805e3d7849a327fa43"
  },
  "evidence": [
    {
      "bytes": 1563,
      "path": "/workspace/wtime/target/performance/P130/shared-finalization/attempt-0001/evidence.json",
      "sha256": "ecb263457e4a29db3cf4b774e6040b9e6f027a3f580506805e3d7849a327fa43"
    }
  ],
  "outcome": "contract_only",
  "status": "done",
  "task": "P130"
}
```
