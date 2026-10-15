# P139 — Collect side-specific Register entry settings

Status: **done**

Outcome: **contract_only**

Kind: `foundation` · Branch: `portable` · Model: **Sol 6.1 Low**

## Purpose and dependencies

P136 provides source-bound paired collection of fresh Register entry lifetimes, but currently applies one strategy and threshold to both sides of a pair. P137 analyzes each complete run and row without pooling or making a decision. P138 supplies separate, instrumented cache lifecycle diagnostics. P139 extends the existing collection and analysis formats so the archived baseline can use eager Register at threshold zero while the current candidate uses adaptive Register at explicit positive thresholds. P139 changes only benchmark/verification tooling and adds no Rust, runtime, fixture, timer, or cache-policy changes.

P139 depends on **P136, P137, and P138**. P066 gains P139 as a prerequisite while retaining every existing semantic, build/dependency, workload selection, focused-stage, mandatory-control, cold/churn, statistical, and promotion gate. P139 smoke and synthetic evidence do not satisfy any P066 gate.

## Scope and ownership

- `tools/wtime-bench/entry_batch_compare.py` — extend collection schema and side-specific argv/payload validation; preserve reading and verification of collected v1 reports.
- `tools/wtime-bench/entry_batch_analyze.py` — analyze v1 and v2 reports while retaining each side's strategy and threshold.
- `tools/wtime-bench/tests/test_p139_entry_settings.py` — focused collector/analyzer tests.
- `tools/verification/producers/p139.py` — source-bound freeze, smoke verification, and evidence checking.
- `tools/verification/tests/test_p139_entry_batch.py` — producer, source-binding and receipt tests.
- `docs/benchmarks/performance/P139-contract.json` — declarative acceptance and completion contract.
- `docs/benchmarks/performance/P139-verification.json` — source-bound workflow, development groups, and full-close phases.

Root owns edits to the two existing `entry_batch_*.py` files. Keep runtime and `tools/wtime-bench/src` unchanged. Preserve P136/P137 readers and receipt validation for immutable v1 outputs.

## Input/output contract

The collector accepts rows whose caller-supplied v2 objects have exactly these fields:

```json
{
  "fixture": "...",
  "case": "p135_counter",
  "input": 7,
  "expected": 8,
  "calls": 100,
  "baseline_strategy": "eager",
  "baseline_threshold": 0,
  "candidate_strategy": "adaptive",
  "candidate_threshold": 3
}
```

The existing limits still apply: `calls` is one of 1, 10, 100, or 10000; eager requires threshold zero; adaptive requires a positive threshold; inputs/results are valid i32 values. The stored row may add only the established derived identity fields (stable `row_id`, fixture hash, mode, profile, metric and fuel setting). Its user-specified semantic and policy fields remain exactly the nine above. Duplicate rows are rejected using the complete fixture/case/input/oracle/count/both-side-settings identity.

Emit `wtime-register-entry-pairs-v2` reports for new collections. Each baseline and candidate command must be made from the matching fields in that row. The baseline adapter remains the P136 archived-source adapter, and the candidate remains a separately built current-workspace Register executable. Both build records, executable file hashes, fixture, protocol/driver/leaf closure, manifest and lock, toolchain, mode/profile, instrumentation status, seeds, and common command inputs stay source-bound as before. The report records the actual per-side argv and settings. Validation requires each side's payload to repeat its own configured strategy and threshold; it does not require the two sides' policy fields to match. The other shared fields (fixture bytes, case, input, oracle, call count, mode, profile, fuel, toolchain, and protocol) must still join exactly.

Continue to read and validate `wtime-register-entry-pairs-v1` without rewriting or relabeling it. For v1, `strategy` and `threshold` retain their existing shared-side meaning; existing report hashes, raw records, command arrays, and checkpoints are validated under the original schema. V2 has its own schema and identity hash. Resume is allowed only when schema, row settings, two build/source records, executable file identities, seeds, and helper hashes exactly match the original journal header. A changed threshold or strategy creates a new collection; it cannot resume or append to old pairs.

