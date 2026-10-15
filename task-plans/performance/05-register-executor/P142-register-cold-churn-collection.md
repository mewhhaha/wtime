# P142 — Collect and analyze source-bound Register cold and churn sequences

Status: **done**

Outcome: **contract_only**

Kind: `foundation` · Branch: `portable` · Model: **Sol 6.1 Low**

## Purpose and dependencies

P142 adds a source-bound, two-pair-per-row foundation smoke for the P141 individual-entry sequence protocol. It reuses P136's adapter build and provenance machinery, P139's source-bound side identity, and P141's deterministic uniform66 schedule. The fixed rows use schedule thresholds `N=3` and `N=10`, each comparing eager Register threshold zero with adaptive threshold `N`. The collector preserves independent cold/churn process pairs and the analyzer reports descriptive per-process event-class summaries. This smoke does not establish timing evidence, select a threshold, or satisfy a P066 gate.

P142 depends on **P139** and **P141**. P141 supplies the strict timing and diagnostic protocols; P139 supplies source-bound collection conventions. P066 depends on P142 and retains every existing semantic, focused-stage, protected-control, build/dependency, cold/churn, statistical, and promotion gate.

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [MEASUREMENT.md](../MEASUREMENT.md), [P136 paired collection](P136-register-entry-paired-collection.md), [P137 analysis](P137-entry-total-paired-analysis.md), [P138 cache diagnostics](P138-register-cache-diagnostics.md), [P139 side settings](P139-side-specific-register-entry-settings.md), and [P141 sequence driver](P141-register-entry-sequence-driver.md). Inspect the current schemas and validators before adding the P142 sequence-specific schema; do not change or reinterpret existing report formats.

- `tools/wtime-bench/entry_sequence_compare.py` — strict source-bound collector and resumable checkpoint.
- `tools/wtime-bench/entry_sequence_analyze.py` — descriptive event-class analyzer.
- `tools/wtime-bench/tests/test_p142_sequence_collection.py` — collector tests.
- `tools/wtime-bench/tests/test_p142_sequence_analysis.py` — analyzer tests.
- `tools/verification/producers/p142_freeze.py`, `tools/verification/producers/p142_reuse_builds.py`, and `tools/verification/producers/p142.py` — freeze, retained-build revalidation, and read-only proof producers.
- `docs/benchmarks/performance/P142-contract.json` — acceptance contract.
- `docs/benchmarks/performance/P142-verification.json` — four-phase close contract.

Reuse P136's archive verification, isolated adapter workspace, executable discovery, and build-record validation without copying or mutating source inputs. Keep runtime/cache policy, P141 timing semantics, P136/P137/P139 schemas, and P066 selection and promotion logic unchanged. The P142 producer and analyzer consume and validate the two-pair smoke only; later full-sample collection is outside this foundation close.

## Input/output contract

Freeze the immediate P141 predecessor and complete identities for the P138 core, P141 harness/driver, uniform66 fixture, P139/P141 prerequisite proofs, and freeze helpers. Pin the predecessor commit and manifest, and require the P138 build budgets to validate and be reused because core source is unchanged. Freeze exactly two rows (`N=3`, `N=10`) and seeds `142001`, `142002`.

Retain the successful eager baseline and adaptive candidate adapter builds from verify attempt `attempt-0001`, produced with P136's isolated Register adapter machinery and the same P141 protocol/driver overlay. The final verify phase must source-current revalidate both immutable adapters, their original build commands and environment, and their separate binary/source identities without compiling or launching a guest. Measure consumes those same retained adapter directories directly; reject aliasing or drift before collection and recheck source stability afterward.

Collect exactly two complete process pairs per row over the two distinct seeds (eight pairs total). Each process is a fresh engine/module/store lifetime. Randomize eager/adaptive order deterministically within each pair. Preserve actual argv, command stdout/stderr, ordered raw per-invocation records, preparation/instantiation observations, fixture/source/binary identities, and process results. Reject missing, extra or reordered calls, body/visit/phase mismatches, bad oracle results, invalid timings, incomplete/duplicate pairs, mixed identities, and source/settings drift on resume.

The timed rows use P141's uninstrumented timing protocol. Validate the separate no-timer diagnostic traces independently and join event classes to timed records only by exact ordered `(body,visit,phase)` keys. Keep diagnostic instrumentation and its wires out of timing records. The fixed schedule exercises cold, compile, ready, FIFO eviction, and `f00` recompile behavior.

Analyze each run, row, side, and validated event class independently. Reduce each process's calls to p50, p99, and maximum before pairing; never pool calls as independent samples, combine seeds or rows, divide by calls, or mix preparation/instantiation with entry latency. Because this is a two-pair smoke, paired ratios and confidence intervals remain null and no bootstrap uncertainty is manufactured. Report `timing_evidence:false`, `gate_eligible:false`, `candidate_promoted:false`, `decision:null`, and `threshold_selection:null`. This foundation smoke cannot satisfy the full P066 cold/churn evidence requirement.

## Implementation recipe

1. Validate the completed P139/P141 prerequisite envelopes and P138 fixture provenance. Freeze the exact P142 predecessor, rows and source identities before building.
2. Revalidate the successful P136-based baseline and current-candidate adapters retained from verify attempt `attempt-0001`; preserve their distinct source and executable identities and common P141 overlay without rebuilding during final verification.
3. Run the collector with two seeds and two process pairs per row. Keep raw commands, outputs and checkpoint records; validate all retained pairs and source identities before resuming.
4. Validate the separate P141 diagnostic class map and exact call-key join; keep instrumented diagnostic records separate from timed observations.
5. Run the descriptive analyzer on the source-bound smoke. Retain per-process event summaries and null inferential fields without manufacturing confidence intervals or a decision.
6. Verify with focused collector, identity and analyzer tests. Use only temporary synthetic fixtures in tests and ensure every test output remains within its fresh temporary root.
7. Close the task with the two-pair smoke explicitly marked non-timing and nonpromoting. Preserve all phase receipts and raw logs; do not claim a threshold recommendation, performance result, or P066 gate completion.

