# P079 — Check specialist retention, allocation and compilation costs

Status: **done**

Outcome: **contract_only**

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
- `docs/benchmarks/performance/P079-verification.json`
- `tools/wtime-bench/tests/test_p079_specialist_budgets.py`
- `crates/wtime-runtime/src/gc.rs`
- `crates/wtime-runtime/src/continuation.rs`
- `crates/wtime-runtime/tests/performance/p079_gc.rs`
- `crates/wtime-runtime/tests/performance/p079_continuation.rs`
- `tools/verification/producers/p079.py`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

All P068, P069, P071, P072, P073, P074, P076, P077 and P078 runtime candidates ended rejected or inconclusive and were restored. There is no selected specialist pool, ledger, or packed-storage implementation to measure as an adopted optimization. This gate records resource and physical-retention evidence for the current portable runtime and any future explicitly selected specialist; it does not authorize a production behavior change.

The GC payload charge includes live object and field-storage charges; `continuation_bytes()` includes the owned continuation frame, label, operand, and related vector capacities and is checked against a physical-capacity scan. These are accounting charges, not total retained memory: `GcArena.slots`, `GcArena.free`, root-map storage, and the Store continuation registry can retain metadata capacity after payload cleanup. Report owned capacities separately through test-only probes; do not claim they represent total process RSS. Probe hooks and focused cases must be `cfg(test)` only. Keep semantic accounting, limits, and resource policy unchanged.

## Implementation recipe

1. Confirm the prior specialist outcomes and archives, then inspect the current GC and continuation ownership paths; do not assume a pool, ledger, packed store, or numeric capacity report exists.
2. Add `cfg(test)`-only probes and the three named GC/continuation cases below to report owned Vec slot/free/field/frame capacities across large-then-small reuse and full cleanup, separately from live payload charges; leave production behavior and resource policy unchanged.
3. Run the real host allocation-failure path in the memory constructor and its recovery check, plus the GC's explicit object- and byte-budget failure cases; do not describe those GC budget failures as allocator failures. Record default and `experimental-executors` build and dependency closure results with source-current inputs.
4. Have `tools/verification/producers/p079.py` bind the probes, allocator-failure result, default and optional build/dependency records, and budget fixture checks; retain only evidence-supported limits and make no promotion claim for unselected specialists.

## Meaningful cases

- `p079_gc_large_small_reuses_bounded_metadata_and_releases_payloads` checks large/small reuse, cleared payload charge, and retained slot/free capacity before arena drop.
- `p079_gc_budget_failures_publish_no_partial_object` checks atomic failure under explicit GC object and byte budgets and verifies recovery; this is not the host allocator-failure test.
- `p079_continuation_large_small_matches_owned_capacities_and_reclaims` matches continuation charges to owned vector capacities across repeated large/small captures, then checks payload cleanup and separately reports Store registry capacity.
- The real host allocator-failure test exercises the memory constructor and recovery path; default and `experimental-executors` build/dependency records are source-current.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
python3 -m unittest discover -s tools/wtime-bench/tests -p 'test_p079*.py' -v
bash tools/checks/ci.sh allocation
python3 tools/checks/build_budget.py --output target/performance/P079/build-default.json
python3 tools/checks/build_budget.py --features wtime-runtime/experimental-executors --output target/performance/P079/build-experimental-executors.json
python3 tools/checks/dependency_budget.py
```

## Measurement and decision

Selection: `integration`. Explicit execution setting: `decoded`. Decision rule: `gate`.

Validate current selected artifacts and all stated checks, retaining unselected/rejected optional work explicitly. A missing aspiration is reported with follow-ups; semantic, integrity and hard-budget failures remain blockers. Read [MEASUREMENT.md](../MEASUREMENT.md).

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. Executed evidence is recorded below.

## Final result

P079 completed as **contract_only** with 506 passing tests and zero failures or ignored cases. The default runtime unit suite passed 104 tests; the combined-feature unit suite passed 176; the combined performance integration suite passed 214; exception-root tests passed 2; and the two Python checker suites passed 10. These are focused integration and unit gates, not the final exhaustive conformance refresh.

The exact production sources match the archived predecessor after removing the two test-only module inclusions. All nine prior specialist candidates remain rejected or inconclusive. No production optimization or default policy changed. A real 512 MiB memory-constructor allocation failed safely under a 128 MiB address-space limit and then recovered; the worker executable and its hash are retained in the verification receipt. GC object/byte-budget failures were independently checked for atomicity and cleanup.

The large GC graph used 606,208 bytes of payload charge and 524,288 bytes of field capacity. Reclamation returned both to zero; 106,496 slot bytes and 8,192 free-list bytes remained owned for reuse until arena drop, with no growth during the smaller workload. Continuation captures alternated 16,816 and 1,296 bytes, matched independently enumerated owned capacities, and returned payload/state counts to zero after consumption and collection. Store-held registry capacity is reported separately. Object headers embedded in slots must not be counted twice. These are owned-capacity observations, not allocator padding or process RSS.

Default, register-only, and experimental-executor isolated builds passed their hard budgets and source-stability checks; the default core still has zero external dependencies. Optional Register cache lifecycle traces satisfy the existing entry, work and retained-byte limits. Build durations are observations, not paired speed claims. No guest timing or performance promotion is claimed.

The first full attempt passed its executed tests and builds but could not issue a receipt because two private tests were assigned to the integration target in the contract. The second attempt corrected their target and retained the worker binary explicitly, but existing integration-test prints interfered with named-outcome parsing under `--nocapture`. The final attempt uses `--show-output` for that target, preserving both complete named outcomes and diagnostic traces. All earlier attempts remain in the records; none had a runtime assertion failure.

Artifacts: [P079-completion.json](../../../wtime/docs/benchmarks/performance/P079-completion.json), [P079-evidence.json](../../../wtime/docs/benchmarks/performance/P079-evidence.json), [P079-freeze.json](../../../wtime/docs/benchmarks/performance/P079-freeze.json), [P079-proof.json](../../../wtime/docs/benchmarks/performance/P079-proof.json), [specialist-budgets.json](../../../wtime/docs/benchmarks/performance/specialist-budgets.json), [P079-records.json.gz](../../../wtime/docs/benchmarks/performance/P079-records.json.gz).

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/P079/shared-finalization/attempt-0003/evidence.json": "740ff68716350ab20ec8bbc5dcfbed8d787f3b2132350b927f6c7aa1869f8c16"
  },
  "evidence": [
    {
      "bytes": 1969,
      "path": "/workspace/wtime/target/performance/P079/shared-finalization/attempt-0003/evidence.json",
      "sha256": "740ff68716350ab20ec8bbc5dcfbed8d787f3b2132350b927f6c7aa1869f8c16"
    }
  ],
  "outcome": "contract_only",
  "status": "done",
  "task": "P079"
}
```
