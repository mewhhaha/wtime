# P138 — Expose source-bound Register cache diagnostics

Status: **done**

Outcome: **contract_only**

Kind: `foundation` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P134 — Integrate opt-in lazy Register entry](P134-opt-in-lazy-register-entry.md)

[P137 — Analyze paired Register entry totals and phases](P137-entry-total-paired-analysis.md)

P134 owns the opt-in lazy-entry cache and P137 owns raw entry-total analysis. P138 supplies explicit, on-demand cache-state diagnostics and separately labelled traces so P066 can observe cold compilation and cache churn. P066 remains the measured candidate gate: P138 diagnostics are instrumented evidence only and cannot replace or relax any existing P066 semantic, build/dependency, protected-control, selection, or production timing gate.

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [MEASUREMENT.md](../MEASUREMENT.md), [fast-performance.md](../../../wtime/docs/verification/fast-performance.md), [the P134 contract](P134-opt-in-lazy-register-entry.md), and [the P137 contract](P137-entry-total-paired-analysis.md). Inspect the current cache lifecycle, cache lock order, retention accounting, Register entry integration, performance test registration, and P136/P137 source-bound verification before implementation.

- `crates/wtime-runtime/src/register_cache_diagnostics.rs` (new public diagnostic value types)
- `crates/wtime-runtime/src/execution/register/cache.rs` (on-demand snapshot and fixed-size lifecycle aggregates)
- `crates/wtime-runtime/src/lib.rs` (feature-gated exports and `Module::register_cache_snapshot`)
- `crates/wtime-runtime/tests/performance/p138.rs` (new guest-facing tests)
- `crates/wtime-runtime/tests/performance/p138_private.rs` (new private cache lifecycle tests)
- `crates/wtime-runtime/tests/performance/p138_trace.rs` (shared diagnostic trace serializer)
- `crates/wtime-runtime/tests/performance.rs` (bounded P138 module registration)
- `crates/wtime-runtime/tests/fixtures/performance/p138/uniform66.wat` (fixture source)
- `crates/wtime-runtime/tests/fixtures/performance/p138/uniform66.wasm` (checked-in encoded fixture)
- `crates/wtime-runtime/tests/fixtures/performance/p138/LICENSE.txt` (MIT license)
- `crates/wtime-runtime/tests/fixtures/performance/p138/provenance.json` (source and artifact hashes)
- `tools/verification/producers/p138.py` (new source-bound producer)
- `tools/verification/tests/test_p138_cache_diagnostics.py` (new producer tests)
- `docs/benchmarks/performance/P138-contract.json` (declarative acceptance contract)
- `docs/benchmarks/performance/P138-verification.json` (verification workflow, development groups, and evidence receipt)

One integrator owns the public `lib.rs` surface and `tests/performance.rs` registration. Keep this to two focused patches: (1) observational API and bounded counters, then (2) fixture-driven/private traces and source-bound verification. Do not alter cache promotion, retry, cooldown, eviction, compilation, work/fuel behavior, timer/collector, benchmark workload selection, P066 policy, or ordinary default-path behavior.

## Input/output contract

Expose `Module::register_cache_snapshot() -> Option<RegisterCacheSnapshot>` only when both `register-executor` and `execution-reference` features are enabled. A snapshot is an owned copy of numeric diagnostic fields, available on demand; it executes no guest code, mutates no cache state, clones no artifact `Arc`, and returns `None` for a poisoned cache or a foreign owner. The API and all aggregate storage are absent unless both features are enabled. Ordinary eager Register modules and default/Decoded modules return `None` for a lazy-cache snapshot.

Each snapshot identifies the owning engine, threshold, entry limit, and active body. It includes at most 64 resident entries in insertion order. Each row records body index, entry count, local preparation attempts, state (`Cold`, `Compiling`, `Ready`, `Cooldown` with remaining count, or `Negative`), and ready-artifact retained bytes. It also reports settled-ledger remaining attempted/work amounts and retained bytes/limit; active builder debits settle only when the attempt finishes, plus lifetime saturating counts for preparation starts, actual builder starts, completed preparations, published artifacts, publication failures, declined, exhausted, invalid, aborted, and evictions. `preparation_starts` counts an admitted lease even if wrapper allocation exhausts before the builder; `builder_starts` counts only an actual builder call. `completed` records `Preparation::Complete` even if owner/budget validation prevents publication; `published` counts successful Ready publication, and `publication_failures` counts completed-but-unpublished results. Snapshot code acquires cache state before the ledger, follows the established lock order, copies only scalar state while guarded, and never holds either lock while invoking guest or builder code.

