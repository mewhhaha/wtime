# P048 — Fuse integer compare and conditional control transfers

Status: **done**

Outcome: **rejected**

Kind: `candidate` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P002 — Compare separately built baseline and candidate binaries](../00-measurement/P002-paired-driver.md)

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P008 — Add scaling, branch and call benchmark families](../00-measurement/P008-scalar-workloads.md)

[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)

[P016 — Add explicit executor policies and test integration hooks](../01-execution-contracts/P016-execution-policy.md)

[P040 — Lower structured control to resolved jump and result plans](../03-compact-executor/P040-compact-control-plans.md)

[P047 — Fuse a bounded set of local and constant arithmetic patterns](P047-local-arithmetic-fusions.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C2, C3, C4, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/execution/compact/fuse.rs`
- `crates/wtime-runtime/src/execution/compact/execute.rs`
- `crates/wtime-runtime/tests/performance/p048.rs`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

Compare/br_if and compare/if plans avoid materializing an intermediate numeric predicate while retaining original predicate stack state for interrupted execution.

## Implementation recipe

0. Before runtime edits, create docs/benchmarks/performance/P048-selection.json and docs/benchmarks/performance/P048-verification.json; freeze the exact target small/large, held-out, protected and cold rows, and bind all semantic cases to successful full verification receipts.

1. Start with i32/i64 eq/ne/lt signed/unsigned followed by one conditional branch; preserve source branch-result move plans.
2. Reject multi-target entry, reference comparison, catch/control boundaries and unsupported predicate uses; don't infer branch predictability from hints alone.
3. Retain predicate materialization at a fuel/cancel stop between compare and branch and logical label/stack events on both outcomes.
4. Measure predictable/unpredictable/held-out branch families, keeping branch-hint semantics advisory and unchanged.
5. Forward the explicit `compact` setting through the harness/policy integrator and record actual optimized/fallback counts. Persistent research selectors are nondefault; reversible changes to an existing tier use before/after artifacts and are selected only by their gate.

## Meaningful cases

- Taken/untaken signed/unsigned boundary predicates match.
- A stop between compare and branch retains the exact predicate operand.
- Multivalue result branches and hint-present/absent modules preserve behavior.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime --features compact-executor,execution-reference p048_ -- --nocapture
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml --features compact-executor -j 4
python3 tools/checks/build_budget.py --output target/performance/P048/build.json
python3 tools/checks/build_budget.py --features wtime-runtime/compact-executor --output target/performance/P048/optional-build.json
python3 -B tools/verify.py plan --contract docs/benchmarks/performance/P048-verification.json
python3 -B tools/verify.py close --contract docs/benchmarks/performance/P048-verification.json --output target/performance/P048/complete --execute
```

Declare the semantic-test and build commands above in the appropriate contract phases;
run `plan` and `close` externally to drive that contract. Bind full transitive
compilation/fixture inputs and require every semantic
case declared in the selection manifest. Its `measure` phase uses
`focused_compare.py --manifest docs/benchmarks/performance/P048-selection.json`
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

Run these commands before changing the candidate sources. The archive is immutable and captures the immediate predecessor. Retain P001's original archive for milestone totals. For native work, use the named portable control and verify its actual policy/quantum; if this archive already exists with different sources, choose a new recorded run-specific directory and update the comparison path rather than overwriting it.

```sh
python3 tools/wtime-bench/baseline.py --mode compact --output target/performance/P048-before
python3 -B tools/wtime-bench/selection_manifest.py freeze docs/benchmarks/performance/P048-selection.json target/performance/P048-before/manifest.json target/performance/P048/selection-freeze.json
```

Create and validate the task-owned selection manifest before these commands.
The freezer requires the archived core to match the live predecessor, so run
it before the first runtime edit. The shared contract revalidates this receipt
in its `freeze` phase without replacing it after implementation.

## Executed result

P048 completed with outcome **rejected** after one statistics computation. The declared selection represents 1,500 logical pair references; aliases reuse samples, yielding 1,380 unique complete pairs across two runs. The targeted warm-call candidate/baseline ratios were 1.19343217 (95% interval 1.13393741–1.24607920) in run 1 and 1.20111768 (1.14651776–1.25308836) in run 2, approximately 19–20% slower than the immediate predecessor. The targeted lane therefore showed a supported regression, and many additional primary and protected controls also had supported regressions.

Full Compact runtime verification passed all 352 cases with zero failures or skips. Default and optional Compact build budgets and dependency checks passed. The default build used 9.6533 s clean, 0.3793 s leaf, and 1,000,712 stripped bytes; the optional build used 13.9644 s clean, 0.9298 s leaf, and 1,084,288 stripped bytes.

The eight candidate paths were preserved at `target/performance/P048/rollback/attempt-0001/candidate`; rollback restored the exact 111-source core with fresh timestamps, retaining the accepted P047 change only. Seven restored P047 checks passed in development attempt 0073. P048 made no retained runtime change and has no opt-in experiment or performance claim.

Analysis: `target/verification/runs/P048/analyze/attempt-0001/statistics.json`. Verification: `target/verification/runs/P048/verify/attempt-0001/receipt.json`. Rollback: `target/performance/P048/rollback/attempt-0001/receipt.json`. Completion: `target/performance/P048/shared-finalization/attempt-0001/completion.json`. Publication: `target/verification/invocations/publish/attempt-0019`, with primary task snapshot `target/performance/TASK-status/P048-publication-0001`.

## Completion evidence

The published status and immutable evidence record are shown below. Preserve the rejected outcome, exact rollback, and all source, build, selection, timing, and runtime verification artifacts. Do not report a speedup, retained P048 runtime change, or Wasmtime result.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/P048/shared-finalization/attempt-0001/evidence.json": "0d1747200c305e2f7ca507f04e45bbcbf61998b8cea12c95da01404c17b55286"
  },
  "evidence": [
    {
      "bytes": 2649,
      "path": "/workspace/wtime/target/performance/P048/shared-finalization/attempt-0001/evidence.json",
      "sha256": "0d1747200c305e2f7ca507f04e45bbcbf61998b8cea12c95da01404c17b55286"
    }
  ],
  "outcome": "rejected",
  "status": "done",
  "task": "P048"
}
```
