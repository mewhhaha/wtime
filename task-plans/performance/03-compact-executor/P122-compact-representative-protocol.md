# P122 — Prove Compact representative timing keeps the source-bound guest boundary

Status: **done**

Outcome: **contract_only**

Kind: `foundation` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P015 — Add reproducible application-shaped held-out kernels](../00-measurement/P015-representative-kernels.md)

[P035 — Evaluate structural wins and choose the next bottleneck](../02-structural-wins/P035-structural-milestone.md)

[P045 — Verify and fuzz prepared compact programs](P045-compact-verifier-fuzz.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [MEASUREMENT.md](../MEASUREMENT.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md). Apply C1–C8 from [CONTRACTS.md](../CONTRACTS.md). Use Luna for exploration and mechanical tooling; root owns runtime semantics and shared integration.

- `tools/wtime-bench/src/representative_driver.rs`
- `tools/wtime-bench/representative_compare.py`
- `tools/wtime-bench/tests/test_p122_representative_protocol.py`

P122 owns representative_compare.py, the staged representative driver, its tests, the P122 probe producer and contract, and the admitted source-bound fixture receipt. Root retains runtime lib.rs, Cargo feature, dispatch, and shared performance-test registration.

- `docs/benchmarks/performance/P122-result.json`
- `docs/benchmarks/performance/P122-verification.json`
- `tools/verification/producers/p122.py`
- `target/performance/P122/fixtures/attempt-0001/**`

## Input/output contract

The existing representative staged command accepts explicit Compact candidate mode while retaining run_after_setup as the only measured stage, decoded baseline mode as appropriate, per-call setup/reset and oracle/readback outside the timed guest interval, and source-bound output/memory proofs. The production comparison binary is uninstrumented. This task does not change runtime semantics or use diagnostic counters in timing.

## Implementation recipe

1. Inspect the staged representative endpoint and freeze its actual source/binary/model identities in the P122 contract before editing.
2. Make mode and stage explicit through the representative comparator and each wire result; permit candidate Compact with a correctly identified decoded baseline, and preserve preparation, fresh Store/Instance, setup/reset, per-call checking, and output/memory oracle boundaries.
3. Prove that Compact candidate execution is selected by the actual protocol and that production instrumentation remains false; do not derive mode from feature availability.
4. Keep calibration bounded by the existing target/max-attempt/max-batch/size-product limits and validate that every requested guest call completed before accepting measured_guest_ns.

## Meaningful cases

- run_after_setup Compact observation binds the exact source and Wasm fixture hashes and confirms actual mode Compact.
- Compile/instantiate, reset/setup, output readback, and oracle checks remain outside each timed guest interval; only measured guest intervals contribute to ns_per_operation.
- Decoded/Compact mode mismatch, wrong stage, instrumentation-enabled response, omitted invocation, or invalid memory proof is rejected.

## Verification

Activate Rust with `. /workspace/.setup/rust-env.sh`. Define the task-owned verification contract and incremental development groups before running checks. Preserve all helper sources/commands before execution and use Python -B.

```sh
cargo test --locked --manifest-path tools/wtime-bench/Cargo.toml --features compact-executor p122_ -- --nocapture
python3 -B -m unittest discover -s tools/wtime-bench/tests -p 'test_p122*.py' -v
python3 -B tools/verify.py plan --contract docs/benchmarks/performance/P122-verification.json
```

## Measurement and decision

Foundation correctness only. P046 owns the full corpus and production performance decision; no timing or promotion claim follows from development receipts. Full conformance collection may be reused by P046 only while its consumed sources/profiles/corpus hashes remain current.

## Completion evidence

Full verification must execute p122_compact_staged_protocol_keeps_run_after_setup, p122_guest_interval_excludes_setup_reset_and_oracle, p122_rejects_wrong_mode_stage_or_instrumentation, and independent Python fixture/report cases. Retain exact route/stage/mode, actual calls, source/Wasm hashes, capability response, timer-boundary fields and binary hash. P122 does not make a speed claim and its tests are not P046 measurements.

Publish only after full task closure is reviewed; then run both plan validators. The listed protocol tests, production and diagnostic builds, and real fixture probe were executed; see [P122-result.json](../../../wtime/docs/benchmarks/performance/P122-result.json) and the immutable completion receipt below.

## Executed results

The final verification attempt passed all four named Python protocol tests. Its probe covered all nine representative rows over three real fixtures, with three production calls and one diagnostic call per row. Production binaries were uninstrumented; diagnostic binaries were instrumented. The fixture receipt binds each WAT/Wasm pair to source hashes and records the pinned encoder and validator identities. No timing evidence or performance claim was produced. Earlier failed verification attempts remain preserved in the verification store; attempt 0003 is authoritative. See [P122-result.json](../../../wtime/docs/benchmarks/performance/P122-result.json).

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/P122/shared-finalization/attempt-0004/evidence.json": "83e35bbac0154eea6a46e452c117df9c38b9041141e8b585d047522a782e5abb"
  },
  "evidence": [
    {
      "bytes": 1092,
      "path": "/workspace/wtime/target/performance/P122/shared-finalization/attempt-0004/evidence.json",
      "sha256": "83e35bbac0154eea6a46e452c117df9c38b9041141e8b585d047522a782e5abb"
    }
  ],
  "outcome": "contract_only",
  "status": "done",
  "task": "P122"
}
```
