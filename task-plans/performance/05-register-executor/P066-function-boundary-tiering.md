# P066 — Compile register code lazily at bounded function thresholds

Status: **todo**

Outcome: **pending**

Kind: `candidate` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P002 — Compare separately built baseline and candidate binaries](../00-measurement/P002-paired-driver.md)

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)

[P015 — Add reproducible application-shaped held-out kernels](../00-measurement/P015-representative-kernels.md)

[P016 — Add explicit executor policies and test integration hooks](../01-execution-contracts/P016-execution-policy.md)

[P057 — Define a small numeric register bytecode and eligibility rules](P057-register-ir-contract.md)

[P061 — Execute checked integer register operations](P061-register-integer-execution.md)

[P062 — Add strict floating-point register operations](P062-register-float-execution.md)

[P063 — Add direct checked unshared memory register operations](P063-register-unshared-memory.md)

[P064 — Exit register execution before calls and resume decoded state](P064-register-call-exits.md)

[P065 — Verify roots and captured state around register exits](P065-register-root-and-capture-barriers.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/execution/tiering.rs`
- `crates/wtime-runtime/src/lib.rs`
- `crates/wtime-runtime/tests/performance/p066.rs`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

An opt-in adaptive policy counts bounded function entries and promotes at an invocation/call boundary using measured thresholds. No mid-loop OSR is attempted; failed/unsupported compilation keeps portable execution.

## Implementation recipe

0. Before runtime edits, create docs/benchmarks/performance/P066-selection.json and docs/benchmarks/performance/P066-verification.json; freeze the exact target small/large, held-out, protected and cold rows, and bind all semantic cases to successful full verification receipts.

1. Keep saturating hotness counters bounded per retained function, choose thresholds from measured preparation/savings and record tier transitions diagnostically.
2. Compile/publish immutable register artifacts transactionally outside canonical/cache locks; synchronize duplicate attempts and account optimizer work/code bytes.
3. Use a deterministic negative-result/cooldown policy for ineligible or transient-failed compilation, with bounded cache eviction and no retry on every call.
4. Measure 1/10/100/10000-call totals, cold-code distributions and eviction/recompile spikes, retaining explicit policy settings in reports.
5. Forward the explicit `register` setting through the harness/policy integrator and record actual optimized/fallback counts. Persistent research selectors are nondefault; reversible changes to an existing tier use before/after artifacts and are selected only by their gate.

## Meaningful cases

- Cold functions never compile and hot eligible functions transition at the declared boundary.
- Concurrent/shared-module promotion and failure cannot publish partial/foreign-context code.
- Eviction/churn obeys code budget and doesn't reset guest fuel or duplicate effects.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime --features register-executor,execution-reference p066_ -- --nocapture
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml --features register-executor -j 4
python3 tools/checks/build_budget.py --output target/performance/P066/build.json
python3 tools/checks/build_budget.py --features wtime-runtime/register-executor --output target/performance/P066/optional-build.json
python3 -B tools/verify.py plan --contract docs/benchmarks/performance/P066-verification.json
python3 -B tools/verify.py close --contract docs/benchmarks/performance/P066-verification.json --output target/performance/P066/complete --execute
```

Declare the semantic-test and build commands above in the appropriate contract phases;
run `plan` and `close` externally to drive that contract. Bind full transitive
compilation/fixture inputs and require every semantic
case declared in the selection manifest. Its `measure` phase uses
`focused_compare.py --manifest docs/benchmarks/performance/P066-selection.json`
with `--frozen {freeze_dir}/selection.json`, `--semantic-receipt {verify_dir}/receipt.json`,
the immutable predecessor adapter, and the actual production executable/mode.
Its `analyze` phase calls `tools/verify.py statistics` once, supplying that
selection manifest as `--contract`, the paired report, and all build attestations.
Do not run an exhaustive comparison or a separate raw analyzer during this candidate gate.
See the [exact workflow](../../../wtime/docs/verification/fast-performance.md).

## Measurement and decision

Selection: `representative`. Explicit execution setting: `register`. Decision rule: `measured_promotion`.

Use the paired protocol and family/control/cold/build rules in [MEASUREMENT.md](../MEASUREMENT.md). Predeclare the targeted family and held-out controls before changing the implementation. Keep instrumentation off in production timings. Adopt only with supported gains in both runs; otherwise record rejected/inconclusive and remove default-path cost. A necessary immutable-interface change may be contract_only with its separate justification.

## Candidate gate schedule

Before implementation, freeze and verify the immediate-predecessor archive and a task-owned predeclaration manifest. Bind the manifest to exact registry workload IDs and input values, optimization shapes, target small/large rows, held-out rows, protected controls, stages, feature/runtime settings, and applicable cold products. Candidate promotion compares against this immediate predecessor only; P001 is reserved for exhaustive milestone totals. Measure the complete declared candidate set in two independent runs with at least 30 complete pairs per row and run, using 10,000 deterministic bootstrap resamples and uninstrumented production binaries. The applicable large-module preparation control and tiny `call_overhead` first-call/load-and-call cold controls are mandatory. Preserve every task-specific semantic, fuel/cancellation, trap/memory, fallback, and optimizer-bound case regardless of the focused timing selection. See [MEASUREMENT.md](../MEASUREMENT.md) for cold-product identity, sample reuse, thresholds, and milestone rules.

## Before editing

Run these commands before changing the candidate sources. The archive is immutable and captures the immediate predecessor. Retain P001's original archive for milestone totals. For native work, use the named portable control and verify its actual policy/quantum; if this archive already exists with different sources, choose a new recorded run-specific directory and update the comparison path rather than overwriting it.

```sh
python3 tools/wtime-bench/baseline.py --output target/performance/P066-before
python3 -B tools/wtime-bench/selection_manifest.py freeze docs/benchmarks/performance/P066-selection.json target/performance/P066-before/manifest.json --output target/performance/P066/selection-freeze.json
```

Create and validate the task-owned selection manifest before these commands.
The freezer requires the archived core to match the live predecessor, so run
it before the first runtime edit. The shared contract revalidates this receipt
in its `freeze` phase without replacing it after implementation.

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. No commands in this planning file have been executed as implementation evidence.
