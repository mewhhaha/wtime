# P011 — Measure reference, GC and exception paths

Status: **done**

Outcome: **contract_only**

Kind: `foundation` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P003 — Define workload selections and engine capability preflight](P003-workload-schema.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `tools/wtime-bench/fixtures/gc/*.wat`
- `tools/wtime-bench/fixtures/exceptions/*.wat`
- `tools/wtime-bench/src/main.rs`
- `tools/wtime-bench/workloads.json`
- `tools/wtime-bench/tests/test_p011_gc.py`

- `tools/wtime-bench/gc_workloads.py`

- `tools/wtime-bench/src/protocol.rs`

- `tools/wtime-bench/src/gc_exception_driver.rs`

- `tools/wtime-bench/gc_scaling.py`

- `tools/wtime-bench/tests/test_p011_driver.py`

- `tools/wtime-bench/tests/test_p011_scaling.py`

- `docs/benchmarks/performance/P011-evidence.json`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

GC/ref cases report allocation/traversal/cast and explicit collection as distinct stages; exception cases distinguish normal path, caught throw and uncaught outcome. Reference identities are normalized per engine.

## Implementation recipe

1. Add struct/array allocate-traverse kernels, packed arrays, repeated nominal/exact casts, descriptor references and typed function references with deterministic object-graph checksums.
2. Add host-root/reentrant-collect and release/reclaim cases; preserve roots intentionally and measure Store::collect separately from ordinary invocation, with CollectionStats live/reclaimed plus Store::gc_payload_bytes() recorded separately.
3. Add nested try_table/cross-call/host exception paths and outcome normalization; compare only semantics actually supported by Wasmtime capability preflight.
4. Populate gc-exceptions selection, separately label engine-specific collection controls and retain current canonical cross-module tests as correctness prerequisites.

## Meaningful cases

- Sole host/local/continuation-held references survive collection and released objects become reclaimable.
- Exact casts reject proper subtypes and cross-module equivalent types agree.
- Normal/caught/uncaught paths return the expected checksum/tag/payload without comparing raw handles.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
python3 -m unittest discover -s tools/wtime-bench/tests -p 'test_p011*.py' -v
```

## Measurement and decision

Selection: `gc-exceptions`. Explicit execution setting: `decoded`. Decision rule: `integrity`.

This is a tooling/contract deliverable. Require correct nonzero checks and bounded/source-bound output; do not invent a runtime speedup requirement. If it changes production work, record cold/build/retention deltas under [MEASUREMENT.md](../MEASUREMENT.md).

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. Executed evidence is recorded below.

## Executed evidence

```json
{
  "artifacts": [
    "/workspace/wtime/target/performance/P011/report.json",
    "/workspace/wtime/target/performance/P011/binary/wtime-bench",
    "/workspace/wtime/target/performance/P011-tests-corrected.log",
    "/workspace/wtime/docs/benchmarks/performance/P011-evidence.json"
  ],
  "build": {
    "corrected_build_log": {
      "path": "/workspace/wtime/target/performance/P011-build-corrected.log",
      "sha256": "2411544b46a01e54422666b93c4aa59e82a48581689c6a08e3066cbaf1ef1190",
      "size_bytes": 131
    },
    "corrected_release_build_seconds": 5.82,
    "earlier_compile_failure_log": {
      "path": "/workspace/wtime/target/performance/P011-build.log",
      "sha256": "da147c351df67af2ca4e9f754b5f5b2b7ddd0ddf72aecd5242d87e0a213c2d93",
      "size_bytes": 788
    },
    "pre_correction_successful_build_log": {
      "path": "/workspace/wtime/target/performance/P011-build-final.log",
      "sha256": "545df2414cd816a2e945c3281aafe4c9d713c5c59d100fa6f2b09cee87dc2e21",
      "size_bytes": 131
    }
  },
  "capabilities": {
    "cases": 45,
    "recorded_reason": "The pinned Wasmtime build does not include the required GC/exception support.",
    "statuses": {
      "unsupported": 45
    },
    "timing": "not measured; pinned Wasmtime build omits GC and exception support",
    "wasmtime_comparator": "pinned Wasmtime 49.0.2 build"
  },
  "command": "python3 tools/wtime-bench/gc_scaling.py --candidate tools/wtime-bench/target/release/wtime-bench --encoder target/debug/wtime-conformance --model tools/wtime-bench/gc_workloads.py --registry tools/wtime-bench/workloads.json --encoded-dir target/performance/P011-fixtures --source-root /workspace/wtime --output target/performance/P011/report.json",
  "draft_corrections": [
    "Initial try_table catch-target fixtures were invalid; the catch targets were corrected, model/encoded fixture hashes refreshed, and the final report binds the corrected files.",
    "The initial consumed-continuation oracle expected zero reclaimed objects; final accounting correctly expects one reclaimed continuation record in addition to guest GC allocations.",
    "The root lifecycle driver sequence was corrected to include the fourth operation.",
    "The initial build log records an Observation Debug trait compile error; the corrected build succeeded.",
    "No runtime core source was changed for P011."
  ],
  "logs": {
    "P011-build-corrected.log": {
      "path": "/workspace/wtime/target/performance/P011-build-corrected.log",
      "sha256": "2411544b46a01e54422666b93c4aa59e82a48581689c6a08e3066cbaf1ef1190",
      "size_bytes": 131
    },
    "P011-build-final.log": {
      "path": "/workspace/wtime/target/performance/P011-build-final.log",
      "sha256": "545df2414cd816a2e945c3281aafe4c9d713c5c59d100fa6f2b09cee87dc2e21",
      "size_bytes": 131
    },
    "P011-build.log": {
      "path": "/workspace/wtime/target/performance/P011-build.log",
      "sha256": "da147c351df67af2ca4e9f754b5f5b2b7ddd0ddf72aecd5242d87e0a213c2d93",
      "size_bytes": 788
    },
    "P011-run.log": {
      "path": "/workspace/wtime/target/performance/P011-run.log",
      "sha256": "36ea53324af5fbe3fc838c7f8e09c54df10415ea2c52d1709ab254109965a539",
      "size_bytes": 109
    },
    "P011-tests-corrected.log": {
      "path": "/workspace/wtime/target/performance/P011-tests-corrected.log",
      "sha256": "da9a9a8fb9f27a6441432418f6e4ab61f804e68ee4601c48c43ded324e6a5eba",
      "size_bytes": 2267
    },
    "P011-tests.log": {
      "path": "/workspace/wtime/target/performance/P011-tests.log",
      "sha256": "5cca1408bda96be9b9a3cef6a80d452e38e22606680c00c4be5e36d9e63f5bbb",
      "size_bytes": 17819
    }
  },
  "measurement": {
    "continuation_accounting_note": "model allocation_count is guest GC allocations; CollectionStats.reclaimed must include those plus one consumed continuation record",
    "exception_arena_note": "Exception arena reclamation is not a public CollectionStats measure; do not report exception reclamation counts as public GC statistics.",
    "fixture_scope": "Source-bound generated guest workload fixtures; not external application benchmarks.",
    "instrumented": false,
    "mode": "decoded",
    "observation_shape": "Each observation records setup/invocation/collection timings separately; root lifecycle and host-reentry cases record their additional named timing fields.",
    "observations": 225,
    "passed": true,
    "profile": "all-phase3-core",
    "samples_per_workload": 5,
    "seed": 20497,
    "source_stable": true,
    "stage_timing_field_counts": {
      "checksum_ns": 15,
      "collect_after_checksum_ns": 15,
      "collect_after_release_ns": 15,
      "collect_ns": 210,
      "collect_with_root_ns": 15,
      "host_collect_ns": 30,
      "invoke_ns": 210,
      "make_ns": 15,
      "setup_ns": 225
    },
    "workloads": 45
  },
  "outcome": "corrected run passed and source-stable; no Wasmtime timing comparison",
  "schema": "wtime-p011-evidence-v1",
  "task": "P011",
  "tests": {
    "corrected": {
      "duration_seconds": 0.64,
      "failed": 0,
      "log": {
        "path": "/workspace/wtime/target/performance/P011-tests-corrected.log",
        "sha256": "da9a9a8fb9f27a6441432418f6e4ab61f804e68ee4601c48c43ded324e6a5eba",
        "size_bytes": 2267
      },
      "passed": 13,
      "skipped": 0,
      "test_names": [
        "test_all_45_registered_cases_run_against_their_independent_oracle (test_p011_driver.GcExceptionDriverDraftTests.test_all_45_registered_cases_run_against_their_independent_oracle) ... ok",
        "test_caught_normal_host_and_uncaught_exception_outcomes (test_p011_driver.GcExceptionDriverDraftTests.test_caught_normal_host_and_uncaught_exception_outcomes) ... ok",
        "test_gc_checksums_collection_and_root_release (test_p011_driver.GcExceptionDriverDraftTests.test_gc_checksums_collection_and_root_release) ... ok",
        "test_nominal_function_reference_and_continuation_roots (test_p011_driver.GcExceptionDriverDraftTests.test_nominal_function_reference_and_continuation_roots) ... ok",
        "test_reentrant_collect_and_descriptor_cross_module (test_p011_driver.GcExceptionDriverDraftTests.test_reentrant_collect_and_descriptor_cross_module) ... ok",
        "test_array_graph_cast_and_typed_call_oracles (test_p011_gc.GcExceptionWorkloadTests.test_array_graph_cast_and_typed_call_oracles) ... ok",
        "test_exception_outcomes_are_normalized_across_normal_caught_and_uncaught_paths (test_p011_gc.GcExceptionWorkloadTests.test_exception_outcomes_are_normalized_across_normal_caught_and_uncaught_paths) ... ok",
        "test_schema_hashes_and_driver_requirements_are_explicit (test_p011_gc.GcExceptionWorkloadTests.test_schema_hashes_and_driver_requirements_are_explicit) ... ok",
        "test_wat_files_are_bound_to_existing_encoder_output (test_p011_gc.GcExceptionWorkloadTests.test_wat_files_are_bound_to_existing_encoder_output) ... ok",
        "test_any_source_or_executable_mutation_invalidates_the_run (test_p011_scaling.P011RunnerTests.test_any_source_or_executable_mutation_invalidates_the_run) ... ok",
        "test_continuation_reclamation_includes_consumed_continuation_record (test_p011_scaling.P011RunnerTests.test_continuation_reclamation_includes_consumed_continuation_record) ... ok",
        "test_single_and_cross_module_dispatch_use_the_public_driver_commands (test_p011_scaling.P011RunnerTests.test_single_and_cross_module_dispatch_use_the_public_driver_commands) ... ok",
        "test_uncaught_exception_tag_and_payload_are_checked_against_oracle (test_p011_scaling.P011RunnerTests.test_uncaught_exception_tag_and_payload_are_checked_against_oracle) ... ok"
      ]
    },
    "initial_draft": {
      "counting_note": "unittest failure/error totals include repeated subTest outcomes; do not add these to the 13 test-method count.",
      "log": {
        "path": "/workspace/wtime/target/performance/P011-tests.log",
        "sha256": "5cca1408bda96be9b9a3cef6a80d452e38e22606680c00c4be5e36d9e63f5bbb",
        "size_bytes": 17819
      },
      "methods_reported_ok": 10,
      "reported_error_count": 6,
      "reported_failure_count": 7,
      "test_methods_run": 13
    }
  },
  "artifact_details": {
    "encoder": {
      "current_path_matches_record": false,
      "current_sha256": "5123b38fd1e6b2358027778d94256202e53ff7024b3b05cd94aa5ea50d7949d4",
      "path": "/workspace/wtime/target/debug/wtime-conformance",
      "recorded_sha256": "6616cabefdaf1cb99514a43fc27b63a039210727ac0ee22e2dab3cfb2f5b4ce7"
    },
    "fixture_encoder_reproduction_checks_count": 16,
    "fixtures": [
      {
        "fixture": "caught_throw",
        "wasm_sha256": "9ae413316b376fffe5b1755f7fb842b8b9d6f3a6deefef9c0e72fff84f080eb9",
        "wat_sha256": "5433ec09235db09ebfd9e02c9ad89afb76732b7b42586c9204d0bbb5c67245f9"
      },
      {
        "fixture": "cross_call_catch",
        "wasm_sha256": "58971ea6345c2693f4fc0cf173b2b13a61a108d5da8bd395de5b4aa70e119f44",
        "wat_sha256": "d8b1444bb104bc896f56ad1e67fca80a5977c58f52e05805b231c4753d2ea521"
      },
      {
        "fixture": "host_throw_catch",
        "wasm_sha256": "16fc8471e8b82acabbb7a1b1c5501ea0b9df884c539f452e49cc4f39c834eec2",
        "wat_sha256": "52c9f9b5da3990de5a948b6614680fc0099891c226d4b6903ed56560a3d2b087"
      },
      {
        "fixture": "normal_path",
        "wasm_sha256": "c4b8a9ad3a665906d9ee6528c32f784520cf1322934645bdb16c5c171ba49721",
        "wat_sha256": "cd26d75fd8d2f01183fd3c40ad2b0eb5865d433a4efe2826cd86dda709e2d9cb"
      },
      {
        "fixture": "throw_ref_unwind",
        "wasm_sha256": "2567e8fbc746714d9e316044665a2501670b77946f55e66b63938b99b17c8936",
        "wat_sha256": "47a2d51cc83f365c3099c82712c57d422968535750eb91f078868c204ebf0f9f"
      },
      {
        "fixture": "uncaught_throw",
        "wasm_sha256": "365142a10cf7d4a6da89ca87f404d5c21e4bb442c5daf0d5053a5ce81232837e",
        "wat_sha256": "e89d6eca82d1d903fcac6dd8bda281f6ec837f4c268513c416b979528a674fd8"
      },
      {
        "fixture": "continuation_root",
        "wasm_sha256": "f3a432da4b984b715e74e26016d593a61e5dd4b24e421d1434bf63bf83c39f31",
        "wat_sha256": "05d3278c9be02485919784bae44f7b5fcd55a63856296b0fb1e4a2c854379448"
      },
      {
        "fixture": "descriptor_consumer",
        "wasm_sha256": "991fabf92a687b0557bd6e7b0959c500dca3bd4549fc3ccce1ad517cf44438ca",
        "wat_sha256": "f80a41c42745545c0007917baeb5b905f3085a49b507cb8e9f4331fec1607118"
      },
      {
        "fixture": "descriptor_graph",
        "wasm_sha256": "474d6c1d97ac8548a942098293451ac53e66561e199e95dcfa91bfd72ed9d443",
        "wat_sha256": "f7bf5ef8fb302e705e232586f720e9d864e5431be67b29590ea9b7f98b9b1a93"
      },
      {
        "fixture": "descriptor_provider",
        "wasm_sha256": "a3f7000c6231b8acf83b56a22e08b6f4fceff4dc7930712f16c7b734d6d9cbdb",
        "wat_sha256": "3197bba5170209b278b8a436d09923fd383255d10b51fe0014383f5099dec9d3"
      },
      {
        "fixture": "host_reentry_collect",
        "wasm_sha256": "63852bfe7dd4659db0cabbd8a01fe4249734ea4a719c8a0dcb5396e1c8a7c8e4",
        "wat_sha256": "91d9a3ba857c39ace65f4a9e1e3ff91cdfe96ca0b52ad45539b63a61534ae7cd"
      },
      {
        "fixture": "nominal_exact_casts",
        "wasm_sha256": "6fc8a2cac95a90fe0d1a3d7c1cf1a9f57041df41f6ec8ccae747c1368b9f03e2",
        "wat_sha256": "278b748822210879d166c0131c10b88ac32b38bf8437c1ada05667e1d8156ada"
      },
      {
        "fixture": "packed_array",
        "wasm_sha256": "fbfb37283a67bee4e8a6a769a8491e68115758c321885cd776cbb4ce9bbceec1",
        "wat_sha256": "7580ef1c80342937d900b7acac0bde6e4cddf21fc03de15b9da0d7c91dad6a8d"
      },
      {
        "fixture": "root_lifecycle",
        "wasm_sha256": "7210514252b38d64d46fb0d7452e91aa838ae87077922e9277aa81b65bd9fa5f",
        "wat_sha256": "79f4ac8511123ba14bdde5a31cd1953bae1c69f1dcba9ec5878785e1db447a51"
      },
      {
        "fixture": "struct_graph",
        "wasm_sha256": "641b2835a88599370dfb09c90dca596caded5d22deb98f4300435b4900aced0d",
        "wat_sha256": "9fe9e8f7c7c9cd995d063bf4a54eb8d4aab61eb526f101b8c50126c50ddbc8df"
      },
      {
        "fixture": "typed_function_refs",
        "wasm_sha256": "c07a43c30d251223ebd9769ef3742eb32ec5d3ef751d6a9f0e1507ad353b4cb7",
        "wat_sha256": "5d62ffda642de10f8ddbfa828512e8bc7d36d3cbcd70ce2e52f5532c030d990b"
      }
    ],
    "live_candidate_binary": {
      "current_sha256": "e61f3766d7dc6642810102c35a7c55ab7e284d4ec9b6e20f45462e8f3efbbaad",
      "matches_report": true,
      "path": "/workspace/wtime/tools/wtime-bench/target/release/wtime-bench",
      "size_bytes": 16811304
    },
    "model_runner_registry_source_hashes": {
      "all_match_current": true,
      "model": {
        "path": "/workspace/wtime/tools/wtime-bench/gc_workloads.py",
        "sha256": "aedd39f75c5c407690251ccd40b8a00139d9a6b9b41f3a1b9980df90b37f543c"
      },
      "registry": {
        "path": "/workspace/wtime/tools/wtime-bench/workloads.json",
        "sha256": "75a4ed312d698130de0b10c4a05dd7b32085e7689e4ac6ebd4bca6977dac4025"
      },
      "runner": {
        "path": "/workspace/wtime/tools/wtime-bench/gc_scaling.py",
        "sha256": "58a6f5419f016a4f1f35512d8ae713041620449bd39d48a98685b251f5701f7e"
      },
      "workload_schema": {
        "path": "/workspace/wtime/tools/wtime-bench/workloads.py",
        "sha256": "99b4503219120e06630234ddcea52a59d4c5caea4f05c76894cff0486bde918a"
      }
    },
    "report": {
      "path": "/workspace/wtime/target/performance/P011/report.json",
      "sha256": "c22ea1937a43f69bf0ce774e05d9d1ac0d65c3ddd0d1837a38d2eb5a245a8272",
      "size_bytes": 643329
    },
    "retained_candidate_binary": {
      "matches_report": true,
      "path": "/workspace/wtime/target/performance/P011/binary/wtime-bench",
      "sha256": "e61f3766d7dc6642810102c35a7c55ab7e284d4ec9b6e20f45462e8f3efbbaad",
      "size_bytes": 16811304
    },
    "runtime_and_benchmark_sources": {
      "all_match_current": true,
      "canonical_sha256": "ba2dcf43500ec9f7f600b0dad4ee6516efeb486254d6f950d8f2b2650e9e3e79",
      "file_count": 66
    },
    "source_snapshot": {
      "all_match_current": false,
      "file_count": 104,
      "mismatches": [
        {
          "current_sha256": "5123b38fd1e6b2358027778d94256202e53ff7024b3b05cd94aa5ea50d7949d4",
          "path": "/workspace/wtime/target/debug/wtime-conformance",
          "recorded_sha256": "6616cabefdaf1cb99514a43fc27b63a039210727ac0ee22e2dab3cfb2f5b4ce7"
        }
      ]
    }
  },
  "commands": [
    {
      "command": "cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml --features runtime-only -j 4",
      "exit_code": 0
    },
    {
      "command": "WTIME_GC_EXCEPTION_BENCH=/workspace/wtime/tools/wtime-bench/target/release/wtime-bench WTIME_CONFORMANCE_ENCODER=/workspace/wtime/target/debug/wtime-conformance python3 -m unittest discover -s tools/wtime-bench/tests -p 'test_p011*.py' -v",
      "exit_code": 0,
      "passed": 13,
      "failed": 0,
      "skipped": 0
    },
    {
      "command": "python3 tools/wtime-bench/gc_scaling.py --candidate tools/wtime-bench/target/release/wtime-bench --encoder target/debug/wtime-conformance --model tools/wtime-bench/gc_workloads.py --registry tools/wtime-bench/workloads.json --encoded-dir target/performance/P011-fixtures --source-root /workspace/wtime --output target/performance/P011/report.json",
      "exit_code": 0,
      "cases": 45,
      "observations": 225
    }
  ],
  "limitations": [
    "Wasmtime GC/exceptions unsupported in pinned comparator build; no matching timing or ratio.",
    "CollectionStats measures GC arena resources; no separate public exception-arena reclaimed counter.",
    "Historical source-bound report precedes subsequent P014 harness and conformance-tool changes."
  ]
}
```
