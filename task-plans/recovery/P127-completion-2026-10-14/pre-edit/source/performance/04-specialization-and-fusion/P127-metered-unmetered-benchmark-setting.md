# P127 — Bind metered and unmetered settings to staged benchmark runs

Status: **done**

Outcome: **contract_only**

Kind: `foundation` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P002 — Compare separately built baseline and candidate binaries](../00-measurement/P002-paired-driver.md)

[P003 — Define workload selections and engine capability preflight](../00-measurement/P003-workload-schema.md)

[P008 — Add scaling, branch and call benchmark families](../00-measurement/P008-scalar-workloads.md)

[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)

[P016 — Add explicit executor policies and test integration hooks](../01-execution-contracts/P016-execution-policy.md)

[P020 — Centralize exact source instruction accounting](../01-execution-contracts/P020-exact-execution-accounting.md)

[P126 — Add scalable memory-width and read-scan workload aliases](P126-scalable-memory-workload-aliases.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), contracts **C2, C3, C4, C6, C7, C8**, and [the staged performance workflow](../../../wtime/docs/verification/fast-performance.md). This foundation makes the benchmark harness able to declare and prove metered versus unmetered settings. It does not change runtime/core sources or decide whether metering is faster.

- `tools/wtime-bench/compare.py`
- `tools/wtime-bench/focused_compare.py`
- `tools/wtime-bench/src/protocol.rs`
- `tools/wtime-bench/src/scalar_driver.rs`
- `tools/wtime-bench/p127_fuel_policy.py`
- `tools/wtime-bench/tests/test_p127_fuel_setting.py`
- `tools/verification/producers/p127_fuel.py`
- `tools/verification/tests/test_p127_fuel.py`
- `docs/benchmarks/performance/P127-selection.json`
- `docs/benchmarks/performance/P127-verification.json`
- `tools/wtime-bench/src/fuel_policy.rs`
- `tools/wtime-bench/execution_identity.py`
- `tools/wtime-bench/tests/test_p127_comparison.py`
- `tools/wtime-bench/tests/test_fast_execution_identity.py`
- `tools/verification/producers/p127_driver_tests.py`
- `tools/verification/producers/p127_semantics.py`
- `tools/verification/producers/p127_fuel_cases.rs`
- `tools/verification/producers/p127_prerequisite.py`

The actual runtime tree remains outside this task. `tools/wtime-bench/src/main.rs` is unchanged and not needed; the task-owned copied Rust accounting source is a test-only diagnostic, not a runtime edit. The benchmark protocol must encode `fuel=None` as `unmetered` and an explicit bounded integer as `metered`, binding the setting into source identity, observation keys, metadata, command argv, and raw reports. P127 executes one current-source command per row/stage/mode/fuel tuple; it does not pair baseline and candidate measurements. The focused-comparison interface accepts one selected policy per invocation so P053 can later pass that identical setting to both paired sides. Never infer fuel from build mode or change it between stages.

## Input/output contract

Keep workload inputs fixed and run each declared row under both `unmetered` (`fuel=None`) and `metered` (`fuel=50_000_000`) settings. The frozen rows are `scalar_accumulator_n0`, `_n17`, `_n32`, `_n10000`; `scalar_dependent_n0`, `_n17`, `_n32`, `_n10000`; `call_overhead`; and `prepare_large`. Run every registry-declared stage for each row, both `decoded` and `compact` modes, and both fuel settings. These stage sets produce **184 distinct real row/stage/mode/fuel observations and driver commands**: nine rows with five stages plus `prepare_large` with `prepare`, across four mode/setting combinations. Warm calls use a fixed batch of three and bind it into the record; P127 does no warm-batch calibration or paired comparison. P053 later owns its declared 20 ms paired calibration, passing the same selected fuel policy to both sides. No aggregate statistics, speed gate, or promotion decision is part of P127.

