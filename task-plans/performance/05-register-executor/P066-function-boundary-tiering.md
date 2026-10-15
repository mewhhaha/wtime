# P066 — Compile register code lazily at bounded function thresholds

Status: **done**

Outcome: **rejected**

Kind: `candidate` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P002 — Compare separately built baseline and candidate binaries](../00-measurement/P002-paired-driver.md)

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)

[P015 — Add reproducible application-shaped held-out kernels](../00-measurement/P015-representative-kernels.md)

[P016 — Add explicit executor policies and test integration hooks](../01-execution-contracts/P016-execution-policy.md)

[P057 — Define a small numeric register bytecode and eligibility rules](P057-register-ir-contract.md)

[P061 — Execute checked integer register operations](P061-register-integer-execution.md)

[P062 — Add strict floating-point register operations](P062-register-float-execution.md)

[P063 — Add direct checked unshared memory register operations](P063-register-unshared-memory.md)

[P064 — Exit register execution before calls and resume decoded state](P064-register-call-exits.md)

[P065 — Verify roots and captured state around register exits](P065-register-root-and-capture-barriers.md)

[P132 — Extract transactional per-function Register preparation](P132-extract-transactional-register-preparation.md)

[P133 — Add a bounded lazy Register preparation cache](P133-bounded-lazy-register-cache.md)

[P134 — Integrate opt-in lazy Register entry](P134-opt-in-lazy-register-entry.md)

[P135 — Add a fresh-entry Register batch protocol](P135-fresh-entry-batch-protocol.md)

[P136 — Bind Register entry settings to source-bound paired collection](P136-register-entry-paired-collection.md)

[P137 — Analyze paired Register entry totals and phases](P137-entry-total-paired-analysis.md)

[P138 — Expose source-bound Register cache diagnostics](P138-register-cache-diagnostics.md)

[P139 — Collect side-specific Register entry settings](P139-side-specific-register-entry-settings.md)

[P140 — Freeze and attest the adaptive Register entry threshold](P140-register-entry-threshold.md)

[P141 — Add individual-entry Register timing and diagnostics](P141-register-entry-sequence-driver.md)

