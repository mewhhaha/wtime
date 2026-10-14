# P010 — Add SIMD and strict floating-point benchmarks

Status: **done**

Outcome: **contract_only**

Kind: `foundation` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P003 — Define workload selections and engine capability preflight](P003-workload-schema.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `tools/wtime-bench/fixtures/simd/*.wat`
- `tools/wtime-bench/simd_workloads.py`
- `tools/wtime-bench/workloads.json`
- `tools/wtime-bench/tests/test_p010_simd.py`
- `tools/wtime-bench/simd_scaling.py`
- `tools/wtime-bench/src/protocol.rs`
- `tools/wtime-bench/tests/test_p010_scaling.py`
- `tools/wtime-bench/tests/test_p010_driver.py`

- `tools/wtime-bench/tests/test_p010_registry.py`

- `docs/benchmarks/performance/P010-evidence.json`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

SIMD selection executes vector instructions in loops and returns checked bit/checksum results; strict floating cases use bitwise or permitted-NaN oracles rather than approximate equality.

## Implementation recipe

1. Add integer lane add/mul/saturation, bitwise/select, shuffle/swizzle, lane memory and reduction kernels with scalar-array Python reference implementations.
2. Add f32/f64 and vector float arithmetic, signed-zero min/max, NaN and conversion boundary correctness controls; record allowed result sets for relaxed operations.
3. Vary loop counts and memory sizes, gate Wasmtime SIMD/relaxed support through execution probes, and keep scalar/vector work equivalence explicit.
4. Report lane operations/bytes and separate pure compute from memory kernels; record baselines without native intrinsics or fast-math.

## Meaningful cases

- Randomized lane vectors match independent wrapping/saturating oracle.
- Signed zero and NaN controls pass allowed bit rules.
- The compiled fixture contains the required SIMD instructions and comparator settings enable them.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
python3 -m unittest discover -s tools/wtime-bench/tests -p 'test_p010*.py' -v
```

## Measurement and decision

Selection: `simd`. Explicit execution setting: `decoded`. Decision rule: `integrity`.

This is a tooling/contract deliverable. Require correct nonzero checks and bounded/source-bound output; do not invent a runtime speedup requirement. If it changes production work, record cold/build/retention deltas under [MEASUREMENT.md](../MEASUREMENT.md).

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. Executed evidence is recorded below.

## Executed evidence

```json
{
  "artifacts": [
    "/workspace/wtime/target/performance/P010-final/report.json",
    "/workspace/wtime/target/performance/P010/binary/wtime-bench",
    "/workspace/wtime/target/performance/P010-tests-ready.log",
    "/workspace/wtime/docs/benchmarks/performance/P010-evidence.json"
  ],
  "capabilities": {
    "all_compile_execute_checked": true,
    "candidate_protocol": {
      "engine": "wtime",
      "features": [
        "mvp",
        "stable-core",
        "all-phase3-core",
        "mvp-saturating",
        "simd",
        "simd-relaxed"
      ],
      "instrumented": false,
      "modes": [
        "decoded",
        "portable"
      ],
      "schema_version": 2,
      "types": [
        "i32",
        "i64",
        "f32_bits",
        "f64_bits",
        "v128_bits",
        "reference"
      ]
    },
    "capability_counts": {
      "wasmtime-49.0.2:supported:compile=True": 114
    },
    "case_level_wasmtime_49_0_2_probes": 114
  },
  "measurement": {
    "candidate_observations": 2850,
    "cases": 114,
    "exact_reproduction_command_from_report_metadata": "cd /workspace/wtime && python3 tools/wtime-bench/simd_scaling.py --candidate /workspace/wtime/target/performance/P010/binary/wtime-bench --wasmtime /workspace/wtime/target/performance/P010/binary/wtime-bench --model /workspace/wtime/tools/wtime-bench/simd_workloads.py --registry /workspace/wtime/tools/wtime-bench/workloads.json --encoded-dir /workspace/wtime/target/performance/P010-fixtures --source-root /workspace/wtime --encoder /workspace/wtime/target/debug/wtime-conformance --selection simd --samples 5 --seed 20496 --output /workspace/wtime/target/performance/P010-final/report.json",
    "instrumented": false,
    "mode": "decoded",
    "observations_by_stage": {
      "first_call": 570,
      "instantiate": 570,
      "load_and_call": 570,
      "prepare": 570,
      "warm_call": 570
    },
    "passed": true,
    "profile_counts": {
      "mvp": 42,
      "mvp-saturating": 11,
      "simd": 57,
      "simd-relaxed": 4
    },
    "report": {
      "path": "/workspace/wtime/target/performance/P010-final/report.json",
      "sha256": "37b4965b7056800be7b98b0062bd3a246851a83f3268218bf3a18c82c9c7a049",
      "size_bytes": 951218
    },
    "samples_per_stage": 5,
    "seed": 20496,
    "selection": "simd",
    "source_stable_at_measurement": true,
    "stages": [
      "prepare",
      "instantiate",
      "first_call",
      "warm_call",
      "load_and_call"
    ],
    "task": "P010",
    "unique_fixtures": 20,
    "wasmtime_timing_comparison": "none; only capability checks were performed"
  },
  "result": "passed; historical measurement source snapshot stable; 29 current tests passed",
  "schema": "wtime-p010-evidence-v2",
  "source_identity": {
    "all_fixture_sources_and_bytes_still_match_report": true,
    "measurement_snapshot_count": 111,
    "measurement_snapshot_current_match_count": 109,
    "post_measurement_changes": [
      {
        "current_sha256": "2f489f55331b042aa5a8c5b6652d09c36f75053b697ac5703b7300487caea44f",
        "path": "/workspace/wtime/tools/wtime-bench/src/protocol.rs",
        "recorded_sha256": "b7a9e62658fa6e78d400ade48252dd2fd45795eaf3d5afa8ce6fe8daf5f8695e"
      },
      {
        "current_sha256": "0de7a9981645757602bb47e754262640191dfcfe5c78d1e029075f5ba0d25acb",
        "path": "/workspace/wtime/tools/wtime-bench/workloads.json",
        "recorded_sha256": "032858f01f2477c975b86464286300c5d05e50833a73111daae2d9cfd69ecc80"
      }
    ],
    "post_measurement_changes_context": "P011 integration occurred after P010 completion and changed protocol.rs plus workloads.json; P010 report retains its original source_snapshot and source_stable result.",
    "restored_build": {
      "all_202_recorded_inputs_match_current": true,
      "report": {
        "path": "/workspace/wtime/target/performance/P023/restored-build.json",
        "sha256": "cc1aa457a4c527ce2c7b130867157679f8e69f382c94e8a3b25e75cf1948b1f7",
        "size_bytes": 71200
      },
      "scope_note": "This restored-build inventory is separate from P010 benchmark/harness metadata; it does not erase the later protocol.rs and workloads.json changes.",
      "source_count": 202,
      "source_sha256": "bffd2c459a14b64880f0489721cbcb31fe13ed15f2c012c5d9c2bfe233d37302",
      "source_sha256_after": "bffd2c459a14b64880f0489721cbcb31fe13ed15f2c012c5d9c2bfe233d37302",
      "source_stable": true
    },
    "runner_sha256_current": "825c68bfe2136a3d7937c0873fbdf39b9ed84fdca3364f0d050d57e89b959a8b",
    "workloads_schema_validator_sha256_current": "99b4503219120e06630234ddcea52a59d4c5caea4f05c76894cff0486bde918a"
  },
  "task": "P010",
  "tests": {
    "command": "cd /workspace/wtime && WTIME_BENCH_SIMD_DRIVER=/workspace/wtime/target/performance/P010/binary/wtime-bench WTIME_CONFORMANCE_ENCODER=/workspace/wtime/target/debug/wtime-conformance python3 -m unittest discover -s tools/wtime-bench/tests -p 'test_p010*.py' -v",
    "log": {
      "path": "/workspace/wtime/target/performance/P010-tests-ready.log",
      "sha256": "bfe56ef61fa1ded4f09216fffda87ba3c995c7f4c817902e5dcf324467de89c1",
      "size_bytes": 4729
    },
    "passed": true,
    "seconds": 1.537,
    "skipped": 0,
    "test_files": [
      "test_p010_driver.py",
      "test_p010_registry.py",
      "test_p010_scaling.py",
      "test_p010_simd.py"
    ],
    "tests": 29
  },
  "artifact_details": {
    "candidate_binary": {
      "matches_report": true,
      "path": "/workspace/wtime/target/performance/P010/binary/wtime-bench",
      "recorded_sha256": "c6e94e4bc78ce9bf89381b9afdb6a33082fa7d8be790d27c86d6c9192d355f8a",
      "sha256": "c6e94e4bc78ce9bf89381b9afdb6a33082fa7d8be790d27c86d6c9192d355f8a",
      "size_bytes": 16743016
    },
    "fixture_encoder": {
      "matches_report": true,
      "path": "/workspace/wtime/target/debug/wtime-conformance",
      "recorded_sha256": "6616cabefdaf1cb99514a43fc27b63a039210727ac0ee22e2dab3cfb2f5b4ce7",
      "sha256": "6616cabefdaf1cb99514a43fc27b63a039210727ac0ee22e2dab3cfb2f5b4ce7",
      "size_bytes": 23921656
    },
    "fixtures": [
      {
        "fixture": "f64_arithmetic",
        "wasm_sha256": "4b8677abfcf31c65bf72fb19952629c037e4903a8596a508f32af4ce38a25643",
        "wat_sha256": "1ded010deec79fb83fbee8fd7f14512270be182574bd216c7b691cc49c3e0c3b"
      },
      {
        "fixture": "f64_minmax",
        "wasm_sha256": "69ed757b2f76f1605f3b4c9332d995887848a44406d99b929df38676c4486b77",
        "wat_sha256": "27da5ed0409e2c811187eeb54c37da2c4ed3a3592645af5b93e6908702ecdcfd"
      },
      {
        "fixture": "float_arithmetic",
        "wasm_sha256": "ece84319c9f3b540e99b4ddaf2585f778dda6cd8735d11b27f7ba35e6d5fbef0",
        "wat_sha256": "98b2b50a26ef07e2a35c811eae0398deedfce2b1f99c11c4fe2e5c6b85004372"
      },
      {
        "fixture": "float_edges",
        "wasm_sha256": "6d21067a0c105455857d2d0c46b6b142e98f0794d2f1870da0ea5f467768a506",
        "wat_sha256": "ba3bc8c3abb8f6cf8a9f75dd586d0d78d5bb7a178f67e16d6e41424bf6e6be26"
      },
      {
        "fixture": "integer_lanes",
        "wasm_sha256": "95ad86610276d9db5ca8ea3d6426d087fca8a702225606139e79673de60003a7",
        "wat_sha256": "fd4846ae1a22a31b17f5dc344bb20c015388d675991c9ac3a7c809ef83e7ad44"
      },
      {
        "fixture": "integer_lanes_typed",
        "wasm_sha256": "3a690996697e34089e18a5e9883ce551f6d600dece89e70580fd601ae2a82150",
        "wat_sha256": "19abdb4ac058a1b3fec2be1c463975e87ab5f29f124a72e39d1ce409ddb4eccb"
      },
      {
        "fixture": "lane_memory",
        "wasm_sha256": "9c957181e2ca5d74e9a3f9fec4e4cf5fb22e3eea231ccbc4c826c5c8847bfa01",
        "wat_sha256": "4f35680b9f3cb94cba0296cccb0cf0a8f96588ea637bea83cd83a6caac059141"
      },
      {
        "fixture": "lane_memory_footprint",
        "wasm_sha256": "a92144289dae9ec957773fe0f5bfef50c2ef4d188f96338aff50a4cb55af5c2c",
        "wat_sha256": "8a713b01f558578da5179920488b9ec580de5681341d8a874d2a6c58e240ae4e"
      },
      {
        "fixture": "relaxed_swizzle",
        "wasm_sha256": "2017b5e0e15c1dd4406fc76832f92bcab1ea1f32187b1ca0dcc4c80e7aa121da",
        "wat_sha256": "cc13f0de9250eaf780cf6082345e9c9bfc38be317a47ef1ce306d994624dc6f5"
      },
      {
        "fixture": "relaxed_swizzle_oob",
        "wasm_sha256": "0f79b71c6396e4e7e9c3f60828edfd1e8b185d83e65b2a1466292d29bb16dc3b",
        "wat_sha256": "af53075fe822615e699161579301dd484114bdac67c4fa41621ac8b58ce6861c"
      },
      {
        "fixture": "saturating_select",
        "wasm_sha256": "22633c8544d3db06b0f47cef19966e357743e0db9a84815fc66d87b390a87bea",
        "wat_sha256": "a4edecbc5fa5ace4c9d4d69b29be6b3131f1500b6cae84d8ebafe813454f70c6"
      },
      {
        "fixture": "scalar_f64_arithmetic",
        "wasm_sha256": "6f5956a62e16a5298cb1297d0691bfb26e50bc92243198413748b82580d2f5ac",
        "wat_sha256": "d5bd87cd9029c7050048fea8b3932be7a0531d68cc1ab91b3578d059e7a8667a"
      },
      {
        "fixture": "scalar_float_arithmetic",
        "wasm_sha256": "9a866ed614814bc74a36aad06d6adefc51cd9bf914c8cd7901ee19a13649ce92",
        "wat_sha256": "a11d4969aaa9fb4066b72d0123f905bfa2e2e708f2f178234a0252a7bd727bab"
      },
      {
        "fixture": "scalar_float_edges",
        "wasm_sha256": "621b5c278b5a3c706996d06a19cf12578c57dbd92e7015d57e4e8a384018405c",
        "wat_sha256": "5bbb77c7bb55bb2f436b9fe97f4e9f6c4a512473d9943024981489aeaa2386ba"
      },
      {
        "fixture": "scalar_integer_lanes",
        "wasm_sha256": "71f6ccd9b822a455d0248276eacdf61a61f39a2435000c0d3f12a9d3e3f5a15c",
        "wat_sha256": "60fa829c372957e8aa2533153ceff2977361ec361eb01b38d72415083caba53e"
      },
      {
        "fixture": "scalar_integer_lanes_typed",
        "wasm_sha256": "45101f560f2675ae21c5b80b7e89974a3e0bff7814735bbf4609999ec2c44e19",
        "wat_sha256": "6b90a27d798fb8ac6254a8e8c19dce1be396600447c65c9cd3baad1c38799156"
      },
      {
        "fixture": "scalar_shuffle",
        "wasm_sha256": "f7a3e0e209078c5afac21d24285412d7206921427e73fae451e90fa2df8c485a",
        "wat_sha256": "485df07d8a162fcbcbcc537ce0c067124629eb67946ad1e432de41d9c1beef23"
      },
      {
        "fixture": "shuffle",
        "wasm_sha256": "b71c56264d750cc8f7300e3b9900534072d92838447a2f600c920e20625f5f77",
        "wat_sha256": "ce2af411702310c035dc1a3133feaa096b40fe43def12806e1bac0915dfe877a"
      },
      {
        "fixture": "typed_f64",
        "wasm_sha256": "8b91d640241294a1dee9cfbe8605287a0b4e3d5e215d0c9aea529f77ec6da841",
        "wat_sha256": "486ba5030a6eebb0c08bc2dd530b2492c916c0ecc07614ba92479541517e8e9f"
      },
      {
        "fixture": "typed_v128",
        "wasm_sha256": "12f3541a902e0ea324af36e547d6af9ea2879720ccefb7e4fdb8b37b024b1a12",
        "wat_sha256": "faea7f37b608c510fbba7a2fc6bcb1491df12b6741421f8636c0a637353787d2"
      }
    ],
    "model_sha256": "c43645c67deb4efc566602800d50913a25d434c3146df5ee686fc4bb7b555f2b",
    "registry_sha256_at_measurement": "032858f01f2477c975b86464286300c5d05e50833a73111daae2d9cfd69ecc80",
    "registry_sha256_current_after_P011_integration": "0de7a9981645757602bb47e754262640191dfcfe5c78d1e029075f5ba0d25acb",
    "report_run_log": {
      "path": "/workspace/wtime/target/performance/P010-final-run.log",
      "sha256": "8c94a62c822f7466755bbc5de10521194b1f2422f8a085683e06e56c82a40b27",
      "size_bytes": 93
    },
    "wasmtime_capability_binary": {
      "matches_report": true,
      "path": "/workspace/wtime/target/performance/P010/binary/wtime-bench",
      "recorded_sha256": "c6e94e4bc78ce9bf89381b9afdb6a33082fa7d8be790d27c86d6c9192d355f8a",
      "role": "capability harness only; not a separate Wasmtime timing run",
      "sha256": "c6e94e4bc78ce9bf89381b9afdb6a33082fa7d8be790d27c86d6c9192d355f8a",
      "size_bytes": 16743016
    }
  },
  "commands": [
    {
      "command": "WTIME_BENCH_SIMD_DRIVER=/workspace/wtime/target/performance/P010/binary/wtime-bench WTIME_CONFORMANCE_ENCODER=/workspace/wtime/target/debug/wtime-conformance python3 -m unittest discover -s tools/wtime-bench/tests -p 'test_p010*.py' -v",
      "exit_code": 0,
      "passed": 29,
      "failed": 0,
      "skipped": 0
    },
    {
      "command": "python3 tools/wtime-bench/simd_scaling.py --candidate target/performance/P010/binary/wtime-bench --wasmtime target/performance/P010/binary/wtime-bench --model tools/wtime-bench/simd_workloads.py --registry tools/wtime-bench/workloads.json --encoded-dir target/performance/P010-fixtures --encoder target/debug/wtime-conformance --source-root /workspace/wtime --output target/performance/P010-final/report.json",
      "exit_code": 0,
      "cases": 114,
      "observations": 2850
    }
  ],
  "limitations": [
    "Wasmtime capability probes only; no Wasmtime timings or speedup claimed.",
    "Historical source-bound measurement precedes later P011 harness integration."
  ]
}
```
