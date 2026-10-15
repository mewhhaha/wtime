# P140 — Freeze and attest the adaptive Register entry threshold

Status: **done**

Outcome: **contract_only**

Kind: `foundation` · Branch: `portable` · Model: **Sol 6.1 Low**

## Purpose and dependencies

P140 adds an explicit, source-bound way for the standard scalar and single-case focused-stage protocol to run the Register engine with a chosen positive entry threshold. P139 provides separate baseline and candidate policy settings for fresh-entry collection. P140 provides the corresponding standard focused-stage setting needed by P066, whose existing semantic, focused, cold, churn, build-budget, and promotion gates remain intact.

P140 depends on **P127** for the established source-fuel argument and wire contract, and **P139** for the source-bound Register evidence lane. P140 is a prerequisite for **P066**. P140 changes the benchmark harness and evidence tooling only; it does not edit runtime/core crates or make a threshold recommendation.

## Scope and ownership

Own the paths listed below. Keep the implementation to two coordinated patches: Rust argument/configuration/wire support, then Python freeze/collection/gate/attestation support. Do not edit `crates/`, `tools/wtime-runtime`, Cargo features, runtime policy, or the P066 selection itself.

- `tools/wtime-bench/src/fuel_policy.rs`
- `tools/wtime-bench/src/protocol.rs`
- `tools/wtime-bench/src/scalar_driver.rs`
- `tools/wtime-bench/src/p140_tests.rs`
- `tools/wtime-bench/compare.py`
- `tools/wtime-bench/focused_compare.py`
- `tools/wtime-bench/selection_manifest.py`
- `tools/wtime-bench/candidate_gate.py`
- `tools/wtime-bench/execution_identity.py`
- `tools/wtime-bench/p140_entry_policy.py`
- `tools/wtime-bench/tests/test_p140_entry_policy.py`
- `tools/wtime-bench/tests/test_p140_bindings.py`
- `tools/verification/production_attestation.py`
- `tools/verification/producers/p140.py`
- `tools/verification/producers/p140_driver_tests.py`
- `docs/benchmarks/performance/P140-contract.json`
- `docs/benchmarks/performance/P140-verification.json`
- `docs/benchmarks/performance/P140-selection.json`
## Input/output contract

Add optional positive-u64 --register-entry-threshold N to the standard scalar and --single-case focused-stage protocols, composable with --source-fuel in either order. Reject zero, malformed, duplicate, overflow, and non-Register combinations before fixture I/O. Apply the value only to the candidate; preserve baseline argv/wire and all omitted-threshold bytes. Freeze optional selection_manifest execution_settings {register_entry_threshold:{baseline:null,candidate:N}}; bind and revalidate the setting in the freeze receipt, candidate-only focused argv/wire/report, candidate gate, and uninstrumented Register production capability/attestation. Preserve fuel semantics, legacy manifests, P136/P137 entry-batch schema, and all P066 gates. No Rust core/runtime edits, environment/global setting, timing decision, or threshold promotion. P140 is prerequisite to P066.

The candidate `--protocol` JSON advertises the strict Register threshold capability contract with schema version, mode, and inclusive `u64` bounds. Production attestation binds the retained candidate protocol, report capability, production record, and frozen setting. It does not require the older baseline to advertise this capability because the baseline receives no threshold argument.

Source fuel remains independently configurable and composable. Baseline and candidate source-fuel behavior continues to follow the existing P127 shared setting; P140 does not change fuel semantics or allow threshold selection to alter source fuel.

Selection manifests may add exactly the optional `execution_settings` object declared in the contract. Existing manifests without this field remain valid and preserve their prior frozen representation. P136/P137 entry-batch policy rows remain a separate protocol and are unchanged.

## Implementation recipe

1. Verify and preserve the immutable P140-before archive and prerequisite receipts; inspect the actual Rust parser/protocol/scalar-driver path and Python selection/focused-comparison/gate/attestation path before editing.
2. Extend the Rust harness argument value to parse source fuel and positive-u64 Register entry threshold in either order, at most once each. Apply the threshold only through the existing Engine::with_register_entry_threshold API on standard Register focused paths; reject unsupported modes before reading fixtures.
3. Preserve byte-identical omitted-threshold output and the existing P127 source-fuel-only output. Add an explicit candidate wire field only when a threshold was requested and advertise a strict Register threshold capability contract in production --protocol output.
4. Add optional, exact-shape execution_settings to selection manifests and frozen receipts. Existing manifests without that key remain valid and retain their legacy identity behavior; threshold values reject booleans, zero, overflow and unknown fields.
5. Forward the frozen threshold only to candidate scalar/single-case observations, validate candidate capability/argv/raw wire/report setting against it, and retain baseline effective setting as null without changing baseline command or output.
6. Extend candidate_gate and production_attestation to bind the frozen setting, production protocol/capability and every candidate observation. Do not weaken semantic, protected/held-out, cold, build/dependency, sample-count, statistical, or promotion checks.
7. Add at least three Rust harness cases and three contained Python identity/gate/attestation cases. Run a source-bound one-stage Register smoke as contract_only evidence with timing_evidence:false and gate_eligible:false; do not analyze or claim performance.

## Meaningful cases