P137's analysis API keeps both versions readable. For v1 rows, retain the shared legacy settings as the row settings and side effective settings. For v2, preserve `baseline_strategy`, `baseline_threshold`, `candidate_strategy`, and `candidate_threshold` in every analysis row and bind them into deterministic bootstrap seeds. Continue reporting each run and row separately; do not pool runs, call counts, settings, or fixtures. Use only raw `total_ns` and existing phase totals in raw nanoseconds, and preserve P137's paired-log-median estimate and 10,000-resample percentile CI for full data. Smoke summaries remain descriptive, with null ratio/CI, `timing_evidence:false`, `gate_eligible:false`, and `decision:null`. No per-call division, threshold recommendation, break-even estimate, or promotion decision is added.

The source-bound producer verifies immutable P136, P137, and P138 completion/proof references, then validates new evidence with the current P139-aware P136 collector and P137 analyzer. It does not call P136/P137 historical public source-current checkers, because P139 changes their helper sources. It revalidates P139 source maps, fixture/helper hashes, archive manifests and report/journal contents before and after analysis. Evidence checking is read-only and never starts benchmark binaries or invokes guest code. Collection does not calibrate, warm, preflight, or divide timing samples.

## Implementation recipe

1. Inspect the P136 collector/analyzer APIs and their immutable v1 report, checkpoint, and P137 smoke analysis. Add schema dispatch at the report boundary: strict v1 normalization/validation remains unchanged; v2 accepts only the exact caller row fields above and writes a distinct v2 identity and journal header.
2. In v2, generate baseline argv from baseline settings and candidate argv from candidate settings. Validate argv, payload strategy/threshold, source binding and common row identity independently for each side. Keep deterministic randomized process order, exact call counts, raw command files and crash-safe pair journaling. Refuse incomplete, duplicate, foreign, changed-settings, stale-source, stale-binary, or mixed-schema resumes before launching a guest.
3. Extend P137 analysis with a narrow schema-aware row-settings adapter. Reuse its existing validation, deterministic `_seed_for`, `_summary`, `_bootstrap_interval`, and paired-log-median method. Bind the report hash, input references and both policy settings before/after analysis. Preserve v1's existing output interpretation.
4. Add `p139.py` to verify the already closed P136/P137/P138 prerequisites, pin the actual post-P138 immediate Register archive, build/attest the archived eager baseline adapter and current adaptive candidate harness from their verified source roots, collect a non-promoting smoke, validate its already-produced descriptive analysis, and emit source-bound receipts. Evidence checking must not rebuild, retime, rerun analysis, or run guest code.
5. Run the exact Python development groups. Full close builds the two minimal runtime-only adapters once from the pinned archive and current source; no Wasmtime package, execution-reference, or performance-counter feature is present in either timing binary. It then collects the smoke rows below and revalidates all receipts. The candidate and baseline executable identities are separate file references; never represent one executable by two directory aliases or claim an archived binary is current.

All synthetic tests create fresh temporary roots and check that every resolved output path remains within its root before writing. Copy executed helper sources before execution, use `python3 -B`, retain failed command records, and never mutate the immutable archive or prior P136/P137/P138 receipts.

## Meaningful cases

The paired-collector/analyzer suite contains these eight cases:

- `test_p139_v1_and_v2_rows_normalize_without_mixing_schemas`
- `test_p139_collector_uses_distinct_eager_and_adaptive_side_argv`
- `test_p139_report_rejects_swapped_side_argv_payload_and_binding`
- `test_p139_rejects_bool_unknown_and_mixed_settings`
- `test_p139_checkpoint_resume_rejects_changed_side_identity_and_pair_corruption`
- `test_p139_v1_report_remains_analyzable`
- `test_p139_v2_two_run_thirty_pair_analysis_keeps_rows_and_bootstraps_10000`
- `test_p139_smoke_keeps_raw_totals_and_null_uncertainty`

