# P126 — Add scalable memory-width and read-scan workload aliases

Status: **done**

Outcome: **contract_only**

Kind: `foundation` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P009 — Add memory width, bulk and growth benchmark families](../00-measurement/P009-memory-workloads.md)

[P016 — Add explicit executor policies and test integration hooks](../01-execution-contracts/P016-execution-policy.md)

[P025 — Remove per-load heap allocation for all scalar widths](../02-structural-wins/P025-fixed-scalar-reads.md)

[P043 — Resolve memory bindings and specialize checked scalar accesses](../03-compact-executor/P043-compact-memory-plans.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), and contracts **C1, C2, C3, C5, C7** in [CONTRACTS.md](../CONTRACTS.md). This is a workload and verification foundation for P049. It does not modify the runtime or admit a performance result.

- `tools/wtime-bench/workloads.json`
- `tools/wtime-bench/memory_workloads.py`
- `tools/wtime-bench/memory_scaling.py`
- `tools/wtime-bench/src/memory_driver.rs`
- `tools/wtime-bench/tests/test_p126_memory_scaling.py`
- `tools/verification/producers/p126_memory.py`
- `tools/verification/tests/test_p126_memory.py`
- `docs/benchmarks/performance/P126-verification.json`

Use the existing three admitted fixtures unchanged: `fixtures/memory/widths.wat`, `fixtures/memory/widths_shuffled.wat`, and `fixtures/memory/read_scan.wat`. Preserve their bytes and hashes. Preserve all 441 original registry rows and all original selection membership; append the 18 aliases only to the existing ready `memory` selection and leave every other selection unchanged.

## Input/output contract

Append exactly 18 aliases for the existing predictable-width, shuffled-width, and read-scan families with suffixes 0, 1, 17, 32, 256, and 1024, while preserving all 441 original rows and all original selection membership; append aliases only to the existing ready memory selection and leave every other selection unchanged. Preserve the three admitted fixture bytes. Suffixes are exact iteration counts: predictable-width and shuffled-width rows perform 19*n memory operations, and read-scan rows perform 2*n. Normalize all five stages in decoded and compact modes using two source-current runtime-only binaries. The foundation proof produces 180 real observations/driver commands; warm_call runs batch three, so the proof executes 252 logical guest invocations in total. Validate results, checksum, and complete memory state with independent oracles, plus batch repeats and resets. This is contract-only evidence, not a candidate comparison or speed-promotion gate.

## Implementation recipe

1. Record and validate the existing 441-row registry, all selection definitions, and hashes for the three admitted fixtures before editing. This is preparation and does not change a source or registry file.
2. Append the 18 fixed aliases to workloads.json and the memory workload model. Preserve all original rows and selection membership, append aliases only to the existing ready memory selection, leave every other selection unchanged, and do not change fixture bytes or introduce new parameter ranges.
3. Extend the existing scaling/driver route to carry exact alias values and normalize all five stages in decoded and compact modes. Add the named tests and P126-verification.json development groups; its full contract must build the current-source runtime-only producer and prove all 180 genuine row-stage-mode observations, independent result/checksum/full-memory oracles, and warm-batch reset behavior. Keep P049's existing semantic cases and acceptance requirements intact.

## Meaningful cases

- test_old_rows_and_selection_membership_remain_identical: the original 441 rows and all old selection membership retain their pinned identities, with aliases appended only to memory.
- test_alias_rows_match_generated_metadata_and_full_registry: all 18 aliases match the generated model, exact suffix parameters, and complete registry.
- test_alias_oracles_return_complete_independent_memory_images: all width and read-scan aliases match independent result and full-memory-image oracles, with 19*n and 2*n work units respectively.
- test_page_bounds_are_explicit: model inputs beyond the admitted page bounds fail instead of expanding the registry interface.
- test_exact_alias_rows_and_stages_are_required: the verifier rejects missing or duplicated aliases and requires all declared stages.
- test_full_memory_oracle_rejects_altered_model_image: altered expected memory hashes fail oracle verification.
- test_protocol_requires_requested_mode_and_uninstrumented_binary: a missing mode or instrumented binary fails preflight.
- test_staged_driver_argv_preserves_stage_and_batch_fields: stage identity and warm batch size are bound in the actual driver command.
- test_wrong_memory_response_is_archived_before_oracle_failure: raw driver output and command evidence are retained before a bad memory image is rejected.
- test_containment_refuses_escape_before_any_write: outputs cannot escape the dedicated evidence directory.
- test_source_drift_comparison_is_exact: any runtime/tool/fixture/source identity change fails closed.
- test_warm_call_uses_batch_three_and_full_state: warm_call binds batch size three and the full expected memory state.
- memory_driver::repeatable_memory_case_tests::accepts_width_aliases_and_preserves_existing_cases: valid aliases and pre-existing repeatable cases remain recognized.
- memory_driver::repeatable_memory_case_tests::rejects_unknown_or_malformed_width_aliases: unknown families, malformed suffixes, and overflowing values are rejected.

