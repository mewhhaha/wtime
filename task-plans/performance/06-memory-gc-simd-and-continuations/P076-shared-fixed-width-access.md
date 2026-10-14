# P076 — Remove shared read allocation without weakening coordination

Status: **done**

Outcome: **inconclusive**

Kind: `candidate` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P002 — Compare separately built baseline and candidate binaries](../00-measurement/P002-paired-driver.md)

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P013 — Measure shared memory, contention and wait latency](../00-measurement/P013-thread-workloads.md)

[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)

[P016 — Add explicit executor policies and test integration hooks](../01-execution-contracts/P016-execution-policy.md)

[P025 — Remove per-load heap allocation for all scalar widths](../02-structural-wins/P025-fixed-scalar-reads.md)

[P026 — Use fixed buffers for SIMD and lane memory access](../02-structural-wins/P026-fixed-vector-reads.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C5, C7, C8** in [CONTRACTS.md](../CONTRACTS.md). The bounded runtime change has two parts: add one locked caller-buffer API, then route ordinary shared scalar/vector read callers through it in decoded, reference and Compact execution. Keep atomic/SC/wait coordination and unshared read behavior unchanged.

- `crates/wtime-runtime/src/shared_memory.rs`
- `crates/wtime-runtime/src/execution.rs`
- `crates/wtime-runtime/src/execution/compact/memory.rs`
- `crates/wtime-runtime/tests/performance/p076.rs`
- `crates/wtime-runtime/tests/performance/p025.rs`
- `crates/wtime-runtime/tests/performance/p026.rs`
- `crates/wtime-runtime/tests/performance/p043.rs`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

Add a locked caller-buffer API for fixed-width reads and route ordinary shared scalar and vector callers through it in decoded, reference, and Compact execution; preserve existing lock, bounds, alignment, current-length, and fuel behavior, leave atomics/SC/waits untouched, and preserve existing unshared read behavior.

## Implementation recipe

1. Before editing, verify and retain the existing P076-before archive containing the 109-source immediate predecessor; freeze docs/benchmarks/performance/P076-v2-selection.json into target/performance/P076/selection-freeze-v2.json before any runtime edit.
2. Add one bounded shared-memory caller-buffer read API that acquires the backing lock across validation and copy, preserving bounds, alignment, current-length, alias, and lock-order behavior.
3. Route all ordinary shared scalar and vector read callers through that API in decoded, execution-reference, and Compact paths; keep atomics, sequentially consistent operations, waits, and unshared allocating behavior unchanged.
4. Run the four named P076 semantic tests in default and optional feature configurations, all six existing shared-coordination tests, and all named P025/P026/P043 diagnostic checks; preserve their exact command feature sets.
5. Build the runtime-only decoded production binary and verify default and Compact companion build budgets, dependency closure, the two-CPU thread harness capability, and all source-bound full-verification checks.
6. Execute the 14-row, 16-group candidate selection against the frozen immediate predecessor in two runs of 30 complete pairs per group; retain all raw evidence and compute the paired statistics once. Limit timing claims to ordinary scalar shared reads and do not claim a speed result before analysis.

The immediate predecessor archive at `target/performance/P076-before` contains the 109-source pre-edit core. The v2 freeze is `target/performance/P076/selection-freeze-v2.json`. The current-harness baseline adapter at `target/performance/P076/baseline-adapter/attempt-0001/adapter` was built from that same archived core. Its thread capability declaration describes this adapter's protocol; use the adapter's explicit capability proof rather than treating the protocol metadata as an assertion about older binary capabilities.

## Meaningful cases

The full contract counts 30 named case executions: the four P076 semantic cases run in both default and optional feature configurations (eight executions), six existing shared-coordination cases, and all P025/P026/P043 diagnostic cases (sixteen executions). They are:

- default-feature P076 semantic case: p076_fixed_width_alias_reads_preserve_atomic_alignment_and_bounds
- default-feature P076 semantic case: p076_concurrent_grow_and_mixed_reads_copy_one_locked_image
- default-feature P076 semantic case: p076_guest_scalar_vector_and_lane_reads_use_fixed_shared_path
- default-feature P076 semantic case: p076_source_fuel_cancellation_oob_and_alias_growth_preserve_effects
- compact-executor,execution-reference P076 semantic case: p076_fixed_width_alias_reads_preserve_atomic_alignment_and_bounds
- compact-executor,execution-reference P076 semantic case: p076_concurrent_grow_and_mixed_reads_copy_one_locked_image
- compact-executor,execution-reference P076 semantic case: p076_guest_scalar_vector_and_lane_reads_use_fixed_shared_path
- compact-executor,execution-reference P076 semantic case: p076_source_fuel_cancellation_oob_and_alias_growth_preserve_effects
- existing shared-coordination case: t133_cancel_infinite_wait_removes_registration
- existing shared-coordination case: t133_aliases_counted_notification_and_registration_races
- existing shared-coordination case: t130_two_memory_sc_store_buffering_and_fence_visibility
- existing shared-coordination case: t131_atomic_subword_and_alignment
- existing shared-coordination case: t133_wait_mismatch_timeout_notify
- existing shared-coordination case: t130_concurrent_updates_and_distinct_memories
- performance-counters P025 diagnostic case: p025_unaligned_scalar_widths_sign_extension_and_float_bits_match
- performance-counters P025 diagnostic case: p025_load_boundaries_and_effective_address_overflow_preserve_prior_store
- performance-counters P025 diagnostic case: p025_shared_ordinary_load_and_public_host_read_keep_their_contracts
- performance-counters P025 diagnostic case: p025_memory64_checked_add_and_custom_page_boundaries_preserve_prefix
- performance-counters,execution-reference P026 diagnostic case: p026_extension_loads
- performance-counters,execution-reference P026 diagnostic case: p026_splat_zero_loads
- performance-counters,execution-reference P026 diagnostic case: p026_lanes_and_oob_prefix
- performance-counters,execution-reference P026 diagnostic case: p026_shared_vectors
- performance-counters,execution-reference P026 diagnostic case: p026_reference_extending_load_uses_same_checked_backend
- performance-counters,execution-reference,compact-executor P043 diagnostic case: p043_every_scalar_load_width_sign_and_raw_float_has_exact_fuel_prefix
- performance-counters,execution-reference,compact-executor P043 diagnostic case: p043_all_nine_store_kinds_have_independent_full_memory_prefix_images
- performance-counters,execution-reference,compact-executor P043 diagnostic case: p043_full_boundary_ranges_trap_before_side_effect_and_vector_memory_falls_back
- performance-counters,execution-reference,compact-executor P043 diagnostic case: p043_memory64_overflow_trap_keeps_every_preceding_store_and_fuel_priority
- performance-counters,execution-reference,compact-executor P043 diagnostic case: p043_one_byte_pages_use_byte_lengths_and_preserve_last_byte_store_prefix
- performance-counters,execution-reference,compact-executor P043 diagnostic case: p043_shared_aliases_distinct_instances_and_host_growth_refresh_live_ranges
- performance-counters,execution-reference,compact-executor P043 diagnostic case: p043_unshared_growth_and_memory_cursors_materialize_identical_source_state

## Verification

Activate Rust as described in EXECUTION.md. The required configurations and full close workflow are listed below. The four P076 cases must pass with default features and with `compact-executor,execution-reference`; the existing shared-coordination cases and diagnostics retain their declared feature sets. The source-bound verification also builds a runtime-only decoded production executable, checks default and Compact companion budgets, validates dependencies, and binds compilation, fixtures, current sources, the archived predecessor, v2 freeze and baseline adapter.

```sh
cargo test --locked -p wtime-runtime --lib p076_ -- --nocapture
cargo test --locked -p wtime-runtime --lib --features compact-executor,execution-reference p076_ -- --nocapture
python3 -B tools/verification/producers/runtime_production.py --mode decoded --output target/performance/P076/production
python3 -B tools/checks/build_budget.py --output target/performance/P076/build.json
cargo test --locked -p wtime-runtime --lib shared_memory::tests:: -- --nocapture
cargo test --locked -p wtime-runtime --test performance --features performance-counters p025_ -- --nocapture
cargo test --locked -p wtime-runtime --test performance --features performance-counters,execution-reference p026_ -- --nocapture
cargo test --locked -p wtime-runtime --test performance --features performance-counters,execution-reference,compact-executor p043_ -- --nocapture
python3 -B tools/checks/build_budget.py --features wtime-runtime/compact-executor --output target/performance/P076/compact-companion-build.json
python3 -B tools/verify.py plan --contract docs/benchmarks/performance/P076-v2-verification.json
python3 -B tools/verify.py dev --contract docs/benchmarks/performance/P076-v2-verification.json --group semantics --execute
python3 -B tools/verify.py dev --contract docs/benchmarks/performance/P076-v2-verification.json --group tier-semantics --execute
python3 -B tools/verify.py dev --contract docs/benchmarks/performance/P076-v2-verification.json --group diagnostics-p025 --execute
python3 -B tools/verify.py dev --contract docs/benchmarks/performance/P076-v2-verification.json --group diagnostics-p026 --execute
python3 -B tools/verify.py dev --contract docs/benchmarks/performance/P076-v2-verification.json --group diagnostics-p043 --execute
python3 -B tools/verify.py dev --contract docs/benchmarks/performance/P076-v2-verification.json --group tooling --execute
python3 -B tools/verify.py close --contract docs/benchmarks/performance/P076-v2-verification.json --output target/performance/P076/complete-v2 --execute
```

The measure stage invokes `focused_compare.py` with the frozen v2 manifest, semantic receipt, current-harness predecessor adapter, production binary, `--cpu 1`, and `--thread-cpus 1,2`. The analyze stage computes candidate statistics once. Do not use a second raw analyzer or retime unchanged workloads after analysis-only repairs.

## Measurement and decision

Selection: `threads`. Explicit execution setting: `decoded`. Decision rule: `measured_promotion`.

The v2 selection contains 14 rows, 16 row-stage groups and 960 complete pairs (two runs × 30 pairs for each group). Targeted rows are `threads_ordinary_rw_n32` and `threads_ordinary_rw_n1024`, each at `steady_state_group`; the held-out row is `threads_atomic_load_store_n32`. Protected thread controls are `threads_atomic_load_store_n1024`, `threads_atomic_add_same_w1_n32`, `threads_atomic_add_same_w4_n1024`, `threads_atomic_add_distinct_w4_n1024`, and `threads_sc_store_buffering`. Protected scalar/cold rows include `scalar_dynamic_api_n32`, `call_overhead` at prepare/first_call/load_and_call, and `prepare_large` at prepare. The `memory_widths_predictable`, `memory_widths_read_scan`, and `simd_lane_memory_n1000` controls measure neighboring unshared paths.

Thread groups run with the predeclared CPU mask 1,2; scalar and cold controls use CPU 1. The gate calibrates complete groups to at least 20 ms on both sides, with a per-worker batch cap of 128; SC uses complete 1024-trial groups. Each timed pair retains all 21 worker trials or 1,024 SC trials. Setup and readback are outside timed intervals. These internal worker/trial observations are not independent outer pairs. Instrumentation stays off in production timing.

Timing claims cover ordinary scalar shared reads only. Shared vector/lane reads receive source-bound semantic coverage. Wait/notify throughput is outside this changed path, while all existing wait cleanup, atomic, aliasing and SC semantic checks remain required. Preserve held-out and protected controls; do not claim a speed result until full verification, both paired runs and the single statistics gate complete.

## Candidate gate schedule

P076-before and the initial selection remain preserved. Before runtime changes, verify the pre-edit archive and freeze the strengthened v2 selection. Run all 14 rows in 16 groups in two independent runs with at least 30 complete outer pairs per group and 10,000 bootstrap resamples. The baseline adapter at `target/performance/P076/baseline-adapter/attempt-0001/adapter` uses the current harness with the immutable 109-source predecessor core. Compare only to this immediate predecessor; P001 remains for milestone totals. Retain full raw timing and worker-trial evidence. Keep wait semantics and all P025/P026/P043 diagnostics even though wait throughput is outside the timed selection.

## Before editing

The pre-edit archive at `target/performance/P076-before` and initial selection are already retained. Verify/reuse them; do not replace different bytes. Freeze the strengthened v2 selection before any runtime source edit. The freeze output is the third positional argument.

```sh
python3 -B tools/wtime-bench/baseline.py --output target/performance/P076-before
python3 -B tools/wtime-bench/selection_manifest.py freeze docs/benchmarks/performance/P076-v2-selection.json target/performance/P076-before/manifest.json target/performance/P076/selection-freeze-v2.json
```

## Executed result

The full P076 gate completed with outcome **inconclusive**; it did not support a 5% improvement or promote the change. Across two runs, the 14-row, 16-group selection produced 960 unique complete pairs. The targeted ordinary-read candidate/baseline median ratio was 1.10659784 (95% interval 0.94123693–1.22866035) in run 1 and 1.04057869 (0.92572437–1.26005572) in run 2. Both intervals span faster and slower outcomes, so the aggregate has no supported 5% gain. The gate found protected-control uncertainty as well. Paired statistics were computed once with 10,000 bootstrap resamples.

Full verification passed all 30 named test executions: the four P076 tests in default and optional `compact-executor,execution-reference` configurations, six existing shared-coordination tests, four P025 diagnostics, five P026 diagnostics, and seven P043 diagnostics. The development reruns for restored code also passed: shared coordination (6, attempt 0013), P025 (4, attempt 0014), P026 (5, attempt 0015), and P043 (7, attempt 0016). Both default and Compact companion build budgets passed (60 s clean, 5 s leaf, 5 MiB stripped limits); dependency closure passed with zero violations.

Rollback restored the exact 109-source predecessor represented by P076-before and refreshed timestamps. The seven changed candidate source/test files remain preserved at `target/performance/P076/rollback/attempt-0001/candidate`. Canonical analysis is `/workspace/wtime-specialists/target/verification/runs/P076/analyze/attempt-0002/statistics.json`; canonical completion is `/workspace/wtime-specialists/target/performance/P076/shared-finalization/attempt-0002/completion.json`; publication is `/workspace/wtime-specialists/target/verification/invocations/publish/attempt-0005`, with the primary task snapshot at `/workspace/wtime/target/performance/TASK-status/P076-publication-0001`.

No speedup or default-adoption claim follows from this inconclusive result.

## Completion evidence

The full contract requires the 30 named case executions, default and optional P076 semantic configurations, existing shared-coordination checks, P025/P026/P043 diagnostic groups, default/Compact budgets and dependency check. Keep outer paired counts separate from complete worker/SC groups, report the single analysis decision, and state the shared-host/CPU-isolation limitation. The published gate outcome and immutable completion evidence are recorded above and in `target/performance/P076/shared-finalization/attempt-0002/completion.json`.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime-specialists/target/performance/P076/shared-finalization/attempt-0002/evidence.json": "e99166eb22b7c624c514abfb37ee8d13147d7e04121509817bffcb19b3ee1fe7"
  },
  "evidence": [
    {
      "bytes": 2723,
      "path": "/workspace/wtime-specialists/target/performance/P076/shared-finalization/attempt-0002/evidence.json",
      "sha256": "e99166eb22b7c624c514abfb37ee8d13147d7e04121509817bffcb19b3ee1fe7"
    }
  ],
  "outcome": "inconclusive",
  "status": "done",
  "task": "P076"
}
```
