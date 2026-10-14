# P052 — Try safe unboxed storage for proven numeric activations

Status: **todo**

Outcome: **pending**

Kind: `experiment` · Branch: `optional` · Model: **Sol 6.1 Low**

## Prerequisites

[P002 — Compare separately built baseline and candidate binaries](../00-measurement/P002-paired-driver.md)

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P008 — Add scaling, branch and call benchmark families](../00-measurement/P008-scalar-workloads.md)

[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)

[P016 — Add explicit executor policies and test integration hooks](../01-execution-contracts/P016-execution-policy.md)

[P021 — Compute bounded per-function execution metadata](../01-execution-contracts/P021-function-execution-plans.md)

[P038 — Lower constants, locals and common integer operations](../03-compact-executor/P038-compact-scalar-lowering.md)

[P051 — Try a two-value numeric operand cache](P051-operand-top-cache.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/execution/compact/numeric_slots.rs`
- `crates/wtime-runtime/src/execution/compact/execute.rs`
- `crates/wtime-runtime/tests/performance/p052.rs`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

Eligible numeric-only functions store i32/i64/f32/f64 bits in typed-plan-indexed u64 slots; public Value and all reference/V128 functions stay on the general representation.

## Implementation recipe

1. Derive a whole-function eligibility certificate excluding refs, host/dynamic calls and unsupported proposal barriers; retain safe function-entry fallback.
2. Use explicit bit conversions and initialized Vec<u64> storage, with checked slot bounds/type metadata and no unsafe casts or NaN-boxed references.
3. Implement lossless materialization to public Values/source stack for return/trap/accounting; preserve signed zero/NaN bits and slot budget accounting.
4. Measure Value-size/traffic savings and prep/build cost; don't enable unboxed storage based on a single arithmetic benchmark.
5. Forward the explicit `compact-numeric-slots` setting through the harness/policy integrator and record actual optimized/fallback counts. Persistent research selectors are nondefault; reversible changes to an existing tier use before/after artifacts and are selected only by their gate.

## Meaningful cases

- Mixed i32/i64/float bit values survive materialization exactly.
- Ref/V128/effectful functions decline and never lose roots.
- Slot/work allocation failure falls back without partial execution or admission bypass.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime --features compact-executor,experimental-executors,execution-reference p052_ -- --nocapture
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml --features compact-executor,experimental-executors -j 4
python3 tools/wtime-bench/compare.py --baseline target/performance/P052-before --candidate tools/wtime-bench/target/release/wtime-bench --candidate-mode compact-numeric-slots --selection scalar --output target/performance/P052
python3 tools/wtime-bench/analyze.py --input target/performance/P052 --output target/performance/P052/analysis.json
python3 tools/checks/build_budget.py --output target/performance/P052/build.json
python3 tools/checks/build_budget.py --features wtime-runtime/compact-executor --output target/performance/P052/optional-build.json
python3 tools/wtime-bench/gates.py --task P052 --comparison target/performance/P052 --build target/performance/P052/build.json --optional-build target/performance/P052/optional-build.json --output target/performance/P052/decision.json
```

## Measurement and decision

Selection: `scalar`. Explicit execution setting: `compact-numeric-slots`. Decision rule: `measured_promotion`.

Use the paired protocol and family/control/cold/build rules in [MEASUREMENT.md](../MEASUREMENT.md). Predeclare the targeted family and held-out controls before changing the implementation. Keep instrumentation off in production timings. Adopt only with supported gains in both runs; otherwise record rejected/inconclusive and remove default-path cost. A necessary immutable-interface change may be contract_only with its separate justification.

## Before editing

Run these commands before changing the candidate sources. The archive is immutable and captures the immediate predecessor. Retain P001's original archive for milestone totals. For native work, use the named portable control and verify its actual policy/quantum; if this archive already exists with different sources, choose a new recorded run-specific directory and update the comparison path rather than overwriting it.

```sh
python3 tools/wtime-bench/baseline.py --output target/performance/P052-before
```

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. No commands in this planning file have been executed as implementation evidence.
