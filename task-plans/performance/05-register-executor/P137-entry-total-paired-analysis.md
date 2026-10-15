# P137 — Analyze paired Register entry totals and phases

Status: **done**

Outcome: **contract_only**

Kind: `foundation` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P136 — Bind Register entry settings to source-bound paired collection](P136-register-entry-paired-collection.md)

P136 provides source-bound pairs of fresh Register entry lifetimes, with raw whole-lifetime and phase nanoseconds. P137 adds reproducible summaries of those raw paired observations. It makes no threshold, workload-selection, break-even, or promotion decision; P066 retains the complete candidate gate.

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [MEASUREMENT.md](../MEASUREMENT.md), [fast-performance.md](../../../wtime/docs/verification/fast-performance.md), and [the P136 pair contract](../../../wtime/docs/architecture/register-entry-pairs.md). Inspect the current `entry_batch_compare.py`, `analyze.py`, `bootstrap.py`, `statistics.py`, P136 producer and receipts before implementation.

- `tools/wtime-bench/entry_batch_analyze.py` (new API and CLI)
- `tools/wtime-bench/tests/test_p137_entry_analysis.py` (new focused analyzer tests)
- `tools/verification/producers/p137.py` (new source-bound analysis producer)
- `tools/verification/tests/test_p137_entry_analysis.py` (new producer and receipt tests)
- `docs/benchmarks/performance/P137-contract.json` (short declarative acceptance contract)
- `docs/benchmarks/performance/P137-verification.json` (verification workflow, development groups and evidence receipt)

Keep this task limited to raw P136 total/phase analysis, source-bound analysis receipts and tests. Do not change P136 collection, runtime/core code, timer behavior, fixtures, workload selection, candidate thresholds, or cache diagnostics. P066 still requires separate evidence for cold-code behavior and eviction/recompile churn; P137's raw timing summaries do not satisfy those requirements.

## Input/output contract

Expose:

```python
analyze(report: dict, collection_root: Path, allow_smoke: bool = False) -> dict
```

Require the P136 report to pass `entry_batch_compare.validate_report` against its collection root, including all untouched raw side records, report identity digest, pair joins, settings, fixtures, executables, and source/build records. Recheck referenced build records, executable hashes, fixture hashes, helper identities, and report/identity hashes before and after analysis. The producer and CLI also run P136's full source-current `verify_collection` wrapper against the recorded archive, adapter, candidate and harness inputs; never launch either executable or guest during analysis.

Production analysis requires `timing_evidence:true`, distinct archived/current source identities, exactly two independent runs with distinct seeds, the complete declared row matrix, and at least 30 complete pairs for every row in each run. Reject same-source aliases and smoke collections as production evidence. Reject missing, duplicate, foreign, stale, mixed-setting or malformed observations. Input timing samples must be non-boolean nonnegative integers no greater than the Rust `u128` range. Full analysis requires each `total_ns` to be positive because it is used in a logarithmic ratio; phase values may be zero. Smoke may contain zero totals and phases because it computes no ratios or confidence intervals.

For every run and each distinct row, retain baseline and candidate summaries for `total_ns` and `prepare_ns`, `instantiate_ns`, and `invoke_ns`: median, MAD, p25, p75, minimum, maximum, and p99, all with unit `ns`. Raw samples remain integer nanoseconds; interpolated summaries can be fractional nanoseconds. For `total_ns`, calculate the point estimate as `exp(median(log(candidate_total_ns / baseline_total_ns)))`, not the arithmetic median of raw ratios. Compute its deterministic 95% percentile interval by exponentiating the 2.5th and 97.5th percentiles of 10,000 resampled paired-log-ratio medians. Reuse `analyze._summary`, `analyze._bootstrap_interval`, and `analyze._seed_for`; bind the bootstrap seed to the run seed and exact row identity. Report each run independently. Never pool call counts, strategies, thresholds, rows, or the two runs. Phase summaries are descriptive only; do not bootstrap phase ratios unless separately specified by a later contract.

Do not divide by calls, calculate ns/op, infer tail latency from a batch, compute a break-even point, recommend a threshold, or issue a candidate decision. Set decision and promotion fields to null/false. Retain exact row settings (call count, strategy, threshold), raw metric name `entry_total`, raw nanosecond unit, method, confidence level, resample count, pair count, run seed, input identities and source bindings in the output.