## Verification

Activate Rust as described in EXECUTION.md. The verification contract owns its full command list and declares the development groups `registry-model`, `driver-oracle`, and `runtime-routes`.

```sh
python3 -B tools/verify.py plan --contract docs/benchmarks/performance/P126-verification.json
python3 -B tools/verify.py dev --contract docs/benchmarks/performance/P126-verification.json --group registry-model --execute
python3 -B tools/verify.py dev --contract docs/benchmarks/performance/P126-verification.json --group driver-oracle --execute
python3 -B tools/verify.py dev --contract docs/benchmarks/performance/P126-verification.json --group runtime-routes --execute
python3 -B tools/verify.py close --contract docs/benchmarks/performance/P126-verification.json --output target/performance/P126/complete --execute
```

The full contract must bind and execute every alias in every declared stage and mode against one source-current runtime-only binary, then independently validate the 180 observations, memory image, checksum, warm-batch repeat, and reset. Development receipts are iteration evidence only. No paired candidate gate or promotion decision belongs to P126.

## Measurement and decision

Selection: the 18 appended rows in the existing memory families. Execution modes: `decoded` and `compact`. Decision rule: `integrity`. The result is contract-only foundation evidence. No performance comparison, runtime change, default adoption, or Wasmtime claim follows from P126.

## Executed result

P126 closed as **done / contract-only**. All 14 named checks passed, with zero failures or ignored checks. The proof preserved all 441 original registry rows and appended 18 aliases across the three unchanged fixtures, for 459 rows total; runtime source-entry count remained 111. It issued 180 real observations/driver commands (90 per mode, 36 per stage) using current decoded and compact runtime-only binaries. The three-call warm batch accounts for 252 logical guest invocations. Independent result, checksum, full-memory-state, repeat, and reset checks passed. This verifies the workload and proof contract; it makes no performance claim.

Two preflight attempts were retained and corrected before any driver or guest commands: the first declared an invalid custom build phase; the second had an incomplete source-delta scope (omitting the top-level Cargo/toolchain inputs and including CLI files). The canonical successful close is attempt `0038`; publication is `/workspace/wtime/target/verification/invocations/publish/attempt-0020`, with task snapshot `/workspace/wtime/target/performance/TASK-status/P126-publication-0001`.

The [published completion record](../../../wtime/target/performance/P126/shared-finalization/attempt-0002/completion.json) records evidence SHA-256 `498e0ba5cb2104f3d638e72d18fdbe72d527ce64bfeec93da84f760a972b9f15`. The full receipt is `/workspace/wtime/target/verification/runs/P126/verify/attempt-0001/receipt.json`; the independent memory proof is `/workspace/wtime/target/verification/runs/P126/verify/attempt-0001/memory-proof/report.json`.

## Completion evidence

The executed result is recorded below. P126 is published `done`/`contract_only`; P049 remains responsible for its existing semantic cases, candidate selection, and measured decision.


## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/P126/shared-finalization/attempt-0002/evidence.json": "498e0ba5cb2104f3d638e72d18fdbe72d527ce64bfeec93da84f760a972b9f15"
  },
  "evidence": [
    {
      "bytes": 1149,
      "path": "/workspace/wtime/target/performance/P126/shared-finalization/attempt-0002/evidence.json",
      "sha256": "498e0ba5cb2104f3d638e72d18fdbe72d527ce64bfeec93da84f760a972b9f15"
    }
  ],
  "outcome": "contract_only",
  "status": "done",
  "task": "P126"
}
```
