# P133 — Add a bounded lazy Register preparation cache

Status: **done**

Outcome: **contract_only**

Kind: `foundation` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P132 — Extract transactional per-function Register preparation](P132-extract-transactional-register-preparation.md)

P132 supplies the bounded `Preparation` transaction and per-function prepared artifact. Its P065 dependency carries the Register exit/capture correctness gate. P066 remains the measured production integration and keeps its full existing prerequisites and gates; P066 must depend on P133.

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/execution/register/cache.rs` (new)
- `crates/wtime-runtime/src/execution/register/mod.rs`
- `crates/wtime-runtime/src/execution/register/execute.rs` (minimal `pub(super)` preparation and owner accessors)
- `crates/wtime-runtime/tests/performance/p133_private.rs` (included by cache tests)

One integrator owns `lib.rs`, `execution.rs`, dispatch, Cargo features, and `tests/performance.rs`. P133 adds a private standalone cache API for P066; it does not add the cache to `ModuleData`, `Engine`, or `Store`, and it does not change eager `PreparedModule` preparation or guest execution.

## Input/output contract

Add a safe, private, per-module/body cache that can return an immutable `Arc<CachedFunction>` containing one P132 `PreparedFunction` and a retention token. Bind the cache at construction to the module’s engine ID and exact decoded source `Arc<[u8]>`; validate both before every lookup and again before publish. Use the decoded body index as the cache key. The cache retains the source `Arc` and engine ID only, never `Arc<ModuleData>`, so it cannot create an ownership cycle.

Each body has a saturating entry counter and one of `Cold`, `Compiling`, `Ready`, `Cooldown`, or `Negative`. A private fixed hot threshold starts one preparation attempt. The cache permits one compile lease per module at a time; duplicate calls and other bodies encountered while the lease is busy fall back immediately. The lease reserves the existing P132 remaining attempted-work, retained-work, and metadata budgets. Preparation runs after releasing cache and canonical-type locks. Only a fully validated P132 `Complete` outcome can publish `Ready`; `Declined`, `Exhausted`, `Invalid`, owner mismatch, mutex poison, and lease abort publish no partial function.

Bound the cache to at most 64 indexed bodies and the existing 100,000-work and 1 MiB metadata limits. Account actual capacities and all cache index, entry, `Arc` control/header, wrapper, token, and prepared-function storage against the metadata limit. Use checked arithmetic and fallible reservation for vectors before publication. A deterministic FIFO evicts the oldest eligible entry; an in-flight `Compiling` entry cannot be evicted. Evicting a ready entry drops the cache’s `Arc`; if a caller still holds an `Arc<CachedFunction>`, its retained bytes stay charged until that last reference drops. Use a separate `Arc<Mutex<Ledger>>` for retention tokens; the ledger must never hold cache entries or cached-function references. Dropping a retention token releases retained metadata bytes only. Attempted and retained work are never refunded, including after eviction. If the ledger is poisoned, keep the charge conservatively and fail closed rather than risk undercounting.

Treat a generic P132 `Declined` as a bounded unknown/transient result: wait at least 128 further body entries before retry, allow at most two total attempts, then transition to `Negative`. `Exhausted`, `Invalid`, malformed body/plan input, and foreign module ownership fail closed without retrying each call. Saturating counters never wrap. Poisoned locks return fallback/fail-closed results without panicking.

The API performs no guest instruction, fuel charge, trap, side effect, call transition, or policy selection. It only prepares and retains immutable metadata. P066 owns later runtime integration, threshold measurement, and policy decisions.

## Implementation recipe

1. Add a private cache module and expose only the minimum P132 preparation types/functions to the sibling module with `pub(super)`: `PreparedFunction`, the four-outcome `Preparation`, and `prepare_function`. Add narrow accessors for the engine ID and decoded source `Arc`; do not expose module internals or make the cache public.
2. Implement a source-bound cache with fixed maximum entry capacity, saturating per-body counters, the five declared states, deterministic FIFO ordering, and a single RAII compile lease. A lease reserves the P132 remaining budget while active, releases all locks before `prepare_function`, and commits its outcome atomically after rechecking source and engine ownership. Duplicate and other-body calls while a lease is active return fallback.
3. Keep cache metadata accounting conservative and checked. Include actual vector capacity, cache/index state, cache entry storage, `Arc` header allowance, `CachedFunction` wrapper, retention token, and all P132-prepared function allocations. A separate ledger tracks live retained bytes and monotonic work; a token drop releases bytes only, never work. Eviction drops the cache reference while active `Arc` holders remain valid and charged.
4. Map P132 outcomes to states without partial publication: `Complete` may become `Ready`; `Declined` enters the declared cooldown; `Exhausted` and `Invalid` become fail-closed `Negative`; owner mismatch and poisoned state fail closed; dropping an unfinished lease clears `Compiling`, publishes nothing, preserves any already-consumed work, and prevents immediate retry.
5. Add the private cache tests in `p133_private.rs` and include them from `cache.rs`. Do not add Engine/Store integration, guest execution changes, policy selectors, or performance claims. Keep the task to at most two focused changes: the bounded cache/lease and the focused tests.

## Meaningful cases

- p133_cold_threshold_and_single_publish: a real admitted module stays cold below the declared threshold; the threshold entry creates one immutable cached artifact, and later lookup returns that same prepared result without another attempt.
- p133_concurrent_duplicate_uses_one_unlocked_builder: a barrier-controlled real-module attempt proves one builder owns the lease, duplicate and other-body calls fall back, and the builder runs while both cache and canonical-type locks are available.
- p133_decline_poison_foreign_and_aborted_lease_fail_closed: unsupported/declined preparation cannot publish partial state, retries wait at least 128 entries and stop after two attempts, malformed or foreign owners fail closed, poisoned locks do not panic, and an aborted lease clears `Compiling` without immediate retry.
- p133_fifo_eviction_pinned_lifetime_and_budget_churn: deterministic FIFO stays within 64 entries and metadata bounds; an evicted function remains valid and charged while a test-held `Arc` exists, its last drop releases bytes but not work, and repeated cooldown/eviction churn cannot reset the work budget.

## Verification

Activate Rust as described in [EXECUTION.md](../EXECUTION.md). Run the P133 private tests with Register and reference features, P132 regression tests, and default/combined Register build checks. Use at least four real compiled Wasm module fixtures across the cases, including eligible functions, an ineligible body, a foreign-engine/source module, and a module with enough bodies to exercise the 64-entry cap. Tests must prove the P132 builder ran exactly once on the success path and that cache-only work does not charge guest fuel or alter execution policy.

```sh
cargo test --locked -p wtime-runtime --features register-executor,execution-reference p133_ -- --nocapture
cargo test --locked -p wtime-runtime --features register-executor,execution-reference p132_ -- --nocapture
cargo check --locked -p wtime-runtime
cargo check --locked -p wtime-runtime --features register-executor,execution-reference
```

## Measurement and decision

Selection: `register`. Explicit execution setting: `register`. Decision rule: `integrity`.

This is a bounded cache foundation with no timing or promotion claim. It leaves eager module preparation and default execution unchanged. P066 owns measured thresholds, policy wiring, compile savings, cold behavior, and production integration under its complete existing gates.

## Before editing

Verify the immutable P001 archive and completed P132 correctness record. Create a fresh immutable Register archive for the immediate P132-successor source before runtime edits; record source identity and hashes, and never overwrite an archive with different identities.

```sh
python3 -B tools/wtime-bench/baseline.py --output target/performance/P133-before --mode register
```

## Completion evidence

Record P001 and P133-before identities, exact feature settings and commands, focused test counts, source/test/artifact hashes, cache entry and state transitions, builder/duplicate counts, work and byte ledger values, lease-abort outcomes, and pinned-`Arc` byte-release behavior. Keep task status pending until all semantic and budget checks pass. No timings are acceptance evidence.


## Executed evidence

The successful full closure is `target/performance/P133/shared-finalization/attempt-0002` (close 0088; freeze attempt 0002; verify attempt 0001). Freeze and verification passed as `contract_only`, with timing evidence disabled. The verification executed 723 tests: 308 Register/reference, 414 combined-feature, and one default-policy test; all passed with zero failures or ignored tests. Four proof records passed, including single publication, unlocked duplicate fallback, fail-closed retry/lease behavior, and 64-entry FIFO eviction with 66 bodies. The held artifact retained 1,054 charged bytes through its final drop. Both default and optional Register builds passed dependency-closure and hard budgets (1,000,960 and 1,078,992 stripped bytes respectively). The cache remains standalone and unwired to Engine/Store; no timing or performance promotion is claimed. Durable evidence is indexed in `docs/benchmarks/performance/P133-records.json`, with design notes in `docs/architecture/register-cache.md`.

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/P133/shared-finalization/attempt-0002/evidence.json": "a52be75954ba51759c88cfa10971136885947db484304ce17a6e737e878284e7"
  },
  "evidence": [
    {
      "bytes": 1535,
      "path": "/workspace/wtime/target/performance/P133/shared-finalization/attempt-0002/evidence.json",
      "sha256": "a52be75954ba51759c88cfa10971136885947db484304ce17a6e737e878284e7"
    }
  ],
  "outcome": "contract_only",
  "status": "done",
  "task": "P133"
}
```
