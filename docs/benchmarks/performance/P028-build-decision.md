# P028 build and promotion decision rules (proposal only)

This file predeclares the scalar comparison and build gates for P028. It is planning metadata, not measured evidence or a promotion decision. Keep `workloads.json`'s existing scalar selection unchanged at exactly 85 unique workloads.

## Workload roles

Use `P028-contract.json` with selection `scalar`. The primary family is the eight selected serial dependency-chain rows from `scalar_accumulator` and `scalar_dependent`, each at n=1, 32, 1024, and 10000. The recommended primary set is the smallest useful choice that measures common integer dependency chains with add/compare and add/multiply/compare mixes. Do not widen the primary family to unrelated control-flow workloads or claim that the benchmark measures every specialized operator.

The four n=0/n=17 rows for those two families are explicitly held-out protected controls:

- `scalar_accumulator_n0`
- `scalar_accumulator_n17`
- `scalar_dependent_n0`
- `scalar_dependent_n17`

The other 73 selected workloads are also protected controls, covering the remaining scalar families and three legacy controls. They remain present in the exact selected set and protect call/control/cold behavior; they are not primary numeric evidence:

- `call_overhead`
- `integer_loop`
- `branch_calls`
- `scalar_branch_entropy_n1`
- `scalar_branch_entropy_n32`
- `scalar_branch_entropy_n1024`
- `scalar_branch_entropy_n10000`
- `scalar_branch_entropy_n0`
- `scalar_branch_entropy_n17`
- `scalar_br_table_n1`
- `scalar_br_table_n32`
- `scalar_br_table_n1024`
- `scalar_br_table_n10000`
- `scalar_br_table_n0`
- `scalar_br_table_n17`
- `scalar_direct_calls_n1`
- `scalar_direct_calls_n32`
- `scalar_direct_calls_n1024`
- `scalar_direct_calls_n10000`
- `scalar_direct_calls_n0`
- `scalar_direct_calls_n17`
- `scalar_indirect_calls_n1`
- `scalar_indirect_calls_n32`
- `scalar_indirect_calls_n1024`
- `scalar_indirect_calls_n10000`
- `scalar_indirect_calls_n0`
- `scalar_indirect_calls_n17`
- `scalar_tail_calls_n1`
- `scalar_tail_calls_n32`
- `scalar_tail_calls_n1024`
- `scalar_tail_calls_n10000`
- `scalar_tail_calls_n0`
- `scalar_tail_calls_n17`
- `scalar_dynamic_api_n1`
- `scalar_dynamic_api_n32`
- `scalar_dynamic_api_n1024`
- `scalar_dynamic_api_n10000`
- `scalar_dynamic_api_n0`
- `scalar_dynamic_api_n17`
- `scalar_multivalue_branch_n1`
- `scalar_multivalue_branch_n32`
- `scalar_multivalue_branch_n1024`
- `scalar_multivalue_branch_n10000`
- `scalar_multivalue_branch_n0`
- `scalar_multivalue_branch_n17`
- `scalar_ordinary_recursion_n0`
- `scalar_ordinary_recursion_n1`
- `scalar_ordinary_recursion_n17`
- `scalar_ordinary_recursion_n32`
- `scalar_call_ref_n1`
- `scalar_call_ref_n32`
- `scalar_call_ref_n1024`
- `scalar_call_ref_n10000`
- `scalar_call_ref_n0`
- `scalar_call_ref_n17`
- `scalar_table_mutation_n1`
- `scalar_table_mutation_n32`
- `scalar_table_mutation_n1024`
- `scalar_table_mutation_n10000`
- `scalar_table_mutation_n0`
- `scalar_table_mutation_n17`
- `scalar_host_call_n1`
- `scalar_host_call_n32`
- `scalar_host_call_n1024`
- `scalar_host_call_n10000`
- `scalar_host_call_n0`
- `scalar_host_call_n17`
- `scalar_try_table_normal_n1`
- `scalar_try_table_normal_n32`
- `scalar_try_table_normal_n1024`
- `scalar_try_table_normal_n10000`
- `scalar_try_table_normal_n0`
- `scalar_try_table_normal_n17`

The contract accounts for all 85 selected IDs: 8 primary plus 77 protected, including the four named held-out controls. All selected workloads currently declare the same five stages: `prepare`, `instantiate`, `first_call`, `warm_call`, and `load_and_call`. Preserve their registered reset policies and require all five stages for every selected workload in both runs. The gate reads stages from `workloads.json`; the promotion contract schema intentionally contains only schema version, selection, primary workload IDs, and protected workload IDs.

## Measurement and interpretation

Run two independently seeded paired runs with at least 30 randomized baseline/candidate pairs per workload/stage, decoded mode, and the exact same 85-row selection. Keep all registered correctness oracles and probes active. The primary-family decision uses an equally weighted geometric mean over the eight primary workload medians: each run must show at least 5% improvement with its 95% interval wholly below ratio 1.00. Each protected warm workload/input group must have an upper regression interval no greater than 1.05 in both runs. Preserve the existing cold gates for `prepare`, `instantiate`, `first_call`, and `load_and_call`, and report every declared stage without collapsing unlike stages.

The selected primary WATs support performance claims for i32 add, unsigned loop comparison, and (for `scalar_dependent`) i32 multiply. The broader selected controls also include some i32 subtract, bitwise AND/XOR, `eqz`, `ne`, and branching mixes. No timed primary workload executes i64 arithmetic/comparison/unary operations. No selected timed workload executes clz/ctz/popcnt, `or`, shifts, or rotates; those remain correctness-only coverage unless the benchmark model is deliberately expanded before baseline archival. The reference executor shares `numeric::exec`, so decoded/reference agreement is not an independent arithmetic oracle; use the independent Python workload oracle and literal/helper-level edge-case expectations for numeric results.

## Build and cold-start gate

Use `target/performance/P028-before` as the immutable immediate-predecessor build baseline. Measure one quiet before build and one quiet candidate build with the same toolchain, target, feature set, clean/unchanged state, and resource constraints. Do not retry or substitute samples after observing results. Compare every `gates.RELATIVE_LIMITS` field to the immediate predecessor; every candidate/baseline ratio must be at most 1.25. Also require all existing absolute hard limits, default-dependency, source-stability, and build-integrity checks to pass. Record raw before/candidate values and ratios in the decision artifacts.

A build-budget miss, missing/incomplete measurement, source-identity mismatch, or primary/protected/cold gate failure is no-go or inconclusive under the existing semantics. Do not change the primary/control list, thresholds, stages, or task status to turn a failed or noisy measurement into a pass.

## Evidence and status boundary

The P028 contract and this file are drafts only. They contain no benchmark report, build sample, runtime result, promotion decision, or completed-task claim. Evidence should bind the exact 85 workloads, WAT/source and binary hashes, both paired reports, analysis, build-budget artifacts, focused-case counts, and the contract hash. Do not claim measured coverage for i64 or other absent integer operations.
