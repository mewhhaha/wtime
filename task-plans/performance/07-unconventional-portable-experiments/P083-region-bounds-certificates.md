# P083 — Hoist successful unshared bounds checks with conservative guards

Status: **todo**

Outcome: **pending**

Kind: `experiment` · Branch: `optional` · Model: **Sol 6.1 Low**

## Prerequisites

[P002 — Compare separately built baseline and candidate binaries](../00-measurement/P002-paired-driver.md)

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P009 — Add memory width, bulk and growth benchmark families](../00-measurement/P009-memory-workloads.md)

[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)

[P016 — Add explicit executor policies and test integration hooks](../01-execution-contracts/P016-execution-policy.md)

[P019 — Define bounded optimizer effect and source expansion metadata](../01-execution-contracts/P019-effect-certificates.md)

[P020 — Centralize exact source instruction accounting](../01-execution-contracts/P020-exact-execution-accounting.md)

[P043 — Resolve memory bindings and specialize checked scalar accesses](../03-compact-executor/P043-compact-memory-plans.md)

[P063 — Add direct checked unshared memory register operations](../05-register-executor/P063-register-unshared-memory.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C5, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/execution/experimental/regions.rs`
- `crates/wtime-runtime/tests/performance/p083.rs`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

A region certificate proves all eligible affine unshared accesses in a bounded loop are in range. Failed/overflowing guards fall back before guest work; they do not raise an early speculative trap.

## Implementation recipe

1. Start with one memory, one induction variable, constant stride/width and no growth/host/GC/shared/control side effect inside the region.
2. Compute conservative min/max byte ranges using checked wide arithmetic; if wrapping guest addresses or trip counts cannot be proved, decline the certificate.
3. Borrow current safe memory slices only for the effect-free region lifetime and retain source fuel/cancel/stack/trap events; use safe indexing whose bounds the compiler can eliminate.
4. Measure predictable/shuffled/failed-guard memory families and earlier-side-effect trap-order controls, retaining guard cost and preparation bytes.
5. Forward the explicit `region-bounds` setting through the harness/policy integrator and record actual optimized/fallback counts. Persistent research selectors are nondefault; reversible changes to an existing tier use before/after artifacts and are selected only by their gate.

## Meaningful cases

- A proven successful range matches every scalar access/result.
- A failed guard returns to source checks, preserving prior stores and first failing address.
- Growth/shared memory/wrapping induction disqualify the region and no stale slice escapes.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime --features experimental-executors,execution-reference p083_ -- --nocapture
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml --features experimental-executors -j 4
python3 tools/wtime-bench/compare.py --baseline target/performance/P083-before --candidate tools/wtime-bench/target/release/wtime-bench --candidate-mode region-bounds --selection memory --output target/performance/P083
python3 tools/wtime-bench/analyze.py --input target/performance/P083 --output target/performance/P083/analysis.json
python3 tools/checks/build_budget.py --output target/performance/P083/build.json
python3 tools/checks/build_budget.py --features wtime-runtime/experimental-executors --output target/performance/P083/optional-build.json
python3 tools/wtime-bench/gates.py --task P083 --comparison target/performance/P083 --build target/performance/P083/build.json --optional-build target/performance/P083/optional-build.json --output target/performance/P083/decision.json
```

## Measurement and decision

Selection: `memory`. Explicit execution setting: `region-bounds`. Decision rule: `measured_promotion`.

Use the paired protocol and family/control/cold/build rules in [MEASUREMENT.md](../MEASUREMENT.md). Predeclare the targeted family and held-out controls before changing the implementation. Keep instrumentation off in production timings. Adopt only with supported gains in both runs; otherwise record rejected/inconclusive and remove default-path cost. A necessary immutable-interface change may be contract_only with its separate justification.

## Before editing

Run these commands before changing the candidate sources. The archive is immutable and captures the immediate predecessor. Retain P001's original archive for milestone totals. For native work, use the named portable control and verify its actual policy/quantum; if this archive already exists with different sources, choose a new recorded run-specific directory and update the comparison path rather than overwriting it.

```sh
python3 tools/wtime-bench/baseline.py --output target/performance/P083-before
```

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. No commands in this planning file have been executed as implementation evidence.
