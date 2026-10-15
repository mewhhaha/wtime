# P070 — Try compact safe storage for packed numeric GC arrays

Status: **deferred**

Outcome: **not_selected**

Kind: `experiment` · Branch: `optional` · Model: **Sol 6.1 Low**

## Prerequisites

[P002 — Compare separately built baseline and candidate binaries](../00-measurement/P002-paired-driver.md)

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P011 — Measure reference, GC and exception paths](../00-measurement/P011-gc-exception-workloads.md)

[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)

[P016 — Add explicit executor policies and test integration hooks](../01-execution-contracts/P016-execution-policy.md)

[P034 — Replace repeated flattened type scans with immutable layout lookups](../02-structural-wins/P034-flat-gc-metadata.md)

[P068 — Copy only references into host and collection root snapshots](P068-reference-only-root-snapshots.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/gc.rs`
- `crates/wtime-runtime/src/gc_exec.rs`
- `crates/wtime-runtime/tests/performance/p070.rs`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

Nondefault packed-array storage uses initialized Vec<u8>/Vec<u16> for i8/i16 elements while reference/mixed objects retain Value storage; physical allocation bytes and source semantics stay accurately accounted.

## Implementation recipe

1. Add an explicit ObjectStorage variant and safe access/copy/fill adapters for packed arrays, starting only with i8 then i16.
2. Preserve get_s/get_u truncation/extension, array bounds/type/descriptors and full-range prechecks for copy/init; mixed/unpacked types use the old representation.
3. Update tracing to recognize no-reference packed payloads and compute real reserved capacities transactionally before object publication.
4. Measure allocate/traverse/copy and GC retained bytes; keep default off until graph/effect/budget checks pass.
5. Forward the explicit `packed-gc-arrays` setting through the harness/policy integrator and record actual optimized/fallback counts. Persistent research selectors are nondefault; reversible changes to an existing tier use before/after artifacts and are selected only by their gate.

## Meaningful cases

- Packed signed/unsigned gets and wrapping writes match existing semantics.
- Overlap/OOB array copies preserve order and no partial forbidden mutation.
- Allocation failure, descriptor roots and mixed/reference arrays retain accurate accounting/tracing.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime --features experimental-executors,execution-reference p070_ -- --nocapture
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml --features experimental-executors -j 4
python3 tools/wtime-bench/compare.py --baseline target/performance/P070-before --candidate tools/wtime-bench/target/release/wtime-bench --candidate-mode packed-gc-arrays --selection gc-exceptions --output target/performance/P070
python3 tools/wtime-bench/analyze.py --input target/performance/P070 --output target/performance/P070/analysis.json
python3 tools/checks/build_budget.py --output target/performance/P070/build.json
python3 tools/checks/build_budget.py --features wtime-runtime/experimental-executors --output target/performance/P070/optional-build.json
python3 tools/wtime-bench/gates.py --task P070 --comparison target/performance/P070 --build target/performance/P070/build.json --optional-build target/performance/P070/optional-build.json --output target/performance/P070/decision.json
```

## Measurement and decision

Selection: `gc-exceptions`. Explicit execution setting: `packed-gc-arrays`. Decision rule: `measured_promotion`.

Use the paired protocol and family/control/cold/build rules in [MEASUREMENT.md](../MEASUREMENT.md). Predeclare the targeted family and held-out controls before changing the implementation. Keep instrumentation off in production timings. Adopt only with supported gains in both runs; otherwise record rejected/inconclusive and remove default-path cost. A necessary immutable-interface change may be contract_only with its separate justification.

## Before editing

Run these commands before changing the candidate sources. The archive is immutable and captures the immediate predecessor. Retain P001's original archive for milestone totals. For native work, use the named portable control and verify its actual policy/quantum; if this archive already exists with different sources, choose a new recorded run-specific directory and update the comparison path rather than overwriting it.

```sh
python3 tools/wtime-bench/baseline.py --output target/performance/P070-before
```

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. Executed evidence is recorded below.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/final-optional-dispositions-0002/proof.json": "5a2943b1b7f0e4fedf44a2d4e96153937683fc97d45e079e55c0ef43a5e3b97f"
  },
  "evidence": [
    {
      "bytes": 38990,
      "path": "/workspace/wtime/target/performance/final-optional-dispositions-0002/proof.json",
      "sha256": "5a2943b1b7f0e4fedf44a2d4e96153937683fc97d45e079e55c0ef43a5e3b97f"
    }
  ],
  "implementation_claimed": false,
  "outcome": "not_selected",
  "reason": "Conditional research is not selected for the current portfolio; this record claims no implementation, test execution, or speed result.",
  "schema": "wtime-task-disposition-record-v1",
  "status": "deferred",
  "task": "P070",
  "tests_claimed": 0,
  "timing_evidence": false
}
```
