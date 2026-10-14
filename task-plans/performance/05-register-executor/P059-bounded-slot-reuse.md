# P059 — Try conservative temporary-slot reuse without global SSA

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

[P058 — Translate numeric stack code into direct slot operands](P058-stack-to-register-lowering.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/execution/register/slots.rs`
- `crates/wtime-runtime/tests/performance/p059.rs`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

An optional local allocator reuses numeric temporary slots only for proven nonoverlapping lifetimes; locals/block-entry values stay fixed and cross-edge values are conservatively pinned.

## Implementation recipe

1. Compute per-basic-block first-definition/last-use intervals including move and source-materialization uses; extend live-out/unknown lifetimes to block end.
2. Sort intervals deterministically, expire only strictly nonoverlapping intervals and assign the lowest free typed-compatible slot with a bounded free list.
3. Keep a verified virtual-to-physical map for every source stop and parallel edge move; fall back to the stack-height layout on work/slot budget failure.
4. Measure retained activation slots/cache traffic versus allocation/preparation cost, promoting only if it beats the simple layout.
5. Forward the explicit `register-slot-reuse` setting through the harness/policy integrator and record actual optimized/fallback counts. Persistent research selectors are nondefault; reversible changes to an existing tier use before/after artifacts and are selected only by their gate.

## Meaningful cases

- Adjacent intervals may reuse slots but overlapping/source-visible values cannot.
- Backedge/live-out values remain pinned through loops and interrupted materialization.
- Allocator work failure yields the original valid layout, not an incomplete map.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime --features register-executor,experimental-executors,execution-reference p059_ -- --nocapture
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml --features register-executor,experimental-executors -j 4
python3 tools/wtime-bench/compare.py --baseline target/performance/P059-before --candidate tools/wtime-bench/target/release/wtime-bench --candidate-mode register-slot-reuse --selection scalar --output target/performance/P059
python3 tools/wtime-bench/analyze.py --input target/performance/P059 --output target/performance/P059/analysis.json
python3 tools/checks/build_budget.py --output target/performance/P059/build.json
python3 tools/checks/build_budget.py --features wtime-runtime/register-executor --output target/performance/P059/optional-build.json
python3 tools/wtime-bench/gates.py --task P059 --comparison target/performance/P059 --build target/performance/P059/build.json --optional-build target/performance/P059/optional-build.json --output target/performance/P059/decision.json
```

## Measurement and decision

Selection: `scalar`. Explicit execution setting: `register-slot-reuse`. Decision rule: `measured_promotion`.

Use the paired protocol and family/control/cold/build rules in [MEASUREMENT.md](../MEASUREMENT.md). Predeclare the targeted family and held-out controls before changing the implementation. Keep instrumentation off in production timings. Adopt only with supported gains in both runs; otherwise record rejected/inconclusive and remove default-path cost. A necessary immutable-interface change may be contract_only with its separate justification.

## Before editing

Run these commands before changing the candidate sources. The archive is immutable and captures the immediate predecessor. Retain P001's original archive for milestone totals. For native work, use the named portable control and verify its actual policy/quantum; if this archive already exists with different sources, choose a new recorded run-specific directory and update the comparison path rather than overwriting it.

```sh
python3 tools/wtime-bench/baseline.py --output target/performance/P059-before
```

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. No commands in this planning file have been executed as implementation evidence.
