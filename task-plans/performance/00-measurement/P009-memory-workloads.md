# P009 — Add memory width, bulk and growth benchmark families

Status: **done**

Outcome: **complete**

Kind: `foundation` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P003 — Define workload selections and engine capability preflight](P003-workload-schema.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `tools/wtime-bench/fixtures/memory/*.wat`
- `tools/wtime-bench/oracles/memory.py`
- `tools/wtime-bench/workloads.json`
- `tools/wtime-bench/tests/test_p009_memory.py`

- `tools/wtime-bench/memory_workloads.py`
- `tools/wtime-bench/memory_scaling.py`
- `tools/wtime-bench/src/memory_driver.rs`
- `tools/wtime-bench/src/protocol.rs`
- `tools/wtime-bench/tests/test_p009_memory_driver.py`
- `tools/wtime-bench/tests/test_p009_memory_scaling.py`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

Memory selection records bytes/accesses per operation, observable memory digests, reset policy and capability for scalar widths, memory64/multi-memory/page sizes, bulk overlap and growth.

## Implementation recipe

1. Add 1/2/4/8-byte signed/unsigned loads/stores, predictable and shuffled indices, unaligned scalar addresses, read-only scans and write/read mixes with deterministic byte-array oracles.
2. Add memory.copy/fill at 64B/4KiB/1MiB, both overlap directions and aliased imported memory; use fresh/reset state as declared, excluding reset cost from warm_call only when both engines do so.
3. Separate grow success/failure and memory64/page-size correctness from hot-access throughput; ensure increasing memory cannot contaminate repeated samples.
4. Include effective-address overflow/OOB partial-side-effect probes and full memory digests; preserve the original memory_fill_scan bytes as a historical control.

## Meaningful cases

- Both overlap directions equal Python memmove results.
- An OOB operation leaves exactly the permitted prior stores, with no later mutation.
- Growth stages have deterministic initial size and narrow signed loads retain sign extension.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
python3 -m unittest discover -s tools/wtime-bench/tests -p 'test_p009*.py' -v
```

## Measurement and decision

Selection: `memory`. Explicit execution setting: `decoded`. Decision rule: `integrity`.

This is a tooling/contract deliverable. Require correct nonzero checks and bounded/source-bound output; do not invent a runtime speedup requirement. If it changes production work, record cold/build/retention deltas under [MEASUREMENT.md](../MEASUREMENT.md).

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. Executed evidence is recorded below.

## Executed evidence

```json
{
  "task": "P009",
  "decision": "complete",
  "commands": [
    {
      "command": "WTIME_BENCH_MEMORY_DRIVER=tools/wtime-bench/profiling/target/release/wtime-bench python3 -m unittest discover -s tools/wtime-bench/tests -p 'test_p009*.py' -v",
      "exit_code": 0,
      "passed": 16,
      "failed": 0,
      "skipped": 0,
      "real_driver_subcases": 5
    },
    {
      "command": "python3 tools/wtime-bench/memory_scaling.py --binary tools/wtime-bench/profiling/target/release/wtime-bench --encoder target/debug/wtime-conformance --probe-binary tools/wtime-bench/target/release/wtime-bench --samples 5 --output target/performance/P009-scaling",
      "exit_code": 0,
      "observations": 120,
      "boundary_probes": 1
    },
    {
      "command": "python3 tools/wtime-bench/memory_scaling.py --binary tools/wtime-bench/profiling/target/release/wtime-bench --encoder target/debug/wtime-conformance --probe-binary tools/wtime-bench/target/release/wtime-bench --samples 1 --output target/performance/P009-compact",
      "exit_code": 0,
      "observations": 24,
      "boundary_probes": 1
    }
  ],
  "artifacts": [
    "/workspace/wtime/target/performance/P009-scaling/report.json",
    "/workspace/wtime/target/performance/P009-compact/report.json",
    "/workspace/wtime/target/performance/P009-tests.log",
    "/workspace/wtime/target/performance/P009-compact-tests.log",
    "/workspace/wtime/target/performance/P009-scaling/workloads-expanded.json"
  ],
  "artifact_sha256": {
    "/workspace/wtime/target/performance/P009-scaling/report.json": "8d9d8ca1982fe77b0d596b009da9d5dff5cf6dac339f5d007dc16e31cb3e3414",
    "/workspace/wtime/target/performance/P009-compact/report.json": "ef287923c7f9e4c3a5e8530f92ac8700d064dfa6cf0eb57de3c2ab608ca180c8",
    "/workspace/wtime/target/performance/P009-tests.log": "391759b42da35d506f2a60757820a3a2cbe4cc151e86d4cc4b00cf2f517c82df",
    "/workspace/wtime/target/performance/P009-compact-tests.log": "9e88e980aac897eff1a85fbda48c403eca645e5df735c4afc55fe0641ea73e56",
    "/workspace/wtime/target/performance/P009-scaling/workloads-expanded.json": "bc6e15e20312dc4709aca3c8ae58e98824720ffe96c4f618a5abc50905951ebf"
  },
  "settings": {
    "instrumented": false,
    "memory_hex_persisted": false,
    "mode": "decoded",
    "reset_policy": "fresh_instance_per_invocation",
    "samples_per_entry": 1,
    "stage": "load_and_call"
  },
  "source_set_sha256": "1100c007809c7f2cf63a3370eeb1021c19b1d1f9d92cd733cfb8d0d14b61e311",
  "candidate_binary_sha256": "16bdc67e6b38041de7f2032b7cea4e775e3d4e275fda909caea5baa8b275e9b4",
  "comparator_binary_sha256": "71485b2febdf3f3475fae528d777d22191a689b5f4bbdcf409e8a938be64d11a",
  "encoder_sha256": "6616cabefdaf1cb99514a43fc27b63a039210727ac0ee22e2dab3cfb2f5b4ce7",
  "fixture_hashes": {
    "bulk_copy_aliased.wat": {
      "path": "fixtures/memory/bulk_copy_aliased.wasm",
      "wasm_sha256": "62090599a827936312ab6e8aef30c8dd58950e5541925dc1f104cbcae631ea9f",
      "wat_sha256": "537407a213ca8fab037aa8c8a21c9026f69fb687c4ef549ca98b63c80f78b350"
    },
    "bulk_copy_backward.wat": {
      "path": "fixtures/memory/bulk_copy_backward.wasm",
      "wasm_sha256": "c471424ab8f70544c8910c159fb722762f50a69d8275672baadfdd4be610f24f",
      "wat_sha256": "fb8df158fd1b32d9f4904b8ecd7e1cbd30a4fc72516137ca3cd9e50c588fc98b"
    },
    "bulk_copy_forward.wat": {
      "path": "fixtures/memory/bulk_copy_forward.wasm",
      "wasm_sha256": "fa79408dcd1e5a6de0b30efec318fef7af1b32f38fa91aa85e096bc89e8242dd",
      "wat_sha256": "b9a27938596761c1436161193e7a391ef7bd967dc0929d84a5749f27c44ebe87"
    },
    "bulk_fill.wat": {
      "path": "fixtures/memory/bulk_fill.wasm",
      "wasm_sha256": "fce9add69d0ce7c47d47bf4df17d3af2a189bd0553f45dc49d26aec4d368bb91",
      "wat_sha256": "e5c02412b38dfe2fcbead753ff43c6005063f8f32e4d34c2af6dc61c613fe2c3"
    },
    "custom_page_one_byte.wat": {
      "path": "fixtures/memory/custom_page_one_byte.wasm",
      "wasm_sha256": "29dd399af1a1ba4bf59db09733986c819b5dfc072dce44b3a398adda316ba3b9",
      "wat_sha256": "db0b93cd8845b3e62363b0022f3d5aa5328550cd15f76f0a6bc3259728e5726b"
    },
    "effective_address_overflow.wat": {
      "path": "fixtures/memory/effective_address_overflow.wasm",
      "wasm_sha256": "670a1ec729e5580947ddc083bef8455df6d256918740102938ce4b79cc083ec9",
      "wat_sha256": "55bfac00d3a850898829d29e3762959f1743da76bb3067bd05ec455b12b5c3bb"
    },
    "growth.wat": {
      "path": "fixtures/memory/growth.wasm",
      "wasm_sha256": "077018a548b5a666bd3b909d844c7f620162312fa8032f927384ed5946766152",
      "wat_sha256": "6f2a67b40f25e75f20ec7113aafcff2c7736afa865e8071a0d141cdf74a85c82"
    },
    "memory64.wat": {
      "path": "fixtures/memory/memory64.wasm",
      "wasm_sha256": "760627846a98bffa0faf30cc8cb953b9eb72580a24eda9954d8da9d24ee8a3ff",
      "wat_sha256": "665bef075c3a08b202de3556fc1d00a86df030c5d9dd6f1bb39a26c8efa517aa"
    },
    "multi_memory_second.wat": {
      "path": "fixtures/memory/multi_memory_second.wasm",
      "wasm_sha256": "577f2cbd04a7c384a896c859ef04cd02573120f375c4d6df5b15790be610a310",
      "wat_sha256": "2aa28f49f655d02744d404d30e7932f38a03bd7e278ec4dda45b88c03ca7d276"
    },
    "oob_order.wat": {
      "path": "fixtures/memory/oob_order.wasm",
      "wasm_sha256": "44cfc8f92c7be0e77a8625cfbe2f1306007ff3e10e8e26aa551a4ee44037dabf",
      "wat_sha256": "a23b97a75e92b9e9d4e56040ff358bfc899ea5dfe1d5de0a799fbb7274ec0166"
    },
    "read_scan.wat": {
      "path": "fixtures/memory/read_scan.wasm",
      "wasm_sha256": "fa603f253df536b584a90272ab4bb9beb46cb4a0c8e51ca0a2ba3da3eda0ae18",
      "wat_sha256": "038f4cb301322d2c71d31a4606a6b6dab35dd27ff555f26c7d894f5a580d4259"
    },
    "widths.wat": {
      "path": "fixtures/memory/widths.wasm",
      "wasm_sha256": "febd4bc1275c2db30f6d67c722a577c3e5fba64ee072c14175c5fb2f5769df74",
      "wat_sha256": "486d62872cff7ba519404a536d9d402eb267dbba18df867d79cc376c189c4425"
    },
    "widths_shuffled.wat": {
      "path": "fixtures/memory/widths_shuffled.wasm",
      "wasm_sha256": "e948f8be21a6e70d46be7dd3a1a3d78d700ba7535a6220bf8d213b2392eab511",
      "wat_sha256": "ecefc97475a82eca997250f5e55aadcf58ec787e4a60b1ae6c0da88ce39191a8"
    }
  },
  "wasmtime_capabilities": {
    "compile_execute_supported": 19,
    "explicit_unsupported": 5,
    "timings_collected": 0
  },
  "registry": {
    "current_bytes": 204560,
    "current_sha256": "254b260e127cb2734ff1fde9e918a377e61599a5ce758b4d2da65f29f1c5e508",
    "entries": 24,
    "selection_entries_including_historical_control": 25
  },
  "limitations": [
    "Only load_and_call observations are claimed; every measured invocation has fresh memory state, engine creation and post-call full-memory observation are outside timing. Read-scan initialization is explicitly included in its guest call.",
    "The five-sample report is bound to the archived expanded registry; the one-sample report verifies all 24 cases with the compact digest-only registry. Neither report is relabeled.",
    "The historical memory_fill_scan control retains its original bytes and is checked in P023-final-before archive smoke and the existing P021 paired baseline. The new memory reports contain the 24 added cases.",
    "Wasmtime capability probes collect no timing. Alias/import setup and trap-oracle cases are explicit comparator harness gaps. No speedup claim or production core change."
  ]
}
```