The independent fuel proof separately establishes that sufficient `Some(50_000_000)` and `None` return the same exact result/state for every scalar proof row, that low budgets preserve P020's exact first-unexecuted boundary and remaining-fuel semantics, and that the reference accounting path agrees. Cancellation, prompt retirement, operand-height behavior, fuel-error priority, and continuation resume transitions retain existing P020 semantics. It covers nine scalar rows × three modes (`decoded`, `compact`, `execution-reference`) × two policies × three repeats (162 guest calls), plus nine execution-reference source-event probes. Separate diagnostics cover 24 low-fuel prefixes, 12 captured cursor windows, 27 None/Some continuation tuples (54 guest calls), and five merge/three priority unit tuples using the accounting helper. The source-test-only Rust copy is diagnostic; the actual runtime tree is not edited.

## Implementation recipe

1. Before editing the harness, preserve and validate the current workload registry/selection identities and existing P020 source-bound fuel evidence. Add a P127 selection containing only the ten fixed rows above, their unchanged stages and inputs, and the explicit two-value fuel-policy axis. Do not add rows to `workloads.json`.
2. Extend the scalar harness protocol with a typed fuel setting. `None` means unmetered; `Some(50_000_000)` means metered. Reject booleans, negative values, unsupported values, missing setting fields, and settings that drift between stages or are absent from an observation. Preserve the setting in command argv, source-bound identity, fixed-batch identity, report metadata, and every raw observation. The CLI form is `--source-fuel none|50000000`; omitted flags preserve the old wire contract.
3. Keep execution mode, feature profile, fuel policy, and stage as separate typed fields. For `prepare`, record the selected setting even though no guest call occurs. For warm calls, bind the single selected policy and fixed batch of three to the observation; do not calibrate in P127. Keep production binaries uninstrumented: runtime-only for Decoded and `compact-executor` for Compact. Unit driver checks use runtime-only, `compact-executor`, and `execution-reference` feature configurations. The separate private fuel proof uses instrumented `compact-executor` plus `execution-reference` only for semantic diagnostics, never as a production measurement binary.
4. Add a separate proof producer that checks every expected row/stage/mode/policy tuple, invokes the real scalar driver, archives command/stdout/stderr before oracle validation, and rejects missing, duplicate, mixed-setting, instrumented, or source-drifted records. Its full close collects all 184 observations and the independent fuel proof without running statistics. The independent proof covers nine scalar rows × three modes × two policies × three repeats (162 guest calls) plus nine reference source-event probes; separate diagnostics retain 24 low-fuel prefixes, 12 cursor windows, 27 continuation tuples/54 guest calls, and five merge/three priority unit tuples.

## Meaningful cases

### Policy model tests

- `test_cli_value_requires_exact_well_typed_policy_dict`: CLI policy values accept only the exact supported typed setting.
- `test_dependent_oracle_matches_literal_recurrence_in_admitted_wat`: Dependent workload oracle agrees with the admitted literal recurrence.
- `test_exact_typed_policy_and_cli_contract`: Typed policy and explicit command-line policy preserve their contract.
- `test_fixed_matrix_is_46_rows_times_four_policies_modes`: The fixed workload/stage matrix has the declared row and policy/mode cardinality.
- `test_oracles_are_independent_and_prepare_has_null`: Independent result oracles hold, and prepare observations have no guest result.
- `test_protocol_contract_encodes_null_vs_exact_budget`: Wire protocol distinguishes null fuel from the exact metered budget.
- `test_registry_drift_rejects_without_mutating_source`: Registry drift is rejected without modifying shared workload input.
- `test_validate_wire_setting_requires_presence_and_exact_json_types`: Wire validation requires a present setting with exact JSON types.

### Producer and independent-fuel tests

