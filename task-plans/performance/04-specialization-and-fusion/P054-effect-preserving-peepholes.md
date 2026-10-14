# P054 — Fold pure integer work while preserving source events

Status: **done**

Outcome: **inconclusive**

Kind: `candidate` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P002 — Compare separately built baseline and candidate binaries](../00-measurement/P002-paired-driver.md)

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P008 — Add scaling, branch and call benchmark families](../00-measurement/P008-scalar-workloads.md)

[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)

[P016 — Add explicit executor policies and test integration hooks](../01-execution-contracts/P016-execution-policy.md)

[P019 — Define bounded optimizer effect and source expansion metadata](../01-execution-contracts/P019-effect-certificates.md)

[P020 — Centralize exact source instruction accounting](../01-execution-contracts/P020-exact-execution-accounting.md)

[P037 — Map compact instructions to source events and control targets](../03-compact-executor/P037-compact-source-maps.md)

[P047 — Fuse a bounded set of local and constant arithmetic patterns](P047-local-arithmetic-fusions.md)

[P128 — Add minimal scalar identity and literal-folding workloads](P128-minimal-scalar-workloads.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/execution/compact/fuse.rs`
- `crates/wtime-runtime/tests/performance/p054.rs`
- `crates/wtime-runtime/tests/performance.rs`
- `crates/wtime-runtime/tests/fixtures/performance/p054/regions.wat`
- `crates/wtime-runtime/tests/fixtures/performance/p054/regions.wasm`
- `tools/wtime-bench/focused_compare.py`
- `tools/wtime-bench/selection_manifest.py`
- `tools/wtime-bench/tests/test_p054_scalar_route.py`
- `tools/wtime-bench/tests/test_p054_selection_model.py`

Extend the existing verified `Pattern`/`Fusions`/`recognize`/`execute` implementation in `fuse.rs`; `peephole.rs` does not exist and is not owned. The P054 integrator owns test registration, the compact-route integration, and the fixed task-local `fixture_models` binding. The new P054 admitted region fixtures cover only the bounded i32 literal add/and/or/xor and same-local identity forms. `scalar_p128_workloads.py` and its three P128 fixture sources remain P128-owned, source-hashed inputs; P054 binds the fixed model without editing it or the global 459-row registry.

## Input/output contract

Extend the existing verified fusion path with a fixed whitelist of straight-line i32 literal and identity patterns inside one basic block. Preserve the 1:1 source map, source targets and guards, source-fuel/cancellation events, and canonical pending-value materialization; unsupported shapes use the original compact path.

## Implementation recipe

1. Before runtime edits, archive the compact production predecessor and freeze the candidate selection using P128 admitted scalar fixtures. Build the predecessor with `--mode compact`; pass the freeze output positionally.
2. Extend the existing verified Pattern, Fusions, recognize, and execute machinery in crates/wtime-runtime/src/execution/compact/fuse.rs with a fixed whitelist of straight-line i32 literal and same-local identity patterns inside one basic block: literal const/const i32.add, i32.and, i32.or, i32.xor, and local.get/local.set identity.
3. Keep the 1:1 source map, target/guard checks, and verification path unchanged. Charge each original source event and materialize canonical pending stack/local values at every fuel stop, cancellation, and fallback boundary.
4. Bind the P054 selection to the allowlisted `p128_scalar` fixture model by its source SHA256 in fixture_models. Add P054-owned admitted regions.wat/.wasm and test the model binding in a fresh temporary root; do not edit the P128 model, P128 fixture sources, global 459-row registry, or shared selection membership.
5. The full close ran all four P054 semantic cases (358 Rust checks including the restored predecessor), five selection-model tests, 15 legacy fast-selection tests, four compact-route tests, nine P128 model tests, and 15 fast-candidate tests (48 Python checks); all passed. Neither run met the required 5% improvement threshold, so publish inconclusive, restore the core, and make no promotion claim.

## Meaningful cases

- p054_literal_and_identity_results_use_verified_fusions: public literal and local-copy fixtures route through verified fusions and match independent scalar results.
- p054_every_source_prefix_and_materialization_matches_decoded: public fuel-prefix probes compare each intermediate stack/local state with decoded execution.
- p054_cancellation_reconstructs_pending_values: private cancellation checks reconstruct canonical pending values at each optimized source boundary.
- p054_budget_corruption_and_unsupported_shapes_decline: private verification rejects corrupt/budget-exhausted descriptors and unsupported or externally-entered shapes.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

Full candidate close ran 358 Rust checks and 48 Python tooling tests, all passed with zero failures or ignored checks. These counts describe historical candidate verification; rollback verification of the live restored source is 354 passed, zero failed, zero ignored. The P054-specific Rust test and fusion changes are not in the live tree; the existing P047/P050 fusion implementation remains.

```sh
cargo test --locked -p wtime-runtime --features compact-executor,execution-reference p054_ -- --nocapture
cargo test --locked -p wtime-runtime --features compact-executor,execution-reference p054_ -- --nocapture
python3 -B -m unittest discover -s tools/wtime-bench/tests -p test_p054_selection_model.py -v -b
python3 -B -m unittest discover -s tools/wtime-bench/tests -p test_fast_selection.py -v -b
python3 -B -m unittest discover -s tools/wtime-bench/tests -p test_p054_scalar_route.py -v -b
python3 -B -m unittest discover -s tools/wtime-bench/tests -p test_p128_scalar_workloads.py -v -b
python3 -B -m unittest discover -s tools/wtime-bench/tests -p test_fast_candidate.py -v -b
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml --features compact-executor -j 4
python3 tools/checks/build_budget.py --output target/performance/P054/build.json
python3 tools/checks/build_budget.py --features wtime-runtime/compact-executor --output target/performance/P054/optional-build.json
python3 -B tools/verify.py plan --contract docs/benchmarks/performance/P054-verification.json
python3 -B tools/verify.py close --contract docs/benchmarks/performance/P054-verification.json --output target/performance/P054/complete --execute
```

Declare the semantic-test and build commands above in the appropriate contract phases;
run `plan` and `close` externally to drive that contract. Bind full transitive
compilation/fixture inputs and require every semantic
case declared in the selection manifest. Its `measure` phase uses
`focused_compare.py --manifest docs/benchmarks/performance/P054-selection.json`
with `--frozen {freeze_dir}/selection.json`, `--semantic-receipt {verify_dir}/receipt.json`,
the immutable predecessor adapter, and the actual production executable/mode.
Its `analyze` phase calls `tools/verify.py statistics` once, supplying that
selection manifest as `--contract`, the paired report, and all build attestations.
Do not run an exhaustive comparison or a separate raw analyzer during this candidate gate.
See the [exact workflow](../../../wtime/docs/verification/fast-performance.md).

## Completion notes

P054 was published **done / inconclusive**. The canonical completion record is [`completion.json`](../../../wtime/target/performance/P054/shared-finalization/attempt-0002/completion.json); its evidence SHA256 is `99cdd6d0b72520f79c6235e36684594a71c85ec1f9b80903f529a84b2c150695`. Verification passed; measurement did not satisfy the required adoption threshold, so the candidate was not promoted.

The historical candidate close passed **358 Rust checks** and **48 Python tooling tests**, with zero failures or ignored checks. The Rust count included four P054 cases on top of the 354-check predecessor. The tooling total was five P054 selection-model tests, 15 legacy fast-selection tests, four P054 compact-route tests, nine P128 model tests, and 15 fast-candidate tests. The retained P054 fixture-model binding and compact-route tooling are not runtime promotion.

The paired report contained **67 physical groups**, **76 logical groups**, **4,020 distinct pairs**, and **4,560 logical pair references**. Statistics were computed exactly once. Run 1's primary median ratio was **0.9590705373001746** (95% CI **0.9468033479591415–0.9691445512333159**); run 2 was **0.9536466227154456** (95% CI **0.94111404935326–0.966551255356909**). Both point estimates favored the candidate, but neither supported the required 5% improvement. The decision recorded **58 reasons**, including **56 inconclusive control results**, and **zero supported regressions**. The outcome remains inconclusive, with no speed-promotion claim.

The candidate patch is replayable from [`P054-inconclusive-candidate.patch`](../../../wtime/docs/benchmarks/performance/P054-inconclusive-candidate.patch). Rollback removed the P054 Rust fusion changes and test file; the live core has **113 files**, each matching the frozen predecessor. The restored verification report is [`report.json`](../../../wtime/target/performance/P054/restored-verification/attempt-0001/report.json): **354 passed, 0 failed, 0 ignored**. No P054 candidate Rust test or P054-specific fusion changes remain live; the existing P047/P050 fusion implementation is retained. The selection-model binding and compact-route tooling remain available, while the candidate patch preserves the rejected experiment for replay.

The frozen selection used 16 rows and 76 logical groups, derived from the fixed P128 model and existing controls. The global registry remained at 459 rows. These results are limited to this declared candidate selection and are not P001 portfolio evidence.

## Measurement and decision

Selection: `scalar`. Explicit execution setting: `compact`. Decision rule: `measured_promotion`. The focused selection is derived from the P128 model: its three admitted fixtures at n=0, 17, 32, and 10000 provide 12 fixture rows (n32/n10000 targeted, n17 held out, n0 protected). Add the existing accumulator/dependent n32 protected controls and the `call_overhead` and `prepare_large` controls for 16 rows and 76 logical stage groups. This was the frozen P054 candidate selection; P128 fixture aliases did not add registry rows or change shared selection membership.

Use the paired protocol and family/control/cold/build rules in [MEASUREMENT.md](../MEASUREMENT.md). Predeclare the targeted family and held-out controls before changing the implementation. Keep instrumentation off in production timings. Adopt only with supported gains in both runs; otherwise record rejected/inconclusive and remove default-path cost. A necessary immutable-interface change may be contract_only with its separate justification.

## Candidate gate schedule

Before implementation, freeze and verify the immediate-predecessor archive and a task-owned predeclaration manifest. Bind the manifest to exact registry workload IDs and input values, optimization shapes, target small/large rows, held-out rows, protected controls, stages, feature/runtime settings, and applicable cold products. Candidate promotion compares against this immediate predecessor only; P001 is reserved for exhaustive milestone totals. Measure the complete declared candidate set in two independent runs with at least 30 complete pairs per row and run, using 10,000 deterministic bootstrap resamples and uninstrumented production binaries. The applicable large-module preparation control and tiny `call_overhead` first-call/load-and-call cold controls are mandatory. Preserve every task-specific semantic, fuel/cancellation, trap/memory, fallback, and optimizer-bound case regardless of the focused timing selection. See [MEASUREMENT.md](../MEASUREMENT.md) for cold-product identity, sample reuse, thresholds, and milestone rules.

## Before editing

Run these commands before changing the candidate sources. The archive is immutable and captures the immediate predecessor. Retain P001's original archive for milestone totals. For native work, use the named portable control and verify its actual policy/quantum; if this archive already exists with different sources, choose a new recorded run-specific directory and update the comparison path rather than overwriting it.

```sh
python3 tools/wtime-bench/baseline.py --mode compact --output target/performance/P054-before
python3 -B tools/wtime-bench/selection_manifest.py freeze docs/benchmarks/performance/P054-selection.json target/performance/P054-before/manifest.json target/performance/P054/selection-freeze.json
```

Create and validate the task-owned selection manifest before these commands.
The predecessor archive must be the actual compact production mode. The freeze output path is positional (do not pass `--output`); run this before the first runtime edit. The shared contract revalidates the receipt in its `freeze` phase without replacing it after implementation.

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

The published status/outcome, canonical evidence, decision, and rollback results are recorded below and in Completion notes.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/P054/shared-finalization/attempt-0002/evidence.json": "99cdd6d0b72520f79c6235e36684594a71c85ec1f9b80903f529a84b2c150695"
  },
  "evidence": [
    {
      "bytes": 2471,
      "path": "/workspace/wtime/target/performance/P054/shared-finalization/attempt-0002/evidence.json",
      "sha256": "99cdd6d0b72520f79c6235e36684594a71c85ec1f9b80903f529a84b2c150695"
    }
  ],
  "outcome": "inconclusive",
  "status": "done",
  "task": "P054"
}
```
