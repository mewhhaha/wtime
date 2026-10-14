# P044 — Materialize compact state for GC, exceptions and continuations

Status: **done**

Outcome: **inconclusive**

Kind: `candidate` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P002 — Compare separately built baseline and candidate binaries](../00-measurement/P002-paired-driver.md)

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)

[P016 — Add explicit executor policies and test integration hooks](../01-execution-contracts/P016-execution-policy.md)

[P022 — Add adversarial state and interaction tier fixtures](../01-execution-contracts/P022-state-differential-fixtures.md)

[P039 — Lower float/vector constants and barrier adapters](P039-compact-float-vector-adapters.md)

[P040 — Lower structured control to resolved jump and result plans](P040-compact-control-plans.md)

[P041 — Resolve direct guest call bindings once per instance](P041-instance-call-plans.md)

[P043 — Resolve memory bindings and specialize checked scalar accesses](P043-compact-memory-plans.md)

[P113 — Add integration-prefix benchmark and paired baseline support](P113-integration-prefix-benchmark-foundation.md)

[P114 — Verify bounded straight-line prefix and exit certificates](P114-prefix-exit-certificate.md)

[P115 — Integrate canonical handoff and reference-local admission](P115-canonical-prefix-handoff.md)

[P116 — Cover GC locals, host reentry and root lifetime](P116-prefix-gc-reentry-tests.md)

[P117 — Cover exception transfer from compact source prefixes](P117-prefix-eh-transfer-tests.md)

[P118 — Cover suspend and bind transfer from compact prefixes](P118-prefix-suspend-bind-tests.md)

[P119 — Cover resume and resume_throw after compact exits](P119-prefix-resume-tests.md)

