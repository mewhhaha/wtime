# P128 — Add minimal scalar identity and literal-folding workloads

Status: **done**

Outcome: **contract_only**

Kind: `foundation` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P003 — Define workload selections and engine capability preflight](../00-measurement/P003-workload-schema.md)

[P008 — Add scaling, branch and call benchmark families](../00-measurement/P008-scalar-workloads.md)

[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)

[P016 — Add explicit executor policies and test integration hooks](../01-execution-contracts/P016-execution-policy.md)

[P019 — Define bounded optimizer effect and source expansion metadata](../01-execution-contracts/P019-effect-certificates.md)

[P020 — Centralize exact source instruction accounting](../01-execution-contracts/P020-exact-execution-accounting.md)

[P037 — Map compact instructions to source events and control targets](../03-compact-executor/P037-compact-source-maps.md)

[P047 — Fuse a bounded set of local and constant arithmetic patterns](P047-local-arithmetic-fusions.md)

[P126 — Add scalable memory-width and read-scan workload aliases](P126-scalable-memory-workload-aliases.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), contracts **C1, C2, C3, C6, C7, C8**, and [the staged performance workflow](../../../wtime/docs/verification/fast-performance.md). This is a fixture/oracle/scaling foundation for P054. It does not change the runtime optimizer, append registry rows, or claim a speed result.

- `tools/wtime-bench/fixtures/scalar/p128_identity_copy.wat`
- `tools/wtime-bench/fixtures/scalar/p128_literal_wrap_add.wat`
- `tools/wtime-bench/fixtures/scalar/p128_literal_bitwise.wat`
- `tools/wtime-bench/scalar_p128_workloads.py`
- `tools/wtime-bench/p128_scalar_scaling.py`
- `tools/wtime-bench/tests/test_p128_scalar_workloads.py`
- `tools/verification/producers/p128_scalar.py`
- `tools/verification/tests/test_p128_scalar.py`
- `docs/benchmarks/performance/P128-selection.json`
- `docs/benchmarks/performance/P128-verification.json`
- `tools/wtime-bench/execution_identity.py`
- `tools/wtime-bench/tests/test_fast_execution_identity.py`
- `tools/verification/producers/p128_evidence.py`
- `tools/verification/tests/data/admitted_wasm/p128_identity_copy.wasm`
- `tools/verification/tests/data/admitted_wasm/p128_literal_wrap_add.wasm`
- `tools/verification/tests/data/admitted_wasm/p128_literal_bitwise.wasm`

Reuse the existing scalar stage driver and runtime protocol through a task-owned adapter; do not edit Rust runtime or benchmark-driver sources unless the admitted driver cannot carry the fixture path and exact stage without adding registry membership. Any necessary driver change must be limited to that interface and leave core/runtime code unchanged.

## Input/output contract

Define exactly three MVP i32 loop fixtures: one identity local-copy sequence, one literal-only wrapping `i32.add`, and one literal-only integer bitwise identity (`i32.and`). The loop count aliases are fixed at `n=0,1,17,32,1024,10000`, producing 18 model rows, never arbitrary ranges. Exact dynamic source events, including the implicit function end, follow `9 + 14*n`; the source proof requires 18 rows and 126 source-event/fuel/reference commands (one event command plus six fuel probes per row). Each fixture's work count is exactly `n` opportunities of its named shape. The identity fixture includes a separate observable increment so the loop remains meaningful while the copy itself is neutral. The literal-add and bitwise fixtures feed each folded literal result into an accumulator, so the result depends on the repeated operation while each candidate expression remains literal-only.

