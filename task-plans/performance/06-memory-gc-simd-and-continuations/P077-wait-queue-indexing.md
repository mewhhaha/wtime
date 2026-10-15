# P077 — Improve wait registration lookup while retaining exact notifications

Status: **done**

Outcome: **rejected**

Kind: `candidate` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P002 — Compare separately built baseline and candidate binaries](../00-measurement/P002-paired-driver.md)

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P013 — Measure shared memory, contention and wait latency](../00-measurement/P013-thread-workloads.md)

[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)

[P016 — Add explicit executor policies and test integration hooks](../01-execution-contracts/P016-execution-policy.md)

[P076 — Remove shared read allocation without weakening coordination](P076-shared-fixed-width-access.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C2, C3, C5, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/shared_memory.rs`
- `crates/wtime-runtime/tests/performance/p077.rs`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

A bounded per-address wait queue/index improves lookup/removal but preserves atomic registration, timeout/cancellation cleanup, counted notify and existing global SC coordination.

## Implementation recipe

0. Before runtime edits, create docs/benchmarks/performance/P077-selection.json and docs/benchmarks/performance/P077-verification.json; freeze the exact target small/large, held-out, protected and cold rows, and bind all semantic cases to successful full verification receipts.

1. The current backend already has BTreeMap<address, Vec<waiter>>. Profile per-address stale/linear notify/removal scans, then use a bounded per-address queue with stable registration IDs; do not add a redundant address map.
2. Give registrations stable IDs and remove them exactly once on notify/timeout/cancel; avoid scanning unrelated addresses and prune empty queues.
3. Keep registration-versus-value-check/notify ordering intact and max waiters/queue bytes bounded; aliases resolve to the same backing/address queue.
4. Measure actual wait latency and address fanout controls, retaining all current registration-race tests.
5. Record the changed path with focused assertions/diagnostic counts and exact source/binary identities; compare the pre-edit archive, and revert a rejected structural optimization rather than adding a permanent toggle solely for testing.

## Meaningful cases

- Notify(count) wakes exactly the allowed number under alias/race conditions.
- Timeout/cancel leaves no stale registration or empty unbounded queue.
- Multiple addresses do not cross-notify and infinite waits still poll cancellation.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime p077_ -- --nocapture
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml -j 4
python3 tools/checks/build_budget.py --output target/performance/P077/build.json
python3 -B tools/verify.py plan --contract docs/benchmarks/performance/P077-verification.json
python3 -B tools/verify.py close --contract docs/benchmarks/performance/P077-verification.json --output target/performance/P077/complete --execute
```

Declare the semantic-test and build commands above in the appropriate contract phases;
run `plan` and `close` externally to drive that contract. Bind full transitive
compilation/fixture inputs and require every semantic
case declared in the selection manifest. Its `measure` phase uses
`focused_compare.py --manifest docs/benchmarks/performance/P077-selection.json`
with `--frozen {freeze_dir}/selection.json`, `--semantic-receipt {verify_dir}/receipt.json`,
the immutable predecessor adapter, and the actual production executable/mode.
Its `analyze` phase calls `tools/verify.py statistics` once, supplying that
selection manifest as `--contract`, the paired report, and all build attestations.
Do not run an exhaustive comparison or a separate raw analyzer during this candidate gate.
See the [exact workflow](../../../wtime/docs/verification/fast-performance.md).

## Measurement and decision

Selection: `threads`. Explicit execution setting: `decoded`. Decision rule: `measured_promotion`.

Use the paired protocol and family/control/cold/build rules in [MEASUREMENT.md](../MEASUREMENT.md). Predeclare the targeted family and held-out controls before changing the implementation. Keep instrumentation off in production timings. Adopt only with supported gains in both runs; otherwise record rejected/inconclusive and remove default-path cost. A necessary immutable-interface change may be contract_only with its separate justification.

## Candidate gate schedule

Before implementation, freeze and verify the immediate-predecessor archive and a task-owned predeclaration manifest. Bind the manifest to exact registry workload IDs and input values, optimization shapes, target small/large rows, held-out rows, protected controls, stages, feature/runtime settings, and applicable cold products. Candidate promotion compares against this immediate predecessor only; P001 is reserved for exhaustive milestone totals. Measure the complete declared candidate set in two independent runs with at least 30 complete pairs per row and run, using 10,000 deterministic bootstrap resamples and uninstrumented production binaries. The applicable large-module preparation control and tiny `call_overhead` first-call/load-and-call cold controls are mandatory. Preserve every task-specific semantic, fuel/cancellation, trap/memory, fallback, and optimizer-bound case regardless of the focused timing selection. See [MEASUREMENT.md](../MEASUREMENT.md) for cold-product identity, sample reuse, thresholds, and milestone rules.

## Before editing

Run these commands before changing the candidate sources. The archive is immutable and captures the immediate predecessor. Retain P001's original archive for milestone totals. For native work, use the named portable control and verify its actual policy/quantum; if this archive already exists with different sources, choose a new recorded run-specific directory and update the comparison path rather than overwriting it.

```sh
python3 tools/wtime-bench/baseline.py --output target/performance/P077-before
python3 -B tools/wtime-bench/selection_manifest.py freeze docs/benchmarks/performance/P077-selection.json target/performance/P077-before/manifest.json target/performance/P077/selection-freeze.json
```

Create and validate the task-owned selection manifest before these commands.
The freezer requires the archived core to match the live predecessor, so run
it before the first runtime edit. The shared contract revalidates this receipt
in its `freeze` phase without replacing it after implementation.

## Completion evidence

P077 completed with outcome **rejected**. The full candidate close was `close-0120/attempt-0002`, using freeze attempt 0001, verification attempt 0002, measurement attempt 0002, and analysis attempt 0001. Its 900 pairs covered 15 groups over two independent runs (30 pairs per group per run), with seeds 77001 and 77002, CPU affinity fixed to CPU 1, workers 1–4, and 10,000 bootstrap resamples. Production timings were uninstrumented. The cohort timer began before waiter-gate release and ended after all waiter and drain results; it includes readiness polling and zero-result notify probes.

All 171 candidate test executions passed: 112 Rust executions (105 default-suite, 4 combined focused, 3 driver) and 59 Python executions (9 model, 9 bridge, 16 thread, 10 gate, 15 generic). The 13 frozen P077 semantic cases passed. The primary candidate/baseline ratios were 0.9761586921823108 (95% CI 0.9399016077964196–1.0431538744540014) and 1.0143695031737103 (0.9674887554731454–1.0458325008564127). The protected legacy-wait ratios were 1.6759117381058724 (1.4751563449132326–1.7604719191829266) and 1.652816725810792 (1.5604600406344655–1.7636165703070408). The results do not support the required 5% gain, so the candidate was rejected and reverted.

Focused diagnostics recorded 32 queue visits for 32 successful notifications versus 528 in the predecessor, and six comparisons for middle cleanup at the tested queue size. Deque removal still shifts later entries, so this is not an O(1) cleanup claim. The candidate enforced limits of 65,536 live waiters and 16 MiB for owned queue capacities, address values, and live waiter allocations; allocator and BTree internals are outside that byte estimate. The live waiter charge remained held through worker cleanup. These changes were reverted with the candidate.

The candidate clean build took 2.473659468 seconds, the CLI leaf build 0.130840664 seconds, and the default release build 5.044756799 seconds. The stripped binary was 1,008,264 bytes and peak RSS was 218,940 KiB. It introduced zero external dependencies. The separate unpaired build comparison is descriptive and does not support a causal speed claim.

A separate restoration close, `close-0121` / verification attempt 0003, passed 277 executions: 101 default, 173 combined, and 3 protocol, with zero failures or ignored tests. It verified the restored runtime crate source identity and a fresh default production build. This restoration verification does not replace or promote the rejected candidate result.

Evidence: [completion](../../../wtime/docs/benchmarks/performance/P077-completion.json), [candidate evidence](../../../wtime/docs/benchmarks/performance/P077-evidence.json), [freeze](../../../wtime/docs/benchmarks/performance/P077-freeze.json), [proof](../../../wtime/docs/benchmarks/performance/P077-proof.json), [statistics](../../../wtime/docs/benchmarks/performance/P077-statistics.json), [paired records](../../../wtime/docs/benchmarks/performance/P077-records.json.gz), [restoration](../../../wtime/docs/benchmarks/performance/P077-restoration.json), and [build comparison](../../../wtime/docs/benchmarks/performance/P077-build-comparison.json).

Executed evidence is recorded below.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/P077/shared-finalization/attempt-0002/evidence.json": "2eb4a89125609632ec81f37d3043a213e2bad745c74a0bd56c400a26148818f2"
  },
  "evidence": [
    {
      "bytes": 2164,
      "path": "/workspace/wtime/target/performance/P077/shared-finalization/attempt-0002/evidence.json",
      "sha256": "2eb4a89125609632ec81f37d3043a213e2bad745c74a0bd56c400a26148818f2"
    }
  ],
  "outcome": "rejected",
  "status": "done",
  "task": "P077"
}
```
