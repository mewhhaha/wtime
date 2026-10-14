# P023 — Fetch ordinary instructions without cloning their payload

Status: **done**

Outcome: **inconclusive**

Kind: `candidate` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P002 — Compare separately built baseline and candidate binaries](../00-measurement/P002-paired-driver.md)

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P008 — Add scaling, branch and call benchmark families](../00-measurement/P008-scalar-workloads.md)

[P016 — Add explicit executor policies and test integration hooks](../01-execution-contracts/P016-execution-policy.md)

[P017 — Make every Module clone share immutable metadata](../01-execution-contracts/P017-shared-module-payload.md)

[P021 — Compute bounded per-function execution metadata](../01-execution-contracts/P021-function-execution-plans.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/execution.rs`
- `crates/wtime-runtime/tests/performance/p023.rs`

- `crates/wtime-runtime/src/perf.rs`
- `crates/wtime-runtime/tests/performance.rs`
- `crates/wtime-runtime/tests/fixtures/performance/borrow/*`
- `tools/wtime-bench/gates.py`
- `tools/wtime-bench/tests/test_p023_contract_gate.py`

- `docs/benchmarks/performance/P023-contract.json`
- `docs/benchmarks/performance/P023-evidence.json`
- `docs/benchmarks/performance/P023-candidate.patch`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

Decoded dispatch borrows immutable instructions from one retained module handle; vector-backed immediates are borrowed or indexed, with owned copies only at explicit state-capture boundaries.

## Implementation recipe

1. Split immutable code ownership from mutable Store execution so normal fetch borrows an Instruction instead of cloning the Immediate enum each dispatch.
2. Narrow borrows around Store-mutating helpers; if a handle clone is needed, clone only the cheap Arc payload from P017, never the decoded instruction or side vectors.
3. Change helper signatures to borrowed immediates where safe and retain explicit ownership for catch/handler data kept in frames or continuations.
4. Compare instruction-clone diagnostic events and scalar/control timings; preserve source opcode coverage, span and PC behavior.
5. Record the changed path with focused assertions/diagnostic counts and exact source/binary identities; compare the pre-edit archive, and revert a rejected structural optimization rather than adding a permanent toggle solely for testing.

## Meaningful cases

- br_table and typed-select vector immediates execute without clone events.
- try_table/resume handler metadata remains alive through capture and reentry.
- Numeric/local hot loops produce identical results, fuel and source coverage.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime p023_ -- --nocapture
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml -j 4
python3 tools/wtime-bench/compare.py --baseline target/performance/P023-final-before --candidate tools/wtime-bench/target/release/wtime-bench --candidate-mode decoded --selection scalar --output target/performance/P023
python3 tools/wtime-bench/analyze.py --input target/performance/P023 --output target/performance/P023/analysis.json
python3 tools/checks/build_budget.py --output target/performance/P023/build.json
python3 tools/wtime-bench/gates.py --task P023 --comparison target/performance/P023 --build target/performance/P023/build.json --contract docs/benchmarks/performance/P023-contract.json --output target/performance/P023/decision.json
```

## Measurement and decision

Selection: `scalar`. Explicit execution setting: `decoded`. Decision rule: `measured_promotion`.

Use the paired protocol and family/control/cold/build rules in [MEASUREMENT.md](../MEASUREMENT.md). Predeclare the targeted family and held-out controls before changing the implementation. Keep instrumentation off in production timings. Adopt only with supported gains in both runs; otherwise record rejected/inconclusive and remove default-path cost. A necessary immutable-interface change may be contract_only with its separate justification.

## Before editing

Run these commands before changing the candidate sources. The archive is immutable and captures the immediate predecessor. Retain P001's original archive for milestone totals. For native work, use the named portable control and verify its actual policy/quantum; if this archive already exists with different sources, choose a new recorded run-specific directory and update the comparison path rather than overwriting it.

```sh
python3 tools/wtime-bench/baseline.py --output target/performance/P023-final-before
```

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. Executed evidence is recorded below.

## Predeclared family contract

Before the candidate dispatch edit, `target/performance/P023-contract.json` enumerated all 79 scalar workloads: the 50 P008 primary cases plus `integer_loop` and `branch_calls` form the primary family; the 26 P008 held-out cases plus `call_overhead` form protected controls. Contract SHA-256: `fdbae8d5ad595168fd27f73d062d7196be183ed22969bef83a39cd56a336eb62`. No workload is removed. Family promotion uses equally weighted workload medians and 10,000 paired bootstrap resamples; every individual warm group retains the 1.05 regression bound. The contract-aware gate corrects the old gate that treated held-out controls as primary and overwrote multiple input/fixture groups.

The first trial patch passed five focused tests, then was restored before the final benchmark-harness snapshot. `P023-final-before` freezes the unchanged predecessor core with the completed memory harness; earlier `P023-before` and `P023-before-memory-harness` archives remain immutable historical snapshots. Final candidate measurements compare identical harness Rust leaves against `P023-final-before`.

The byte-identical predeclared contract is retained in `docs/benchmarks/performance/P023-contract.json`, so fresh checkouts and tests do not depend on an ignored target artifact.

## Executed evidence

```json
{
  "builds": {
    "candidate_before_rollback": {
      "budget_pass": true,
      "clean_seconds": 13.380958232999546,
      "default_dependency_closure_pass": true,
      "leaf_seconds": 0.7914848969958257,
      "release_seconds": 25.17184570700192,
      "runtime_leaf_seconds": 2.6003097869979683,
      "source_sha256": "c2462df6cab24bf2115ca94a4f93e9d64454d871a671f40b0f1a6c1d1e96ad40",
      "source_sha256_after": "c2462df6cab24bf2115ca94a4f93e9d64454d871a671f40b0f1a6c1d1e96ad40",
      "source_stable": true,
      "stripped_bytes": 997704
    },
    "relative_growth": {
      "clean_seconds": 1.9742192143086739,
      "leaf_seconds": 2.992089867278887,
      "release_seconds": 1.699337013824561
    },
    "restored_after_rollback": {
      "budget_pass": true,
      "clean_seconds": 11.469917409995105,
      "default_dependency_closure_pass": true,
      "leaf_seconds": 0.6406223219964886,
      "release_seconds": 22.467193958997086,
      "runtime_leaf_seconds": 1.3151658040005714,
      "source_sha256": "bffd2c459a14b64880f0489721cbcb31fe13ed15f2c012c5d9c2bfe233d37302",
      "source_sha256_after": "bffd2c459a14b64880f0489721cbcb31fe13ed15f2c012c5d9c2bfe233d37302",
      "source_stable": true,
      "stripped_bytes": 1000264
    }
  },
  "diagnostics": {
    "binary_sha256": {
      "baseline": "f2517f5968ead673ec73cbc6300a0dd1f4e0a5696eee67899d6fd705d95b0366",
      "candidate": "0ab4d02b99317ad34e29fba794edd0035458978bbec1dcd2218c474cc5f440fb"
    },
    "fixture_wasm": {
      "scalar_accumulator_n17.wasm": {
        "sha256": "ef61bf7afc44af7b71ef78eadcba48c0d85949d7d9096df8fbbbab53c05036b1",
        "size_bytes": 108
      },
      "scalar_br_table_n32.wasm": {
        "sha256": "d43d5dd71d5ca1c3dae820aa85c13a2d69415e028b8d48097fed4558f743163a",
        "size_bytes": 222
      },
      "scalar_multivalue_branch_n17.wasm": {
        "sha256": "03ffdc2041ae8ba0c8d078422d36152cf1d35f1293c94a4074fe7e83791743ae",
        "size_bytes": 199
      }
    },
    "observations": 6,
    "passed": true,
    "path": "/workspace/wtime/target/performance/P023/diagnostics/report.json",
    "rows": [
      {
        "case": "scalar_accumulator_n17",
        "instruction_clone_bytes": 0,
        "instruction_clone_events": 229,
        "raw_stderr_sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        "raw_stdout_sha256": "ae2a3e491d908e8032ec18a654b92bd1f1487781d31d7196f8261a418e8102ef",
        "result": 136,
        "side": "baseline",
        "source_dispatches": 229
      },
      {
        "case": "scalar_accumulator_n17",
        "instruction_clone_bytes": 0,
        "instruction_clone_events": 0,
        "raw_stderr_sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        "raw_stdout_sha256": "cbaaa2881d62964a1d1058a9f4a3dc824f75cb2f90ca2589e1d974cdaff16be4",
        "result": 136,
        "side": "candidate",
        "source_dispatches": 229
      },
      {
        "case": "scalar_br_table_n32",
        "instruction_clone_bytes": 512,
        "instruction_clone_events": 776,
        "raw_stderr_sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        "raw_stdout_sha256": "c7b9d00a7c0cddbbf500432e4813174b3da0c5396af52dc6c4b229d618ac0b2a",
        "result": 192,
        "side": "baseline",
        "source_dispatches": 776
      },
      {
        "case": "scalar_br_table_n32",
        "instruction_clone_bytes": 0,
        "instruction_clone_events": 0,
        "raw_stderr_sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        "raw_stdout_sha256": "bc04532514e3675492f0a14a2fcd8b9b73ecc6a91b1a593ce94bbc9495001701",
        "result": 192,
        "side": "candidate",
        "source_dispatches": 776
      },
      {
        "case": "scalar_multivalue_branch_n17",
        "instruction_clone_bytes": 0,
        "instruction_clone_events": 460,
        "raw_stderr_sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        "raw_stdout_sha256": "7f819d096feadec92cb209abde3645893972300c4691551e0eba6d2430c224d4",
        "result": 29523,
        "side": "baseline",
        "source_dispatches": 460
      },
      {
        "case": "scalar_multivalue_branch_n17",
        "instruction_clone_bytes": 0,
        "instruction_clone_events": 0,
        "raw_stderr_sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        "raw_stdout_sha256": "b639109f9f3cff33d12059ab31cfa292b37363d5c9544ab8cb9522593c7f236b",
        "result": 29523,
        "side": "candidate",
        "source_dispatches": 460
      }
    ],
    "sha256": "8431523bddcbc05181cf51c4fc8f783a073273c7570ada04f777295832dceb65",
    "source_stable": true
  },
  "gate": {
    "decision": "inconclusive",
    "default_build_budget_pass": true,
    "full_reason_count": 246,
    "full_reason_list_sha256": "cd00d0dbbc5c6ce72040b3dd0f69e78e376cdeccf490554d88b07f05cb1f0585",
    "reason_summary": [
      "multiple cold/control cells crossed the gate uncertainty thresholds",
      "primary-family confidence result did not clear the full gate",
      "relative build growth versus frozen original remained uncertain"
    ],
    "relative_build_growth_to_frozen_original": {
      "clean_seconds": 1.9742192143086739,
      "leaf_seconds": 2.992089867278887,
      "release_seconds": 1.699337013824561
    }
  },
  "identities": {
    "analysis": {
      "path": "/workspace/wtime/target/performance/P023/analysis.json",
      "sha256": "5645266d571a1e9e0d6277d3acbcdedf831ebb7b4c7beb4fca4bc4b8444ffad3",
      "size_bytes": 1162748
    },
    "baseline_binary": {
      "sha256": "71485b2febdf3f3475fae528d777d22191a689b5f4bbdcf409e8a938be64d11a"
    },
    "candidate_build_source_digest": "c2462df6cab24bf2115ca94a4f93e9d64454d871a671f40b0f1a6c1d1e96ad40",
    "candidate_source_map": {
      "canonical_sha256": "bd62b02f0e3cc944bf4a528abbdf2aba74189626ec1d8933b6d4923228cabd89",
      "file_count": 84,
      "runtime_execution_rs_sha256": "c7e3ee3e36bf998fbb7fd8fa04656eb767ba70853becf12b4437298222b7e777",
      "runtime_perf_rs_sha256": "194102fe2d8fb58eb9bfd97e257f703403fd364dde3a4b41f8ead7232bcde53c"
    },
    "candidate_trial_binary": {
      "persisted_in_main_trial_directory": false,
      "sha256": "929d0a0109d5efea8f055dc8f2ca6a34b2d9dd358b7f3791152e85849ad1ed6a"
    },
    "decision": {
      "decision": "inconclusive",
      "path": "/workspace/wtime/target/performance/P023/decision.json",
      "sha256": "011e92360228d919c7ed8cd0c8d01cbe9b3a3bc9d130e10cccf9c4eb83d50b53",
      "size_bytes": 1583712
    },
    "fixture_encoder_sha256": "6616cabefdaf1cb99514a43fc27b63a039210727ac0ee22e2dab3cfb2f5b4ce7",
    "fixtures": {
      "count": 79,
      "identities": [
        {
          "id": "call_overhead",
          "sha256": "0815ca7d6026206d0773fe10c6b9643862bfffe8ba361c9789369ef9668ee746"
        },
        {
          "id": "integer_loop",
          "sha256": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c"
        },
        {
          "id": "branch_calls",
          "sha256": "9b58d26f201900b5b6cce684133a98ab68b3cf55d979b64ce895e914e9d5327b"
        },
        {
          "id": "scalar_accumulator_n1",
          "sha256": "ef61bf7afc44af7b71ef78eadcba48c0d85949d7d9096df8fbbbab53c05036b1"
        },
        {
          "id": "scalar_accumulator_n32",
          "sha256": "ef61bf7afc44af7b71ef78eadcba48c0d85949d7d9096df8fbbbab53c05036b1"
        },
        {
          "id": "scalar_accumulator_n1024",
          "sha256": "ef61bf7afc44af7b71ef78eadcba48c0d85949d7d9096df8fbbbab53c05036b1"
        },
        {
          "id": "scalar_accumulator_n10000",
          "sha256": "ef61bf7afc44af7b71ef78eadcba48c0d85949d7d9096df8fbbbab53c05036b1"
        },
        {
          "id": "scalar_accumulator_n0",
          "sha256": "ef61bf7afc44af7b71ef78eadcba48c0d85949d7d9096df8fbbbab53c05036b1"
        },
        {
          "id": "scalar_accumulator_n17",
          "sha256": "ef61bf7afc44af7b71ef78eadcba48c0d85949d7d9096df8fbbbab53c05036b1"
        },
        {
          "id": "scalar_dependent_n1",
          "sha256": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c"
        },
        {
          "id": "scalar_dependent_n32",
          "sha256": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c"
        },
        {
          "id": "scalar_dependent_n1024",
          "sha256": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c"
        },
        {
          "id": "scalar_dependent_n10000",
          "sha256": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c"
        },
        {
          "id": "scalar_dependent_n0",
          "sha256": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c"
        },
        {
          "id": "scalar_dependent_n17",
          "sha256": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c"
        },
        {
          "id": "scalar_branch_entropy_n1",
          "sha256": "e4c87dbab50a2438cb1f749bc6fd6f9c834c81484acd42864735bd1ad1b6dec0"
        },
        {
          "id": "scalar_branch_entropy_n32",
          "sha256": "e4c87dbab50a2438cb1f749bc6fd6f9c834c81484acd42864735bd1ad1b6dec0"
        },
        {
          "id": "scalar_branch_entropy_n1024",
          "sha256": "e4c87dbab50a2438cb1f749bc6fd6f9c834c81484acd42864735bd1ad1b6dec0"
        },
        {
          "id": "scalar_branch_entropy_n10000",
          "sha256": "e4c87dbab50a2438cb1f749bc6fd6f9c834c81484acd42864735bd1ad1b6dec0"
        },
        {
          "id": "scalar_branch_entropy_n0",
          "sha256": "e4c87dbab50a2438cb1f749bc6fd6f9c834c81484acd42864735bd1ad1b6dec0"
        },
        {
          "id": "scalar_branch_entropy_n17",
          "sha256": "e4c87dbab50a2438cb1f749bc6fd6f9c834c81484acd42864735bd1ad1b6dec0"
        },
        {
          "id": "scalar_br_table_n1",
          "sha256": "d43d5dd71d5ca1c3dae820aa85c13a2d69415e028b8d48097fed4558f743163a"
        },
        {
          "id": "scalar_br_table_n32",
          "sha256": "d43d5dd71d5ca1c3dae820aa85c13a2d69415e028b8d48097fed4558f743163a"
        },
        {
          "id": "scalar_br_table_n1024",
          "sha256": "d43d5dd71d5ca1c3dae820aa85c13a2d69415e028b8d48097fed4558f743163a"
        },
        {
          "id": "scalar_br_table_n10000",
          "sha256": "d43d5dd71d5ca1c3dae820aa85c13a2d69415e028b8d48097fed4558f743163a"
        },
        {
          "id": "scalar_br_table_n0",
          "sha256": "d43d5dd71d5ca1c3dae820aa85c13a2d69415e028b8d48097fed4558f743163a"
        },
        {
          "id": "scalar_br_table_n17",
          "sha256": "d43d5dd71d5ca1c3dae820aa85c13a2d69415e028b8d48097fed4558f743163a"
        },
        {
          "id": "scalar_direct_calls_n1",
          "sha256": "f6adbecce99c05bf06f213f4f5c0d5d9f001074acbbb2e6bbe814696bb866e45"
        },
        {
          "id": "scalar_direct_calls_n32",
          "sha256": "f6adbecce99c05bf06f213f4f5c0d5d9f001074acbbb2e6bbe814696bb866e45"
        },
        {
          "id": "scalar_direct_calls_n1024",
          "sha256": "f6adbecce99c05bf06f213f4f5c0d5d9f001074acbbb2e6bbe814696bb866e45"
        },
        {
          "id": "scalar_direct_calls_n10000",
          "sha256": "f6adbecce99c05bf06f213f4f5c0d5d9f001074acbbb2e6bbe814696bb866e45"
        },
        {
          "id": "scalar_direct_calls_n0",
          "sha256": "f6adbecce99c05bf06f213f4f5c0d5d9f001074acbbb2e6bbe814696bb866e45"
        },
        {
          "id": "scalar_direct_calls_n17",
          "sha256": "f6adbecce99c05bf06f213f4f5c0d5d9f001074acbbb2e6bbe814696bb866e45"
        },
        {
          "id": "scalar_indirect_calls_n1",
          "sha256": "4b42e82f54ea6ab7d0f62c70766c904e41d6a408e57fa14b5abbf514c0da78a9"
        },
        {
          "id": "scalar_indirect_calls_n32",
          "sha256": "4b42e82f54ea6ab7d0f62c70766c904e41d6a408e57fa14b5abbf514c0da78a9"
        },
        {
          "id": "scalar_indirect_calls_n1024",
          "sha256": "4b42e82f54ea6ab7d0f62c70766c904e41d6a408e57fa14b5abbf514c0da78a9"
        },
        {
          "id": "scalar_indirect_calls_n10000",
          "sha256": "4b42e82f54ea6ab7d0f62c70766c904e41d6a408e57fa14b5abbf514c0da78a9"
        },
        {
          "id": "scalar_indirect_calls_n0",
          "sha256": "4b42e82f54ea6ab7d0f62c70766c904e41d6a408e57fa14b5abbf514c0da78a9"
        },
        {
          "id": "scalar_indirect_calls_n17",
          "sha256": "4b42e82f54ea6ab7d0f62c70766c904e41d6a408e57fa14b5abbf514c0da78a9"
        },
        {
          "id": "scalar_tail_calls_n1",
          "sha256": "3e23be294e7a9eda62c89dbc366b6a6cab90edfbda532da27a7e4fc6d0aa4a56"
        },
        {
          "id": "scalar_tail_calls_n32",
          "sha256": "3e23be294e7a9eda62c89dbc366b6a6cab90edfbda532da27a7e4fc6d0aa4a56"
        },
        {
          "id": "scalar_tail_calls_n1024",
          "sha256": "3e23be294e7a9eda62c89dbc366b6a6cab90edfbda532da27a7e4fc6d0aa4a56"
        },
        {
          "id": "scalar_tail_calls_n10000",
          "sha256": "3e23be294e7a9eda62c89dbc366b6a6cab90edfbda532da27a7e4fc6d0aa4a56"
        },
        {
          "id": "scalar_tail_calls_n0",
          "sha256": "3e23be294e7a9eda62c89dbc366b6a6cab90edfbda532da27a7e4fc6d0aa4a56"
        },
        {
          "id": "scalar_tail_calls_n17",
          "sha256": "3e23be294e7a9eda62c89dbc366b6a6cab90edfbda532da27a7e4fc6d0aa4a56"
        },
        {
          "id": "scalar_dynamic_api_n1",
          "sha256": "c493d0aae68d0c5aa67fc0b562e616c45eb3d42b0302951158a763fa202ce589"
        },
        {
          "id": "scalar_dynamic_api_n32",
          "sha256": "c493d0aae68d0c5aa67fc0b562e616c45eb3d42b0302951158a763fa202ce589"
        },
        {
          "id": "scalar_dynamic_api_n1024",
          "sha256": "c493d0aae68d0c5aa67fc0b562e616c45eb3d42b0302951158a763fa202ce589"
        },
        {
          "id": "scalar_dynamic_api_n10000",
          "sha256": "c493d0aae68d0c5aa67fc0b562e616c45eb3d42b0302951158a763fa202ce589"
        },
        {
          "id": "scalar_dynamic_api_n0",
          "sha256": "c493d0aae68d0c5aa67fc0b562e616c45eb3d42b0302951158a763fa202ce589"
        },
        {
          "id": "scalar_dynamic_api_n17",
          "sha256": "c493d0aae68d0c5aa67fc0b562e616c45eb3d42b0302951158a763fa202ce589"
        },
        {
          "id": "scalar_multivalue_branch_n1",
          "sha256": "03ffdc2041ae8ba0c8d078422d36152cf1d35f1293c94a4074fe7e83791743ae"
        },
        {
          "id": "scalar_multivalue_branch_n32",
          "sha256": "03ffdc2041ae8ba0c8d078422d36152cf1d35f1293c94a4074fe7e83791743ae"
        },
        {
          "id": "scalar_multivalue_branch_n1024",
          "sha256": "03ffdc2041ae8ba0c8d078422d36152cf1d35f1293c94a4074fe7e83791743ae"
        },
        {
          "id": "scalar_multivalue_branch_n10000",
          "sha256": "03ffdc2041ae8ba0c8d078422d36152cf1d35f1293c94a4074fe7e83791743ae"
        },
        {
          "id": "scalar_multivalue_branch_n0",
          "sha256": "03ffdc2041ae8ba0c8d078422d36152cf1d35f1293c94a4074fe7e83791743ae"
        },
        {
          "id": "scalar_multivalue_branch_n17",
          "sha256": "03ffdc2041ae8ba0c8d078422d36152cf1d35f1293c94a4074fe7e83791743ae"
        },
        {
          "id": "scalar_ordinary_recursion_n0",
          "sha256": "e1bd7ea6ada8b15b2770ac387f8fc2256f20d7f87c4bc34e7559b239d8875ba1"
        },
        {
          "id": "scalar_ordinary_recursion_n1",
          "sha256": "e1bd7ea6ada8b15b2770ac387f8fc2256f20d7f87c4bc34e7559b239d8875ba1"
        },
        {
          "id": "scalar_ordinary_recursion_n17",
          "sha256": "e1bd7ea6ada8b15b2770ac387f8fc2256f20d7f87c4bc34e7559b239d8875ba1"
        },
        {
          "id": "scalar_ordinary_recursion_n32",
          "sha256": "e1bd7ea6ada8b15b2770ac387f8fc2256f20d7f87c4bc34e7559b239d8875ba1"
        },
        {
          "id": "scalar_call_ref_n1",
          "sha256": "4f766dc9e29324ea942c892982363dec0ade98565288878a70967ce3b33f322b"
        },
        {
          "id": "scalar_call_ref_n32",
          "sha256": "4f766dc9e29324ea942c892982363dec0ade98565288878a70967ce3b33f322b"
        },
        {
          "id": "scalar_call_ref_n1024",
          "sha256": "4f766dc9e29324ea942c892982363dec0ade98565288878a70967ce3b33f322b"
        },
        {
          "id": "scalar_call_ref_n10000",
          "sha256": "4f766dc9e29324ea942c892982363dec0ade98565288878a70967ce3b33f322b"
        },
        {
          "id": "scalar_call_ref_n0",
          "sha256": "4f766dc9e29324ea942c892982363dec0ade98565288878a70967ce3b33f322b"
        },
        {
          "id": "scalar_call_ref_n17",
          "sha256": "4f766dc9e29324ea942c892982363dec0ade98565288878a70967ce3b33f322b"
        },
        {
          "id": "scalar_table_mutation_n1",
          "sha256": "458f2b111e48d2efca98083b8c56860c43e82be8752813d1e2814d23c28d4f43"
        },
        {
          "id": "scalar_table_mutation_n32",
          "sha256": "458f2b111e48d2efca98083b8c56860c43e82be8752813d1e2814d23c28d4f43"
        },
        {
          "id": "scalar_table_mutation_n1024",
          "sha256": "458f2b111e48d2efca98083b8c56860c43e82be8752813d1e2814d23c28d4f43"
        },
        {
          "id": "scalar_table_mutation_n10000",
          "sha256": "458f2b111e48d2efca98083b8c56860c43e82be8752813d1e2814d23c28d4f43"
        },
        {
          "id": "scalar_table_mutation_n0",
          "sha256": "458f2b111e48d2efca98083b8c56860c43e82be8752813d1e2814d23c28d4f43"
        },
        {
          "id": "scalar_table_mutation_n17",
          "sha256": "458f2b111e48d2efca98083b8c56860c43e82be8752813d1e2814d23c28d4f43"
        },
        {
          "id": "scalar_host_call_n1",
          "sha256": "c60311d57c929c3ff1d29adb70501e7bbf8419517acbd86c8d072438b610ebb7"
        },
        {
          "id": "scalar_host_call_n32",
          "sha256": "c60311d57c929c3ff1d29adb70501e7bbf8419517acbd86c8d072438b610ebb7"
        },
        {
          "id": "scalar_host_call_n1024",
          "sha256": "c60311d57c929c3ff1d29adb70501e7bbf8419517acbd86c8d072438b610ebb7"
        },
        {
          "id": "scalar_host_call_n10000",
          "sha256": "c60311d57c929c3ff1d29adb70501e7bbf8419517acbd86c8d072438b610ebb7"
        },
        {
          "id": "scalar_host_call_n0",
          "sha256": "c60311d57c929c3ff1d29adb70501e7bbf8419517acbd86c8d072438b610ebb7"
        },
        {
          "id": "scalar_host_call_n17",
          "sha256": "c60311d57c929c3ff1d29adb70501e7bbf8419517acbd86c8d072438b610ebb7"
        }
      ]
    },
    "frozen_archive_manifest": {
      "manifest_sha256_in_report": "69d6a194aa1e8e6f0848b58290a015542a43ec5dca55265e6bded99337b53571",
      "path": "/workspace/wtime/target/performance/P023-final-before/manifest.json",
      "sha256": "69d6a194aa1e8e6f0848b58290a015542a43ec5dca55265e6bded99337b53571"
    },
    "full_report": {
      "path": "/workspace/wtime/target/performance/P023/report.json",
      "preserved_historical_identity": true,
      "sha256": "7a18bbcd0a09f235bddfbcfc0a0a396bc85ef847693f5d8f9204806802460ad6",
      "size_bytes": 55431232
    },
    "initial_decision": {
      "path": "/workspace/wtime/target/performance/P023/decision-initial.json",
      "sha256": "011e92360228d919c7ed8cd0c8d01cbe9b3a3bc9d130e10cccf9c4eb83d50b53",
      "size_bytes": 1583712
    },
    "protocol_sha256": "517263c4715f8eca43b6e878ac2a8d2e5b6fd85c70b654c8946f0aba49610929"
  },
  "logs": {
    "P023-build-budget.log": {
      "path": "/workspace/wtime/target/performance/P023-build-budget.log",
      "sha256": "e12a6200db8efa9f81ce2eaa27278a3384212f19c126314b8785b775535e33be",
      "size_bytes": 1703
    },
    "P023-compare.log": {
      "path": "/workspace/wtime/target/performance/P023-compare.log",
      "sha256": "0912fed32f99794409765d8798b11ecf2332704e17b929afef7972576d2a0eee",
      "size_bytes": 44870
    },
    "P023-diagnostics.log": {
      "path": "/workspace/wtime/target/performance/P023-diagnostics.log",
      "sha256": "e24ac5365ecbc4a5496de4a1a519808180282472c5dab239774b025aa15f720a",
      "size_bytes": 118
    },
    "P023-restored-build.log": {
      "path": "/workspace/wtime/target/performance/P023-restored-build.log",
      "sha256": "98a845d2578e171ee4b4455a72062413ec7e3c9563cfaad8db506aa512922442",
      "size_bytes": 1705
    },
    "P023-restored-tests.log": {
      "path": "/workspace/wtime/target/performance/P023-restored-tests.log",
      "sha256": "e7c1de9a9935f9cf6e7005c2b49b91ced569643766e46eaf7c1e41be1e095d1b",
      "size_bytes": 12661
    },
    "P023-runtime-tests.log": {
      "path": "/workspace/wtime/target/performance/P023-runtime-tests.log",
      "sha256": "c21adaf592e920def3714fd220c65f2c1cbfb1c7ddd5f17173358dd5f3d93c59",
      "size_bytes": 12668
    },
    "P023-tests.log": {
      "path": "/workspace/wtime/target/performance/P023-tests.log",
      "sha256": "4007e5e355206d0e89eb7fae0bbd5ddaa7ad83cbc206bcd1f189cebbaa004801",
      "size_bytes": 1673
    }
  },
  "outcome": "inconclusive",
  "rollback": {
    "byte_comparison": {
      "execution.rs": {
        "byte_identical": true,
        "frozen_pretrial_sha256": "1baf160513d2daf53706f2bc03a54f7d01777b75da65656c5a6bedb4e995f4a5",
        "workspace_sha256": "1baf160513d2daf53706f2bc03a54f7d01777b75da65656c5a6bedb4e995f4a5"
      },
      "perf.rs": {
        "byte_identical": true,
        "frozen_pretrial_sha256": "f523ab7bc803da4fdcd5365c7c310b572ff7704252c245530e37a21ac740f59b",
        "workspace_sha256": "f523ab7bc803da4fdcd5365c7c310b572ff7704252c245530e37a21ac740f59b"
      }
    },
    "source_tree_restored": true,
    "tests_retained_with_counter_assertions_removed_and_test_names_adjusted": true
  },
  "schema": "wtime-p023-evidence-v1",
  "scope": "Implemented, tested and measured borrowed dispatch; inconclusive gate; default core restored and validated.",
  "task": "P023",
  "tests": {
    "P023-restored-tests.log": {
      "failed_total": 0,
      "passed_total": 172,
      "targets": [
        {
          "failed": 0,
          "filtered": 0,
          "ignored": 0,
          "measured": 0,
          "passed": 104,
          "status": "ok"
        },
        {
          "failed": 0,
          "filtered": 0,
          "ignored": 0,
          "measured": 0,
          "passed": 8,
          "status": "ok"
        },
        {
          "failed": 0,
          "filtered": 0,
          "ignored": 0,
          "measured": 0,
          "passed": 5,
          "status": "ok"
        },
        {
          "failed": 0,
          "filtered": 0,
          "ignored": 0,
          "measured": 0,
          "passed": 28,
          "status": "ok"
        },
        {
          "failed": 0,
          "filtered": 0,
          "ignored": 0,
          "measured": 0,
          "passed": 23,
          "status": "ok"
        },
        {
          "failed": 0,
          "filtered": 0,
          "ignored": 0,
          "measured": 0,
          "passed": 4,
          "status": "ok"
        },
        {
          "failed": 0,
          "filtered": 0,
          "ignored": 0,
          "measured": 0,
          "passed": 0,
          "status": "ok"
        }
      ]
    },
    "P023-runtime-tests.log": {
      "failed_total": 0,
      "passed_total": 172,
      "targets": [
        {
          "failed": 0,
          "filtered": 0,
          "ignored": 0,
          "measured": 0,
          "passed": 104,
          "status": "ok"
        },
        {
          "failed": 0,
          "filtered": 0,
          "ignored": 0,
          "measured": 0,
          "passed": 8,
          "status": "ok"
        },
        {
          "failed": 0,
          "filtered": 0,
          "ignored": 0,
          "measured": 0,
          "passed": 5,
          "status": "ok"
        },
        {
          "failed": 0,
          "filtered": 0,
          "ignored": 0,
          "measured": 0,
          "passed": 28,
          "status": "ok"
        },
        {
          "failed": 0,
          "filtered": 0,
          "ignored": 0,
          "measured": 0,
          "passed": 23,
          "status": "ok"
        },
        {
          "failed": 0,
          "filtered": 0,
          "ignored": 0,
          "measured": 0,
          "passed": 4,
          "status": "ok"
        },
        {
          "failed": 0,
          "filtered": 0,
          "ignored": 0,
          "measured": 0,
          "passed": 0,
          "status": "ok"
        }
      ]
    },
    "focused_candidate_log": {
      "path": "/workspace/wtime/target/performance/P023-tests.log",
      "sha256": "4007e5e355206d0e89eb7fae0bbd5ddaa7ad83cbc206bcd1f189cebbaa004801",
      "size_bytes": 1673
    }
  },
  "trial": {
    "comparison": "candidate runtime vs frozen current-source archive",
    "design": {
      "runs": 2,
      "samples_per_cell_per_run": 30,
      "stages_per_workload": 5,
      "workloads": 79
    },
    "engine": "wtime on both sides; this is not a Wasmtime comparison",
    "immediate_predecessor_comparison": {
      "adopted": false,
      "engine": "wtime",
      "is_wasmtime": false
    },
    "instrumented": false,
    "mode": "decoded",
    "paired_observation_count": 23700,
    "primary_family_ci": {
      "reported_true": true,
      "source": "parent trial record; primary-family confidence result",
      "values": [
        0.7313,
        0.736
      ]
    },
    "profile": "release",
    "runs": [
      {
        "pairs": 11850,
        "run_id": 0,
        "seed": 20261011
      },
      {
        "pairs": 11850,
        "run_id": 1,
        "seed": 21261014
      }
    ],
    "settings": {
      "baseline_mode": "decoded",
      "cancellation_quantum": 1,
      "candidate_mode": "decoded",
      "feature_profiles": [
        "all-phase3-core",
        "mvp"
      ],
      "features": "per-workload",
      "fuel": false,
      "instrumented": false,
      "profile": "release"
    },
    "source_stable": true
  },
  "candidate_adopted": false,
  "artifacts": [
    "docs/benchmarks/performance/P023-evidence.json",
    "docs/benchmarks/performance/P023-candidate.patch",
    "docs/benchmarks/performance/P023-contract.json",
    "target/performance/P023-final-before/manifest.json",
    "target/performance/P023/report.json",
    "target/performance/P023/analysis.json",
    "target/performance/P023/build.json",
    "target/performance/P023/restored-build.json",
    "target/performance/P023/decision.json",
    "target/performance/P023/diagnostics/report.json",
    "target/performance/P023-tests.log",
    "target/performance/P023-runtime-tests.log",
    "target/performance/P023-restored-tests.log"
  ],
  "commands": [
    {
      "command": "cargo test --locked -p wtime-runtime --features performance-counters,execution-reference",
      "returncode": 0,
      "candidate_passed": 172,
      "restored_passed": 172
    },
    {
      "command": "python3 tools/wtime-bench/compare.py --baseline target/performance/P023-final-before --candidate tools/wtime-bench/target/release/wtime-bench --candidate-mode decoded --selection scalar --output target/performance/P023",
      "returncode": 0,
      "pairs": 23700,
      "runs": 2
    },
    {
      "command": "python3 tools/wtime-bench/analyze.py --input target/performance/P023 --output target/performance/P023/analysis.json",
      "returncode": 0,
      "groups": 790
    },
    {
      "command": "python3 tools/checks/build_budget.py --output target/performance/P023/build.json",
      "returncode": 0
    },
    {
      "command": "python3 tools/checks/build_budget.py --output target/performance/P023/restored-build.json",
      "returncode": 0
    },
    {
      "command": "python3 tools/wtime-bench/gates.py --task P023 --comparison target/performance/P023 --build target/performance/P023/build.json --contract docs/benchmarks/performance/P023-contract.json --output target/performance/P023/decision-initial.json",
      "returncode": 1,
      "decision": "inconclusive"
    },
    {
      "command": "python3 /tmp/run_p023_diagnostics.py --output target/performance/P023/diagnostics",
      "returncode": 0,
      "observations": 6
    }
  ],
  "limitations": [
    "Candidate was not adopted; warm gains are versus the immediate predecessor, not Wasmtime.",
    "Trial report, gate and diagnostic hashes describe the candidate before rollback. Restored build and test evidence describe the resulting default source.",
    "Cold-stage controls and relative build growth did not satisfy the predeclared gate; no post-hoc threshold changes.",
    "Raw reports and frozen binaries remain local target artifacts; small source-bound summary and candidate patch are retained in the repository."
  ],
  "retained_artifact_sha256": {
    "docs/benchmarks/performance/P023-evidence.json": "59136c18b466f65690f43f9322c3f968fd164ba1773eb9a8843c69fe1b5a1fd3",
    "docs/benchmarks/performance/P023-candidate.patch": "a5bc34bac0139a1372b0f4b20f809800094df18c124606333153de96b9ab6265",
    "docs/benchmarks/performance/P023-contract.json": "fdbae8d5ad595168fd27f73d062d7196be183ed22969bef83a39cd56a336eb62"
  }
}
```
