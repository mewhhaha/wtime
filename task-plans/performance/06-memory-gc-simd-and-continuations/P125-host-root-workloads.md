# P125 — Add source-bound host root-snapshot workloads

Status: **done**

Outcome: **contract_only**

Kind: `foundation` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P011 — Measure reference, GC and exception paths](../00-measurement/P011-gc-exception-workloads.md)

[P033 — Cache immutable subtype answers with identity and work guards](../02-structural-wins/P033-bounded-subtype-cache.md)

[P034 — Replace repeated flattened type scans with immutable layout lookups](../02-structural-wins/P034-flat-gc-metadata.md)

[P123 — Adapt supported typed workload routes to immutable P001 sources](../03-compact-executor/P123-p001-typed-harness-adapter.md)
(reuse its adapter plumbing; P068-before remains the required baseline)

## Scope and ownership

Read `EXECUTION.md`, `MEASUREMENT.md`, `BACKEND_CONTRACTS.md`, and contracts C2, C3, C4, C6, C7, C8. P125 owns harness, fixture, workload-model, and verification changes only. It does not change runtime core sources.

- `tools/wtime-bench/src/gc_roots_driver.rs`
- `tools/wtime-bench/src/protocol.rs`
- `tools/wtime-bench/root_workloads.py`
- `tools/wtime-bench/root_compare.py`
- `tools/wtime-bench/focused_compare.py`
- `tools/wtime-bench/candidate_gate.py`
- `tools/wtime-bench/selection_manifest.py`
- `tools/wtime-bench/execution_identity.py`
- `tools/wtime-bench/workloads.json`
- `tools/wtime-bench/fixtures/gc/roots/**`
- `tools/wtime-bench/tests/test_root_workloads.py`
- `tools/wtime-bench/tests/test_root_compare.py`
- `docs/benchmarks/performance/P125-verification.json`

The Rust driver may add a narrow CLI route to the current harness. Build the immediate-predecessor adapter from the immutable `/workspace/wtime-specialists/target/performance/P068-before` archive with the current harness and pinned tool hashes. P001 through P123 is supplemental only and cannot replace P068-before. Preserve the default core's zero-external-dependency and safe-Rust constraints.

## Input/output contract

Add exactly six distinct WAT/Wasm fixtures that produce 24 gc-root-snapshots workload rows: numeric i32 and reference-bank shapes at fixed bank sizes 64, 256, and 512, each with callback count n in {0,1,17,32}. Bank size is fixture-shape metadata, not an extra guest function parameter; n is the input and exact callback count. Numeric locals hold n+j and are all consumed after callbacks, with independent result sum(n+j)+42*n+n for j=0..bank-1. Reference locals are nonnull aliases of one GC struct holding n; allocate one struct before the callback loop, initialize all bank locals, perform n callbacks, then use every alias after the loop, with independent result bank*n+42*n+n. Each callback collects and re-enters a nested guest helper on the same Store; the helper returns 42 and guest code adds one per callback, so the per-callback contribution is 42*n+n, not a callback return of 43. The one object is live=1/reclaimed=0 during each callback collection, then is reclaimed after invocation roots end. Even the numeric n=0 case allocates the object before the loop and reclaims it after invocation; it runs zero callbacks. Run all 24 rows across five shared stages on both current pre-P068 sources and an adapter built from the immutable P068-before archive with the current harness. The current source map must match the archived pre-P068 core before paired execution. Stage meanings are: prepare times module compilation; instantiate times Store, link, and instance setup; first_call and warm_call time guest invocation, including each configured in-invocation host callback collection and same-Store reentry; load_and_call times the combined path. Thus there are 120 row-stage records per side. Oracle calculation, reset, and the separate post-invocation cleanup collection stay outside timed intervals. The callback collection itself is part of the guest invocation and remains timed. Include one representative warm repeated-batch smoke and one paired-adapter smoke. Bind exact archive, source, fixture/Wasm, harness, pinned tool, driver, adapter, and binary hashes. These paired foundation smokes validate the inputs and protocol only; they do not establish a speed result or gate promotion. The only valid bank sizes are 64, 256, and 512; the only valid callback counts are 0, 1, 17, and 32. Validate those fixed integers and use checked bounded work accounting such as (slots+8)*(n+1) before execution. Reject values outside the fixed sets. No general arbitrary-size workload API is required. Add the new selection without altering existing legacy selection contracts. Keep the original P068 selection freeze, reports, and failed gate attempts immutable; versioned P125 workload/verification artifacts may be added. Do not require byte-for-byte preservation of the mutable P068 verification document.

