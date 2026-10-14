# P132 — Extract transactional per-function Register preparation

Status: **done**

Outcome: **contract_only**

Kind: `foundation` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P065 — Verify roots and captured state around register exits](P065-register-root-and-capture-barriers.md)

[P130 — Prepare bounded parallel register edge moves](P130-bounded-parallel-edge-moves.md)

[P131 — Prepare bounded Register prefixes for decoded handoff](P131-bounded-register-prefix.md)

P065 transitively supplies the register IR, numeric execution, memory, and exit foundations; P130 supplies `MovePlan::prepare`; P131 supplies the current preparation implementation and strict whole-function checks. P066 must depend on P132 so its lazy compilation policy uses the extracted per-function preparation boundary.

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/execution/register/execute.rs`
- `crates/wtime-runtime/src/execution/register/preparation.rs` (new private leaf module, if needed)
- `crates/wtime-runtime/tests/performance/p132_private.rs`
- `crates/wtime-runtime/tests/performance/p132.rs`

One integrator owns `lib.rs`, dispatch, Cargo features and `tests/performance.rs`; the leaf owner submits a bounded test-registration hunk. Keep the public `PreparedModule::prepare` behavior and its module-wide output unchanged. Do not introduce lazy policy, cache state, synchronization, diagnostics, or changes to `P066` in this task.

## Input/output contract

Extract the existing per-function preparation transaction from PreparedModule::prepare into a private helper returning Complete(PreparedFunction, Budget), Declined, Exhausted, or Invalid. Preserve ordered eager module preparation and the current Vec<Option<PreparedFunction>> output. Complete publishes a fully validated bounded function and tentative retained-budget state; Declined publishes nothing, leaves retained budget unchanged, and retains the attempt-work charge; Exhausted preserves the current loop break; Invalid preserves the current whole-module None. Use safe body/layout/plan indexing and never panic on invalid indices. Preserve all preparation, activation, admission, fallback, edge-plan, capacity-accounting, fuel, trap, memory, call-exit, and execution-policy semantics. The helper returns PreparedFunction without Arc allocation or lazy/cache policy APIs.

Preserve current semantics exactly: `MAX_WORK = 100_000`, `MAX_BYTES = 1 MiB`, and `MAX_ACTIVATION_BYTES = 2 MiB`; checked worst-case charge calculation; attempted-work charging before eligibility declines; numeric-only admission; strict operation validation; lowered-function then decoded-exit preparation fallback; exact edge/move-plan validation; actual allocation-capacity accounting; and transactional retained-budget commit only after all validation succeeds. An unsupported or over-budget function declines without publishing any partial artifact or spending retained budget. A failed attempt still consumes its bounded attempted-work charge, as it does today. Successful artifacts retain their current function and module ordering and cause no change to activation, fuel, traps, memory, call exits, or execution policy. The extracted helper returns a bounded `PreparedFunction` without adding `Arc` allocations; cache and lazy-policy APIs are outside this foundation task.

## Implementation recipe

1. Extract the per-body portion of PreparedModule::prepare in execute.rs or a private preparation.rs helper. Keep module index allocation and the ordered module loop in PreparedModule::prepare.
2. Make the helper own safe body/layout/plan lookup, layout and activation bound checks, checked worst-case charge calculation, transactional Budget cloning, LoweredFunction::lower / exits::prepare fallback, valid_operation checks, and complete edge/move-plan construction. Return Complete, Declined, Exhausted, or Invalid explicitly and never publish a partial artifact.
3. Preserve MAX_WORK=100000, MAX_BYTES=1 MiB, MAX_ACTIVATION_BYTES=2 MiB, attempted-work charging before eligibility declines, and transactional retained-budget commit only after full validation. Map Exhausted to the current loop break and Invalid to the current whole-module None.
4. Add focused semantic regression cases against the immutable P065-successor predecessor and refactored source. Compare exact preparation outcomes and selected prepared-artifact fields/hashes, not timings; retain default and combined-feature checks.
5. Keep the change to at most two focused patches: helper extraction, then focused tests and test registration. Move broader semantic or policy changes into a child task.

## Meaningful cases

- p132_eligible_functions_preserve_prepared_artifacts: eligible integer, float, unshared-memory, and call-prefix/decoded-exit functions preserve admission and selected artifact fields.
- p132_unsupported_and_overbudget_functions_decline_without_publication: unsupported types/operations and huge work or activation bounds publish no function and spend no retained budget.
- p132_module_attempts_and_aggregate_budget_match_predecessor: multiple functions preserve order and aggregate behavior; failed attempts retain attempted-work charges and successful functions alone commit retained metadata budget.
- p132_module_preparation_remains_policy_neutral: default and combined builds preserve eager module output and policy selection without lazy/cache behavior.

## Verification

Activate Rust as described in [EXECUTION.md](../EXECUTION.md). Read [SOURCE_NOTES.md](../SOURCE_NOTES.md), [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), and [CONTRACTS.md](../CONTRACTS.md). Run the focused P132 cases and existing P065/P131 integration checks in their exact feature configurations, including default and combined Register/reference features. Compare a fresh immutable P065-successor source snapshot with the refactored result; do not use timings as a gate. The focused tests must execute the named eligibility, decline, aggregate-budget, and policy-neutral cases.

```sh
cargo test --locked -p wtime-runtime --features register-executor,execution-reference p132_ -- --nocapture
cargo test --locked -p wtime-runtime --features register-executor,execution-reference p131_ -- --nocapture
cargo test --locked -p wtime-runtime --features register-executor,execution-reference p065_ -- --nocapture
cargo check --locked -p wtime-runtime
cargo check --locked -p wtime-runtime --features register-executor,execution-reference
```

## Measurement and decision

Selection: `register`. Explicit execution setting: `register`. Decision rule: `integrity`.

This is an internal preparation-boundary extraction. It makes no performance claim, adds no policy selector, and must retain eager module preparation behavior until P066 owns an explicitly bounded lazy policy.

## Before editing

Verify the immutable P001 archive and successful P065 correctness record. Create a fresh immutable P065-successor Register archive before runtime edits, recording its source-tree identity and hash in the freeze record. The archived source is the exact predecessor for the focused preparation inventory and field comparison. Do not overwrite an existing archive with different identities.

```sh
python3 -B tools/wtime-bench/baseline.py --output target/performance/P132-before --mode register
```

## Completion evidence

Record the P001 and frozen P065-successor archive identities, source/test/artifact hashes, exact commands and feature settings, per-case counts, exact per-function admission outcomes and selected artifact-field hashes. Record module-level prepared-function counts and attempted-work/retained-budget outcomes for the bounded multi-function cases. Do not report benchmark timing as acceptance evidence.


## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/P132/shared-finalization/attempt-0003/evidence.json": "8c5fccf8198bb7a79ba730957998cf6465d379d6e47497a69d3b9130eeb692f9"
  },
  "evidence": [
    {
      "bytes": 1567,
      "path": "/workspace/wtime/target/performance/P132/shared-finalization/attempt-0003/evidence.json",
      "sha256": "8c5fccf8198bb7a79ba730957998cf6465d379d6e47497a69d3b9130eeb692f9"
    }
  ],
  "outcome": "contract_only",
  "status": "done",
  "task": "P132"
}
```

