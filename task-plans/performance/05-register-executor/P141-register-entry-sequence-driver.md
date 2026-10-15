# P141 — Add individual-entry Register timing and diagnostics

Status: **done**

Outcome: **contract_only**

Kind: `foundation` · Branch: `portable` · Model: **Sol 6.1 Low**

## Purpose and dependencies

P141 adds a deterministic per-invocation driver for observing cold and churn behavior of the existing Register cache. P138 supplies the fixed `uniform66` fixture, the 64-entry FIFO cache, and on-demand cache snapshots; P140 supplies the eager/adaptive entry setting. P142 will bind this driver to source-bound paired collection and analysis. P141 is protocol and observation tooling only: it does not change cache policy or claim a speedup.

P141 depends on **P138** and **P140**. P066 depends on the later P142 evidence foundation and retains all of its current semantic, focused-stage, control, cold/churn, build/dependency, statistical, and promotion gates.

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [MEASUREMENT.md](../MEASUREMENT.md), the [P138 diagnostics contract](P138-register-cache-diagnostics.md), and the [P140 threshold contract](P140-register-entry-threshold.md). Inspect the current protocol dispatcher, Register policy construction, Store invocation API, P138 cache snapshot, P138 fixture provenance, and feature-specific test registration before editing.

- `tools/wtime-bench/src/protocol.rs`
- `tools/wtime-bench/src/entry_sequence_driver.rs` (new leaf protocol driver)
- `tools/verification/producers/p141.py` (source-bound smoke and report checker)
- `tools/verification/producers/p141_driver_tests.py` (exact Cargo test configurations and named cases)
- `tools/wtime-bench/entry_sequence_policy.py` (strict wire/schedule policy)
- `tools/wtime-bench/execution_identity.py` (classify the new timing helper identity)
- `tools/wtime-bench/tests/test_p141_sequence_policy.py`
- `tools/wtime-bench/tests/test_fast_execution_identity.py`
- `docs/benchmarks/performance/P141-contract.json`
- `docs/benchmarks/performance/P141-verification.json`

Use the existing `tools/wtime-bench` protocol entry point and Cargo test binary; do not create a new runtime crate, alter P138's fixture, add dependencies, or edit cache policy. Keep timer, diagnostic, and source-bound evidence code in the task-owned leaf and producer.

## Input/output contract

Add two strict protocols: `--entry-sequence FILE INPUT SCHEDULE_THRESHOLD register eager|adaptive THRESHOLD` and `--entry-sequence-diagnostic FILE INPUT SCHEDULE_THRESHOLD register eager|adaptive THRESHOLD`. Parse and validate the entire argument vector before opening `FILE`. Require the fixed Register mode, schedule threshold `N` in `1..=64`, `eager` with engine threshold `0`, and `adaptive` with engine threshold exactly equal to schedule threshold N. Do not accept fuel, warmup, batch invocation, duplicate settings, unknown modes, non-integers, booleans-as-numbers, zero adaptive thresholds, or out-of-range schedule thresholds. Use the checked-in P138 `uniform66.wasm` bytes unchanged.

Generate a deterministic schedule that calls each export `f00` through `f65` exactly `N+1` times in order, then revisits `f00` exactly `N+1` times. Each call invokes one export once, in one Store and one module instance. Compute the independent expected result as the wrapping `i32` sum of the input and the numeric body index; check every returned result. No guest fuel is configured.

The eager side uses ordinary eager Register execution with effective entry threshold zero. The adaptive side uses P140's explicit positive threshold. Both sides receive identical fixture bytes, input, schedule threshold and call order. Separate module preparation and instantiation durations from entry observations. In `timing` mode, time each `Store::invoke` individually and only: do not include snapshot reads, result formatting, logging, command processing or I/O in `elapsed_ns`. Emit ordered raw records with `{body,visit,phase,result,elapsed_ns}`, distinguishing the initial pass from the revisit. Timing builds use `runtime-only,register-executor` without `execution-reference` or performance counters and set `instrumented:false`.

In `--entry-sequence-diagnostic` mode, require `execution-reference` and execute the identical call schedule without timers. Capture per-call P138 cache state/counters and dispatch deltas, independently validate each expected result, and verify cold, compile, ready and recompile transitions against the deterministic schedule. The Python policy assigns phase-specific cold/compile/warm/recompile classes from exact body/visit/phase positions; the diagnostic wire independently validates observed state and counter transitions. Prove the second `f00` series occurs after its FIFO eviction from the 64-entry cache. Diagnostic builds use `runtime-only,compact-executor,register-executor,execution-reference`, and their per-call wires set `instrumented:true` and `diagnostic:true` without duration or timestamp fields. The source-bound smoke report carries `timing_evidence:false`, `gate_eligible:false`, and `candidate_promoted:false`; diagnostic wires remain separate from timing observations.

## Implementation recipe

1. Confirm the completed P138 and P140 prerequisite identities and fixture provenance. Freeze the current P140 source/feature inputs for the producer contract; keep all generated outputs under a fresh, exclusive P141 target directory.
2. Add the new leaf module to the existing protocol dispatcher. Implement strict argument parsing and validation before file reads; share one validated schedule representation between timing and diagnostic modes. Keep the timing build uninstrumented and require `execution-reference` only for diagnostic builds.
3. Build eager and adaptive Register engines with the exact requested settings, compile the uniform fixture, and instantiate once. In timing mode isolate preparation and instantiation durations and time only one `Store::invoke` at a time. Keep serialization and all I/O outside every invoke timer.
4. Implement diagnostic observations as a separate no-timer path. Take P138 snapshots and dispatch deltas around calls; validate every result and check observed state/counter transitions against the deterministic schedule, and verify the `f00` eviction and recompile against the documented FIFO limit.
5. Add the three actual named Rust tests for strict parsing/build rejection, full 66-body/revisit execution, and oracle/FIFO recompile. Add four Python policy tests plus the seven existing execution-identity compatibility tests. Add producer checks for source/fixture identities, ordered records, exact counts, timer isolation, diagnostic wire shape and non-promoting smoke fields.
6. Preserve the actual source-bound attempt, including both feature-specific Cargo test groups, the production build, four timing observations and four diagnostic observations (2010 calls per lane), raw commands and outputs. If a checker-only fix follows collection, bind the previously passing, source-current attempt unchanged; rerun the Python policy suite and read-only producer revalidation only. Retain and identify any failed/superseded verification attempt rather than replacing it.

