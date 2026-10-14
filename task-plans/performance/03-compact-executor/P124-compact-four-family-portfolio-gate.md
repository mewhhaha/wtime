# P124 — Validate the complete Compact portfolio gate producer

Status: **done**

Outcome: **contract_only**

Kind: `foundation` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P015 — Add reproducible application-shaped held-out kernels](../00-measurement/P015-representative-kernels.md)

[P035 — Evaluate structural wins and choose the next bottleneck](../02-structural-wins/P035-structural-milestone.md)

[P045 — Verify and fuzz prepared compact programs](P045-compact-verifier-fuzz.md)

[P121 — Record complete Compact conformance and preparation coverage](P121-compact-full-conformance-counts.md)

[P122 — Prove Compact representative timing keeps the source-bound guest boundary](P122-compact-representative-protocol.md)

[P123 — Adapt supported typed workload routes to immutable P001 sources](P123-p001-typed-harness-adapter.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [MEASUREMENT.md](../MEASUREMENT.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md). Apply C1–C8 from [CONTRACTS.md](../CONTRACTS.md). Use Luna for exploration and mechanical tooling; root owns runtime semantics and shared integration.

- `docs/benchmarks/performance/P124-contract.json`
- `docs/benchmarks/performance/P124-result.json`
- `tools/wtime-bench/p046_gate.py`
- `tools/wtime-bench/tests/test_p124_portfolio.py`
- `tools/wtime-bench/bootstrap.py`
- `tools/wtime-bench/tests/test_p124_bootstrap.py`
- `docs/benchmarks/performance/P124-verification.json`
- `tools/verification/statistics.py`

P124 owns the P046 portfolio checker, contract and checker tests; parent retains P046 status, docs/benchmarks/performance/compact.md, runtime integration and final promotion decision.

## Input/output contract

A validated portfolio gate producer consumes source-bound full-family comparison and corpus/build records, checks all required rows and both paired runs, computes each unique data set once, and emits one final decision. P046 owns real measurement, final analysis and default selection; this foundation cannot claim speed or complete P046.

## Implementation recipe

1. Define the complete family/baseline matrix and checked source/model/stage identities. Preserve all full-selection native timing/oracle paths; P046 invokes real measurement once per required family/baseline, each containing two internal runs.
2. For each selection assert the report contains both internal runs, all selected workload-stage groups, 30 complete pairs per group per run, source stability and actual baseline/candidate identities. Require representative mode Compact with the source-bound run_after_setup contract.
3. Require the full current decoded structural baseline and candidate matrix. Include all P001 family routes proven supported by P123; otherwise preserve the concrete P001 compile/profile/API limitation and its impact without weakening any current-baseline family requirements. Do not claim unsupported rows measured.
4. Compose P121 full suite counter/preparation evidence and all absolute/relative build, dependency, retained-code, preparation/first-use and cold gates, preserving inconclusive and hard-failure states.
5. Provide one shared composite-statistics entry point invoked once by P046 after measurement. Compute each distinct raw data set once using the versioned exact-rank median bootstrap, retain paired intervals and uncertainty, weight primary-family workloads equally, then evaluate one final portfolio decision; tests cannot substitute for actual P046 timings.
6. Write an auditable receipt to target/performance/P046 with command/settings/hash/count inventories and a proposed default decision; leave actual promotion to the parent after inspecting evidence.

## Meaningful cases

- A missing family, workload, stage, either internal run, or one paired observation makes the portfolio incomplete; subset flags are rejected.
- A representative report with decoded candidate, non-run_after_setup timing, or instrumented binary is rejected.
- A missing/failed/zero-work P121 corpus report or hard build/dependency/cold gate cannot be overridden by a family speedup; shared statistics is invoked exactly once.
- Bootstrap median-rank probabilities match exhaustive resampling for even and odd sample counts; seeded draws remain stable, and primary-family aggregation gives equal weight to each workload.

## Verification

Activate Rust with `. /workspace/.setup/rust-env.sh`. Define the task-owned verification contract and incremental development groups before running checks. Preserve all helper sources/commands before execution and use Python -B.

```sh
python3 -B -m unittest discover -s tools/wtime-bench/tests -p 'test_p046*.py' -v
python3 -B -m unittest discover -s tools/wtime-bench/tests -p 'test_p124*.py' -v
python3 -B tools/verify.py plan --contract docs/benchmarks/performance/P124-verification.json
python3 /workspace/tasks/performance/check_plan.py
python3 /workspace/tasks/check_plan.py
```

## Measurement and decision

Foundation correctness only. P046 owns the full corpus and production performance decision; no timing or promotion claim follows from development receipts. Full conformance collection may be reused by P046 only while its consumed sources/profiles/corpus hashes remain current.

## Completion evidence

Execute at least three named producer validation tests in isolated temporary roots, including missing rows/runs, diagnostic or wrong-mode records, and hard failures surviving primary gains. Bind checker and model source hashes. P046 executes the producer over actual full-family measurements and owns the resulting performance decision.

Publish only after full task closure is reviewed; then run both plan validators. All listed commands are requirements, not claims that they have run.

## Executed evidence

The full P124 verification phase passed at `target/verification/runs/P124/verify/attempt-0001`: all nine matrix/bootstrap tests passed in 77.120 seconds and both portfolio orchestration tests passed in 0.019 seconds. There were no failed or skipped tests. The matrix covers 65,700 paired observations per baseline across scalar (85 workloads), memory (35), SIMD (114), and representative (9), with two 30-pair internal runs per workload-stage group.

The checker tests validate the portfolio producer and statistics contract using synthetic inputs. P124 is `contract_only`: it does not record P046 timings, change the default engine, or claim a performance improvement. P046 remains responsible for the real paired matrix and final promotion decision.

Both task graph validators passed after this entry was updated. Their archived helper, source snapshots, commands, logs, and results are in `target/performance/plan-validation/attempt-0009`.

The finalization evidence receipt is bound by SHA-256 `b90c03aa061fb6c15bcd0ac0579c4fee4dfce835d8bfbfc73d56a348fc47ee3f`. The verification phase receipt is `target/verification/runs/P124/verify/attempt-0001/receipt.json`.

The structured result is recorded in [P124-result.json](../../../wtime/docs/benchmarks/performance/P124-result.json).

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/P124/shared-finalization/attempt-0001/evidence.json": "b90c03aa061fb6c15bcd0ac0579c4fee4dfce835d8bfbfc73d56a348fc47ee3f"
  },
  "evidence": [
    {
      "bytes": 1165,
      "path": "/workspace/wtime/target/performance/P124/shared-finalization/attempt-0001/evidence.json",
      "sha256": "b90c03aa061fb6c15bcd0ac0579c4fee4dfce835d8bfbfc73d56a348fc47ee3f"
    }
  ],
  "outcome": "contract_only",
  "status": "done",
  "task": "P124"
}
```
