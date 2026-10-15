# P064 — Exit register execution before calls and resume decoded state

Status: **done**

Outcome: **rejected**

Kind: `candidate` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P002 — Compare separately built baseline and candidate binaries](../00-measurement/P002-paired-driver.md)

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P008 — Add scaling, branch and call benchmark families](../00-measurement/P008-scalar-workloads.md)

[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)

[P016 — Add explicit executor policies and test integration hooks](../01-execution-contracts/P016-execution-policy.md)

[P018 — Define source cursors and materialized guest state](../01-execution-contracts/P018-source-cursor.md)

[P022 — Add adversarial state and interaction tier fixtures](../01-execution-contracts/P022-state-differential-fixtures.md)

[P041 — Resolve direct guest call bindings once per instance](../03-compact-executor/P041-instance-call-plans.md)

[P061 — Execute checked integer register operations](P061-register-integer-execution.md)

[P063 — Add direct checked unshared memory register operations](P063-register-unshared-memory.md)

[P131 — Prepare bounded Register prefixes for decoded handoff](P131-bounded-register-prefix.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/execution/register/exits.rs`
- `crates/wtime-runtime/src/execution.rs`
- `crates/wtime-runtime/tests/performance/p064.rs`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

A register function may execute a numeric prefix then hand off its exact active invocation to decoded execution before a call/barrier. The handoff is one-way initially and never uses Store::invoke to restart/reset fuel.

## Implementation recipe

0. Before runtime edits, create docs/benchmarks/performance/P064-selection.json and docs/benchmarks/performance/P064-verification.json; freeze the exact target small/large, held-out, protected and cold rows, and bind all semantic cases to successful full verification receipts.

1. Add ExitToDecoded carrying the next source PC and full materialization map; extend eligibility only for prefixes whose pending barrier state is representable.
2. Restore locals/operands/logical labels/frame base/current fuel into the existing activation loop, preserving outer frames and prompt state.
3. Execute the pending direct/indirect/host/tail call once through the normal semantic path; retain public/dynamic checks and root snapshots.
4. Compare call-after-numeric-prefix and callback/reentry fixtures, recording register work and exact handoff counts.
5. Forward the explicit `register` setting through the harness/policy integrator and record actual optimized/fallback counts. Persistent research selectors are nondefault; reversible changes to an existing tier use before/after artifacts and are selected only by their gate.

## Meaningful cases

- A store-before-call is not replayed and the call occurs exactly once.
- Host reentry sees complete outer/inner state and remaining fuel.
- Tail/indirect/type/depth errors arise at the same source point after handoff.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime --features register-executor,execution-reference p064_ -- --nocapture
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml --features register-executor -j 4
python3 tools/checks/build_budget.py --output target/performance/P064/build.json
python3 tools/checks/build_budget.py --features wtime-runtime/register-executor --output target/performance/P064/optional-build.json
python3 -B tools/verify.py plan --contract docs/benchmarks/performance/P064-verification.json
python3 -B tools/verify.py close --contract docs/benchmarks/performance/P064-verification.json --output target/performance/P064/complete --execute
```

Declare the semantic-test and build commands above in the appropriate contract phases;
run `plan` and `close` externally to drive that contract. Bind full transitive
compilation/fixture inputs and require every semantic
case declared in the selection manifest. Its `measure` phase uses
`focused_compare.py --manifest docs/benchmarks/performance/P064-selection.json`
with `--frozen {freeze_dir}/selection.json`, `--semantic-receipt {verify_dir}/receipt.json`,
the immutable predecessor adapter, and the actual production executable/mode.
Its `analyze` phase calls `tools/verify.py statistics` once, supplying that
selection manifest as `--contract`, the paired report, and all build attestations.
Do not run an exhaustive comparison or a separate raw analyzer during this candidate gate.
See the [exact workflow](../../../wtime/docs/verification/fast-performance.md).

## Measurement and decision

Selection: `scalar`. Explicit execution setting: `register`. Decision rule: `measured_promotion`.

Use the paired protocol and family/control/cold/build rules in [MEASUREMENT.md](../MEASUREMENT.md). Predeclare the targeted family and held-out controls before changing the implementation. Keep instrumentation off in production timings. Adopt only with supported gains in both runs; otherwise record rejected/inconclusive and remove default-path cost. A necessary immutable-interface change may be contract_only with its separate justification.

## Candidate gate schedule

Before implementation, freeze and verify the immediate-predecessor archive and a task-owned predeclaration manifest. Bind the manifest to exact registry workload IDs and input values, optimization shapes, target small/large rows, held-out rows, protected controls, stages, feature/runtime settings, and applicable cold products. Candidate promotion compares against this immediate predecessor only; P001 is reserved for exhaustive milestone totals. Measure the complete declared candidate set in two independent runs with at least 30 complete pairs per row and run, using 10,000 deterministic bootstrap resamples and uninstrumented production binaries. The applicable large-module preparation control and tiny `call_overhead` first-call/load-and-call cold controls are mandatory. Preserve every task-specific semantic, fuel/cancellation, trap/memory, fallback, and optimizer-bound case regardless of the focused timing selection. See [MEASUREMENT.md](../MEASUREMENT.md) for cold-product identity, sample reuse, thresholds, and milestone rules.

## Before editing

Run these commands before changing the candidate sources. The archive is immutable and captures the immediate predecessor. Retain P001's original archive for milestone totals. For native work, use the named portable control and verify its actual policy/quantum; if this archive already exists with different sources, choose a new recorded run-specific directory and update the comparison path rather than overwriting it.

```sh
python3 tools/wtime-bench/baseline.py --output target/performance/P064-before
python3 -B tools/wtime-bench/selection_manifest.py freeze docs/benchmarks/performance/P064-selection.json target/performance/P064-before/manifest.json target/performance/P064/selection-freeze.json
```

Create and validate the task-owned selection manifest before these commands.
The freezer requires the archived core to match the live predecessor, so run
it before the first runtime edit. The shared contract revalidates this receipt
in its `freeze` phase without replacing it after implementation.

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. Executed evidence is recorded below.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/P064/shared-finalization/attempt-0002/evidence.json": "2c112dae8233a37e0be25acfe7526bea40e3e771f61f0d18bd540e2cdf480b89"
  },
  "evidence": [
    {
      "bytes": 2599,
      "path": "/workspace/wtime/target/performance/P064/shared-finalization/attempt-0002/evidence.json",
      "sha256": "2c112dae8233a37e0be25acfe7526bea40e3e771f61f0d18bd540e2cdf480b89"
    }
  ],
  "outcome": "rejected",
  "status": "done",
  "task": "P064"
}
```

The full contract closed successfully with freeze attempt 0001, verify attempt 0002, measure attempt 0001, analyze attempt 0001, and close attempt 0079; publication attempt 0001 recorded `done/rejected`. Verification executed 294 register semantic tests, one default-policy test, and five combined-control tests (300 passed, zero failed or ignored). The frozen candidate contained 9 workload rows across 29 row-stage groups; measurement retained 1,500 physical pairs and 1,740 logical pair references, with two independent runs of 30 pairs per group and 10,000 bootstrap resamples. Warm ratios were 0.992807658 (95% CI 0.9817787–1.0123182) and 0.996689483 (95% CI 0.9838080–1.0130657). The gate rejected promotion because neither run supported the required 5% gain and small/held-out cold-load controls regressed. The default clean build was 2.583861 MB-equivalent and the optional Register build 3.005915; stripped sizes were 1,000,712 and 1,078,248 bytes respectively, with zero external core dependencies and all build budgets passing. The immutable predecessor was `target/performance/P064-before` at commit `85c90e5`. Durable reports are `docs/benchmarks/performance/P064*`; export attempt `target/recovery/P064-export-0002` completed with eight copies and hash records. Fixture inputs and probe identities are recorded in `crates/wtime-runtime/tests/fixtures/performance/p064/provenance.json`. The first full verification exposed a stale P060 counter assertion; it was corrected to account for three new caller steps and the full verification then passed, with no semantic failure suppressed. The implementation and interface are documented in `docs/architecture/register-call-exits.md`. Register remains opt-in; this focused result makes no broad portfolio or P001 claim, and the nondefault interface work remains relevant to P065/P066.