Lifecycle aggregates are fixed-size scalars, saturating, and included in the existing cache metadata size accounting. Update them only at the corresponding existing lifecycle transition; never infer compile/preparation work from dispatch or invocation counters. The diagnostic build briefly updates builder-start accounting under the state lock before releasing it and calling the builder; the builder runs with both state and ledger locks released. Diagnostics must not change cache policy, retained artifact lifetime, cooldown/retry decisions, optimizer work, guest fuel, or observable semantics.

Emit a separate diagnostic JSON trace that identifies the fixture/body and records the cache states/counters without timestamps or duration fields. Each emitted trace row is explicitly `instrumented: true` and `timing_evidence: false`; its source-bound containing report additionally records `gate_eligible: false` and `candidate_promoted: false`. Traces are not input to P136/P137 timing analysis and cannot promote a candidate.

The checked-in MVP fixture exports `f00` through `f65`, each `(param i32) (result i32)` and returning the input plus its function index. Its provenance records the MIT license and WAT/Wasm hashes. The independent oracle checks each result without using the runtime's prepared artifact as expected output. Include the implicit function-end event in source-fuel accounting.

## Implementation recipe

1. Freeze the immediate Register predecessor before runtime edits, using the verified P137 commit `78c581a72433fb44b8d135eb0b9971a0eab6bd73`; create a new immutable archive at `target/performance/P138-before` with `--mode register`. Preserve the archive and its manifest/source map. If the checkout no longer matches that commit, stop and record the actual verified predecessor in a distinct new path; never overwrite or relabel an existing archive.
2. Add public owned snapshot structs and a method under `cfg(all(feature = "register-executor", feature = "execution-reference"))`. Snapshot the cache state and ledger in their established state-then-ledger order; copy bounded scalar rows, remaining budgets and lifetime aggregates, clone no `Arc`, and fail closed on poison or owner mismatch.
3. Add the fixed-size saturating lifecycle counters to the same feature-gated cache state and existing metadata-size calculation. Increment exactly at eviction, admitted preparation start, actual builder start, completion, successful publication or publication failure, and each settled decline, exhaustion, invalid result, or abort. Keep cache admission, retry, cooldown, FIFO eviction, work/byte limits and all guest behavior unchanged.
4. Add `uniform66.wat` and its reproducibly encoded Wasm, MIT license and provenance. Test guest results through the exported functions and compare decoded, eager Register and adaptive Register source-event prefixes at fuel budgets 0 through 4, with independent arithmetic expectations and no diagnostic invocation that changes the tested execution.
5. Add private and guest-facing cases below. Write separate no-time diagnostic traces with exact body identities/cache states; bind the fixture and generator/source, both feature flags, predecessor archive, runtime/test sources, producer/checker, contract and receipts. Keep traces explicitly instrumented and nonpromoting.
6. Keep the focused runtime test groups development-only for iteration. A full close runs the unfiltered Register/reference and combined-feature runtime suites; each suite must include all six P138 cases plus the P133/P134/P065 regressions. The full close also runs the default and Register-only build budgets and source-bound checker once. Do not run duplicate filtered tests or checker commands as part of full closure. The producer checks archive integrity, source stability, fixture hashes, trace schemas/body identities, exact feature configuration, and the explicit non-promoting labels. No benchmark collection, threshold selection or performance claim is part of this task.

## Meaningful cases

- `p138_cold_threshold_and_single_builder_are_observed`: at threshold three, the first two entries remain cold and invoke decoded behavior; the third admits one preparation and invokes exactly one builder, then publishes a ready row. Later ready hits do not start more builders. Check result oracle, body identity, attempts, entry counts and exact lifecycle transitions.
- `p138_guest_fifo_eviction_and_recompilation_stay_bounded`: invoke all 66 exports with threshold one, verify insertion-order FIFO residency is capped at 64, then invoke an evicted body to force a new builder. Check eviction/recompile aggregates and that attempted work and retained bytes remain within their configured limits.
- `p138_snapshot_preserves_all_fuel_prefixes_and_wraparound_results`: compare values, traps and fuel consumption for source fuel 0–4 against decoded and eager Register controls, including the implicit function-end event; diagnostic reads must not alter guest-visible state.
- `p138_private_snapshots_do_not_pin_evicted_artifacts`: privately hold a live artifact pin while FIFO eviction removes its resident row; prove its bytes remain charged and artifact usable until the last `Arc` drops, then observe release without refunding attempted work or a snapshot-owned Arc clone.
- `p138_private_failures_cooldown_abort_poison_and_foreign_are_observed`: drive decline/cooldown, abort, poison, invalid, exhausted-work and exhausted-wrapper cases, plus foreign owner and publication refusal; verify exact `preparation_starts`, `builder_starts`, `completed`, `published`, `publication_failures`, `declined`, `exhausted`, `invalid`, `aborted`, and eviction totals, bounded retry, fail-closed snapshots and no deadlock.
- `p138_private_active_builder_snapshot_and_saturating_counters`: snapshot an active builder as `Compiling`, prove builder execution holds neither cache lock, verify settled budgets exclude in-flight debits, and prove every lifecycle aggregate saturates at `u64::MAX`.