## Meaningful cases

- `p141_sequence_parser_and_build_reject_before_fixture_io`: accept the exact timing and diagnostic switches; reject malformed schedules, mismatched thresholds, non-Register mode, bad strategy, duplicates, fuel and unknown options before attempting a missing fixture, and reject incompatible build features before file access.
- `p141_sequence_executes_every_initial_and_revisit_entry`: run all 66 bodies through `N+1` calls and revisit `f00`; verify ordered body/visit/phase records and exact call counts for both strategies and modes.
- `p141_sequence_oracle_and_fifo_recompile_are_checked`: validate every wrapping result, observed P138 lifecycle/cache state, 64-entry eviction and f00 recompile; ensure diagnostic wires contain no timing or timestamp fields.
- `test_exact_schedule_and_wraparound_oracle_for_all_thresholds`: validate schedule cardinality and wrapping i32 oracle at schedule thresholds 1, 3, 10 and 64.
- `test_rejects_exact_type_count_order_oracle_and_extra_field_drift`: reject wire type, ordering, count, oracle and unknown-field drift.
- `test_rejects_instrumented_or_mixed_timing_and_diagnostic_wires`: reject timed/diagnostic schema mixing and instrumentation mismatches.
- `test_adaptive_transition_drift_and_eager_diagnostic_contract`: reject observed cache/counter drift and invalid eager diagnostics.

## Verification

The retained source-bound attempt-0001 diagnostic driver used exactly `runtime-only,compact-executor,register-executor,execution-reference` and passed at least 18 unfiltered tests, including all three named P141 Rust cases. Its production driver used exactly `runtime-only,register-executor` and passed at least 12 unfiltered tests with the same required cases. The retained Python policy command passed all four `test_p141_sequence_policy.py` cases and seven `test_fast_execution_identity.py` cases. The immutable smoke covers thresholds 3 and 10 with four timing and four diagnostic observations, 2010 individual calls per lane. Diagnostic wires are validated from `P141_TRACE` lines in the retained `driver-diagnostic/tests.stderr`. The final verification reuses these source-current reports, command logs, build record and smoke report through the declared retained-protocol identity; it reruns all 11 Python policy/identity tests and runs `p141.py revalidate` against the retained smoke. The revalidation report is the final `P141-proof.json`; it launches no build or guest and asserts source stability. The failed checker attempt remains in the records. No threshold selection or decision is emitted.

```sh
python3 -B tools/verify.py plan --contract docs/benchmarks/performance/P141-verification.json
python3 -B tools/verify.py close --contract docs/benchmarks/performance/P141-verification.json --output target/performance/P141/complete --execute
```

## Measurement and decision

Selection: `register`. Execution mode: `register`. Decision rule: `integrity`.

P141 supplies a repeatable per-entry protocol and non-timing cache diagnostics. It makes no speed claim, threshold recommendation, candidate decision or promotion. P142 owns paired source-bound collection and descriptive analysis; P066 remains the candidate gate and retains every existing requirement.

## Completion evidence

P141 completed as `contract_only` in close attempt-0002 with evidence SHA-256 `2c1db3a11a9d843430f7ebd75a2813d897b7334b40f071e1b314b19a32685c67`; publication attempt publish0052 recorded snapshot `P141-publication-0001`. The retained source-bound observation attempt passed 18 diagnostic Rust tests, 12 production Rust tests, and 11 Python tests, then validated four timing and four diagnostic observations across thresholds 3 and 10 (2010 calls per lane), including cold, FIFO eviction, and recompile behavior. Core sources remained unchanged with zero external dependencies; P138 build budgets were revalidated and reused. Verify attempt-0001 preserved the report-adapter mismatch failure while retaining the successful build, tests, and raw smoke unchanged; close attempt-0002 passed using read-only revalidation, with no build or guest replay. The exported records contain 420 members (3,163,319 bytes; SHA-256 `bc774d16cd519ea6f3e07d9119727168ee9f8fec9c54b84fcc8788ba97ec3d87`), and `P141-proof.json` is the revalidation report (SHA-256 `6e3f8044a61e4746e750e63ff334d694ee17b1dd096efe5a119930673815d5ac`). The exports are `P141-completion.json`, `P141-evidence.json`, `P141-freeze.json`, `P141-proof.json`, and `P141-records.json.gz`; smoke remains bound through the proof and records.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/P141/shared-finalization/attempt-0002/evidence.json": "2c1db3a11a9d843430f7ebd75a2813d897b7334b40f071e1b314b19a32685c67"
  },
  "evidence": [
    {
      "bytes": 1897,
      "path": "/workspace/wtime/target/performance/P141/shared-finalization/attempt-0002/evidence.json",
      "sha256": "2c1db3a11a9d843430f7ebd75a2813d897b7334b40f071e1b314b19a32685c67"
    }
  ],
  "outcome": "contract_only",
  "status": "done",
  "task": "P141"
}
```
