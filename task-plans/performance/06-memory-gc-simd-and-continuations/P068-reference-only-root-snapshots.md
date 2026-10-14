# P068 — Copy only references into host and collection root snapshots

Status: **done**

Outcome: **inconclusive**

Kind: `candidate` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P002 — Compare separately built baseline and candidate binaries](../00-measurement/P002-paired-driver.md)

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P011 — Measure reference, GC and exception paths](../00-measurement/P011-gc-exception-workloads.md)

[P125 — Add source-bound host root-snapshot workloads](P125-host-root-workloads.md)

[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)

[P016 — Add explicit executor policies and test integration hooks](../01-execution-contracts/P016-execution-policy.md)

[P022 — Add adversarial state and interaction tier fixtures](../01-execution-contracts/P022-state-differential-fixtures.md)

[P031 — Reuse invocation scratch capacity safely under host reentry](../02-structural-wins/P031-activation-scratch-pool.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C2, C3, C4, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/lib.rs`
- `crates/wtime-runtime/src/root_snapshots.rs`
- `crates/wtime-runtime/src/continuation.rs`
- `crates/wtime-runtime/tests/performance/p068_private.rs`
- `crates/wtime-runtime/tests/performance.rs`
- `tools/wtime-bench/src/gc_roots_driver.rs`
- `tools/wtime-bench/src/protocol.rs`
- `tools/wtime-bench/root_workloads.py`
- `tools/wtime-bench/root_compare.py`
- `tools/wtime-bench/focused_compare.py`
- `tools/wtime-bench/candidate_gate.py`
- `tools/wtime-bench/selection_manifest.py`
- `tools/wtime-bench/execution_identity.py`
- `tools/wtime-bench/semantic_receipt.py`
- `tools/wtime-bench/workloads.json`
- `tools/wtime-bench/fixtures/gc/roots/**`
- `tools/wtime-bench/tests/test_root_workloads.py`
- `tools/wtime-bench/tests/test_root_compare.py`
- `docs/benchmarks/performance/P068-numeric-selection.json`
- `docs/benchmarks/performance/P068-numeric-verification.json`

Collector globals already exclude numeric Values, as confirmed by source inspection. `crates/wtime-runtime/src/gc_exec.rs` is unchanged. One integrator owns lib.rs and tests/performance.rs registration; leaf owners submit bounded integration hunks.

## Input/output contract

Active host snapshots and temporary collector roots omit numeric Values but retain every reference currently traced, including exception, continuation, and descriptor roots. Snapshot buffers are owned, never borrow Store slices across host callbacks, are bounded to four buffers of at most 1024 Values each, and are cleared after every callback outcome. Collector globals already exclude numeric Values, so gc_exec.rs requires no rewrite. The candidate preserves all existing root-provider semantics and public liveness policy.

## Implementation recipe

1. Before runtime edits, verify the immutable P068-before archive and its matching current pre-edit source map. Preserve the legacy P068-selection.json, original selection freeze, reports, and failed attempts as historical immutable evidence. Use docs/benchmarks/performance/P068-numeric-selection.json and docs/benchmarks/performance/P068-numeric-verification.json; freeze to target/performance/P068/numeric-selection-freeze.json with the positional output argument. The baseline adapter is target/verification/runs/P125/verify/attempt-0002/proof/adapter, built from P068-before with the current harness.
2. Implement owned root snapshot storage in root_snapshots.rs and its continuation snapshot integration in continuation.rs. Filter numeric Values from active owned host and temporary collector snapshots while preserving every RefValue variant and current root lifetimes. Do not modify gc_exec.rs: inspection confirms collector globals already exclude numeric Values.
3. Use at most four owned snapshot buffers, each bounded to 1024 Values. Clear snapshots and release roots after successful callbacks, host errors, traps, suspensions, rejected calls, and unwind paths. Never borrow Store slices across a callback or same-Store reentry.
4. Run six P068 named semantic checks and the two existing T113 root-provider checks: all reference variants remain owned; numeric reentry copies no numeric roots; all callback outcomes release snapshots; the sole host GC argument survives reentry; guest pending references survive collection; failure then success leaves no stale roots; each Store provider is a sole root; nested host reentry retains arguments and pending results.
5. Freeze the 13-row P068-numeric selection: two targeted numeric rows (64,n32 and 512,n32) at first_call, warm_call, load_and_call; two held-out numeric rows (256,n17 and 512,n0) at warm_call; three protected reference rows (64,n32; 512,n32; 256,n17) at warm_call; three protected legacy GC rows (gc_root_lifecycle_n32, gc_continuation_root_n32, gc_host_reentry_collect_n32) at load_and_call; scalar_dynamic_api_n32 at warm_call; call_overhead at prepare, first_call, load_and_call; and prepare_large at prepare. This is 13 workload rows and 19 row-stage groups.
6. Compare candidate and immediate-predecessor adapter over the exact 19 groups with two independent runs of 30 complete pairs per group, for 1140 pairs total. Keep instrumentation off in production timing; retain all semantic checks and cold/build controls. The paired report is evidence for P068 measured promotion only after its full gate; foundation adapter smokes are not a speed claim.
7. Register p068_private.rs in tests/performance.rs and bind the six P068 semantic cases plus both T113 provider cases, transitive runtime/test sources, fixture inputs, selection, freeze, adapter, and current harness in P068-numeric-verification.json. Preserve the default core as safe Rust with zero external dependencies.

## Meaningful cases

- p068_all_reference_variants_remain_owned: sole references in each provider survive collection and all RefValue variants remain owned.
- p068_numeric_reentry_has_no_copied_roots: numeric-heavy host reentry copies no numeric roots while outcomes stay unchanged.
- p068_callback_outcomes_release_snapshots: host error, trap, suspension rejection, and unwind clear snapshots and do not leak roots.
- p068_sole_host_gc_argument_survives_reentry: the sole host GC argument remains live throughout callback collection and same-Store reentry.
- p068_guest_pending_reference_survives_collection: a pending guest reference remains rooted while host callback collection occurs.
- p068_failure_then_success_has_no_stale_roots: a failed callback followed by a successful invocation observes no stale root snapshot.
- t113_each_store_provider_is_a_sole_root: each existing Store root provider independently preserves its sole referenced object.
- t113_nested_host_reentry_retains_arguments_and_pending_results: nested same-Store reentry retains callback arguments and pending guest results.

## Verification

Activate Rust as described in EXECUTION.md. The focused tests execute all eight named P068/T113 cases. Optimized-path tests prove the snapshot pool path ran. Use the numeric contract; the legacy P068 selection and reports remain historical only.

```sh
cargo test --locked -p wtime-runtime p068_ -- --nocapture
cargo test --locked -p wtime-runtime t113_ -- --nocapture
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml -j 4
python3 tools/checks/build_budget.py --output target/performance/P068/build.json
python3 -B tools/verify.py plan --contract docs/benchmarks/performance/P068-numeric-verification.json
python3 -B tools/verify.py close --contract docs/benchmarks/performance/P068-numeric-verification.json --output target/performance/P068/numeric-complete --execute
```

The new measurement manifest is `docs/benchmarks/performance/P068-numeric-selection.json`; its verification contract is `docs/benchmarks/performance/P068-numeric-verification.json`. The frozen selection output is `target/performance/P068/numeric-selection-freeze.json`. The freezer takes the output path as its third positional argument, without `--output`. The measure phase uses the current-harness adapter at `target/verification/runs/P125/verify/attempt-0002/proof/adapter` and the actual production executable. The candidate-gate analysis computes paired statistics once and binds all build attestations.



## Measurement and decision

Selection: `P068-numeric-selection`, family `gc-root-snapshots` with protected `gc-exceptions`, scalar, and original-MVP controls. Explicit execution setting: `decoded`. Decision rule: `measured_promotion`.

The selection has 13 workload rows and 19 row-stage groups. Target numeric rows are `gc_root_snapshots_numeric_64_n32` and `gc_root_snapshots_numeric_512_n32`, each at `first_call`, `warm_call`, and `load_and_call`. Held-out numeric rows are `gc_root_snapshots_numeric_256_n17` and `gc_root_snapshots_numeric_512_n0` at `warm_call`. Protected reference rows are `gc_root_snapshots_reference_64_n32`, `gc_root_snapshots_reference_512_n32`, and `gc_root_snapshots_reference_256_n17` at `warm_call`. Protected legacy GC rows `gc_root_lifecycle_n32`, `gc_continuation_root_n32`, and `gc_host_reentry_collect_n32` use `load_and_call`; `scalar_dynamic_api_n32` uses `warm_call`; `call_overhead` uses `prepare`, `first_call`, and `load_and_call`; and `prepare_large` uses `prepare`.

Run two independent runs with 30 complete pairs per row-stage group, for 1140 pairs total. The P068-before archive is the immediate predecessor. The paired baseline is the current-harness adapter built from that archive at `target/verification/runs/P125/verify/attempt-0002/proof/adapter`. Stage timing follows the shared contract: `prepare` times compilation; `instantiate` times Store, link, and instance setup; `first_call` and `warm_call` time guest invocation, including the host callback collection and same-Store reentry; `load_and_call` times the combined path. Oracle calculation, reset, and separate post-invocation cleanup collection stay outside timed intervals. The in-callback collection is part of the guest invocation and is timed. Foundation P125 paired smoke records establish source/harness identity only and are not a speed gate. The P068 numeric candidate decision uses the full paired protocol and build/dependency budgets. Do not claim a speed result or promote before the numeric verification, measurement, and analysis phases pass.

## Candidate gate schedule

Before runtime edits, verify the immutable `target/performance/P068-before` archive and source map, then validate the current numeric selection freeze. Keep `docs/benchmarks/performance/P068-selection.json`, its original freeze and reports, and all failed attempts immutable as historical evidence. Use the new `docs/benchmarks/performance/P068-numeric-selection.json`, `docs/benchmarks/performance/P068-numeric-verification.json`, and `target/performance/P068/numeric-selection-freeze.json`. Freeze the numeric manifest against `target/performance/P068-before/manifest.json` with the output path supplied positionally. Compare only to the immediate predecessor adapter; P001 remains for exhaustive milestone totals. Execute the complete 19 selected row-stage groups in two independent runs of 30 complete pairs each and use the task contract's deterministic bootstrap analysis once. Preserve all eight named semantic cases and protected controls. Keep instrumentation off in production timings.

## Before editing

The immediate predecessor archive already exists at `target/performance/P068-before`; verify/reuse it without replacing different bytes. Retain P001's original archive for milestone totals. Freeze the new numeric selection to `target/performance/P068/numeric-selection-freeze.json` using the freezer's positional output argument. The old `P068-selection.json` and old selection freeze/report paths are retained as immutable historical evidence.

```sh
python3 tools/wtime-bench/baseline.py --output target/performance/P068-before
python3 -B tools/wtime-bench/selection_manifest.py freeze docs/benchmarks/performance/P068-numeric-selection.json target/performance/P068-before/manifest.json target/performance/P068/numeric-selection-freeze.json
```

The numeric verification contract revalidates the frozen receipt. Do not use `--output` for the `selection_manifest.py freeze` output path.

## Executed result

The full numeric P068 gate completed with outcome **inconclusive**. Across its 1,140 pairs, the numeric lane ratios were 0.4975 and 0.4649, while protected-control uncertainty prevented promotion. The runtime change was rolled back exactly; the candidate remains at `target/performance/P068/rollback/attempt-0001/candidate`. P125's source-bound workload foundation is integrated in the primary checkout, while the P068 runtime candidate remains rolled back, so no P068 runtime patch is pending integration. The legacy P068 selection, original freeze, historical reports, and failed attempts remain unchanged. The published completion evidence is `target/performance/P068/numeric-finalization/attempt-0001/completion.json`.

## Completion evidence

Record actual command/status/settings/counts, all selected workload and stage IDs, source/binary/fixture/selection/freeze/adapter/harness hashes, eight named semantic cases, pair counts, the single analysis result, build/dependency deltas, and remaining limitations. Retain the legacy P068 selection, original freeze, historical reports, and failed attempts unchanged. The completed numeric gate was inconclusive and did not promote the runtime candidate; see Executed result and the immutable completion evidence.

The planning-time commands above are not evidence; the completed gate and its outcome are recorded in Executed result and the immutable completion evidence.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime-specialists/target/performance/P068/numeric-finalization/attempt-0001/evidence.json": "8660e2fb7ab6405de20e0f8a507c963a2c2780b6da14688366a16b3379b1837f"
  },
  "evidence": [
    {
      "bytes": 2200,
      "path": "/workspace/wtime-specialists/target/performance/P068/numeric-finalization/attempt-0001/evidence.json",
      "sha256": "8660e2fb7ab6405de20e0f8a507c963a2c2780b6da14688366a16b3379b1837f"
    }
  ],
  "outcome": "inconclusive",
  "status": "done",
  "task": "P068"
}
```
