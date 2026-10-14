# P047 — Fuse a bounded set of local and constant arithmetic patterns

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

[P019 — Define bounded optimizer effect and source expansion metadata](../01-execution-contracts/P019-effect-certificates.md)

[P020 — Centralize exact source instruction accounting](../01-execution-contracts/P020-exact-execution-accounting.md)

[P037 — Map compact instructions to source events and control targets](../03-compact-executor/P037-compact-source-maps.md)

[P038 — Lower constants, locals and common integer operations](../03-compact-executor/P038-compact-scalar-lowering.md)

[P046 — Decide whether compact execution earns default use](../03-compact-executor/P046-compact-milestone.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/execution/compact/fuse.rs`
- `crates/wtime-runtime/src/execution/compact/execute.rs`
- `crates/wtime-runtime/tests/performance/p047.rs`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

Initial fusions cover local.get/local.get/i32.add and local.get/i32.const/i32.add/local.set only; each expansion retains source events and can stop at any original boundary.

## Implementation recipe

0. Before runtime edits, create docs/benchmarks/performance/P047-selection.json and docs/benchmarks/performance/P047-verification.json; freeze the exact target small/large, held-out, protected and cold rows, and bind all semantic cases to successful full verification receipts.

1. Match only same-block validated nontrapping integer/local sequences with no branch target entering the middle and no reference/root transition.
2. Represent source expansion plus transient stack peak; execute all required fuel/cancellation/coverage events even if physical stack traffic is eliminated.
3. Provide a source-prefix fallback when fuel/cancellation/limits interrupt a fusion, reconstructing locals/operands at the exact original point.
4. Benchmark original and held-out arithmetic/constant families and cap generated variants to prevent combinatorial Rust/code-size growth.
5. Forward the explicit `compact` setting through the harness/policy integrator and record actual optimized/fallback counts. Persistent research selectors are nondefault; reversible changes to an existing tier use before/after artifacts and are selected only by their gate.

## Meaningful cases

- Fuel interruption at each constituent instruction gives identical state.
- A branch target or reference local forbids the unsafe fusion.
- Held-out constants and wrapping overflow match the source program.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime --features compact-executor,execution-reference p047_ -- --nocapture
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml --features compact-executor -j 4
python3 tools/checks/build_budget.py --output target/performance/P047/build.json
python3 tools/checks/build_budget.py --features wtime-runtime/compact-executor --output target/performance/P047/optional-build.json
python3 -B tools/verify.py plan --contract docs/benchmarks/performance/P047-verification.json
python3 -B tools/verify.py close --contract docs/benchmarks/performance/P047-verification.json --output target/performance/P047/complete --execute
```

Declare the semantic-test and build commands above in the appropriate contract phases;
run `plan` and `close` externally to drive that contract. Bind full transitive
compilation/fixture inputs and require every semantic
case declared in the selection manifest. Its `measure` phase uses
`focused_compare.py --manifest docs/benchmarks/performance/P047-selection.json`
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
python3 tools/wtime-bench/baseline.py --mode compact --output target/performance/P047-before
python3 -B tools/wtime-bench/selection_manifest.py freeze docs/benchmarks/performance/P047-selection.json target/performance/P047-before/manifest.json --output target/performance/P047/selection-freeze.json
```

Create and validate the task-owned selection manifest before these commands.
The freezer requires the archived core to match the live predecessor, so run
it before the first runtime edit. The shared contract revalidates this receipt
in its `freeze` phase without replacing it after implementation.

## Executed result

The full P047 gate completed with outcome **inconclusive** after one statistics computation over 1,500 unique paired samples (two runs, 30 pairs per declared group). The targeted warm-call portfolio median ratios were 0.78600466 (95% interval 0.75059993–0.81349370) in run 1 and 0.78787446 (0.76205491–0.83081033) in run 2, about 21% lower candidate time in both runs. The complete decision remained inconclusive because 15 protected, cold, and non-target group regression bounds were inconclusive. This result does not authorize default adoption or a Wasmtime comparison.

Full Compact runtime verification passed 347 cases with zero failures or skips, including all seven P047 focused checks: `p047_cancel_and_materialize`, `p047_branch_entry_and_peak_stack`, `p047_every_source_fuel_prefix`, `p047_fused_results_and_overflow`, `p047_selected_accumulator_and_dependent_shapes_execute_verified_groups`, `p047_corrupt_or_over_budget_declines`, and `p047_internal_cancellation_materializes_the_completed_pure_prefix`. Default and optional `compact-executor` builds passed their budgets and dependency checks. The default build used 15.0455 s clean, 0.5912 s leaf, and 1,000,712 stripped bytes; the optional build used 11.0066 s clean, 0.5593 s leaf, and 1,080,560 stripped bytes.

The implementation is retained only behind the existing `compact-executor` feature and explicit `ExecutionPolicy::Compact`. It fuses two bounded pure i32 patterns with no dependency or unsafe-code additions. `Engine::new` and `Portable` still select Decoded; the compact module and dispatch remain feature-gated, and no fusion table is prepared on the default path. Four default-guard P036 checks passed in development attempt 0071. Retention is justified by the supported targeted gain and the bounded planner/source-prefix interface used by P048, while the unresolved protected and cold uncertainty remains explicit. This is an opt-in experiment, not promotion.

Analysis: `target/verification/runs/P047/analyze/attempt-0001/statistics.json`. Verification: `target/verification/runs/P047/verify/attempt-0001/receipt.json`. Retention justification: `target/performance/P047/experimental-retention/attempt-0001/receipt.json`. Completion: `target/performance/P047/shared-finalization/attempt-0002/completion.json`. Publication: `target/verification/invocations/publish/attempt-0018`, with primary task snapshot `target/performance/TASK-status/P047-publication-0001`.

## Completion evidence

The published status and immutable evidence record are shown below. Preserve the gate's inconclusive outcome, opt-in-only disposition, and all source, build, selection, timing, and runtime verification artifacts. Do not report a speedup, default adoption, or Wasmtime result.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/P047/shared-finalization/attempt-0002/evidence.json": "d0da913b38f68e6c94e9cea2b733fb01ab7b48567d5b4cdf9c0108dfa0cac10d"
  },
  "evidence": [
    {
      "bytes": 2592,
      "path": "/workspace/wtime/target/performance/P047/shared-finalization/attempt-0002/evidence.json",
      "sha256": "d0da913b38f68e6c94e9cea2b733fb01ab7b48567d5b4cdf9c0108dfa0cac10d"
    }
  ],
  "outcome": "inconclusive",
  "status": "done",
  "task": "P047"
}
```
