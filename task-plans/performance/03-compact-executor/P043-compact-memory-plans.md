# P043 — Resolve memory bindings and specialize checked scalar accesses

Status: **done**

Outcome: **rejected**

Kind: `candidate` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P002 — Compare separately built baseline and candidate binaries](../00-measurement/P002-paired-driver.md)

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P009 — Add memory width, bulk and growth benchmark families](../00-measurement/P009-memory-workloads.md)

[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)

[P016 — Add explicit executor policies and test integration hooks](../01-execution-contracts/P016-execution-policy.md)

[P025 — Remove per-load heap allocation for all scalar widths](../02-structural-wins/P025-fixed-scalar-reads.md)

[P026 — Use fixed buffers for SIMD and lane memory access](../02-structural-wins/P026-fixed-vector-reads.md)

[P027 — Use slice operations for unshared bulk memory and table copies](../02-structural-wins/P027-unshared-bulk-slices.md)

[P037 — Map compact instructions to source events and control targets](P037-compact-source-maps.md)

[P038 — Lower constants, locals and common integer operations](P038-compact-scalar-lowering.md)

[P041 — Resolve direct guest call bindings once per instance](P041-instance-call-plans.md)

[P042 — Add guarded bounded indirect and call_ref caches](P042-indirect-call-inline-cache.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C5, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/execution/compact/memory.rs`
- `crates/wtime-runtime/src/lib.rs`
- `crates/wtime-runtime/tests/performance/p043.rs`

- `crates/wtime-runtime/src/execution.rs`
- `crates/wtime-runtime/src/linker.rs`
- `crates/wtime-runtime/src/perf.rs`
- `crates/wtime-runtime/src/execution/compact/ir.rs`
- `crates/wtime-runtime/src/execution/compact/mod.rs`
- `crates/wtime-runtime/src/execution/compact/lower_scalar.rs`
- `crates/wtime-runtime/src/execution/compact/execute.rs`
- `crates/wtime-runtime/src/execution/compact/calls.rs`
- `crates/wtime-runtime/tests/performance.rs`
- `crates/wtime-runtime/tests/performance/p043_private.rs`
- `crates/wtime-runtime/tests/fixtures/performance/p043`
- `tools/wtime-bench/p043_fixtures.py`
- `tools/wtime-bench/p043_coverage.py`
- `tools/wtime-bench/p043_workflow.py`
- `tools/wtime-bench/p043-diagnostics`
- `docs/benchmarks/performance/P043-contract.json`
- `docs/benchmarks/performance/P043-verification.json`
- `docs/benchmarks/performance/P043-build-decision.md`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

Compact memory ops carry width/extension/store kind/offset and local memory binding index; instance plans resolve checked Mem identities. Unshared access uses safe slices, shared access uses synchronized helpers.

## Implementation recipe

1. Lower scalar and SIMD memory width/offset/index type once, retaining full u64 offsets and fallback for unsupported compact widths.
2. Resolve memory handles per instance without retaining slices or base pointers; each access computes checked effective address and the complete current-length range.
3. Inline safe unshared buffer access where it reduces helper work, preserve shared/custom-page/memory64/multi-memory behavior and refresh length after growth/reentry.
4. Compare memory families, allocation diagnostics and preparation/code-size costs with full side-effect digests.
5. Forward the explicit `compact` setting through the harness/policy integrator and record actual optimized/fallback counts. Persistent research selectors are nondefault; reversible changes to an existing tier use before/after artifacts and are selected only by their gate.

## Meaningful cases

- Memory growth and host reentry cannot leave a stale length/backing view.
- Memory64 offset overflow and all scalar/vector boundary widths trap correctly.
- Aliased shared memories retain synchronized access and distinct-instance bindings.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
python3 -B tools/verify.py plan --contract docs/benchmarks/performance/P043-verification.json
python3 -B tools/verify.py dev --contract docs/benchmarks/performance/P043-verification.json --group focused --execute
python3 -B tools/verify.py dev --contract docs/benchmarks/performance/P043-verification.json --group diagnostic-focused --execute
python3 -B tools/verify.py close --contract docs/benchmarks/performance/P043-verification.json --output target/performance/P043/shared-finalization/attempt-0001 --execute
```

## Measurement and decision

Selection: `memory`. Explicit execution setting: `compact`. Decision rule: `measured_promotion`.

Use the paired protocol and family/control/cold/build rules in [MEASUREMENT.md](../MEASUREMENT.md). Predeclare the targeted family and held-out controls before changing the implementation. Keep instrumentation off in production timings. Adopt only with supported gains in both runs; otherwise record rejected/inconclusive and remove default-path cost. A necessary immutable-interface change may be contract_only with its separate justification.

## Before editing

Run these commands before changing the candidate sources. The archive is immutable and captures the immediate predecessor. Retain P001's original archive for milestone totals. For native work, use the named portable control and verify its actual policy/quantum; if this archive already exists with different sources, choose a new recorded run-specific directory and update the comparison path rather than overwriting it.

```sh
python3 -B tools/wtime-bench/baseline.py --mode compact --output target/performance/P043-before
```

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. Executed evidence is recorded below.

## Current predeclaration

The shared contract requires all ten named P043 cases, P038–P042 regression cases, the unfiltered default suite, a separate counter-enabled P043 run, isolated source-copy build budgets, and 35 diagnostic memory rows with complete image digests. Primary is exactly `memory_widths_predictable` and `memory_widths_shuffled`; all other 33 rows are protected. The fresh Compact predecessor and verified P042 build reports were frozen before runtime edits. Use the shared single statistics producer; finalization and publication remain separate. Preparatory drafts are retained and are not execution evidence.

## Result and retained experiment

The [result index](../../../wtime/docs/benchmarks/performance/P043-result.json)
links the authoritative four-phase proof, exact commands, source/binary/fixture
identities, test groups, coverage, measurements, and decision. Correctness
passed: 228 default tests, 313 Compact/reference tests, ten separately
counter-enabled P043 cases, 35 diagnostic rows, and 5,460 paired observations
across two runs of 91 groups with 30 pairs each. Shared statistics ran once.

The two primary warm workloads improved by approximately 31–39% against the
immediate P042 Compact predecessor. Promotion was **rejected**: cold/control
results and predeclared relative build-growth limits failed. Both absolute
build/dependency budgets passed. These cloud build measurements do not establish
which source change caused the timing differences.

The checked scalar-memory plans remain behind the explicit experimental
`compact-executor` feature. Their tested binding interface and measured warm
gains justify retaining them for further preparation/cold-cost experiments.
Default execution stays Decoded; SIMD memory, growth, and bulk operations retain
decoded fallback. The legacy `memory_fill_scan` timing protocol reports a
checksum; its complete image is checked separately in untimed diagnostics using
an export-section-only adapter. This comparison is against P042 Compact.
Earlier failures and interrupted attempts remain archived.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/P043/shared-finalization/attempt-0001/evidence.json": "4d32df830cf46c058a76b34ea7370eaced71134001dbaa89258ae9870ec8e699"
  },
  "evidence": [
    {
      "bytes": 2398,
      "path": "/workspace/wtime/target/performance/P043/shared-finalization/attempt-0001/evidence.json",
      "sha256": "4d32df830cf46c058a76b34ea7370eaced71134001dbaa89258ae9870ec8e699"
    }
  ],
  "outcome": "rejected",
  "status": "done",
  "task": "P043"
}
```
