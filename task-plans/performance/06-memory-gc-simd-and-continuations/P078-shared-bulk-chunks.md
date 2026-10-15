# P078 — Reduce shared bulk-copy overhead with coordinated fixed chunks

Status: **done**

Outcome: **inconclusive**

Kind: `candidate` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P002 — Compare separately built baseline and candidate binaries](../00-measurement/P002-paired-driver.md)

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P009 — Add memory width, bulk and growth benchmark families](../00-measurement/P009-memory-workloads.md)

[P013 — Measure shared memory, contention and wait latency](../00-measurement/P013-thread-workloads.md)

[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)

[P016 — Add explicit executor policies and test integration hooks](../01-execution-contracts/P016-execution-policy.md)

[P027 — Use slice operations for unshared bulk memory and table copies](../02-structural-wins/P027-unshared-bulk-slices.md)

[P076 — Remove shared read allocation without weakening coordination](P076-shared-fixed-width-access.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C5, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/shared_memory.rs`
- `crates/wtime-runtime/src/execution.rs`
- `crates/wtime-runtime/tests/performance/p078.rs`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

Shared bulk operations use bounded reusable chunk storage or safe ordered locks without per-chunk heap allocation; full range checks, alias overlap direction, synchronization and cancellation remain correct.

## Implementation recipe

0. Before runtime edits, create docs/benchmarks/performance/P078-selection.json and docs/benchmarks/performance/P078-verification.json; freeze the exact target small/large, held-out, protected and cold rows, and bind all semantic cases to successful full verification receipts.

1. Classify same backing, distinct backing and shared/unshared mixed copies; preserve complete prechecks and avoid deadlock with one documented lock ordering.
2. Use a reusable fixed/fallible bounded chunk buffer where borrowing both backings is unsafe; no stack buffer larger than the existing declared chunk policy is introduced casually.
3. Preserve overlap/memmove direction and chunk cancellation checkpoints, keeping regular/atomic races on their existing synchronization path.
4. Measure bulk sizes/aliases under one and several workers, checking complete memory digests and wait/SC controls.
5. Record the changed path with focused assertions/diagnostic counts and exact source/binary identities; compare the pre-edit archive, and revert a rejected structural optimization rather than adding a permanent toggle solely for testing.

## Meaningful cases

- Same-backing forward/backward overlap matches memmove.
- Distinct-backing concurrent opposite-direction copies cannot deadlock.
- OOB prechecks and cancellation leave only permitted completed chunks.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime p078_ -- --nocapture
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml -j 4
python3 tools/checks/build_budget.py --output target/performance/P078/build.json
python3 -B tools/verify.py plan --contract docs/benchmarks/performance/P078-verification.json
python3 -B tools/verify.py close --contract docs/benchmarks/performance/P078-verification.json --output target/performance/P078/complete --execute
```

Declare the semantic-test and build commands above in the appropriate contract phases;
run `plan` and `close` externally to drive that contract. Bind full transitive
compilation/fixture inputs and require every semantic
case declared in the selection manifest. Its `measure` phase uses
`focused_compare.py --manifest docs/benchmarks/performance/P078-selection.json`
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
python3 tools/wtime-bench/baseline.py --output target/performance/P078-before
python3 -B tools/wtime-bench/selection_manifest.py freeze docs/benchmarks/performance/P078-selection.json target/performance/P078-before/manifest.json target/performance/P078/selection-freeze.json
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
    "/workspace/wtime/target/performance/P078/shared-finalization/attempt-0002/evidence.json": "7b6597ce8aac9259987f3a4baa93a04db789c70931b7ca7ca24909a47fb5d8f4"
  },
  "evidence": [
    {
      "bytes": 2172,
      "path": "/workspace/wtime/target/performance/P078/shared-finalization/attempt-0002/evidence.json",
      "sha256": "7b6597ce8aac9259987f3a4baa93a04db789c70931b7ca7ca24909a47fb5d8f4"
    }
  ],
  "outcome": "inconclusive",
  "status": "done",
  "task": "P078"
}
```

## Completion record

The final candidate close is `inconclusive`, with the task remaining done and no
candidate promoted. The frozen portfolio compared the immediate predecessor
against the candidate using decoded execution. It contained 21 groups (eight
targeted shared-copy rows, four held-out 4097-byte rows, six thread controls,
and three cold controls), with 30 pairs per group in each of two runs. The
1,260 unique pair observations used seeds 78001 and 78002, the selected CPU 1,
worker affinity `[1, 2, 3, 4]`, and one 10,000-resample statistics computation.
Aliases added no samples.

The equal-weight targeted shared-copy candidate/baseline median was 0.871525
with 95% interval [0.857468, 0.890047] in run 1, and 0.874265 with interval
[0.853292, 0.886226] in run 2. Thirteen per-run bounds on small-copy, thread,
and cold-control checks remained inconclusive, so the candidate did not pass
the full gate. No Wasmtime comparison or 100× performance claim is made.

The candidate changed only shared-to-shared copies: same-backing copies used
one lock and `copy_within`, while distinct backings used ordered two-lock
acquisition. Full-range prechecks and cancellation checkpoints remained in
place; other paths were unchanged. The named runtime cases included
`p078_shared_aliases_match_full_memmove_images`,
`p078_opposing_shared_copies_finish_without_deadlock`,
`p078_bulk_prechecks_and_cancellation_preserve_chunks`, and
`p078_copy_avoids_chunk_allocations`. Candidate verification also ran the
bounded driver and protocol cases. Across its declared full configuration, 120
Rust and 54 Python tests ran (174 total), with no failures or skips. The first
full measurement attempt exposed an unsupported wait-row report and was
corrected before the final close. A development check of the smallest
single-worker alias row took 8.29 ms for a 128-group batch, below the 20 ms
target. The new driver's bounded cap was raised to 1024 before full timing.

The frozen selection is SHA-256
`41841d12d26fcab66a6247a7394c2502bde12b3062547f3f9aff5f6a24669d08`; its
freeze receipt is `bc6358333fc1216ec136ea429d2c4c21ac1041478e54c224f8ce8e6207225f46`.
The immediate-predecessor archive manifest is
`e82fe66aaf16fa742b3daa0f95242132dbb5c0ee45af795acd83f4d6ec7f9507`. The
shared-copy fixture identities are WAT
`67f616d4c45c3e722949cc0bc092ba98da299984a6549df17b95392b247c0bc6` and Wasm
`0ab2531cf95cb52c6fac3a249319343d7ef09ade4b618cacb36c0c7bf2af8a1d`. The
candidate binary is `6b65bc5077389a8cfb1490b4d959ca4a19d308d20a16157f026cefc38be0e2e8`.
Its default build passed with zero external dependencies, a 2.764 s clean
build, a 0.1457 s CLI leaf build, a 5.010 s release build, a 1,004,432-byte
stripped binary, and 221,136 KiB peak RSS.

The three runtime files were restored with fresh modification times. The
restoration close matched all 176 archived source entries to the predecessor,
retained no runtime change, and built the restored production binary
`ff6cda780e8e1a42373c3f6a6cd294a86e2b52690cd66e501f62df5230fe6711`. The
restoration verification is contract-only evidence; it did not retime or
promote the candidate. It passed 101 default unit tests, 173 combined-feature
unit tests, eight existing bulk oracles, and three driver cases: 285 test
executions, with no failures or ignored cases.

The benchmark fixture, independent full-memory oracle, driver, retained image
proofs, and focused wait/notify adapter remain available. The candidate
verification contract describes historical candidate sources. Reproducing it
requires restoring its three candidate Rust files from the record archive;
the separate restoration contract verifies the retained checkout.

[Build comparison](../../../wtime/docs/benchmarks/performance/P078-build-comparison.json)
binds the predecessor budget to all 739 current restored package inputs. The
candidate added 3,472 stripped CLI bytes (0.35%). Its unpaired clean/release
build ratios were 1.0724/1.0104 and peak RSS ratio was 1.0108. These recorded
budget observations do not establish a build-speed improvement.

Final published records and detailed receipts:
[completion](../../../wtime/docs/benchmarks/performance/P078-completion.json),
[evidence](../../../wtime/docs/benchmarks/performance/P078-evidence.json),
[freeze](../../../wtime/docs/benchmarks/performance/P078-freeze.json),
[proof](../../../wtime/docs/benchmarks/performance/P078-proof.json),
[statistics](../../../wtime/docs/benchmarks/performance/P078-statistics.json),
[restoration](../../../wtime/docs/benchmarks/performance/P078-restoration.json), and
[record archive](../../../wtime/docs/benchmarks/performance/P078-records.json.gz). The archive
retains 929 files in 33,763,782 compressed bytes; its SHA-256 is
`52b429319975a043150e1fb21aafb93e56661f1f370b6a0aedc96d236f331c14`.
The candidate close
is `target/performance/P078/shared-finalization/attempt-0002`; restoration is
`target/performance/P078/restoration-finalization/attempt-0001`. Their evidence
SHA-256 values are respectively
`7b6597ce8aac9259987f3a4baa93a04db789c70931b7ca7ca24909a47fb5d8f4` and
`954ab266ef3974b74746bcc96dc53d4e6750ddd0fe99135d951c3a9d13fe6d51`.
