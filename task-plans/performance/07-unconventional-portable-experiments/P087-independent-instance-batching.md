# P087 — Try lockstep execution of independent numeric invocations

Status: **deferred**

Outcome: **not_selected**

Kind: `experiment` · Branch: `optional` · Model: **Sol 6.1 Low**

## Prerequisites

[P002 — Compare separately built baseline and candidate binaries](../00-measurement/P002-paired-driver.md)

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P008 — Add scaling, branch and call benchmark families](../00-measurement/P008-scalar-workloads.md)

[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)

[P016 — Add explicit executor policies and test integration hooks](../01-execution-contracts/P016-execution-policy.md)

[P057 — Define a small numeric register bytecode and eligibility rules](../05-register-executor/P057-register-ir-contract.md)

[P058 — Translate numeric stack code into direct slot operands](../05-register-executor/P058-stack-to-register-lowering.md)

[P061 — Execute checked integer register operations](../05-register-executor/P061-register-integer-execution.md)

[P067 — Evaluate register execution and adaptive break-even](../05-register-executor/P067-register-milestone.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/execution/experimental/batch.rs`
- `tools/wtime-bench/fixtures/scalar/batch.wat`
- `crates/wtime-runtime/tests/performance/p087.rs`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

An opt-in experimental run_batch API processes bounded independent numeric inputs using safe lane arrays, preserving per-item result/error/accounting. It reports throughput and per-item latency, not a single-module speedup.

## Implementation recipe

1. Limit eligibility to the same immutable pure numeric function with no shared memory/import/ref/host/continuation effects; retain one logical state/fuel/cancellation identity per invocation.
2. Execute identical opcode paths over small fixed lane arrays, splitting/diverting divergent lanes to the normal executor with exact next source state.
3. Cap batch size/storage and retain independent errors/results in input order; reject aliasing/shared-state workloads rather than assuming independence.
4. Measure equal-work sequential versus batched throughput and actual completion latency, including packing/setup and held-out divergent inputs.
5. Forward the explicit `numeric-batch` setting through the harness/policy integrator and record actual optimized/fallback counts. Persistent research selectors are nondefault; reversible changes to an existing tier use before/after artifacts and are selected only by their gate.

## Meaningful cases

- Different inputs/results and per-item fuel failures stay independent and ordered.
- Divergent branches fall back without duplicating work.
- Shared/imported/ref workloads decline and batch storage remains bounded.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime --features experimental-executors,execution-reference p087_ -- --nocapture
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml --features experimental-executors -j 4
python3 tools/wtime-bench/compare.py --baseline target/performance/P087-before --candidate tools/wtime-bench/target/release/wtime-bench --candidate-mode numeric-batch --selection scalar --output target/performance/P087
python3 tools/wtime-bench/analyze.py --input target/performance/P087 --output target/performance/P087/analysis.json
python3 tools/checks/build_budget.py --output target/performance/P087/build.json
python3 tools/checks/build_budget.py --features wtime-runtime/experimental-executors --output target/performance/P087/optional-build.json
python3 tools/wtime-bench/gates.py --task P087 --comparison target/performance/P087 --build target/performance/P087/build.json --optional-build target/performance/P087/optional-build.json --output target/performance/P087/decision.json
```

## Measurement and decision

Selection: `scalar`. Explicit execution setting: `numeric-batch`. Decision rule: `measured_promotion`.

Use the paired protocol and family/control/cold/build rules in [MEASUREMENT.md](../MEASUREMENT.md). Predeclare the targeted family and held-out controls before changing the implementation. Keep instrumentation off in production timings. Adopt only with supported gains in both runs; otherwise record rejected/inconclusive and remove default-path cost. A necessary immutable-interface change may be contract_only with its separate justification.

## Before editing

Run these commands before changing the candidate sources. The archive is immutable and captures the immediate predecessor. Retain P001's original archive for milestone totals. For native work, use the named portable control and verify its actual policy/quantum; if this archive already exists with different sources, choose a new recorded run-specific directory and update the comparison path rather than overwriting it.

```sh
python3 tools/wtime-bench/baseline.py --output target/performance/P087-before
```

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. Executed evidence is recorded below.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/final-optional-dispositions-0002/proof.json": "5a2943b1b7f0e4fedf44a2d4e96153937683fc97d45e079e55c0ef43a5e3b97f"
  },
  "evidence": [
    {
      "bytes": 38990,
      "path": "/workspace/wtime/target/performance/final-optional-dispositions-0002/proof.json",
      "sha256": "5a2943b1b7f0e4fedf44a2d4e96153937683fc97d45e079e55c0ef43a5e3b97f"
    }
  ],
  "implementation_claimed": false,
  "outcome": "not_selected",
  "reason": "Conditional research is not selected for the current portfolio; this record claims no implementation, test execution, or speed result.",
  "schema": "wtime-task-disposition-record-v1",
  "status": "deferred",
  "task": "P087",
  "tests_claimed": 0,
  "timing_evidence": false
}
```