The full contract closed successfully with freeze attempt 0003, verify attempt 0003, and close attempt 0086; publication attempt 0001 recorded `done/contract_only`. Verification executed 715 tests: 304 Register/reference, 410 combined Compact/Register/reference, and one default-policy test, with zero failures or ignored tests. The exact frozen inventory matched byte-for-byte: 1,133 rows covering 20 fixtures (19 with prepared bodies), 181 prepared and 15 declined bodies out of 196 total, 805 source PCs, 68 constants, 33 memory arguments, and 10 edges. The predecessor was the immutable P065-successor Register archive `target/performance/P132-before`, commit `4e333ac`, manifest SHA-256 `8a4aebc05209ec9fa9a77c6fcb2b229e17820271779661473698c523aa672374`; the original archive and library-test source tree were preserved.

The extracted helper preserves work and byte-budget accounting, uses safe body indexing, and returns explicit `Invalid`, `Exhausted`, `Declined`, or `Complete` outcomes. The default and combined configurations confirm eager module preparation and policy neutrality. Default clean and optional Register build sizes were 2.759334 and 2.788837 MB-equivalent; stripped sizes were 1,000,960 and 1,078,880 bytes. All build budgets passed, with zero external dependencies in the default core. Durable reports are `docs/benchmarks/performance/P132*`, including the gzip golden inventory, corrected reference, freeze, and proof. Export attempt `target/recovery/P132-export-0003` produced eight evidence copies and hash records; earlier failed export attempts remain archived. The architecture note is `docs/architecture/register-preparation.md`. This foundation makes no timing or performance claim, adds no cache/Arc allocation or policy change, and leaves lazy compilation to P066, which already depends on P132.
