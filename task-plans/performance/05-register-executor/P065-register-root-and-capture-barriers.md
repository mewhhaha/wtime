# P065 — Verify roots and captured state around register exits

Status: **todo**

Outcome: **pending**

Kind: `gate` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)

[P022 — Add adversarial state and interaction tier fixtures](../01-execution-contracts/P022-state-differential-fixtures.md)

[P044 — Materialize compact state for GC, exceptions and continuations](../03-compact-executor/P044-compact-proposal-boundaries.md)

[P062 — Add strict floating-point register operations](P062-register-float-execution.md)

[P063 — Add direct checked unshared memory register operations](P063-register-unshared-memory.md)

[P064 — Exit register execution before calls and resume decoded state](P064-register-call-exits.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/execution/register/exits.rs`
- `crates/wtime-runtime/src/continuation.rs`
- `crates/wtime-runtime/tests/performance/p065.rs`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

Register execution exposes or materializes all source state before GC/exception/continuation/host transitions; initial numeric-only eligibility remains, while outer reference-bearing frames stay traced.

## Implementation recipe

1. Add verifier assertions that every root-relevant source effect has an exit before it; reject hidden refs or incomplete local/operand maps.
2. Exercise numeric register callees under reference-bearing decoded/compact callers and verify outer locals/operands/published roots survive reentrant collection.
3. On exception/capture/suspend paths force exact decoded state before transfer and preserve cross-module nominal context, exactness, prompts and saved fuel.
4. Record optimized prefix and barrier execution counts; do not claim reference-heavy register support from fallback-only tests.

## Meaningful cases

- An outer sole-root reference survives a host collection after register execution.
- Captured/unwound frames retain original source PC and numeric values.
- Cross-module exact suspension and one-shot/fuel/byte-budget controls pass with an actual register prefix.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime --features register-executor,execution-reference p065_ -- --nocapture
python3 tools/wtime-bench/tier_check.py --tier register --selection integration --output target/performance/P065/conformance
```

## Measurement and decision

Selection: `integration`. Explicit execution setting: `register`. Decision rule: `gate`.

Validate current selected artifacts and all stated checks, retaining unselected/rejected optional work explicitly. A missing aspiration is reported with follow-ups; semantic, integrity and hard-budget failures remain blockers. Read [MEASUREMENT.md](../MEASUREMENT.md).

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. No commands in this planning file have been executed as implementation evidence.
