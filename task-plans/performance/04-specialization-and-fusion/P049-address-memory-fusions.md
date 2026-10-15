# P049 — Fuse pure address formation with a checked memory operation

Status: **done**

Outcome: **rejected**

Kind: `candidate` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P002 — Compare separately built baseline and candidate binaries](../00-measurement/P002-paired-driver.md)

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P009 — Add memory width, bulk and growth benchmark families](../00-measurement/P009-memory-workloads.md)

[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)

[P016 — Add explicit executor policies and test integration hooks](../01-execution-contracts/P016-execution-policy.md)

[P043 — Resolve memory bindings and specialize checked scalar accesses](../03-compact-executor/P043-compact-memory-plans.md)

[P047 — Fuse a bounded set of local and constant arithmetic patterns](P047-local-arithmetic-fusions.md)

[P126 — Add scalable memory-width and read-scan workload aliases](P126-scalable-memory-workload-aliases.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C5, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/execution/compact/fuse_memory.rs`
- `crates/wtime-runtime/src/execution/compact/memory.rs`
- `crates/wtime-runtime/tests/performance/p049.rs`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

Eligible local/constant integer address formation plus one load/store shares compact dispatch; effective address overflow and the source memory trap occur at the original point, with interrupted prefix materialization.

## Implementation recipe

0. Before runtime edits, create docs/benchmarks/performance/P049-selection.json and docs/benchmarks/performance/P049-verification.json; freeze the exact target small/large, held-out, protected and cold rows, and bind all semantic cases to successful full verification receipts.

1. Start with one local.get plus constant offset/address arithmetic pattern and one scalar unshared access; keep shared/complex/wide patterns on unfused compact execution initially.
2. Distinguish wrapping guest address arithmetic from checked memory immediate addition; never combine them into a different overflow/trap rule.
3. Preserve source accounting/transient stack events and store-value order; perform no memory read/write until the original memory instruction is reached.
4. Measure read/mixed widths and full side effects plus OOB/trap-order controls; keep variant count bounded.
5. Forward the explicit `compact` setting through the harness/policy integrator and record actual optimized/fallback counts. Persistent research selectors are nondefault; reversible changes to an existing tier use before/after artifacts and are selected only by their gate.

## Meaningful cases

- Wrapped guest address and overflowing memory immediate retain distinct semantics.
- Fuel stop before the memory op performs no access or store.
- Prior store followed by a failing fused load leaves only the permitted prior side effect.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime --features compact-executor,execution-reference p049_ -- --nocapture
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml --features compact-executor -j 4
python3 tools/checks/build_budget.py --output target/performance/P049/build.json
python3 tools/checks/build_budget.py --features wtime-runtime/compact-executor --output target/performance/P049/optional-build.json
python3 -B tools/verify.py plan --contract docs/benchmarks/performance/P049-verification.json
python3 -B tools/verify.py close --contract docs/benchmarks/performance/P049-verification.json --output target/performance/P049/complete --execute
```

Declare the semantic-test and build commands above in the appropriate contract phases;
run `plan` and `close` externally to drive that contract. Bind full transitive
compilation/fixture inputs and require every semantic
case declared in the selection manifest. Its `measure` phase uses
`focused_compare.py --manifest docs/benchmarks/performance/P049-selection.json`
with `--frozen {freeze_dir}/selection.json`, `--semantic-receipt {verify_dir}/receipt.json`,
the immutable predecessor adapter, and the actual production executable/mode.
Its `analyze` phase calls `tools/verify.py statistics` once, supplying that
selection manifest as `--contract`, the paired report, and all build attestations.
Do not run an exhaustive comparison or a separate raw analyzer during this candidate gate.
See the [exact workflow](../../../wtime/docs/verification/fast-performance.md).

## Measurement and decision

Selection: `memory`. Explicit execution setting: `compact`. Decision rule: `measured_promotion`.

Use the paired protocol and family/control/cold/build rules in [MEASUREMENT.md](../MEASUREMENT.md). Predeclare the targeted family and held-out controls before changing the implementation. Keep instrumentation off in production timings. Adopt only with supported gains in both runs; otherwise record rejected/inconclusive and remove default-path cost. A necessary immutable-interface change may be contract_only with its separate justification.

## Candidate gate schedule

Before implementation, freeze and verify the immediate-predecessor archive and a task-owned predeclaration manifest. Bind the manifest to exact registry workload IDs and input values, optimization shapes, target small/large rows, held-out rows, protected controls, stages, feature/runtime settings, and applicable cold products. Candidate promotion compares against this immediate predecessor only; P001 is reserved for exhaustive milestone totals. Measure the complete declared candidate set in two independent runs with at least 30 complete pairs per row and run, using 10,000 deterministic bootstrap resamples and uninstrumented production binaries. The applicable large-module preparation control and tiny `call_overhead` first-call/load-and-call cold controls are mandatory. Preserve every task-specific semantic, fuel/cancellation, trap/memory, fallback, and optimizer-bound case regardless of the focused timing selection. See [MEASUREMENT.md](../MEASUREMENT.md) for cold-product identity, sample reuse, thresholds, and milestone rules.

## Before editing

Run these commands before changing the candidate sources. The archive is immutable and captures the immediate predecessor. Retain P001's original archive for milestone totals. For native work, use the named portable control and verify its actual policy/quantum; if this archive already exists with different sources, choose a new recorded run-specific directory and update the comparison path rather than overwriting it.

```sh
python3 tools/wtime-bench/baseline.py --mode compact --output target/performance/P049-before
python3 -B tools/wtime-bench/selection_manifest.py freeze docs/benchmarks/performance/P049-selection.json target/performance/P049-before/manifest.json target/performance/P049/selection-freeze.json
```

Create and validate the task-owned selection manifest before these commands.
The freezer requires the archived core to match the live predecessor, so run
it before the first runtime edit. The shared contract revalidates this receipt
in its `freeze` phase without replacing it after implementation.

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. Executed evidence is recorded below.

## Executed result

P049 completed as **done / rejected**. Its one statistics computation used 1,740 unique physical pairs for 1,800 logical references across two runs, with 30 pairs per logical group and 10,000 bootstrap resamples. The selected warm-call median ratios were 0.73615654 (95% interval 0.66134717–0.81614486) and 0.65142163 (0.52063894–0.79280063). Although those targeted results indicated roughly 26–35% lower time, the gate rejected the candidate: it found supported regressions in `call_overhead` and `load_and_call`, including `read_scan_1024` at `warm_call`, and 37 additional uncertainty reasons. No portfolio or Wasmtime speed claim follows.

All eight focused candidate checks and all 355 full runtime cases passed, with zero failures or ignored cases. Both build budgets passed: default clean 25.384 s, leaf 3.626 s, stripped binary 1,000,712 bytes and RSS 285,808 KiB; compact clean 32.530 s, leaf 1.163 s, stripped binary 1,090,576 bytes and RSS 306,636 KiB. The gate retained all 11 candidate core paths in its rollback archive and restored the exact 111-source predecessor with fresh timestamps. Seventeen restored P047/P043 checks passed. P049 contributes no retained implementation or helper.

The canonical completion is `/workspace/wtime/target/performance/P049/shared-finalization/attempt-0001/completion.json` (evidence SHA-256 `6a9deac5f7327fbb69eb019b174f2c746b4251f35f2f768f36fc1cb6fc2f26b3`). The single statistics artifact is `/workspace/wtime/target/verification/runs/P049/analyze/attempt-0001/statistics.json`; full verification is `/workspace/wtime/target/verification/runs/P049/verify/attempt-0001/receipt.json`; rollback is `/workspace/wtime/target/performance/P049/rollback/attempt-0001/receipt.json`.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/P049/shared-finalization/attempt-0001/evidence.json": "6a9deac5f7327fbb69eb019b174f2c746b4251f35f2f768f36fc1cb6fc2f26b3"
  },
  "evidence": [
    {
      "bytes": 2451,
      "path": "/workspace/wtime/target/performance/P049/shared-finalization/attempt-0001/evidence.json",
      "sha256": "6a9deac5f7327fbb69eb019b174f2c746b4251f35f2f768f36fc1cb6fc2f26b3"
    }
  ],
  "outcome": "rejected",
  "status": "done",
  "task": "P049"
}
```
