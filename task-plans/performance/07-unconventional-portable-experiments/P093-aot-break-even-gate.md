# P093 — Evaluate safe Rust AOT distribution and break-even

Status: **todo**

Outcome: **pending**

Kind: `gate` · Branch: `optional` · Model: **Sol 6.1 Low**

## Prerequisites

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)

[P015 — Add reproducible application-shaped held-out kernels](../00-measurement/P015-representative-kernels.md)

[P091 — Export a verified numeric program for an offline Rust AOT tool](P091-safe-aot-export-contract.md)

[P092 — Generate safe numeric Rust kernels from verified programs](P092-safe-rust-aot-generator.md)

Conditional branch: `P091.outcome == go`. If its decision is declined, record this task as deferred/not_selected; do not claim it implemented.

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `docs/benchmarks/performance/aot.md`
- `tools/wtime-bench/tests/test_p093_aot_gate.py`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

The AOT decision includes generation/rustc cost, shipped artifact bytes, registration/startup, execution, exact source policy and break-even; AOT cannot be called fast compilation because offline compilation was excluded.

## Implementation recipe

1. Benchmark generated pure numeric kernels/held-out scalar inputs as the primary family, and representative kernels as explicit eligible-or-fallback controls, against matched-policy portable and Wasmtime original-byte execution.
2. Report full generator+compiler cost, incremental regeneration, executable size/retained code, registration/validation and total 1/10/100/10000-call latency.
3. Run semantic/source-accounting checks and default dependency/build closure; verify the AOT tool/artifacts remain optional.
4. Record go/no_go for continued AOT work and explicit supported subset/distribution tradeoffs; do not ship unused generators by default.

## Meaningful cases

- Offline compiler time is always visible and finite crossover is supported by measured savings.
- Original bytes/policy and generated manifest identity match.
- No unsafe/reference support or unseen application-wide speed ratio is inferred from numeric kernels.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
python3 -m unittest discover -s tools/wtime-bench/tests -p 'test_p093*.py' -v
python3 tools/wtime-bench/tier_check.py --tier aot --selection representative --output target/performance/P093/conformance
```

## Measurement and decision

Selection: `representative`. Explicit execution setting: `aot`. Decision rule: `gate`.

Validate current selected artifacts and all stated checks, retaining unselected/rejected optional work explicitly. A missing aspiration is reported with follow-ups; semantic, integrity and hard-budget failures remain blockers. Read [MEASUREMENT.md](../MEASUREMENT.md).

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. No commands in this planning file have been executed as implementation evidence.