- p140_parser_composes_entry_threshold_and_source_fuel: both option orders preserve typed values and old omitted/fuel-only representations.
- p140_actual_adaptive_engines_preserve_threshold_and_fuel: a tiny real scalar module exposes the requested threshold in its module cache snapshot and proves actual Register dispatch with an independent result oracle.
- p140_bad_entry_settings_are_rejected_before_fixture_io: zero, malformed, duplicate, overflow and unsupported-mode inputs fail before a missing fixture is opened.
- p140_omitted_threshold_preserves_legacy_wire: omitted threshold is byte-identical, source-fuel-only JSON remains unchanged, and fuel composes with threshold without semantic change.
- test_compare_forwards_threshold_after_fuel_and_keeps_baseline_legacy: threshold is appended after source fuel only to candidate argv and candidate wire; baseline argv/wire remain legacy.
- test_focused_request_rejects_mismatch_omission_and_nonregister_before_process: mismatch, omission, and non-Register selection fail before archive work or process launch.
- test_selection_manifest_and_gate_binding_reject_setting_command_and_wire_drift: frozen settings, CLI, report, command, raw wire, and capability must agree.
- test_runtime_attestation_requires_adaptive_register_capability: retained uninstrumented Register production must advertise the exact threshold capability.
- test_threshold_contract_and_integer_domain_are_exact: accept the inclusive positive-u64 domain and reject booleans, zero, negatives, overflow, strings, floats, and null.
- test_declaration_accepts_only_exact_candidate_only_shape: accept absent settings or the exact baseline-null/candidate-positive shape and reject unknown keys and invalid values.
- test_request_enforces_mode_frozen_match_and_omission_distinction: enforce Register mode, frozen-value equality, and the distinction between omitted and explicitly configured thresholds.
- test_wire_candidate_only_legacy_and_capability_tamper: preserve legacy wire when omitted and reject candidate wire or protocol capability drift.
## Verification

The source-bound verification contract owns the exact Rust/Python test invocations, feature grouping, retained P139 archive checks, and one-stage Register smoke. Its Python suite runs all 71 named tests from the eight compatibility modules and P140 binding tests; the Rust driver report requires at least 15 actual Cargo tests including all four P140 cases. Drive the work only through its plan, focused development groups, and final close commands. The smoke validates configuration and output binding and remains contract-only evidence.

```sh
python3 -B tools/verify.py plan --contract docs/benchmarks/performance/P140-verification.json
python3 -B tools/verify.py dev --contract docs/benchmarks/performance/P140-verification.json --group rust-harness --execute
python3 -B tools/verify.py dev --contract docs/benchmarks/performance/P140-verification.json --group python-policy --execute
python3 -B tools/verify.py close --contract docs/benchmarks/performance/P140-verification.json --output target/performance/P140/complete --execute
```

The P140-before archive is pinned at `target/performance/P140-before`: commit `95b5e89756bfc9612aff3057aaa494c74cc67c30`, manifest SHA-256 `2b5adfc824b1e1ac3dbab65e3b1cef369e2267d2116b9cb803db7bc1242f7010`. Preserve it and all P139/P138 receipts. The smoke must exercise actual Register execution with an explicit threshold and retain candidate command, raw wire, protocol capability, report settings, and source/binary hashes. Label it `contract_only`, `timing_evidence:false`, and `gate_eligible:false`; do not compute a candidate decision, promote a threshold, or describe it as performance evidence.

## Completion evidence

Bind the immutable P139 predecessor, actual harness/core feature set, exact positive-u64 capability contract, parser and wire tests, Python test counts, frozen per-side setting, candidate-only argv/wire, baseline legacy identity, retained protocol/report/production attestation and source/binary/fixture hashes. Require a real one-stage Register smoke labeled contract_only, timing_evidence:false and gate_eligible:false. P066 retains every existing correctness, build/dependency, focused-stage, mandatory control, cold/churn, statistical and promotion gate.

When complete, record the immutable predecessor identity, actual harness/core feature set, exact positive-u64 capability contract, parser and wire test counts, Python test counts, frozen per-side setting, candidate-only argv/wire, baseline legacy identity, retained protocol/report/production attestation, and source/binary/fixture hashes. P066 retains every existing correctness, build/dependency, focused-stage, mandatory-control, cold/churn, statistical, and promotion gate.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/P140/shared-finalization/attempt-0003/evidence.json": "fc46c0df6192000b7c9013a50a585774a00ab67ed814567b779d0e45a4d9af1d"
  },
  "evidence": [
    {
      "bytes": 1698,
      "path": "/workspace/wtime/target/performance/P140/shared-finalization/attempt-0003/evidence.json",
      "sha256": "fc46c0df6192000b7c9013a50a585774a00ab67ed814567b779d0e45a4d9af1d"
    }
  ],
  "outcome": "contract_only",
  "status": "done",
  "task": "P140"
}
```

### Final close record

The final source-bound close is `target/performance/P140/shared-finalization/attempt-0003/completion.json` (evidence SHA-256 `fc46c0df6192000b7c9013a50a585774a00ab67ed814567b779d0e45a4d9af1d`), published by snapshot `P140-publication-0002`. Verification attempt-0003 passed all 15 Rust harness tests and 71 Python tests. The production Register build advertised the threshold capability, and the source-bound smoke exercised threshold 3. Strict JSON metadata checks now reject booleans and floats; the corrected validators and regression cases passed in this final run. P138 build budgets were revalidated and reused. No Rust core edits or new dependencies were introduced. The smoke remains contract-only, with no timing evidence or candidate promotion.

The retained history includes freeze attempts 0001–0003, verify attempts 0001–0003, both finalizer records, and both publication snapshots. Verify attempt-0001 failed after Cargo reported 15 passing tests because its wrapper failed to record the executed Cargo binary path. Attempt-0002 completed successfully but was superseded when review found the strict JSON boolean/float type gap. Final close and publication attempt-0003 / snapshot 0002 include the correction and passed; earlier records remain unchanged.