The oracle independently performs explicit modulo-2^32 arithmetic and signed-i32 normalization. Expected results are `n` for identity-copy, signed-i32 of `n * 0x80000000` for wrapping-add (`0x7fffffff + 1`), and signed-i32 of `n * 0x13579bdf` for bitwise AND (`0x13579bdf & 0xffffffff`). No fixture contains multiply, divide, floating point, memory, calls, reference operations, traps, or control-flow exits from the single loop body. Keep all **459 existing registry rows, their IDs/hashes, and every selection membership unchanged**; the 18 rows exist only in the P128 fixture model/selection contract until P054 proposes a separately reviewed append. Normalize every workload at all five stages (`prepare`, `instantiate`, `first_call`, `warm_call`, `load_and_call`) in `decoded` and `compact` modes using source-current uninstrumented runtime-only binaries. This is exactly **180 real observations/driver commands** (18 shape/input rows × 5 stages × 2 modes). Warm calls bind bounded batch size three; no statistics or timing comparison is required.

## Implementation recipe

1. Before adding fixtures, snapshot and validate the full 459-row registry, every selection's membership, the existing scalar model, stage-driver interface, and P047 named tests. Do not rewrite or reformat registry/workload data.
2. Add the three minimal WAT inputs. Encode each through the repository's admitted encoder into an isolated evidence directory; bind both source and Wasm hashes in the P128 selection. Review decoded instruction order to confirm the copy pair and literal-only add/bitwise operation are present exactly as described. Reject a fixture if its actual opcode order or loop work count differs from the contract.
3. Add `scalar_p128_workloads.py` with only the three fixed shapes and six exact n values, independent result oracles, fixed work-unit metadata, five stages, and MVP profile. Make all other suffixes/operations invalid. `p128_scalar_scaling.py` validates generated rows and converts them into exact task-owned staged-driver invocations without editing `workloads.json` or the shared registry.
4. Add selection and verification contracts that bind the 18 fixture model rows, frozen WAT/Wasm hashes, exact stage list, oracle implementation, executable/source identities, mode, stage, and warm batch. The proof producer archives raw output and command records before oracle checks, independently checks result and modulo arithmetic, validates the 18-row source-event/fuel proof and its 126 source-event/fuel/reference commands, and requires every one of the 180 staged observations.
5. Keep P054's candidate freeze separate: its eventual selection may choose n32 and n10000 as small/large targets, n17 as held out, and n0/n1/n1024 as protected rows, while retaining `call_overhead` and `prepare_large` cold controls and P047 checks. P128 does not freeze or preapprove that candidate selection.

## Meaningful cases

### Model and fixture tests

- `test_admitted_wat_and_wasm_bytes_match_frozen_hashes`: Each admitted WAT and Wasm input matches its frozen hash.
- `test_fixed_alias_grid_is_exact_and_registry_stays_459_rows`: Only the 18 fixed model rows are admitted and the 459-row shared registry and selections remain unchanged.
- `test_fixture_work_count_is_exactly_n_and_invalid_aliases_fail`: Each fixed alias has exactly n work units and malformed aliases fail.
- `test_frozen_registry_rows_and_selections_survive_model_generation`: Model generation preserves the frozen registry rows and selection membership.
- `test_identity_copy_fixture_has_an_independent_n_oracle`: The neutral copy shape retains its separate observable increment and returns n.
- `test_literal_add_wraps_as_i32_without_float_or_trapping_ops`: Literal wrapping addition matches independent signed-i32 arithmetic.
- `test_literal_bitwise_fixture_matches_independent_mask_oracle`: Literal AND results match the independent masked oracle.
- `test_rows_bind_profile_modes_stages_oracle_and_mvp_work_units`: Every row binds its MVP profile, both modes, all five stages, and independent oracle.
- `test_wat_opcode_shape_rejects_multiply_divide_float_memory_or_calls`: The fixture opcode shape excludes unrelated and disallowed operations.

### Proof mock tests

- `test_complete_matrix_and_all_outputs_are_contained`: The mock proof covers all 180 stage/mode observations and keeps outputs inside its fresh root.
- `test_nonzero_command_output_and_status_are_retained`: Failed command output and status remain archived.
- `test_oracle_mismatch_preserves_raw_output_before_failure`: Raw output is retained before an oracle mismatch fails the proof.
- `test_output_escape_is_rejected_before_any_write`: An output path outside the fresh root is rejected before writing.
- `test_stale_record_rejected_before_guest_commands`: Stale records or source drift fail before guest commands are run.

