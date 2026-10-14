# P074 — Specialize portable SIMD bitwise and select operations

Status: **done**

Outcome: **inconclusive**

Kind: `candidate` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P002 — Compare separately built baseline and candidate binaries](../00-measurement/P002-paired-driver.md)

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P010 — Add SIMD and strict floating-point benchmarks](../00-measurement/P010-simd-float-workloads.md)

[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)

[P016 — Add explicit executor policies and test integration hooks](../01-execution-contracts/P016-execution-policy.md)

[P026 — Use fixed buffers for SIMD and lane memory access](../02-structural-wins/P026-fixed-vector-reads.md)

[P039 — Lower float/vector constants and barrier adapters](../03-compact-executor/P039-compact-float-vector-adapters.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C2, C3, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md). P074 changes only the portable SIMD leaf implementation and its owned semantic fixture/tests; registration is a private unit module in `simd.rs`, with no `tests/performance.rs` or `performance.rs` registration edit.

- `crates/wtime-runtime/src/simd.rs`
- `crates/wtime-runtime/tests/performance/p074.rs`
- `crates/wtime-runtime/tests/fixtures/performance/p074/bitops.wat`
- `crates/wtime-runtime/tests/fixtures/performance/p074/bitops.wasm`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

Use safe u128 values decoded with from_le_bytes and encoded with to_le_bytes for the seven V128 bitwise/select operators, preserving public vector layout, exact source fuel and cancellation state, and the existing portable fallback for unrelated SIMD operations; add source-bound P074 fixtures and prove the declared mixed-XOR timing portfolio against the immediate predecessor.

## Implementation recipe

1. Before candidate edits, archive the actual immediate predecessor and freeze docs/benchmarks/performance/P074-selection.json into target/performance/P074/selection-freeze.json; retain P001 only for later milestone totals.
2. Implement the seven V128 bitwise/select operators with safe u128/from_le_bytes/to_le_bytes conversion and no intrinsics, unsafe casts, or public vector-layout changes; retain fallback for every unrelated SIMD operation.
3. Add the six-source-event WAT/Wasm fixture at crates/wtime-runtime/tests/fixtures/performance/p074/bitops.wat and .wasm; exercise exact fuel prefixes, partial V128 global state, cancellation, recovery, and optional-tier materialization at pc22 with three V128 operands.
4. Add and register private unit coverage in simd.rs for the three named P074 tests, including 512 deterministic random pairs for each of not/and/andnot/or/xor, bitselect masks, any_true, and i8add/f32 signed-zero fallback.
5. Run the declared default and optional-tier semantic groups, production decoded build and budget check, then the full source-bound P074 candidate protocol with two runs of 30 complete pairs and 10,000 bootstrap resamples.
6. Record the exact decision and evidence; promote only the declared mixed-XOR compute portfolio if both target sizes pass all gates, otherwise retain the rejected or inconclusive result without overstating coverage of all seven semantic operators.

## Meaningful cases

- p074_random_bitwise_patterns_use_word_dispatch checks 512 deterministic pairs for each of not, and, andnot, or, and xor against an independent byte oracle.
- p074_masks_any_true_and_fallback_preserve_bits checks bitselect masks, any_true, and unrelated i8add and f32 signed-zero behavior on the portable fallback.
- p074_fuel_prefixes_and_cancellation_keep_global_state checks the 27 source events including implicit end, fuel prefixes 0 through 28, partial V128 global state, cancellation and recovery; optional features also check materialization at pc22 with three V128 operands.

## Verification

Activate Rust as described in EXECUTION.md. Run the focused default and optional-tier configurations separately. Then run the source-bound full contract; development receipts support iteration only and cannot satisfy the full candidate gate. The full verification includes the decoded production build, a separate default build-budget check, existing SIMD semantics, the three named P074 cases in both modes, and the declared analysis tooling.

```sh
cargo test --locked -p wtime-runtime --lib p074_ -- --nocapture
cargo test --locked -p wtime-runtime --lib --features compact-executor,execution-reference p074_ -- --nocapture
python3 -B tools/verification/producers/runtime_production.py --mode decoded --output target/performance/P074/production
python3 -B tools/checks/build_budget.py --output target/performance/P074/build.json
cargo test --locked -p wtime-runtime --lib simd::tests:: -- --nocapture
python3 -B tools/verify.py dev --contract docs/benchmarks/performance/P074-verification.json --group semantics --execute
python3 -B tools/verify.py dev --contract docs/benchmarks/performance/P074-verification.json --group tier-semantics --execute
python3 -B tools/verify.py dev --contract docs/benchmarks/performance/P074-verification.json --group tooling --execute
python3 -B tools/verify.py close --contract docs/benchmarks/performance/P074-verification.json --output target/performance/P074/complete --execute
```

