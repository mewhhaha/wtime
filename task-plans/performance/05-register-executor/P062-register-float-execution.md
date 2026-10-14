# P062 — Add strict floating-point register operations

Status: **todo**

Outcome: **pending**

Kind: `candidate` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P002 — Compare separately built baseline and candidate binaries](../00-measurement/P002-paired-driver.md)

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P010 — Add SIMD and strict floating-point benchmarks](../00-measurement/P010-simd-float-workloads.md)

[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)

[P016 — Add explicit executor policies and test integration hooks](../01-execution-contracts/P016-execution-policy.md)

[P039 — Lower float/vector constants and barrier adapters](../03-compact-executor/P039-compact-float-vector-adapters.md)

[P061 — Execute checked integer register operations](P061-register-integer-execution.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/execution/register/float.rs`
- `crates/wtime-runtime/src/execution/register/lower.rs`
- `crates/wtime-runtime/tests/performance/p062.rs`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

Numeric register slots retain raw f32/f64 bits; arithmetic/conversion helpers obey existing NaN/signed-zero/rounding/trap rules. No host fast-math or approximate comparisons are enabled.

## Implementation recipe

0. Before runtime edits, create docs/benchmarks/performance/P062-selection.json and docs/benchmarks/performance/P062-verification.json; freeze the exact target small/large, held-out, protected and cold rows, and bind all semantic cases to successful full verification receipts.

1. Lower float constants/moves and a small common arithmetic family with safe to_bits/from_bits conversions.
2. Reuse precise min/max/copysign/rounding/conversion helpers where semantic differences matter; retain saturating versus trapping conversion tags.
3. Expand eligibility only for fully implemented operations and source materialization; mixed int/float locals retain fixed typed plans.
4. Run bit/allowed-NaN boundary controls and float kernels, comparing preparation/code size before broad opcode specialization.
5. Forward the explicit `register` setting through the harness/policy integrator and record actual optimized/fallback counts. Persistent research selectors are nondefault; reversible changes to an existing tier use before/after artifacts and are selected only by their gate.

## Meaningful cases

- Signed-zero min/max and raw NaN constants preserve required bit behavior.
- Trapping and saturating conversions distinguish boundary cases.
- Mixed numeric materialization and fuel stops preserve exact slot types/bits.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime --features register-executor,execution-reference p062_ -- --nocapture
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml --features register-executor -j 4
python3 tools/checks/build_budget.py --output target/performance/P062/build.json
python3 tools/checks/build_budget.py --features wtime-runtime/register-executor --output target/performance/P062/optional-build.json
python3 -B tools/verify.py plan --contract docs/benchmarks/performance/P062-verification.json
python3 -B tools/verify.py close --contract docs/benchmarks/performance/P062-verification.json --output target/performance/P062/complete --execute
```

Declare the semantic-test and build commands above in the appropriate contract phases;
run `plan` and `close` externally to drive that contract. Bind full transitive
compilation/fixture inputs and require every semantic
case declared in the selection manifest. Its `measure` phase uses
`focused_compare.py --manifest docs/benchmarks/performance/P062-selection.json`
with `--frozen {freeze_dir}/selection.json`, `--semantic-receipt {verify_dir}/receipt.json`,
the immutable predecessor adapter, and the actual production executable/mode.
Its `analyze` phase calls `tools/verify.py statistics` once, supplying that
selection manifest as `--contract`, the paired report, and all build attestations.
Do not run an exhaustive comparison or a separate raw analyzer during this candidate gate.
See the [exact workflow](../../../wtime/docs/verification/fast-performance.md).

## Measurement and decision

Selection: `simd`. Explicit execution setting: `register`. Decision rule: `measured_promotion`.

Use the paired protocol and family/control/cold/build rules in [MEASUREMENT.md](../MEASUREMENT.md). Predeclare the targeted family and held-out controls before changing the implementation. Keep instrumentation off in production timings. Adopt only with supported gains in both runs; otherwise record rejected/inconclusive and remove default-path cost. A necessary immutable-interface change may be contract_only with its separate justification.

## Candidate gate schedule

Before implementation, freeze and verify the immediate-predecessor archive and a task-owned predeclaration manifest. Bind the manifest to exact registry workload IDs and input values, optimization shapes, target small/large rows, held-out rows, protected controls, stages, feature/runtime settings, and applicable cold products. Candidate promotion compares against this immediate predecessor only; P001 is reserved for exhaustive milestone totals. Measure the complete declared candidate set in two independent runs with at least 30 complete pairs per row and run, using 10,000 deterministic bootstrap resamples and uninstrumented production binaries. The applicable large-module preparation control and tiny `call_overhead` first-call/load-and-call cold controls are mandatory. Preserve every task-specific semantic, fuel/cancellation, trap/memory, fallback, and optimizer-bound case regardless of the focused timing selection. See [MEASUREMENT.md](../MEASUREMENT.md) for cold-product identity, sample reuse, thresholds, and milestone rules.

## Before editing

Run these commands before changing the candidate sources. The archive is immutable and captures the immediate predecessor. Retain P001's original archive for milestone totals. For native work, use the named portable control and verify its actual policy/quantum; if this archive already exists with different sources, choose a new recorded run-specific directory and update the comparison path rather than overwriting it.

```sh
python3 tools/wtime-bench/baseline.py --output target/performance/P062-before
python3 -B tools/wtime-bench/selection_manifest.py freeze docs/benchmarks/performance/P062-selection.json target/performance/P062-before/manifest.json --output target/performance/P062/selection-freeze.json
```

Create and validate the task-owned selection manifest before these commands.
The freezer requires the archived core to match the live predecessor, so run
it before the first runtime edit. The shared contract revalidates this receipt
in its `freeze` phase without replacing it after implementation.

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. No commands in this planning file have been executed as implementation evidence.
