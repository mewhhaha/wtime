# P088 — Evaluate an explicit bounded cancellation polling policy

Status: **todo**

Outcome: **pending**

Kind: `experiment` · Branch: `optional` · Model: **Sol 6.1 Low**

## Prerequisites

[P002 — Compare separately built baseline and candidate binaries](../00-measurement/P002-paired-driver.md)

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)

[P016 — Add explicit executor policies and test integration hooks](../01-execution-contracts/P016-execution-policy.md)

[P020 — Centralize exact source instruction accounting](../01-execution-contracts/P020-exact-execution-accounting.md)

[P022 — Add adversarial state and interaction tier fixtures](../01-execution-contracts/P022-state-differential-fixtures.md)

[P053 — Separate metered and unmetered dispatch without weaker defaults](../04-specialization-and-fusion/P053-accounting-mode-specialization.md)

[P067 — Evaluate register execution and adaptive break-even](../05-register-executor/P067-register-milestone.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C2, C3, C4, C5, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/execution/accounting.rs`
- `crates/wtime-runtime/src/lib.rs`
- `tools/wtime-bench/baseline.py`
- `tools/wtime-bench/compare.py`
- `tools/wtime-bench/src/main.rs`
- `crates/wtime-runtime/tests/performance/p088.rs`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

ExecutionPolicy may explicitly request cancellation_quantum 1..=1024, default 1; it remains exact-fuel and bounded, polls before host/publication and retains blocking/bulk cancellation. Reports expose policy and matched-policy baseline archives.

## Implementation recipe

1. Add validated policy configuration without changing Engine::new defaults; define exact maximum original instructions between polls, including branches, eliminated ops and inline logical calls.
2. Keep host/return/publication/loop safety checks and existing bulk/wait policies; account remaining quantum through capture/resume and reentry rather than resetting an unbounded window.
3. Extend archive/driver settings to freeze separately named same-source/same-quantum portable controls; reject silent policy mismatches and label historical quantum=1 comparisons as different policies.
4. Measure 1/16/64/256/1024 quantum speed versus worst-case instruction/real-time cancellation latency, adopting only as opt-in if bounds and benefits hold.
5. Forward the explicit `bounded-polling` setting through the harness/policy integrator and record actual optimized/fallback counts. Persistent research selectors are nondefault; reversible changes to an existing tier use before/after artifacts and are selected only by their gate.

## Meaningful cases

- Zero/unbounded/out-of-range quantum is rejected and default remains 1.
- Adversarial loops, fused groups, suspend/resume and reentry respect the instruction bound.
- Cancellation prevents host entry/result publication and interruptible waits/bulk remain responsive.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime --features experimental-executors,execution-reference p088_ -- --nocapture
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml --features experimental-executors -j 4
python3 tools/wtime-bench/compare.py --baseline target/performance/P088-before --candidate tools/wtime-bench/target/release/wtime-bench --candidate-mode bounded-polling --selection integration --output target/performance/P088
python3 tools/wtime-bench/analyze.py --input target/performance/P088 --output target/performance/P088/analysis.json
python3 tools/checks/build_budget.py --output target/performance/P088/build.json
python3 tools/checks/build_budget.py --features wtime-runtime/experimental-executors --output target/performance/P088/optional-build.json
python3 tools/wtime-bench/gates.py --task P088 --comparison target/performance/P088 --build target/performance/P088/build.json --optional-build target/performance/P088/optional-build.json --output target/performance/P088/decision.json
```

## Measurement and decision

Selection: `integration`. Explicit execution setting: `bounded-polling`. Decision rule: `measured_promotion`.

Use the paired protocol and family/control/cold/build rules in [MEASUREMENT.md](../MEASUREMENT.md). Predeclare the targeted family and held-out controls before changing the implementation. Keep instrumentation off in production timings. Adopt only with supported gains in both runs; otherwise record rejected/inconclusive and remove default-path cost. A necessary immutable-interface change may be contract_only with its separate justification.

## Before editing

Run these commands before changing the candidate sources. The archive is immutable and captures the immediate predecessor. Retain P001's original archive for milestone totals. For native work, use the named portable control and verify its actual policy/quantum; if this archive already exists with different sources, choose a new recorded run-specific directory and update the comparison path rather than overwriting it.

```sh
python3 tools/wtime-bench/baseline.py --output target/performance/P088-before
```

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. No commands in this planning file have been executed as implementation evidence.
