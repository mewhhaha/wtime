# P072 — Maintain continuation bytes incrementally and transactionally

Status: **done**

Outcome: **inconclusive**

Kind: `candidate` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P002 — Compare separately built baseline and candidate binaries](../00-measurement/P002-paired-driver.md)

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P012 — Measure continuation capture, resume and accounting](../00-measurement/P012-continuation-workloads.md)

[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)

[P016 — Add explicit executor policies and test integration hooks](../01-execution-contracts/P016-execution-policy.md)

[P029 — Stop cloning function signatures into every activation](../02-structural-wins/P029-shared-function-metadata.md)

[P031 — Reuse invocation scratch capacity safely under host reentry](../02-structural-wins/P031-activation-scratch-pool.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/continuation.rs`
- `crates/wtime-runtime/src/gc_exec.rs`
- `crates/wtime-runtime/tests/performance/p072.rs`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

Store maintains the exact retained continuation-byte total; creation/capture/bind/consume/drop/GC transitions update it transactionally, replacing repeated scans while a test-only full scan remains the oracle.

## Implementation recipe

0. Before runtime edits, create docs/benchmarks/performance/P072-selection.json and docs/benchmarks/performance/P072-verification.json; freeze the exact target small/large, held-out, protected and cold rows, and bind all semantic cases to successful full verification receipts.

1. List every state/capacity transition and ownership owner in continuation.rs, including host suspension, failed bind/install and GC removal.
2. Compute checked per-state retained bytes before allocation/mutation, reserve aggregate budget, commit only on success and release exactly once on consume/drop/reclaim.
3. Keep shared metadata versus owned activation accounting consistent with P029/P031; compare the ledger to the full recomputation after each test transition.
4. Measure many-live-continuation creation/capture scaling and budget-check cost, preserving one-shot/signature/fuel semantics.
5. Record the changed path with focused assertions/diagnostic counts and exact source/binary identities; compare the pre-edit archive, and revert a rejected structural optimization rather than adding a permanent toggle solely for testing.

## Meaningful cases

- Long randomized transition sequences keep ledger==full scan.
- Failed capture/bind/install changes no retained total and consumes no continuation unexpectedly.
- Consume/reclaim/host outcome cleanup returns bytes exactly once and near-limit admission remains correct.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime p072_ -- --nocapture
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml -j 4
python3 tools/checks/build_budget.py --output target/performance/P072/build.json
python3 -B tools/verify.py plan --contract docs/benchmarks/performance/P072-verification.json
python3 -B tools/verify.py close --contract docs/benchmarks/performance/P072-verification.json --output target/performance/P072/complete --execute
```

Declare the semantic-test and build commands above in the appropriate contract phases;
run `plan` and `close` externally to drive that contract. Bind full transitive
compilation/fixture inputs and require every semantic
case declared in the selection manifest. Its `measure` phase uses
`focused_compare.py --manifest docs/benchmarks/performance/P072-selection.json`
with `--frozen {freeze_dir}/selection.json`, `--semantic-receipt {verify_dir}/receipt.json`,
the immutable predecessor adapter, and the actual production executable/mode.
Its `analyze` phase calls `tools/verify.py statistics` once, supplying that
selection manifest as `--contract`, the paired report, and all build attestations.
Do not run an exhaustive comparison or a separate raw analyzer during this candidate gate.
See the [exact workflow](../../../wtime/docs/verification/fast-performance.md).

## Measurement and decision

Selection: `continuations`. Explicit execution setting: `decoded`. Decision rule: `measured_promotion`.

Use the paired protocol and family/control/cold/build rules in [MEASUREMENT.md](../MEASUREMENT.md). Predeclare the targeted family and held-out controls before changing the implementation. Keep instrumentation off in production timings. Adopt only with supported gains in both runs; otherwise record rejected/inconclusive and remove default-path cost. A necessary immutable-interface change may be contract_only with its separate justification.

## Candidate gate schedule

Before implementation, freeze and verify the immediate-predecessor archive and a task-owned predeclaration manifest. Bind the manifest to exact registry workload IDs and input values, optimization shapes, target small/large rows, held-out rows, protected controls, stages, feature/runtime settings, and applicable cold products. Candidate promotion compares against this immediate predecessor only; P001 is reserved for exhaustive milestone totals. Measure the complete declared candidate set in two independent runs with at least 30 complete pairs per row and run, using 10,000 deterministic bootstrap resamples and uninstrumented production binaries. The applicable large-module preparation control and tiny `call_overhead` first-call/load-and-call cold controls are mandatory. Preserve every task-specific semantic, fuel/cancellation, trap/memory, fallback, and optimizer-bound case regardless of the focused timing selection. See [MEASUREMENT.md](../MEASUREMENT.md) for cold-product identity, sample reuse, thresholds, and milestone rules.

## Before editing

Run these commands before changing the candidate sources. The archive is immutable and captures the immediate predecessor. Retain P001's original archive for milestone totals. For native work, use the named portable control and verify its actual policy/quantum; if this archive already exists with different sources, choose a new recorded run-specific directory and update the comparison path rather than overwriting it.

```sh
python3 tools/wtime-bench/baseline.py --output target/performance/P072-before
python3 -B tools/wtime-bench/selection_manifest.py freeze docs/benchmarks/performance/P072-selection.json target/performance/P072-before/manifest.json --output target/performance/P072/selection-freeze.json
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
    "/workspace/wtime/target/performance/P072/shared-finalization/attempt-0003/evidence.json": "08418ee2689db4cbb84a110b048c0eee0c6c830e169be2486624dc90e4139125"
  },
  "evidence": [
    {
      "bytes": 2168,
      "path": "/workspace/wtime/target/performance/P072/shared-finalization/attempt-0003/evidence.json",
      "sha256": "08418ee2689db4cbb84a110b048c0eee0c6c830e169be2486624dc90e4139125"
    }
  ],
  "outcome": "inconclusive",
  "status": "done",
  "task": "P072"
}
```


## Evaluated candidate and restoration (2026-10-15)

P072 was executed and is **inconclusive**; the incremental ledger is not retained
in the default runtime. The task added the source-bound live-set driver and
independent lifecycle model, then evaluated checked Store/per-state charges,
transactional capture/bind/consume updates, and GC reclamation against a full
storage recomputation. Candidate tests include 1,024 randomized transitions,
near-limit capture/bind failures, failed install reservation, failed GC,
one-shot consumption, source-fuel prefixes, cancellation, and repeated cleanup.

The pre-edit archive is commit `4f6d232e280972f2efa27b014d2644c270ba3b95`,
manifest SHA-256 `995a4cb27dfd9fe78c59f6127848ddf6e0988ce3a3f8c7ba5a03a898d657c912`.
Selection SHA-256 `df3b33d8f6a3f18aa095a1b2cf85678e0776e73105ce64efad3f26edd86df9ed`
was frozen before runtime edits. The immutable depth-zero guest has Wasm SHA-256
`2e30292dab3ac305fd029a6b8ff92d5af14f96900b40d5053317c75e8ba589a3`.

The full candidate closure passed with 105 unfiltered default runtime unit tests,
four focused cases with Compact/Register/reference/counters compiled, three
production driver tests, five model checks, eight lifecycle/analyzer checks,
15 existing candidate-gate checks, and five producer checks. There were no
failed or ignored tests in the final receipts. All build/dependency commands
succeeded. A contract flag error and an incomplete semantic input declaration
were corrected before candidate timing; unchanged successful builds were
revalidated by pinned receipts rather than rebuilt. The first measurement
attempt failed its source-binding preflight before timing. Failed attempts and
the initial driver/test drafts remain in the retained record.

The executed focused comparison collected **660 complete pairs**: 11 groups,
30 pairs per group in each of two runs (seeds 72001/72002), decoded execution,
production binaries, fresh-process batch one, and full suspension/root/resume/GC
oracles. No warm calibration or synthetic per-call division was used. Statistics
were computed once with 10,000 resamples. Target capture ratios were
0.7600668132 (95% CI 0.7188082331–0.7948179766) and 0.7342571642
(0.7149677045–0.7633980087). The 512-live capture medians improved about
40–44%, and its setup medians about 65–66%; small live-set setup and a tiny
first-call control could not establish the required regression bound.
These results do not promote the candidate or imply a general Wasmtime gain.

Candidate clean build: 2.726194269 seconds; touched CLI leaf: 0.143437581 seconds;
stripped CLI: 1008344 bytes; zero external default-core dependencies.
The candidate binary SHA-256 is
`f5112aad50edbfceb84876af8ca07b95a222841fc3082901e6bfce17cb1c91cc`;
the separately built frozen-core adapter is
`b11e2322e486a74b1242d0f7a7d8fde78b753892487696b0742b88032492631b`.
The raw report SHA-256 is
`3556e843d566bfe0bb41e1d95e1fec038e0a4c0325ea0c4f9899a45bf202ef14`;
statistics SHA-256 is
`a18e4eaf8889f0bbed4e2cd94585c0a2e6e65ca27d399ce335198d8134f45904`.

After publication, both changed runtime files were restored from the verified
predecessor with fresh modification times. A separate full restoration closure
passed 101 default and 173 optional-feature unit tests (both unfiltered), plus
three driver tests and a fresh production build. All 174 archived crate/root
source entries match the predecessor. The restored production binary SHA-256 is
`ad8171499be51c4aec8105bcecbf54b674371b617aad0cca9ff62a17ca10714b`.
Restoration evidence SHA-256 is
`c68873d22464596845b0f7668d9abff39985fb83eaf99890b5d67963567c8719`.
Candidate source snapshots and their successful semantic receipts are retained
as historical evidence; restoration is separately verified and is not promotion.

Artifacts: `docs/benchmarks/performance/P072-{completion,evidence,freeze,proof,statistics,restoration}.json`,
`docs/benchmarks/performance/P072-records.json.gz`, and both task-owned
verification contracts. Native AArch64 verification remains outstanding in the
original plan and is unrelated to this portable candidate outcome.
