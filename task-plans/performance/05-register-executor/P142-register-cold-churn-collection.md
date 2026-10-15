# P142 — Collect and analyze source-bound Register cold and churn sequences

Status: **todo**

Outcome: **pending**

Kind: `foundation` · Branch: `portable` · Model: **Sol 6.1 Low**

## Purpose and dependencies

P142 builds the source-bound paired collector and descriptive analyzer for P141's individual-entry sequence protocol. It reuses P136's verified adapter build and provenance machinery, P139's side-specific setting discipline, and P141's deterministic uniform66 schedule. Each pair compares eager Register threshold zero with adaptive Register threshold `N` for rows `N=3` and `N=10`. Results characterize cold, compile, ready, eviction and recompile behavior for P066; they do not choose a threshold or relax any P066 gate.

P142 depends on **P139** and **P141**. P141 supplies the strict timing and diagnostic protocols; P139 supplies source-bound side identity and collection conventions. P066 depends on P142 and retains its existing semantic, focused-stage, protected-control, build/dependency, cold/churn, statistical and promotion gates.

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [MEASUREMENT.md](../MEASUREMENT.md), [P136 paired collection](P136-register-entry-paired-collection.md), [P137 analysis](P137-entry-total-paired-analysis.md), [P138 cache diagnostics](P138-register-cache-diagnostics.md), [P139 side settings](P139-side-specific-register-entry-settings.md), and [P141 sequence driver](P141-register-entry-sequence-driver.md). Inspect their current schemas and validators before adding the new sequence-specific schema; do not change or reinterpret their existing report formats.

- `tools/wtime-bench/entry_sequence_compare.py` (new strict collector and resumable checkpoint)
- `tools/wtime-bench/entry_sequence_analyze.py` (new event-class analyzer)
- `tools/wtime-bench/tests/test_p142_entry_sequence.py`
- `tools/verification/producers/p142.py`
- `tools/verification/tests/test_p142_entry_sequence.py`
- `docs/benchmarks/performance/P142-contract.json`
- `docs/benchmarks/performance/P142-verification.json`
- `docs/benchmarks/performance/P142-selection.json`

The collector reuses P136 archive verification, workspace construction, executable discovery and build-record validation without copying or mutating its source inputs. One tooling integrator owns the P142 producer and report schema. Do not edit runtime/cache policy, P141 timing semantics, P136/P137/P139 schemas, or P066 selection and promotion logic.

## Input/output contract

Freeze the immediate predecessor and a complete source identity for the P141 driver, current Rust harness, runtime, fixture/provenance, collector, analyzer, producer, contracts, selection, feature configuration and toolchain. Build separate eager baseline and adaptive candidate executables with P136's isolated Register adapter machinery. Both adapters must use the same P141 protocol/driver and common overlay bytes. Bind archived and current source maps, commands, lockfiles, toolchain, binaries, fixture, contract and settings independently; reject aliasing, drift or changed identities before guest execution and recheck source stability after collection.

Use a new P142 schema and crash-safe journal; never reinterpret or modify P136/P139 v1/v2 records. The row set is the fixed P138 `uniform66` fixture, one input, schedule threshold `N` in `{3,10}`, and eager/adaptive sides. Eager always uses Register threshold zero; adaptive uses the row's positive threshold `N`. For each side observation, invoke the P141 `timing` protocol and preserve its actual argv, command stdout/stderr, ordered raw response, preparation/instantiation observations, fixture/source/binary identities, and process result. One process is one fresh engine/module/store lifetime and one independent observation; never batch or warm invocations.

Run two independent runs with distinct recorded seeds and at least 30 complete process pairs per row in each run. Randomize eager/adaptive process order deterministically per pair. Each raw sequence contains the complete schedule and one elapsed-nanosecond value per invocation. Reject missing/extra/reordered calls, body/visit/phase mismatch, bad oracle output, malformed or negative timings, incomplete/duplicate pairs, mixed source or binary identities, and changed settings before or during resume.

Run P141's separate no-timer diagnostic schedule to verify the actual event-class labels for each row. Bind that diagnostic report and class map independently. Match classes to timed observations by the exact ordered `(body,visit,phase)` key; do not infer labels from latency. Validate classes cold, compile, ready and recompile, including FIFO eviction and `f00` recompile. Keep instrumented diagnostics outside all timing records and analysis.

For each independent run, row and validated event class, reduce each process's individual invoke latencies to p50, p99 and maximum before pairing. Compute paired candidate/eager log-median ratios and deterministic 95% bootstrap intervals using 10,000 resamples over independent process pairs. Never pool calls as independent samples, divide by call count, combine run seeds, aggregate thresholds, or mix preparation/instantiation with entry latency. Preserve preparation and instantiation measurements separately as descriptive fields. Full collection may be timing evidence, but it is descriptive only: set `decision:null`, `gate_eligible:false`, and `candidate_promoted:false`; do not make a speed or threshold recommendation. A two-pair-per-row smoke is allowed only as foundation smoke and must set `timing_evidence:false`; it cannot satisfy the full source-bound two-run analysis or any P066 gate.

