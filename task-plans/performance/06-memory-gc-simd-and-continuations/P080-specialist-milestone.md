# P080 — Evaluate GC, SIMD, continuation and shared-memory progress

Status: **todo**

Outcome: **pending**

Kind: `gate` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)

[P067 — Evaluate register execution and adaptive break-even](../05-register-executor/P067-register-milestone.md)

[P068 — Copy only references into host and collection root snapshots](P068-reference-only-root-snapshots.md)

[P069 — Use conservative typed root maps for prepared frames](P069-typed-root-slot-maps.md)

[P071 — Reuse bounded GC mark and root work buffers](P071-collector-scratch-reuse.md)

[P072 — Maintain continuation bytes incrementally and transactionally](P072-continuation-byte-ledger.md)

[P073 — Share immutable continuation metadata and bound snapshot copies](P073-continuation-frame-sharing.md)

[P074 — Specialize portable SIMD bitwise and select operations](P074-portable-vector-bitops.md)

[P076 — Remove shared read allocation without weakening coordination](P076-shared-fixed-width-access.md)

[P077 — Improve wait registration lookup while retaining exact notifications](P077-wait-queue-indexing.md)

[P078 — Reduce shared bulk-copy overhead with coordinated fixed chunks](P078-shared-bulk-chunks.md)

[P079 — Check specialist retention, allocation and compilation costs](P079-specialist-budget-gate.md)

Optional results to review (do not block the portable path):

[P070 — Try compact safe storage for packed numeric GC arrays](P070-packed-gc-array-storage.md)

[P075 — Try typed safe lane kernels that autovectorize](P075-portable-lane-specialization.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C5, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `docs/benchmarks/performance/specialists.md`
- `tools/wtime-bench/tests/test_p080_specialist_gate.py`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

A source-bound report evaluates each specialist family against its own baseline and valid comparators, with full interactions, tail-latency evidence where collected and combined portable cost.

## Implementation recipe

1. Run gc-exceptions/continuations/simd/threads selections with actual specialized-path counts and wtime-only labels for unsupported comparator behavior.
2. Run pinned full corpus/interactions and cancellation/bulk/wait/root/continuation regressions in the final combined portable build.
3. Collect two paired uninstrumented runs, proper individual latency samples for pause/wait claims and P079 resource/build evidence.
4. State family-specific gains and remaining bottlenecks; do not infer a universal Wasmtime ratio from mixed unsupported proposals.

## Meaningful cases

- Every claimed specialist has nonzero optimized coverage and valid source hashes.
- No unsupported Wasmtime proposal has a ratio or synthetic success.
- Combined roots/SC/exactness/fuel behavior and build/retention gates remain correct.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
python3 -m unittest discover -s tools/wtime-bench/tests -p 'test_p080*.py' -v
python3 tools/wtime-bench/tier_check.py --tier decoded --selection integration --output target/performance/P080/conformance
```

## Measurement and decision

Selection: `integration`. Explicit execution setting: `decoded`. Decision rule: `gate`.

Validate current selected artifacts and all stated checks, retaining unselected/rejected optional work explicitly. A missing aspiration is reported with follow-ups; semantic, integrity and hard-budget failures remain blockers. Read [MEASUREMENT.md](../MEASUREMENT.md).

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. No commands in this planning file have been executed as implementation evidence.