### Execution identity tests

- `test_adapter_archive_corruption_is_still_rejected`: Tampered adapter archives are rejected.
- `test_analysis_changes_are_visible_but_do_not_stale_legacy_pairs`: Analysis-only changes remain visible without invalidating legacy execution pairs.
- `test_focused_identity_uses_task_declaration_without_registry_selection_lookup`: Focused identity follows task declaration without registry selection lookup.
- `test_focused_producer_changes_stale_focused_pairs`: Focused producer changes invalidate focused execution pairs.
- `test_membership_changes_and_unknown_python_modules_fail_closed`: Unclassified module additions and missing identity inputs fail closed.
- `test_runtime_timer_fixture_and_oracle_edits_stale_legacy_pairs`: Runtime, timer, fixture, and oracle edits invalidate old execution identities.
- `test_verify_inputs_uses_live_sources_without_running_a_comparison`: Identity verification reads live inputs without launching a comparison.



## Verification

Use development groups `fixture-model`, `independent-oracle`, and `staged-runtime`. The full contract builds two source-current, uninstrumented runtime-only production records: Decoded with `runtime_features=[]`, and Compact with `runtime_features=["compact-executor"]`. It uses the MVP feature profile, encodes only the three task-owned WAT fixtures in contained output, and requires all 180 real row/stage/mode observations. `execution-reference` is reserved for semantic/reference tests and is not a production measurement mode.

```sh
python3 -B tools/verify.py plan --contract docs/benchmarks/performance/P128-verification.json
python3 -B tools/verify.py dev --contract docs/benchmarks/performance/P128-verification.json --group fixture-model --execute
python3 -B tools/verify.py dev --contract docs/benchmarks/performance/P128-verification.json --group independent-oracle --execute
python3 -B tools/verify.py dev --contract docs/benchmarks/performance/P128-verification.json --group staged-runtime --execute
python3 -B tools/verify.py close --contract docs/benchmarks/performance/P128-verification.json --output target/performance/P128/complete --execute
```

All 21 named checks (nine model, five proof-mock, and seven execution-identity tests) must execute with nonzero assertions. Retain the P047 seven-case suite unchanged and require its results as a prerequisite receipt/reference; P128 adds no runtime transformation and cannot replace P054 semantic/fuel/materialization cases. Close validates fixture identities, the 18-row `9 + 14*n` source-event proof with 126 source-event/fuel/reference commands, full 180-observation staged coverage, independent oracles, source-current runtime identity, warm batch, and containment.

## Measurement and decision

Selection: the three P128 scalar shapes at fixed n values. Execution modes: `decoded`, `compact`. Decision rule: `integrity`. This validates fixture semantics and the staged protocol only. It does not run statistics, promote a transformation, claim runtime speed, change defaults, or make a Wasmtime comparison.

## Completion evidence

Record the unchanged registry/selection hashes, three WAT and generated Wasm hashes, all 18 fixed aliases and work counts, the `9 + 14*n` source-event formula including implicit function end and all 126 source-event/fuel/reference commands, independent oracle results, 180 observations by stage/mode, warm-batch identity, runtime-only binary/source identity, P047 prerequisite results, named test counts, and limitations. No task completion is claimed by this draft.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/P128/shared-finalization/attempt-0002/evidence.json": "abe68e19068eb70dfbc771c9e063e270ebf8743b514313500aecf69e4d5ddedf"
  },
  "evidence": [
    {
      "bytes": 1518,
      "path": "/workspace/wtime/target/performance/P128/shared-finalization/attempt-0002/evidence.json",
      "sha256": "abe68e19068eb70dfbc771c9e063e270ebf8743b514313500aecf69e4d5ddedf"
    }
  ],
  "outcome": "contract_only",
  "status": "done",
  "task": "P128"
}
```
