# P145 — Bind Register workload execution-path diagnostics

Status: **done**

Outcome: **contract_only**

Kind: `foundation` · Branch: `portable` · Model: **Sol 6.1 Low**

## Purpose and dependencies

P145 adds an offline, source-bound path probe for the Register conformance corpus. P143 supplies explicit threshold-3 conformance and P144 supplies activation-capacity diagnostics. P067 uses this evidence to distinguish real optimized execution, prefix exits, and whole-function fallback across its frozen workload selection. This tool reports execution counts and physical activation capacity only; it adds no timing fields and does not change the runtime or timed binary.

Dependencies: [P143 — Forward explicit Register thresholds through conformance](P143-register-threshold-conformance.md) and [P144 — Report Register activation capacity high-water](P144-register-activation-capacity-diagnostics.md).

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), and P143, P144, and P067 contracts before implementation.

- `tools/wtime-path-probe/Cargo.toml`
- `tools/wtime-path-probe/Cargo.lock`
- `tools/wtime-path-probe/src/main.rs`
- `tools/verification/producers/p145.py`
- `tools/verification/tests/test_p145.py`
- `docs/benchmarks/performance/P145-verification.json`
- P067's `docs/benchmarks/performance/register.md` and `tools/verification/producers/p067.py` integration hunk

Keep the standalone Rust tool offline, safe, and free of external dependencies. Do not add it to the production Cargo workspace. Runtime and timing source identities remain unchanged.

## Input/output contract

The probe consumes the exact P067 frozen benchmark fixture, required feature bits, typed input, independent expected result or expected trap, and any declared setup driver. Supported setup drivers are `plain`, `host_tick`, `table_a`, `representative`, `alias`, `multi`, `seeded-copy`, `seeded-fill`, and `compile_only`; `prepare_large` is compile-only. It validates fixture/source/binary hashes, exact row identities, driver profile, typed arguments, expected outcomes, complete row counts, counter reset, and cleanup. It emits source-bound per-workload `register_dispatches`, `exits`, `fallback_frames`, and the P144 activation-capacity high-water; it emits no timing values.

For eager Register, invoke each runnable workload once. For adaptive Register threshold 3, repeat using fresh Stores that share the same Module and reset P144 invocation counters before each run, so compilation/preparation is excluded from invocation diagnostics. Classify rows from observed counters as optimized, prefix-exit, whole-function fallback, or mixed. `mixed` means at least one Register dispatch and at least one fallback frame with no exit; this can occur when a cold adaptive caller invokes an already-hot child. Never synthesize successful coverage. Validate optimized, prefix-exit, fallback, and mixed coverage separately.

The integration selection has 174 rows including `prepare_large`; 173 are runnable and `prepare_large` is compile-only. The adaptive selection has 95 rows including that same prepare-only row; 94 are runnable. Report the excluded prepare-only row explicitly and validate exact selected IDs, fixture and source hashes, executable identity, arguments, profile, outcome/trap, counter reset, and cleanup.

## Implementation recipe

1. Inspect the P067 frozen selection and P143/P144 report formats. Define strict typed input and output records and bind the fixture, source, binary, selection, and optional representative preparation export.
2. Implement the offline Rust probe with only the standard library. Read the frozen fixture and feature requirements; execute eager once or adaptive threshold 3 against fresh Stores sharing the Module; reset P144 invocation counters before each call and preserve exact typed results or traps.
3. Emit actual Register dispatch, exit, fallback-frame, and activation-capacity diagnostics per workload. Classify optimized, prefix-exit, whole-function fallback, and mixed execution from those counters; a row with dispatches and fallback frames but no exit is mixed. Retain preparation-only `prepare_large` as compile-only, never an invocation.
4. Add the P145 producer and tests. Reject incomplete or duplicate row sets, altered fixture/source/binary hashes, mismatched settings or arguments, invalid typed values, wrong results/traps, inconsistent counters, stale capacity values, missing cleanup, and zero optimized coverage where the corpus expects it.
5. Bind P145 validation into P067's frozen report without changing P067 timing inputs or executable identity. Keep eager and adaptive diagnostic evidence separate and report no timing result or threshold recommendation.

