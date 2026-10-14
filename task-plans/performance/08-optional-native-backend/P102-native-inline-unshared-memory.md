# P102 — Try audited native checked access to a borrowed unshared view

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

[P083 — Hoist successful unshared bounds checks with conservative guards](../07-unconventional-portable-experiments/P083-region-bounds-certificates.md)

[P095 — Create the minimal optional numeric backend interface](P095-native-ir-and-abi.md)

[P097 — Emit a tiny verified x86_64 integer block subset](P097-x86-integer-block-emitter.md)

[P098 — Add checked branches and bounded native numeric loops](P098-x86-control-emitter.md)

[P099 — Add exact fuel, traps and bounded cancellation to native execution](P099-native-accounting-exits.md)

[P101 — Cross native memory boundaries through safe checked callouts](P101-native-memory-callouts.md)

Conditional branch: `P094.outcome == go`. If its decision is declined, record this task as deferred/not_selected; do not claim it implemented.

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C5, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-native/src/x86/memory.rs`
- `crates/wtime-native/src/abi.rs`
- `crates/wtime-runtime/src/execution/native.rs`
- `crates/wtime-native/tests/memory.rs`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

A safe native run API temporarily borrows initialized unshared bytes and verifies length/width/address guards before machine access; pointers cannot outlive the call, and shared/growth/host operations are excluded.

## Implementation recipe

1. Define a private borrowed-memory ABI in the native crate; safe runtime borrowing prevents growth/reentry/alias mutation until return and no pointer enters cached code metadata.
2. Emit checked immediate addition with carry handling and range guard width<=len then address<=len-width; respect i32 wrapping before checked memory offset and source trap ordering.
3. Audit load/store templates, width/sign extension/little-endian rules and code verifier evidence; return an explicit trap/status instead of using signals or inaccessible guard pages.
4. Run adversarial offsets/lengths, source-prefix side effects and native sanitizers where available, then measure matched-policy memory cases and bounds-guard cost.
5. Forward the explicit `native-inline-memory` setting through the harness/policy integrator and record actual optimized/fallback counts. Persistent research selectors are nondefault; reversible changes to an existing tier use before/after artifacts and are selected only by their gate.
6. Attach the separately labeled optional-build report to the performance decision, alongside the default build report; a default-only build cannot establish native/experimental build cost.

## Meaningful cases

- Zero length, width>end, u64 overflow and last-valid-byte guards are correct.
- No cached memory pointer survives growth/drop or escapes a run.
- Unaligned widths/signed loads and prior-store/OOB ordering match safe Rust.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-native memory -- --nocapture
python3 tools/wtime-bench/tier_check.py --tier native --selection memory --output target/performance/P102/conformance
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml --features native-executor -j 4
python3 tools/wtime-bench/compare.py --baseline target/performance/P102-native-matched-baseline --candidate tools/wtime-bench/target/release/wtime-bench --candidate-mode native-inline-memory --selection memory --output target/performance/P102
python3 tools/wtime-bench/analyze.py --input target/performance/P102 --output target/performance/P102/analysis.json
python3 tools/checks/build_budget.py --output target/performance/P102/build.json
python3 tools/checks/build_budget.py --features wtime-runtime/native-executor --output target/performance/P102/optional-build.json
python3 tools/wtime-bench/gates.py --task P102 --comparison target/performance/P102 --build target/performance/P102/build.json --optional-build target/performance/P102/optional-build.json --output target/performance/P102/decision.json
```

## Measurement and decision

Selection: `memory`. Explicit execution setting: `native-inline-memory`. Decision rule: `measured_promotion`.

Use the paired protocol and family/control/cold/build rules in [MEASUREMENT.md](../MEASUREMENT.md). Predeclare the targeted family and held-out controls before changing the implementation. Keep instrumentation off in production timings. Adopt only with supported gains in both runs; otherwise record rejected/inconclusive and remove default-path cost. A necessary immutable-interface change may be contract_only with its separate justification.

## Before editing

Run these commands before changing the candidate sources. The archive is immutable and captures the immediate predecessor. Retain P001's original archive for milestone totals. For native work, use the named portable control and verify its actual policy/quantum; if this archive already exists with different sources, choose a new recorded run-specific directory and update the comparison path rather than overwriting it.

```sh
python3 tools/wtime-bench/baseline.py --output target/performance/P102-native-matched-baseline --mode portable
```

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. No commands in this planning file have been executed as implementation evidence.
