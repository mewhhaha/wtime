# P121 — Record complete Compact conformance and preparation coverage

Status: **done**

Outcome: **contract_only**

Kind: `foundation` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)

[P045 — Verify and fuzz prepared compact programs](P045-compact-verifier-fuzz.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [MEASUREMENT.md](../MEASUREMENT.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md). Apply C1–C8 from [CONTRACTS.md](../CONTRACTS.md). Use Luna for exploration and mechanical tooling; root owns runtime semantics and shared integration.

- `tools/wtime-conformance/Cargo.toml`
- `tools/wtime-conformance/src/adapter.rs`
- `tools/wtime-conformance/src/cli.rs`
- `tools/wtime-conformance/src/report.rs`
- `tools/wtime-conformance/src/script.rs`
- `tools/wtime-conformance/tests/p121_compact_counts.rs`
- `tools/wtime-conformance/tests/test_p121_report.py`
- `crates/wtime-runtime/src/lib.rs`
- `crates/wtime-runtime/tests/performance/p121.rs`
- `crates/wtime-runtime/tests/performance.rs`
- `tools/verification/producers/p121.py`
- `docs/benchmarks/performance/P121-verification.json`

Root owns the gated Module diagnostic and performance test registration; Luna handles mechanical conformance collection and tooling.

- `docs/benchmarks/performance/P121-result.json`
- `docs/benchmarks/performance/P121-verification.json`
- `tools/verification/producers/p121.py`

## Input/output contract

A development-only conformance build reports exact full-suite directive status, per-module prepared Compact accepted/declined/attempted-work/retained-byte totals, and optimized/fallback dispatched source-event totals. Every parent and worker Store is observed once; checked arithmetic errors fail. The report preserves failures, skips, unsupported directives, zero-work status, fixture/profile identity, instrumentation state, and immutable source/binary/corpus hashes. It consumes the parent-owned Module::compact_preparation_counts API and consumes a root-owned cfg-gated diagnostic method. That method has diagnostic-only gated cost, with no semantic or production-path cost. The shared performance-test registration is root-owned.

## Implementation recipe

1. Root adds only #[cfg(all(feature="compact-executor", feature="performance-counters"))] Module::compact_preparation_counts() -> Option<(usize,usize,usize,usize)> delegating to PreparedModule::counts. Conformance forwards performance-counters; the API has diagnostic-only gated cost and no semantic or production-path cost.
2. Accumulate successful compile/definition preparation tuples once; None denotes no published artifact and is not fabricated as zero attempted work. Checked usize aggregation overflow is an explicit failure.
3. Snapshot the parent Store after script execution and attach each child Store snapshot to its worker Report before the existing Wait/final join merge, exactly once.
4. For each Store, define total_dispatched_source_events=source_dispatches and optimized_compact_source_events=compact_steps. Compute decoded_fallback_source_events with checked_sub; reject underflow and any checked-add overflow. Do not add total and optimized, because compact execute records both events.
5. Emit explicit diagnostic/instrumented identity and all report fields without letting fallback hide failed, skipped, unsupported, or zero-work suites; verify counts against all four locked selections/profile commands in the draft README.

## Meaningful cases

- Compact operations increase both source_dispatches and compact_steps, and the derived fallback count is total minus optimized exactly once.
- Worker-thread optimized and fallback counters survive Wait and end-of-script joins without duplicate aggregation; checked sum/subtraction overflow fails the report.
- No prepared artifact does not count as accepted work, and failures/skips/unsupported directives or zero Compact optimized work prevent a passing full-suite record.

## Verification

Activate Rust with `. /workspace/.setup/rust-env.sh`. Define the task-owned verification contract and incremental development groups before running checks. Preserve all helper sources/commands before execution and use Python -B.

```sh
cargo test --locked -p wtime-runtime --features compact-executor,performance-counters p121_ -- --nocapture
cargo test --locked -p wtime-conformance --features compact-executor,performance-counters p121_ -- --nocapture
python3 -B -m unittest discover -s tools/wtime-conformance/tests -p 'test_p121*.py' -v
python3 -B tools/verify.py plan --contract docs/benchmarks/performance/P121-verification.json
```

## Measurement and decision

Foundation correctness only. P046 owns the full corpus and production performance decision; no timing or promotion claim follows from development receipts. Full conformance collection may be reused by P046 only while its consumed sources/profiles/corpus hashes remain current.

## Completion evidence

Full verification must execute p121_compact_counts_are_per_module_and_checked, p121_worker_snapshots_merge_once_with_checked_arithmetic, p121_fallback_is_derived_without_double_count_and_hidden_failures_fail, and the Python report-validation cases. Retain exact selected/executed/failed/skipped/unsupported counts, preparation tuple sums, worker count totals, decoded fallback and optimized event totals, coverage profiles, corpus lock/file hashes, source hash, instrumented binary hash, and explicit non-timing status. P121 evidence cannot claim P046 gate success.

Publish only after full task closure is reviewed; then run both plan validators. The listed checks were executed; see [P121-result.json](../../../wtime/docs/benchmarks/performance/P121-result.json) and the immutable completion receipt below.

## Executed results

The four locked selections (MVP, core3, phase3, interactions) passed with 145,760 of 145,760 selected directives executed; failures, skips, and unsupported directives were all zero. They dispatched 235,772,645 source events: 163,877,104 optimized Compact and 71,895,541 decoded fallback. The runs observed 29 worker Stores and 4,409 published prepared artifacts. Another 1,221 unpublished artifacts had unknown attempted work. Retained-budget accounting was 22,345,236 bytes using conservative capacity plus headers, not physical allocator bytes. This is correctness and diagnostic coverage evidence only.

The runtime diagnostic tests passed 3/3, conformance tests 4/4, and Python report tests 5/5. Plain and performance-counters-only checks, plus the release conformance build, passed.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/P121/shared-finalization/attempt-0002/evidence.json": "9b5ab71a529c02911e865c1ad588b68ba9fb00314f20f661c64420efc81a4204"
  },
  "evidence": [
    {
      "bytes": 1568,
      "path": "/workspace/wtime/target/performance/P121/shared-finalization/attempt-0002/evidence.json",
      "sha256": "9b5ab71a529c02911e865c1ad588b68ba9fb00314f20f661c64420efc81a4204"
    }
  ],
  "outcome": "contract_only",
  "status": "done",
  "task": "P121"
}
```