## Meaningful cases

- `test_smoke_collects_contained_source_bound_pairs_and_replays`: collect and replay the fixed two-row, two-seed, two-pair smoke while rejecting paths outside the safe collection root.
- `test_failure_preserves_first_side_and_resume_uses_new_attempts`: retain the first side after failure and resume only with new attempts after validating checkpoint and identities.
- `test_report_rejects_wire_order_and_raw_record_corruption`: reject reordered event records and mutation of retained raw wire data.
- `test_rows_runs_classes_and_ci_are_separate_and_deterministic`: preserve run/row/event-class separation and stable descriptive analysis.
- `test_smoke_requires_opt_in_and_has_descriptive_null_uncertainty`: require explicit smoke analysis opt-in and keep inferential values null.
- `test_bad_sample_count_payload_shape_and_stale_fixture_are_rejected`: reject bad sample shapes, changed fixtures, and invalid smoke counts.
- `test_analysis_changes_are_visible_but_do_not_stale_legacy_pairs`: ensure source identities capture the intended analysis boundary without invalidating unrelated legacy pairs.
- `test_adapter_archive_corruption_is_still_rejected`: reject corrupted predecessor adapter archives.

## Verification

The verification contract has four stages: freeze, verify, measure, and analyze. Verify runs the four collector tests plus seven execution-identity tests, then uses `p142_reuse_builds.py` to verify the successful baseline and candidate adapter builds from retained attempt `attempt-0001`; this revalidation launches no build or guest. Measure consumes those exact adapter directories for the source-bound two-pair smoke. Analyze runs four analyzer tests, emits descriptive analysis, and produces a read-only proof. No stage replays a full-sample measurement or promotes a candidate.

```sh
python3 -B tools/verify.py plan --contract docs/benchmarks/performance/P142-verification.json
python3 -B tools/verify.py close --contract docs/benchmarks/performance/P142-verification.json --output target/performance/P142/complete --execute
```

## Measurement and decision

Selection: `register`. Execution mode: `register`. Decision rule: `integrity`.

P142 produces source-bound, descriptive cold/churn smoke observations only. It does not establish timing evidence or a candidate decision; the two threshold rows remain distinct and promotion is false. P066 retains all existing semantic, focused-stage, protected-control, build/dependency, cold/churn, statistical, and promotion gates.

## Completion evidence

Record the P139/P141 proof identities, pinned predecessor and fixture provenance, source/binary/toolchain identities, two row thresholds, two seeds, eight complete pairs, 16 processes, 8,040 individual calls, and 24 event-class groups. Preserve actual argv, raw process outputs, diagnostic class mapping, analysis helper hashes, named test counts and source-current phase receipts. State that core build budgets were revalidated and reused, no new core dependency was introduced, and the smoke has no timing or promotion claim.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/P142/shared-finalization/attempt-0002/evidence.json": "2bde06b821108ce9b2e20c517cc37a7f6c068a5199091f7e04f490b166a3c60e"
  },
  "evidence": [
    {
      "bytes": 2601,
      "path": "/workspace/wtime/target/performance/P142/shared-finalization/attempt-0002/evidence.json",
      "sha256": "2bde06b821108ce9b2e20c517cc37a7f6c068a5199091f7e04f490b166a3c60e"
    }
  ],
  "outcome": "contract_only",
  "status": "done",
  "task": "P142"
}
```


P142 completed as a source-current, nonpromoting foundation smoke. The successful shared close is `target/performance/P142/shared-finalization/attempt-0002/completion.json`; its evidence SHA-256 is `2bde06b821108ce9b2e20c517cc37a7f6c068a5199091f7e04f490b166a3c60e`. Freeze and verify completed in attempt `0002`; measure and analyze completed in attempt `0001`. Verification passed 11 collector/identity Python tests and revalidated the successful baseline and candidate adapters retained from verify attempt `0001`, without rebuilding them. The first close attempt was retained; its only blocker was a stale required-test-name declaration after all 11 tests and both original adapter builds succeeded.

The smoke collected eight complete pairs over two runs and two rows: 16 processes, 8,040 individual calls, and 24 event-class groups. The P138 uniform66 fixture and the P138 default/Register build budgets were revalidated and reused across all 738 source inputs; core source remained unchanged and no core dependency was added. Four analyzer tests passed. The proof reports `timing_evidence:false`, `gate_eligible:false`, `candidate_promoted:false`, null decision and threshold selection, with no statistics recomputation by the final proof. The post-measurement proof/export checks launched no build or guest and performed no retiming.

Publication `P142-publication-0001` succeeded. The exported records archive `docs/benchmarks/performance/P142-records.json.gz` contains 1,408 retained members, is 5,839,446 bytes, and has SHA-256 `9fa75904e964fe27dbe48c5b9f2a97ecf9f4bb1b11594987b61f42d353b32bd4`. Exporter attempts `0001` and `0002` failed before documentation writes on strict evidence-shape checks; both drafts and logs are retained. Export attempt `0003` passed and produced the completion, evidence, freeze, proof, and analysis JSON exports.
