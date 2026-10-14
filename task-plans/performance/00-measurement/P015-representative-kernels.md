# P015 — Add reproducible application-shaped held-out kernels

Status: **done**

Outcome: **contract_only**

Kind: `foundation` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P003 — Define workload selections and engine capability preflight](P003-workload-schema.md)

[P008 — Add scaling, branch and call benchmark families](P008-scalar-workloads.md)

[P009 — Add memory width, bulk and growth benchmark families](P009-memory-workloads.md)

[P010 — Add SIMD and strict floating-point benchmarks](P010-simd-float-workloads.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `tools/wtime-bench/fixtures/representative/*`
- `tools/wtime-bench/oracles/representative.py`
- `tools/wtime-bench/workloads.json`
- `tools/wtime-bench/tests/test_p015_representative.py`
- `docs/benchmarks/performance/fixture-provenance.json`

- `tools/wtime-bench/src/representative_driver.rs`

- `tools/wtime-bench/src/protocol.rs`

- `tools/wtime-bench/representative_scaling.py`

- `tools/wtime-bench/representative_workloads.py`

- `tools/wtime-bench/tests/test_p015_scaling.py`

- `tools/wtime-bench/workloads.py`

- `docs/benchmarks/performance/P015-evidence.json`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

Representative selection includes small deterministic parser/hash, image/filter and graph kernels with pinned source/license/toolchain/artifact provenance and independent output digests; no WASI requirement is introduced.

## Implementation recipe

1. Choose three compact original or permissively licensed kernels that exercise mixed control/memory/calls; retain source and record provenance instead of downloading moving benchmark suites.
2. Build freestanding Wasm from a pinned available compiler where feasible, or supply readable original WAT with a reproducible generator; record exact command and compiler/hash, avoiding unchecked prebuilt artifacts.
3. Define small/medium inputs, expected digest via a host reference implementation, import-free run entry points and deterministic reset/setup.
4. Freeze a held-out input set before tuning and record results across baseline/Wasmtime/candidate; keep artifacts small and generated large binaries ignored.

## Meaningful cases

- Regeneration matches recorded Wasm hashes on the pinned toolchain.
- Each input digest matches an independent host implementation.
- No fixture depends on WASI/browser behavior and held-out data are not selected after seeing optimization wins.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
python3 -m unittest discover -s tools/wtime-bench/tests -p 'test_p015*.py' -v
```

## Measurement and decision

Selection: `representative`. Explicit execution setting: `decoded`. Decision rule: `integrity`.

This is a tooling/contract deliverable. Require correct nonzero checks and bounded/source-bound output; do not invent a runtime speedup requirement. If it changes production work, record cold/build/retention deltas under [MEASUREMENT.md](../MEASUREMENT.md).

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. Executed evidence is recorded below.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/docs/benchmarks/performance/fixture-provenance.json": {
      "bytes": 2142,
      "sha256": "a2b3012c834b172ab04f615236c3cbd09cbdfc9418d79573881e62dc9639bc7f"
    },
    "/workspace/wtime/target/debug/wtime-conformance": {
      "bytes": 24493024,
      "sha256": "5123b38fd1e6b2358027778d94256202e53ff7024b3b05cd94aa5ea50d7949d4"
    },
    "/workspace/wtime/target/performance/P015-baseline.log": {
      "bytes": 244,
      "sha256": "203c518814df36f5a53cd549b0a46877c9f757bec3069ce8531dc1507594014b"
    },
    "/workspace/wtime/target/performance/P015-before/binary/wtime-bench": {
      "bytes": 17262064,
      "sha256": "4e6d01abd36fde9e2b5f94ecc367949a4de7d0f946ce20fcdcdc7b335dd91d08"
    },
    "/workspace/wtime/target/performance/P015-before/manifest.json": {
      "bytes": 29601,
      "sha256": "2bac1d438973ef3bb46b1775117c695a9477528e81ca5f1d604e62e5935d5368"
    },
    "/workspace/wtime/target/performance/P015-build.log": {
      "bytes": 815,
      "sha256": "1b303c2719e2fda5107278c487c89f985c32ed673ff1ef818f1ae07823b9e77c"
    },
    "/workspace/wtime/target/performance/P015-build2.log": {
      "bytes": 132,
      "sha256": "ad1e89aabe876927dde1a9748f42dbc440d111329b3914ab3d246453c6661cc2"
    },
    "/workspace/wtime/target/performance/P015-fixtures/graph_bfs.wasm": {
      "bytes": 460,
      "sha256": "28de8728d043d81703727cd71f4f67c9c1aa32b45fecba0f8bb760560ee2f63c"
    },
    "/workspace/wtime/target/performance/P015-fixtures/image_filter.wasm": {
      "bytes": 399,
      "sha256": "678eee3d25fff15f867befd2602cf403f25db7da67339f1f1b7b9b2c4e2e6f0a"
    },
    "/workspace/wtime/target/performance/P015-fixtures/parser_hash.wasm": {
      "bytes": 290,
      "sha256": "4ab2c42b5ec50f2b8a2d4c325ebaa54a6673b7ce42265cf9f9f477788f634a98"
    },
    "/workspace/wtime/target/performance/P015-run.log": {
      "bytes": 106,
      "sha256": "483d26389b377243a76841be88a0dba2171a5bc4104007bde10ab7287c55e34e"
    },
    "/workspace/wtime/target/performance/P015-run2.log": {
      "bytes": 100,
      "sha256": "3a4780848fd335f18c9fe1c7bbaaa2af9750feb60631ba765bca05cd81eba8ce"
    },
    "/workspace/wtime/target/performance/P015-run3.log": {
      "bytes": 85,
      "sha256": "ad4bd3301293d5f45b7937953aa4656f0c0aebc57721bfdd5672fcdd099d3948"
    },
    "/workspace/wtime/target/performance/P015-tests-final.log": {
      "bytes": 1711,
      "sha256": "e3b1dd68bb5d8440d5b61a885a35c9ab7a43c5691bd33fcce70b260fa945e81d"
    },
    "/workspace/wtime/target/performance/P015-tests.log": {
      "bytes": 2121,
      "sha256": "eab63a5db4717c45f66f20ff68905f6b9dce80786eed992ddca5782bb9399f98"
    },
    "/workspace/wtime/target/performance/P015-tests2.log": {
      "bytes": 1711,
      "sha256": "e2b0370c0c67eac88914a5f2ba726313175604f938a73ba356b50a4fedae77c7"
    },
    "/workspace/wtime/target/performance/P015/binary/wtime-bench": {
      "bytes": 17262080,
      "sha256": "a3c01d83a184acbea2659e3b62a411a817a36ef6b9d781c81b041182a05fab70"
    },
    "/workspace/wtime/target/performance/P015/report.json": {
      "bytes": 17878942,
      "sha256": "187c94fabf93fcbb07505a225459029cc431305bd04852b959522e75fd4ef003"
    },
    "/workspace/wtime/tools/wtime-bench/fixtures/representative/graph_bfs.wat": {
      "bytes": 2552,
      "sha256": "641c210679045e870953ec340aa479d4282fd5d798c8e5857b507c49c2643069"
    },
    "/workspace/wtime/tools/wtime-bench/fixtures/representative/image_filter.wat": {
      "bytes": 2137,
      "sha256": "081e25e5b2f01d521167678c1aca46c967f4379989e945f14bc59bcc670e225c"
    },
    "/workspace/wtime/tools/wtime-bench/fixtures/representative/parser_hash.wat": {
      "bytes": 1419,
      "sha256": "44aefe0593f79d3732466f248d1c69f1e9450145b15c6bd64908bc8f217bf32b"
    }
  },
  "artifacts": [
    "/workspace/wtime/target/performance/P015/report.json",
    "/workspace/wtime/target/performance/P015-run3.log",
    "/workspace/wtime/target/performance/P015-tests-final.log",
    "/workspace/wtime/target/performance/P015-baseline.log",
    "/workspace/wtime/target/performance/P015-build2.log",
    "/workspace/wtime/target/performance/P015-before/manifest.json",
    "/workspace/wtime/target/performance/P015-before/binary/wtime-bench",
    "/workspace/wtime/target/performance/P015/binary/wtime-bench",
    "/workspace/wtime/target/debug/wtime-conformance",
    "/workspace/wtime/target/performance/P015-fixtures/graph_bfs.wasm",
    "/workspace/wtime/target/performance/P015-fixtures/image_filter.wasm",
    "/workspace/wtime/target/performance/P015-fixtures/parser_hash.wasm",
    "/workspace/wtime/tools/wtime-bench/fixtures/representative/graph_bfs.wat",
    "/workspace/wtime/tools/wtime-bench/fixtures/representative/image_filter.wat",
    "/workspace/wtime/tools/wtime-bench/fixtures/representative/parser_hash.wat",
    "/workspace/wtime/docs/benchmarks/performance/fixture-provenance.json",
    "/workspace/wtime/target/performance/P015-run.log",
    "/workspace/wtime/target/performance/P015-run2.log",
    "/workspace/wtime/target/performance/P015-tests.log",
    "/workspace/wtime/target/performance/P015-tests2.log",
    "/workspace/wtime/target/performance/P015-build.log"
  ],
  "baseline_archive": {
    "listed_files": 109,
    "manifest_sha256": "2bac1d438973ef3bb46b1775117c695a9477528e81ca5f1d604e62e5935d5368",
    "mismatches": [],
    "path": "/workspace/wtime/target/performance/P015-before",
    "verified_files": 109
  },
  "binary_identities": {
    "baseline": {
      "path": "/workspace/wtime/target/performance/P015-before/binary/wtime-bench",
      "sha256": "4e6d01abd36fde9e2b5f94ecc367949a4de7d0f946ce20fcdcdc7b335dd91d08"
    },
    "candidate": {
      "path": "/workspace/wtime/tools/wtime-bench/target/release/wtime-bench",
      "sha256": "a3c01d83a184acbea2659e3b62a411a817a36ef6b9d781c81b041182a05fab70"
    },
    "encoder": {
      "path": "/workspace/wtime/target/debug/wtime-conformance",
      "sha256": "5123b38fd1e6b2358027778d94256202e53ff7024b3b05cd94aa5ea50d7949d4"
    },
    "wasmtime_harness": {
      "path": "/workspace/wtime/tools/wtime-bench/target/release/wtime-bench",
      "sha256": "a3c01d83a184acbea2659e3b62a411a817a36ef6b9d781c81b041182a05fab70"
    }
  },
  "commands": [
    {
      "command": ". /workspace/.setup/rust-env.sh && cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml -j 4 --no-default-features",
      "evidence": "target/performance/P015-build2.log",
      "exit_code": 0,
      "note": "Final candidate build log; no build duration is used as performance evidence."
    },
    {
      "command": "python3 tools/wtime-bench/representative_scaling.py --baseline target/performance/P015-before/binary/wtime-bench --candidate tools/wtime-bench/target/release/wtime-bench --wasmtime tools/wtime-bench/target/release/wtime-bench --encoder target/debug/wtime-conformance --source-root /workspace/wtime --registry tools/wtime-bench/workloads.json --model tools/wtime-bench/representative_workloads.py --encoded-dir target/performance/P015-fixtures --output target/performance/P015/report.json --samples 5",
      "evidence": "target/performance/P015-run3.log",
      "exit_code": 0,
      "note": "Invocation reconstructed from the report paths and runner arguments; the retained run log confirms 9 cases and source_stable=true."
    },
    {
      "command": "python3 -m unittest discover -s tools/wtime-bench/tests -p 'test_p015*.py' -v",
      "evidence": "target/performance/P015-tests-final.log",
      "exit_code": 0,
      "failed": 0,
      "passed": 9,
      "skipped": 0
    },
    {
      "command": "python3 tools/wtime-bench/baseline.py --output target/performance/P015-before",
      "evidence": "target/performance/P015-baseline.log",
      "exit_code": 0,
      "note": "Archive contains 109 manifest entries; all manifest paths, sizes, and SHA-256 values independently verified."
    }
  ],
  "correctness": {
    "all_full_memory_hashes_match": true,
    "all_observations_status_ok": true,
    "all_results_and_output_hashes_match": true,
    "one_64KiB_page_per_observation": true,
    "readback_outside_timing": true
  },
  "counts": {
    "observations": 135,
    "per_engine": {
      "baseline": 45,
      "candidate": 45,
      "wasmtime": 45
    },
    "wasmtime_capabilities_supported": 9,
    "wasmtime_capabilities_unsupported": 0,
    "workloads": 9
  },
  "fixtures": {
    "provenance": {
      "kernels": {
        "representative_graph_bfs": {
          "encoder": "wtime-conformance existing encoder",
          "encoder_sha256": "5123b38fd1e6b2358027778d94256202e53ff7024b3b05cd94aa5ea50d7949d4",
          "encoding_command": "wtime-conformance encode {source} -o {output}",
          "external_source_or_download": false,
          "license": "CC0-1.0",
          "origin": "original WAT authored for the wtime performance fixture set",
          "source": "fixtures/representative/graph_bfs.wat",
          "source_sha256": "641c210679045e870953ec340aa479d4282fd5d798c8e5857b507c49c2643069",
          "wasm_sha256": "28de8728d043d81703727cd71f4f67c9c1aa32b45fecba0f8bb760560ee2f63c"
        },
        "representative_image_filter": {
          "encoder": "wtime-conformance existing encoder",
          "encoder_sha256": "5123b38fd1e6b2358027778d94256202e53ff7024b3b05cd94aa5ea50d7949d4",
          "encoding_command": "wtime-conformance encode {source} -o {output}",
          "external_source_or_download": false,
          "license": "CC0-1.0",
          "origin": "original WAT authored for the wtime performance fixture set",
          "source": "fixtures/representative/image_filter.wat",
          "source_sha256": "081e25e5b2f01d521167678c1aca46c967f4379989e945f14bc59bcc670e225c",
          "wasm_sha256": "678eee3d25fff15f867befd2602cf403f25db7da67339f1f1b7b9b2c4e2e6f0a"
        },
        "representative_parser_hash": {
          "encoder": "wtime-conformance existing encoder",
          "encoder_sha256": "5123b38fd1e6b2358027778d94256202e53ff7024b3b05cd94aa5ea50d7949d4",
          "encoding_command": "wtime-conformance encode {source} -o {output}",
          "external_source_or_download": false,
          "license": "CC0-1.0",
          "origin": "original WAT authored for the wtime performance fixture set",
          "source": "fixtures/representative/parser_hash.wat",
          "source_sha256": "44aefe0593f79d3732466f248d1c69f1e9450145b15c6bd64908bc8f217bf32b",
          "wasm_sha256": "4ab2c42b5ec50f2b8a2d4c325ebaa54a6673b7ce42265cf9f9f477788f634a98"
        }
      },
      "toolchain_notes": "Readable original WAT avoids an external compiler and moving benchmark suite; output bytes are produced by the pinned existing encoder."
    },
    "provenance_artifact_sha256": "a2b3012c834b172ab04f615236c3cbd09cbdfc9418d79573881e62dc9639bc7f",
    "wasm_sha256": {
      "graph_bfs.wat": "28de8728d043d81703727cd71f4f67c9c1aa32b45fecba0f8bb760560ee2f63c",
      "image_filter.wat": "678eee3d25fff15f867befd2602cf403f25db7da67339f1f1b7b9b2c4e2e6f0a",
      "parser_hash.wat": "4ab2c42b5ec50f2b8a2d4c325ebaa54a6673b7ce42265cf9f9f477788f634a98"
    }
  },
  "limitations": [
    "Evidence is integrity-only: batch size is one and five samples per input are not calibrated comparison evidence.",
    "No Wasmtime-relative timing ratio or speedup is claimed.",
    "The report preserves all 135 raw observations, including full memory payloads; this summary deliberately omits them.",
    "Source stability is the runner\u2019s within-run result. Later workspace changes are listed separately and do not rewrite the historical report."
  ],
  "outcome": "integrity_only",
  "passed": true,
  "raw_observations_preserved": true,
  "report": "/workspace/wtime/target/performance/P015/report.json",
  "report_sha256": "187c94fabf93fcbb07505a225459029cc431305bd04852b959522e75fd4ef003",
  "selection": "representative",
  "settings": {
    "baseline_mode": "decoded",
    "candidate_mode": "decoded",
    "instrumented": false,
    "profile": "mvp",
    "samples_per_input_per_engine": 5,
    "stage": "run_after_setup",
    "timing_scope": "run after setup; setup, compile, instantiate, and full-memory readback are outside timing",
    "wasmtime_mode": "wasmtime"
  },
  "source_binding": {
    "post_run_snapshot_differences": [
      "/workspace/wtime/crates/wtime-runtime/tests/performance/p022.rs",
      "/workspace/wtime/tools/wtime-bench/src/memory_driver.rs",
      "/workspace/wtime/tools/wtime-bench/target/release/wtime-bench",
      "/workspace/wtime/tools/wtime-bench/workloads.json"
    ],
    "source_snapshot_entries": 98,
    "source_stable_during_run": true
  },
  "source_stable": true,
  "status": "done",
  "task": "P015"
}
```
