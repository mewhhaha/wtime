# P099 — Add exact fuel, traps and bounded cancellation to native execution

Status: **todo**

Outcome: **pending**

Kind: `candidate` · Branch: `optional` · Model: **Sol 6.1 Low**

## Prerequisites

[P002 — Compare separately built baseline and candidate binaries](../00-measurement/P002-paired-driver.md)

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)

[P016 — Add explicit executor policies and test integration hooks](../01-execution-contracts/P016-execution-policy.md)

[P018 — Define source cursors and materialized guest state](../01-execution-contracts/P018-source-cursor.md)

[P020 — Centralize exact source instruction accounting](../01-execution-contracts/P020-exact-execution-accounting.md)

[P022 — Add adversarial state and interaction tier fixtures](../01-execution-contracts/P022-state-differential-fixtures.md)

[P095 — Create the minimal optional numeric backend interface](P095-native-ir-and-abi.md)

[P097 — Emit a tiny verified x86_64 integer block subset](P097-x86-integer-block-emitter.md)

[P098 — Add checked branches and bounded native numeric loops](P098-x86-control-emitter.md)

Optional results to review (do not block the portable path):

[P088 — Evaluate an explicit bounded cancellation polling policy](../07-unconventional-portable-experiments/P088-explicit-bounded-polling.md)

Conditional branch: `P094.outcome == go`. If its decision is declined, record this task as deferred/not_selected; do not claim it implemented.

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-native/src/accounting.rs`
- `crates/wtime-native/src/abi.rs`
- `crates/wtime-runtime/src/execution/native.rs`
- `crates/wtime-native/tests/accounting.rs`
- `crates/wtime-runtime/tests/performance/p099.rs`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

Native code returns explicit source exits and exact fuel/logical-limit state; polling defaults to original quantum 1, while any larger explicit policy uses the matched controls from P088. No panic/exception crosses the native ABI.

## Implementation recipe

0. Before runtime edits, create docs/benchmarks/performance/P099-selection.json and docs/benchmarks/performance/P099-verification.json; freeze the exact target small/large, held-out, protected and cold rows, and bind all semantic cases to successful full verification receipts.

1. Define ABI-safe helper/exit paths with private valid pointer lifetimes and no unwinding; helpers read cancellation through the audited boundary and return status rather than Rust errors/panics through generated frames.
2. Implement original source-event/fuel/stack/depth accounting, including interruption inside fused/structured code and saved continuation fuel at the portable boundary.
3. Guard division/overflow/conversion traps before hardware faults, attribute original source PCs and return before any unsupported effect is executed.
4. Before edits, run the Before editing command to freeze P099-native-matched-baseline from the current selected portable policy. Default quantum 1 needs no P088; a larger selected quantum requires its adopted contract and a separately named same-quantum control. Compare the verified archive path exactly.
5. Measure exact-policy and optional bounded-quantum modes separately, preserving historical policy differences and real cancellation-latency evidence.
6. Forward the explicit `native` setting through the harness/policy integrator and record actual optimized/fallback counts. Persistent research selectors are nondefault; reversible changes to an existing tier use before/after artifacts and are selected only by their gate.
7. Attach the separately labeled optional-build report to the performance decision, alongside the default build report; a default-only build cannot establish native/experimental build cost.

## Meaningful cases

- Every fuel prefix yields identical slots/source PC/prior effects.
- Zero/MIN division cannot raise an unmanaged machine fault.
- Cancellation/resource exits and native-to-portable handoff cannot replay effects or unwind across the ABI.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime --features native-executor,execution-reference p099_ -- --nocapture
cargo test --locked -p wtime-native p099_ -- --nocapture
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml --features native-executor -j 4
python3 tools/checks/build_budget.py --output target/performance/P099/build.json
python3 tools/checks/build_budget.py --features wtime-runtime/native-executor --output target/performance/P099/optional-build.json
python3 -B tools/verify.py plan --contract docs/benchmarks/performance/P099-verification.json
python3 -B tools/verify.py close --contract docs/benchmarks/performance/P099-verification.json --output target/performance/P099/complete --execute
```

Declare the semantic-test and build commands above in the appropriate contract phases;
run `plan` and `close` externally to drive that contract. Bind full transitive
compilation/fixture inputs and require every semantic
case declared in the selection manifest. Its `measure` phase uses
`focused_compare.py --manifest docs/benchmarks/performance/P099-selection.json`
with `--frozen {freeze_dir}/selection.json`, `--semantic-receipt {verify_dir}/receipt.json`,
the immutable predecessor adapter, and the actual production executable/mode.
Its `analyze` phase calls `tools/verify.py statistics` once, supplying that
selection manifest as `--contract`, the paired report, and all build attestations.
Do not run an exhaustive comparison or a separate raw analyzer during this candidate gate.
See the [exact workflow](../../../wtime/docs/verification/fast-performance.md).

## Measurement and decision

Selection: `integration`. Explicit execution setting: `native`. Decision rule: `measured_promotion`.

Use the paired protocol and family/control/cold/build rules in [MEASUREMENT.md](../MEASUREMENT.md). Predeclare the targeted family and held-out controls before changing the implementation. Keep instrumentation off in production timings. Adopt only with supported gains in both runs; otherwise record rejected/inconclusive and remove default-path cost. A necessary immutable-interface change may be contract_only with its separate justification.

## Candidate gate schedule

Before implementation, freeze and verify the immediate-predecessor archive and a task-owned predeclaration manifest. Bind the manifest to exact registry workload IDs and input values, optimization shapes, target small/large rows, held-out rows, protected controls, stages, feature/runtime settings, and applicable cold products. Candidate promotion compares against this immediate predecessor only; P001 is reserved for exhaustive milestone totals. Measure the complete declared candidate set in two independent runs with at least 30 complete pairs per row and run, using 10,000 deterministic bootstrap resamples and uninstrumented production binaries. The applicable large-module preparation control and tiny `call_overhead` first-call/load-and-call cold controls are mandatory. Preserve every task-specific semantic, fuel/cancellation, trap/memory, fallback, and optimizer-bound case regardless of the focused timing selection. See [MEASUREMENT.md](../MEASUREMENT.md) for cold-product identity, sample reuse, thresholds, and milestone rules.

## Before editing

Run these commands before changing the candidate sources. The archive is immutable and captures the immediate predecessor. Retain P001's original archive for milestone totals. For native work, use the named portable control and verify its actual policy/quantum; if this archive already exists with different sources, choose a new recorded run-specific directory and update the comparison path rather than overwriting it.

```sh
python3 tools/wtime-bench/baseline.py --output target/performance/P099-native-matched-baseline --mode portable
python3 -B tools/wtime-bench/selection_manifest.py freeze docs/benchmarks/performance/P099-selection.json target/performance/P099-native-matched-baseline/manifest.json target/performance/P099/selection-freeze.json
```

Create and validate the task-owned selection manifest before these commands.
The freezer requires the archived core to match the live predecessor, so run
it before the first runtime edit. The shared contract revalidates this receipt
in its `freeze` phase without replacing it after implementation.

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. No commands in this planning file have been executed as implementation evidence.
