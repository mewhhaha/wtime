# P013 — Measure shared memory, contention and wait latency

Status: **done**

Outcome: **contract_only**

Kind: `foundation` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P003 — Define workload selections and engine capability preflight](P003-workload-schema.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `tools/wtime-bench/fixtures/threads/*.wat`
- `tools/wtime-bench/src/main.rs`
- `tools/wtime-bench/workloads.json`
- `tools/wtime-bench/tests/test_p013_threads.py`

- `tools/wtime-bench/src/thread_driver.rs`

- `tools/wtime-bench/src/protocol.rs`

- `tools/wtime-bench/thread_scaling.py`

- `tools/wtime-bench/thread_workloads.py`

- `tools/wtime-bench/workloads.py`

- `tools/wtime-bench/tests/test_p013_driver.py`

- `tools/wtime-bench/tests/test_p013_scaling.py`

- `docs/benchmarks/performance/P013-evidence.json`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

Thread selection specifies 1/2/4 workers capped by quota, common shared backing, barriers, operation counts and start/teardown boundaries; atomic throughput and individual wait/notify latency are separate reports.

## Implementation recipe

1. Use the existing shared-memory/host-worker API and one shared backing per trial; add ordinary read/write, atomic load/store/RMW and same-vs-distinct-memory contention kernels.
2. Start workers behind a barrier, exclude setup only in a labeled steady-state stage, validate final atomic counts and report wall time/operations rather than summing overlapping worker times.
3. Collect actual individual wait/notify observations with bounded timeout/cancellation and recorded registration behavior; do not fabricate p99 from batched means.
4. Probe comparator support and equivalent synchronization; run independent seeded trials and retain scheduler/SC/wait correctness caveats.

## Meaningful cases

- Final RMW count equals workers*iterations and no worker is lost.
- Timeout/cancellation cleans wait registration and notify count is exact.
- Distinct-memory SC store-buffering tests forbid the prohibited result.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
python3 -m unittest discover -s tools/wtime-bench/tests -p 'test_p013*.py' -v
```

## Measurement and decision

Selection: `threads`. Explicit execution setting: `decoded`. Decision rule: `integrity`.

This is a tooling/contract deliverable. Require correct nonzero checks and bounded/source-bound output; do not invent a runtime speedup requirement. If it changes production work, record cold/build/retention deltas under [MEASUREMENT.md](../MEASUREMENT.md).

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. Executed evidence is recorded below.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/P013-build-final.log": {
      "bytes": 132,
      "sha256": "15f9b87b80829789f0a6b1b01abba2e0892cb8d9f0e65b5c3fc6c0c0edcc9626"
    },
    "/workspace/wtime/target/performance/P013-fixtures/atomic_add.wasm": {
      "bytes": 123,
      "sha256": "46b8bdb39ef9b69deda41fa91c18f3cc4dd7ee7f58ee20dc6f1824d22a558029"
    },
    "/workspace/wtime/target/performance/P013-fixtures/atomic_load_store.wasm": {
      "bytes": 140,
      "sha256": "85c25183d6ca23da20bbce7ae10b89acfb90499f413fa891151e558e7f4975ce"
    },
    "/workspace/wtime/target/performance/P013-fixtures/ordinary_rw.wasm": {
      "bytes": 138,
      "sha256": "2a0e01aaf458a30970c52b18855a27a508f6bac0f87dcd9c961a3246af58abdf"
    },
    "/workspace/wtime/target/performance/P013-fixtures/sc_store_buffering.wasm": {
      "bytes": 119,
      "sha256": "d28965e8c9f3a157e7e3a55f80680e1542654699f3a49016a2b3532f404c4d22"
    },
    "/workspace/wtime/target/performance/P013-fixtures/wait_notify.wasm": {
      "bytes": 185,
      "sha256": "6a307ae5db566a43b1c3a8753e361f0aef14304ed7e56144f029fa5d6c268e06"
    },
    "/workspace/wtime/target/performance/P013-run.log": {
      "bytes": 86,
      "sha256": "c8d5846b07d948f66de2e2b9a3541e3cf4976ba479c1b6ddf9d9ea93a5dc2c34"
    },
    "/workspace/wtime/target/performance/P013-tests-final2.log": {
      "bytes": 2376,
      "sha256": "3f5f84b5fff43ba489eab740ae3e4fe26883f0dab92c723776f7691d0ba54a20"
    },
    "/workspace/wtime/target/performance/P013/binary/wtime-bench": {
      "bytes": 17189288,
      "sha256": "2d9b4e89d610fe152d18abae399bc1b8e1fad704b6bfa77270f92386a8858750"
    },
    "/workspace/wtime/target/performance/P013/report.json": {
      "bytes": 392532,
      "sha256": "959d62f71b50220e74dc2940457c9a16345a3a773818a482324f2659e016ae5d"
    },
    "/workspace/wtime/tools/wtime-bench/fixtures/threads/atomic_add.wat": {
      "bytes": 390,
      "sha256": "f821deb4c8c142de5972053b9644029f7092a1be1468eb1ed10c1002d7e2776c"
    },
    "/workspace/wtime/tools/wtime-bench/fixtures/threads/atomic_load_store.wat": {
      "bytes": 498,
      "sha256": "400f2e0826eff320b781ebd216dcf45a61fcccc73e8f60f8ef0af19e9ead330d"
    },
    "/workspace/wtime/tools/wtime-bench/fixtures/threads/ordinary_rw.wat": {
      "bytes": 474,
      "sha256": "0a0d74f8b24c53299ac14d0094f13e38129326cfc421a41399e4a3ab9653f002"
    },
    "/workspace/wtime/tools/wtime-bench/fixtures/threads/sc_store_buffering.wat": {
      "bytes": 369,
      "sha256": "fc6f24e92a0904de4797731ce2642e47adb46eeac2f8a62549db67889b63a12e"
    },
    "/workspace/wtime/tools/wtime-bench/fixtures/threads/wait_notify.wat": {
      "bytes": 514,
      "sha256": "94ce5b5bd02b46b71b0677795e79beb85081b928edc154bda30f2b3daadaebcb"
    }
  },
  "artifacts": [
    "/workspace/wtime/target/performance/P013/report.json",
    "/workspace/wtime/target/performance/P013/binary/wtime-bench",
    "/workspace/wtime/target/performance/P013-tests-final2.log",
    "/workspace/wtime/target/performance/P012-default-build.log",
    "/workspace/wtime/target/performance/P013-run.log",
    "/workspace/wtime/tools/wtime-bench/fixtures/threads/atomic_add.wat",
    "/workspace/wtime/target/performance/P013-fixtures/atomic_add.wasm",
    "/workspace/wtime/tools/wtime-bench/fixtures/threads/atomic_load_store.wat",
    "/workspace/wtime/target/performance/P013-fixtures/atomic_load_store.wasm",
    "/workspace/wtime/tools/wtime-bench/fixtures/threads/ordinary_rw.wat",
    "/workspace/wtime/target/performance/P013-fixtures/ordinary_rw.wasm",
    "/workspace/wtime/tools/wtime-bench/fixtures/threads/sc_store_buffering.wat",
    "/workspace/wtime/target/performance/P013-fixtures/sc_store_buffering.wasm",
    "/workspace/wtime/tools/wtime-bench/fixtures/threads/wait_notify.wat",
    "/workspace/wtime/target/performance/P013-fixtures/wait_notify.wasm"
  ],
  "binary": {
    "bytes": 17189288,
    "path": "/workspace/wtime/target/performance/P013/binary/wtime-bench",
    "sha256": "2d9b4e89d610fe152d18abae399bc1b8e1fad704b6bfa77270f92386a8858750"
  },
  "binary_matches_report": true,
  "build": {
    "artifact": {
      "path": "/workspace/wtime/target/performance/P012-default-build.log",
      "bytes": 131,
      "sha256": "86c87daf729868cc4966b59e8dcdd508a4feae7628f7a5e76db0886207e957d9"
    },
    "duration_seconds_from_log": 9.57,
    "features": [
      "runtime-only"
    ],
    "note": "Retained binary is byte-identical to P012-default measured binary; later thread Python/model changes did not rebuild Rust. P013-build-final.log is historical and not used to bind this binary."
  },
  "commands": [
    ". /workspace/.setup/rust-env.sh && cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml --features runtime-only -j4",
    "WTIME_BENCH_THREAD_DRIVER=/workspace/wtime/tools/wtime-bench/target/release/wtime-bench WTIME_CONFORMANCE_ENCODER=/workspace/wtime/target/debug/wtime-conformance python3 -m unittest discover -s tools/wtime-bench/tests -p 'test_p013*.py' -v",
    "python3 tools/wtime-bench/thread_scaling.py --driver tools/wtime-bench/target/release/wtime-bench --driver-source tools/wtime-bench/src/thread_driver.rs --encoder target/debug/wtime-conformance --source-root /workspace/wtime --model tools/wtime-bench/thread_workloads.py --registry tools/wtime-bench/workloads.json --encoded-dir target/performance/P013-fixtures --selection threads --trials 21 --latency-samples 1024 --timeout-ns 10000000 --seed 20499 --output target/performance/P013/report.json",
    "target/performance/P013/binary/wtime-bench --protocol"
  ],
  "configuration": {
    "execution_mode": "decoded",
    "host": {
      "affinity": [
        0,
        1,
        2,
        3,
        4
      ],
      "cpu_count": 5,
      "effective_worker_quota": "reported by runtime thread::available_parallelism per case",
      "platform": "Linux-6.18.44-x86_64-with-glibc2.41",
      "python": "3.12.14"
    },
    "independent_trials_per_case": 21,
    "individual_wait_samples": 1024,
    "instrumented": false,
    "protocol_handshake": {
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
    "seed": 20499,
    "steady_state_group": {
      "setup_readback_join_excluded": true,
      "stage_label_note": "steady_state_group names the timed worker group; no cold load/compile timing is claimed",
      "timed_boundary": "start_barrier_to_last_worker_result_receive",
      "trial_count": 21
    },
    "wait_timeout_ns": 10000000,
    "wait_timing": "guest notify call start through guest waiter return; notify invocation overhead included",
    "wasmtime": {
      "reason": "registered capability is unsupported without threads/shared memory",
      "timing": "not_measured"
    }
  },
  "counts": {
    "cleanup_probes": 1,
    "encoded_module_fixtures": 5,
    "individual_wait_latency_samples": 1024,
    "source_snapshot_files": 187,
    "steady_state_trials": 336,
    "steady_state_workloads": 16,
    "store_buffering_trials": 1024,
    "total_timed_observations": 2384,
    "workloads": 18
  },
  "fixtures": [
    {
      "name": "atomic_add",
      "wasm": {
        "bytes": 123,
        "path": "/workspace/wtime/target/performance/P013-fixtures/atomic_add.wasm",
        "sha256": "46b8bdb39ef9b69deda41fa91c18f3cc4dd7ee7f58ee20dc6f1824d22a558029"
      },
      "wat": {
        "bytes": 390,
        "path": "/workspace/wtime/tools/wtime-bench/fixtures/threads/atomic_add.wat",
        "sha256": "f821deb4c8c142de5972053b9644029f7092a1be1468eb1ed10c1002d7e2776c"
      }
    },
    {
      "name": "atomic_load_store",
      "wasm": {
        "bytes": 140,
        "path": "/workspace/wtime/target/performance/P013-fixtures/atomic_load_store.wasm",
        "sha256": "85c25183d6ca23da20bbce7ae10b89acfb90499f413fa891151e558e7f4975ce"
      },
      "wat": {
        "bytes": 498,
        "path": "/workspace/wtime/tools/wtime-bench/fixtures/threads/atomic_load_store.wat",
        "sha256": "400f2e0826eff320b781ebd216dcf45a61fcccc73e8f60f8ef0af19e9ead330d"
      }
    },
    {
      "name": "ordinary_rw",
      "wasm": {
        "bytes": 138,
        "path": "/workspace/wtime/target/performance/P013-fixtures/ordinary_rw.wasm",
        "sha256": "2a0e01aaf458a30970c52b18855a27a508f6bac0f87dcd9c961a3246af58abdf"
      },
      "wat": {
        "bytes": 474,
        "path": "/workspace/wtime/tools/wtime-bench/fixtures/threads/ordinary_rw.wat",
        "sha256": "0a0d74f8b24c53299ac14d0094f13e38129326cfc421a41399e4a3ab9653f002"
      }
    },
    {
      "name": "sc_store_buffering",
      "wasm": {
        "bytes": 119,
        "path": "/workspace/wtime/target/performance/P013-fixtures/sc_store_buffering.wasm",
        "sha256": "d28965e8c9f3a157e7e3a55f80680e1542654699f3a49016a2b3532f404c4d22"
      },
      "wat": {
        "bytes": 369,
        "path": "/workspace/wtime/tools/wtime-bench/fixtures/threads/sc_store_buffering.wat",
        "sha256": "fc6f24e92a0904de4797731ce2642e47adb46eeac2f8a62549db67889b63a12e"
      }
    },
    {
      "name": "wait_notify",
      "wasm": {
        "bytes": 185,
        "path": "/workspace/wtime/target/performance/P013-fixtures/wait_notify.wasm",
        "sha256": "6a307ae5db566a43b1c3a8753e361f0aef14304ed7e56144f029fa5d6c268e06"
      },
      "wat": {
        "bytes": 514,
        "path": "/workspace/wtime/tools/wtime-bench/fixtures/threads/wait_notify.wat",
        "sha256": "94ce5b5bd02b46b71b0677795e79beb85081b928edc154bda30f2b3daadaebcb"
      }
    }
  ],
  "limits": [
    "Pinned default Wasmtime lacks threads/shared-memory capability; all 18 comparator entries are unsupported and timing was not measured. No cross-runtime timing ratio is available.",
    "Wait timing includes guest notify invocation overhead and there is no independent public pre-timer registration observer; successful notify returning one demonstrates registration at its call linearization point.",
    "Timeout/cancellation cleanup values are correctness diagnostics, not timing observations.",
    "The store-buffering oracle forbids [0,0]; observed legal outcomes are recorded in the workload summary.",
    "P013 is a generated shared-memory/thread workload suite, not representative application performance. Report source_stable binds the exact measurement-time 187-file source snapshot."
  ],
  "outcome": "contract_only",
  "passed": true,
  "report": {
    "bytes": 392532,
    "path": "/workspace/wtime/target/performance/P013/report.json",
    "sha256": "959d62f71b50220e74dc2940457c9a16345a3a773818a482324f2659e016ae5d"
  },
  "run_log": {
    "bytes": 86,
    "path": "/workspace/wtime/target/performance/P013-run.log",
    "sha256": "c8d5846b07d948f66de2e2b9a3541e3cf4976ba479c1b6ddf9d9ea93a5dc2c34"
  },
  "selection": "threads",
  "source_binding": {
    "driver_binary_sha256": "2d9b4e89d610fe152d18abae399bc1b8e1fad704b6bfa77270f92386a8858750",
    "driver_source_sha256": "96801dbc985a214e335c2be69dedce48a1cd2acf37055547f3d5fc87a21f4ee3",
    "encoder_binary_sha256": "5123b38fd1e6b2358027778d94256202e53ff7024b3b05cd94aa5ea50d7949d4",
    "registry_sha256": "37633e7b0fd99c729b74d408da76a2bb329e6ffb230c1bc75782a4af57b086c9",
    "schema_sha256": "03e7e3370e5f1463361577f28612b184e1b02e64006f74be3ad8167133d907d8",
    "source_snapshot_sha256": "d873ebcb4888110f2b053eaead5a7e0d8c0c3ebd7a9b9f993dada7397f235d8a",
    "workload_model_sha256": "f50f832f5f0eed010b3277fdcb15cacd0b600298026df1ebac1e00265864f21c"
  },
  "source_stable": true,
  "status": "done",
  "task": "P013",
  "tests": {
    "artifact": {
      "bytes": 2376,
      "path": "/workspace/wtime/target/performance/P013-tests-final2.log",
      "sha256": "3f5f84b5fff43ba489eab740ae3e4fe26883f0dab92c723776f7691d0ba54a20"
    },
    "failed": 0,
    "passed": 13,
    "skipped": 0
  },
  "workloads": [
    {
      "capability": {
        "reason": "The pinned default Wasmtime oracle is configured without threads/shared-memory support; capability preflight must emit no timing row.",
        "status": "unsupported",
        "timing": "not_measured"
      },
      "declared_trials": 21,
      "effective_workers": 1,
      "requested_workers": 1,
      "stage": "steady_state_group",
      "status": "ok",
      "timed_trials": 21,
      "timing_boundary": "start_barrier_to_last_worker_result_receive; setup/readback/join excluded",
      "total_operations_per_trial": 32,
      "trial_wall_ns_count": 21,
      "worker_quota": 4,
      "workload": "threads_atomic_add_same_w1_n32"
    },
    {
      "capability": {
        "reason": "The pinned default Wasmtime oracle is configured without threads/shared-memory support; capability preflight must emit no timing row.",
        "status": "unsupported",
        "timing": "not_measured"
      },
      "declared_trials": 21,
      "effective_workers": 1,
      "requested_workers": 1,
      "stage": "steady_state_group",
      "status": "ok",
      "timed_trials": 21,
      "timing_boundary": "start_barrier_to_last_worker_result_receive; setup/readback/join excluded",
      "total_operations_per_trial": 1024,
      "trial_wall_ns_count": 21,
      "worker_quota": 4,
      "workload": "threads_atomic_add_same_w1_n1024"
    },
    {
      "capability": {
        "reason": "The pinned default Wasmtime oracle is configured without threads/shared-memory support; capability preflight must emit no timing row.",
        "status": "unsupported",
        "timing": "not_measured"
      },
      "declared_trials": 21,
      "effective_workers": 2,
      "requested_workers": 2,
      "stage": "steady_state_group",
      "status": "ok",
      "timed_trials": 21,
      "timing_boundary": "start_barrier_to_last_worker_result_receive; setup/readback/join excluded",
      "total_operations_per_trial": 64,
      "trial_wall_ns_count": 21,
      "worker_quota": 4,
      "workload": "threads_atomic_add_same_w2_n32"
    },
    {
      "capability": {
        "reason": "The pinned default Wasmtime oracle is configured without threads/shared-memory support; capability preflight must emit no timing row.",
        "status": "unsupported",
        "timing": "not_measured"
      },
      "declared_trials": 21,
      "effective_workers": 2,
      "requested_workers": 2,
      "stage": "steady_state_group",
      "status": "ok",
      "timed_trials": 21,
      "timing_boundary": "start_barrier_to_last_worker_result_receive; setup/readback/join excluded",
      "total_operations_per_trial": 2048,
      "trial_wall_ns_count": 21,
      "worker_quota": 4,
      "workload": "threads_atomic_add_same_w2_n1024"
    },
    {
      "capability": {
        "reason": "The pinned default Wasmtime oracle is configured without threads/shared-memory support; capability preflight must emit no timing row.",
        "status": "unsupported",
        "timing": "not_measured"
      },
      "declared_trials": 21,
      "effective_workers": 4,
      "requested_workers": 4,
      "stage": "steady_state_group",
      "status": "ok",
      "timed_trials": 21,
      "timing_boundary": "start_barrier_to_last_worker_result_receive; setup/readback/join excluded",
      "total_operations_per_trial": 128,
      "trial_wall_ns_count": 21,
      "worker_quota": 4,
      "workload": "threads_atomic_add_same_w4_n32"
    },
    {
      "capability": {
        "reason": "The pinned default Wasmtime oracle is configured without threads/shared-memory support; capability preflight must emit no timing row.",
        "status": "unsupported",
        "timing": "not_measured"
      },
      "declared_trials": 21,
      "effective_workers": 4,
      "requested_workers": 4,
      "stage": "steady_state_group",
      "status": "ok",
      "timed_trials": 21,
      "timing_boundary": "start_barrier_to_last_worker_result_receive; setup/readback/join excluded",
      "total_operations_per_trial": 4096,
      "trial_wall_ns_count": 21,
      "worker_quota": 4,
      "workload": "threads_atomic_add_same_w4_n1024"
    },
    {
      "capability": {
        "reason": "The pinned default Wasmtime oracle is configured without threads/shared-memory support; capability preflight must emit no timing row.",
        "status": "unsupported",
        "timing": "not_measured"
      },
      "declared_trials": 21,
      "effective_workers": 1,
      "requested_workers": 1,
      "stage": "steady_state_group",
      "status": "ok",
      "timed_trials": 21,
      "timing_boundary": "start_barrier_to_last_worker_result_receive; setup/readback/join excluded",
      "total_operations_per_trial": 32,
      "trial_wall_ns_count": 21,
      "worker_quota": 4,
      "workload": "threads_atomic_add_distinct_w1_n32"
    },
    {
      "capability": {
        "reason": "The pinned default Wasmtime oracle is configured without threads/shared-memory support; capability preflight must emit no timing row.",
        "status": "unsupported",
        "timing": "not_measured"
      },
      "declared_trials": 21,
      "effective_workers": 1,
      "requested_workers": 1,
      "stage": "steady_state_group",
      "status": "ok",
      "timed_trials": 21,
      "timing_boundary": "start_barrier_to_last_worker_result_receive; setup/readback/join excluded",
      "total_operations_per_trial": 1024,
      "trial_wall_ns_count": 21,
      "worker_quota": 4,
      "workload": "threads_atomic_add_distinct_w1_n1024"
    },
    {
      "capability": {
        "reason": "The pinned default Wasmtime oracle is configured without threads/shared-memory support; capability preflight must emit no timing row.",
        "status": "unsupported",
        "timing": "not_measured"
      },
      "declared_trials": 21,
      "effective_workers": 2,
      "requested_workers": 2,
      "stage": "steady_state_group",
      "status": "ok",
      "timed_trials": 21,
      "timing_boundary": "start_barrier_to_last_worker_result_receive; setup/readback/join excluded",
      "total_operations_per_trial": 64,
      "trial_wall_ns_count": 21,
      "worker_quota": 4,
      "workload": "threads_atomic_add_distinct_w2_n32"
    },
    {
      "capability": {
        "reason": "The pinned default Wasmtime oracle is configured without threads/shared-memory support; capability preflight must emit no timing row.",
        "status": "unsupported",
        "timing": "not_measured"
      },
      "declared_trials": 21,
      "effective_workers": 2,
      "requested_workers": 2,
      "stage": "steady_state_group",
      "status": "ok",
      "timed_trials": 21,
      "timing_boundary": "start_barrier_to_last_worker_result_receive; setup/readback/join excluded",
      "total_operations_per_trial": 2048,
      "trial_wall_ns_count": 21,
      "worker_quota": 4,
      "workload": "threads_atomic_add_distinct_w2_n1024"
    },
    {
      "capability": {
        "reason": "The pinned default Wasmtime oracle is configured without threads/shared-memory support; capability preflight must emit no timing row.",
        "status": "unsupported",
        "timing": "not_measured"
      },
      "declared_trials": 21,
      "effective_workers": 4,
      "requested_workers": 4,
      "stage": "steady_state_group",
      "status": "ok",
      "timed_trials": 21,
      "timing_boundary": "start_barrier_to_last_worker_result_receive; setup/readback/join excluded",
      "total_operations_per_trial": 128,
      "trial_wall_ns_count": 21,
      "worker_quota": 4,
      "workload": "threads_atomic_add_distinct_w4_n32"
    },
    {
      "capability": {
        "reason": "The pinned default Wasmtime oracle is configured without threads/shared-memory support; capability preflight must emit no timing row.",
        "status": "unsupported",
        "timing": "not_measured"
      },
      "declared_trials": 21,
      "effective_workers": 4,
      "requested_workers": 4,
      "stage": "steady_state_group",
      "status": "ok",
      "timed_trials": 21,
      "timing_boundary": "start_barrier_to_last_worker_result_receive; setup/readback/join excluded",
      "total_operations_per_trial": 4096,
      "trial_wall_ns_count": 21,
      "worker_quota": 4,
      "workload": "threads_atomic_add_distinct_w4_n1024"
    },
    {
      "capability": {
        "reason": "The pinned default Wasmtime oracle is configured without threads/shared-memory support; capability preflight must emit no timing row.",
        "status": "unsupported",
        "timing": "not_measured"
      },
      "declared_trials": 21,
      "effective_workers": 1,
      "requested_workers": 1,
      "stage": "steady_state_group",
      "status": "ok",
      "timed_trials": 21,
      "timing_boundary": "start_barrier_to_last_worker_result_receive; setup/readback/join excluded",
      "total_operations_per_trial": 32,
      "trial_wall_ns_count": 21,
      "worker_quota": 4,
      "workload": "threads_ordinary_rw_n32"
    },
    {
      "capability": {
        "reason": "The pinned default Wasmtime oracle is configured without threads/shared-memory support; capability preflight must emit no timing row.",
        "status": "unsupported",
        "timing": "not_measured"
      },
      "declared_trials": 21,
      "effective_workers": 1,
      "requested_workers": 1,
      "stage": "steady_state_group",
      "status": "ok",
      "timed_trials": 21,
      "timing_boundary": "start_barrier_to_last_worker_result_receive; setup/readback/join excluded",
      "total_operations_per_trial": 1024,
      "trial_wall_ns_count": 21,
      "worker_quota": 4,
      "workload": "threads_ordinary_rw_n1024"
    },
    {
      "capability": {
        "reason": "The pinned default Wasmtime oracle is configured without threads/shared-memory support; capability preflight must emit no timing row.",
        "status": "unsupported",
        "timing": "not_measured"
      },
      "declared_trials": 21,
      "effective_workers": 1,
      "requested_workers": 1,
      "stage": "steady_state_group",
      "status": "ok",
      "timed_trials": 21,
      "timing_boundary": "start_barrier_to_last_worker_result_receive; setup/readback/join excluded",
      "total_operations_per_trial": 32,
      "trial_wall_ns_count": 21,
      "worker_quota": 4,
      "workload": "threads_atomic_load_store_n32"
    },
    {
      "capability": {
        "reason": "The pinned default Wasmtime oracle is configured without threads/shared-memory support; capability preflight must emit no timing row.",
        "status": "unsupported",
        "timing": "not_measured"
      },
      "declared_trials": 21,
      "effective_workers": 1,
      "requested_workers": 1,
      "stage": "steady_state_group",
      "status": "ok",
      "timed_trials": 21,
      "timing_boundary": "start_barrier_to_last_worker_result_receive; setup/readback/join excluded",
      "total_operations_per_trial": 1024,
      "trial_wall_ns_count": 21,
      "worker_quota": 4,
      "workload": "threads_atomic_load_store_n1024"
    },
    {
      "capability": {
        "reason": "The pinned default Wasmtime oracle is configured without threads/shared-memory support; capability preflight must emit no timing row.",
        "status": "unsupported",
        "timing": "not_measured"
      },
      "cleanup": {
        "cancellation_error_kind": "ResourceLimit",
        "cancellation_message": "execution cancelled during atomic wait",
        "cancellation_notify_after": 0,
        "case": "threads_wait_cleanup",
        "registration_observer_available": false,
        "schema_version": 1,
        "status": "ok",
        "timeout_elapsed_ns": 10087375,
        "timeout_notify_after": 0,
        "timeout_ns": 10000000,
        "timeout_result": 2,
        "value_mismatch_result": 1,
        "zero_timeout_result": 2
      },
      "p50_ns": 2963,
      "p99_ns": 821649,
      "p99_status": "reported",
      "registration_limit": "There is no independent public pre-timer registration observer. A successful notify returning one proves registration at that call's linearization point; each reported interval starts before the guest notify call and ends at guest waiter return, so it includes notify invocation overhead. Failed notify(1)==0 probes are excluded.",
      "registration_observed": true,
      "sample_count": 1024,
      "sample_kind": "individual_wait_notify_round_trips",
      "stage": "steady_state_group",
      "status": "ok",
      "timeout_ns": 10000000,
      "wait_timing": "guest notify call start through guest waiter return; notify invocation overhead included",
      "workload": "threads_wait_notify_latency"
    },
    {
      "all_trial_results_count": 1024,
      "capability": {
        "reason": "The pinned default Wasmtime oracle is configured without threads/shared-memory support; capability preflight must emit no timing row.",
        "status": "unsupported",
        "timing": "not_measured"
      },
      "declared_trials": 1024,
      "forbidden_outcome": [
        0,
        0
      ],
      "observed_outcome_counts": {
        "0,1": 239,
        "1,0": 731,
        "1,1": 54
      },
      "stage": "steady_state_group",
      "status": "ok",
      "timed_trials": 1024,
      "timing_boundary": null,
      "trial_wall_ns_count": 1024,
      "workload": "threads_sc_store_buffering"
    }
  ]
}
```
