# P110 — Verify the final portfolio stays small and quick to prepare

Status: **todo**

Outcome: **pending**

Kind: `gate` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)

[P108 — Choose defaults, opt-in tiers and retired experiments](P108-select-performance-portfolio.md)

[P109 — Refresh full conformance and hardening for selected tiers](P109-selected-tier-conformance.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `docs/benchmarks/performance/final-budgets.json`
- `tools/wtime-bench/tests/test_p110_final_budgets.py`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

The final source-bound budget report covers default clean/unchanged/leaf/release build, RSS, stripped size, dependency closure, preparation/one-shot latency and retained code; optional tiers have separate totals.

## Implementation recipe

1. Run the restored temporary-target build budget protocol and both CLI/runtime leaf rebuild measurements with fixed jobs/toolchain/reference environment.
2. Measure tiny/large/mixed cold module preparation, instantiation, first-use and retained metadata, including tier/cache worker startup when enabled.
3. Assert zero external/unsafe default closure and existing hard ceilings, reviewing 1.25 relative build/size/RSS or preparation growth against the actual frozen baseline.
4. Report optional compiler/bootstrap/mapping/AOT artifacts separately but include them in deployment/full-use totals where required; simplify/reject offenders rather than hiding their cost.

## Meaningful cases

- Files touched for build measurement are restored on success/failure.
- Default cold/dependency/build ceilings and all required report fields are enforced.
- Optional tier cost is neither mixed into default totals nor omitted from its own startup/deployment report.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
python3 -m unittest discover -s tools/wtime-bench/tests -p 'test_p110*.py' -v
python3 tools/checks/build_budget.py --output target/performance/P110/build.json
python3 tools/checks/dependency_budget.py
```

## Measurement and decision

Selection: `integration`. Explicit execution setting: `decoded`. Decision rule: `gate`.

Validate current selected artifacts and all stated checks, retaining unselected/rejected optional work explicitly. A missing aspiration is reported with follow-ups; semantic, integrity and hard-budget failures remain blockers. Read [MEASUREMENT.md](../MEASUREMENT.md).

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. No commands in this planning file have been executed as implementation evidence.
