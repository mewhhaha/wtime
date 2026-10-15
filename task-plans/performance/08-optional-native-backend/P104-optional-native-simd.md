# P104 — Isolate a tiny feature-detected native SIMD kernel set

Status: **deferred**

Outcome: **not_selected**

Kind: `experiment` · Branch: `optional` · Model: **Sol 6.1 Low**

## Prerequisites

[P002 — Compare separately built baseline and candidate binaries](../00-measurement/P002-paired-driver.md)

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P010 — Add SIMD and strict floating-point benchmarks](../00-measurement/P010-simd-float-workloads.md)

[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)

[P016 — Add explicit executor policies and test integration hooks](../01-execution-contracts/P016-execution-policy.md)

[P074 — Specialize portable SIMD bitwise and select operations](../06-memory-gc-simd-and-continuations/P074-portable-vector-bitops.md)

[P075 — Try typed safe lane kernels that autovectorize](../06-memory-gc-simd-and-continuations/P075-portable-lane-specialization.md)

[P094 — Decide the tiny native backend boundary and budget](P094-native-boundary-decision.md)

[P095 — Create the minimal optional numeric backend interface](P095-native-ir-and-abi.md)

[P099 — Add exact fuel, traps and bounded cancellation to native execution](P099-native-accounting-exits.md)

Conditional branch: `P094.outcome == go`. If its decision is declined, record this task as deferred/not_selected; do not claim it implemented.

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-native/src/simd.rs`
- `crates/wtime-runtime/src/execution/native.rs`
- `crates/wtime-native/tests/simd.rs`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

Safe wrappers expose a few audited architecture SIMD integer kernels selected only by an explicit optional native policy and actual CPU capability; portable SIMD remains complete and default.

## Implementation recipe

1. Start with bitwise and i32x4 add/mul or equivalent profiled kernels; use std::arch unsafe intrinsics only inside the declared native crate.
2. Guard every nonbaseline target feature using real runtime detection and target cfg, with identical portable fallback and no UB on an unsupported CPU.
3. Preserve vector byte/lane semantics and forbid widening scope to unchecked shared loads or fast float/relaxed approximations.
4. Compare kernel and end-to-end SIMD performance/build cost, proving dispatch overhead and supported/fallback paths with forced test capabilities.
5. Forward the explicit `native-simd` setting through the harness/policy integrator and record actual optimized/fallback counts. Persistent research selectors are nondefault; reversible changes to an existing tier use before/after artifacts and are selected only by their gate.
6. Attach the separately labeled optional-build report to the performance decision, alongside the default build report; a default-only build cannot establish native/experimental build cost.

## Meaningful cases

- Forced unsupported capability chooses portable code and never executes the intrinsic.
- Random vector/boundary lanes match independent portable semantics.
- Default build has no native/unsafe dependency and feature-enabled kernels have target evidence.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-native simd -- --nocapture
python3 tools/checks/dependency_budget.py
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml --features native-executor -j 4
python3 tools/wtime-bench/compare.py --baseline target/performance/P104-native-matched-baseline --candidate tools/wtime-bench/target/release/wtime-bench --candidate-mode native-simd --selection simd --output target/performance/P104
python3 tools/wtime-bench/analyze.py --input target/performance/P104 --output target/performance/P104/analysis.json
python3 tools/checks/build_budget.py --output target/performance/P104/build.json
python3 tools/checks/build_budget.py --features wtime-runtime/native-executor --output target/performance/P104/optional-build.json
python3 tools/wtime-bench/gates.py --task P104 --comparison target/performance/P104 --build target/performance/P104/build.json --optional-build target/performance/P104/optional-build.json --output target/performance/P104/decision.json
```

## Measurement and decision

Selection: `simd`. Explicit execution setting: `native-simd`. Decision rule: `measured_promotion`.

Use the paired protocol and family/control/cold/build rules in [MEASUREMENT.md](../MEASUREMENT.md). Predeclare the targeted family and held-out controls before changing the implementation. Keep instrumentation off in production timings. Adopt only with supported gains in both runs; otherwise record rejected/inconclusive and remove default-path cost. A necessary immutable-interface change may be contract_only with its separate justification.

## Before editing

Run these commands before changing the candidate sources. The archive is immutable and captures the immediate predecessor. Retain P001's original archive for milestone totals. For native work, use the named portable control and verify its actual policy/quantum; if this archive already exists with different sources, choose a new recorded run-specific directory and update the comparison path rather than overwriting it.

```sh
python3 tools/wtime-bench/baseline.py --output target/performance/P104-native-matched-baseline --mode portable
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
  "task": "P104",
  "tests_claimed": 0,
  "timing_evidence": false
}
```
