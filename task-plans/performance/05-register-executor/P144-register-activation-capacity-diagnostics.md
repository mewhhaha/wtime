# P144 — Report Register activation capacity high-water

Status: **todo**

Outcome: **pending**

Kind: `foundation` · Branch: `portable` · Model: **Sol 6.1 Low**

## Purpose and dependencies

P144 adds an independent no-timer diagnostic for the physical vector capacities owned by active Register frames. P065 protects frame exits and captured state, P066 owns threshold semantics and the gate, P138 separates retained compiled-code bytes, and P141 provides the individual-entry schedule and diagnostic protocol. This report exposes activation storage without changing admission, cache policy, execution semantics, or timed wires.

P144 is not a memory-pool or performance candidate. Its result is a per-invocation capacity high-water, explicitly separate from P138 compiled-artifact retained bytes and from process RSS.

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), and P065, P066, P138, and P141 contracts. The diagnostic state and hooks exist only under `execution-reference`; the default build and default execution path gain no diagnostic storage or work.

- `crates/wtime-runtime/src/execution/register/execute.rs`
- `crates/wtime-runtime/src/execution.rs` (minimal active-frame sampling integration)
- `crates/wtime-runtime/src/register_activation_diagnostics.rs` (new feature-gated diagnostic leaf)
- `crates/wtime-runtime/src/lib.rs` (feature-gated diagnostic wiring)
- `tools/wtime-bench/src/entry_sequence_driver.rs` (new separate no-timer protocol route)
- `crates/wtime-runtime/tests/performance.rs` (focused test registration)
- `crates/wtime-runtime/tests/performance/p144.rs` (new protocol cases)
- `crates/wtime-runtime/tests/performance/p144_private.rs` (new capacity/lifecycle cases)
- `tools/wtime-bench/entry_sequence_policy.py`
- `tools/wtime-bench/tests/test_p141_sequence_policy.py` (preserve all existing schemas)
- `tools/wtime-bench/tests/test_p144_activation_policy.py` (new contained tests)
- `tools/verification/producers/p144.py`
- `tools/verification/tests/test_p144.py`
- `docs/benchmarks/performance/P144-verification.json`

## Input/output contract

Add a separate no-timer activation-capacity diagnostic to the P141 entry-sequence driver. Leave both P141 timing records and its existing cache-lifecycle diagnostic schema unchanged. For each `Store::invoke`, reset the invocation-local diagnostic high-water immediately before invocation, sample actual capacities at source and callback boundaries, and return the maximum concurrent sum after the invocation. A later top-level invocation starts from zero; completed activation storage is not reported as retained capacity. A diagnostic-only scope guard must restore outer active counts on every success, error, and unwind, including nested guest/host reentry. Reset is valid only while no invocation scope is active; an attempted reset during active or nested execution must fail without disturbing the current high-water.

For each active frame with a Register `Activation`, compute owned bytes exactly as `activation.slots.capacity() * size_of::<u64>() + frame.labels.capacity() * size_of::<Label>()`. Sum these values across all simultaneously active Register frames, including nested guest/host reentry, using checked arithmetic. Report the maximum observed live sum at the declared source/callback boundaries for that invocation, plus enough typed frame/count data for the private oracle to verify the formula. This is an observed live-vector-capacity high-water, not the process allocator's transient peak or allocation overhead. A declined/fallback invocation with no Register activation reports zero. Vector length, prepared code size, cache retained-byte charge, continuation payload, allocator padding, and process RSS are not substitutes for these capacities.

The diagnostic is gated by `execution-reference`, has no timestamp or duration fields, and is independently validated from timing observations. Report cache artifact bytes only in their existing P138 field; never add them to activation capacity. The diagnostic must not affect source fuel, calls, fallback decisions, cache eviction/pinning, cancellation, or guest results.

## Implementation recipe

1. Confirm P065/P066/P138/P141 source identities and inspect `Activation`, `Frame`, active frame-stack transitions, and P141's strict diagnostic parser before adding instrumentation.
2. Add feature-gated, Store-local invocation high-water state and the smallest execution hooks needed to sample the exact checked capacity sum at source/callback boundaries across active and nested Register frames. Use a diagnostic-only scope guard to restore parent counts on every exit; reject resets while any scope is active. Keep the state absent when `execution-reference` is disabled and reset it before each top-level diagnostic invocation.
3. Add a separate no-timer entry-sequence diagnostic route and wire field for invocation high-water. Preserve P141 timing and cache-diagnostic schemas; keep P138 retained-code bytes separate and report zero when the invocation used no Register activation.
4. Add private capacity-formula, nested-frame, reset, fallback, eviction, pinned-frame, and cancellation cleanup checks. Ensure high-water never accumulates across invocations and inactive activation vectors are not represented as retained Store memory.
5. Add the named Rust and Python tests below, compile the default and exact diagnostic feature configurations, and close the source-bound P144 contract with no timing or promotion claim.

## Meaningful cases

- `p144_eager_and_threshold_three_report_capacity_high_water_per_invoke` verifies actual slot/label vector capacities for eager Register and threshold-3 activations, with a fresh high-water for every invocation.
- `p144_declined_fallback_reports_zero_register_activation_capacity` proves unsupported or declined frames report zero activation bytes while preserving their existing fallback result and reason.
- `p144_nested_active_frames_sum_owned_vec_capacities_and_reset_per_invoke` compares reported high-water against the independent sum of each live nested Register frame's slot and label Vec capacities, then proves the next invocation starts at zero.
- `p144_eviction_pin_and_cancel_cleanup_keep_capacity_diagnostic_bounded` checks cache eviction/pinning and cancellation do not inflate activation high-water, leak invocation state, or merge it with P138 artifact bytes.

## Verification

Run default runtime tests and the `register-executor,execution-reference` diagnostic tests separately. The producer must reject timed diagnostic records, length-based or cached-code substitutions, arithmetic overflow, incomplete nested-frame samples, stale sources, and any change to existing P141 schemas. All named P065/P134/P138 semantic controls remain required in the diagnostic feature group.

```sh
python3 -B tools/verify.py plan --contract docs/benchmarks/performance/P144-verification.json
python3 -B tools/verify.py close --contract docs/benchmarks/performance/P144-verification.json --output target/performance/P144/complete --execute
```

## Measurement and decision

Selection: `register`. Execution setting: `register`. Decision rule: `integrity`.

P144 provides physical activation-capacity diagnostics only. It emits no timing evidence, candidate decision, threshold recommendation, or promotion, and does not change runtime policy.

## Completion evidence

Bind default/diagnostic feature groups, test counts, source and binary identities, exact per-invocation capacity formula and reset semantics, nested-frame samples, fallback-zero checks, and separate P138 artifact-byte fields. Record `timing_evidence:false` and `candidate_promoted:false`.
