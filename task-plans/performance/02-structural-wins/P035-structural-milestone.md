# P035 — Evaluate structural wins and choose the next bottleneck

Status: **done**

Outcome: **complete**

Kind: `gate` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P007 — Capture CPU profiles and rank actionable costs](../00-measurement/P007-profile-hotpaths.md)

[P008 — Add scaling, branch and call benchmark families](../00-measurement/P008-scalar-workloads.md)

[P009 — Add memory width, bulk and growth benchmark families](../00-measurement/P009-memory-workloads.md)

[P015 — Add reproducible application-shaped held-out kernels](../00-measurement/P015-representative-kernels.md)

[P017 — Make every Module clone share immutable metadata](../01-execution-contracts/P017-shared-module-payload.md)

[P023 — Fetch ordinary instructions without cloning their payload](P023-borrow-instructions.md)

[P024 — Move branch results and share label metadata without allocation](P024-in-place-branch-results.md)

[P025 — Remove per-load heap allocation for all scalar widths](P025-fixed-scalar-reads.md)

[P026 — Use fixed buffers for SIMD and lane memory access](P026-fixed-vector-reads.md)

[P027 — Use slice operations for unshared bulk memory and table copies](P027-unshared-bulk-slices.md)

[P028 — Execute common scalar operations in place on the operand stack](P028-in-place-numeric-stack.md)

[P029 — Stop cloning function signatures into every activation](P029-shared-function-metadata.md)

[P030 — Split checked public entry from validated guest call entry](P030-validated-guest-frame-entry.md)

[P031 — Reuse invocation scratch capacity safely under host reentry](P031-activation-scratch-pool.md)

[P032 — Move guest arguments and tail calls without split_off allocation](P032-call-argument-ranges.md)

[P033 — Cache immutable subtype answers with identity and work guards](P033-bounded-subtype-cache.md)

