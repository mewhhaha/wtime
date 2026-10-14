# P086 — Try exact closed-form acceleration of one pure integer loop

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

[P019 — Define bounded optimizer effect and source expansion metadata](../01-execution-contracts/P019-effect-certificates.md)

[P020 — Centralize exact source instruction accounting](../01-execution-contracts/P020-exact-execution-accounting.md)

[P058 — Translate numeric stack code into direct slot operands](../05-register-executor/P058-stack-to-register-lowering.md)

[P061 — Execute checked integer register operations](../05-register-executor/P061-register-integer-execution.md)

[P088 — Evaluate an explicit bounded cancellation polling policy](P088-explicit-bounded-polling.md)

Conditional branch: `P088.outcome == adopted`. If its decision is declined, record this task as deferred/not_selected; do not claim it implemented.

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/execution/experimental/loop_summary.rs`
- `crates/wtime-runtime/tests/performance/p086.rs`
- `tools/wtime-bench/fixtures/scalar/affine_sum.wat`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

The first loop summary recognizes one validated nontrapping finite affine-sum loop grammar, proves induction/trip-count conditions, and executes bounded chunks with exact source accounting. Default quantum=1 may decline it.

## Implementation recipe

1. Specify one structural grammar: numeric induction += positive constant, proven finite unsigned bound, accumulator += a*induction+b under wrapping integer arithmetic, and no memory/call/ref effects.
2. Derive the modulo-width arithmetic-series result and source-event count with checked wide arithmetic; failed termination/overflow proofs fall back without raising a new trap.
3. Use P088's explicit bounded quantum only when selected, apply exact original fuel/transient limits and reconstruct the source prefix for partial chunks; never silently weaken default cancellation.
4. Compare exhaustive small domains and randomized large guarded inputs to step execution; report policy differences and held-out coefficients, not a fixture-keyed shortcut.
5. Forward the explicit `loop-summary` setting through the harness/policy integrator and record actual optimized/fallback counts. Persistent research selectors are nondefault; reversible changes to an existing tier use before/after artifacts and are selected only by their gate.

## Meaningful cases

- All small-domain trip counts/coefficient combinations match wrapping step execution.
- Nonterminating/wrapping/impure patterns decline and zero-trip loops retain structural events.
- Partial fuel/cancel chunks preserve original state and quantum=1 remains the default.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime --features experimental-executors,execution-reference p086_ -- --nocapture
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml --features experimental-executors -j 4
python3 tools/wtime-bench/compare.py --baseline target/performance/P086-before --candidate tools/wtime-bench/target/release/wtime-bench --candidate-mode loop-summary --selection scalar --output target/performance/P086
python3 tools/wtime-bench/analyze.py --input target/performance/P086 --output target/performance/P086/analysis.json
python3 tools/checks/build_budget.py --output target/performance/P086/build.json
python3 tools/checks/build_budget.py --features wtime-runtime/experimental-executors --output target/performance/P086/optional-build.json
python3 tools/wtime-bench/gates.py --task P086 --comparison target/performance/P086 --build target/performance/P086/build.json --optional-build target/performance/P086/optional-build.json --output target/performance/P086/decision.json
```

## Measurement and decision

Selection: `scalar`. Explicit execution setting: `loop-summary`. Decision rule: `measured_promotion`.

Use the paired protocol and family/control/cold/build rules in [MEASUREMENT.md](../MEASUREMENT.md). Predeclare the targeted family and held-out controls before changing the implementation. Keep instrumentation off in production timings. Adopt only with supported gains in both runs; otherwise record rejected/inconclusive and remove default-path cost. A necessary immutable-interface change may be contract_only with its separate justification.

## Before editing

Run these commands before changing the candidate sources. The archive is immutable and captures the immediate predecessor. Retain P001's original archive for milestone totals. For native work, use the named portable control and verify its actual policy/quantum; if this archive already exists with different sources, choose a new recorded run-specific directory and update the comparison path rather than overwriting it.

```sh
python3 tools/wtime-bench/baseline.py --output target/performance/P086-before
```

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. No commands in this planning file have been executed as implementation evidence.
