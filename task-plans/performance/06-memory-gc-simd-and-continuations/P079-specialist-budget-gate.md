# P079 — Check specialist retention, allocation and compilation costs

Status: **todo**

Outcome: **pending**

Kind: `gate` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)

[P068 — Copy only references into host and collection root snapshots](P068-reference-only-root-snapshots.md)

[P069 — Use conservative typed root maps for prepared frames](P069-typed-root-slot-maps.md)

[P071 — Reuse bounded GC mark and root work buffers](P071-collector-scratch-reuse.md)

[P072 — Maintain continuation bytes incrementally and transactionally](P072-continuation-byte-ledger.md)

[P073 — Share immutable continuation metadata and bound snapshot copies](P073-continuation-frame-sharing.md)

[P074 — Specialize portable SIMD bitwise and select operations](P074-portable-vector-bitops.md)

[P076 — Remove shared read allocation without weakening coordination](P076-shared-fixed-width-access.md)

[P077 — Improve wait registration lookup while retaining exact notifications](P077-wait-queue-indexing.md)

[P078 — Reduce shared bulk-copy overhead with coordinated fixed chunks](P078-shared-bulk-chunks.md)

Optional results to review (do not block the portable path):

[P070 — Try compact safe storage for packed numeric GC arrays](P070-packed-gc-array-storage.md)

[P075 — Try typed safe lane kernels that autovectorize](P075-portable-lane-specialization.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C5, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `docs/benchmarks/performance/specialist-budgets.json`
- `tools/wtime-bench/tests/test_p079_specialist_budgets.py`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

The gate binds selected specialist code to actual allocation-failure/retention/dependency/build evidence; compact GC storage, pooling and ledgers cannot hide larger retained memory or resource-policy changes.

## Implementation recipe

1. Run existing allocation-failure/hardening checks plus focused cache/pool/ledger failure cases with exact selected features.
2. Measure retained bytes after create/use/drop/reclaim and large-then-small workloads; compare physical accounting with owned capacities and full-scan ledger oracles.
3. Run default and optional build/size/RSS/dependency closure checks, keeping optional unsafe/native code outside this portable gate.
4. Reject any default specialist that exceeds limits or lacks cleanup/accounting evidence, recording negative decisions separately.

## Meaningful cases

- Allocation failure cannot publish partially packed objects or corrupt totals.
- Dropped/consumed resources release bounded retained ownership exactly once.
- Default build/dependency ceilings and source-current report identities are enforced.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
python3 -m unittest discover -s tools/wtime-bench/tests -p 'test_p079*.py' -v
bash tools/checks/ci.sh allocation
python3 tools/checks/build_budget.py --output target/performance/P079/build.json
python3 tools/checks/dependency_budget.py
```

## Measurement and decision

Selection: `integration`. Explicit execution setting: `decoded`. Decision rule: `gate`.

Validate current selected artifacts and all stated checks, retaining unselected/rejected optional work explicitly. A missing aspiration is reported with follow-ups; semantic, integrity and hard-budget failures remain blockers. Read [MEASUREMENT.md](../MEASUREMENT.md).

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. No commands in this planning file have been executed as implementation evidence.
