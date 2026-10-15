# P073 — Share immutable continuation metadata and bound snapshot copies

Status: **done**

Outcome: **rejected**

Kind: `candidate` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P002 — Compare separately built baseline and candidate binaries](../00-measurement/P002-paired-driver.md)

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P012 — Measure continuation capture, resume and accounting](../00-measurement/P012-continuation-workloads.md)

[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)

[P016 — Add explicit executor policies and test integration hooks](../01-execution-contracts/P016-execution-policy.md)

[P024 — Move branch results and share label metadata without allocation](../02-structural-wins/P024-in-place-branch-results.md)

[P029 — Stop cloning function signatures into every activation](../02-structural-wins/P029-shared-function-metadata.md)

[P031 — Reuse invocation scratch capacity safely under host reentry](../02-structural-wins/P031-activation-scratch-pool.md)

[P069 — Use conservative typed root maps for prepared frames](P069-typed-root-slot-maps.md)

[P072 — Maintain continuation bytes incrementally and transactionally](P072-continuation-byte-ledger.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/lib.rs`
- `crates/wtime-runtime/src/linker.rs`
- `crates/wtime-runtime/src/execution.rs`
- `crates/wtime-runtime/src/reference.rs`
- `crates/wtime-runtime/src/execution/cursor.rs`
- `crates/wtime-runtime/src/execution/register/execute.rs`
- `crates/wtime-runtime/src/execution/compact/lower_control.rs`
- `crates/wtime-runtime/src/continuation.rs`
- `crates/wtime-runtime/tests/performance/p073.rs`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

Continuation capture moves detached segments, shares immutable function and catch metadata, and snapshots every reference edge without duplicating numeric tracing state. Preserve owned public API return types, continuation-owned signatures and arguments, and all existing fuel, one-shot, exactness, and byte-budget semantics.

## Implementation recipe

0. Complete and validate a paired adapter for the existing P012 continuation rows against the immutable baseline. This is harness foundation work; do not invent or synthesize timing evidence. Only then create docs/benchmarks/performance/P073-selection.json and docs/benchmarks/performance/P073-verification.json, and freeze the exact target small/large, held-out, protected and cold rows with all semantic cases bound to successful full receipts.
1. Metadata sharing: confirm capture already moves its frame/operand vectors; change Function.ty to shared Arc<FuncType> metadata while preserving public APIs that return owned function types. Replace per-label copied catch vectors with a catch count and resolve immutable catch data from the defining module/body at Label.start - 1 during unwind, with bounds and type checks. Remove only per-frame owned signature/catch capacity from capture_bytes; retain inline frame/label sizes and all signatures/arguments owned by the continuation.
2. Reference-only tracing snapshots: filter captured-root snapshots and bound-argument GC fields to every Value::Ref variant, while retaining the complete numeric resume state separately. Keep prompts and captured operands owned and detached; do not introduce copy-on-write mutable state or lazy root discovery.
3. Preserve all existing fuel, one-shot, exactness, byte-budget, and resume ownership behavior. Measure capture depth/value/root scaling and collection of captured references, comparing physical retained bytes as well as latency.
4. Record the changed paths with focused assertions/diagnostic counts and exact source/binary identities; compare the pre-edit archive, and revert a rejected structural optimization rather than adding a permanent toggle solely for testing.

## Meaningful cases

- Captured frames are not recycled into invocation pools early.
- Exception/catch metadata survives drop of original Module handles.
- One-shot bind/resume/switch and sole captured references preserve exact budget/fuel/signature behavior.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime p073_ -- --nocapture
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml -j 4
python3 tools/checks/build_budget.py --output target/performance/P073/build.json
python3 -B tools/verify.py plan --contract docs/benchmarks/performance/P073-verification.json
python3 -B tools/verify.py close --contract docs/benchmarks/performance/P073-verification.json --output target/performance/P073/complete --execute
```

Declare the semantic-test and build commands above in the appropriate contract phases;
run `plan` and `close` externally to drive that contract. Bind full transitive
compilation/fixture inputs and require every semantic
case declared in the selection manifest. Its `measure` phase uses
`focused_compare.py --manifest docs/benchmarks/performance/P073-selection.json`
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
python3 tools/wtime-bench/baseline.py --output target/performance/P073-before
python3 -B tools/wtime-bench/selection_manifest.py freeze docs/benchmarks/performance/P073-selection.json target/performance/P073-before/manifest.json target/performance/P073/selection-freeze.json
```

Create and validate the task-owned selection manifest before these commands.
The freezer requires the archived core to match the live predecessor, so run
it before the first runtime edit. The shared contract revalidates this receipt
in its `freeze` phase without replacing it after implementation.

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. Executed evidence is recorded below.

## Final result

P073 completed with a **rejected** performance decision. The candidate close passed 458 tests (426 in verification and 32 in analysis), with zero failures or ignored tests. The measurement contains 1,200 pairs across 20 groups and eight rows, with two 30-pair runs (seeds 73001 and 73002) on CPU 1 in decoded, uninstrumented mode. The primary ratios were 0.905544845 [0.864489835, 0.963826801] and 0.935793463 [0.909572262, 0.999599596]. A supported setup regression at `continuation_depth_0` and uncertain protected/cold controls prevented adoption.

At depth 32, prototype accounting changed continuation bytes from 16,816 to 13,848 and GC bytes from 2,128 to 80; those prototype changes were removed. The entire candidate runtime was restored across eight files and its one added test was removed. Current compiled runtime sources match the predecessor; the harness/fixture/protocol foundation remains. Restoration close 0131 passed 357 unit tests (101 default runtime, 173 combined-feature runtime, and 83 validator), verified all 176 archived crate/root inputs, and built a fresh production executable bound to those restored inputs. These are unit suites; exhaustive conformance remains part of the later milestone gates. The P073 budget report is a validated, unpaired comparison: five added and eight changed inputs against the P066 same-core report with an older harness. It does not support a causal build-speed claim.

Analysis attempts 1 and 2 failed at the statistics stage and remain preserved. After fixing stage binding in the analyzer, analysis attempt 3 computed the decision once. Measurement attempt 2 revalidated the original 1,200-pair measurement from attempt 1 without starting guest processes; it collected no new timings.

Candidate result artifacts: [completion](../../../wtime/docs/benchmarks/performance/P073-completion.json), [evidence](../../../wtime/docs/benchmarks/performance/P073-evidence.json), [freeze](../../../wtime/docs/benchmarks/performance/P073-freeze.json), [proof](../../../wtime/docs/benchmarks/performance/P073-proof.json), [statistics](../../../wtime/docs/benchmarks/performance/P073-statistics.json), [build-comparison](../../../wtime/docs/benchmarks/performance/P073-build-comparison.json), [restoration](../../../wtime/docs/benchmarks/performance/P073-restoration.json), [records.json](../../../wtime/docs/benchmarks/performance/P073-records.json.gz). Full failed-attempt history remains available in the retained P073 records.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/P073/shared-finalization/attempt-0003/evidence.json": "d279f9c24261fe9587b3a312a252bd83defe68df77f3fba386bc608c3f45c51f"
  },
  "evidence": [
    {
      "bytes": 2166,
      "path": "/workspace/wtime/target/performance/P073/shared-finalization/attempt-0003/evidence.json",
      "sha256": "d279f9c24261fe9587b3a312a252bd83defe68df77f3fba386bc608c3f45c51f"
    }
  ],
  "outcome": "rejected",
  "status": "done",
  "task": "P073"
}
```