[P142 — Collect and analyze source-bound Register cold and churn sequences](P142-register-cold-churn-collection.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/execution/tiering.rs`
- `crates/wtime-runtime/src/lib.rs`
- `crates/wtime-runtime/tests/performance/p066.rs`
- `docs/benchmarks/performance/P066-selection.json`
- `docs/benchmarks/performance/P066-contract.json`
- `docs/benchmarks/performance/P066-verification.json`
- `tools/verification/producers/p066.py`
- `tools/verification/producers/p066_freeze.py`
- `tools/verification/producers/p066_protocol.py`
- `tools/verification/producers/p066_gate.py`
- `tools/wtime-bench/tests/test_p066_adapter.py`
- `tools/wtime-bench/tests/test_p066_representative.py`
- `tools/verification/tests/test_p066_gate.py`
- `tools/verification/producers/p066_driver_tests.py`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

An opt-in adaptive policy counts bounded function entries and promotes at an invocation/call boundary at the predeclared threshold 3. Eager threshold 0 is the baseline. No mid-loop OSR is attempted; failed/unsupported compilation keeps portable execution. P139 totals and P142 cold/churn evidence supplement but do not replace focused measurements or existing semantic and budget gates.

## Implementation recipe


1. Before runtime edits, freeze the immediate-predecessor P066-before archive, docs/benchmarks/performance/P066-selection.json, and P066-contract.json. Threshold 3 versus eager 0 is predeclared and is not tuned from results; retain protected cold controls and the frozen P135/P138 protocols.
2. Keep saturating hotness counters bounded per retained function, implement the predeclared threshold policy, and record tier transitions diagnostically.
3. Compile/publish immutable register artifacts transactionally outside canonical/cache locks; synchronize duplicate attempts and account optimizer work/code bytes.
4. Measure frozen 1/10/100/10000-call counter rows and the uniform66 cold/churn schedule at threshold 3 using two independent runs, 30 complete pairs per row/run, seeds 66001 and 66002, and 10,000 deterministic paired bootstrap resamples.
5. Run the shared four-phase P066 verification contract. Compute focused statistics once and combine the source-bound entry-total and cold/churn analyses in one final proof; preserve all semantic, focused-stage, control, build/dependency, provenance, and promotion gates.
6. Forward the explicit `register` setting through the harness/policy integrator and record actual optimized/fallback counts. Persistent research selectors are nondefault; reversible changes to an existing tier use before/after artifacts and are selected only by their gate.

## Meaningful cases

- Cold functions never compile and hot eligible functions transition at the declared boundary.
- Concurrent/shared-module promotion and failure cannot publish partial/foreign-context code.
- Eviction/churn obeys code budget and doesn't reset guest fuel or duplicate effects.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
python3 -B tools/verify.py plan --contract docs/benchmarks/performance/P066-verification.json
python3 -B tools/verify.py close --contract docs/benchmarks/performance/P066-verification.json --output target/performance/P066/shared-finalization --execute
```

Declare the semantic-test and build commands above in the appropriate contract phases;
run `plan` and `close` externally to drive that contract. Bind full transitive
compilation/fixture inputs and require every semantic
case declared in the selection manifest. Its `measure` phase runs the focused comparison plus the source-bound P139 entry-total and P142 individual-entry cold/churn collectors, each with two independent runs, 30 complete pairs per row/run, and seeds 66001 and 66002. Focused and entry-total statistics each use 10,000 deterministic paired bootstrap resamples. Cold/churn analysis groups six validated event classes by process and reports p50, p99, and max; it never pools calls as independent samples. The final `analyze` phase computes focused statistics once and combines them with the entry-total and cold/churn analyses in the P066 producer. The combined decision is emitted once at `/decision`; neither supplementary analysis makes a separate candidate decision.
See the [exact workflow](../../../wtime/docs/verification/fast-performance.md).

## Measurement and decision

Selection: `representative`. Explicit execution setting: `register`. Decision rule: `measured_promotion`.

Use the paired protocol and family/control/cold/build rules in [MEASUREMENT.md](../MEASUREMENT.md). Predeclare the targeted family and held-out controls before changing the implementation. Keep instrumentation off in production timings. Adopt only with supported gains in both runs; otherwise record rejected/inconclusive and remove default-path cost. A necessary immutable-interface change may be contract_only with its separate justification.

## Candidate gate schedule

Before implementation, freeze and verify the immediate-predecessor archive and a task-owned predeclaration manifest. Bind the manifest to exact registry workload IDs and input values, optimization shapes, target small/large rows, held-out rows, protected controls, stages, feature/runtime settings, and applicable cold products. Candidate promotion compares against this immediate predecessor only; P001 is reserved for exhaustive milestone totals. Measure the complete declared candidate set in two independent runs with at least 30 complete pairs per row and run, using 10,000 deterministic bootstrap resamples and uninstrumented production binaries. The applicable large-module preparation control and tiny `call_overhead` first-call/load-and-call cold controls are mandatory. Preserve every task-specific semantic, fuel/cancellation, trap/memory, fallback, and optimizer-bound case regardless of the focused timing selection. See [MEASUREMENT.md](../MEASUREMENT.md) for cold-product identity, sample reuse, thresholds, and milestone rules.

The entry-total lane uses the frozen counter fixture at call counts 1, 10, 100, and 10000, with eager threshold 0 on the baseline and adaptive threshold 3 on the candidate. Its raw-total row must have candidate/baseline 95% confidence upper bound at most 1.05. The cold/churn lane uses the P138 `uniform66.wasm` fixture at threshold 3, input 2147483647, and FIFO capacity 64; each of its six event classes must meet the same 1.05 upper-bound requirement for p99 and max. A delayed compilation spike can produce a rejected or inconclusive result. Keep the production timers uninstrumented and preserve all previous P132–P142 provenance, budget, semantic, focused-stage, protected-control, and candidate gates.

## Before editing

The root archived P066-before and externally froze the selection and acceptance contract before candidate sources changed. Retain P001's original archive for milestone totals. For native work, use the named portable control and verify its actual policy/quantum.

```sh
python3 -B tools/verify.py plan --contract docs/benchmarks/performance/P066-verification.json
```

The external predeclaration binds the archived predecessor, selected registry rows, explicit threshold policy, and cold/churn schedule. The shared contract revalidates those immutable inputs at close.

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. The JSON selection and contracts are protocol inputs; no commands in this planning file are implementation evidence.

P066 completed with outcome **rejected**. Final close `attempt-0004` passed under four source-current phases (freeze `attempt-0002`, verify `attempt-0003`, measure `attempt-0002`, analyze `attempt-0002`); completion evidence is [P066-evidence.json](../../../wtime/target/performance/P066/shared-finalization/attempt-0004/evidence.json), and the exported decision is [P066-proof.json](../../../wtime/docs/benchmarks/performance/P066-proof.json). The final selected groups passed 357 Rust tests and 74 Python tests with no failures or ignored tests. Verification included 322 full Register runtime tests, 3 compact Register cases, 2 default P016 controls, 18 diagnostic and 12 production harness tests, plus the 56 policy and 18 analyzer tests.

The run measured 720 focused pairs, 240 entry-total pairs (1,213,320 guest calls), and 60 cold/churn pairs (32,160 calls), using the frozen threshold 3 against eager threshold 0. Focused results were adopted (ratios 0.86473 and 0.88091); the combined gate rejected the candidate because cold/compile/recompile latency spikes exceeded limits, reaching roughly 15x. Threshold 3 remains opt-in and defaults are unchanged. P138 default/register build budgets were revalidated and reused (2.5773608s/1,040,104 leaf bytes and 2.869403s/1,459,040 leaf bytes); no P066 runtime core source changed and external dependency count remains zero.

Statistics were computed once in the first analysis attempt. The final analysis revalidated those retained reports without retiming or rebuilding. Earlier failed/superseded attempts are retained in [P066-records.json.gz](../../../wtime/docs/benchmarks/performance/P066-records.json.gz); the exported proof, freeze, focused statistics, entry-total analysis, and sequence analysis are preserved alongside it.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/P066/shared-finalization/attempt-0004/evidence.json": "d5a337a5ad6fac7868ae7fda541c4654c13b61a9e92ddb94cf3202ec6869040e"
  },
  "evidence": [
    {
      "bytes": 2493,
      "path": "/workspace/wtime/target/performance/P066/shared-finalization/attempt-0004/evidence.json",
      "sha256": "d5a337a5ad6fac7868ae7fda541c4654c13b61a9e92ddb94cf3202ec6869040e"
    }
  ],
  "outcome": "rejected",
  "status": "done",
  "task": "P066"
}
```
