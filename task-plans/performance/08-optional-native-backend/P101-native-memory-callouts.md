# P101 — Cross native memory boundaries through safe checked callouts

Status: **todo**

Outcome: **pending**

Kind: `experiment` · Branch: `optional` · Model: **Sol 6.1 Low**

## Prerequisites

[P002 — Compare separately built baseline and candidate binaries](../00-measurement/P002-paired-driver.md)

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P009 — Add memory width, bulk and growth benchmark families](../00-measurement/P009-memory-workloads.md)

[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)

[P016 — Add explicit executor policies and test integration hooks](../01-execution-contracts/P016-execution-policy.md)

[P043 — Resolve memory bindings and specialize checked scalar accesses](../03-compact-executor/P043-compact-memory-plans.md)

[P063 — Add direct checked unshared memory register operations](../05-register-executor/P063-register-unshared-memory.md)

[P099 — Add exact fuel, traps and bounded cancellation to native execution](P099-native-accounting-exits.md)

[P100 — Compile and retain native code only for eligible hot functions](P100-bounded-native-tiering.md)

Conditional branch: `P094.outcome == go`. If its decision is declined, record this task as deferred/not_selected; do not claim it implemented.

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C5, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/execution/native.rs`
- `crates/wtime-native/src/abi.rs`
- `crates/wtime-runtime/tests/performance/p101.rs`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

The first memory extension returns to safe Rust at one original load/store, performs it exactly once through existing checked instance bindings, then reenters at a verified source successor; no guest memory pointer is cached.

## Implementation recipe

1. Add explicit MemoryExit descriptors for supported numeric loads/stores and materialize address/value/source state before the memory operation.
2. Perform effective-address/full-width checks and shared/unshared backend dispatch in safe Rust, retaining original trap/fuel/event order.
3. Resume native only at a verified materialized successor; keep growth/host/ref/GC/capture operations on full portable fallback.
4. Measure callout overhead on mixed memory/compute cases and decide whether this safe extension is useful before attempting inline raw memory access.
5. Forward the explicit `native-memory-callouts` setting through the harness/policy integrator and record actual optimized/fallback counts. Persistent research selectors are nondefault; reversible changes to an existing tier use before/after artifacts and are selected only by their gate.
6. Attach the separately labeled optional-build report to the performance decision, alongside the default build report; a default-only build cannot establish native/experimental build cost.

## Meaningful cases

- A memory op is performed once and source handoff cannot replay a store.
- Shared/memory64/overflow/OOB traps retain backend semantics.
- Host growth/reentry and unsupported effects force safe full materialization.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime --features native-executor,execution-reference p101_ -- --nocapture
cargo test --locked -p wtime-native p101_ -- --nocapture
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml --features native-executor -j 4
python3 tools/wtime-bench/compare.py --baseline target/performance/P101-native-matched-baseline --candidate tools/wtime-bench/target/release/wtime-bench --candidate-mode native-memory-callouts --selection memory --output target/performance/P101
python3 tools/wtime-bench/analyze.py --input target/performance/P101 --output target/performance/P101/analysis.json
python3 tools/checks/build_budget.py --output target/performance/P101/build.json
python3 tools/checks/build_budget.py --features wtime-runtime/native-executor --output target/performance/P101/optional-build.json
python3 tools/wtime-bench/gates.py --task P101 --comparison target/performance/P101 --build target/performance/P101/build.json --optional-build target/performance/P101/optional-build.json --output target/performance/P101/decision.json
```

## Measurement and decision

Selection: `memory`. Explicit execution setting: `native-memory-callouts`. Decision rule: `measured_promotion`.

Use the paired protocol and family/control/cold/build rules in [MEASUREMENT.md](../MEASUREMENT.md). Predeclare the targeted family and held-out controls before changing the implementation. Keep instrumentation off in production timings. Adopt only with supported gains in both runs; otherwise record rejected/inconclusive and remove default-path cost. A necessary immutable-interface change may be contract_only with its separate justification.

## Before editing

Run these commands before changing the candidate sources. The archive is immutable and captures the immediate predecessor. Retain P001's original archive for milestone totals. For native work, use the named portable control and verify its actual policy/quantum; if this archive already exists with different sources, choose a new recorded run-specific directory and update the comparison path rather than overwriting it.

```sh
python3 tools/wtime-bench/baseline.py --output target/performance/P101-native-matched-baseline --mode portable
```

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. No commands in this planning file have been executed as implementation evidence.
