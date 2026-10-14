# P063 — Add direct checked unshared memory register operations

Status: **done**

Outcome: **rejected**

Kind: `candidate` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P002 — Compare separately built baseline and candidate binaries](../00-measurement/P002-paired-driver.md)

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P009 — Add memory width, bulk and growth benchmark families](../00-measurement/P009-memory-workloads.md)

[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)

[P016 — Add explicit executor policies and test integration hooks](../01-execution-contracts/P016-execution-policy.md)

[P043 — Resolve memory bindings and specialize checked scalar accesses](../03-compact-executor/P043-compact-memory-plans.md)

[P061 — Execute checked integer register operations](P061-register-integer-execution.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C5, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/execution/register/memory.rs`
- `crates/wtime-runtime/src/execution/register/lower.rs`
- `crates/wtime-runtime/tests/performance/p063.rs`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

Eligible scalar memory ops use register address/value operands and safe checked current slices; shared memory and unsupported wide forms decline or exit before their source operation.

## Implementation recipe

0. Before runtime edits, create docs/benchmarks/performance/P063-selection.json and docs/benchmarks/performance/P063-verification.json; freeze the exact target small/large, held-out, protected and cold rows, and bind all semantic cases to successful full verification receipts.

1. Lower width/extension/offset/local binding once and compute wrapping guest arithmetic versus checked immediate addition exactly as the scalar executor.
2. Use fixed buffers/safe slices without retained base pointers, reflecting current growth and instance bindings on every access.
3. Attach trap/source/accounting events and materialize interruption before a store; never perform a memory effect then deopt to replay it.
4. Measure width/shuffled/mixed/representative families and full memory digests plus overflow/OOB ordering.
5. Forward the explicit `register` setting through the harness/policy integrator and record actual optimized/fallback counts. Persistent research selectors are nondefault; reversible changes to an existing tier use before/after artifacts and are selected only by their gate.

## Meaningful cases

- All signed/unsigned widths and memory-end addresses match reference.
- Prior side effects remain while the failing access and later effects do not happen.
- Shared/wide unsupported forms visibly decline and cannot use stale unshared views.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime --features register-executor,execution-reference p063_ -- --nocapture
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml --features register-executor -j 4
python3 tools/checks/build_budget.py --output target/performance/P063/build.json
python3 tools/checks/build_budget.py --features wtime-runtime/register-executor --output target/performance/P063/optional-build.json
python3 -B tools/verify.py plan --contract docs/benchmarks/performance/P063-verification.json
python3 -B tools/verify.py close --contract docs/benchmarks/performance/P063-verification.json --output target/performance/P063/complete --execute
```

Declare the semantic-test and build commands above in the appropriate contract phases;
run `plan` and `close` externally to drive that contract. Bind full transitive
compilation/fixture inputs and require every semantic
case declared in the selection manifest. Its `measure` phase uses
`focused_compare.py --manifest docs/benchmarks/performance/P063-selection.json`
with `--frozen {freeze_dir}/selection.json`, `--semantic-receipt {verify_dir}/receipt.json`,
the direct frozen Register-mode predecessor archive, and the actual production executable/mode.
Its `analyze` phase calls `tools/verify.py statistics` once, supplying that
selection manifest as `--contract`, the paired report, and all build attestations.
Do not run an exhaustive comparison or a separate raw analyzer during this candidate gate.
See the [exact workflow](../../../wtime/docs/verification/fast-performance.md).

## Measurement and decision

Selection: `memory`. Explicit execution setting: `register`. Decision rule: `measured_promotion`.

Use the paired protocol and family/control/cold/build rules in [MEASUREMENT.md](../MEASUREMENT.md). Predeclare the targeted family and held-out controls before changing the implementation. Keep instrumentation off in production timings. Adopt only with supported gains in both runs; otherwise record rejected/inconclusive and remove default-path cost. A necessary immutable-interface change may be contract_only with its separate justification.

## Candidate gate schedule

Before implementation, freeze and verify the immediate-predecessor archive and a task-owned predeclaration manifest. Bind the manifest to exact registry workload IDs and input values, optimization shapes, target small/large rows, held-out rows, protected controls, stages, feature/runtime settings, and applicable cold products. Candidate promotion compares against this immediate predecessor only; P001 is reserved for exhaustive milestone totals. Measure the complete declared candidate set in two independent runs with at least 30 complete pairs per row and run, using 10,000 deterministic bootstrap resamples and uninstrumented production binaries. The applicable large-module preparation control and tiny `call_overhead` first-call/load-and-call cold controls are mandatory. Preserve every task-specific semantic, fuel/cancellation, trap/memory, fallback, and optimizer-bound case regardless of the focused timing selection. See [MEASUREMENT.md](../MEASUREMENT.md) for cold-product identity, sample reuse, thresholds, and milestone rules.

## Before editing

Run these commands before changing the candidate sources. The archive is immutable and captures the immediate predecessor. Retain P001's original archive for milestone totals. For native work, use the named portable control and verify its actual policy/quantum; if this archive already exists with different sources, choose a new recorded run-specific directory and update the comparison path rather than overwriting it.

```sh
python3 tools/wtime-bench/baseline.py --mode register --output target/performance/P063-before
python3 -B tools/wtime-bench/selection_manifest.py freeze docs/benchmarks/performance/P063-selection.json target/performance/P063-before/manifest.json --output target/performance/P063/selection-freeze.json
```

Create and validate the task-owned selection manifest before these commands.
The freezer requires the archived core to match the live predecessor, so run
it before the first runtime edit. The shared contract revalidates this receipt
in its `freeze` phase without replacing it after implementation.

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. Executed evidence is recorded below.

## Executed evidence

All four shared-contract phases (freeze, verify, measure, analyze) passed in
attempt `0001`; P063 finalized as close `0076`, finalization attempt `0001`.
The performance decision rejected promotion. Keep the Register memory interface
only as a nondefault prerequisite for P064, P065, and P066, with no Decoded or
Portable promotion.

The direct P063-before baseline archive is the Register-mode source at commit
`e0f2f64`, with no adapter. Verification passed 292 tests: 285 Register/reference
semantic tests, one default-policy test, and six combined-control tests, with
zero failures and zero ignored tests. Semantic checks compared 75 full logical
snapshots (42 loads, 28 stores, and 5 traps). Diagnostic snapshots cap memory
at 64 bytes and mark that data truncated; complete 65,536-byte images were
compared separately. The suite checked 30 independent store images at source
fuel values 0 through 29. All nine target kernels ran in Register with no
fallback. Coverage included binding and growth aliases, backend-promotion and
foreign-store guards, source identity, budgets, traps, and priority.

The candidate measurement used 9 rows in 29 logical groups, with 1,500 unique
physical pairs, 1,740 logical references, 30 pairs per row in each of two runs,
and 10,000 bootstrap resamples. Primary warm-call candidate/baseline median
ratios were 0.82483247 (95% CI 0.814988–0.837242) and 0.82638884
(95% CI 0.814944–0.836837). Promotion was rejected because small and zero-size
preparation and cold measurements regressed, along with some zero-size warm
measurements.

Default and optional Register builds passed their hard budgets with zero
external dependencies. Clean build times were 2.617875 s (default) and
3.601011 s (Register); stripped binaries were 1,000,712 and 1,071,960 bytes.

Durable artifacts and hashes are indexed by
[`P063-records.json`](../../../wtime/docs/benchmarks/performance/P063-records.json).
The frozen selection is in [`P063-freeze.json`](../../../wtime/docs/benchmarks/performance/P063-freeze.json),
paired measurement in [`P063-comparison.json.gz`](../../../wtime/docs/benchmarks/performance/P063-comparison.json.gz),
analysis in [`P063-statistics.json`](../../../wtime/docs/benchmarks/performance/P063-statistics.json),
build attestations in [`P063-default-build.json`](../../../wtime/docs/benchmarks/performance/P063-default-build.json),
[`P063-register-build.json`](../../../wtime/docs/benchmarks/performance/P063-register-build.json),
and [`P063-production-build.json`](../../../wtime/docs/benchmarks/performance/P063-production-build.json),
and final publication in [`P063-completion.json`](../../../wtime/docs/benchmarks/performance/P063-completion.json)
and [`P063-evidence.json`](../../../wtime/docs/benchmarks/performance/P063-evidence.json).
The records index lists the four phase-receipt paths and hashes.
