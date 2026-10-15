# P103 — Extend native numeric code with checked traps and strict floats

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

[P062 — Add strict floating-point register operations](../05-register-executor/P062-register-float-execution.md)

[P097 — Emit a tiny verified x86_64 integer block subset](P097-x86-integer-block-emitter.md)

[P099 — Add exact fuel, traps and bounded cancellation to native execution](P099-native-accounting-exits.md)

[P100 — Compile and retain native code only for eligible hot functions](P100-bounded-native-tiering.md)

Conditional branch: `P094.outcome == go`. If its decision is declined, record this task as deferred/not_selected; do not claim it implemented.

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-native/src/x86/numeric.rs`
- `crates/wtime-runtime/src/execution/native.rs`
- `crates/wtime-native/tests/numeric.rs`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

Native arithmetic extends only to independently covered integer division and f32/f64 add/mul/move; sensitive min/max/conversions/NaN rules use exact helper exits or remain portable.

## Implementation recipe

1. Implement integer divide/remainder guards and explicit source trap exits before hardware exceptions.
2. Add a small strict float instruction subset with raw-bit slot handling and no fast-math/contracted operations unless permitted by the specific Wasm opcode.
3. Keep unsupported rounding/min/max/conversion/relaxed operations on verified helper exits, and inspect ABI floating register/state requirements.
4. Run bit/allowed-NaN/exhaustive boundary cases and measure actual float kernels before expanding the ISA subset.
5. Forward the explicit `native-strict-numerics` setting through the harness/policy integrator and record actual optimized/fallback counts. Persistent research selectors are nondefault; reversible changes to an existing tier use before/after artifacts and are selected only by their gate.
6. Attach the separately labeled optional-build report to the performance decision, alongside the default build report; a default-only build cannot establish native/experimental build cost.

## Meaningful cases

- Signed-zero/NaN constants and arithmetic satisfy normative allowed sets.
- MIN/-1, divide-by-zero and conversion-boundary paths return managed original traps.
- Unsupported numeric rules decline/exit without corrupting ABI or source state.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-native numeric -- --nocapture
python3 tools/wtime-bench/tier_check.py --tier native --selection simd --output target/performance/P103/conformance
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml --features native-executor -j 4
python3 tools/wtime-bench/compare.py --baseline target/performance/P103-native-matched-baseline --candidate tools/wtime-bench/target/release/wtime-bench --candidate-mode native-strict-numerics --selection simd --output target/performance/P103
python3 tools/wtime-bench/analyze.py --input target/performance/P103 --output target/performance/P103/analysis.json
python3 tools/checks/build_budget.py --output target/performance/P103/build.json
python3 tools/checks/build_budget.py --features wtime-runtime/native-executor --output target/performance/P103/optional-build.json
python3 tools/wtime-bench/gates.py --task P103 --comparison target/performance/P103 --build target/performance/P103/build.json --optional-build target/performance/P103/optional-build.json --output target/performance/P103/decision.json
```

## Measurement and decision

Selection: `simd`. Explicit execution setting: `native-strict-numerics`. Decision rule: `measured_promotion`.

Use the paired protocol and family/control/cold/build rules in [MEASUREMENT.md](../MEASUREMENT.md). Predeclare the targeted family and held-out controls before changing the implementation. Keep instrumentation off in production timings. Adopt only with supported gains in both runs; otherwise record rejected/inconclusive and remove default-path cost. A necessary immutable-interface change may be contract_only with its separate justification.

## Before editing

Run these commands before changing the candidate sources. The archive is immutable and captures the immediate predecessor. Retain P001's original archive for milestone totals. For native work, use the named portable control and verify its actual policy/quantum; if this archive already exists with different sources, choose a new recorded run-specific directory and update the comparison path rather than overwriting it.

```sh
python3 tools/wtime-bench/baseline.py --output target/performance/P103-native-matched-baseline --mode portable
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
  "task": "P103",
  "tests_claimed": 0,
  "timing_evidence": false
}
```
