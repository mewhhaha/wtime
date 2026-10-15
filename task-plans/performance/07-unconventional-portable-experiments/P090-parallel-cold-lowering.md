# P090 — Try bounded parallel lowering for large modules only

Status: **deferred**

Outcome: **not_selected**

Kind: `experiment` · Branch: `optional` · Model: **Sol 6.1 Low**

## Prerequisites

[P002 — Compare separately built baseline and candidate binaries](../00-measurement/P002-paired-driver.md)

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)

[P015 — Add reproducible application-shaped held-out kernels](../00-measurement/P015-representative-kernels.md)

[P016 — Add explicit executor policies and test integration hooks](../01-execution-contracts/P016-execution-policy.md)

[P021 — Compute bounded per-function execution metadata](../01-execution-contracts/P021-function-execution-plans.md)

[P036 — Define small immutable execution instructions and side tables](../03-compact-executor/P036-compact-op-format.md)

[P057 — Define a small numeric register bytecode and eligibility rules](../05-register-executor/P057-register-ir-contract.md)

[P066 — Compile register code lazily at bounded function thresholds](../05-register-executor/P066-function-boundary-tiering.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/execution/experimental/parallel_lower.rs`
- `crates/wtime-runtime/tests/performance/p090.rs`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

An opt-in large-module policy parallelizes independent compact/register function lowering after serial admission/type interning; worker/work budgets and deterministic result/error order are explicit.

## Implementation recipe

1. Leave decode/validation/canonical interning serial initially, measure lowering subphase fraction and decline parallel work for small modules.
2. Use bounded std::thread::scope workers capped by configured/quota-aware limits with fallible spawn handling and indexed result ownership; no Tokio/Rayon dependency.
3. Aggregate optimizer bytes/work transactionally and choose deterministic earliest-source error/decline while cancelling or joining all started workers safely.
4. Measure 1000-function and mixed-size preparations including worker startup, CPU time/RSS/cold latency, keeping default serial unless broad gates justify it.
5. Forward the explicit `parallel-lowering` setting through the harness/policy integrator and record actual optimized/fallback counts. Persistent research selectors are nondefault; reversible changes to an existing tier use before/after artifacts and are selected only by their gate.

## Meaningful cases

- Serial/parallel lowering publishes equivalent maps and canonical contexts.
- Spawn/allocation/work failure joins workers and falls back without partial code.
- Small modules remain serial and worker count/code/RSS stay bounded.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime --features experimental-executors,execution-reference p090_ -- --nocapture
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml --features experimental-executors -j 4
python3 tools/wtime-bench/compare.py --baseline target/performance/P090-before --candidate tools/wtime-bench/target/release/wtime-bench --candidate-mode parallel-lowering --selection representative --output target/performance/P090
python3 tools/wtime-bench/analyze.py --input target/performance/P090 --output target/performance/P090/analysis.json
python3 tools/checks/build_budget.py --output target/performance/P090/build.json
python3 tools/checks/build_budget.py --features wtime-runtime/experimental-executors --output target/performance/P090/optional-build.json
python3 tools/wtime-bench/gates.py --task P090 --comparison target/performance/P090 --build target/performance/P090/build.json --optional-build target/performance/P090/optional-build.json --output target/performance/P090/decision.json
```

## Measurement and decision

Selection: `representative`. Explicit execution setting: `parallel-lowering`. Decision rule: `measured_promotion`.

Use the paired protocol and family/control/cold/build rules in [MEASUREMENT.md](../MEASUREMENT.md). Predeclare the targeted family and held-out controls before changing the implementation. Keep instrumentation off in production timings. Adopt only with supported gains in both runs; otherwise record rejected/inconclusive and remove default-path cost. A necessary immutable-interface change may be contract_only with its separate justification.

## Before editing

Run these commands before changing the candidate sources. The archive is immutable and captures the immediate predecessor. Retain P001's original archive for milestone totals. For native work, use the named portable control and verify its actual policy/quantum; if this archive already exists with different sources, choose a new recorded run-specific directory and update the comparison path rather than overwriting it.

```sh
python3 tools/wtime-bench/baseline.py --output target/performance/P090-before
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
  "task": "P090",
  "tests_claimed": 0,
  "timing_evidence": false
}
```