The source-bound producer suite checks the frozen eight-row side-policy matrix, omission/order rejection, pinned archive identity, actual fixture provenance and MVP validator logs, fresh temporary output containment, complete source closure, and rejection of unbound helper copies. The full contract also reruns the seven P136 collector and eight P137 analysis compatibility cases; these guard the v1 readers changed by P139.

## Verification

Keep the Python unit groups separate from the source-current full close. The full smoke matrix has eight rows: the P135 counter case at N = 1, 10, 100, and 10000, each with `baseline_strategy=eager`, `baseline_threshold=0`, and `candidate_strategy=adaptive` at threshold 3 or 10. Run two pairs per row in each of two distinct seeded runs (32 pairs, 64 fresh-process observations). It validates settings forwarding, exact fresh lifetimes, receipts, and descriptive analysis only; it is not timing evidence and cannot satisfy P066's 30-pair gate.

The contract runs all three named Python groups once: eight P139 collector/analyzer tests, seven P139 producer tests, and 15 P136/P137 v1-compatibility tests. It then builds one archived adapter and one current-workspace adapter, collects the eight rows, analyzes the retained smoke once, and validates receipts without launching more guests or recomputing analysis. Activate the pinned Rust environment before freeze/build/check commands so P138's Cargo metadata checks use the selected toolchain.

```sh
python3 -B -m unittest discover -s tools/wtime-bench/tests -p test_p139_entry_settings.py -v
python3 -B -m unittest -b -v tools.verification.tests.test_p139_entry_batch
python3 -B -m unittest -b -v tools.wtime_bench.tests.test_p136_entry_batch tools.wtime_bench.tests.test_p137_entry_analysis
. /workspace/.setup/rust-env.sh
python3 -B tools/verification/producers/p139.py freeze --output target/verification/P139-freeze
python3 -B tools/wtime-bench/entry_batch_adapter.py build --core-archive target/performance/P139-before --harness tools/wtime-bench --output target/verification/P139-baseline-adapter
python3 -B tools/wtime-bench/entry_batch_adapter.py build-current --repo . --harness tools/wtime-bench --output target/verification/P139-current-adapter
python3 -B tools/wtime-bench/entry_batch_compare.py --archive target/performance/P139-before --adapter target/verification/P139-baseline-adapter --candidate target/verification/P139-current-adapter --repo . --harness tools/wtime-bench --rows target/verification/P139-freeze/rows.json --pairs 2 --seeds 139001 139002 --output target/verification/P139-collection --smoke
python3 -B tools/wtime-bench/entry_batch_analyze.py --comparison target/verification/P139-collection --output target/verification/P139-analysis --allow-smoke
python3 -B tools/verification/producers/p139.py verify --freeze target/verification/P139-freeze/report.json --baseline target/verification/P139-baseline-adapter --candidate target/verification/P139-current-adapter --collection target/verification/P139-collection --analysis target/verification/P139-analysis/report.json --output target/verification/P139-verify
python3 -B tools/verification/producers/p139.py check-evidence --report target/verification/P139-verify/report.json
python3 -B tools/verify.py plan --contract docs/benchmarks/performance/P139-verification.json
python3 -B tools/verify.py close --contract docs/benchmarks/performance/P139-verification.json --output target/performance/P139/complete --execute
```

The immutable P139-before archive is pinned to commit `1a41f327580bc3fdbaa8460bb8625ba46ca5a798` with manifest SHA-256 `327564c81bdef90aa3e5958a1b373c7b3778173eb5f2370f16474f14d59b58b4`. It already exists at `target/performance/P139-before`; never replace it. Its core source map matches the post-P138 checkout. Reuse P138's current default/Register build-budget evidence because P139 changes no runtime or build inputs.
## Decision boundary

Selection: `register`. Baseline setting: `eager`, threshold 0. Candidate setting: `adaptive`, positive threshold selected by P066's frozen contract. Decision rule: `integrity`.

