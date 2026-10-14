# P050 — Inline tiny direct numeric leaves with logical call accounting

Status: **done**

Outcome: **inconclusive**

Kind: `candidate` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P002 — Compare separately built baseline and candidate binaries](../00-measurement/P002-paired-driver.md)

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P008 — Add scaling, branch and call benchmark families](../00-measurement/P008-scalar-workloads.md)

[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)

[P016 — Add explicit executor policies and test integration hooks](../01-execution-contracts/P016-execution-policy.md)

[P020 — Centralize exact source instruction accounting](../01-execution-contracts/P020-exact-execution-accounting.md)

[P041 — Resolve direct guest call bindings once per instance](../03-compact-executor/P041-instance-call-plans.md)

[P047 — Fuse a bounded set of local and constant arithmetic patterns](P047-local-arithmetic-fusions.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/execution/compact/inline.rs`
- `crates/wtime-runtime/src/execution/compact/execute.rs`
- `crates/wtime-runtime/tests/performance/p050.rs`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

Only same-instance direct numeric leaves with no host/ref/GC/exception/continuation effects are initially inlined. Logical call/depth/locals/source events remain reconstructible even without a physical frame.

## Implementation recipe

0. Before runtime edits, create docs/benchmarks/performance/P050-selection.json and docs/benchmarks/performance/P050-verification.json; freeze the exact target small/large, held-out, protected and cold rows, and bind all semantic cases to successful full verification receipts.

1. Set small per-function and aggregate expansion limits, reject recursion/dynamic/import targets and predeclare eligibility independently of fixture names.
2. Remap numeric local slots and source origins, initialize locals exactly, and preserve call/return fuel events plus logical max_call_depth checks at original points.
3. Materialize logical callee state at an interrupted source point; if depth/stack/cancellation proof is unavailable, decline inlining instead of ignoring limits.
4. Measure direct-call and code/preparation controls; compare a held-out leaf with division/trap to ensure it is either fully accounted or declined.
5. Forward the explicit `compact` setting through the harness/policy integrator and record actual optimized/fallback counts. Persistent research selectors are nondefault; reversible changes to an existing tier use before/after artifacts and are selected only by their gate.

## Meaningful cases

- Low logical call depth still fails even with no physical native/compact frame.
- Fuel stops inside an inline leaf reconstruct its arguments/locals/return point.
- Recursive/imported/effectful leaves decline and expansion stays within code budgets.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime --features compact-executor,execution-reference p050_ -- --nocapture
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml --features compact-executor -j 4
python3 tools/checks/build_budget.py --output target/performance/P050/build.json
python3 tools/checks/build_budget.py --features wtime-runtime/compact-executor --output target/performance/P050/optional-build.json
python3 -B tools/verify.py plan --contract docs/benchmarks/performance/P050-verification.json
python3 -B tools/verify.py close --contract docs/benchmarks/performance/P050-verification.json --output target/performance/P050/complete --execute
```

Declare the semantic-test and build commands above in the appropriate contract phases;
run `plan` and `close` externally to drive that contract. Bind full transitive
compilation/fixture inputs and require every semantic
case declared in the selection manifest. Its `measure` phase uses
`focused_compare.py --manifest docs/benchmarks/performance/P050-selection.json`
with `--frozen {freeze_dir}/selection.json`, `--semantic-receipt {verify_dir}/receipt.json`,
the immutable predecessor adapter, and the actual production executable/mode.
Its `analyze` phase calls `tools/verify.py statistics` once, supplying that
selection manifest as `--contract`, the paired report, and all build attestations.
Do not run an exhaustive comparison or a separate raw analyzer during this candidate gate.
See the [exact workflow](../../../wtime/docs/verification/fast-performance.md).

## Measurement and decision

Selection: `scalar`. Explicit execution setting: `compact`. Decision rule: `measured_promotion`.

Use the paired protocol and family/control/cold/build rules in [MEASUREMENT.md](../MEASUREMENT.md). Predeclare the targeted family and held-out controls before changing the implementation. Keep instrumentation off in production timings. Adopt only with supported gains in both runs; otherwise record rejected/inconclusive and remove default-path cost. A necessary immutable-interface change may be contract_only with its separate justification.

## Candidate gate schedule

Before implementation, freeze and verify the immediate-predecessor archive and a task-owned predeclaration manifest. Bind the manifest to exact registry workload IDs and input values, optimization shapes, target small/large rows, held-out rows, protected controls, stages, feature/runtime settings, and applicable cold products. Candidate promotion compares against this immediate predecessor only; P001 is reserved for exhaustive milestone totals. Measure the complete declared candidate set in two independent runs with at least 30 complete pairs per row and run, using 10,000 deterministic bootstrap resamples and uninstrumented production binaries. The applicable large-module preparation control and tiny `call_overhead` first-call/load-and-call cold controls are mandatory. Preserve every task-specific semantic, fuel/cancellation, trap/memory, fallback, and optimizer-bound case regardless of the focused timing selection. See [MEASUREMENT.md](../MEASUREMENT.md) for cold-product identity, sample reuse, thresholds, and milestone rules.

## Before editing

Run these commands before changing the candidate sources. The archive is immutable and captures the immediate predecessor. Retain P001's original archive for milestone totals. P050 follows P049's rollback, with P047's opt-in fusion still present, so archive the immediate predecessor under the explicit compact policy. For native work, use the named portable control and verify its actual policy/quantum; if this archive already exists with different sources, choose a new recorded run-specific directory and update the comparison path rather than overwriting it.

```sh
python3 tools/wtime-bench/baseline.py --mode compact --output target/performance/P050-before
python3 -B tools/wtime-bench/selection_manifest.py freeze docs/benchmarks/performance/P050-selection.json target/performance/P050-before/manifest.json --output target/performance/P050/selection-freeze.json
```

Create and validate the task-owned selection manifest before these commands.
The freezer requires the archived core to match the live predecessor, so run
it before the first runtime edit. The shared contract revalidates this receipt
in its `freeze` phase without replacing it after implementation.

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. Executed evidence is recorded below.

## Executed result

P050 completed **done / inconclusive**. The one accepted statistics artifact reports 3,360 unique physical pairs and 3,660 logical pair references across two runs; aliases reused raw pairs and added no samples. The targeted `scalar_direct_calls_n32`/`n10000` warm-call candidate/baseline median ratios were 0.51972298 (95% interval 0.42718021–0.59793480) and 0.48282901 (0.39673729–0.64129041), corresponding to 48.03% and 51.72% lower warm-call time. Both intervals support the targeted gains. No protected or cold regression was supported, but 56 regression bounds remained uncertain, so the official decision is inconclusive. This supports opt-in experimental retention only, not default adoption, a full-portfolio or P001 total, or a Wasmtime comparison.

All 354 full Compact runtime cases and seven named P050 focused checks passed, with zero failures or ignored cases. Default and Compact build-budget checks passed without threshold changes. The final default report measured 27.333 s clean, 0.683 s leaf, 1,000,712 stripped bytes, and 280,156 KiB clean peak RSS; Compact measured 37.936 s clean, 3.814 s leaf, 1,091,296 stripped bytes, and 313,804 KiB clean peak RSS. Two earlier build-budget attempts failed and remain retained. `Engine::new(Config)` still chooses Decoded; the retained leaf inliner and call hook remain behind `compact-executor` and require explicit `ExecutionPolicy::Compact`.

The accepted analysis is attempt `0002`. Attempt `0001` had already computed an analysis but failed its final task-index reference guard after an index mutation; that output was discarded. The analysis-only retry reused the same 3,360 physical pairs and passing build/correctness evidence. Therefore the accepted artifact contains one computation, but the full attempt history did not compute statistics only once.

The [published completion record](../../../wtime/target/performance/P050/shared-finalization/attempt-0003/completion.json) has evidence SHA-256 `8cb480f26bcf239f4929c13d7e9ee414377f59a94349506e3ffa53bfe810d4fe`. The accepted statistics are `/workspace/wtime/target/verification/runs/P050/analyze/attempt-0002/statistics.json`; full verification is `/workspace/wtime/target/verification/runs/P050/verify/attempt-0002/receipt.json`; the opt-in retention justification is `/workspace/wtime/target/performance/P050/experimental-retention/attempt-0001/receipt.json`.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/P050/shared-finalization/attempt-0003/evidence.json": "8cb480f26bcf239f4929c13d7e9ee414377f59a94349506e3ffa53bfe810d4fe"
  },
  "evidence": [
    {
      "bytes": 2431,
      "path": "/workspace/wtime/target/performance/P050/shared-finalization/attempt-0003/evidence.json",
      "sha256": "8cb480f26bcf239f4929c13d7e9ee414377f59a94349506e3ffa53bfe810d4fe"
    }
  ],
  "outcome": "inconclusive",
  "status": "done",
  "task": "P050"
}
```
