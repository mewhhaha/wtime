# P056 — Select fusions and keep the portable hot path small

Status: **todo**

Outcome: **pending**

Kind: `gate` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)

[P015 — Add reproducible application-shaped held-out kernels](../00-measurement/P015-representative-kernels.md)

[P046 — Decide whether compact execution earns default use](../03-compact-executor/P046-compact-milestone.md)

[P047 — Fuse a bounded set of local and constant arithmetic patterns](P047-local-arithmetic-fusions.md)

[P048 — Fuse integer compare and conditional control transfers](P048-compare-branch-fusions.md)

[P049 — Fuse pure address formation with a checked memory operation](P049-address-memory-fusions.md)

[P050 — Inline tiny direct numeric leaves with logical call accounting](P050-bounded-leaf-inlining.md)

[P053 — Separate metered and unmetered dispatch without weaker defaults](P053-accounting-mode-specialization.md)

[P054 — Fold pure integer work while preserving source events](P054-effect-preserving-peepholes.md)

Optional results to review (do not block the portable path):

[P051 — Try a two-value numeric operand cache](P051-operand-top-cache.md)

[P052 — Try safe unboxed storage for proven numeric activations](P052-numeric-slot-storage.md)

[P055 — Compare safe match, split dispatch and handler-table shapes](P055-dispatch-shape-experiment.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C5, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `docs/benchmarks/performance/specialization.md`
- `tools/wtime-bench/tests/test_p056_specialization_gate.py`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

A specialization portfolio records each variant decision, held-out results, exact source accounting evidence and aggregate cold/build/code-size cost; only individually justified compatible variants are enabled.

## Implementation recipe

1. Re-run integration tier checks with source-prefix fuel/cancel tests and the full predeclared scalar/memory/SIMD/representative controls.
2. Measure variants independently and as a final combined build to detect antagonistic code layout, duplicated specialization and compounded prep cost.
3. Retain negative experiments and remove their default-path/storage cost; cap the accepted fusion set and document supported versus adapter/fallback behavior.
4. Report portable aspiration progress and remaining profiles, with explicit follow-ups rather than a hidden growth in dependency or optimizer scope.

## Meaningful cases

- Individually winning variants that regress when combined are not all enabled.
- Eliminated/fused source events remain fully meter/coverage bound.
- All cold/build/retained metadata gates and held-out controls are present and source-current.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
python3 -m unittest discover -s tools/wtime-bench/tests -p 'test_p056*.py' -v
python3 tools/wtime-bench/tier_check.py --tier compact --selection integration --output target/performance/P056/conformance
```

## Measurement and decision

Selection: `integration`. Explicit execution setting: `compact`. Decision rule: `gate`.

Validate current selected artifacts and all stated checks, retaining unselected/rejected optional work explicitly. A missing aspiration is reported with follow-ups; semantic, integrity and hard-budget failures remain blockers. Read [MEASUREMENT.md](../MEASUREMENT.md).

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. No commands in this planning file have been executed as implementation evidence.