## Meaningful cases

- The complete eager and adaptive row sets have the exact expected count, with `prepare_large` recorded as compile-only and no fabricated execution row.
- Numeric integer/strict-float workloads produce real Register-dispatch evidence; vector workloads demonstrate separately counted fallback.
- A cold adaptive caller that dispatches an already-hot child is classified as mixed when it has Register dispatches and fallback frames without an exit.
- Typed traps and representative prepare exports are accepted only when their exact frozen hashes and expected outcomes match; stale or malformed inputs fail closed.
- Adaptive threshold-3 runs use fresh Stores sharing the Module, reset invocation diagnostics each time, and report cold and promoted invocations without including compile work in activation counters.

## Verification

Run the offline tool and producer tests without guest timing or external dependencies. Validate the exact eager 173-row and adaptive 94-row invocation sets, declared setup drivers, source and binary bindings, optimized/prefix-exit/fallback/mixed distinctions, prepare-only handling, per-run counter resets and cleanup, and separate P143/P144 feature evidence. The producer must reject missing, extra, duplicated, stale, or malformed observations and report `timing_evidence:false`.

```sh
python3 -B -m unittest discover -s tools/verification/tests -p 'test_p145.py' -v -b
cargo test --locked --offline -j 1 --manifest-path tools/wtime-path-probe/Cargo.toml -- --show-output
python3 -B tools/verify.py plan --contract docs/benchmarks/performance/P145-verification.json
python3 -B tools/verify.py close --contract docs/benchmarks/performance/P145-verification.json --output target/performance/P145/complete --execute
```

## Measurement and decision

Selection: P067 `integration`. Execution policy: eager Register and adaptive Register threshold 3. Decision rule: `integrity`.

P145 is path-coverage evidence only. It does not change runtime policy, timed binaries, source identities, or the P067 timing decision, and it makes no promotion or threshold recommendation.

## Completion evidence

Bind the frozen selection, complete row counts, fixtures, source and binary hashes, exact arguments and feature profiles, expected results or traps, per-row dispatch/exit/fallback counters, activation high-water, cleanup, and test results. Record compile-only handling for `prepare_large` and `timing_evidence:false`.

## Final result

P145 completed as **contract_only** with 11 passing tests (8 Python and 3 Rust) and no timing evidence. The verification reused 390 source-current Rust controls. The frozen path probe recorded 549 invocations: eager Register had 173 (77 optimized, 56 prefix-exit, 40 whole-function fallback); adaptive threshold 3 had 376 (76 optimized, 126 prefix-exit, 126 whole-function fallback, 48 mixed). In each strategy, `prepare_large` remained compile-only; it contributed no invocation. Adaptive calls used fresh Stores sharing the Module and reset activation counters per call, excluding preparation from those counts. The probe is untimed and does not change runtime policy or the measured executable.

Published records: [P145-completion.json](../../../wtime/docs/benchmarks/performance/P145-completion.json), [P145-evidence.json](../../../wtime/docs/benchmarks/performance/P145-evidence.json), [P145-freeze.json](../../../wtime/docs/benchmarks/performance/P145-freeze.json), [P145-proof.json](../../../wtime/docs/benchmarks/performance/P145-proof.json), [P145-observations.json](../../../wtime/docs/benchmarks/performance/P145-observations.json), [P145-records.json.gz](../../../wtime/docs/benchmarks/performance/P145-records.json.gz).

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/P145/shared-finalization/attempt-0001/evidence.json": "237319a4f64c564892f1c31c8105b3490be2fad5952f3889162ef01b2042abd1"
  },
  "evidence": [
    {
      "bytes": 1866,
      "path": "/workspace/wtime/target/performance/P145/shared-finalization/attempt-0001/evidence.json",
      "sha256": "237319a4f64c564892f1c31c8105b3490be2fad5952f3889162ef01b2042abd1"
    }
  ],
  "outcome": "contract_only",
  "status": "done",
  "task": "P145"
}
```