## Verification

Activate Rust as described in EXECUTION.md. The P138 runtime cases are available in focused development groups for iteration only; those groups do not count as completion evidence. Full closure runs one unfiltered `register-executor,execution-reference` runtime suite and one unfiltered `compact-executor,register-executor,execution-reference` suite. Each suite must execute all six P138 cases, all P133/P134/P065 regressions, and the full declared runtime tests. The source checker and default/Register build budgets also run once in that full verification phase. Do not separately rerun filtered P138/P133/P134 tests or the checker when producing full-close evidence.

The contract binds the full transitive runtime/test compilation sources, fixture bytes and provenance, source-bound producer/checker, predecessor archive and exact feature settings. The trace lane is semantic/cache-lifecycle evidence only; it must not run inside or feed a timing collection. Use the contract-driven full close as the single required verification execution:

```sh
python3 -B tools/verify.py plan --contract docs/benchmarks/performance/P138-verification.json
python3 -B tools/verify.py close --contract docs/benchmarks/performance/P138-verification.json --output target/performance/P138/complete --execute
```

Require every named fixture case and source-current evidence. Keep P138 pending until snapshots, traces, tests, budget checks, validators and source-bound close pass.

## Measurement and decision

Selection: `register`. Explicit execution setting: `register`. Decision rule: `integrity`.

P138 reports cache lifecycle and retained-budget observations only in explicitly instrumented diagnostic traces. It provides no timing samples, threshold recommendation, candidate decision or promotion. P066 adds P138 as a prerequisite but retains every existing gate; its production measurements must keep diagnostics disabled. P066 remains responsible for measured cold-code distributions and cache churn evidence with instrumentation excluded from timing.

## Before editing

Verify the immutable P137 completion and create a fresh immutable Register archive from its immediate-predecessor source. The archive must identify commit `78c581a72433fb44b8d135eb0b9971a0eab6bd73`; preserve its manifest and source map and never overwrite it. If the checkout differs, record the verified actual predecessor in a new path and update the source-bound contract before runtime edits.

```sh
python3 -B tools/wtime-bench/baseline.py --output target/performance/P138-before --mode register
```

## Completion evidence

P138 closed successfully as `contract_only`. The source-bound proof records 154 runtime source files (map SHA-256 `2964f5992579be9b024f887313dcd0ec3896af9a577d9f55f5f85522cb6ac349`), six named cases, and 88 diagnostic trace rows. The immediate Register archive is pinned to P137 commit `78c581a72433fb44b8d135eb0b9971a0eab6bd73`, with manifest SHA-256 `a70e79157fe54adc7d4a8413f73223fb4eea6c6532a245810fae59d235f2eba1`. Fixture WAT/Wasm hashes are `ec89d4fb76a1c3085bad8f8c490a16b2f771e0ef1abd02c4a4e217e3e067d99a` and `1a2cf9feffcba7b68af7ecf2864ebf08a121f90bed0a229fbd09569368d490c5`.

The full verify passed 319 Register/reference tests, 425 combined-feature tests, one default-policy test, and all six producer checks. Both current build-budget reports passed: default stripped size 1,000,960 bytes and Register 1,090,512 bytes, with zero default-dependency violations. The full-close final evidence is `d6f5744b8156dcf44ba6eda623c407a104da965f7aaa1de2f992be9d9bd71b33`; the source-bound proof is `e8c20587fe00c4a80f1aa156763237201a3983520fc80ec82bf844df878b2543`. Traces remain explicitly instrumented and non-timing; no benchmark samples, performance decision, or candidate promotion were produced.

See [completion](../../../wtime/docs/benchmarks/performance/P138-completion.json), [proof](../../../wtime/docs/benchmarks/performance/P138-proof.json), [freeze](../../../wtime/docs/benchmarks/performance/P138-freeze.json), [evidence](../../../wtime/docs/benchmarks/performance/P138-evidence.json), and the [compressed records](../../../wtime/docs/benchmarks/performance/P138-records.json.gz).

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/P138/shared-finalization/attempt-0002/evidence.json": "d6f5744b8156dcf44ba6eda623c407a104da965f7aaa1de2f992be9d9bd71b33"
  },
  "evidence": [
    {
      "bytes": 1563,
      "path": "/workspace/wtime/target/performance/P138/shared-finalization/attempt-0002/evidence.json",
      "sha256": "d6f5744b8156dcf44ba6eda623c407a104da965f7aaa1de2f992be9d9bd71b33"
    }
  ],
  "outcome": "contract_only",
  "status": "done",
  "task": "P138"
}
```
