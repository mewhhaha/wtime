# P067 — Evaluate register execution and adaptive break-even

Status: **todo**

Outcome: **pending**

Kind: `gate` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)

[P015 — Add reproducible application-shaped held-out kernels](../00-measurement/P015-representative-kernels.md)

[P056 — Select fusions and keep the portable hot path small](../04-specialization-and-fusion/P056-specialization-milestone.md)

[P057 — Define a small numeric register bytecode and eligibility rules](P057-register-ir-contract.md)

[P058 — Translate numeric stack code into direct slot operands](P058-stack-to-register-lowering.md)

[P060 — Implement safe parallel moves and register control transfers](P060-register-edge-moves.md)

[P061 — Execute checked integer register operations](P061-register-integer-execution.md)

[P062 — Add strict floating-point register operations](P062-register-float-execution.md)

[P063 — Add direct checked unshared memory register operations](P063-register-unshared-memory.md)

[P064 — Exit register execution before calls and resume decoded state](P064-register-call-exits.md)

[P065 — Verify roots and captured state around register exits](P065-register-root-and-capture-barriers.md)

[P066 — Compile register code lazily at bounded function thresholds](P066-function-boundary-tiering.md)

Optional results to review (do not block the portable path):

[P059 — Try conservative temporary-slot reuse without global SSA](P059-bounded-slot-reuse.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C5, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `docs/benchmarks/performance/register.md`
- `tools/wtime-bench/tests/test_p067_register_gate.py`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

A register-tier decision reports eligible optimized functions, all fallback reasons, semantic gates, warm/cold totals and adaptive code/build budgets; universal proposal support remains through portable fallback.

## Implementation recipe

1. Run pinned corpus/interaction/source-prefix checks through explicit and adaptive register policies, proving real optimized execution for supported cases.
2. Compare scalar/memory/strict-float/representative families to archived and best portable baselines; measure retained activation/code bytes and compile spikes.
3. Check preparation/cold/build/dependency controls and derive supported break-even thresholds rather than a fixed universal hot-count guess.
4. Report the 100x portable aspiration and whether register adds enough value over compact; reject/default-disable it if complexity wins no broad measurable benefit.

## Meaningful cases

- Fallback-only corpora do not masquerade as register-path coverage.
- Adaptive first-use totals include promotion and eviction costs.
- Any source accounting/root/feature regression blocks promotion despite faster scalar loops.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
python3 -m unittest discover -s tools/wtime-bench/tests -p 'test_p067*.py' -v
python3 tools/wtime-bench/tier_check.py --tier register --selection integration --output target/performance/P067/conformance
```

## Measurement and decision

Selection: `integration`. Explicit execution setting: `register`. Decision rule: `gate`.

Validate current selected artifacts and all stated checks, retaining unselected/rejected optional work explicitly. A missing aspiration is reported with follow-ups; semantic, integrity and hard-budget failures remain blockers. Read [MEASUREMENT.md](../MEASUREMENT.md).

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. No commands in this planning file have been executed as implementation evidence.
