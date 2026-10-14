# P112 — Publish the selected performance results and next climb

Status: **todo**

Outcome: **pending**

Kind: `gate` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)

[P108 — Choose defaults, opt-in tiers and retired experiments](P108-select-performance-portfolio.md)

[P109 — Refresh full conformance and hardening for selected tiers](P109-selected-tier-conformance.md)

[P110 — Verify the final portfolio stays small and quick to prepare](P110-final-cold-and-build-budget.md)

[P111 — Add honest smoke, regression and reference-machine lanes](P111-performance-ci-lanes.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C5, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `docs/benchmarks/performance/README.md`
- `docs/architecture/performance-policy.md`
- `docs/architecture/release-manifest.json`
- `/workspace/tasks/performance/TASKS.json`
- `tools/wtime-bench/tests/test_p112_contract.py`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

A final reviewable report states measured warm/cold/build/size changes, selected policy/subsets, valid Wasmtime comparisons, retained negative experiments and outstanding tasks; it does not claim the original release is verified on missing native targets.

## Implementation recipe

1. Summarize current versus frozen baseline with raw artifact links, both run decisions, representative workloads, matched settings and break-even/deployment costs.
2. State which aspirations were reached and where; distinguish synthetic, held-out, unsupported comparator and unrun architecture cases.
3. Refresh current release source/evidence manifests without erasing historical reports or completing original T166/T169 without native AArch64 evidence.
4. Reconcile task status/outcome/evidence and record rejected/deferred experiments plus narrowly measured next steps; validate both task plans and final portfolio gates.

## Meaningful cases

- Every published speed/size/compile claim has current artifacts/settings and a valid comparison.
- Deferred/rejected research and missing native AArch64 evidence remain explicit.
- Plan statuses/links/DAG and release evidence identities match actual completed work.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
python3 -m unittest discover -s tools/wtime-bench/tests -p 'test_p112*.py' -v
python3 /workspace/tasks/performance/check_plan.py
python3 /workspace/tasks/check_plan.py
git diff --check
```

## Measurement and decision

Selection: `integration`. Explicit execution setting: `decoded`. Decision rule: `gate`.

Validate current selected artifacts and all stated checks, retaining unselected/rejected optional work explicitly. A missing aspiration is reported with follow-ups; semantic, integrity and hard-budget failures remain blockers. Read [MEASUREMENT.md](../MEASUREMENT.md).

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. No commands in this planning file have been executed as implementation evidence.
