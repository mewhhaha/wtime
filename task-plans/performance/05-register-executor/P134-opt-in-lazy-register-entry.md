# P134 — Integrate opt-in lazy Register entry

Status: **done**

Outcome: **contract_only**

Kind: `foundation` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P133 — Add a bounded lazy Register preparation cache](P133-bounded-lazy-register-cache.md)

P133 supplies the bounded source-owned cache and single-lease compilation API. P134 wires that cache into an explicit Register Engine construction path and frame entry. P066 remains the measured candidate gate, retains every existing prerequisite and verification requirement, and adds P134 as a dependency. P066 owns the required 1/10/100/10000-call comparisons and all performance decisions.

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/lib.rs`
- `crates/wtime-runtime/src/execution.rs`
- `crates/wtime-runtime/src/execution/register/mod.rs`
- `crates/wtime-runtime/src/execution/register/cache.rs`
- `crates/wtime-runtime/src/execution/register/execute.rs`
- `crates/wtime-runtime/tests/performance/p134.rs`
- `crates/wtime-runtime/tests/performance/p134_private.rs`
- `crates/wtime-runtime/tests/performance.rs` (bounded test registration)

One integrator owns Engine and ModuleData definitions, frame and dispatch integration, Register module exports, and test registration. Keep the change to at most two focused patches: opt-in Engine/module-cache/frame integration, then focused semantic tests. P134 does not add a public Config field, a new `ExecutionPolicy` variant or protocol mode, a benchmark harness, or a performance claim.

## Input/output contract

Add an opt-in Engine constructor such as `Engine::with_register_entry_threshold(config, threshold)` that selects the existing `ExecutionPolicy::Register` and stores a validated positive threshold privately. The existing `Engine::new` and `Engine::with_execution_policy(config, ExecutionPolicy::Register)` remain eager and behave exactly as before. The default policy remains Decoded.

For the opt-in constructor, `Engine::compile` stores a P133 lazy cache in the ModuleData owned by that module and does not create the eager whole-module `PreparedModule`. The cache remains shared through cloned `Module` values and across Stores using that Module; it does not retain `Arc<ModuleData>`. Keep the P133 engine/source owner check on lookup and publish.

At each new function entry, `Store::frame` performs one cache lookup for the function body. Before the hot threshold, a cold body starts decoded with no Register activation. At threshold, a completed cached function may initialize a Register `Activation` at PC 0. The `Activation` owns one `Arc<CachedFunction>` pin; `register::step` borrows the pinned function and never clones an `Arc` per opcode. Calls construct a new frame and count as new entries. A running frame is never promoted later, and continuation restoration never replays entry lookup, compiles, or restarts the frame.

When a Register prefix reaches its decoded handoff, it materializes once and drops the `Activation`, which releases its cache pin unless another live activation reference exists. An activation that is cloned or moved while still live naturally retains its own `Arc` pin across cache eviction. The P065 suspend/capture barrier first performs the decoded handoff, so the actual captured frame contains saved decoded state and no active Register pin; resumption continues that state without entry lookup, recompilation, or reactivation. Cache preparation itself does not charge guest fuel, check cancellation at a different source boundary, execute host code, repeat a side effect, mutate guest state, or alter trap and result behavior. No per-Store cache or duplicate compilation is introduced.

## Implementation recipe

1. Add a feature-gated Engine field for the optional Register entry threshold and a private constructor path that selects existing `ExecutionPolicy::Register`. Reject a zero threshold. Preserve `Engine::new` and the existing eager `with_execution_policy(..., Register)` branch; introduce no Config knob or protocol enum.
2. In `Engine::compile`, create/store a threshold-configured P133 cache for the opt-in path and skip eager `PreparedModule::prepare` only for that constructor. Store the cache on `ModuleData`, shared by all Module/Store clones and bound to the existing engine ID and source Arc. Keep eager preparation for the current Register constructor.
3. At `Store::frame`, make one cache lookup per body entry. On a ready result, create the existing activation from the cached `PreparedFunction` at PC 0 and move the returned `Arc<CachedFunction>` into that `Activation`. Keep activation and source values frame-local; `register::step` borrows the pinned function without cloning an Arc per opcode. If the lookup declines, use the unchanged decoded path for the whole frame.
4. Preserve activation-owned pinning through normal Register execution. At P064 decoded handoff, materialize once and drop the `Activation`; do not keep an obsolete pin on the decoded frame. If a live activation is cloned or moved into a captured segment, its Arc keeps the P133 metadata charged through eviction. For the actual P065 suspend/capture barrier, verify that handoff drops the activation before capture and that restoration continues decoded state without a new entry count, preparation attempt, OSR, or replay.
5. Use existing execution-reference diagnostics to distinguish cold fallback from hot Register dispatch, and add only narrowly scoped test diagnostics if needed to prove compilation/promotion counts and cross-Store sharing. Compare guest outputs, source fuel, cancellation, callbacks, memory, capture state, and continuation budgets against the eager Register and decoded controls.
6. Register the four P134 semantic cases and retain default and combined-feature coverage. Leave 1/10/100/10000-call timing and threshold-selection measurements to P066.

## Meaningful cases

- `p134_cold_hot_entry_threshold_and_diagnostics`: a lazy Register Engine compiles without an eager Register artifact; a real admitted body falls back below threshold, promotes at the exact threshold, and dispatches Register code on later entries. Existing diagnostics prove cold fallback, hot dispatch, one promotion, no per-opcode Arc cloning, and no mid-frame OSR.
- `p134_shared_module_concurrency_and_store_ownership`: two independent Stores use the same Module and share one cache/compile lease under concurrent hot entries; the function is prepared once, while each Store builds its own frame, locals, labels, and memory binding checks. Calls through a foreign engine/source use that module’s owner-bound cache or fail closed.
- `p134_host_effect_fuel_and_foreign_refusal`: a cached full Register prefix reaches a call/host decoded boundary exactly once; the callback, memory changes, traps, fuel result, and cancellation boundary match eager Register/decoded controls. A foreign engine/source owner cannot activate the cached body.
- `p134_eviction_pin_and_continuation_resume_without_reentry`: a private frame test holds a live `Activation` pin while FIFO eviction removes its cache entry, then proves the artifact stays valid and charged until the activation drops. A separate public numeric P065 capture/resume test proves the suspend barrier materializes and drops the activation before capture, resumes the saved decoded state without entry lookup or recompilation, and preserves one-shot behavior and source fuel.

## Verification

Activate Rust as described in [EXECUTION.md](../EXECUTION.md). Run P134 under Register and reference features, retain P133 cache and P065 capture/exit regression checks, and check default plus combined Register builds. Test both opt-in lazy construction and the existing eager Register constructor. Tests must prove the requested path executed and compare actual values/effects/fuel/capture behavior; no timing result is required here.

```sh
cargo test --locked -p wtime-runtime --features register-executor,execution-reference p134_ -- --nocapture
cargo test --locked -p wtime-runtime --features register-executor,execution-reference p133_ -- --nocapture
cargo test --locked -p wtime-runtime --features register-executor,execution-reference p065_ -- --nocapture
cargo check --locked -p wtime-runtime
cargo check --locked -p wtime-runtime --features register-executor,execution-reference
```

## Measurement and decision

Selection: `register`. Explicit execution setting: `register`. Decision rule: `integrity`.

P134 is an opt-in integration foundation and makes no speed claim. P066 owns all candidate comparisons, full correctness/build gates, call-count totals, and policy decisions.

## Before editing

Verify the immutable P001 archive and completed P133 correctness record. Create a fresh immutable Register archive for the immediate P133-successor source before edits, recording exact source identity and hashes. Do not overwrite an archive with different identities.

```sh
python3 -B tools/wtime-bench/baseline.py --output target/performance/P134-before --mode register
```

## Completion evidence

Record P001 and P134-before identities, source/test/artifact hashes, exact commands and feature settings, cold/hot fallback and dispatch counts, compile/promotion counts, per-Store shared-cache results, guest values and source fuel, callback and memory effects, cancellation and trap outcomes, decoded continuation source/capture state, active-Activation pin behavior through eviction, and pinned byte/work ledger values. Keep status pending until each named semantic case and feature check passes. No timing claim is made.

## Executed evidence

The successful full closure is `target/performance/P134/shared-finalization/attempt-0001` (close 0089; freeze attempt 0001; verify attempt 0001). Freeze and verification passed as `contract_only`, with timing evidence disabled. Verification ran 733 tests: 313 Register/reference, 419 combined-feature, and one default-policy test; all passed with zero failures or ignored tests. All five P134 proof cases passed, alongside the four P133 and six P065 regression cases. Two Stores shared one Module cache and one preparation work debit; the live activation retained 1,054 metadata bytes through eviction, and continuation resume performed zero entry lookups. Default and optional Register builds passed dependency and hard-budget checks at 1,000,960 and 1,090,512 stripped bytes; their clean-build observations were 2.746473179 seconds and 3.308659858 seconds. The entry constructor remains opt-in, the ordinary Register constructor remains eager, and no speed claim is made. Durable evidence is indexed in `docs/benchmarks/performance/P134-records.json`; design notes are in `docs/architecture/register-entry.md`.

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/P134/shared-finalization/attempt-0001/evidence.json": "e03bc0fa5d417a131158858f56739749b3fdf4eb4e4857db52f7b02c2bcd30c5"
  },
  "evidence": [
    {
      "bytes": 1570,
      "path": "/workspace/wtime/target/performance/P134/shared-finalization/attempt-0001/evidence.json",
      "sha256": "e03bc0fa5d417a131158858f56739749b3fdf4eb4e4857db52f7b02c2bcd30c5"
    }
  ],
  "outcome": "contract_only",
  "status": "done",
  "task": "P134"
}
```