The measurement phase runs `focused_compare.py` with the frozen selection, semantic receipt, immediate-predecessor archive and actual decoded production executable. The analyze phase computes the paired statistics and gate decision once. Do not run exhaustive comparison or a separate raw analyzer during this candidate gate. Full verification requires all declared semantic cases and exact source identities.

## Measurement and decision

Selection: `simd`. Explicit execution setting: `decoded`. Decision rule: `measured_promotion`.

The selection has 14 rows, 20 row-stage groups and 1,200 pairs: two independent runs × 30 pairs for each row-stage group. It targets `simd_integer_lanes_n17` and `simd_integer_lanes_n1000` at first_call, warm_call and load_and_call; held-out rows are `simd_integer_lanes_n0` and `simd_integer_lanes_random_00` at warm_call. Protected rows are `simd_scalar_integer_lanes_n17`, `simd_scalar_integer_lanes_n1000`, `simd_shuffle_n1000`, `simd_lane_memory_n1000`, `simd_float_arithmetic_n1000`, `simd_float_min_zero`, `simd_relaxed_swizzle_n1000`, and `simd_relaxed_swizzle_oob_17` at warm_call. `call_overhead` covers prepare, first_call and load_and_call; `prepare_large` covers prepare.

Speed claims apply only to the declared mixed-XOR compute portfolio. The semantic suite exercises all seven bitwise/select operators. Use uninstrumented production timings, 10,000 bootstrap resamples, the declared cold products, and all protected controls. Adopt only when both target sizes and all gate rules pass; record rejection or inconclusive evidence honestly otherwise.

## Candidate gate schedule

Before implementation, freeze the selection against the actual immediate predecessor. The manifest binds exact registry workload IDs and inputs, target, held-out and protected rows, stages, feature/runtime settings and cold products. Run the complete declared set in two independent runs with at least 30 complete pairs per row and run. Candidate promotion compares only against the immediate predecessor; P001 is reserved for exhaustive milestone reporting. Preserve all semantic, fuel/cancellation and fallback cases regardless of timing selection. See [MEASUREMENT.md](../MEASUREMENT.md) for identity, sample reuse and gate rules.

## Before editing

Run these commands before changing candidate sources. The archive must contain the actual immediate predecessor source and production binary, and must not overwrite a different archive. Freeze uses the archived manifest as its second positional argument and writes the frozen selection to its third positional argument.

```sh
python3 -B tools/wtime-bench/baseline.py --output target/performance/P074-before
python3 -B tools/wtime-bench/selection_manifest.py freeze docs/benchmarks/performance/P074-selection.json target/performance/P074-before/manifest.json target/performance/P074/selection-freeze.json
```

Create and validate the selection manifest before freezing. The full contract revalidates the receipt without replacing it after implementation.

## Executed result

The full P074 gate completed with outcome **inconclusive**. It measured 1,200 pairs across two runs; primary candidate/baseline ratios were 1.00110978 and 1.02392696, with wide confidence intervals and no supported 5% gain. The exact prior SIMD implementation was restored from `target/performance/P074-before` with fresh modification times; the candidate and tests remain available at `target/performance/P074/rollback/attempt-0001/candidate`. The restored five existing SIMD tests passed in development attempt 0009. The new runtime-only production proof passed. Its initial fresh build took 54.45 seconds; that is a build observation, not a hot compile-speed claim. Execution ran in an isolated source copy. The candidate was inconclusive and was rolled back, so no P074 runtime change is awaiting primary integration. The published completion evidence is `target/performance/P074/shared-finalization/attempt-0002/completion.json`.

## Completion evidence

The published task outcome is **done/inconclusive**. The measured result, rollback, and immutable completion evidence are recorded above and in `target/performance/P074/shared-finalization/attempt-0002/completion.json`; no speedup or default-adoption claim follows.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime-specialists/target/performance/P074/shared-finalization/attempt-0002/evidence.json": "6746b12c486ce586bf95c8956fe4516aaaacc8fe46e90601f219b1d0dfd8c2df"
  },
  "evidence": [
    {
      "bytes": 2380,
      "path": "/workspace/wtime-specialists/target/performance/P074/shared-finalization/attempt-0002/evidence.json",
      "sha256": "6746b12c486ce586bf95c8956fe4516aaaacc8fe46e90601f219b1d0dfd8c2df"
    }
  ],
  "outcome": "inconclusive",
  "status": "done",
  "task": "P074"
}
```