## Implementation recipe

1. Inspect P011 GC rows, P033/P034 append-only family checks, P123 adapter plumbing, and the current paired protocol. Verify the immutable /workspace/wtime-specialists/target/performance/P068-before archive and confirm its core source map matches the current pre-P068 core. Use the current new harness to build the archived adapter and pin the tool hashes. Preserve the original P068 selection freeze, reports, and failed gate attempts; never substitute P001 for the immediate predecessor and do not edit runtime core sources.
2. Add six distinct deterministic WAT fixtures: numeric i32 local banks and nonnull reference aliases of one GC struct holding n, each at fixed bank sizes 64/256/512. The bank size is fixture metadata, not a guest parameter. Allocate one object before each callback loop, initialize the whole bank, execute exactly n callbacks, then use all bank entries. Each callback collects and re-enters a nested guest helper on the same Store; that helper returns 42 and guest code adds one per callback. Independent loop oracles are sum(n+j)+42*n+n and bank*n+42*n+n. Hash the exact six WAT/Wasm fixtures.
3. Append exactly 24 workload rows and a separate gc-root-snapshots selection containing exactly those rows. Preserve every legacy workload row and selection contract. Record fixture-shape bank size, n, exact callback count, reset policy, all five stage definitions, required features, independent oracle, and source/Wasm identities. Reject duplicate IDs, altered legacy rows, wrong fixture-size mapping, unsupported bank sizes, or unsupported callback counts.
4. Implement src/gc_roots_driver.rs and narrow routing in src/protocol.rs, focused_compare.py, candidate_gate.py, selection_manifest.py, and execution_identity.py. Run all 24 rows over five stages on current pre-P068 sources and the current-harness adapter built from P068-before (120 records per side), with actual capability probes and independent oracles. Pin the P068-before archive manifest and the current helper, harness, driver, adapter, fixture, and binary hashes. Run a representative warm repeated-batch smoke and paired-adapter smoke; P001 via P123 may be an additional smoke only.
5. Validate all 24 row oracles and root-lifetime outcomes on both sides. The object remains live=1/reclaimed=0 during callback collection and is reclaimed by the separate post-invocation collection; n=0 still allocates/reclaims it in both shapes. Time callback collection and same-Store reentry as part of guest invocation. Keep oracle calculation, reset, and the separate post-invocation cleanup collection outside timed intervals. prepare, instantiate, first_call, warm_call, and load_and_call retain their stage-specific meanings above.
6. Enforce the finite bank and callback value sets before work begins and bound work with checked arithmetic using only those validated fixed integers. Reject an out-of-set value before allocation or guest execution.
7. Add six named checker cases and the shared verification contract/development groups. Cases must reject missing or extra callbacks, wrong child-root counts, wrong reset, forged stage timings, and changed archive/tool/fixture identities. Full close proves all 24 rows across all five stages on both sides plus the representative warm-batch and paired-adapter smokes. Keep P125 a source-bound foundation; do not use it as a speed gate or promotion result.

## Meaningful cases

