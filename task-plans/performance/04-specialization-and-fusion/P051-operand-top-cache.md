# P051 — Try a two-value numeric operand cache

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

[P020 — Centralize exact source instruction accounting](../01-execution-contracts/P020-exact-execution-accounting.md)

[P038 — Lower constants, locals and common integer operations](../03-compact-executor/P038-compact-scalar-lowering.md)

[P044 — Materialize compact state for GC, exceptions and continuations](../03-compact-executor/P044-compact-proposal-boundaries.md)

[P047 — Fuse a bounded set of local and constant arithmetic patterns](P047-local-arithmetic-fusions.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C2, C3, C4, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/execution/compact/stack_cache.rs`
- `crates/wtime-runtime/src/execution/compact/execute.rs`
- `crates/wtime-runtime/tests/performance/p051.rs`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

A nondefault two-slot cache holds numeric-only top operands between compatible compact ops; it flushes before every barrier/control exit and records true logical stack height including cached slots.

## Implementation recipe

1. Implement a safe tiny cache state for scalar numeric Values first, avoiding references/V128 and keeping explicit spill/fill routines.
2. Count cached values in transient operand limits and materialize them in exact order at fuel/cancel exits, branch moves, host calls and proposal adapters.
3. Decline cache use when an effect/source map cannot identify a safe flush point; keep generic stack execution as a selectable control.
4. Measure arithmetic and mixed-barrier families plus Rust register/code-size effects; remove the cache from the default if it merely duplicates compiler optimizations.
5. Forward the explicit `compact-top-cache` setting through the harness/policy integrator and record actual optimized/fallback counts. Persistent research selectors are nondefault; reversible changes to an existing tier use before/after artifacts and are selected only by their gate.

## Meaningful cases

- Spill/fill preserves order through unary/binary/branch patterns.
- Logical operand limits include cached values at every source boundary.
- Host reentry/trap/continuation adapters see complete stack state.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime --features compact-executor,experimental-executors,execution-reference p051_ -- --nocapture
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml --features compact-executor,experimental-executors -j 4
python3 tools/wtime-bench/compare.py --baseline target/performance/P051-before --candidate tools/wtime-bench/target/release/wtime-bench --candidate-mode compact-top-cache --selection scalar --output target/performance/P051
python3 tools/wtime-bench/analyze.py --input target/performance/P051 --output target/performance/P051/analysis.json
python3 tools/checks/build_budget.py --output target/performance/P051/build.json
python3 tools/checks/build_budget.py --features wtime-runtime/compact-executor --output target/performance/P051/optional-build.json
python3 tools/wtime-bench/gates.py --task P051 --comparison target/performance/P051 --build target/performance/P051/build.json --optional-build target/performance/P051/optional-build.json --output target/performance/P051/decision.json
```

## Measurement and decision

Selection: `scalar`. Explicit execution setting: `compact-top-cache`. Decision rule: `measured_promotion`.

Use the paired protocol and family/control/cold/build rules in [MEASUREMENT.md](../MEASUREMENT.md). Predeclare the targeted family and held-out controls before changing the implementation. Keep instrumentation off in production timings. Adopt only with supported gains in both runs; otherwise record rejected/inconclusive and remove default-path cost. A necessary immutable-interface change may be contract_only with its separate justification.

## Before editing

Run these commands before changing the candidate sources. The archive is immutable and captures the immediate predecessor. Retain P001's original archive for milestone totals. For native work, use the named portable control and verify its actual policy/quantum; if this archive already exists with different sources, choose a new recorded run-specific directory and update the comparison path rather than overwriting it.

```sh
python3 tools/wtime-bench/baseline.py --output target/performance/P051-before
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
  "task": "P051",
  "tests_claimed": 0,
  "timing_evidence": false
}
```
