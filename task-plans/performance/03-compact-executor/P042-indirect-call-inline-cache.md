# P042 — Add guarded bounded indirect and call_ref caches

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

[P033 — Cache immutable subtype answers with identity and work guards](../02-structural-wins/P033-bounded-subtype-cache.md)

[P041 — Resolve direct guest call bindings once per instance](P041-instance-call-plans.md)

## Scope and ownership

Read EXECUTION.md, SOURCE_NOTES.md, BACKEND_CONTRACTS.md and contracts C1, C2, C3, C4, C5, C6, C7 and C8.

- `crates/wtime-runtime/src/execution/compact/calls.rs`
- `crates/wtime-runtime/src/lib.rs`
- `crates/wtime-runtime/src/canonical.rs` (private canonical identity owner query)
- `crates/wtime-runtime/tests/performance/p042.rs` (focused public-API correctness cases)
- `crates/wtime-runtime/src/execution/compact/indirect.rs` (new bounded per-call-site cache leaf)
- `crates/wtime-runtime/src/execution/compact/mod.rs`
- `crates/wtime-runtime/src/execution.rs`
- `crates/wtime-runtime/src/linker.rs`
- `crates/wtime-runtime/src/perf.rs`
- `crates/wtime-runtime/tests/performance.rs` (P042 test registration)
- `crates/wtime-runtime/tests/performance/p042_private.rs` (private cache invariants and invalidation checks)
- `crates/wtime-runtime/tests/fixtures/performance/p042` (P042 runtime fixtures)
- `tools/wtime-bench/workloads.json` (preserve default scalar rows and add the combined P042 selection)
- `tools/wtime-bench/scalar_workloads.py` (independent oracle models for supplemental families)
- `tools/wtime-bench/fixtures/scalar/p042/table-indirect-2targets.wat`
- `tools/wtime-bench/fixtures/scalar/p042/table-indirect-2targets.wasm`
- `tools/wtime-bench/fixtures/scalar/p042/table-indirect-3targets.wat`
- `tools/wtime-bench/fixtures/scalar/p042/table-indirect-3targets.wasm`
- `tools/wtime-bench/fixtures/scalar/p042/typed-call-ref-2targets.wat`
- `tools/wtime-bench/fixtures/scalar/p042/typed-call-ref-2targets.wasm`
- `tools/wtime-bench/fixtures/scalar/p042/typed-call-ref-3targets.wat`
- `tools/wtime-bench/fixtures/scalar/p042/typed-call-ref-3targets.wasm`
- `tools/wtime-bench/fixtures/scalar/p042/table-between-calls.wat`
- `tools/wtime-bench/fixtures/scalar/p042/table-between-calls.wasm`
- `docs/benchmarks/performance/P042-contract.json` (combined timing roles)
- `docs/benchmarks/performance/P042-diagnostic-contract.json` (separate scalar diagnostic roles)

## Input/output contract

On Compact execution, keep a per-instance, per-call-site cache with at most two successful entries for the existing `call_indirect` and `call_ref` paths. Each invocation first resolves the current table/reference value and validates current bounds, null state, Store identity and handle generation. Only then may it use the cache, keyed by current target identity/generation and canonical signature context. A hit may reuse successful compatibility and guest/host classification; it must preserve the shared host, root, trap and tail-call flows. Never cache mutable slot contents as if they were current, cache errors, or admit an entire reference-bearing function body. Misses and unsupported cases take the ordinary checked path. Counters are diagnostic only and production timing remains uninstrumented.

## Implementation recipe

1. Add a small cache owned by the Compact instance and indexed by the existing indirect/call_ref call site; cap each site at two successful target entries.
2. On every operation load the current table/reference element and perform the existing bounds, null, Store, generation and target-handle validation before lookup.
3. Key compatible entries by current target identity/generation plus canonical expected type; cache successful compatibility and guest/host classification only.
4. Preserve the current miss path, subtype/work limits, host callbacks, GC roots, error ordering and tail-call behavior. Do not add whole-function reference-body admission or bypass the shared call path.
5. Test same-target hits, alternating targets, third-target eviction, replacement between batches, host targets, aliases, null/OOB/mismatch and Store/arena reuse. Keep the per-site capacity at two.
6. Preserve the default 85-row scalar selection and append the five validated P042 families to `scalar-p042-cache`: two-/three-target indirect calls, two-/three-target call_ref, and table-target changes between exported calls. Primary includes the original ten nonzero indirect/call_ref rows plus supplemental n=17,32,1024,10000 rows (30 total); protect the remaining 80 rows, including n=0 controls. Use the combined 110-row contract for the paired gate. Keep the separate 85-row diagnostic contract for instrumented cache counters; never mix its counters into production timings.

## Meaningful cases

- Monomorphic same-target calls, two-target polymorphism, three-target churn, and alternating table targets between `run` invocations preserve the independent result oracle.
- Instrumented focused tests prove same-target hits, two cached targets, third-target eviction, and fresh target resolution after table/reference replacement; timing WATs deliberately use equivalent target results and do not prove invalidation.
- Null, OOB, stale generation, Store mismatch, canonical type mismatch, host reentry, GC roots and tail-call errors/results remain unchanged.

