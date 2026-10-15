# P071 — Reuse bounded GC mark and root work buffers

Status: **done**

Outcome: **rejected**

Kind: `candidate` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P002 — Compare separately built baseline and candidate binaries](../00-measurement/P002-paired-driver.md)

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P011 — Measure reference, GC and exception paths](../00-measurement/P011-gc-exception-workloads.md)

[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)

[P016 — Add explicit executor policies and test integration hooks](../01-execution-contracts/P016-execution-policy.md)

[P068 — Copy only references into host and collection root snapshots](P068-reference-only-root-snapshots.md)

[P069 — Use conservative typed root maps for prepared frames](P069-typed-root-slot-maps.md)

Optional results to review (do not block the portable path):

[P070 — Try compact safe storage for packed numeric GC arrays](P070-packed-gc-array-storage.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C2, C4, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/gc.rs`
- `crates/wtime-runtime/src/lib.rs`
- `crates/wtime-runtime/tests/performance/p071.rs`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

The nonmoving collector reuses bounded scratch for marks/worklists/root input, preserving mark-on-enqueue, max_gc_mark_work and transactional failure cleanup.

## Implementation recipe

0. Before runtime edits, create docs/benchmarks/performance/P071-selection.json and docs/benchmarks/performance/P071-verification.json; freeze the exact target small/large, held-out, protected and cold rows, and bind all semantic cases to successful full verification receipts.

1. Identify current transient allocation sites, move scratch to collector-owned inactive buffers and clear logical state on every collection result.
2. Keep work limits and mark-on-enqueue semantics unchanged; bound retained capacity and trim oversized buffers after declared thresholds.
3. Reject nested active use if collection can reenter, or prove the current synchronous collector cannot call host code; never reuse scratch still borrowed by a live collection.
4. Measure repeated small/large graph collections and pause distributions from individual samples plus retained bytes.
5. Record the changed path with focused assertions/diagnostic counts and exact source/binary identities; compare the pre-edit archive, and revert a rejected structural optimization rather than adding a permanent toggle solely for testing.

## Meaningful cases

- Cycle/diamond/shared-child graphs visit nodes within the same work budget.
- Failed limited collection restores scratch/marks so the next collection succeeds correctly.
- Large then small collections respect retention policy and reference roots/live counts match.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime p071_ -- --nocapture
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml -j 4
python3 tools/checks/build_budget.py --output target/performance/P071/build.json
python3 -B tools/verify.py plan --contract docs/benchmarks/performance/P071-verification.json
python3 -B tools/verify.py close --contract docs/benchmarks/performance/P071-verification.json --output target/performance/P071/complete --execute
```

Declare the semantic-test and build commands above in the appropriate contract phases;
run `plan` and `close` externally to drive that contract. Bind full transitive
compilation/fixture inputs and require every semantic
case declared in the selection manifest. Its `measure` phase uses
`focused_compare.py --manifest docs/benchmarks/performance/P071-selection.json`
with `--frozen {freeze_dir}/selection.json`, `--semantic-receipt {verify_dir}/receipt.json`,
the immutable predecessor adapter, and the actual production executable/mode.
Its `analyze` phase calls `tools/verify.py statistics` once, supplying that
selection manifest as `--contract`, the paired report, and all build attestations.
Do not run an exhaustive comparison or a separate raw analyzer during this candidate gate.
See the [exact workflow](../../../wtime/docs/verification/fast-performance.md).

## Measurement and decision

Selection: `gc-exceptions`. Explicit execution setting: `decoded`. Decision rule: `measured_promotion`.

Use the paired protocol and family/control/cold/build rules in [MEASUREMENT.md](../MEASUREMENT.md). Predeclare the targeted family and held-out controls before changing the implementation. Keep instrumentation off in production timings. Adopt only with supported gains in both runs; otherwise record rejected/inconclusive and remove default-path cost. A necessary immutable-interface change may be contract_only with its separate justification.

## Candidate gate schedule

Before implementation, freeze and verify the immediate-predecessor archive and a task-owned predeclaration manifest. Bind the manifest to exact registry workload IDs and input values, optimization shapes, target small/large rows, held-out rows, protected controls, stages, feature/runtime settings, and applicable cold products. Candidate promotion compares against this immediate predecessor only; P001 is reserved for exhaustive milestone totals. Measure the complete declared candidate set in two independent runs with at least 30 complete pairs per row and run, using 10,000 deterministic bootstrap resamples and uninstrumented production binaries. The applicable large-module preparation control and tiny `call_overhead` first-call/load-and-call cold controls are mandatory. Preserve every task-specific semantic, fuel/cancellation, trap/memory, fallback, and optimizer-bound case regardless of the focused timing selection. See [MEASUREMENT.md](../MEASUREMENT.md) for cold-product identity, sample reuse, thresholds, and milestone rules.

## Before editing

Run these commands before changing the candidate sources. The archive is immutable and captures the immediate predecessor. Retain P001's original archive for milestone totals. For native work, use the named portable control and verify its actual policy/quantum; if this archive already exists with different sources, choose a new recorded run-specific directory and update the comparison path rather than overwriting it.

Earlier P071 predecessor archives and their attempt history remain retained. The final candidate comparison and freeze use the immutable `target/performance/P071-before-0002` archive.

```sh
python3 tools/wtime-bench/baseline.py --output target/performance/P071-before-0002
python3 -B tools/wtime-bench/selection_manifest.py freeze docs/benchmarks/performance/P071-selection.json target/performance/P071-before-0002/manifest.json target/performance/P071/selection-freeze.json
```

Create and validate the task-owned selection manifest before these commands.
The freezer requires the archived core to match the live predecessor, so run
it before the first runtime edit. The shared contract revalidates this receipt
in its `freeze` phase without replacing it after implementation.

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. Executed evidence is recorded below.

## Final result

P071 completed with a **rejected** performance decision. The full candidate verification passed 434 tests (369 Rust, including 83 validator cases, and 65 Python). The uninstrumented decoded measurement contains 1,320 paired observations across 22 groups, two 30-pair runs with seeds 71001 and 71002 on CPU 1. The primary estimates were 0.9974012074 [0.9785465393, 1.0082817435] and 0.9853042066 [0.9638408014, 1.0022414333], so the declared 5% improvement was not supported; host reentry showed a supported regression.

The candidate tested three bounded scratch buffers capped at 4,096 slots, trimmed capacity above `max(64, 4 × used)`, and preserved exact mark-work accounting. The focused cases covered each cap, reuse, and error recovery. The runtime candidate changes were fully reverted, with 357 restoration tests passing and all 176 archived core/build inputs matching. The retained foundation covers nine repeated-collection rows (wide graphs with 32 and 1,024 distinct leaves and a 256-node chain), 45 guest/host collection-stage checks using the same Store/global root, and cleanup of all objects. These observations include guest plus host collection work and are not isolated GC-only timings.

The predecessor archive used for the final candidate comparison is `target/performance/P071-before-0002`; earlier archive attempts and their history remain preserved. The rejected runtime changes are not retained in the current source tree. Durable completion, evidence, proof, statistics, freeze, records, restoration, and build-comparison artifacts are linked from TASKS.json.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/P071/shared-finalization/attempt-0001/evidence.json": "7ee6abe588b238388c0e3013a3220472fa6ce2fa09302371754a35a0ff9cc5d8"
  },
  "evidence": [
    {
      "bytes": 2164,
      "path": "/workspace/wtime/target/performance/P071/shared-finalization/attempt-0001/evidence.json",
      "sha256": "7ee6abe588b238388c0e3013a3220472fa6ce2fa09302371754a35a0ff9cc5d8"
    }
  ],
  "outcome": "rejected",
  "status": "done",
  "task": "P071"
}
```
