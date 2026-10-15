# P143 — Forward explicit Register thresholds through conformance

Status: **done**

Outcome: **contract_only**

Kind: `foundation` · Branch: `portable` · Model: **Sol 6.1 Low**

## Purpose and dependencies

P143 lets the existing register-tier state and source-fuel checker exercise the explicit adaptive Register threshold used by P066. P065 supplies exit/root semantics, P066 defines the candidate gate and frozen threshold 3, and P138 provides the existing diagnostic capability and optimized/fallback evidence. P143 changes only conformance and verification plumbing; it does not change runtime policy or select a threshold.

P067 remains the milestone gate. Its full semantic conformance scope still covers every declared family under explicit Register. The optional adaptive timing subset is limited to scalar and representative rows plus cold/entry totals, as declared by P067. P143 supplies source-bound tier/state conformance for decoded/default, eager Register, and Register threshold 3; do not add adaptive timing drivers for memory or SIMD here.

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), the P065, P066, and P138 contracts, and semantic contracts **C1, C2, C3, C4, C5, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md). Keep the runtime API unchanged. Limit `tools/wtime-bench/src/protocol.rs` changes to the registration and wire fields needed by this route, including typed-case threshold/fuel forwarding.

- `tools/wtime-bench/tier_check.py`
- `tools/wtime-bench/src/entry_sequence_driver.rs` (diagnostic Register APIs require both `execution-reference` and `register-executor`; verify the no-Register diagnostic build as part of tier-check)
- `tools/wtime-bench/src/entry_batch_driver.rs` (diagnostic Register APIs require both `execution-reference` and `register-executor`; verify the no-Register diagnostic build as part of tier-check)
- `tools/wtime-bench/src/tier_state_driver.rs` (state/fuel route)
- `tools/wtime-bench/src/protocol.rs` (bounded protocol integration, including typed-case forwarding)
- `tools/wtime-conformance/src/adapter.rs`
- `tools/wtime-conformance/src/cli.rs`
- `tools/wtime-conformance/src/report.rs`
- `tools/wtime-conformance/src/p143_tests.rs` (new focused test leaf, included from `cli.rs`)
- `tools/wtime-bench/tests/test_p143_tier_threshold.py`
- `tools/verification/producers/p143.py`
- `tools/verification/tests/test_p143_threshold_gate.py`
- `docs/benchmarks/performance/P143-verification.json`

## Input/output contract

Add one optional `--register-entry-threshold N` setting to the Register tier state/source-fuel check. Accept a positive `u64` only when the selected candidate tier is `register`; reject zero, malformed or overflowing values, duplicate options, and every non-Register use before fixture reads or child-process launch. Omission preserves existing default/eager protocol bytes, behavior, and report values; additive policy metadata identifies the effective strategy and optional threshold. The foundation route exercises threshold 3, matching P066's existing predeclaration; it makes no threshold recommendation.

Forward the setting through the conformance CLI, Adapter, state/fuel driver, report, and replay identity. The effective policy is explicit in reports as strategy and threshold: decoded/default and eager/omitted remain distinguishable from adaptive/Register/3. Replays must use the same setting and current source identities. A report binds the current corpus lock, exact selected corpus file count and hashes, semantic-case identities, binaries, and argv. Keep existing fixture/state outputs and fuel accounting intact.

For every selected P014 state and source-fuel case, compare decoded/default, eager Register with omission, and adaptive Register threshold 3 against the independent expected state. Validate full memory images, globals, tables/reference graph, outcomes, source cursor, and fuel prefixes. Require source-bound evidence of actual optimized Register execution and actual fallback coverage; a candidate that only reports fallback is not coverage. This is conformance evidence, not timing evidence. P067 retains full semantic conformance across all declared families under explicit Register; its optional adaptive timing subset is scalar and representative rows plus cold/entry totals. Do not add adaptive timing routes for memory or SIMD families.

## Implementation recipe

1. Verify the P065, P066, and P138 prerequisite records and inspect the current tier checker, conformance option parser, Adapter, report, replay path, and optional-feature boundaries before editing.
2. Add strict optional threshold parsing and forward the value through the tier state/source-fuel driver and conformance protocol. Preserve byte-identical omission behavior; reject invalid and non-Register options before opening fixtures or spawning a process. Extend the typed-case route to forward optional fuel and entry settings through the existing threshold-aware engine constructor at every stage, and echo the effective settings.
3. Run the complete selected P014 state and fuel-prefix cases against decoded/default, eager Register with omitted threshold, and adaptive Register threshold 3. Bind actual optimized and fallback evidence, full state oracles, source-fuel results, and current corpus counts/hashes.
4. Bind strategy, threshold, argv, binary/source maps, corpus lock, selected file count, and fixture hashes through report and replay; reject stale corpus or changed policy on replay.
5. Add the named Rust and Python tests below; check the bench drivers in their no-Register `execution-reference` configuration to prove diagnostic APIs are gated on both required features; then run default and exact Register feature groups and verify a full source-bound P143 contract close with no timing claim or threshold decision.