An explicit `--allow-smoke` path is analysis-only and cannot satisfy candidate evidence. It accepts the P136 two-pair-per-row smoke only after receipt verification and source stability checks, labels `timing_evidence:false` and `gate_eligible:false`, and emits descriptive raw summaries only. Its ratio and CI fields are null. Smoke descriptives may include zero timings, but never compute logarithms or bootstrap intervals for them. Without `--allow-smoke`, refuse smoke input.

The analysis record binds the P136 report content hash and identity digest, all referenced inputs and helpers, analysis settings, and recomputable result. Verify those identities again after analysis; write only to a new exclusive output path. Re-analysis of identical inputs must produce the same statistics and record content apart from explicitly excluded creation metadata.

## Implementation recipe

1. Add `entry_batch_analyze.py` with the typed API and CLI. Validate the report through P136's collector validator; verify the complete source/build/executable/fixture/helper closure; then analyze raw totals/phases without launching Wasm. Reuse the existing deterministic summary and median-bootstrap primitives rather than adding a second statistical implementation.
2. Separate full candidate-compatible evidence from smoke descriptives. Enforce the two-run complete matrix and >=30 complete pairs per row/run, positive full-analysis timings, unique row/run joins, and source/settings identity. For each run and row, summarize each side in raw ns; compute paired total ratios and a separate deterministic 10,000-resample CI. Do not calculate an aggregate across runs or policy settings.
3. Add `p137.py` to verify the consumed P136 completion/proof and collection identities, call the P136 source-current collection verifier without subprocess timing, invoke the analyzer once, and write a source-bound receipt. Freeze the P136 smoke collection and immutable completion/proof in the P137 contract.
4. Add focused tests for full and smoke paths, exact bootstrap output, raw units, row/run separation, failure modes, stale references and deterministic re-analysis. Synthetic reports and all resolved outputs must stay inside a fresh temporary root; the analysis-only verification must reuse P136's retained real 32-pair smoke collection after producer verification, with no new Rust build, archive, timing, or guest execution.
5. Run both named P137 test suites and the canonical contract plan/close checks when the producer and contract are ready. Keep P137 status pending until the source-bound analysis receipt and all required checks pass.

## Meaningful cases

- `p137_full_pair_uses_raw_total_and_known_median_bootstrap_ci`: a complete paired fixture with known raw totals produces the expected candidate/baseline median and exact deterministic 95% interval from 10,000 draws.
- `p137_runs_call_counts_and_policy_rows_remain_independent`: two distinct run seeds and rows spanning call counts/strategies/thresholds produce separate results without pooling observations or settings.
- `p137_reports_raw_phase_summaries_without_division`: total and each phase retain ns median/MAD/p25/p75/min/max/p99; output contains no divided metric, threshold recommendation, break-even, or decision.
- `p137_rejects_incomplete_stale_or_invalid_full_evidence`: reject missing/duplicate pairs, changed report/source/build/executable/fixture/helper hashes, wrong settings, same-source alias, fewer than 30 pairs, booleans, and nonpositive full timing values.
- `p137_smoke_requires_opt_in_and_never_claims_timing_evidence`: refuse smoke by default; with explicit opt-in, accept only verified P136 smoke, return descriptives with null ratios/CIs and false timing/gate eligibility, including when raw totals or phases are zero.
- `p137_bootstrap_and_receipt_are_deterministic_without_guest_execution`: repeated analysis of the same verified report yields identical statistics and receipt inputs and does not launch a benchmark subprocess.
- `p137_rejects_mutation_during_analysis`: changing a bound helper or report identity during analysis is detected by the post-analysis stability check.

## Verification

P137's verification is analysis-only. Reuse the completed P136 immutable archive, adapter/candidate build records, collection, and source-bound completion/proof; verify them before analysis. Do not rebuild Rust, run Wasm, or collect timing for this task. Its retained P136 smoke is explicitly non-promoting. Full 30-pair behavior is verified with contained synthetic reports, not timing evidence.

