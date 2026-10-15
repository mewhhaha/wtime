# P082 — Try one dispatch per safe numeric basic-block tape

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

[P018 — Define source cursors and materialized guest state](../01-execution-contracts/P018-source-cursor.md)

[P019 — Define bounded optimizer effect and source expansion metadata](../01-execution-contracts/P019-effect-certificates.md)

[P020 — Centralize exact source instruction accounting](../01-execution-contracts/P020-exact-execution-accounting.md)

[P055 — Compare safe match, split dispatch and handler-table shapes](../04-specialization-and-fusion/P055-dispatch-shape-experiment.md)

[P057 — Define a small numeric register bytecode and eligibility rules](../05-register-executor/P057-register-ir-contract.md)

[P060 — Implement safe parallel moves and register control transfers](../05-register-executor/P060-register-edge-moves.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C2, C3, C4, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/execution/experimental/block_tape.rs`
- `crates/wtime-runtime/tests/performance/p082.rs`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

A nondefault block tape executes a bounded sequence of numeric micro-ops between control/barrier points, returning an explicit loop action; no recursive Rust handlers or unsafe computed goto.

## Implementation recipe

1. Compile at most one basic block at a time to a small micro-op tape or fixed handler descriptor, preserving source events and explicit control successor.
2. Use an iterative dispatcher with safe state references; prohibit refs/memory effects/calls initially and decline blocks with incomplete source materialization.
3. Compare per-op match, block match and safe function-handler variants using identical source/accounting, and inspect actual code-size/branch evidence.
4. Promote only if dispatch savings survive held-out control shapes and cold/build gates; keep negative results without multiplying default executors.
5. Forward the explicit `block-tape` setting through the harness/policy integrator and record actual optimized/fallback counts. Persistent research selectors are nondefault; reversible changes to an existing tier use before/after artifacts and are selected only by their gate.

## Meaningful cases

- Long loops run with bounded Rust stack use.
- Fuel/cancel stop inside the tape yields the exact original prefix.
- Barrier edges and tiny blocks decline or return safely without duplicated effects.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime --features experimental-executors,execution-reference p082_ -- --nocapture
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml --features experimental-executors -j 4
python3 tools/wtime-bench/compare.py --baseline target/performance/P082-before --candidate tools/wtime-bench/target/release/wtime-bench --candidate-mode block-tape --selection scalar --output target/performance/P082
python3 tools/wtime-bench/analyze.py --input target/performance/P082 --output target/performance/P082/analysis.json
python3 tools/checks/build_budget.py --output target/performance/P082/build.json
python3 tools/checks/build_budget.py --features wtime-runtime/experimental-executors --output target/performance/P082/optional-build.json
python3 tools/wtime-bench/gates.py --task P082 --comparison target/performance/P082 --build target/performance/P082/build.json --optional-build target/performance/P082/optional-build.json --output target/performance/P082/decision.json
```

## Measurement and decision

Selection: `scalar`. Explicit execution setting: `block-tape`. Decision rule: `measured_promotion`.

Use the paired protocol and family/control/cold/build rules in [MEASUREMENT.md](../MEASUREMENT.md). Predeclare the targeted family and held-out controls before changing the implementation. Keep instrumentation off in production timings. Adopt only with supported gains in both runs; otherwise record rejected/inconclusive and remove default-path cost. A necessary immutable-interface change may be contract_only with its separate justification.

## Before editing

Run these commands before changing the candidate sources. The archive is immutable and captures the immediate predecessor. Retain P001's original archive for milestone totals. For native work, use the named portable control and verify its actual policy/quantum; if this archive already exists with different sources, choose a new recorded run-specific directory and update the comparison path rather than overwriting it.

```sh
python3 tools/wtime-bench/baseline.py --output target/performance/P082-before
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
  "task": "P082",
  "tests_claimed": 0,
  "timing_evidence": false
}
```
