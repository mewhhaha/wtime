# P129 — Add bounded source-visit selection to execution diagnostics

Status: **done**

Outcome: **contract_only**

Kind: `foundation` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P018 — Define source cursors and materialized guest state](P018-source-cursor.md)

[P020 — Centralize exact source instruction accounting](P020-exact-execution-accounting.md)

[P057 — Define a small numeric register bytecode and eligibility rules](../05-register-executor/P057-register-ir-contract.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), and the cursor/accounting contracts established by the prerequisites.

- `crates/wtime-runtime/src/lib.rs` (execution-reference diagnostic integration only)
- `crates/wtime-runtime/src/execution/cursor.rs`
- `crates/wtime-runtime/tests/performance/p129.rs`
- `crates/wtime-runtime/tests/performance.rs` (module registration only)

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; keep the shared integration hunk bounded. The diagnostic remains behind `execution-reference`, which is excluded from default builds.

## Input/output contract

Keep request_materialization_roundtrip(f, pc) as the zero-skip convenience API and forward it to request_materialization_roundtrip_after_visits(f, pc, skip_visits: usize). The new API rejects skip counts above 100000 with ResourceLimit before guest work. For a validated handle, guest and original function/PC, it counts only matching dynamic occurrences; when the requested occurrence is reached it performs the existing take/snapshot/restore once at the charged-before-fetch boundary. Nonmatching source visits do not consume the selector. Existing handle, guest and PC validation, source accounting, cancellation polling and guest prefix remain unchanged. When the selector is not reached, execution completes normally without taking or restoring a snapshot.

## Implementation recipe

1. Inspect the existing request, boundary matching, and snapshot/restore flow; preserve the current API by forwarding it to the new visit-counted request with skip_visits zero.

2. Add a usize skip counter to the execution-reference materialization request, enforce the 100000 cap before guest execution, and decrement only when the active original instance, function and PC all match; trigger the existing one-time roundtrip when the counter reaches zero.

3. Preserve all current handle/guest/PC rejection and charged-before-fetch behavior, and ensure an unreached request returns the ordinary full execution result without snapshot/restore or altered guest work.

4. Add the three named integration tests, including the P047 loop’s third matching visit with exact state/fuel assertions and bounded nonmatching/unreached paths; register them in the existing performance test target.

## Meaningful cases

- p129_zero_skip_preserves_first_visit: the existing API and explicit zero skip roundtrip at the first matching occurrence and retain current validation and state behavior.

- p129_later_loop_visit_keeps_exact_state_and_fuel: for the P047 branch_loop PC 8, skip two matches and roundtrip on visit three; assert counter 2, sum 16, step 8, exact 35 charged prefix events, remaining fuel 0 and pending charged instruction true, then observe ResourceLimit on invocation.

- p129_invalid_visit_bound_and_unreached_request_are_bounded: skip above 100000 rejects before guest work; skip five beyond the loop’s five visits returns full result 32 with no snapshot/restore.

## Verification

Activate Rust as described in EXECUTION.md. These focused commands are required implementation-time checks; the first must execute all three named cases, while the default build confirms the diagnostic integration remains feature-gated.

```sh
cargo test --locked -p wtime-runtime --features execution-reference p129_ -- --nocapture
cargo check --locked -p wtime-runtime
```

## Measurement and decision

Selection: `scalar`. Explicit execution setting: `decoded`. Decision rule: `integrity`.

This foundation extends a development-only diagnostic selector and makes no runtime speed claim. Confirm the default build excludes the feature-gated hooks; do not add selector work to production execution.

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes, all three named cases, and confirmation that the default build excludes the hooks. Keep status pending until implementation and checks are executed; this planning change is not implementation evidence.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/P129/shared-finalization/attempt-0002/evidence.json": "0c3c659a972308ab7312091e62c3fa8568b7004a68040ef07adcecad02032060"
  },
  "evidence": [
    {
      "bytes": 1526,
      "path": "/workspace/wtime/target/performance/P129/shared-finalization/attempt-0002/evidence.json",
      "sha256": "0c3c659a972308ab7312091e62c3fa8568b7004a68040ef07adcecad02032060"
    }
  ],
  "outcome": "contract_only",
  "status": "done",
  "task": "P129"
}
```