```sh
python3 -B tools/wtime-bench/tests/test_p137_entry_analysis.py -v
python3 -B tools/verification/tests/test_p137_entry_analysis.py -v
python3 -B tools/verification/producers/p137.py freeze --output target/verification/P137-freeze
python3 -B tools/verification/producers/p137.py verify --freeze target/verification/P137-freeze/report.json --output target/verification/P137-verify
python3 -B tools/verification/producers/p137.py check-evidence --report target/verification/P137-verify/report.json
python3 -B tools/wtime-bench/entry_batch_analyze.py --comparison target/verification/runs/P136/verify/attempt-0001/proof/collection --allow-smoke --output target/verification/P137-example-analysis
python3 -B tools/verify.py plan --contract docs/benchmarks/performance/P137-verification.json
python3 -B tools/verify.py close --contract docs/benchmarks/performance/P137-verification.json --output target/performance/P137/complete --execute
```

## Measurement and decision

Selection: `register`. Explicit execution setting: `register`. Decision rule: `integrity`.

P137 summarizes source-bound raw paired entry totals and phases. It does not choose rows or thresholds, establish a speedup, or promote a candidate. P066 retains every prior semantic, build/dependency, mandatory control, cold-code/churn, selection, and two-run measured-promotion gate. In particular, the 1,000-function `prepare_large` compile control does not establish function-entry distributions or eviction/recompile behavior when its non-entry functions are uncalled; P066 must retain separate suitable cold/churn evidence with instrumentation excluded from production timing.

## Completion evidence

Record the P136 completion, proof, report, identity and collection hashes; archived/current source and executable identities; fixture and helper hashes; P137 contract and producer hashes; output hash; run/row/pair counts; exact bootstrap settings; named test counts; and explicit `timing_evidence:false` for smoke-only analysis. Keep status pending until all cases, validators and the source-bound analysis-only verification pass. Do not claim a performance decision.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/P137/shared-finalization/attempt-0002/evidence.json": "e9052b67826b07d012472c97a83c05ed37ca7dbf2348e74480377c1c709b601d"
  },
  "evidence": [
    {
      "bytes": 1535,
      "path": "/workspace/wtime/target/performance/P137/shared-finalization/attempt-0002/evidence.json",
      "sha256": "e9052b67826b07d012472c97a83c05ed37ca7dbf2348e74480377c1c709b601d"
    }
  ],
  "outcome": "contract_only",
  "status": "done",
  "task": "P137"
}
```

The full close passed at
`target/performance/P137/shared-finalization/attempt-0002`, using verify attempt
`0002`. All **14** checks passed: eight analyzer and six source-bound producer
checks; none failed or were ignored. The earlier attempt's analysis and tests
passed, but receipt packaging declared a directory instead of individual helper
files. That failed attempt is preserved; the contract now binds all three files.

Verification reused the exact P136 collection: **two runs, eight rows and 32
pairs**, with no Rust build or Wasm execution. All **732** current workspace
inputs remain unchanged, with source-map SHA-256
`90d7833af9e65339493d1b963cc763b5e68aab93a5c49fb8ed71c974b1bafd86`.
The analysis output SHA-256 is
`55bc9e2bdfc0aa00f5bdb842ecfc9ca0407bec1873b46d38a1f5e063bc6a7290`;
finalizer evidence SHA-256 is
`e9052b67826b07d012472c97a83c05ed37ca7dbf2348e74480377c1c709b601d`.

Outcome: **contract_only**. Real smoke analysis contains raw total/phase
descriptives, null ratios and intervals, zero bootstrap draws, no decision or
threshold selection, and false timing/gate eligibility. Complete production
uncertainty uses the existing 10,000-draw bootstrap and was tested using
contained synthetic pairs, not promoted timing evidence. Existing default and
Register build-budget evidence remains source-current; default stripped size
is 1,000,960 bytes, Register 1,090,512 bytes, with zero external core dependencies.

See [the architecture note](../../../wtime/docs/architecture/register-entry-analysis.md),
[completion](../../../wtime/docs/benchmarks/performance/P137-completion.json),
[analysis](../../../wtime/docs/benchmarks/performance/P137-analysis.json) and
[records](../../../wtime/docs/benchmarks/performance/P137-records.json.gz).
P066 retains every semantic, protected-control, cold-code/cache-churn,
build/dependency and measured-promotion gate.
