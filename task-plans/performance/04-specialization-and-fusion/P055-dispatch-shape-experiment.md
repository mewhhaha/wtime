# P055 — Compare safe match, split dispatch and handler-table shapes

Status: **todo**

Outcome: **pending**

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

- `crates/wtime-runtime/src/execution/compact/dispatch_experiment.rs`
- `tools/wtime-bench/workloads.json`
- `crates/wtime-runtime/tests/performance/p055.rs`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

Nondefault dispatch variants execute the same compact op stream: current match, hot/cold split match, and safe handler functions returning a loop action. No recursive Rust guest stack or computed-goto unsafe trick is introduced.

## Implementation recipe

1. Implement only the common numeric/local/control subset in all variants; route other tags through one identical barrier path and account for indirect-call overhead.
2. Use an iterative outer loop and explicit action/cursor, not handler recursion; retain the same source accounting and checked operands.
3. Record compiler/assembly/code-size/branch-sampling evidence where available and predeclare identical workload/op streams.
4. Promote at most one shape if both runs/control/build gates pass; delete or keep other shapes solely as isolated experiment evidence.
5. Forward the explicit `compact-dispatch-experiment` setting through the harness/policy integrator and record actual optimized/fallback counts. Persistent research selectors are nondefault; reversible changes to an existing tier use before/after artifacts and are selected only by their gate.

## Meaningful cases

- Identical source sequences produce identical op/event counts across variants.
- Deep guest loops do not consume the Rust call stack.
- Barrier/trap/fuel paths retain exact state and small/large code distributions are both measured.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime --features compact-executor,experimental-executors,execution-reference p055_ -- --nocapture
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml --features compact-executor,experimental-executors -j 4
python3 tools/wtime-bench/compare.py --baseline target/performance/P055-before --candidate tools/wtime-bench/target/release/wtime-bench --candidate-mode compact-dispatch-experiment --selection scalar --output target/performance/P055
python3 tools/wtime-bench/analyze.py --input target/performance/P055 --output target/performance/P055/analysis.json
python3 tools/checks/build_budget.py --output target/performance/P055/build.json
python3 tools/checks/build_budget.py --features wtime-runtime/compact-executor --output target/performance/P055/optional-build.json
python3 tools/wtime-bench/gates.py --task P055 --comparison target/performance/P055 --build target/performance/P055/build.json --optional-build target/performance/P055/optional-build.json --output target/performance/P055/decision.json
```

## Measurement and decision

Selection: `scalar`. Explicit execution setting: `compact-dispatch-experiment`. Decision rule: `measured_promotion`.

Use the paired protocol and family/control/cold/build rules in [MEASUREMENT.md](../MEASUREMENT.md). Predeclare the targeted family and held-out controls before changing the implementation. Keep instrumentation off in production timings. Adopt only with supported gains in both runs; otherwise record rejected/inconclusive and remove default-path cost. A necessary immutable-interface change may be contract_only with its separate justification.

## Before editing

Run these commands before changing the candidate sources. The archive is immutable and captures the immediate predecessor. Retain P001's original archive for milestone totals. For native work, use the named portable control and verify its actual policy/quantum; if this archive already exists with different sources, choose a new recorded run-specific directory and update the comparison path rather than overwriting it.

```sh
python3 tools/wtime-bench/baseline.py --output target/performance/P055-before
```

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. No commands in this planning file have been executed as implementation evidence.