- `test_actual_scalar_driver_wire_requires_typed_strings_and_policy_keys`: Real scalar-driver wire requires typed strings and the declared policy fields.
- `test_full_184_policy_mode_stage_matrix_accepts_only_exact_tuple_set`: The producer accepts only the complete 184 row/stage/mode/policy tuple set.
- `test_mock_prepare_large_fixture_is_generated_encoded_and_mvp_validated_inside_attempt`: The mock prepare_large fixture is generated, encoded, and MVP validated inside its attempt root.
- `test_protocol_mutation_is_rejected_and_protocol_logs_are_retained`: Protocol mutation fails closed and protocol logs remain available.
- `test_raw_command_logs_exist_before_response_mismatch_and_escape_is_blocked`: Raw command logs precede result checks and escaping outputs are rejected.
- `test_single_case_route_and_wire_cover_call_overhead_and_prepare_large`: Single-case route and wire cover call_overhead and prepare_large.
- `test_source_maps_use_p128_closure_and_record_protocol_mutations_abort_pre_guest`: Source mapping uses P128 closure; protocol drift aborts before guest execution.
- `test_truthful_p128_record_shape_and_core_record_protocol_staleness_fail_closed`: P128 record shape is truthful and stale core protocol records fail closed.

### Paired policy interface tests

- `test_compare_routes_reject_missing_wrong_float_and_boolean_wire_budget`: Both comparison routes reject absent, malformed, floating, and Boolean fuel settings.
- `test_explicit_metered_and_unmetered_policies_route_through_both_compare_helpers`: Explicit metered and unmetered policies reach both comparison helpers.
- `test_focused_observe_shares_same_setting_for_simulated_paired_sides`: Focused paired observations carry one identical selected policy on both sides.
- `test_focused_prepare_large_is_numeric_single_case_and_returns_null`: Focused prepare_large remains a numeric single case with a null guest result.
- `test_legacy_single_case_and_scalar_argv_remain_byte_for_byte_unchanged`: Legacy call_overhead and scalar command argv stay byte-for-byte compatible when the flag is omitted.
- `test_unsupported_focused_family_fails_before_process_command`: Unsupported focused workload families fail before any process launch.

### Execution identity tests

- `test_adapter_archive_corruption_is_still_rejected`: Corrupted source archives remain rejected.
- `test_analysis_changes_are_visible_but_do_not_stale_legacy_pairs`: Analysis-only changes are visible without invalidating legacy execution pairs.
- `test_focused_identity_uses_task_declaration_without_registry_selection_lookup`: Focused identity follows task declaration without registry selection lookup.
- `test_focused_producer_changes_stale_focused_pairs`: Focused producer changes invalidate focused execution pairs.
- `test_membership_changes_and_unknown_python_modules_fail_closed`: Unclassified modules and identity membership changes fail closed.
- `test_runtime_timer_fixture_and_oracle_edits_stale_legacy_pairs`: Runtime, timer, fixture, and oracle changes invalidate old execution pairs.
- `test_verify_inputs_uses_live_sources_without_running_a_comparison`: Identity verification uses live source inputs without starting a comparison.

### Rust driver tests

- `p127_omitted_and_explicit_unmetered_preserve_distinct_wire_identity`: Omitted and explicit unmetered CLI settings preserve distinct legacy/wire identities.
- `p127_fixed_metered_policy_sets_fuel_without_changing_other_limits`: The fixed metered policy sets fuel and preserves other resource limits.
- `p127_malformed_or_unsupported_fuel_arguments_are_rejected`: Malformed and unsupported fuel arguments are rejected.
- `p127_single_case_builder_uses_selected_fuel_at_guest_source_boundary`: Single-case builder forwards the selected setting at the guest source boundary.
- `p127_scalar_builder_uses_selected_fuel_at_guest_source_boundary`: Scalar builder forwards the selected setting at the guest source boundary.

### Source-test-only private fuel cases

- `p127_actual_accounting_merges_saved_fuel_once_and_preserves_error_priority`: Copied accounting helper merges saved fuel once and preserves the actual error priority.
- `p127_scalar_policies_match_reference_and_refill_each_invocation`: Private source-test driver compares scalar results with reference and refills each invocation.
- `p127_low_fuel_prefixes_bind_memory_cursor_and_remaining_budget`: Private source-test diagnostics bind low-fuel prefixes to memory, cursor, and remaining budget.
- `p127_real_continuations_preserve_none_some_fuel_transitions`: Private source-test diagnostics cover None/Some fuel across real continuation transitions.
## Verification

