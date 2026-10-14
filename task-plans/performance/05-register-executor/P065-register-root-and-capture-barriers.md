# P065 — Verify roots and captured state around register exits

Status: **done**

Outcome: **contract_only**

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

This contract-only close records correctness, root/capture semantics, source-bound integration, and default/optional build budgets. It makes no timing, speed, promotion, or broad portfolio claim; Register remains nondefault and reference/vector frames still take whole-function fallback.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/P065/shared-finalization/attempt-0004/evidence.json": "5bf93a86a64a5a6a902b1ddb2944bd95c59b4c527705254c1eadbaeafdee6118"
  },
  "evidence": [
    {
      "bytes": 1644,
      "path": "/workspace/wtime/target/performance/P065/shared-finalization/attempt-0004/evidence.json",
      "sha256": "5bf93a86a64a5a6a902b1ddb2944bd95c59b4c527705254c1eadbaeafdee6118"
    }
  ],
  "outcome": "contract_only",
  "status": "done",
  "task": "P065"
}
```

The full contract closed successfully with freeze attempt 0001, verify attempt 0004, and close attempt 0083; publication attempt 0001 recorded `done/contract_only`. Verification executed 713 checks: 300 Register semantic tests, 406 unfiltered combined Compact/Register/reference tests, one default-policy test, two conformance-forwarding checks, and four Python tier-policy tests. Every check passed. The integration tier report completed all 89 checks, including 68 fuel probes, six full-state runs, four conformance-mode runs, and three selected cases; it reported zero failures, skips, or unsupported cases. Reference-types conformance ran 105 directives in each mode and interactions ran 15 in each mode (240 directives total).

The new root-transition and source-barrier verifier checks confirm outer-root collection across Register exits and instance growth, capture of local 13 and operand 99 at source PC 5 with normal resume value 112, exact cross-module suspension and nominal-tag rejection without consuming the continuation, exception payload 20 across unwind, and whole-function fallback for reference/vector frames. Fuel, cancellation, one-shot resume, foreign references, frame limits, and byte budgets were exercised. A byte-budget check exposed that spare `Vec` capacity and the common-context header were underestimated; accounting now uses exact `continuation::type_ids` capacity. No semantic assertion was weakened. The first close attempts also corrected missing prerequisites in the Register mapping state validator and conformance feature/parser/replay setup before the final full verification passed.

The immutable predecessor is `target/performance/P065-before`, Register mode, commit `602d2c9`. Default clean and optional Register build sizes were 2.607499 and 2.974204 MB-equivalent; stripped sizes were 1,000,960 and 1,078,632 bytes. All hard budgets passed, and the default core retains zero external dependencies. Durable artifacts are `docs/benchmarks/performance/P065*`; export attempt `target/recovery/P065-export-0002` produced seven report copies plus hash records. The architectural writeup is `docs/architecture/register-root-barriers.md`. The gate establishes no timing evidence or performance promotion; subsequent performance decisions remain with P067.