## Meaningful cases

- `p143_threshold_option_requires_one_positive_u64` rejects invalid and duplicate threshold values.
- `p143_non_register_threshold_is_rejected_and_default_stays_decoded` proves non-Register use is rejected and omission preserves decoded/eager defaults.
- `p143_report_identifies_threshold_and_strategy` verifies the additive policy fields distinguish eager omission from adaptive threshold 3.
- `p143_adapter_threshold_promotes_on_third_entry_and_preserves_fallback` verifies threshold-3 activation and fallback behavior.
- `test_p143_typed_case_forwards_only_register_and_checks_echo` checks typed-case forwarding and echoed policy.
- `test_p143_omitted_threshold_accepts_legacy_missing_field_but_rejects_nonnull` preserves legacy omission handling.
- `test_rejects_wrong_or_missing_top_level_threshold_and_strategy` rejects malformed or stale producer reports.

## Verification

Run the full P143 contract with its declared default and Register feature configurations; it must require all named state/fuel and policy tests, full selected-case counts, current corpus hashes, and nonzero optimized and fallback evidence. The tier checker must also compile the entry sequence and batch diagnostic drivers with `execution-reference` but without `register-executor`, proving Register-only diagnostics are correctly cfg-gated. P143 emits no guest timings, recommendation, performance decision, or promotion.

```sh
python3 -B tools/verify.py plan --contract docs/benchmarks/performance/P143-verification.json
python3 -B tools/verify.py close --contract docs/benchmarks/performance/P143-verification.json --output target/performance/P143/complete --execute
```

## Measurement and decision

Selection: `register`. Execution setting: `register`. Decision rule: `integrity`.

P143 is a source-bound conformance foundation. It adds no performance samples, threshold selection, or candidate promotion. P067 remains the only milestone decision gate for the register portfolio.

## Completion evidence

Bind phase receipts, exact commands/features and test counts, current corpus lock and selected file counts/hashes, source and executable hashes, full state/fuel summaries, and actual optimized/fallback counts. Record `timing_evidence:false`, `candidate_promoted:false`, and no threshold decision.

## Final result

P143 completed as **contract_only** with 72 passing Rust/Python tests and zero failures or ignored cases. The source-bound tier checker completed 89 checks each for decoded, eager Register, and adaptive Register at threshold 3, each against the reference dispatcher. Every run retained 68 full-state source-fuel probes, three typed cases, three state cases, four reference-types/interactions corpus executions, cursor/snapshot checks, and decoded dispatch diagnostics. The selected conformance counts and corpus identities match across all three policies. This is the selected integration matrix; the final exhaustive pinned-corpus refresh remains P109.

Optional threshold values are validated before work, forwarded through typed/state and conformance paths, echoed into reports, and preserved by replay arguments. Omission preserves execution and existing data fields with additive policy metadata. The adapter test proves zero optimized dispatches for the first two entries and real optimized dispatch on entry three; a vector-returning body proves fallback remains active. Invalid, zero, overflowing, duplicate, and non-Register settings are refused. The core crate sources and default decoded policy match the predecessor.

The full workflow exposed pre-existing missing Register feature guards in the entry-batch and entry-sequence diagnostic drivers; those guards now require both Register and execution-reference. A direct counter-only/no-Register compilation check covers that configuration before the slower tier runs. Earlier attempts and a passing two-mode closure remain retained; the final closure includes the additional decoded comparison. No guest performance measurements or threshold recommendation are claimed; typed-protocol duration fields are diagnostic only.

Artifacts: [P143-completion.json](../../../wtime/docs/benchmarks/performance/P143-completion.json), [P143-evidence.json](../../../wtime/docs/benchmarks/performance/P143-evidence.json), [P143-freeze.json](../../../wtime/docs/benchmarks/performance/P143-freeze.json), [P143-proof.json](../../../wtime/docs/benchmarks/performance/P143-proof.json), [P143-decoded-tier.json](../../../wtime/docs/benchmarks/performance/P143-decoded-tier.json), [P143-explicit-tier.json](../../../wtime/docs/benchmarks/performance/P143-explicit-tier.json), [P143-adaptive-tier.json](../../../wtime/docs/benchmarks/performance/P143-adaptive-tier.json), [P143-records.json.gz](../../../wtime/docs/benchmarks/performance/P143-records.json.gz).

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/P143/shared-finalization/attempt-0005/evidence.json": "5fc5e9f5ec2b43360578d12b46cf611237433092b438e488ac775539d640f55d"
  },
  "evidence": [
    {
      "bytes": 1962,
      "path": "/workspace/wtime/target/performance/P143/shared-finalization/attempt-0005/evidence.json",
      "sha256": "5fc5e9f5ec2b43360578d12b46cf611237433092b438e488ac775539d640f55d"
    }
  ],
  "outcome": "contract_only",
  "status": "done",
  "task": "P143"
}
```