Use development groups `policy-model`, `producer-tests`, `paired-policy-interface`, `execution-identity`, and `driver-protocol`. Full staged-runtime and independent fuel/source proof run in close. Production records are source-current, uninstrumented runtime-only Decoded and `compact-executor` Compact; `execution-reference` is confined to the semantic/source proof. The production matrix contains 184 row/stage/mode/policy observations. The independent proof contains 162 guest calls over nine scalar rows, three modes, two policies, and three repeats, plus nine reference source-event probes. It also checks 24 low-fuel prefixes, 12 captured cursor windows, 27 None/Some continuation tuples/54 guest calls, and five merge plus three priority unit tuples. These results do not add timing statistics.

```sh
python3 -B tools/verify.py plan --contract docs/benchmarks/performance/P127-verification.json
python3 -B tools/verify.py dev --contract docs/benchmarks/performance/P127-verification.json --group policy-model --execute
python3 -B tools/verify.py dev --contract docs/benchmarks/performance/P127-verification.json --group producer-tests --execute
python3 -B tools/verify.py dev --contract docs/benchmarks/performance/P127-verification.json --group paired-policy-interface --execute
python3 -B tools/verify.py dev --contract docs/benchmarks/performance/P127-verification.json --group execution-identity --execute
python3 -B tools/verify.py dev --contract docs/benchmarks/performance/P127-verification.json --group driver-protocol --execute
python3 -B tools/verify.py close --contract docs/benchmarks/performance/P127-verification.json --output target/performance/P127/complete --execute
```

The full 38 named checks comprise 29 Python model/producer/comparison/identity cases, five Rust driver tests, and four source-test-only private fuel cases. Runtime feature checks cover runtime-only, `compact-executor`, and `execution-reference`; the separate private fuel proof uses instrumented `compact-executor` plus `execution-reference` for semantic diagnostics only. P020, P018, and T145 source-current prerequisites are inherited through the full 354-case P050 receipt. Full close runs staged and independent proofs, keeps focused paired policy identical on both future P053 sides, and performs no statistics or speed decision.
## Measurement and decision

Selection: scalar staged rows listed above. Execution modes: `decoded`, `compact`; fuel policies: `unmetered`, `metered`. Decision rule: `integrity`. This is a harness/protocol foundation only. It makes no candidate comparison, runtime change, speed claim, default-adoption decision, or Wasmtime claim. P053 remains responsible for its runtime accounting implementation and any later measured decision.

## Completion evidence

Current production and ad-hoc proof runs pass, but the full contract close and publication remain pending; these checks do not complete P127.

Record unchanged registry/selection identities, the exact `--source-fuel none|50000000` policy and omitted-flag compatibility, all 184 observations, warm batch three, source-current Decoded/Compact production identities, 38 named checks, independent proof dimensions (162 guest calls plus nine reference source-event probes), 24 prefixes, 12 cursor windows, 27 continuation tuples/54 guest calls, five merge and three priority unit tuples, the inherited P020/P018/T145 cases from P050, and limitations. Keep focused comparison policy the same on both future P053 sides. P127 remains todo/pending until publisher; no statistics or speed claim.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/P127/shared-finalization/attempt-0001/evidence.json": "cd1c924fb67c6236108caa38241d9dc538c4b22820d2cc7aa14c74c4372c0ab5"
  },
  "evidence": [
    {
      "bytes": 1468,
      "path": "/workspace/wtime/target/performance/P127/shared-finalization/attempt-0001/evidence.json",
      "sha256": "cd1c924fb67c6236108caa38241d9dc538c4b22820d2cc7aa14c74c4372c0ab5"
    }
  ],
  "outcome": "contract_only",
  "status": "done",
  "task": "P127"
}
```
