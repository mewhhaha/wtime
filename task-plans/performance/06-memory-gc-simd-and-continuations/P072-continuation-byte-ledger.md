# P072 — Maintain continuation bytes incrementally and transactionally

Status: **todo**

Outcome: **pending**

Kind: `candidate` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P002 — Compare separately built baseline and candidate binaries](../00-measurement/P002-paired-driver.md)

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P012 — Measure continuation capture, resume and accounting](../00-measurement/P012-continuation-workloads.md)

[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)

[P016 — Add explicit executor policies and test integration hooks](../01-execution-contracts/P016-execution-policy.md)

[P029 — Stop cloning function signatures into every activation](../02-structural-wins/P029-shared-function-metadata.md)

[P031 — Reuse invocation scratch capacity safely under host reentry](../02-structural-wins/P031-activation-scratch-pool.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/continuation.rs`
- `crates/wtime-runtime/src/gc_exec.rs`
- `crates/wtime-runtime/tests/performance/p072.rs`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

Store maintains the exact retained continuation-byte total; creation/capture/bind/consume/drop/GC transitions update it transactionally, replacing repeated scans while a test-only full scan remains the oracle.

## Implementation recipe

0. Before runtime edits, create docs/benchmarks/performance/P072-selection.json and docs/benchmarks/performance/P072-verification.json; freeze the exact target small/large, held-out, protected and cold rows, and bind all semantic cases to successful full verification receipts.

1. List every state/capacity transition and ownership owner in continuation.rs, including host suspension, failed bind/install and GC removal.
2. Compute checked per-state retained bytes before allocation/mutation, reserve aggregate budget, commit only on success and release exactly once on consume/drop/reclaim.
3. Keep shared metadata versus owned activation accounting consistent with P029/P031; compare the ledger to the full recomputation after each test transition.
4. Measure many-live-continuation creation/capture scaling and budget-check cost, preserving one-shot/signature/fuel semantics.
5. Record the changed path with focused assertions/diagnostic counts and exact source/binary identities; compare the pre-edit archive, and revert a rejected structural optimization rather than adding a permanent toggle solely for testing.

## Meaningful cases

- Long randomized transition sequences keep ledger==full scan.
- Failed capture/bind/install changes no retained total and consumes no continuation unexpectedly.
- Consume/reclaim/host outcome cleanup returns bytes exactly once and near-limit admission remains correct.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime p072_ -- --nocapture
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml -j 4
python3 tools/checks/build_budget.py --output target/performance/P072/build.json
python3 -B tools/verify.py plan --contract docs/benchmarks/performance/P072-verification.json
python3 -B tools/verify.py close --contract docs/benchmarks/performance/P072-verification.json --output target/performance/P072/complete --execute
```

Declare the semantic-test and build commands above in the appropriate contract phases;
run `plan` and `close` externally to drive that contract. Bind full transitive
compilation/fixture inputs and require every semantic
case declared in the selection manifest. Its `measure` phase uses
`focused_compare.py --manifest docs/benchmarks/performance/P072-selection.json`
with `--frozen {freeze_dir}/selection.json`, `--semantic-receipt {verify_dir}/receipt.json`,
the immutable predecessor adapter, and the actual production executable/mode.
Its `analyze` phase calls `tools/verify.py statistics` once, supplying that
selection manifest as `--contract`, the paired report, and all build attestations.
Do not run an exhaustive comparison or a separate raw analyzer during this candidate gate.
See the [exact workflow](../../../wtime/docs/verification/fast-performance.md).

## Measurement and decision

Selection: `continuations`. Explicit execution setting: `decoded`. Decision rule: `measured_promotion`.

Use the paired protocol and family/control/cold/build rules in [MEASUREMENT.md](../MEASUREMENT.md). Predeclare the targeted family and held-out controls before changing the implementation. Keep instrumentation off in production timings. Adopt only with supported gains in both runs; otherwise record rejected/inconclusive and remove default-path cost. A necessary immutable-interface change may be contract_only with its separate justification.

## Candidate gate schedule

Before implementation, freeze and verify the immediate-predecessor archive and a task-owned predeclaration manifest. Bind the manifest to exact registry workload IDs and input values, optimization shapes, target small/large rows, held-out rows, protected controls, stages, feature/runtime settings, and applicable cold products. Candidate promotion compares against this immediate predecessor only; P001 is reserved for exhaustive milestone totals. Measure the complete declared candidate set in two independent runs with at least 30 complete pairs per row and run, using 10,000 deterministic bootstrap resamples and uninstrumented production binaries. The applicable large-module preparation control and tiny `call_overhead` first-call/load-and-call cold controls are mandatory. Preserve every task-specific semantic, fuel/cancellation, trap/memory, fallback, and optimizer-bound case regardless of the focused timing selection. See [MEASUREMENT.md](../MEASUREMENT.md) for cold-product identity, sample reuse, thresholds, and milestone rules.

## Before editing

Run these commands before changing the candidate sources. The archive is immutable and captures the immediate predecessor. Retain P001's original archive for milestone totals. For native work, use the named portable control and verify its actual policy/quantum; if this archive already exists with different sources, choose a new recorded run-specific directory and update the comparison path rather than overwriting it.

```sh
python3 tools/wtime-bench/baseline.py --output target/performance/P072-before
python3 -B tools/wtime-bench/selection_manifest.py freeze docs/benchmarks/performance/P072-selection.json target/performance/P072-before/manifest.json --output target/performance/P072/selection-freeze.json
```

Create and validate the task-owned selection manifest before these commands.
The freezer requires the archived core to match the live predecessor, so run
it before the first runtime edit. The shared contract revalidates this receipt
in its `freeze` phase without replacing it after implementation.

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. No commands in this planning file have been executed as implementation evidence.