[P120 — Cover switch transfer and compact resource boundaries](P120-prefix-switch-resource-tests.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C5, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/execution/compact/barriers.rs`
- `crates/wtime-runtime/src/execution.rs`
- `crates/wtime-runtime/src/gc_exec.rs`
- `crates/wtime-runtime/src/continuation.rs`
- `crates/wtime-runtime/src/execution/compact/prefix.rs`
- `crates/wtime-runtime/src/execution/compact/ir.rs`
- `crates/wtime-runtime/src/execution/compact/execute.rs`
- `crates/wtime-runtime/src/execution/compact/lower_scalar.rs`
- `crates/wtime-runtime/src/lib.rs`
- `docs/benchmarks/performance/P044-verification.json`
- `docs/benchmarks/performance/P044-contract.json`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

A bounded straight-line numeric/local/numeric-call prefix over canonical Values may execute compactly and exits one-way before the first control, memory or complex operation. The handoff recovers the original prepared (Instruction, pc), clears compact frame state at the already-charged boundary and executes the decoded suffix. Existing whole numeric/control functions retain their unchanged path. GC/EH/continuation helpers keep existing semantics after exact source-state/root materialization; no whole EH/prompt CFG certificate is introduced. P113–P120 are contract foundations; this parent retains the combined measured_promotion acceptance gate.

## Implementation recipe

1. Preserve the P044-before immediate-P043 Compact archive and the P001 historical baseline. Use the shared P044 verification contract to bind the predecessor, source tree, fixtures, harness, build tools and promotion inputs.
2. Keep the bounded straight-line integer/local/numeric-call prefix over canonical `Value` state and its permanent one-way decoded exit before the first control, memory or complex operation. Existing whole numeric/control Compact functions remain unchanged; no whole EH/prompt CFG certificate is added.
3. Preserve the original source instruction and `pc` at the already-charged handoff, clear compact frame state, and enter the decoded handler once. A branch-over-barrier executes the decoded suffix. Keep references visible to GC roots and preserve contexts, fuel, continuation bytes and cleanup.
4. Retain the P114–P120 correctness work, including P119's bounded shared `Store.resume_fuel` cleanup when resumed invocation termination abandons a segment during initial exception unwind or a catch-to-function-label returns with empty frames. Preserve consumed-handle ordering for the next independent invocation.
5. Run the full default runtime suite and the unfiltered `compact-executor,execution-reference` suite. The compact suite must execute all 21 named P114–P120 cases plus all eight named P022 interactions; require the contract's minimum totals (228 default, 334 compact) and no filtered tests.
6. Build the production benchmark binary, run all six independent integration probes, and pass both default and optional compact build budgets. Keep these checks in `verify` before any timed comparisons.
7. Run `measure` once with the contract's two runs and 30 pairs per group per run; require exactly 1800 pair events with source-stable fixtures and binaries. Keep instrumentation disabled and use the frozen immediate predecessor.
8. Run `analyze` once through the shared statistics producer, which computes paired statistics and the gate decision together, then records the relative build decision. Do not run a second statistics or gate calculation on the same comparison.
9. After reviewing the completion record and all four phase receipts, finalize the contract and publish its completion record. Keep this parent as the combined measured_promotion gate; P113–P120 are correctness foundations and make no independent speed claims.

## Meaningful cases

- Fuel interruption at each admitted prefix source operation matches decoded state and charges exactly once.
- A branch-over-barrier executes the decoded branch and suffix after one-way exit; no control operation is certified inside the prefix.
- A GC ref only in active compact locals survives host reentrant collection and nested invocation.
- try_table/throw, unhandled throw and nested unwind preserve original source pc, contexts and fuel.
- Suspend/bind/resume/resume_throw/switch preserve exact contexts, roots, continuation bytes, fuel and one-shot cleanup.
- Explicit Compact runs all eight P022 interaction cases with positive eligible prefix proof and visible fallback reporting.
- The unfiltered compact suite executes all 21 named P114–P120 tests plus all eight P022 tests; the unfiltered default runtime suite also passes its required total.
- P119 unhandled resume_throw and empty-frame function-label return clear saved resume fuel so a later independent invocation reaches consumed-handle validation without stale fuel preempting it.
- Production build and six integration probes pass; both default and optional compact build budgets pass before measurement.
- Paired measure records exactly 1800 source-stable pair events; the analyze phase computes paired statistics and the single gate decision once.

## Verification

Use the shared verifier and [P044-verification.json](../../../wtime/docs/benchmarks/performance/P044-verification.json). Development checks can use `full-runtime`; they are iterative checks and cannot replace full phase receipts. Run `freeze`, `verify`, `measure`, and `analyze` as separate phases in dependency order. Review their receipts and the concrete completion record before finalization and separate publication. The contract requires the full unfiltered suites, 21 named P114–P120 cases, eight P022 cases, production probes, both build budgets, 1800 pair events, and one statistics/gate computation.

```sh
python3 -B tools/verify.py plan --contract docs/benchmarks/performance/P044-verification.json
python3 -B tools/verify.py dev --contract docs/benchmarks/performance/P044-verification.json --group full-runtime --execute
python3 -B tools/verify.py run --contract docs/benchmarks/performance/P044-verification.json --phase freeze --execute
python3 -B tools/verify.py run --contract docs/benchmarks/performance/P044-verification.json --phase verify --execute
python3 -B tools/verify.py run --contract docs/benchmarks/performance/P044-verification.json --phase measure --execute
python3 -B tools/verify.py run --contract docs/benchmarks/performance/P044-verification.json --phase analyze --execute
python3 -B tools/verify.py finalize --contract docs/benchmarks/performance/P044-verification.json --output target/performance/P044/finalization/attempt-0001 --execute
python3 -B tools/verify.py publish --performance-root /workspace/tasks/performance --completion target/performance/P044/finalization/attempt-0001/completion.json --snapshot target/performance/TASK-status/P044-publication-0001 --execute
```

## Measurement and decision

Selection: `integration`. Explicit execution setting: `compact`. Decision rule: `measured_promotion`.

Use the paired protocol and family/control/cold/build rules in [MEASUREMENT.md](../MEASUREMENT.md). Predeclare the targeted family and held-out controls before changing the implementation. Keep instrumentation off in production timings. Adopt only with supported gains in both runs; otherwise record rejected/inconclusive and remove default-path cost. A necessary immutable-interface change may be contract_only with its separate justification.

## Before editing

Run these commands before changing the candidate sources. The archive is immutable and captures the immediate predecessor. Retain P001's original archive for milestone totals. For native work, use the named portable control and verify its actual policy/quantum; if this archive already exists with different sources, choose a new recorded run-specific directory and update the comparison path rather than overwriting it.

```sh
python3 tools/wtime-bench/baseline.py --mode compact --output target/performance/P044-before
```

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. Executed evidence is recorded below.

## Current readiness

P113–P120 are published as `done/contract_only`; P120's three switch/handle cases
and all eight explicit Compact P022 cases passed. The canonical prefix certificate
and one-way handoff are integrated. P119 also includes bounded shared saved-fuel
cleanup for resumed invocation termination across Decoded, Reference and Compact.
The P044-before predecessor archive contains 30 runtime source files; the current
runtime source tree contains 31, including the new prefix module.

P044 remains **todo / pending**. It has no speed decision yet. Its shared contract
requires the full default and compact runtime suites, all 21 P114–P120 cases and
eight P022 interactions, six production probes, both build budgets, 1800 paired
events, and one paired-statistics/gate analysis before finalization and publication.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/P044/shared-finalization/attempt-0001/evidence.json": "ddfcc67673c4cc621a7cc94e8303df69854de83072a945e7c86bc840ae79b15f"
  },
  "evidence": [
    {
      "bytes": 2342,
      "path": "/workspace/wtime/target/performance/P044/shared-finalization/attempt-0001/evidence.json",
      "sha256": "ddfcc67673c4cc621a7cc94e8303df69854de83072a945e7c86bc840ae79b15f"
    }
  ],
  "outcome": "inconclusive",
  "status": "done",
  "task": "P044"
}
```
