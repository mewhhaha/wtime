# P085 — Execute guarded scalar traces with exact side exits

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

[P020 — Centralize exact source instruction accounting](../01-execution-contracts/P020-exact-execution-accounting.md)

[P058 — Translate numeric stack code into direct slot operands](../05-register-executor/P058-stack-to-register-lowering.md)

[P060 — Implement safe parallel moves and register control transfers](../05-register-executor/P060-register-edge-moves.md)

[P061 — Execute checked integer register operations](../05-register-executor/P061-register-integer-execution.md)

[P084 — Record short scalar loop traces at verified backedges](P084-bounded-trace-recording.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/execution/experimental/trace_execute.rs`
- `crates/wtime-runtime/src/execution/experimental/trace_record.rs`
- `crates/wtime-runtime/tests/performance/p085.rs`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

Eligible traces enter only at a verified numeric loop backedge, execute safe slot operations, and exit before the next unexecuted source operation with complete state; initial OSR is limited to this one boundary.

## Implementation recipe

1. Compile recorded numeric operations into a bounded straight tape with explicit guard branches and source-prefix accounting.
2. Define entry slot copies from the verified loop layout and exit maps for every failed guard, fuel/cancel boundary and resource check.
3. On exit restore compact/decoded activation state exactly, continue without replay and bound trace retries/eviction to avoid adversarial branch churn.
4. Measure stable/changing/held-out loop paths, entry/exit overhead and code/preparation cost before adopting any trace policy.
5. Forward the explicit `scalar-trace` setting through the harness/policy integrator and record actual optimized/fallback counts. Persistent research selectors are nondefault; reversible changes to an existing tier use before/after artifacts and are selected only by their gate.

## Meaningful cases

- Changing an input/branch causes a correct side exit before unexecuted work.
- Each fuel/source stop and backedge entry has identical locals/operands.
- Trace churn remains bounded and unsupported effects always return to portable execution.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime --features experimental-executors,execution-reference p085_ -- --nocapture
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml --features experimental-executors -j 4
python3 tools/wtime-bench/compare.py --baseline target/performance/P085-before --candidate tools/wtime-bench/target/release/wtime-bench --candidate-mode scalar-trace --selection scalar --output target/performance/P085
python3 tools/wtime-bench/analyze.py --input target/performance/P085 --output target/performance/P085/analysis.json
python3 tools/checks/build_budget.py --output target/performance/P085/build.json
python3 tools/checks/build_budget.py --features wtime-runtime/experimental-executors --output target/performance/P085/optional-build.json
python3 tools/wtime-bench/gates.py --task P085 --comparison target/performance/P085 --build target/performance/P085/build.json --optional-build target/performance/P085/optional-build.json --output target/performance/P085/decision.json
```

## Measurement and decision

Selection: `scalar`. Explicit execution setting: `scalar-trace`. Decision rule: `measured_promotion`.

Use the paired protocol and family/control/cold/build rules in [MEASUREMENT.md](../MEASUREMENT.md). Predeclare the targeted family and held-out controls before changing the implementation. Keep instrumentation off in production timings. Adopt only with supported gains in both runs; otherwise record rejected/inconclusive and remove default-path cost. A necessary immutable-interface change may be contract_only with its separate justification.

## Before editing

Run these commands before changing the candidate sources. The archive is immutable and captures the immediate predecessor. Retain P001's original archive for milestone totals. For native work, use the named portable control and verify its actual policy/quantum; if this archive already exists with different sources, choose a new recorded run-specific directory and update the comparison path rather than overwriting it.

```sh
python3 tools/wtime-bench/baseline.py --output target/performance/P085-before
```

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. No commands in this planning file have been executed as implementation evidence.