## Implementation recipe

1. Verify the completed P139 and P141 receipts and the P138 fixture/provenance. Freeze the P142 row/settings manifest and immediate predecessor before building; use a new immutable archive location and do not overwrite earlier task archives.
2. Reuse P136's archive verification, build-record validation and isolated adapter workspace. Overlay identical P141 protocol and leaf-driver bytes on both baseline and candidate adapters. Prove separate executable/source identities and the expected eager-zero versus adaptive-`N` settings before starting a guest.
3. Implement a strict P142 collector with exact two-run row declarations, deterministic process-order seeds, independent process pairs and a crash-safe checkpoint. Log every actual argv, stdout/stderr and raw per-invocation record. On resume, validate all prior pair bytes and revalidate source, executable and fixture identities before new processes.
4. Add a separate P141 diagnostic validation step. Bind its actual body/visit/phase labels and observed cache lifecycle transitions to each row. Join those labels to timing records only by exact call identity; reject class mismatch, omitted observations, unexpected extra calls and any diagnostic timing fields.
5. Implement the analyzer by run, row and event class. First summarize each process's calls to p50, p99 and max; then compute paired log medians and deterministic 10,000-resample intervals over independent process pairs. Keep each run and threshold row separate and keep preparation/instantiation summaries outside entry metrics. Emit no candidate decision or recommendation.
6. Add focused synthetic and CLI tests for schema dispatch, source binding, resume integrity, exact schedule joins, event-class validation, per-process reduction, deterministic bootstrap, and rejection of timing/diagnostic mixing. Synthetic inputs must use a fresh temporary root and prove every resolved output remains inside it before writing.
7. Full close performs two independent runs with at least 30 process pairs per row and a separate two-pair smoke solely for the foundation smoke path. Preserve all raw commands/outputs, helpers, checkpoints, diagnostic traces, analyses and receipts. Do not claim promotion or treat the smoke as measurement evidence.

## Meaningful cases

- `p142_reuses_p136_adapters_with_one_common_p141_overlay`: require the same P141 driver, protocol, fixture and toolchain overlay on distinct archived/current source identities; reject changed overlays, executable aliases, stale source maps and non-Register feature sets.
- `p142_collector_keeps_two_seeded_runs_and_complete_pairs`: require two distinct run seeds, rows 3 and 10, at least 30 complete randomized eager/adaptive process pairs per row/run, exact ordered schedules, raw output preservation and safe checkpoint resume.
- `p142_diagnostic_class_map_must_match_every_timed_call`: verify classes from the separate untimed diagnostic, join only by `(body,visit,phase)`, and reject missing, reordered, unexpected or misclassified calls and timer fields in diagnostics.
- `p142_analyzer_reduces_calls_per_process_before_pairing`: known synthetic sequences produce expected per-process p50/p99/max, per-run/per-row/per-class paired log medians, and deterministic 10,000-resample intervals without pooling calls or runs.
- `p142_rejects_stale_mixed_or_incomplete_source_bound_inputs`: reject archive/contract/helper/fixture/binary mutation, settings drift, duplicate or incomplete pairs, invalid timings, wrong oracle values, foreign paths and source changes during collection or analysis.
- `p142_smoke_is_nonpromoting_and_cannot_satisfy_full_matrix`: accept only the explicit two-pair foundation smoke path, mark it non-timing and ineligible, and reject it as full analysis or P066 evidence.

## Verification

Use the P142 verification contract for exact feature groups, source identities, build records, two full runs, smoke, diagnostic verification and one source-bound analysis. The collector tests, analyzer tests and producer tests must all execute their named cases. Preserve command outputs and helper sources. The full analysis must see at least 30 process pairs for every row in each of two runs; no development or smoke receipt substitutes for these phases.

```sh
python3 -B tools/verify.py plan --contract docs/benchmarks/performance/P142-verification.json
python3 -B tools/verify.py close --contract docs/benchmarks/performance/P142-verification.json --output target/performance/P142/complete --execute
```

## Measurement and decision

Selection: `register`. Execution mode: `register`. Decision rule: `integrity`.

P142 provides source-bound, descriptive cold/churn timing and diagnostic labels. Full measurements are not a candidate promotion; threshold rows remain distinct, decision is null, and gate eligibility and promotion are false. The two-pair smoke is non-timing foundation evidence only. P066 remains responsible for candidate selection and retains all prior semantic, focused-stage, protected-control, build/dependency, cold/churn, statistical and promotion gates.

## Completion evidence

Record P139/P141 prerequisite identities, the pinned predecessor and common P141 overlay, exact source/binary/fixture/toolchain hashes, two independent run seeds, row and pair counts, actual argv and raw process outputs, verified diagnostic class mapping, per-process p50/p99/max, deterministic paired bootstrap settings, named test counts, and source-current receipts. Report the smoke and full descriptive analysis separately; include no threshold recommendation or candidate decision.
