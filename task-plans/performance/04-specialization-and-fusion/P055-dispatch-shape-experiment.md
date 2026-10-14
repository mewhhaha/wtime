# P055 — Compare split match and handler-function scalar dispatch

Status: **done**

Outcome: **inconclusive**

Kind: `experiment` · Branch: `optional` · Model: **Sol 6.1 Low**

## Prerequisites

[P002 — Compare separately built baseline and candidate binaries](../00-measurement/P002-paired-driver.md)

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P007 — Capture CPU profiles and rank actionable costs](../00-measurement/P007-profile-hotpaths.md)

[P008 — Add scaling, branch and call benchmark families](../00-measurement/P008-scalar-workloads.md)

[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)

[P016 — Add explicit executor policies and test integration hooks](../01-execution-contracts/P016-execution-policy.md)

[P038 — Lower constants, locals and common integer operations](../03-compact-executor/P038-compact-scalar-lowering.md)

[P040 — Lower structured control to resolved jump and result plans](../03-compact-executor/P040-compact-control-plans.md)

[P053 — Separate metered and unmetered dispatch without weaker defaults](P053-accounting-mode-specialization.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C2, C3, C4, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/execution/compact/ir.rs`
- `crates/wtime-runtime/src/execution/compact/execute.rs`
- `crates/wtime-runtime/src/execution/compact/dispatch_experiment.rs`
- `crates/wtime-runtime/tests/performance/p055.rs`
- `crates/wtime-runtime/tests/performance.rs` (root-owned registration)
- `tools/wtime-bench/src/protocol.rs`
- `tools/wtime-bench/compare.py`
- `tools/wtime-bench/focused_compare.py`
- `tools/wtime-bench/tests/test_p055_batch_capacity.py`
- `tools/verification/producers/p055_batch.py` (root-owned producer)

The experiment changes only the final scalar dispatch subset in `execute.rs`, using a sibling `dispatch_experiment.rs` imported from `ir.rs`. The original `lower_scalar` pipeline stays unchanged, including fusions, memory, control, and `lower_numeric`. The harness addition is limited to the one single-case cap, exact generic wire validation, and focused calibration. Root owns registration in `tests/performance.rs` and the batch producer. Do not edit `tools/wtime-bench/workloads.json`; keep the admitted scalar op streams identical across source-bound builds.

## Input/output contract

Compare two separately compiled, source-sealed variants of only the existing final Compact scalar dispatch subset: a split match and safe handler functions. The existing lower_scalar pipeline, including fusions, memory, control, and lower_numeric, remains unchanged. Both variants use existing Compact mode and compact-executor; there is no new Cargo feature or mode, runtime selector, Store change, or public diagnostic API. Preserve exact source accounting, fuel, cancellation, barrier, trap, and materialization behavior; do not use computed-goto or unsafe dispatch. A recorded first paired measurement attempt stopped before statistics because run 2 call_overhead warm-up could not meet the 20 ms minimum within the 100,000-call single-case cap; it produced no statistics and is a harness-capacity blocker, not a semantic failure or performance rejection. The recovery is a harness-only single-case cap increase to 1,000,000 with exact generic wire validation and focused calibration; all scalar, SIMD, and thread caps, the 20 ms minimum, and all 13 rows / 61 logical stage groups remain fixed.

## Implementation recipe

1. Freeze the actual Compact production predecessor using baseline.py with `--mode compact`, then freeze the identical admitted scalar workload/op-stream selection with the positional selection-freeze output path.
2. Compare two separately compiled source-sealed implementations of only the existing final scalar dispatch subset: split match and safe handler functions. Keep all earlier lower_scalar fusions, memory, control, and lower_numeric stages unchanged. Use no new Cargo feature, execution mode, runtime selector, Store behavior, or public diagnostic API.
3. Wire the variants at the final scalar dispatch in execution/compact/execute.rs and place the experiment in sibling dispatch_experiment.rs imported from ir.rs. Use compile-time selection sealed by each archived source build; test-only thread-local counters privately prove that the selected route executed. Root owns registration in tests/performance.rs.
4. Preserve exact source-event and source-fuel accounting, cancellation, checked operands, iterative deep-loop behavior, and barrier/trap/fuel materialization semantics. Do not edit the workload registry or weaken source coverage.
5. Build separate source-bound verification contracts at docs/benchmarks/performance/P055-split-verification.json and docs/benchmarks/performance/P055-handlers-verification.json against the same frozen predecessor. Analyze each distinct dataset once; the aggregate combines results without reanalysis.
6. The first paired measurement attempt stopped before statistics: run 2 call_overhead warm-up could not reach the 20 ms floor within the 100,000-call single-case cap. Preserve that failed attempt as measurement history. Apply only a harness-side single-case cap increase to 1,000,000, validate exact generic wire records, and run focused calibration; keep all scalar/SIMD/thread caps, the 20 ms minimum, and the 13-row/61-group selection unchanged.
7. Build the matching upgraded harness against the immutable P055-before Core113 archive using the existing p123_adapter.py --current-api --mode compact flow, with adapter outputs under target/performance/P055-before-adapter. The candidate source remains split/handlers sealed; the comparison adapter manifest must bind the original before archive. Keep P055 todo/pending and make no performance conclusion until a full gate completes.

## Meaningful cases

- `p055_dispatch_shapes_match_original_scalar_operations_and_selected_path`
- `p055_numeric_locals_and_iterative_loops_preserve_source_counts`
- `p055_every_fuel_prefix_barrier_and_materialization_remains_canonical`

## Verification

Activate Rust as described in EXECUTION.md. The focused semantic commands and separate split/handler verification contracts remain as specified below. Planned work is 357 Rust tests (354 base plus 3 P055 cases), 30 existing Python fast-candidate/fast-selection tests, a new batch-capacity Python suite whose count is not yet known, and 6 final aggregation checks. These are plans, not results.

The first paired measurement attempt failed before statistics: run 2 call_overhead warm-up could not meet the 20 ms minimum within the 100,000-call cap for that single case. No statistics were computed. Treat this as harness capacity, not semantic failure or candidate rejection. Add only a harness-side 1,000,000-call single-case cap, with exact generic wire validation and focused calibration. Keep scalar, SIMD, and thread caps, the 20 ms floor, and all 13 rows / 61 logical groups fixed. Build the upgraded harness with the existing `p123_adapter.py --current-api --mode compact` flow against immutable Core113 at `target/performance/P055-before-adapter`; bind the adapter manifest to the original before archive.

The P007 profile evidence is limited: the existing counter profile is Decoded-only, the P055 profile is missing and CPU percentage is unavailable, so there is no Compact dispatch CPU attribution. This experiment tests the dispatch-shape hypothesis without claiming profile support or a performance result.

```sh
cargo test --locked -p wtime-runtime --features compact-executor,execution-reference p055_ -- --nocapture
cargo build --locked -p wtime-runtime --features compact-executor -j 4
python3 -B -m unittest discover -s tools/wtime-bench/tests -p test_fast_candidate.py -v -b
python3 -B -m unittest discover -s tools/wtime-bench/tests -p test_fast_selection.py -v -b
python3 -B -m unittest discover -s tools/wtime-bench/tests -p test_p055_batch_capacity.py -v -b
python3 -B tools/verify.py plan --contract docs/benchmarks/performance/P055-split-verification.json
python3 -B tools/verify.py close --contract docs/benchmarks/performance/P055-split-verification.json --output target/performance/P055/split/complete --execute
python3 -B tools/verify.py plan --contract docs/benchmarks/performance/P055-handlers-verification.json
python3 -B tools/verify.py close --contract docs/benchmarks/performance/P055-handlers-verification.json --output target/performance/P055/handlers/complete --execute
```

The three named Rust cases prove selected-route execution and preserve numeric/local behavior, iterative loops, source counts, fuel prefixes, barriers, and materialization. The new capacity test count must be established by the implemented suite; do not estimate it.

## Measurement and decision

Selection: `scalar`. Explicit execution setting: existing `compact` mode with `compact-executor`. Decision rule: `measured_promotion`. Compare separately compiled split-match and handler-function variants of only the final scalar dispatch subset against one frozen Compact predecessor. The original lower_scalar pipeline, fusions, memory, control, and lower_numeric remain unchanged. The 13 selected rows form 61 logical stage groups; all selection and timing caps stay fixed apart from the one-case warm-up capacity recovery. The first paired attempt stopped before statistics as described above.

Use the paired protocol and family/control/cold/build rules in [MEASUREMENT.md](../MEASUREMENT.md). Predeclare the targeted family and held-out controls before changing the implementation. Keep instrumentation off in production timings. Adopt only with supported gains in both runs; otherwise record rejected/inconclusive and remove default-path cost. A necessary immutable-interface change may be contract_only with its separate justification.

## Before editing

Run these commands before changing the candidate sources. The archive is immutable and captures the actual Compact production predecessor. The selection-freeze output is positional; do not pass `--output`. Both reversible source-bound candidates use this same frozen predecessor and identical scalar op streams. Retain P001's original archive for milestone totals. For native work, use the named portable control and verify its actual policy/quantum; if this archive already exists with different sources, choose a new recorded run-specific directory and update the comparison path rather than overwriting it.

```sh
python3 tools/wtime-bench/baseline.py --mode compact --output target/performance/P055-before
python3 -B tools/wtime-bench/selection_manifest.py freeze docs/benchmarks/performance/P055-selection.json target/performance/P055-before/manifest.json target/performance/P055/selection-freeze.json
```

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

P055 remains todo/pending. Both variants are selected by source-sealed compile-time constants (split versus handlers), with private test-only thread-local counters proving the route; there is no runtime selector or public diagnostic API. No Cargo feature, mode, Store behavior, workload registry, or earlier lowering stage changes. The first measurement attempt failed only at run 2 call_overhead warm-up under the single-case cap, before statistics; preserve it and retry after the bounded harness-only capacity fix. This is not a semantic failure or performance rejection. This metadata records planned counts only and claims no final performance result. Completion must record both independently built variants and their source-bound evidence.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/P055/shared-finalization/attempt-0001/evidence.json": "cb6e4f2c1357eea2e1dc4e42053bb292e8a2cf23ad92777104966f999ddc8df0"
  },
  "evidence": [
    {
      "bytes": 1639,
      "path": "/workspace/wtime/target/performance/P055/shared-finalization/attempt-0001/evidence.json",
      "sha256": "cb6e4f2c1357eea2e1dc4e42053bb292e8a2cf23ad92777104966f999ddc8df0"
    }
  ],
  "outcome": "inconclusive",
  "status": "done",
  "task": "P055"
}
```
