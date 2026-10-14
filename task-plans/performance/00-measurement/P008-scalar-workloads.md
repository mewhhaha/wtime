# P008 — Add scaling, branch and call benchmark families

Status: **done**

Outcome: **complete**

Kind: `foundation` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P003 — Define workload selections and engine capability preflight](P003-workload-schema.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `tools/wtime-bench/fixtures/scalar/*.wat`
- `tools/wtime-bench/oracles/scalar.py`
- `tools/wtime-bench/workloads.json`
- `tools/wtime-bench/tests/test_p008_scalar.py`

- `tools/wtime-bench/scalar_workloads.py`
- `tools/wtime-bench/scaling.py`
- `tools/wtime-bench/src/scalar_driver.rs`
- `tools/wtime-bench/src/protocol.rs`
- `tools/wtime-bench/compare.py`
- `tools/wtime-bench/tests/test_p008_driver.py`
- `tools/wtime-bench/tests/test_p008_scaling.py`
- `tools/wtime-bench/tests/test_p008_compare.py`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

Scalar cases use deterministic run(i32)->i32 exports and independent wrapping-integer oracles across 1/32/1024/10000 iterations; direct, indirect, call_ref and tail-call variants are separately capability-gated.

## Implementation recipe

1. Retain the original LCG/call cases and add accumulator/dependency-chain, unpredictable branch, switch/br_table, multivalue branch and direct/recursive/tail call kernels.
2. Add table mutation between timed batches, typed indirect/call_ref, null/mismatched-target correctness probes and a host-call control; time mutation/setup only in explicitly named stages.
3. Keep inputs runtime-provided and checksum all results so loop bodies cannot be replaced by the harness; add held-out constants/branch shapes before later optimizations.
4. Populate scalar selection/work units and capability matrix, validate every input with the independent oracle, then collect scaling baselines.

## Meaningful cases

- Every scaling input matches wrapping integer oracle.
- Changing an indirect target changes the next result and mismatched/null targets trap correctly.
- Deep tail recursion succeeds under the corresponding low logical-depth policy while ordinary recursion fails.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
python3 -m unittest discover -s tools/wtime-bench/tests -p 'test_p008*.py' -v
```

## Measurement and decision

Selection: `scalar`. Explicit execution setting: `decoded`. Decision rule: `integrity`.

This is a tooling/contract deliverable. Require correct nonzero checks and bounded/source-bound output; do not invent a runtime speedup requirement. If it changes production work, record cold/build/retention deltas under [MEASUREMENT.md](../MEASUREMENT.md).

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. Executed evidence is recorded below.

## Executed evidence

```json
{
  "artifacts": [
    "/workspace/wtime/target/performance/P008-scaling/report.json",
    "/workspace/wtime/target/performance/P008-tests-final.log",
    "/workspace/wtime/target/performance/P021-final/report.json"
  ],
  "commands": [
    {
      "command": "WTIME_BENCH_BINARY=tools/wtime-bench/profiling/target/release/wtime-bench python3 -m unittest discover -s tools/wtime-bench/tests -p test_p008*.py -v",
      "exit_code": 0,
      "passed": 21,
      "failed": 0,
      "skipped": 0
    },
    {
      "command": "python3 tools/wtime-bench/scaling.py --binary tools/wtime-bench/profiling/target/release/wtime-bench --encoder target/debug/wtime-conformance --probe-binary tools/wtime-bench/target/release/wtime-bench --samples 5 --output target/performance/P008-scaling",
      "exit_code": 0,
      "entries": 76,
      "observations": 1900,
      "calibrations": 76,
      "correctness_probes": 11
    },
    {
      "command": "cargo build --locked --release --manifest-path tools/wtime-bench/profiling/Cargo.toml",
      "exit_code": 0
    },
    {
      "command": "cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml",
      "exit_code": 0
    }
  ],
  "hashes": {
    "target/performance/P008-scaling/report.json": "c3ce4a95a9d867dc5da58e67f815a36669368d956262075591aa5d1c47282f16",
    "target/performance/P008-tests-final.log": "d9c8a6b370719e5434e15eae95494d44315e4da706f8748751ee8473c2e24b5b",
    "target/performance/P021-final/report.json": "0707d7f2765dc4b70868f043b65e00842e6a6e6769f793d555490a99cc07ba8a"
  },
  "binary_sha256": null,
  "settings": {
    "policy": "decoded",
    "fuel": false,
    "quantum": 1,
    "profile": "release",
    "instrumented": false,
    "samples_per_entry_stage": 5
  },
  "limitations": [
    "Scalar selection retains original call_overhead/integer_loop/branch_calls plus 76 new scaling/held-out entries across 13 families. Original controls are covered by the source-bound P021 predecessor comparison; P008 new scaling baseline records 1900 observations with independent wrapping oracles, not a paired speed promotion.",
    "Ordinary recursion timing inputs are capped at32, with correctness-only1024/10000 depth failures. Tail10000 succeeds under the same depth32 policy. Table mutation A/B/restoredA, null and mismatch probes execute outside timing; host callback setup is outside invocation timing but included in load_and_call.",
    "Wasmtime pinned build actual capability matrix supports 10 families; call_ref/table mutation have recorded missing proposal/build capabilities and host_tick requires a missing comparator setup driver. No timing ratios are claimed for these unsupported comparator paths.",
    "Typed guest API timing is explicitly unavailable: create_typed_host_func is a callback API. Dynamic guest calls remain Store::invoke.",
    "New paired scalar path verifies both sides driver/profile/typed result and shared fixture hashes, rejects incompatible legacy APIs and stale harness leaf source sets, and runs both-side correctness probes before timing. Seven focused comparison unit tests pass; actual new-family paired promotion is reserved for subsequent candidates.",
    "Oracle/driver tooling uses scalar_workloads.py and tests/ rather than an extra oracles/scalar.py wrapper. No runtime/Cargo dependency change or runtime speedup is part of this benchmark foundation."
  ]
}
```
