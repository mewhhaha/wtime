# P069 — Use conservative typed root maps for prepared frames

Status: **done**

Outcome: **inconclusive**

Kind: `candidate` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P002 — Compare separately built baseline and candidate binaries](../00-measurement/P002-paired-driver.md)

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P011 — Measure reference, GC and exception paths](../00-measurement/P011-gc-exception-workloads.md)

[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)

[P016 — Add explicit executor policies and test integration hooks](../01-execution-contracts/P016-execution-policy.md)

[P021 — Compute bounded per-function execution metadata](../01-execution-contracts/P021-function-execution-plans.md)

[P044 — Materialize compact state for GC, exceptions and continuations](../03-compact-executor/P044-compact-proposal-boundaries.md)

[P065 — Verify roots and captured state around register exits](../05-register-executor/P065-register-root-and-capture-barriers.md)

[P068 — Copy only references into host and collection root snapshots](P068-reference-only-root-snapshots.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C4, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/execution/roots.rs`
- `crates/wtime-runtime/src/gc_exec.rs`
- `crates/wtime-runtime/src/execution.rs`
- `crates/wtime-runtime/src/execution/plan.rs`
- `crates/wtime-runtime/src/lib.rs`
- `crates/wtime-validate/src/lib.rs`
- `crates/wtime-validate/src/root_maps.rs`
- `crates/wtime-runtime/tests/performance/p069.rs`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

Prepared root maps enumerate all reference-typed local/operand slots retained by the current contract, including temporaries at every barrier; they do not drop dead-but-retained locals or alter collection statistics by changing liveness policy.

## Implementation recipe

0. Before runtime edits, create docs/benchmarks/performance/P069-selection.json and docs/benchmarks/performance/P069-verification.json; freeze the exact target small/large, held-out, protected and cold rows, and bind all semantic cases to successful full verification receipts.

1. Derive bounded slot class maps during the existing validation pass and bind them to the FunctionPlan body and source PC. Record retained reference locals and the operand layout at call boundaries without running a second validator. Numeric-only regions need no local reference scan; unknown, unreachable, over-budget, or unmatched layouts fall back to full Value scanning. Charge owned map capacities to module metadata and decline maps transactionally when their work or byte budget is exhausted.
2. Associate maps with exact materialization/source points and include cached/adapter values, exception payloads and outer frames.
3. Validate maps against a development full-scan oracle on every root boundary and preserve one owned reentry snapshot rather than borrowing active vectors.
4. Measure mixed numeric/reference host/GC workloads, preserving the same roots/live counts and limiting metadata growth.
5. Record the changed path with focused assertions/diagnostic counts and exact source/binary identities; compare the pre-edit archive, and revert a rejected structural optimization rather than adding a permanent toggle solely for testing.

## Meaningful cases

- Every source boundary with a transient reference matches the full-scan root set.
- An unused but retained reference local remains a root under current API semantics.
- Unknown/declined layouts use the complete scan and captured roots stay correct.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime p069_ -- --nocapture
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml -j 4
python3 tools/checks/build_budget.py --output target/performance/P069/build.json
python3 -B tools/verify.py plan --contract docs/benchmarks/performance/P069-verification.json
python3 -B tools/verify.py close --contract docs/benchmarks/performance/P069-verification.json --output target/performance/P069/complete --execute
```

Declare the semantic-test and build commands above in the appropriate contract phases;
run `plan` and `close` externally to drive that contract. Bind full transitive
compilation/fixture inputs and require every semantic
case declared in the selection manifest. Its `measure` phase uses
`focused_compare.py --manifest docs/benchmarks/performance/P069-selection.json`
with `--frozen {freeze_dir}/selection.json`, `--semantic-receipt {verify_dir}/receipt.json`,
the immutable predecessor adapter, and the actual production executable/mode.
Its `analyze` phase calls `tools/verify.py statistics` once, supplying that
selection manifest as `--contract`, the paired report, and all build attestations.
Do not run an exhaustive comparison or a separate raw analyzer during this candidate gate.
See the [exact workflow](../../../wtime/docs/verification/fast-performance.md).

## Measurement and decision

Selection: `gc-exceptions`. Explicit execution setting: `decoded`. Decision rule: `measured_promotion`.

Use the paired protocol and family/control/cold/build rules in [MEASUREMENT.md](../MEASUREMENT.md). Predeclare the targeted family and held-out controls before changing the implementation. Keep instrumentation off in production timings. Adopt only with supported gains in both runs; otherwise record rejected/inconclusive and remove default-path cost. A necessary immutable-interface change may be contract_only with its separate justification.

## Candidate gate schedule

Before implementation, freeze and verify the immediate-predecessor archive and a task-owned predeclaration manifest. Bind the manifest to exact registry workload IDs and input values, optimization shapes, target small/large rows, held-out rows, protected controls, stages, feature/runtime settings, and applicable cold products. Candidate promotion compares against this immediate predecessor only; P001 is reserved for exhaustive milestone totals. Measure the complete declared candidate set in two independent runs with at least 30 complete pairs per row and run, using 10,000 deterministic bootstrap resamples and uninstrumented production binaries. The applicable large-module preparation control and tiny `call_overhead` first-call/load-and-call cold controls are mandatory. Preserve every task-specific semantic, fuel/cancellation, trap/memory, fallback, and optimizer-bound case regardless of the focused timing selection. See [MEASUREMENT.md](../MEASUREMENT.md) for cold-product identity, sample reuse, thresholds, and milestone rules.

## Before editing

Run these commands before changing the candidate sources. The archive is immutable and captures the immediate predecessor. Retain P001's original archive for milestone totals. For native work, use the named portable control and verify its actual policy/quantum; if this archive already exists with different sources, choose a new recorded run-specific directory and update the comparison path rather than overwriting it.

```sh
python3 tools/wtime-bench/baseline.py --output target/performance/P069-before
python3 -B tools/wtime-bench/selection_manifest.py freeze docs/benchmarks/performance/P069-selection.json target/performance/P069-before/manifest.json target/performance/P069/selection-freeze.json
```

Create and validate the task-owned selection manifest before these commands.
The freezer requires the archived core to match the live predecessor, so run
it before the first runtime edit. The shared contract revalidates this receipt
in its `freeze` phase without replacing it after implementation.

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. Executed evidence is recorded below.

## Executed evidence

P069 completed with an **inconclusive** performance decision, so the candidate optimization was fully restored. The candidate close passed 425 tests (383 Rust and 42 Python); the restored-tree close passed 357 tests (101 default, 173 combined, and 83 validator), with no failures or ignored tests. The measurement used 1,020 paired observations across 17 groups, two 30-pair runs (seeds 69001 and 69002), CPU 1, decoded mode, and no instrumentation. The two primary estimates were 0.4093314493 [0.3941898078, 0.4184230671] and 0.3942602585 [0.389870884, 0.4031465694]; nine protected bounds remained uncertain, preventing promotion. The implementation used bounded typed root maps with a 1 MiB retained metadata cap and used complete scanning when layouts were unsupported; a mixed snapshot copied 2 reference slots instead of 578 total slots. Three modified Rust files were restored and three added files removed; all 176 archived inputs matched the fresh restored build.

Durable completion, evidence, proof, statistics, freeze, records, restoration, and build-comparison artifacts are linked from TASKS.json. The candidate result is historical evidence only; the current runtime contains the restored predecessor.
