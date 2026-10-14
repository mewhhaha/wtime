# P127 — Bind metered and unmetered settings to staged benchmark runs

Status: **todo**

Outcome: **pending**

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
- `tools/wtime-bench/src/main.rs`
- `tools/wtime-bench/src/protocol.rs`
- `tools/wtime-bench/src/scalar_driver.rs`
- `tools/wtime-bench/p127_fuel_policy.py`
- `tools/wtime-bench/tests/test_p127_fuel_setting.py`
- `tools/verification/producers/p127_fuel.py`
- `tools/verification/tests/test_p127_fuel.py`
- `docs/benchmarks/performance/P127-selection.json`
- `docs/benchmarks/performance/P127-verification.json`

Runtime implementation files remain outside this task. The benchmark protocol must encode `fuel=None` as `unmetered` and an explicit bounded integer as `metered`, binding the setting into source identity, observation keys, metadata, command argv, and raw reports. P127 executes one current-source command per row/stage/mode/fuel tuple; it does not pair baseline and candidate measurements. The focused-comparison interface accepts one selected policy per invocation so P053 can later pass that identical setting to both paired sides. Never infer fuel from build mode or change it between stages.

## Input/output contract

Keep workload inputs fixed and run each declared row under both `unmetered` (`fuel=None`) and `metered` (`fuel=50_000_000`) settings. The frozen rows are `scalar_accumulator_n0`, `_n17`, `_n32`, `_n10000`; `scalar_dependent_n0`, `_n17`, `_n32`, `_n10000`; `call_overhead`; and `prepare_large`. Run every registry-declared stage for each row, both `decoded` and `compact` modes, and both fuel settings. These stage sets produce **184 distinct real row/stage/mode/fuel observations and driver commands**: nine rows with five stages plus `prepare_large` with `prepare`, across four mode/setting combinations. Warm calls use a fixed batch of three and bind it into the record; P127 does no warm-batch calibration or paired comparison. P053 later owns its declared 20 ms paired calibration, passing the same selected fuel policy to both sides. No aggregate statistics, speed gate, or promotion decision is part of P127.

The independent fuel proof separately establishes that sufficient `Some(50_000_000)` and `None` return the same exact result/state for every scalar proof row, that low budgets preserve P020's exact first-unexecuted boundary and remaining-fuel semantics, and that the reference accounting path agrees. Cancellation, prompt retirement, operand-height behavior, fuel-error priority, and continuation resume transitions retain existing P020 semantics.

## Implementation recipe

1. Before editing the harness, preserve and validate the current workload registry/selection identities and existing P020 source-bound fuel evidence. Add a P127 selection containing only the ten fixed rows above, their unchanged stages and inputs, and the explicit two-value fuel-policy axis. Do not add rows to `workloads.json`.
2. Extend the scalar harness protocol with a typed fuel setting. `None` means unmetered; `Some(50_000_000)` means metered. Reject booleans, negative values, unsupported values, missing setting fields, and settings that drift between stages or are absent from an observation. Preserve the setting in command argv, source-bound identity, fixed-batch identity, report metadata, and every raw observation.
3. Keep execution mode, feature profile, fuel policy, and stage as separate typed fields. For `prepare`, record the selected setting even though no guest call occurs. For warm calls, bind the single selected policy and fixed batch of three to the observation; do not calibrate in P127. Keep production binaries uninstrumented and use the existing `compact-executor` feature only to expose the compact mode; `execution-reference` is confined to semantic tests/proof.
4. Add a separate proof producer that checks every expected row/stage/mode/policy tuple, invokes the real scalar driver, archives command/stdout/stderr before oracle validation, and rejects missing, duplicate, mixed-setting, instrumented, or source-drifted records. Its full close must collect all 184 observations and the independent fuel proof without running statistics.

## Meaningful cases

- `test_metering_policy_is_required_typed_and_source_bound`: reports and argv distinguish `None` from an exact integer budget and reject malformed policy values.
- `test_command_policy_is_explicit_and_stage_settings_cannot_drift`: every command carries one explicit mode/fuel tuple; missing or drifting settings fail preflight, and future P053 paired commands must pass the same selected policy to both sides.
- `test_metered_and_unmetered_rows_keep_the_same_result_oracle`: every scalar row completes under both policies with an independently computed expected result.
- `test_fuel_boundary_proof_preserves_prefix_and_remaining_budget`: low-fuel probes retain P020's exact stop point, effects, and remaining budget.
- `test_resume_fuel_reactivates_metering_without_stale_state`: saved-fuel min/None/Some merge behavior is checked against the reference accounting route.
- `test_full_184_row_stage_mode_policy_matrix_is_required`: missing or duplicate observations, an absent cold stage, or wrong policy/mode fails close.
- `test_instrumented_or_source_drifted_timing_input_fails_closed`: coverage/counter binaries and changed harness/runtime inputs cannot be admitted.

## Verification

Use the P127 contract's development groups `policy-model`, `driver-protocol`, `independent-fuel-proof`, and `staged-runtime`. The full contract builds and binds two source-current, uninstrumented runtime-only production records: Decoded with `runtime_features=[]`, and Compact with `runtime_features=["compact-executor"]`. It drives all 184 single-command row/stage/mode/fuel combinations; each observation is one mode-policy tuple, not a paired baseline/candidate sample, and `warm_call` uses fixed batch three. The independent semantic proof uses the test-only `execution-reference` feature and existing P020 prefix/cancellation/continuation cases; do not change runtime files or weaken those checks.

```sh
python3 -B tools/verify.py plan --contract docs/benchmarks/performance/P127-verification.json
python3 -B tools/verify.py dev --contract docs/benchmarks/performance/P127-verification.json --group policy-model --execute
python3 -B tools/verify.py dev --contract docs/benchmarks/performance/P127-verification.json --group driver-protocol --execute
python3 -B tools/verify.py dev --contract docs/benchmarks/performance/P127-verification.json --group independent-fuel-proof --execute
python3 -B tools/verify.py dev --contract docs/benchmarks/performance/P127-verification.json --group staged-runtime --execute
python3 -B tools/verify.py close --contract docs/benchmarks/performance/P127-verification.json --output target/performance/P127/complete --execute
```

The focused tests must execute all named cases above. Validation must establish complete registry/stage coverage, exact policy propagation, independent result and fuel-boundary proofs, source-current uninstrumented production identity, and command/evidence containment. Development receipts are iteration evidence only.

## Measurement and decision

Selection: scalar staged rows listed above. Execution modes: `decoded`, `compact`; fuel policies: `unmetered`, `metered`. Decision rule: `integrity`. This is a harness/protocol foundation only. It makes no candidate comparison, runtime change, speed claim, default-adoption decision, or Wasmtime claim. P053 remains responsible for its runtime accounting implementation and any later measured decision.

## Completion evidence

Record the unchanged registry and selection identities, exact fuel policy values, all 184 real observations by row/stage/mode/policy, fixed warm batch three, two source-current mode-specific production records (Decoded has no runtime features; Compact uses `compact-executor`), independent result and fuel-boundary proof, named test counts, and limitations. No task completion or timing result is claimed by this draft.
