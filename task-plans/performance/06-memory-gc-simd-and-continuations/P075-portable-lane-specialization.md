# P075 — Try typed safe lane kernels that autovectorize

Status: **todo**

Outcome: **pending**

Kind: `experiment` · Branch: `optional` · Model: **Sol 6.1 Low**

## Prerequisites

[P002 — Compare separately built baseline and candidate binaries](../00-measurement/P002-paired-driver.md)

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P010 — Add SIMD and strict floating-point benchmarks](../00-measurement/P010-simd-float-workloads.md)

[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)

[P016 — Add explicit executor policies and test integration hooks](../01-execution-contracts/P016-execution-policy.md)

[P074 — Specialize portable SIMD bitwise and select operations](P074-portable-vector-bitops.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C2, C3, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/simd.rs`
- `crates/wtime-runtime/src/simd_kernels.rs`
- `crates/wtime-runtime/tests/performance/p075.rs`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

A bounded set of hot integer lane kernels uses safe fixed arrays and explicit conversions instead of generic lane width/callback loops; all saturating/float/relaxed semantics remain independently checked.

## Implementation recipe

1. Select at most three profiled integer add/mul/saturation kernels and implement fixed-width array operations with safe conversions.
2. Inspect optimized assembly where tooling permits to see whether LLVM autovectorizes; record actual architecture/codegen settings without treating assembly appearance as a timing win.
3. Leave sensitive float min/max/NaN operations on existing helpers unless a separate exact-bit implementation is covered; never enable unrestricted fast-math.
4. Benchmark vector families/held-out lane widths and measure Rust build/code-size growth before adoption.
5. Forward the explicit `portable-lane-kernels` setting through the harness/policy integrator and record actual optimized/fallback counts. Persistent research selectors are nondefault; reversible changes to an existing tier use before/after artifacts and are selected only by their gate.

## Meaningful cases

- Lane wrap/saturation/extending boundaries match independent arrays.
- Unsupported lane widths use the complete portable fallback.
- Float signed-zero/NaN controls and relaxed permitted sets remain correct.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime --features experimental-executors,execution-reference p075_ -- --nocapture
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml --features experimental-executors -j 4
python3 tools/wtime-bench/compare.py --baseline target/performance/P075-before --candidate tools/wtime-bench/target/release/wtime-bench --candidate-mode portable-lane-kernels --selection simd --output target/performance/P075
python3 tools/wtime-bench/analyze.py --input target/performance/P075 --output target/performance/P075/analysis.json
python3 tools/checks/build_budget.py --output target/performance/P075/build.json
python3 tools/checks/build_budget.py --features wtime-runtime/experimental-executors --output target/performance/P075/optional-build.json
python3 tools/wtime-bench/gates.py --task P075 --comparison target/performance/P075 --build target/performance/P075/build.json --optional-build target/performance/P075/optional-build.json --output target/performance/P075/decision.json
```

## Measurement and decision

Selection: `simd`. Explicit execution setting: `portable-lane-kernels`. Decision rule: `measured_promotion`.

Use the paired protocol and family/control/cold/build rules in [MEASUREMENT.md](../MEASUREMENT.md). Predeclare the targeted family and held-out controls before changing the implementation. Keep instrumentation off in production timings. Adopt only with supported gains in both runs; otherwise record rejected/inconclusive and remove default-path cost. A necessary immutable-interface change may be contract_only with its separate justification.

## Before editing

Run these commands before changing the candidate sources. The archive is immutable and captures the immediate predecessor. Retain P001's original archive for milestone totals. For native work, use the named portable control and verify its actual policy/quantum; if this archive already exists with different sources, choose a new recorded run-specific directory and update the comparison path rather than overwriting it.

```sh
python3 tools/wtime-bench/baseline.py --output target/performance/P075-before
```

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. No commands in this planning file have been executed as implementation evidence.