[P034 — Replace repeated flattened type scans with immutable layout lookups](P034-flat-gc-metadata.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

P035 owns the source-bound selection/checking glue and its focused tests:

- `docs/benchmarks/performance/structural.md`
- `tools/wtime-bench/structural_check.py`
- `tools/wtime-bench/tests/test_p035_milestone.py`
- `tools/wtime-bench/compare.py`
- `tools/wtime-bench/representative_compare.py`
- `tools/wtime-bench/analyze.py`
- `tools/wtime-bench/baseline.py`
- `tools/wtime-bench/profile.py`
- `tools/wtime-bench/profile_routes.py`
- `tools/wtime-bench/representative_workloads.py`
- `tools/wtime-bench/representative_scaling.py`
- `tools/wtime-bench/src/protocol.rs`
- `tools/wtime-bench/src/representative_driver.rs`
- `tools/wtime-bench/tests/test_p035_representative.py`
- `tools/wtime-bench/tests/test_p035_profile_routes.py`
- `tools/wtime-bench/tests/test_p035_tooling_routes.py`


The representative WAT/Wasmtime fixtures remain owned by P015. The runtime is
the restored P034-before/P033-restored state: exactly 20 runtime source files
must match both predecessor archives before P035 captures its own fresh
baseline. P035 may update benchmark tooling and evidence plans, but must not
change the runtime while establishing a current-baseline comparison.

One integrator owns any later runtime dispatch, Cargo features and
`crates/wtime-runtime/tests/performance.rs`; this milestone does not promote a
runtime patch.

## Input/output contract

A source-bound milestone report records the current decoded runtime's performance, profile coverage, correctness and build/dependency checks. The scalar, memory and representative portfolios are separate selections of exactly 85, 35 and 9 registry IDs. The immediately preceding restored P034 runtime is the baseline and candidate source; these paired runs measure repeatability and aspiration progress, not a speedup from a runtime change.

P001 compatibility is a required separate exact three-case comparison containing only `call_overhead`, `integer_loop`, and `branch_calls`. It must verify the frozen P001 archive, legacy adapter, and exact archived fixture identities. Never relabel the five-row `original-mvp` run or include current-only rows as P001 results. If the exact-three legacy route is not available, implement and validate that route before completing P035; an incomplete route cannot satisfy the milestone.

For MVP Wasmtime-supported inputs, use the same fixture bytes for an independent capability/integrity observation and verify output against the workload oracle. Keep Wasmtime timing separate from paired baseline/candidate measurements; this contract does not authorize cross-engine timing comparisons. Unsupported proposals are recorded with their concrete reason, never as zero-cost results.

## Implementation recipe

1. After P034 closes as rejected/inconclusive and its restore is freshly validated, integrate the P035 harness support and run focused tests. Verify all 20 runtime source hashes against both `target/performance/P034-before` and `target/performance/P033-before-corrected`; then capture a fresh decoded `target/performance/P035-before` archive after every support/tool change and capability preflight. Bind the current candidate binary, fixture bytes, selection registry, and sources to that archive.
2. Run one native `compare.py` invocation for each ready `scalar` (85 IDs), `memory` (35 IDs), and `representative` (9 IDs) selection, each with its native runs 1 and 2 and at least 30 paired observations per `(run, workload, stage)` group. Retain report, analysis, raw journal, exact command, seeds, pair counts, fixture/source/binary hashes, and process exits. Keep the three portfolio reports separate.
3. Run the exact three-case P001 legacy comparison separately, using only `call_overhead`, `integer_loop`, and `branch_calls`. Exact-three selection is a completion prerequisite; reject a five-row substitute and validate the adapter, archive and fixture hashes.
4. Collect diagnostic profiles separately from latency through `profile_routes.py`. Emit one explicit row for every ID across all three portfolios: counters are supported for the three scalar legacy compute controls, memory `memory_fill_scan`, and all nine representative IDs; mark the remaining 82 scalar and 34 memory rows unsupported with concrete reasons. Do not fill missing counters with zero or treat counter runs as timing observations.
5. For Wasmtime, recheck the exact fixture bytes and independent expected result on supported inputs. Label unsupported features and keep any Wasmtime sample outside baseline/candidate comparisons unless the measurement protocol is identical and explicitly validated.
6. Run decoded integration conformance, the default build-budget check, and dependency-budget check. Record optimized/fallback coverage, stable source inputs, exact counts, commands, exits and logs. Report the 10x compute aspiration per comparable legacy case only; do not invent an aggregate or infer causality from current-baseline repeats.

## Meaningful cases

- A rejected candidate is absent from the selected runtime and remains recorded with its exact decision and restored predecessor identity.
- Each current selection contains exactly 85 scalar, 35 memory, or 9 representative IDs, and each completed comparison has two native runs with at least 30 pairs per group.
- P001 coverage is exactly the three compatible legacy IDs; missing exact-selection support is a blocked comparison, never a passing five-row substitute.
- Wasmtime and diagnostic-profile support is reported only for observed capabilities and exact fixture/source identities; unsupported cases are explicit.
- Missing, stale, incomplete or mismatched performance, correctness, profile, conformance or budget artifacts fail the structural checker.

## Verification

Activate Rust as described in EXECUTION.md. The following commands are the planned implementation checks; run them only after P034 closure and P035 support integration. Any P035 test pattern must report actual discovered/passed/failed/skipped counts.

```sh
python3 -m unittest discover -s tools/wtime-bench/tests -p 'test_p035*.py' -v
python3 tools/wtime-bench/structural_check.py --input target/performance/P035/input.json --output target/performance/P035/decision.json
python3 tools/wtime-bench/tier_check.py --tier decoded --selection integration --output target/performance/P035/conformance
python3 tools/checks/build_budget.py --output target/performance/P035/build.json
python3 tools/checks/dependency_budget.py
```

The three paired current-selection commands and the separate exact-three P001 command are recorded in the final input index with their actual argv and exits. This plan does not provide an executable command for a route that has not been integrated.

## Measurement and decision

Selections: scalar (85), memory (35), representative (9), and the separate exact-three P001 legacy compatibility set. Explicit execution setting: `decoded`. Decision rule: structural gate.

Each current portfolio uses the immutable fresh P035-before archive of the restored P034 predecessor, with native paired runs 1 and 2 and at least 30 pairs per workload/stage/run. These runs establish a current baseline and per-case aspiration progress; they do not establish an optimization speedup. P001 is reported separately and only for the exact three compatible rows. Wasmtime integrity observations and diagnostic profiles remain separate from latency. Unsupported profile/native-counter coverage must retain its explicit reason. Read [MEASUREMENT.md](../MEASUREMENT.md).

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. Executed evidence is recorded below.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/docs/benchmarks/performance/structural.md": "f9236e0ed9e8080323135b9c56f78b63504d6ce0dce010ddafe97dcf506b9525",
    "/workspace/wtime/target/performance/P035-completion/attempt-20261012T011517300807Z/P035-final.md": "29fedd2370c63a59a0fc8b2aa241e4fba52abd119e3bc1118592f7011e0f98ca",
    "/workspace/wtime/target/performance/P035-completion/attempt-20261012T011517300807Z/p001-aspirations.json": "029adf81af35543f79d756441eb900f80adb4452de2197dadcd45acc9ba17cfe",
    "/workspace/wtime/target/performance/P035-completion/attempt-20261012T011517300807Z/stage-costs.json": "86929a5d8d1086533bb1f491850f9bd7d358fc013f143fd35061bfb68ac9be7b",
    "/workspace/wtime/target/performance/P035-completion/attempt-20261012T011517300807Z/summary.json": "e50943bf1069bbce6ddde80726fd699a4bffff6f5d58751fe9456878a166943f",
    "/workspace/wtime/target/performance/P035-completion/attempt-20261012T011517300807Z/task-completion-payload.json": "a50c5915f72725c65911224c4e85af9476b042cc93cc9f1b7d0050400837bebf",
    "/workspace/wtime/target/performance/P035-publication/root-publish.executed.py": "f8374bfd0177ae4dbe4968bf16201bf24c255be49ef15aec588ad9f4fc2a1c88",
    "/workspace/wtime/target/performance/P035-runner-attempts/attempt-0004/commands.json": "42d4153b87ffc5d5e5ae2ed9fcc57e3192d40b727c9f1535e7dd087c320d5c73",
    "/workspace/wtime/target/performance/P035/decision.json": "7b9f63f93abd84ca3e48a69bcf2f1273eb51ed225b8863590e73905c586a9644",
    "/workspace/wtime/target/performance/P035/input.json": "70c4f905225e0ca388344f0e7cfb89e186c37e5f5a5e732eed26b0666b7e3c45",
    "/workspace/wtime/target/performance/P035/wasmtime-capability/report.json": "eb5070d3771ca8b53da38b59ca178f67cee0bf7ae2400d0c2090a0feb92443e4"
  },
  "checker_status": "passed",
  "counts": {
    "current_groups": 1050,
    "current_pairs": 31500,
    "current_side_observations": 63000,
    "p001_groups": 30,
    "p001_pairs": 900
  },
  "evidence": [
    "/workspace/wtime/target/performance/P035-completion/attempt-20261012T011517300807Z/summary.json",
    "/workspace/wtime/target/performance/P035-completion/attempt-20261012T011517300807Z/stage-costs.json",
    "/workspace/wtime/target/performance/P035-completion/attempt-20261012T011517300807Z/p001-aspirations.json",
    "/workspace/wtime/target/performance/P035-completion/attempt-20261012T011517300807Z/P035-final.md",
    "/workspace/wtime/target/performance/P035-completion/attempt-20261012T011517300807Z/task-completion-payload.json",
    "/workspace/wtime/target/performance/P035/input.json",
    "/workspace/wtime/target/performance/P035/decision.json",
    "/workspace/wtime/docs/benchmarks/performance/structural.md",
    "/workspace/wtime/target/performance/P035-runner-attempts/attempt-0004/commands.json",
    "/workspace/wtime/target/performance/P035/wasmtime-capability/report.json",
    "/workspace/wtime/target/performance/P035-publication/root-publish.executed.py"
  ],
  "limitations": [
    "Current/current measurements establish costs and repeatability, not an optimization gain.",
    "Counters cover 13 cases; 116 diagnostic routes remain unsupported.",
    "Wasmtime capability checks are not matched timing measurements."
  ],
  "outcome": "complete",
  "profile_totals": {
    "selected": 129,
    "supported": 13,
    "unsupported": 116
  },
  "promoted_runtime_change": false,
  "schema": "wtime-task-completion-v1",
  "status": "done",
  "task": "P035"
}
```