## Verification

Complete P033 and P041 first. Require and bind the hash-verified extra-family fixture validation receipt before installing the combined selection or archiving. Run default and diagnostic focused P042 tests proving same-target hits, two-entry polymorphism, eviction, current-target replacement and fallback; rerun P041 call-plan regressions and the full unfiltered runtime suite. Run the diagnostic-only `tools/wtime-bench/p042_coverage.py` with `--contract docs/benchmarks/performance/P042-diagnostic-contract.json`, the exact completed prepare-receipt and check-result paths, and an exclusive coverage output directory. The main contract fixes 30 primary/80 protected combined rows; a separate diagnostic contract fixes the original scalar 10/75 partition. Pass only the main four-key contract to `gates.py`; the coverage collector must bind the diagnostic-contract hash separately. Record actual command exits, exact settings, named counts, source/binary/archive/fixture/receipt hashes and build budgets.

```sh
cargo test --locked -p wtime-runtime --features compact-executor,execution-reference p042_ -- --nocapture
cargo test --locked -p wtime-runtime --features compact-executor,execution-reference,performance-counters p042_ -- --nocapture
cargo test --locked -p wtime-runtime --features compact-executor,execution-reference p041_ -- --nocapture
cargo test --locked -p wtime-runtime
python3 -B tools/wtime-bench/p042_coverage.py --workspace-root . --contract docs/benchmarks/performance/P042-diagnostic-contract.json --prepare-receipt target/performance/P042/prepare/attempt-0001/receipt.json --check-result target/performance/P042/check/attempt-0001/result.json --output target/performance/P042/coverage --execute
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml --no-default-features --features compact-executor -j4
python3 -B tools/wtime-bench/compare.py --baseline target/performance/P042-before --candidate tools/wtime-bench/target/release/wtime-bench --candidate-mode compact --selection scalar-p042-cache --output target/performance/P042/comparison --pairs 30
python3 -B tools/wtime-bench/analyze.py --input target/performance/P042/comparison --output target/performance/P042/comparison/analysis.json
python3 -B tools/checks/build_budget.py --output target/performance/P042/build.json
python3 -B tools/checks/build_budget.py --features wtime-runtime/compact-executor --output target/performance/P042/optional-build.json
python3 -B tools/wtime-bench/gates.py --task P042 --comparison target/performance/P042/comparison --build target/performance/P042/build.json --optional-build target/performance/P042/optional-build.json --contract docs/benchmarks/performance/P042-contract.json --output target/performance/P042/decision.json
```

## Measurement and decision

Selection: `scalar-p042-cache`. Explicit execution setting: `compact`. Decision rule: `measured_promotion`.

The combined timing selection preserves the default 85 scalar IDs in their existing order and appends 25 validated P042 IDs, for 110 rows. Primary is exactly 30 rows: the existing five nonzero sizes each for `scalar_indirect_calls` and `scalar_call_ref`, plus n=17,32,1024,10000 in each of five supplemental families (`scalar_table_indirect_2targets`, `scalar_table_indirect_3targets`, `scalar_typed_call_ref_2targets`, `scalar_typed_call_ref_3targets`, `scalar_table_between_calls`). Protect the remaining 80, including all original scalar controls and five supplemental n=0 rows. Require all primary and protected groups in both runs. The overall gate consumes this single combined comparison; an absent, failed or inconclusive supplemental primary prevents promotion. Keep the instrumented cache-counter collection under `P042-diagnostic-contract.json`, selection `scalar`, exactly 85 rows/10 primary/75 protected. Its 85 oracle preflights and 170 counted same-instance invocations are diagnostic only and never become timing samples. Keep timing uninstrumented, Compact on both sides, 30 pairs per group and existing cold/preparation/build gates. This is a predeclaration, not a speedup claim.

## Before editing

After P041 closes, create a fresh Compact archive of that immediate 28-file predecessor before P042 source edits. Preserve the actual P033 rejected/restored final decision and P041 source-bound closure as prerequisites. P042 adds the planned `execution/compact/indirect.rs` leaf, making a 29-file candidate runtime map; bind the actual map and archive hashes rather than assuming them. Run the fresh archive command with `--mode compact`.

```sh
python3 -B tools/wtime-bench/baseline.py --mode compact --output target/performance/P042-before
```

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. Executed evidence is recorded below.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/P042/shared-finalization/attempt-0001/indirect-cache-evidence.json": "5d8d785f25cb1d3c97178b7fa2bf19b0d5a1fd6286d9aec60ed1fe8b0ec7f315"
  },
  "evidence": [
    {
      "bytes": 31081,
      "path": "/workspace/wtime/target/performance/P042/shared-finalization/attempt-0001/indirect-cache-evidence.json",
      "sha256": "5d8d785f25cb1d3c97178b7fa2bf19b0d5a1fd6286d9aec60ed1fe8b0ec7f315"
    }
  ],
  "outcome": "inconclusive",
  "status": "done",
  "task": "P042"
}
```
