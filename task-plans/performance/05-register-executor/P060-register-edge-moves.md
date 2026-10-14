# P060 — Implement safe parallel moves and register control transfers

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

[P058 — Translate numeric stack code into direct slot operands](P058-stack-to-register-lowering.md)

[P061 — Execute checked integer register operations](P061-register-integer-execution.md)

Optional results to review (do not block the portable path):

[P059 — Try conservative temporary-slot reuse without global SSA](P059-bounded-slot-reuse.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/execution/register/control.rs`
- `crates/wtime-runtime/tests/performance/p060.rs`

P130 supplies the checked immutable parallel-move planner and P061 establishes its initial Jump/Branch consumer. P060 remains responsible for the full control-transfer implementation and candidate gate: block/loop/if/end/br_table/return behavior, exact source-event and logical-height semantics, source-prefix interruption, fuel/cancellation/operand-limit materialization, optimized/fallback accounting, paired measurement, build budgets and promotion.

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

Register branch plans perform simultaneous edge-result moves with one bounded scratch slot for cycles; original block/loop/if/end/br events and logical labels remain reconstructible. P130 and P061 establish the checked move-plan foundation and initial Jump/Branch consumer; P060 retains the full control-transfer candidate gate, including structured control, br_table/return, exact source interruption state, cancellation, fuel and operand limits, fallback accounting, paired measurement and promotion.

## Implementation recipe

0. Before runtime edits, create docs/benchmarks/performance/P060-selection.json and docs/benchmarks/performance/P060-verification.json; freeze the exact target small/large, held-out, protected and cold rows, and bind all semantic cases to successful full verification receipts.

1. Generate checked move lists for block results/loop parameters, remove self moves and reject invalid source/destination/type plans.
2. Schedule acyclic moves first and break cycles using one typed numeric scratch slot; do not allocate per branch or overwrite a still-needed source.
3. Implement Jump/Branch/br_table/return with exact logical height/depth/source event behavior and source-prefix interruption.
4. Run multivalue swap/rotate/backedge fixtures and compare control state against compact/reference paths.
5. Forward the explicit `register` setting through the harness/policy integrator and record actual optimized/fallback counts. Persistent research selectors are nondefault; reversible changes to an existing tier use before/after artifacts and are selected only by their gate.

## Meaningful cases

- Two-slot swaps and longer cycles preserve simultaneous results.
- Zero/multivalue loops and every br_table outcome retain correct values.
- Fuel/cancel/operand-limit stops at structural instructions materialize the same state.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime --features register-executor,execution-reference p060_ -- --nocapture
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml --features register-executor -j 4
python3 tools/checks/build_budget.py --output target/performance/P060/build.json
python3 tools/checks/build_budget.py --features wtime-runtime/register-executor --output target/performance/P060/optional-build.json
python3 -B tools/verify.py plan --contract docs/benchmarks/performance/P060-verification.json
python3 -B tools/verify.py close --contract docs/benchmarks/performance/P060-verification.json --output target/performance/P060/complete --execute
```

Declare the semantic-test and build commands above in the appropriate contract phases;
run `plan` and `close` externally to drive that contract. Bind full transitive
compilation/fixture inputs and require every semantic
case declared in the selection manifest. Its `measure` phase uses
`focused_compare.py --manifest docs/benchmarks/performance/P060-selection.json`
with `--frozen {freeze_dir}/selection.json`, `--semantic-receipt {verify_dir}/receipt.json`,
the direct frozen register-mode predecessor archive, and the actual production executable/mode.
Its `analyze` phase calls `tools/verify.py statistics` once, supplying that
selection manifest as `--contract`, the paired report, and all build attestations.
Do not run an exhaustive comparison or a separate raw analyzer during this candidate gate.
See the [exact workflow](../../../wtime/docs/verification/fast-performance.md).

## Measurement and decision

Selection: `scalar`. Explicit execution setting: `register`. Decision rule: `measured_promotion`.

Use the paired protocol and family/control/cold/build rules in [MEASUREMENT.md](../MEASUREMENT.md). Predeclare the targeted family and held-out controls before changing the implementation. Keep instrumentation off in production timings. Adopt only with supported gains in both runs; otherwise record rejected/inconclusive and remove default-path cost. A necessary immutable-interface change may be contract_only with its separate justification.

## Candidate gate schedule

Before implementation, freeze and verify the immediate-predecessor archive and a task-owned predeclaration manifest. Bind the manifest to exact registry workload IDs and input values, optimization shapes, target small/large rows, held-out rows, protected controls, stages, feature/runtime settings, and applicable cold products. Candidate promotion compares against this immediate predecessor only; P001 is reserved for exhaustive milestone totals. Measure the complete declared candidate set in two independent runs with at least 30 complete pairs per row and run, using 10,000 deterministic bootstrap resamples and uninstrumented production binaries. The applicable large-module preparation control and tiny `call_overhead` first-call/load-and-call cold controls are mandatory. Preserve every task-specific semantic, fuel/cancellation, trap/memory, fallback, and optimizer-bound case regardless of the focused timing selection. See [MEASUREMENT.md](../MEASUREMENT.md) for cold-product identity, sample reuse, thresholds, and milestone rules.

## Before editing

Run these commands before changing the candidate sources. The archive is immutable and captures the immediate predecessor. Retain P001's original archive for milestone totals. For native work, use the named portable control and verify its actual policy/quantum; if this archive already exists with different sources, choose a new recorded run-specific directory and update the comparison path rather than overwriting it.

```sh
python3 tools/wtime-bench/baseline.py --mode register --output target/performance/P060-before
python3 -B tools/wtime-bench/selection_manifest.py freeze docs/benchmarks/performance/P060-selection.json target/performance/P060-before/manifest.json --output target/performance/P060/selection-freeze.json
```

Create and validate the task-owned selection manifest before these commands.
The freezer requires the archived core to match the live predecessor, so run
it before the first runtime edit. The shared contract revalidates this receipt
in its `freeze` phase without replacing it after implementation.

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. Executed evidence is recorded below.

## Executed evidence

P060 completed all four shared-contract phases in attempt `0001` (freeze,
verify, measure, analyze), then finalized as close `0074`, finalization
attempt `0001`. The measured candidate was rejected for promotion. Register
control transfers remain only as the nondefault prerequisite interface for
P062, P063, and P064; there is no default-path promotion.

The frozen baseline is the direct P060-before Register-mode archive from source
commit `52a6461`; it uses the exact Register mode with no adapter. Verification
passed 276 tests: 274 register semantic tests, one decoded-default policy test,
and one combined-control test, with zero failures and zero ignored tests. The
full snapshot matched exactly across 65 table prefixes. The tested cases also
covered the production move-cycle oracle, budgets and foreign-entry guards,
and source-fuel, operand-limit, and caller-priority behavior.

The complete candidate measurement covered 9 declared rows in 29 logical
groups, with 1,500 unique physical pairs, 1,740 logical pair references,
30 pairs per row in each of two runs, and 10,000 bootstrap resamples. The
primary warm-call candidate/baseline median ratios were 0.797854968 (95% CI
0.783787–0.808190) and 0.798515119 (95% CI 0.781656–0.814344). The gate
rejected promotion because small br_table load-and-call and large br_table
preparation showed regressions. The tiny call-overhead first-call bound was
inconclusive.

Both default and optional builds passed their hard budgets with zero external
dependencies. Clean build times were 2.80252 s (default) and 3.29044 s
(optional Register); stripped binaries were 1,000,712 and 1,062,904 bytes.

Durable artifacts and hashes are indexed by
[`P060-records.json`](../../../wtime/docs/benchmarks/performance/P060-records.json).
The frozen selection is in [`P060-freeze.json`](../../../wtime/docs/benchmarks/performance/P060-freeze.json),
paired measurement in [`P060-comparison.json.gz`](../../../wtime/docs/benchmarks/performance/P060-comparison.json.gz),
analysis in [`P060-statistics.json`](../../../wtime/docs/benchmarks/performance/P060-statistics.json),
build attestations in [`P060-default-build.json`](../../../wtime/docs/benchmarks/performance/P060-default-build.json),
[`P060-register-build.json`](../../../wtime/docs/benchmarks/performance/P060-register-build.json),
and [`P060-production-build.json`](../../../wtime/docs/benchmarks/performance/P060-production-build.json),
and final publication in [`P060-completion.json`](../../../wtime/docs/benchmarks/performance/P060-completion.json)
and [`P060-evidence.json`](../../../wtime/docs/benchmarks/performance/P060-evidence.json).
The records index includes the four phase-receipt paths and hashes.
