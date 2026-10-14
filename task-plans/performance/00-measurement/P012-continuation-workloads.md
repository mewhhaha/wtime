# P012 — Measure continuation capture, resume and accounting

Status: **done**

Outcome: **contract_only**

Kind: `foundation` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P003 — Define workload selections and engine capability preflight](P003-workload-schema.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `tools/wtime-bench/fixtures/continuations/*.wat`
- `tools/wtime-bench/src/main.rs`
- `tools/wtime-bench/workloads.json`
- `tools/wtime-bench/tests/test_p012_continuations.py`

- `tools/wtime-bench/src/continuation_driver.rs`

- `tools/wtime-bench/src/protocol.rs`

- `tools/wtime-bench/workloads.py`

- `tools/wtime-bench/continuation_scaling.py`

- `tools/wtime-bench/tests/test_p012_driver.py`

- `tools/wtime-bench/tests/test_p012_scaling.py`

- `docs/benchmarks/performance/P012-evidence.json`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

Continuation selection starts wtime-only and reports capture/resume/switch latency, logical frames/values captured and continuation_bytes; an external ratio requires a verified equivalent comparator.

## Implementation recipe

1. Build fixtures exporting source-level drivers for suspend/resume/bind/throw/switch with 1/8/64 frames and numeric/reference locals; use the current public RunOutcome path and guest resume exports.
2. Separate setup, capture and subsequent guest-driver resume timings; assert one-shot consumption, signatures/results and bytes before/after ownership transitions.
3. Include nested prompts, cross-module tag/type contexts, remaining-fuel and reentrant collection controls; do not add an assumed public resume API that the runtime does not have.
4. Record continuations as unsupported by Wasmtime unless a real configured probe executes matching semantics; collect uninstrumented wtime baselines.

## Meaningful cases

- Resume consumes a continuation once and a second use fails.
- A captured reference survives collection and final resumed result matches its oracle.
- Fuel and byte-budget boundaries remain correct for shallow/deep capture.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
python3 -m unittest discover -s tools/wtime-bench/tests -p 'test_p012*.py' -v
```

## Measurement and decision

Selection: `continuations`. Explicit execution setting: `decoded`. Decision rule: `integrity`.

This is a tooling/contract deliverable. Require correct nonzero checks and bounded/source-bound output; do not invent a runtime speedup requirement. If it changes production work, record cold/build/retention deltas under [MEASUREMENT.md](../MEASUREMENT.md).

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. Executed evidence is recorded below.

## Executed evidence

```json
{
  "artifact_hashes": {
    "target/performance/P012-default-build.log": {
      "bytes": 131,
      "sha256": "86c87daf729868cc4966b59e8dcdd508a4feae7628f7a5e76db0886207e957d9"
    },
    "target/performance/P012-default-tests.log": {
      "bytes": 2525,
      "sha256": "58e90cc638d6ac7714ace57624c50d7af6f72c8b5700fd52fd620cfecb0f88a9"
    },
    "target/performance/P012-default/binary/wtime-bench": {
      "bytes": 17189288,
      "sha256": "2d9b4e89d610fe152d18abae399bc1b8e1fad704b6bfa77270f92386a8858750"
    },
    "target/performance/P012-default/report.json": {
      "bytes": 118505,
      "sha256": "7b3cc55b9c9a660dbd13215ea53ce4e61e194914aa38dfafcbdad6476cea7f88"
    },
    "target/performance/P012-fixtures/bound_nested.wasm": {
      "bytes": 236,
      "sha256": "f7297ddc64454c2dc9ecdd0f58a3d51a05679922dd8b464f78fb0088eefe89c6"
    },
    "target/performance/P012-fixtures/captured_reference.wasm": {
      "bytes": 155,
      "sha256": "5b2e701d1eaede5060f35b08fbfec8fae7ad28851676f2d92535c55e4e5bbf1c"
    },
    "target/performance/P012-fixtures/cross_module_consumer.wasm": {
      "bytes": 187,
      "sha256": "85d6081efdbf66f3b825d6847fd0831e25c0be110e51feccca99b56595adb2ab"
    },
    "target/performance/P012-fixtures/cross_module_provider.wasm": {
      "bytes": 66,
      "sha256": "d6d7298a08c1888fe15c44b430187f10b5b8734ca04dc25b627b2149860c2290"
    },
    "target/performance/P012-fixtures/depth_0.wasm": {
      "bytes": 150,
      "sha256": "2e30292dab3ac305fd029a6b8ff92d5af14f96900b40d5053317c75e8ba589a3"
    },
    "target/performance/P012-fixtures/depth_32.wasm": {
      "bytes": 1318,
      "sha256": "fcc256cb133113e2a21aebab749bd5e0e5a14ebcb514492f740facffca1d1189"
    },
    "target/performance/P012-fixtures/depth_4.wasm": {
      "bytes": 291,
      "sha256": "880e23776c57398a7fa046424de5c8a9943d1af61935f69f85fc6b33ed7b5d17"
    },
    "target/performance/P012-fixtures/one_shot.wasm": {
      "bytes": 245,
      "sha256": "b17fb545ed18f933b6a19ca53876ef334a5d925b6511f7b51b84f0f55f9164c3"
    },
    "tools/wtime-bench/fixtures/continuations/bound_nested.wat": {
      "bytes": 982,
      "sha256": "5bb9f6ec7c849fd71bb91e7e9006ad6b954ab723a724442539bdb956c1c66f70"
    },
    "tools/wtime-bench/fixtures/continuations/captured_reference.wat": {
      "bytes": 515,
      "sha256": "f04c26de19fe09fce7bfb0182559bd2976de582b891712d304dc4bc760c6f7d1"
    },
    "tools/wtime-bench/fixtures/continuations/cross_module_consumer.wat": {
      "bytes": 565,
      "sha256": "6c9a7c8e869e9f3c1d931b63251032bae765c20c78d06f11aa5ff9e1fb24256d"
    },
    "tools/wtime-bench/fixtures/continuations/cross_module_provider.wat": {
      "bytes": 132,
      "sha256": "4e69dd91d7bc14d7e8ba630138ab2ae4c58accdf9de0f284bcfdfcda13b6e651"
    },
    "tools/wtime-bench/fixtures/continuations/depth_0.wat": {
      "bytes": 456,
      "sha256": "c4d1378e50d36ff9ec82b4f78a0f83783664c002c31e27ccfb751da5d97451ac"
    },
    "tools/wtime-bench/fixtures/continuations/depth_32.wat": {
      "bytes": 5165,
      "sha256": "8fc4cd5704aac0b9ca91a13ff8ac87957cd30406b43994fa5eadab23d00ba226"
    },
    "tools/wtime-bench/fixtures/continuations/depth_4.wat": {
      "bytes": 1028,
      "sha256": "05cd6aec67fe1eee5466a1bb39cdf7bc1d701f2fbe41b88620eb62ce06c7d554"
    },
    "tools/wtime-bench/fixtures/continuations/one_shot.wat": {
      "bytes": 885,
      "sha256": "1d33035426adde6a81cb71f9fe3457e869bfd810c68c728e5868b3196daf9ae9"
    }
  },
  "artifacts": [
    "target/performance/P012-default/report.json",
    "target/performance/P012-default/binary/wtime-bench",
    "target/performance/P012-default-build.log",
    "target/performance/P012-default-tests.log",
    "tools/wtime-bench/fixtures/continuations/bound_nested.wat",
    "target/performance/P012-fixtures/bound_nested.wasm",
    "tools/wtime-bench/fixtures/continuations/captured_reference.wat",
    "target/performance/P012-fixtures/captured_reference.wasm",
    "tools/wtime-bench/fixtures/continuations/cross_module_consumer.wat",
    "target/performance/P012-fixtures/cross_module_consumer.wasm",
    "tools/wtime-bench/fixtures/continuations/cross_module_provider.wat",
    "target/performance/P012-fixtures/cross_module_provider.wasm",
    "tools/wtime-bench/fixtures/continuations/depth_0.wat",
    "target/performance/P012-fixtures/depth_0.wasm",
    "tools/wtime-bench/fixtures/continuations/depth_32.wat",
    "target/performance/P012-fixtures/depth_32.wasm",
    "tools/wtime-bench/fixtures/continuations/depth_4.wat",
    "target/performance/P012-fixtures/depth_4.wasm",
    "tools/wtime-bench/fixtures/continuations/one_shot.wat",
    "target/performance/P012-fixtures/one_shot.wasm"
  ],
  "commands": [
    ". /workspace/.setup/rust-env.sh && cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml --features runtime-only -j4",
    "target/performance/P012-default/binary/wtime-bench --protocol",
    "python3 tools/wtime-bench/continuation_scaling.py --candidate tools/wtime-bench/target/release/wtime-bench --encoder target/debug/wtime-conformance --model tools/wtime-bench/continuation_workloads.py --registry tools/wtime-bench/workloads.json --encoded-dir target/performance/P012-fixtures --source-root /workspace/wtime --output target/performance/P012-default/report.json",
    "WTIME_CONTINUATION_BENCH=/workspace/wtime/tools/wtime-bench/target/release/wtime-bench WTIME_CONFORMANCE_ENCODER=/workspace/wtime/target/debug/wtime-conformance python3 -m unittest discover -s tools/wtime-bench/tests -p 'test_p012*.py' -v"
  ],
  "completion_record": {
    "artifacts": [
      {
        "bytes": 118505,
        "path": "target/performance/P012-default/report.json",
        "sha256": "7b3cc55b9c9a660dbd13215ea53ce4e61e194914aa38dfafcbdad6476cea7f88"
      },
      {
        "bytes": 17189288,
        "path": "target/performance/P012-default/binary/wtime-bench",
        "sha256": "2d9b4e89d610fe152d18abae399bc1b8e1fad704b6bfa77270f92386a8858750"
      },
      {
        "bytes": 131,
        "path": "target/performance/P012-default-build.log",
        "sha256": "86c87daf729868cc4966b59e8dcdd508a4feae7628f7a5e76db0886207e957d9"
      },
      {
        "bytes": 2525,
        "path": "target/performance/P012-default-tests.log",
        "sha256": "58e90cc638d6ac7714ace57624c50d7af6f72c8b5700fd52fd620cfecb0f88a9"
      },
      {
        "bytes": 982,
        "path": "tools/wtime-bench/fixtures/continuations/bound_nested.wat",
        "sha256": "5bb9f6ec7c849fd71bb91e7e9006ad6b954ab723a724442539bdb956c1c66f70"
      },
      {
        "bytes": 236,
        "path": "target/performance/P012-fixtures/bound_nested.wasm",
        "sha256": "f7297ddc64454c2dc9ecdd0f58a3d51a05679922dd8b464f78fb0088eefe89c6"
      },
      {
        "bytes": 515,
        "path": "tools/wtime-bench/fixtures/continuations/captured_reference.wat",
        "sha256": "f04c26de19fe09fce7bfb0182559bd2976de582b891712d304dc4bc760c6f7d1"
      },
      {
        "bytes": 155,
        "path": "target/performance/P012-fixtures/captured_reference.wasm",
        "sha256": "5b2e701d1eaede5060f35b08fbfec8fae7ad28851676f2d92535c55e4e5bbf1c"
      },
      {
        "bytes": 565,
        "path": "tools/wtime-bench/fixtures/continuations/cross_module_consumer.wat",
        "sha256": "6c9a7c8e869e9f3c1d931b63251032bae765c20c78d06f11aa5ff9e1fb24256d"
      },
      {
        "bytes": 187,
        "path": "target/performance/P012-fixtures/cross_module_consumer.wasm",
        "sha256": "85d6081efdbf66f3b825d6847fd0831e25c0be110e51feccca99b56595adb2ab"
      },
      {
        "bytes": 132,
        "path": "tools/wtime-bench/fixtures/continuations/cross_module_provider.wat",
        "sha256": "4e69dd91d7bc14d7e8ba630138ab2ae4c58accdf9de0f284bcfdfcda13b6e651"
      },
      {
        "bytes": 66,
        "path": "target/performance/P012-fixtures/cross_module_provider.wasm",
        "sha256": "d6d7298a08c1888fe15c44b430187f10b5b8734ca04dc25b627b2149860c2290"
      },
      {
        "bytes": 456,
        "path": "tools/wtime-bench/fixtures/continuations/depth_0.wat",
        "sha256": "c4d1378e50d36ff9ec82b4f78a0f83783664c002c31e27ccfb751da5d97451ac"
      },
      {
        "bytes": 150,
        "path": "target/performance/P012-fixtures/depth_0.wasm",
        "sha256": "2e30292dab3ac305fd029a6b8ff92d5af14f96900b40d5053317c75e8ba589a3"
      },
      {
        "bytes": 5165,
        "path": "tools/wtime-bench/fixtures/continuations/depth_32.wat",
        "sha256": "8fc4cd5704aac0b9ca91a13ff8ac87957cd30406b43994fa5eadab23d00ba226"
      },
      {
        "bytes": 1318,
        "path": "target/performance/P012-fixtures/depth_32.wasm",
        "sha256": "fcc256cb133113e2a21aebab749bd5e0e5a14ebcb514492f740facffca1d1189"
      },
      {
        "bytes": 1028,
        "path": "tools/wtime-bench/fixtures/continuations/depth_4.wat",
        "sha256": "05cd6aec67fe1eee5466a1bb39cdf7bc1d701f2fbe41b88620eb62ce06c7d554"
      },
      {
        "bytes": 291,
        "path": "target/performance/P012-fixtures/depth_4.wasm",
        "sha256": "880e23776c57398a7fa046424de5c8a9943d1af61935f69f85fc6b33ed7b5d17"
      },
      {
        "bytes": 885,
        "path": "tools/wtime-bench/fixtures/continuations/one_shot.wat",
        "sha256": "1d33035426adde6a81cb71f9fe3457e869bfd810c68c728e5868b3196daf9ae9"
      },
      {
        "bytes": 245,
        "path": "target/performance/P012-fixtures/one_shot.wasm",
        "sha256": "b17fb545ed18f933b6a19ca53876ef334a5d925b6511f7b51b84f0f55f9164c3"
      }
    ],
    "binary_matches_report": true,
    "build": {
      "duration_seconds_from_log": 9.57,
      "features": "runtime-only (no execution-reference)",
      "log": {
        "bytes": 131,
        "path": "target/performance/P012-default-build.log",
        "sha256": "86c87daf729868cc4966b59e8dcdd508a4feae7628f7a5e76db0886207e957d9"
      },
      "note": "Final build log reports 9.57s and completed successfully."
    },
    "commands": {
      "build": ". /workspace/.setup/rust-env.sh && cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml --features runtime-only -j4",
      "protocol": "target/performance/P012-default/binary/wtime-bench --protocol",
      "runner": "python3 tools/wtime-bench/continuation_scaling.py --candidate tools/wtime-bench/target/release/wtime-bench --encoder target/debug/wtime-conformance --model tools/wtime-bench/continuation_workloads.py --registry tools/wtime-bench/workloads.json --encoded-dir target/performance/P012-fixtures --source-root /workspace/wtime --output target/performance/P012-default/report.json",
      "tests": "WTIME_CONTINUATION_BENCH=/workspace/wtime/tools/wtime-bench/target/release/wtime-bench WTIME_CONFORMANCE_ENCODER=/workspace/wtime/target/debug/wtime-conformance python3 -m unittest discover -s tools/wtime-bench/tests -p 'test_p012*.py' -v"
    },
    "configuration": {
      "boundary_probes": 3,
      "fixture_modules": 8,
      "instrumented": false,
      "mode": "decoded",
      "observations": 35,
      "profile": "all-phase3-core",
      "runnable_cases": 7,
      "runtime_features": [
        "runtime-only"
      ],
      "samples_per_case": 5,
      "setup_only_provider": "continuation_cross_module_provider",
      "wasmtime": {
        "reason": "The configured comparator has no equivalent stack-switching support result.",
        "status": "unsupported",
        "timing": "not_measured"
      }
    },
    "default_binary": {
      "bytes": 17189288,
      "path": "target/performance/P012-default/binary/wtime-bench",
      "sha256": "2d9b4e89d610fe152d18abae399bc1b8e1fad704b6bfa77270f92386a8858750"
    },
    "default_report": {
      "bytes": 118505,
      "path": "target/performance/P012-default/report.json",
      "sha256": "7b3cc55b9c9a660dbd13215ea53ce4e61e194914aa38dfafcbdad6476cea7f88"
    },
    "historical_development_reference": {
      "binary": {
        "bytes": 16995560,
        "path": "target/performance/P012-final/binary/wtime-bench",
        "sha256": "c60cbc6f15d0d2ee45bb82f814bd63c50fa58157181ed2bb7c001ec99f56fa67"
      },
      "build_features": [
        "runtime-only",
        "execution-reference"
      ],
      "classification": "Historical development/integrity evidence only. It is not valid default timing evidence because execution-reference enables reference hooks even though the old driver rows recorded instrumented=false.",
      "report": {
        "bytes": 118265,
        "path": "target/performance/P012-final/report.json",
        "sha256": "85573e1fc24e5e726debb77de0f2aed3411d835310a0b6971e00922a8878bf8d"
      }
    },
    "limits": [
      "Default timing evidence is the P012-default report built with runtime-only only and protocol-confirmed instrumented=false.",
      "The configured Wasmtime comparator has no equivalent stack-switching support, so it is unsupported and no timing was measured.",
      "These generated continuation workloads validate specific continuation semantics/accounting; they do not establish representative application performance or a speedup.",
      "There are 8 module fixtures including a provider module used for setup only; the 7 runnable cases each have 5 validated observations.",
      "P013 subsequently changed model/schema/registry metadata, so this P012 report is an immutable historical run with measurement-time source hashes."
    ],
    "source_stable": true,
    "tests": {
      "failed": 0,
      "log": {
        "bytes": 2525,
        "path": "target/performance/P012-default-tests.log",
        "sha256": "58e90cc638d6ac7714ace57624c50d7af6f72c8b5700fd52fd620cfecb0f88a9"
      },
      "passed": 14,
      "skipped": 0
    }
  },
  "outcome": "contract_only",
  "status": "done",
  "task": "P012"
}
```