- `p125_six_fixtures_expand_to_exact_24_rows`: Six distinct fixtures map to the two shapes, three fixed bank sizes, and four allowed n values; bank size is fixture metadata, not an extra guest parameter.
- `p125_numeric_i32_bank_oracle_uses_every_slot`: All fixed i32 locals contain n+j, are consumed after exactly n callbacks, and match sum(n+j)+42*n+n.
- `p125_reference_alias_bank_oracle_and_lifetime`: All nonnull aliases point to the one pre-loop struct holding n; every alias is used after n callbacks and the result is bank*n+42*n+n; each callback collection sees live=1/reclaimed=0 and post-invocation collection sees live=0/reclaimed=1.
- `p125_zero_callbacks_still_allocate_and_reclaim`: Both numeric and reference shapes with n=0 allocate and consume their banks, execute zero host callbacks, and reclaim the one GC object after invocation.
- `p125_predecessor_adapter_runs_all_rows_and_stages`: Current pre-P068 sources and the P068-before adapter built with the current harness produce all 120 row-stage records per side and pass independent oracles; the representative warm batch and paired-adapter smoke preserve the same row, harness, and reset identity.
- `p125_rejects_missing_callbacks_roots_reset_timing_and_hashes`: Validators reject missing/extra callbacks, wrong child-root counts, wrong reset, forged timing records, changed archive/tool/fixture identities, and bank/callback values outside the fixed sets.

## Verification

Use `python3 -B` for imported helpers. The contract has `model`, `driver`, and `capability` development groups. Full close validates all six WAT/Wasm fixtures, expands the exact 24 rows, verifies the immutable P068-before archive/current source match, builds the current-harness adapter, checks two-sided capability and independent oracles, and proves all 24 rows across five stages on both sides. It also runs the representative warm repeated-batch and paired-adapter smokes. No measurement decision or runtime promotion is part of P125.

```sh
python3 -B tools/verify.py plan --contract docs/benchmarks/performance/P125-verification.json
python3 -B tools/verify.py dev --contract docs/benchmarks/performance/P125-verification.json --group model --execute
python3 -B tools/verify.py dev --contract docs/benchmarks/performance/P125-verification.json --group driver --execute
python3 -B tools/verify.py dev --contract docs/benchmarks/performance/P125-verification.json --group capability --execute
python3 -B tools/verify.py close --contract docs/benchmarks/performance/P125-verification.json --output target/performance/P125/complete --execute
```

## Measurement and decision

Selection: `gc-root-snapshots`. Explicit execution setting: `decoded`. Decision rule: `integrity`. Foundation correctness and source-bound adapter proof only. The paired stage records and smoke runs validate workload and protocol identity; no speed comparison, performance gate, runtime change, or promotion follows from P125. P068 was the measured candidate; its numeric gate completed inconclusively, so P125 records source-bound foundation evidence only. Preserve the original P068 selection freeze, reports, and failed attempts; the new P125 selection and verification contract are separate versioned artifacts.

## Completion evidence

Record actual command/status/settings/counts; all six WAT/Wasm hashes; 24 rows and 120 row-stage records per side; stage definitions; the P068-before archive and current-source-match identities; current tool, harness, adapter, driver, fixture, and binary hashes; actual capability probes; named checker counts; and representative warm-batch and paired-adapter smoke results. P125 is published **done/contract_only** and records correctness and source binding only, not a speed result. Its workload model, comparison, and verifier support are integrated in the primary checkout (`tools/wtime-bench/root_workloads.py`, `root_compare.py`, `tests/test_root_workloads.py`, `tests/test_root_compare.py`, and `tools/verification/producers/p125_roots.py`); it records no runtime change or speed result.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime-specialists/target/performance/P125/shared-finalization/attempt-0002/evidence.json": "d3bcb4a5bfb4e8636edc71a6710a9ee64c318876ddd3844d0c880ef0b7df4500"
  },
  "evidence": [
    {
      "bytes": 1541,
      "path": "/workspace/wtime-specialists/target/performance/P125/shared-finalization/attempt-0002/evidence.json",
      "sha256": "d3bcb4a5bfb4e8636edc71a6710a9ee64c318876ddd3844d0c880ef0b7df4500"
    }
  ],
  "outcome": "contract_only",
  "status": "done",
  "task": "P125"
}
```