P139 establishes a truthful side-specific collection and analysis foundation only. It makes no speed claim, does not choose a threshold, and does not replace P066's focused-stage, semantic, build/dependency, mandatory tiny/large controls, cold-code/cache-churn, complete pair count, uncertainty, or promotion decision. P138 diagnostic traces remain separate, explicitly instrumented, and outside production timing. P066's production timing binaries keep diagnostics disabled.

## Completion evidence

P139 completed as `contract_only`: 30 checks passed (eight side-settings,
seven producer and 15 v1 compatibility tests), with zero failures or ignored
cases in the successful close. Its separately built archived/current Register
executables collected 32 pairs across two seeds and eight rows, making 64 fresh
process observations and 161,776 checked guest calls. Baseline eager/0 and
candidate adaptive/3 or adaptive/10 remain explicit in every v2 row. There is
no ratio, confidence interval, threshold choice or performance promotion.

Close `attempt-0004` binds evidence SHA-256
`b43321ef951629dbffe849278bd1f9c4ec5c8fdca07fdcd108bec93c459bf7cc`
and proof `52cfccaf7cfa5fca57d98a68bfb918d455b0a1fa92c346a9ea87ffcc0bd45a19`.
The one retained analysis pass has SHA-256
`26e560ecfe71d4da60a28e913cb9501077e482c68187a21435da74d661410d7f`.
Current P138 build budgets were revalidated and reused: 738 compilation inputs,
zero external core dependencies, and stripped default/Register sizes of
1,000,960/1,090,512 bytes. No Rust source or timer changed.

See [completion](../../../wtime/docs/benchmarks/performance/P139-completion.json),
[proof](../../../wtime/docs/benchmarks/performance/P139-proof.json),
[analysis](../../../wtime/docs/benchmarks/performance/P139-analysis.json),
[freeze](../../../wtime/docs/benchmarks/performance/P139-freeze.json),
[evidence](../../../wtime/docs/benchmarks/performance/P139-evidence.json), and
the [1,597-member record archive](../../../wtime/docs/benchmarks/performance/P139-records.json.gz)
(SHA-256 `2303e3378eddfd637eee0ad3112d6cd3d0504a06feb1df0b5d6beb0c35b340f1`).

The current close reuses the successful builds, 32 smoke pairs and descriptive
analysis from `target/verification/runs/P139/verify/attempt-0002`. All 30 Python
checks passed there; the attempt failed only when the proof checker read an
array with its object reader. The corrected close binds the retained artifact
trees as explicit inputs, reruns the Python checks and revalidates both build
records, executables, sources, journal, raw observations and analysis. It does
not rebuild or rerun Wasm. The failed attempt remains preserved and is not a
completion receipt. Fresh runs without those retained inputs must execute the
original build/collection commands recorded in that attempt.

Record the P139-before archive identity, immutable P136/P137/P138 prerequisite evidence hashes, both archived/current build-record and executable file hashes, per-side settings and exact argv, fixture/helper/source maps, two smoke seeds, 32 raw pairs, schema versions, exact analysis inputs and outputs, required test names, and verification receipts. Smoke must remain `timing_evidence:false`, `gate_eligible:false`, `decision:null`, and `threshold_selection:null`. Keep status pending until the source-bound full contract passes.

## Before editing

```sh
python3 -B tools/wtime-bench/baseline.py --verify target/performance/P139-before
```

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/P139/shared-finalization/attempt-0004/evidence.json": "b43321ef951629dbffe849278bd1f9c4ec5c8fdca07fdcd108bec93c459bf7cc"
  },
  "evidence": [
    {
      "bytes": 1914,
      "path": "/workspace/wtime/target/performance/P139/shared-finalization/attempt-0004/evidence.json",
      "sha256": "b43321ef951629dbffe849278bd1f9c4ec5c8fdca07fdcd108bec93c459bf7cc"
    }
  ],
  "outcome": "contract_only",
  "status": "done",
  "task": "P139"
}
```
