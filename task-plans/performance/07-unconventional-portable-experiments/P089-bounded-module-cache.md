# P089 — Cache identical prepared modules within one Engine

Status: **todo**

Outcome: **pending**

Kind: `experiment` · Branch: `optional` · Model: **Sol 6.1 Low**

## Prerequisites

[P001 — Archive the current runtime and benchmark baseline](../00-measurement/P001-freeze-baseline.md)

[P002 — Compare separately built baseline and candidate binaries](../00-measurement/P002-paired-driver.md)

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)

[P015 — Add reproducible application-shaped held-out kernels](../00-measurement/P015-representative-kernels.md)

[P016 — Add explicit executor policies and test integration hooks](../01-execution-contracts/P016-execution-policy.md)

[P017 — Make every Module clone share immutable metadata](../01-execution-contracts/P017-shared-module-payload.md)

[P021 — Compute bounded per-function execution metadata](../01-execution-contracts/P021-function-execution-plans.md)

[P066 — Compile register code lazily at bounded function thresholds](../05-register-executor/P066-function-boundary-tiering.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/execution/module_cache.rs`
- `crates/wtime-runtime/src/lib.rs`
- `crates/wtime-runtime/tests/performance/p089.rs`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

An opt-in bounded Engine-local cache reuses immutable admitted modules keyed by complete source bytes and feature/limit/spec/execution policy; hash collisions require byte/key equality and no Store binding is cached.

## Implementation recipe

1. Keep cache disabled by default and define count/retained-byte/LRU bounds; use standard-library hashing plus exact equality rather than adding a crypto dependency for correctness.
2. Cache only completely admitted modules under the same Engine/type registry/provenance, retaining validation/resource identity and excluding per-instance handles.
3. Synchronize lookups/publication without compiling under cache/canonical locks and avoid permanent OOM/error caching; release ownership on eviction/drop.
4. Measure fresh-engine and reused-engine cold/cache-hit paths separately, including hash/byte comparison cost and adversarial collision/churn controls.
5. Forward the explicit `module-cache` setting through the harness/policy integrator and record actual optimized/fallback counts. Persistent research selectors are nondefault; reversible changes to an existing tier use before/after artifacts and are selected only by their gate.

## Meaningful cases

- Injected hash collisions cannot return the wrong module.
- Different feature/limit/policy/Engine identity misses or rejects correctly.
- Concurrent duplicate prepares and eviction keep bounded bytes and correct canonical identity.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime --features experimental-executors,execution-reference p089_ -- --nocapture
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml --features experimental-executors -j 4
python3 tools/wtime-bench/compare.py --baseline target/performance/P089-before --candidate tools/wtime-bench/target/release/wtime-bench --candidate-mode module-cache --selection representative --output target/performance/P089
python3 tools/wtime-bench/analyze.py --input target/performance/P089 --output target/performance/P089/analysis.json
python3 tools/checks/build_budget.py --output target/performance/P089/build.json
python3 tools/checks/build_budget.py --features wtime-runtime/experimental-executors --output target/performance/P089/optional-build.json
python3 tools/wtime-bench/gates.py --task P089 --comparison target/performance/P089 --build target/performance/P089/build.json --optional-build target/performance/P089/optional-build.json --output target/performance/P089/decision.json
```

## Measurement and decision

Selection: `representative`. Explicit execution setting: `module-cache`. Decision rule: `measured_promotion`.

Use the paired protocol and family/control/cold/build rules in [MEASUREMENT.md](../MEASUREMENT.md). Predeclare the targeted family and held-out controls before changing the implementation. Keep instrumentation off in production timings. Adopt only with supported gains in both runs; otherwise record rejected/inconclusive and remove default-path cost. A necessary immutable-interface change may be contract_only with its separate justification.

## Before editing

Run these commands before changing the candidate sources. The archive is immutable and captures the immediate predecessor. Retain P001's original archive for milestone totals. For native work, use the named portable control and verify its actual policy/quantum; if this archive already exists with different sources, choose a new recorded run-specific directory and update the comparison path rather than overwriting it.

```sh
python3 tools/wtime-bench/baseline.py --output target/performance/P089-before
```

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. No commands in this planning file have been executed as implementation evidence.
