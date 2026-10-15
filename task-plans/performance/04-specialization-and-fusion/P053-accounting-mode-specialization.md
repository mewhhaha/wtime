# P053 — Separate metered and unmetered dispatch without weaker defaults

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

[P020 — Centralize exact source instruction accounting](../01-execution-contracts/P020-exact-execution-accounting.md)

[P038 — Lower constants, locals and common integer operations](../03-compact-executor/P038-compact-scalar-lowering.md)

[P047 — Fuse a bounded set of local and constant arithmetic patterns](P047-local-arithmetic-fusions.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C2, C3, C4, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/execution/accounting.rs`
- `crates/wtime-runtime/src/execution.rs`
- `crates/wtime-runtime/src/lib.rs`
- `crates/wtime-runtime/src/performance.rs`
- `crates/wtime-runtime/tests/performance/p053.rs`

The compact tier selects a small function-pointer accounting variant once at invocation. The default no-`compact-executor` path keeps its original static accounting. `execution/compact/execute.rs` is unchanged and is not owned by this task.

## Input/output contract

Dispatch selects fuel-present versus fuel-absent accounting once at invocation/tier entry; both retain default per-source cancellation, logical limits and source coverage policy.

## Implementation recipe

1. In the opt-in compact invocation, select a small function-pointer accounting variant once for metered or unmetered execution; retain the original static accounting in the default no-compact path and share the dispatcher body.
2. Preserve per-source cancellation and logical operand limits. On resumed saved fuel changing from None to Some, reselect metered accounting before charging the next source instruction.
3. Keep opcode-coverage instrumentation out of production measurements and prove the disabled path has no runtime branch.
4. Measure the two distinct paired-fuel datasets separately and calculate statistics once for each; aggregate their producer output without reanalysis. Inspect code-size/build cost before adding modes.
5. Forward explicit compact policy through the harness/integrator and record actual optimized/fallback counts; persistent selectors remain nondefault.

## Meaningful cases

- p053_scalar_results_and_source_prefixes_preserve_accounting: public partial-fuel prefixes and independent scalar results exercise actual dispatch-variant counts.
- p053_unmetered_cancellation_and_operand_limits_still_apply: host cancellation at a source prefix and the None-fuel operand-stack limit.
- p053_resumed_saved_fuel_switches_to_metered_once: private saved-fuel boundary merge and error priority.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime --features compact-executor,execution-reference p053_ -- --nocapture
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml --features compact-executor -j 4
python3 tools/checks/build_budget.py --output target/performance/P053/build.json
python3 tools/checks/build_budget.py --features wtime-runtime/compact-executor --output target/performance/P053/optional-build.json
python3 -B tools/verify.py plan --contract docs/benchmarks/performance/P053-verification.json
python3 -B tools/verify.py close --contract docs/benchmarks/performance/P053-verification.json --output target/performance/P053/complete --execute
```

Declare the semantic-test and build commands above in the appropriate contract phases;
run `plan` and `close` externally to drive that contract. Bind full transitive
compilation/fixture inputs and require every semantic
case declared in the selection manifest. Its `measure` phase uses
`focused_compare.py --manifest docs/benchmarks/performance/P053-selection.json`
with `--frozen {freeze_dir}/selection.json`, `--semantic-receipt {verify_dir}/receipt.json`,
the immutable predecessor adapter, and the actual production executable/mode.
Its `analyze` phase calls `tools/verify.py statistics` once, supplying that
selection manifest as `--contract`, the paired report, and all build attestations.
Do not run an exhaustive comparison or a separate raw analyzer during this candidate gate.
See the [exact workflow](../../../wtime/docs/verification/fast-performance.md).

## Completion notes

The P053 candidate completed its full close and was published as **done / rejected**. It was not adopted. The canonical completion record is [`completion.json`](../../../wtime/target/performance/P053/shared-finalization/attempt-0001/completion.json); its evidence SHA256 is `0b064760b030e71b7c4a35bc88d97d8ea5700b7837453f838da6212c33d7d5ee`. The evidence records passed verification and rejected measurement; a passed verification phase does not imply promotion.

The candidate verification ran 357 Rust tests, 25 Python tooling unit tests, and four private Rust fuel-proof tests: 386 total, with zero failures or ignored tests. The 184-observation staged proof and both build budgets passed. The private tests checked P127 fuel transitions; the retained P053 tooling tests include five fuel-gate tests and five dual-policy decision tests. The candidate verification contract and evidence describe historical candidate sources only.

The two policy measurement reports each contain 2,400 distinct physical pairs and 2,760 logical pair references across 40 physical groups and 46 logical groups; aliases add no independent samples. Each has two runs with 30 complete pairs per physical group. Across both reports there are 4,800 distinct pairs. Statistics were computed once for each distinct policy report, twice total, and the aggregate producer performed zero reanalyses. Both policies regressed on the targeted warm-call aggregate in both runs (ratio above 1 means slower):

- Unmetered: run 1 median ratio **1.1085296423** (95% CI **1.0957726352–1.1157650338**); run 2 **1.1048786828** (95% CI **1.0959269140–1.1185939920**). Supported regressions: **27**.
- Metered: run 1 median ratio **1.1013268324** (95% CI **1.0863213270–1.1162405506**); run 2 **1.0860421332** (95% CI **1.0689245810–1.1184738723**). Supported regressions: **14**.

The candidate patch is preserved at [`candidate.patch`](../../../wtime/target/performance/P053/rollback/attempt-0001/candidate.patch), and `git apply --check` passed. Rollback restored all four Rust source files with fresh modification times, removed the new P053 Rust test, and verified the live 113-file runtime core matches the frozen predecessor. A real Cargo rebuild of restored sources passed the unfiltered suite: **354 passed, 0 failed, 0 ignored** ([restored report](../../../wtime/target/performance/P053/restored-verification/attempt-0001/report.json)). The P053-specific candidate Rust tests are therefore historical close evidence; they are not in the restored live tree. The typed fuel gate and dual-decision tooling tests remain.

The default and compact build budgets both passed. Recorded profile output: default clean build **2.486 s**, release **5.119 s**, runtime leaf **0.403 s**, stripped binary **1,000,712 bytes**; compact clean build **2.945 s**, release **5.956 s**, runtime leaf **0.498 s**, stripped binary **1,093,056 bytes**. These profile values are not a causal before/after speed claim.

Earlier setup failures are retained separately from the passing candidate checks: the first freeze invocation used unsupported `--output` syntax and stopped before execution; a first root integration patch used an ambiguous `impl Store` anchor and failed compilation before guest checks; and an early dev run had unknown identity and required-unittest-binding schema errors, corrected before the full close. The corrected freeze and integration ran successfully. The candidate close had no semantic test failures.

The frozen baseline had 113 core source files. Candidate-only integration added one test file, producing 114 files during candidate verification; rollback removed that file and restored the exact 113-file predecessor. The selection covered four accumulator/dependent targeted rows, with held-out and protected controls as declared in the selection manifest. No candidate adoption or broad portfolio claim is made.

## Measurement and decision

Selection: `scalar`. Explicit execution setting: `compact`. Decision rule: `measured_promotion`.

Use the paired protocol and family/control/cold/build rules in [MEASUREMENT.md](../MEASUREMENT.md). Predeclare the targeted family and held-out controls before changing the implementation. Keep instrumentation off in production timings. Adopt only with supported gains in both runs; otherwise record rejected/inconclusive and remove default-path cost. A necessary immutable-interface change may be contract_only with its separate justification.

## Candidate gate schedule

Before implementation, freeze and verify the immediate-predecessor archive and a task-owned predeclaration manifest. Bind the manifest to exact registry workload IDs and input values, optimization shapes, target small/large rows, held-out rows, protected controls, stages, feature/runtime settings, and applicable cold products. Candidate promotion compares against this immediate predecessor only; P001 is reserved for exhaustive milestone totals. Measure the complete declared candidate set in two independent runs with at least 30 complete pairs per row and run, using 10,000 deterministic bootstrap resamples and uninstrumented production binaries. The applicable large-module preparation control and tiny `call_overhead` first-call/load-and-call cold controls are mandatory. Preserve every task-specific semantic, fuel/cancellation, trap/memory, fallback, and optimizer-bound case regardless of the focused timing selection. See [MEASUREMENT.md](../MEASUREMENT.md) for cold-product identity, sample reuse, thresholds, and milestone rules.

## Before editing

Run these commands before changing the candidate sources. The archive is immutable and captures the immediate predecessor. Retain P001's original archive for milestone totals. For native work, use the named portable control and verify its actual policy/quantum; if this archive already exists with different sources, choose a new recorded run-specific directory and update the comparison path rather than overwriting it.

```sh
python3 tools/wtime-bench/baseline.py --output target/performance/P053-before
python3 -B tools/wtime-bench/selection_manifest.py freeze docs/benchmarks/performance/P053-selection.json target/performance/P053-before/manifest.json target/performance/P053/selection-freeze.json
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
    "/workspace/wtime/target/performance/P053/shared-finalization/attempt-0001/evidence.json": "0b064760b030e71b7c4a35bc88d97d8ea5700b7837453f838da6212c33d7d5ee"
  },
  "evidence": [
    {
      "bytes": 2528,
      "path": "/workspace/wtime/target/performance/P053/shared-finalization/attempt-0001/evidence.json",
      "sha256": "0b064760b030e71b7c4a35bc88d97d8ea5700b7837453f838da6212c33d7d5ee"
    }
  ],
  "outcome": "rejected",
  "status": "done",
  "task": "P053"
}
```
