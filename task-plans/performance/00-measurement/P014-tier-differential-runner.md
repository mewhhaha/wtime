# P014 — Run source-bound correctness checks across execution tiers

Status: **done**

Outcome: **contract_only**

Kind: `foundation` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P003 — Define workload selections and engine capability preflight](P003-workload-schema.md)

[P016 — Add explicit executor policies and test integration hooks](../01-execution-contracts/P016-execution-policy.md)

[P018 — Define source cursors and materialized guest state](../01-execution-contracts/P018-source-cursor.md)

[P020 — Centralize exact source instruction accounting](../01-execution-contracts/P020-exact-execution-accounting.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `tools/wtime-bench/tier_check.py`
- `tools/wtime-bench/tests/test_p014_tiers.py`
- `tools/wtime-conformance/src/main.rs`
- `tools/wtime-conformance/src/bin/wtime-probe.rs`
- `tools/wtime-conformance/Cargo.toml`
- `tools/wtime-conformance/src/cli.rs`
- `tools/wtime-conformance/src/adapter.rs`

- `tools/wtime-conformance/src/report.rs`

- `tools/wtime-bench/src/protocol.rs`

- `tools/wtime-bench/src/tier_state_driver.rs`

- `tools/wtime-bench/fixtures/integration/*.wat`

- `docs/benchmarks/performance/P014-evidence.json`

- `tools/wtime-bench/fixtures/integration/expected_state.json`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

tier_check.py --tier NAME --selection NAME --output DIR executes named cases/corpora with the requested tier or recorded fallback and writes fresh source/settings/count-bound evidence; absent tiers and zero executed cases fail.

## Implementation recipe

1. Add explicit executor-mode forwarding to the conformance/probe tools using P016's API; preserve existing default CLI/profile behavior and nondefault coverage instrumentation.
2. Compare reference/current tiers on outcomes, trap categories, source cursor, memory/global/table effects and normalized reference graphs; record helper sharing as an oracle limitation.
3. Run selected normative suites/profiles and generated seeds, recording actual tier selection/fallback reasons, optimized executed source-op counts and all failed/skipped/unsupported cases.
4. Bind reports to exact source/binary/spec/fixture identities, retain replay artifacts, and prevent reference-only execution from counting as optimized-path coverage.
5. Forward nondefault executor/reference features in the conformance Cargo manifest, CLI and adapter Engine creation; tier_check builds the requested feature set and fails on absent forwarding instead of silently testing the default executor.

## Meaningful cases

- A nonexistent or entirely unused tier is an error.
- A deliberate side-effect or source-PC mismatch is detected despite an equal scalar result.
- Unsupported operations fall back with explicit counts while supported optimized cases actually execute.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
python3 -m unittest discover -s tools/wtime-bench/tests -p 'test_p014*.py' -v
python3 tools/wtime-bench/tier_check.py --tier decoded --selection integration --output target/performance/P014/conformance
```

## Measurement and decision

Selection: `integration`. Explicit execution setting: `decoded`. Decision rule: `integrity`.

This is a tooling/contract deliverable. Require correct nonzero checks and bounded/source-bound output; do not invent a runtime speedup requirement. If it changes production work, record cold/build/retention deltas under [MEASUREMENT.md](../MEASUREMENT.md).

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. Executed evidence is recorded below.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/tmp/P014-tool-tests.log": {
      "bytes": 1420,
      "sha256": "a8658e24c977f28139c3f80dee12caa7447bf3029ff08f94e80befa26daf2d73"
    },
    "/workspace/wtime/crates/wtime-runtime/tests/fixtures/performance/cursor/store_successor.wasm": {
      "bytes": 60,
      "sha256": "e05575761936f326eebf004bcfc6b015cb6dbf8ab14412aa7daae268cff3ce96"
    },
    "/workspace/wtime/crates/wtime-runtime/tests/fixtures/performance/cursor/store_successor.wat": {
      "bytes": 276,
      "sha256": "366ad9a071ac01995edfcb5e7291c5bc3c3de4a33c7c52b63edfce3403d1fcbd"
    },
    "/workspace/wtime/target/performance/P014-run.log": {
      "bytes": 122,
      "sha256": "f18168edf968846e1e95b22efa738560b47f0c22e31cf06e14fb2f05a991b56f"
    },
    "/workspace/wtime/target/performance/P014-state-fixtures/state_graph.wasm": {
      "bytes": 333,
      "sha256": "5370ba21415ef9f2245caa51e73c86d8034b4ff294e4645c7dbb4701e15d36c2"
    },
    "/workspace/wtime/target/performance/P014-state-fixtures/state_normal.wasm": {
      "bytes": 138,
      "sha256": "63cdccecaa341f9f2d8ef8ad8f779356549d3227b234e73117f6bdd33e612a9b"
    },
    "/workspace/wtime/target/performance/P014-state-fixtures/state_trap.wasm": {
      "bytes": 148,
      "sha256": "94155562450b9f3668eb31dacb751fe24d2180e68f30a88a91a1cc88cc5476b7"
    },
    "/workspace/wtime/target/performance/P014/binary/wtime-bench": {
      "bytes": 16932296,
      "sha256": "4101e0338828224718ca96c056893a926a38dfef140af5d0776453885f0735f5"
    },
    "/workspace/wtime/target/performance/P014/binary/wtime-bench-counters": {
      "bytes": 16906344,
      "sha256": "72cb431d6200abbc7519fa0f143c14a884457c39d0203c3d3f45bdb090de7fe8"
    },
    "/workspace/wtime/target/performance/P014/binary/wtime-conformance": {
      "bytes": 3594440,
      "sha256": "ed680b5d08c6a26ede7719e9ecfde2b2ee22f17610424fe71560da8803327191"
    },
    "/workspace/wtime/target/performance/P014/conformance/conformance/interactions-decoded.json": {
      "bytes": 5620,
      "sha256": "204ebe1b0f8f78970cbb0aa38964a07765388e65c1eb032a9b6be77992dae139"
    },
    "/workspace/wtime/target/performance/P014/conformance/conformance/interactions-reference.json": {
      "bytes": 5626,
      "sha256": "8a6a820429001a2c3534df365c36f3765678d1902cccfea7ba8a72284111102e"
    },
    "/workspace/wtime/target/performance/P014/conformance/conformance/reference_types-decoded.json": {
      "bytes": 5544,
      "sha256": "ac3085250d16f865626e6eccf89ea201b5b855e4d8de3e114d2b150a21a97828"
    },
    "/workspace/wtime/target/performance/P014/conformance/conformance/reference_types-reference.json": {
      "bytes": 5550,
      "sha256": "f5a951bcc833229adba2aa587764eaea919ce6e528c64d446f9ac3b757863b16"
    },
    "/workspace/wtime/target/performance/P014/conformance/report.json": {
      "bytes": 94199,
      "sha256": "b7339dd2c47982e551ed700f26dd141415ef1b1ba00ccdc225b0ff52a2b02e91"
    },
    "/workspace/wtime/tools/wtime-bench/fixtures/branch_calls.wasm": {
      "bytes": 157,
      "sha256": "9b58d26f201900b5b6cce684133a98ab68b3cf55d979b64ce895e914e9d5327b"
    },
    "/workspace/wtime/tools/wtime-bench/fixtures/branch_calls.wat": {
      "bytes": 604,
      "sha256": "d0af4fdf9ad5be38344267ae72fc1855edb5a8fa1b475fd0af4cb507730b2309"
    },
    "/workspace/wtime/tools/wtime-bench/fixtures/call_overhead.wasm": {
      "bytes": 40,
      "sha256": "0815ca7d6026206d0773fe10c6b9643862bfffe8ba361c9789369ef9668ee746"
    },
    "/workspace/wtime/tools/wtime-bench/fixtures/call_overhead.wat": {
      "bytes": 88,
      "sha256": "9493b8f751690a2fd6e8b19dcee5e2c1d50b717253d292ce34d5692b306663a5"
    },
    "/workspace/wtime/tools/wtime-bench/fixtures/integer_loop.wasm": {
      "bytes": 122,
      "sha256": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c"
    },
    "/workspace/wtime/tools/wtime-bench/fixtures/integer_loop.wat": {
      "bytes": 391,
      "sha256": "791730112109f3d5751a7b3e2c1134e7e0e6f71a7f7ee01ea8481eb989ee3fa4"
    },
    "/workspace/wtime/tools/wtime-bench/fixtures/integration/expected_state.json": {
      "bytes": 2534,
      "sha256": "126d72095766530e35a90ad864c8f90399c88c4bcac3bc039e02324bdd407ba9"
    },
    "/workspace/wtime/tools/wtime-bench/fixtures/integration/state_graph.wat": {
      "bytes": 1488,
      "sha256": "a2c4d3f0986f615828779cf256f15e2db425697e2240fecbd179e5b967612589"
    },
    "/workspace/wtime/tools/wtime-bench/fixtures/integration/state_normal.wat": {
      "bytes": 412,
      "sha256": "9e0278001f41ea1f2cc229b3ddc7c670f24d1f2860adb5c552fa613037f43717"
    },
    "/workspace/wtime/tools/wtime-bench/fixtures/integration/state_trap.wat": {
      "bytes": 459,
      "sha256": "99c3408b7697ad7184440d5d37593796b94a2ddf5c6369f0f27f61be5c1ba72d"
    }
  },
  "artifacts": [
    {
      "bytes": 94199,
      "path": "/workspace/wtime/target/performance/P014/conformance/report.json",
      "sha256": "b7339dd2c47982e551ed700f26dd141415ef1b1ba00ccdc225b0ff52a2b02e91"
    },
    {
      "bytes": 122,
      "path": "/workspace/wtime/target/performance/P014-run.log",
      "sha256": "f18168edf968846e1e95b22efa738560b47f0c22e31cf06e14fb2f05a991b56f"
    },
    {
      "bytes": 1420,
      "path": "/tmp/P014-tool-tests.log",
      "sha256": "a8658e24c977f28139c3f80dee12caa7447bf3029ff08f94e80befa26daf2d73"
    },
    {
      "bytes": 16932296,
      "path": "/workspace/wtime/target/performance/P014/binary/wtime-bench",
      "sha256": "4101e0338828224718ca96c056893a926a38dfef140af5d0776453885f0735f5"
    },
    {
      "bytes": 3594440,
      "path": "/workspace/wtime/target/performance/P014/binary/wtime-conformance",
      "sha256": "ed680b5d08c6a26ede7719e9ecfde2b2ee22f17610424fe71560da8803327191"
    },
    {
      "bytes": 16906344,
      "path": "/workspace/wtime/target/performance/P014/binary/wtime-bench-counters",
      "sha256": "72cb431d6200abbc7519fa0f143c14a884457c39d0203c3d3f45bdb090de7fe8"
    },
    {
      "bytes": 5550,
      "path": "/workspace/wtime/target/performance/P014/conformance/conformance/reference_types-reference.json",
      "sha256": "f5a951bcc833229adba2aa587764eaea919ce6e528c64d446f9ac3b757863b16"
    },
    {
      "bytes": 5544,
      "path": "/workspace/wtime/target/performance/P014/conformance/conformance/reference_types-decoded.json",
      "sha256": "ac3085250d16f865626e6eccf89ea201b5b855e4d8de3e114d2b150a21a97828"
    },
    {
      "bytes": 5626,
      "path": "/workspace/wtime/target/performance/P014/conformance/conformance/interactions-reference.json",
      "sha256": "8a6a820429001a2c3534df365c36f3765678d1902cccfea7ba8a72284111102e"
    },
    {
      "bytes": 5620,
      "path": "/workspace/wtime/target/performance/P014/conformance/conformance/interactions-decoded.json",
      "sha256": "204ebe1b0f8f78970cbb0aa38964a07765388e65c1eb032a9b6be77992dae139"
    },
    {
      "bytes": 88,
      "path": "/workspace/wtime/tools/wtime-bench/fixtures/call_overhead.wat",
      "sha256": "9493b8f751690a2fd6e8b19dcee5e2c1d50b717253d292ce34d5692b306663a5"
    },
    {
      "bytes": 40,
      "path": "/workspace/wtime/tools/wtime-bench/fixtures/call_overhead.wasm",
      "sha256": "0815ca7d6026206d0773fe10c6b9643862bfffe8ba361c9789369ef9668ee746"
    },
    {
      "bytes": 391,
      "path": "/workspace/wtime/tools/wtime-bench/fixtures/integer_loop.wat",
      "sha256": "791730112109f3d5751a7b3e2c1134e7e0e6f71a7f7ee01ea8481eb989ee3fa4"
    },
    {
      "bytes": 122,
      "path": "/workspace/wtime/tools/wtime-bench/fixtures/integer_loop.wasm",
      "sha256": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c"
    },
    {
      "bytes": 604,
      "path": "/workspace/wtime/tools/wtime-bench/fixtures/branch_calls.wat",
      "sha256": "d0af4fdf9ad5be38344267ae72fc1855edb5a8fa1b475fd0af4cb507730b2309"
    },
    {
      "bytes": 157,
      "path": "/workspace/wtime/tools/wtime-bench/fixtures/branch_calls.wasm",
      "sha256": "9b58d26f201900b5b6cce684133a98ab68b3cf55d979b64ce895e914e9d5327b"
    },
    {
      "bytes": 1488,
      "path": "/workspace/wtime/tools/wtime-bench/fixtures/integration/state_graph.wat",
      "sha256": "a2c4d3f0986f615828779cf256f15e2db425697e2240fecbd179e5b967612589"
    },
    {
      "bytes": 333,
      "path": "/workspace/wtime/target/performance/P014-state-fixtures/state_graph.wasm",
      "sha256": "5370ba21415ef9f2245caa51e73c86d8034b4ff294e4645c7dbb4701e15d36c2"
    },
    {
      "bytes": 412,
      "path": "/workspace/wtime/tools/wtime-bench/fixtures/integration/state_normal.wat",
      "sha256": "9e0278001f41ea1f2cc229b3ddc7c670f24d1f2860adb5c552fa613037f43717"
    },
    {
      "bytes": 138,
      "path": "/workspace/wtime/target/performance/P014-state-fixtures/state_normal.wasm",
      "sha256": "63cdccecaa341f9f2d8ef8ad8f779356549d3227b234e73117f6bdd33e612a9b"
    },
    {
      "bytes": 459,
      "path": "/workspace/wtime/tools/wtime-bench/fixtures/integration/state_trap.wat",
      "sha256": "99c3408b7697ad7184440d5d37593796b94a2ddf5c6369f0f27f61be5c1ba72d"
    },
    {
      "bytes": 148,
      "path": "/workspace/wtime/target/performance/P014-state-fixtures/state_trap.wasm",
      "sha256": "94155562450b9f3668eb31dacb751fe24d2180e68f30a88a91a1cc88cc5476b7"
    },
    {
      "bytes": 276,
      "path": "/workspace/wtime/crates/wtime-runtime/tests/fixtures/performance/cursor/store_successor.wat",
      "sha256": "366ad9a071ac01995edfcb5e7291c5bc3c3de4a33c7c52b63edfce3403d1fcbd"
    },
    {
      "bytes": 60,
      "path": "/workspace/wtime/crates/wtime-runtime/tests/fixtures/performance/cursor/store_successor.wasm",
      "sha256": "e05575761936f326eebf004bcfc6b015cb6dbf8ab14412aa7daae268cff3ce96"
    },
    {
      "bytes": 2534,
      "path": "/workspace/wtime/tools/wtime-bench/fixtures/integration/expected_state.json",
      "sha256": "126d72095766530e35a90ad864c8f90399c88c4bcac3bc039e02324bdd407ba9"
    }
  ],
  "baseline_tier": "reference",
  "binary_identities": {
    "benchmark": {
      "bytes": 16932296,
      "path": "/workspace/wtime/target/performance/P014/binary/wtime-bench",
      "sha256": "4101e0338828224718ca96c056893a926a38dfef140af5d0776453885f0735f5"
    },
    "conformance": {
      "bytes": 3594440,
      "path": "/workspace/wtime/target/performance/P014/binary/wtime-conformance",
      "sha256": "ed680b5d08c6a26ede7719e9ecfde2b2ee22f17610424fe71560da8803327191"
    },
    "counters": {
      "bytes": 16906344,
      "path": "/workspace/wtime/target/performance/P014/binary/wtime-bench-counters",
      "sha256": "72cb431d6200abbc7519fa0f143c14a884457c39d0203c3d3f45bdb090de7fe8"
    }
  },
  "build_commands": [
    {
      "argv": [
        "cargo",
        "build",
        "--locked",
        "--release",
        "--manifest-path",
        "tools/wtime-bench/Cargo.toml",
        "-j",
        "4",
        "--no-default-features",
        "--features",
        "execution-reference,runtime-only"
      ],
      "command": "cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml -j 4 --no-default-features --features execution-reference,runtime-only",
      "returncode": 0
    },
    {
      "argv": [
        "cargo",
        "build",
        "--locked",
        "--release",
        "-p",
        "wtime-conformance",
        "--no-default-features",
        "--features",
        "execution-reference,opcode-coverage"
      ],
      "command": "cargo build --locked --release -p wtime-conformance --no-default-features --features execution-reference,opcode-coverage",
      "returncode": 0
    },
    {
      "argv": [
        "cargo",
        "build",
        "--locked",
        "--release",
        "--manifest-path",
        "tools/wtime-bench/Cargo.toml",
        "-j",
        "4",
        "--no-default-features",
        "--features",
        "execution-reference,performance-counters,runtime-only",
        "--target-dir",
        "target/performance/P014/counters-build"
      ],
      "command": "cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml -j 4 --no-default-features --features execution-reference,performance-counters,runtime-only --target-dir target/performance/P014/counters-build",
      "returncode": 0
    }
  ],
  "check_breakdown": {
    "conformance_mode_runs": 4,
    "counter_diagnostic_cases": 3,
    "cursor_checks": 1,
    "fuel_probes": 68,
    "snapshot_roundtrip_checks": 1,
    "state_mode_runs": 6,
    "typed_mode_runs": 6
  },
  "commands": [
    "python3 tools/wtime-bench/tier_check.py --tier decoded --selection integration --output target/performance/P014/conformance",
    "python3 -m unittest discover -s tools/wtime-bench/tests -p 'test_p014*.py' -v",
    "cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml -j 4 --no-default-features --features execution-reference,runtime-only",
    "cargo build --locked --release -p wtime-conformance --no-default-features --features execution-reference,opcode-coverage",
    "cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml -j 4 --no-default-features --features execution-reference,performance-counters,runtime-only --target-dir target/performance/P014/counters-build",
    "/workspace/wtime/target/release/wtime-conformance run --mode reference --profile all-phase3-core --suite reference_types --filter all --report target/performance/P014/conformance/conformance/reference_types-reference.json",
    "/workspace/wtime/target/release/wtime-conformance run --mode decoded --profile all-phase3-core --suite reference_types --filter all --report target/performance/P014/conformance/conformance/reference_types-decoded.json",
    "/workspace/wtime/target/release/wtime-conformance run --mode reference --profile all-phase3-core --suite interactions --filter all --report target/performance/P014/conformance/conformance/interactions-reference.json",
    "/workspace/wtime/target/release/wtime-conformance run --mode decoded --profile all-phase3-core --suite interactions --filter all --report target/performance/P014/conformance/conformance/interactions-decoded.json"
  ],
  "conformance": {
    "runs": [
      {
        "compatibility_assertions": 0,
        "executed": 105,
        "executed_opcodes": [
          11,
          16,
          17,
          32,
          35,
          36,
          37,
          38,
          65,
          106,
          208,
          209,
          210
        ],
        "failed": 0,
        "mode": "reference",
        "report_sha256": "f5a951bcc833229adba2aa587764eaea919ce6e528c64d446f9ac3b757863b16",
        "selected": 105,
        "skipped": 0,
        "suite": "reference_types",
        "unsupported": 0
      },
      {
        "compatibility_assertions": 0,
        "executed": 105,
        "executed_opcodes": [
          11,
          16,
          17,
          32,
          35,
          36,
          37,
          38,
          65,
          106,
          208,
          209,
          210
        ],
        "failed": 0,
        "mode": "decoded",
        "report_sha256": "ac3085250d16f865626e6eccf89ea201b5b855e4d8de3e114d2b150a21a97828",
        "selected": 105,
        "skipped": 0,
        "suite": "reference_types",
        "unsupported": 0
      },
      {
        "compatibility_assertions": 0,
        "executed": 15,
        "executed_opcodes": [
          2,
          8,
          11,
          16,
          17,
          18,
          26,
          31,
          32,
          33,
          65,
          66,
          106,
          208,
          210,
          212,
          224,
          226,
          227,
          64001,
          64002,
          64003,
          64004,
          64256,
          64258,
          64288,
          64289,
          64522,
          64768,
          64779,
          64780,
          64795,
          65040,
          65047,
          65054
        ],
        "failed": 0,
        "mode": "reference",
        "report_sha256": "8a6a820429001a2c3534df365c36f3765678d1902cccfea7ba8a72284111102e",
        "selected": 15,
        "skipped": 0,
        "suite": "interactions",
        "unsupported": 0
      },
      {
        "compatibility_assertions": 0,
        "executed": 15,
        "executed_opcodes": [
          2,
          8,
          11,
          16,
          17,
          18,
          26,
          31,
          32,
          33,
          65,
          66,
          106,
          208,
          210,
          212,
          224,
          226,
          227,
          64001,
          64002,
          64003,
          64004,
          64256,
          64258,
          64288,
          64289,
          64522,
          64768,
          64779,
          64780,
          64795,
          65040,
          65047,
          65054
        ],
        "failed": 0,
        "mode": "decoded",
        "report_sha256": "204ebe1b0f8f78970cbb0aa38964a07765388e65c1eb032a9b6be77992dae139",
        "selected": 15,
        "skipped": 0,
        "suite": "interactions",
        "unsupported": 0
      }
    ],
    "source_hashes_after": {
      ".cache/spec/test/core/ref_func.wast": "ff3d74f5e997d7a71a41d931235c345567136802a9f643f26872013242386d5f",
      ".cache/spec/test/core/ref_is_null.wast": "876a66f7cd8cce163d62a2e8c2c4023a9aaf83c0b502f01521c7e689ee11d94f",
      ".cache/spec/test/core/ref_null.wast": "63330c3c5f1bd0decb7473ba0c73378ce685edb16ce24714b2f7fab05ae68529",
      ".cache/spec/test/core/table_get.wast": "30c289fd0d88affa4750387cd7ef300aa92543ec7e3670d74babe2f92644a97e",
      ".cache/spec/test/core/table_set.wast": "8d41c2420113cc1e9fb04f30b4fd7b67acccbbd7f95d8c5bcc058091b6ed52ce",
      "docs/architecture/corpus-lock.json": "a2560d023c8941ee8191d1213a0efdc5e0402983b9345cf651bfeaa316775f71",
      "tests/fixtures/interactions/captured-descriptor.wast": "0874fc54994b1a032afcd6d3a394307e9bda7ef65e2b705c1cde967ac59e9ca7",
      "tests/fixtures/interactions/exact-compact.wast": "20d32f07469c70cb366d56c19c0fa82460073c2e8e24f3c554322e734524004e",
      "tests/fixtures/interactions/opcode-coverage.wast": "4cc48d66f0b16dfe92942a48bcc90e3c1bf528e7b5cf383290d23f0b90510012",
      "tests/fixtures/interactions/shared-wide-simd.wast": "31afad7581f2562c7110dfa6b4513762e42576023c0bb72b02d37c873309149e"
    },
    "source_hashes_before": {
      ".cache/spec/test/core/ref_func.wast": "ff3d74f5e997d7a71a41d931235c345567136802a9f643f26872013242386d5f",
      ".cache/spec/test/core/ref_is_null.wast": "876a66f7cd8cce163d62a2e8c2c4023a9aaf83c0b502f01521c7e689ee11d94f",
      ".cache/spec/test/core/ref_null.wast": "63330c3c5f1bd0decb7473ba0c73378ce685edb16ce24714b2f7fab05ae68529",
      ".cache/spec/test/core/table_get.wast": "30c289fd0d88affa4750387cd7ef300aa92543ec7e3670d74babe2f92644a97e",
      ".cache/spec/test/core/table_set.wast": "8d41c2420113cc1e9fb04f30b4fd7b67acccbbd7f95d8c5bcc058091b6ed52ce",
      "docs/architecture/corpus-lock.json": "a2560d023c8941ee8191d1213a0efdc5e0402983b9345cf651bfeaa316775f71",
      "tests/fixtures/interactions/captured-descriptor.wast": "0874fc54994b1a032afcd6d3a394307e9bda7ef65e2b705c1cde967ac59e9ca7",
      "tests/fixtures/interactions/exact-compact.wast": "20d32f07469c70cb366d56c19c0fa82460073c2e8e24f3c554322e734524004e",
      "tests/fixtures/interactions/opcode-coverage.wast": "4cc48d66f0b16dfe92942a48bcc90e3c1bf528e7b5cf383290d23f0b90510012",
      "tests/fixtures/interactions/shared-wide-simd.wast": "31afad7581f2562c7110dfa6b4513762e42576023c0bb72b02d37c873309149e"
    },
    "suite_summary": {
      "interactions": {
        "executed_by_mode": {
          "decoded": 15,
          "reference": 15
        },
        "failed_by_mode": {
          "decoded": 0,
          "reference": 0
        },
        "modes": [
          "reference",
          "decoded"
        ],
        "unique_executed_opcodes": [
          2,
          8,
          11,
          16,
          17,
          18,
          26,
          31,
          32,
          33,
          65,
          66,
          106,
          208,
          210,
          212,
          224,
          226,
          227,
          64001,
          64002,
          64003,
          64004,
          64256,
          64258,
          64288,
          64289,
          64522,
          64768,
          64779,
          64780,
          64795,
          65040,
          65047,
          65054
        ]
      },
      "reference_types": {
        "executed_by_mode": {
          "decoded": 105,
          "reference": 105
        },
        "failed_by_mode": {
          "decoded": 0,
          "reference": 0
        },
        "modes": [
          "reference",
          "decoded"
        ],
        "unique_executed_opcodes": [
          11,
          16,
          17,
          32,
          35,
          36,
          37,
          38,
          65,
          106,
          208,
          209,
          210
        ]
      }
    },
    "total_executed": 240,
    "total_failed": 0,
    "unique_executed_opcode_union": [
      2,
      8,
      11,
      16,
      17,
      18,
      26,
      31,
      32,
      33,
      35,
      36,
      37,
      38,
      65,
      66,
      106,
      208,
      209,
      210,
      212,
      224,
      226,
      227,
      64001,
      64002,
      64003,
      64004,
      64256,
      64258,
      64288,
      64289,
      64522,
      64768,
      64779,
      64780,
      64795,
      65040,
      65047,
      65054
    ]
  },
  "conformance_binary_identity": {
    "path": "target/release/wtime-conformance",
    "sha256": "ed680b5d08c6a26ede7719e9ecfde2b2ee22f17610424fe71560da8803327191"
  },
  "counts": {
    "all_checks_attempted": 89,
    "all_checks_executed": 89,
    "all_checks_expected": 89,
    "conformance_mode_runs_executed": 4,
    "conformance_mode_runs_expected": 4,
    "conformance_suites": 2,
    "cursor_checks_executed": 1,
    "failed": 0,
    "mode_runs_executed": 6,
    "mode_runs_expected": 6,
    "selected_cases": 3,
    "semantic_cases": 3,
    "skipped": 0,
    "state_cases": 3,
    "state_fuel_probes_executed": 68,
    "state_fuel_probes_expected": 68,
    "state_mode_runs_executed": 6,
    "state_mode_runs_expected": 6,
    "unsupported": 0
  },
  "cursor_checks": [
    {
      "case": "state_normal",
      "charged": true,
      "prepared_pc": 3,
      "remaining_fuel": 99996,
      "source_pc": 3
    }
  ],
  "diagnostic_counters": {
    "aggregate": {
      "argument_allocations": 5003,
      "branch_result_allocations": 10003,
      "continuation_scan_entries": 0,
      "frame_entries": 5003,
      "instruction_clone_bytes": 0,
      "instruction_clone_events": 277524,
      "module_deep_copy_bytes": 0,
      "module_deep_copy_events": 0,
      "read_fixed_operations": 0,
      "read_vec_operations": 0,
      "root_snapshot_allocations": 0,
      "source_dispatches": 277524,
      "subtype_probes": 0
    },
    "per_case": [
      {
        "binary_sha256": "72cb431d6200abbc7519fa0f143c14a884457c39d0203c3d3f45bdb090de7fe8",
        "case": "call_overhead",
        "counters": {
          "argument_allocations": 1,
          "branch_result_allocations": 1,
          "continuation_scan_entries": 0,
          "frame_entries": 1,
          "instruction_clone_bytes": 0,
          "instruction_clone_events": 4,
          "module_deep_copy_bytes": 0,
          "module_deep_copy_events": 0,
          "read_fixed_operations": 0,
          "read_vec_operations": 0,
          "root_snapshot_allocations": 0,
          "source_dispatches": 4,
          "subtype_probes": 0
        },
        "mode": "decoded",
        "source_dispatches": 4
      },
      {
        "binary_sha256": "72cb431d6200abbc7519fa0f143c14a884457c39d0203c3d3f45bdb090de7fe8",
        "case": "integer_loop",
        "counters": {
          "argument_allocations": 1,
          "branch_result_allocations": 1,
          "continuation_scan_entries": 0,
          "frame_entries": 1,
          "instruction_clone_bytes": 0,
          "instruction_clone_events": 150010,
          "module_deep_copy_bytes": 0,
          "module_deep_copy_events": 0,
          "read_fixed_operations": 0,
          "read_vec_operations": 0,
          "root_snapshot_allocations": 0,
          "source_dispatches": 150010,
          "subtype_probes": 0
        },
        "mode": "decoded",
        "source_dispatches": 150010
      },
      {
        "binary_sha256": "72cb431d6200abbc7519fa0f143c14a884457c39d0203c3d3f45bdb090de7fe8",
        "case": "branch_calls",
        "counters": {
          "argument_allocations": 5001,
          "branch_result_allocations": 10001,
          "continuation_scan_entries": 0,
          "frame_entries": 5001,
          "instruction_clone_bytes": 0,
          "instruction_clone_events": 127510,
          "module_deep_copy_bytes": 0,
          "module_deep_copy_events": 0,
          "read_fixed_operations": 0,
          "read_vec_operations": 0,
          "root_snapshot_allocations": 0,
          "source_dispatches": 127510,
          "subtype_probes": 0
        },
        "mode": "decoded",
        "source_dispatches": 127510
      }
    ]
  },
  "diagnostic_instrumentation": {
    "binary": "target/performance/P014/counters-build/release/wtime-bench",
    "binary_sha256": "72cb431d6200abbc7519fa0f143c14a884457c39d0203c3d3f45bdb090de7fe8",
    "enabled": true,
    "feature": "performance-counters"
  },
  "feature_sets": {
    "benchmark_harness": [
      "runtime-only"
    ],
    "conformance_instrumentation": [
      "opcode-coverage"
    ],
    "conformance_policy": [
      "execution-reference"
    ],
    "diagnostic_instrumentation": [
      "performance-counters"
    ],
    "policy": [
      "execution-reference"
    ]
  },
  "fuel_boundaries": {
    "state_normal": {
      "boundary_verified": true,
      "max_probe_fuel": 16,
      "minimal_success_fuel": {
        "decoded": 10,
        "reference": 10
      },
      "previous_fuel_outcome": {
        "decoded": "ResourceLimit",
        "reference": "ResourceLimit"
      }
    },
    "state_trap": {
      "max_probe_fuel": 16,
      "minimal_expected_trap_fuel": {
        "decoded": 8,
        "reference": 8
      },
      "trap_code": "MemoryOutOfBounds"
    }
  },
  "fuel_outcome_histogram": {
    "state_normal/decoded": {
      "ResourceLimit": 10,
      "returned": 7
    },
    "state_normal/reference": {
      "ResourceLimit": 10,
      "returned": 7
    },
    "state_trap/decoded": {
      "ResourceLimit": 8,
      "Trap": 9
    },
    "state_trap/reference": {
      "ResourceLimit": 8,
      "Trap": 9
    }
  },
  "instrumented": true,
  "limitations": [
    "The reference dispatcher shares admission, frames, numeric/memory/GC/continuation helpers; it is not an independent oracle for bugs in shared helpers.",
    "P018 snapshots are bounded diagnostic views and are checked only in decoded mode; P022 supplies broader interaction-state fixtures.",
    "This is a correctness check. Execution timings in typed protocol output are diagnostic only and are not performance evidence.",
    "The report-level instrumented=true and execution-reference feature identify a development differential build; typed first_call timings are diagnostic only, not benchmark performance evidence.",
    "The performance-counters data comes from a separate feature-enabled executable and is diagnostic instrumentation, not a timing result.",
    "P014 measurements and checks bind the report-time workload registry hash; the checked-in registry has since changed, so its present bytes must not be represented as the original registry.",
    "No speedup, representative application performance, or independent-oracle claim follows from this correctness run."
  ],
  "modes": [
    "decoded",
    "portable",
    "reference"
  ],
  "outcome": "contract_only",
  "passed": true,
  "report": {
    "bytes": 94199,
    "path": "/workspace/wtime/target/performance/P014/conformance/report.json",
    "sha256": "b7339dd2c47982e551ed700f26dd141415ef1b1ba00ccdc225b0ff52a2b02e91"
  },
  "report_binary_identity": {
    "path": "tools/wtime-bench/target/release/wtime-bench",
    "sha256": "4101e0338828224718ca96c056893a926a38dfef140af5d0776453885f0735f5"
  },
  "requested_tier": "decoded",
  "run_log": {
    "bytes": 122,
    "path": "/workspace/wtime/target/performance/P014-run.log",
    "sha256": "f18168edf968846e1e95b22efa738560b47f0c22e31cf06e14fb2f05a991b56f"
  },
  "selection": "integration",
  "snapshot": {
    "binary_sha256": "4101e0338828224718ca96c056893a926a38dfef140af5d0776453885f0735f5",
    "case": "store_successor",
    "function_body": 0,
    "materialization_roundtrips": 1,
    "pending_boundary_charged": true,
    "prepared_pc": 3,
    "remaining_fuel": null,
    "schema_version": 1,
    "source_pc": 3
  },
  "source_binding": {
    "conformance_source_count": 10,
    "conformance_sources_after": {
      ".cache/spec/test/core/ref_func.wast": "ff3d74f5e997d7a71a41d931235c345567136802a9f643f26872013242386d5f",
      ".cache/spec/test/core/ref_is_null.wast": "876a66f7cd8cce163d62a2e8c2c4023a9aaf83c0b502f01521c7e689ee11d94f",
      ".cache/spec/test/core/ref_null.wast": "63330c3c5f1bd0decb7473ba0c73378ce685edb16ce24714b2f7fab05ae68529",
      ".cache/spec/test/core/table_get.wast": "30c289fd0d88affa4750387cd7ef300aa92543ec7e3670d74babe2f92644a97e",
      ".cache/spec/test/core/table_set.wast": "8d41c2420113cc1e9fb04f30b4fd7b67acccbbd7f95d8c5bcc058091b6ed52ce",
      "docs/architecture/corpus-lock.json": "a2560d023c8941ee8191d1213a0efdc5e0402983b9345cf651bfeaa316775f71",
      "tests/fixtures/interactions/captured-descriptor.wast": "0874fc54994b1a032afcd6d3a394307e9bda7ef65e2b705c1cde967ac59e9ca7",
      "tests/fixtures/interactions/exact-compact.wast": "20d32f07469c70cb366d56c19c0fa82460073c2e8e24f3c554322e734524004e",
      "tests/fixtures/interactions/opcode-coverage.wast": "4cc48d66f0b16dfe92942a48bcc90e3c1bf528e7b5cf383290d23f0b90510012",
      "tests/fixtures/interactions/shared-wide-simd.wast": "31afad7581f2562c7110dfa6b4513762e42576023c0bb72b02d37c873309149e"
    },
    "conformance_sources_before": {
      ".cache/spec/test/core/ref_func.wast": "ff3d74f5e997d7a71a41d931235c345567136802a9f643f26872013242386d5f",
      ".cache/spec/test/core/ref_is_null.wast": "876a66f7cd8cce163d62a2e8c2c4023a9aaf83c0b502f01521c7e689ee11d94f",
      ".cache/spec/test/core/ref_null.wast": "63330c3c5f1bd0decb7473ba0c73378ce685edb16ce24714b2f7fab05ae68529",
      ".cache/spec/test/core/table_get.wast": "30c289fd0d88affa4750387cd7ef300aa92543ec7e3670d74babe2f92644a97e",
      ".cache/spec/test/core/table_set.wast": "8d41c2420113cc1e9fb04f30b4fd7b67acccbbd7f95d8c5bcc058091b6ed52ce",
      "docs/architecture/corpus-lock.json": "a2560d023c8941ee8191d1213a0efdc5e0402983b9345cf651bfeaa316775f71",
      "tests/fixtures/interactions/captured-descriptor.wast": "0874fc54994b1a032afcd6d3a394307e9bda7ef65e2b705c1cde967ac59e9ca7",
      "tests/fixtures/interactions/exact-compact.wast": "20d32f07469c70cb366d56c19c0fa82460073c2e8e24f3c554322e734524004e",
      "tests/fixtures/interactions/opcode-coverage.wast": "4cc48d66f0b16dfe92942a48bcc90e3c1bf528e7b5cf383290d23f0b90510012",
      "tests/fixtures/interactions/shared-wide-simd.wast": "31afad7581f2562c7110dfa6b4513762e42576023c0bb72b02d37c873309149e"
    },
    "conformance_sources_stable": true,
    "current_workloads_json_changed_since_report": true,
    "fixture_identities_after": {
      "p018_cursor": {
        "wasm": "crates/wtime-runtime/tests/fixtures/performance/cursor/store_successor.wasm",
        "wasm_sha256": "e05575761936f326eebf004bcfc6b015cb6dbf8ab14412aa7daae268cff3ce96",
        "wat": "crates/wtime-runtime/tests/fixtures/performance/cursor/store_successor.wat",
        "wat_sha256": "366ad9a071ac01995edfcb5e7291c5bc3c3de4a33c7c52b63edfce3403d1fcbd"
      },
      "semantic_cases": {
        "branch_calls": {
          "wasm": "tools/wtime-bench/fixtures/branch_calls.wasm",
          "wasm_sha256": "9b58d26f201900b5b6cce684133a98ab68b3cf55d979b64ce895e914e9d5327b",
          "wat": "tools/wtime-bench/fixtures/branch_calls.wat",
          "wat_sha256": "d0af4fdf9ad5be38344267ae72fc1855edb5a8fa1b475fd0af4cb507730b2309"
        },
        "call_overhead": {
          "wasm": "tools/wtime-bench/fixtures/call_overhead.wasm",
          "wasm_sha256": "0815ca7d6026206d0773fe10c6b9643862bfffe8ba361c9789369ef9668ee746",
          "wat": "tools/wtime-bench/fixtures/call_overhead.wat",
          "wat_sha256": "9493b8f751690a2fd6e8b19dcee5e2c1d50b717253d292ce34d5692b306663a5"
        },
        "integer_loop": {
          "wasm": "tools/wtime-bench/fixtures/integer_loop.wasm",
          "wasm_sha256": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c",
          "wat": "tools/wtime-bench/fixtures/integer_loop.wat",
          "wat_sha256": "791730112109f3d5751a7b3e2c1134e7e0e6f71a7f7ee01ea8481eb989ee3fa4"
        }
      },
      "state_cases": {
        "state_graph": {
          "wasm": "target/performance/P014-state-fixtures/state_graph.wasm",
          "wasm_sha256": "5370ba21415ef9f2245caa51e73c86d8034b4ff294e4645c7dbb4701e15d36c2",
          "wat": "tools/wtime-bench/fixtures/integration/state_graph.wat",
          "wat_sha256": "a2c4d3f0986f615828779cf256f15e2db425697e2240fecbd179e5b967612589"
        },
        "state_normal": {
          "wasm": "target/performance/P014-state-fixtures/state_normal.wasm",
          "wasm_sha256": "63cdccecaa341f9f2d8ef8ad8f779356549d3227b234e73117f6bdd33e612a9b",
          "wat": "tools/wtime-bench/fixtures/integration/state_normal.wat",
          "wat_sha256": "9e0278001f41ea1f2cc229b3ddc7c670f24d1f2860adb5c552fa613037f43717"
        },
        "state_trap": {
          "wasm": "target/performance/P014-state-fixtures/state_trap.wasm",
          "wasm_sha256": "94155562450b9f3668eb31dacb751fe24d2180e68f30a88a91a1cc88cc5476b7",
          "wat": "tools/wtime-bench/fixtures/integration/state_trap.wat",
          "wat_sha256": "99c3408b7697ad7184440d5d37593796b94a2ddf5c6369f0f27f61be5c1ba72d"
        }
      }
    },
    "fixture_identities_before": {
      "p018_cursor": {
        "wasm": "crates/wtime-runtime/tests/fixtures/performance/cursor/store_successor.wasm",
        "wasm_sha256": "e05575761936f326eebf004bcfc6b015cb6dbf8ab14412aa7daae268cff3ce96",
        "wat": "crates/wtime-runtime/tests/fixtures/performance/cursor/store_successor.wat",
        "wat_sha256": "366ad9a071ac01995edfcb5e7291c5bc3c3de4a33c7c52b63edfce3403d1fcbd"
      },
      "semantic_cases": {
        "branch_calls": {
          "wasm": "tools/wtime-bench/fixtures/branch_calls.wasm",
          "wasm_sha256": "9b58d26f201900b5b6cce684133a98ab68b3cf55d979b64ce895e914e9d5327b",
          "wat": "tools/wtime-bench/fixtures/branch_calls.wat",
          "wat_sha256": "d0af4fdf9ad5be38344267ae72fc1855edb5a8fa1b475fd0af4cb507730b2309"
        },
        "call_overhead": {
          "wasm": "tools/wtime-bench/fixtures/call_overhead.wasm",
          "wasm_sha256": "0815ca7d6026206d0773fe10c6b9643862bfffe8ba361c9789369ef9668ee746",
          "wat": "tools/wtime-bench/fixtures/call_overhead.wat",
          "wat_sha256": "9493b8f751690a2fd6e8b19dcee5e2c1d50b717253d292ce34d5692b306663a5"
        },
        "integer_loop": {
          "wasm": "tools/wtime-bench/fixtures/integer_loop.wasm",
          "wasm_sha256": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c",
          "wat": "tools/wtime-bench/fixtures/integer_loop.wat",
          "wat_sha256": "791730112109f3d5751a7b3e2c1134e7e0e6f71a7f7ee01ea8481eb989ee3fa4"
        }
      },
      "state_cases": {
        "state_graph": {
          "wasm": "target/performance/P014-state-fixtures/state_graph.wasm",
          "wasm_sha256": "5370ba21415ef9f2245caa51e73c86d8034b4ff294e4645c7dbb4701e15d36c2",
          "wat": "tools/wtime-bench/fixtures/integration/state_graph.wat",
          "wat_sha256": "a2c4d3f0986f615828779cf256f15e2db425697e2240fecbd179e5b967612589"
        },
        "state_normal": {
          "wasm": "target/performance/P014-state-fixtures/state_normal.wasm",
          "wasm_sha256": "63cdccecaa341f9f2d8ef8ad8f779356549d3227b234e73117f6bdd33e612a9b",
          "wat": "tools/wtime-bench/fixtures/integration/state_normal.wat",
          "wat_sha256": "9e0278001f41ea1f2cc229b3ddc7c670f24d1f2860adb5c552fa613037f43717"
        },
        "state_trap": {
          "wasm": "target/performance/P014-state-fixtures/state_trap.wasm",
          "wasm_sha256": "94155562450b9f3668eb31dacb751fe24d2180e68f30a88a91a1cc88cc5476b7",
          "wat": "tools/wtime-bench/fixtures/integration/state_trap.wat",
          "wat_sha256": "99c3408b7697ad7184440d5d37593796b94a2ddf5c6369f0f27f61be5c1ba72d"
        }
      }
    },
    "fixture_identities_stable": true,
    "implementation_source_count": 87,
    "implementation_sources": {
      "Cargo.lock": "9d704c9854b30ca231109a54598c1c440a7d144b8ece5e21539971fead240820",
      "Cargo.toml": "9656851608b4f5ecb5f973a5ad370c67aef45fc557d4c132469bbf012662efff",
      "crates/wtime-decode/Cargo.toml": "1ae571b0eb1a293cd0ef6a181e33d267c40b228cd1afd5dd44ef9a309d113fa1",
      "crates/wtime-decode/src/instructions.rs": "d352215a69fc6d33b188850974c364023510056c347d0ecf95ce9ba96cd05168",
      "crates/wtime-decode/src/lib.rs": "9a83e26510e41a1d73556ebc74f4402f0aecb3493068dd7f66d4851d451524d9",
      "crates/wtime-decode/src/metadata.rs": "e9b8d4d88fc5b34dccd0a1918346cfed3a03dd256129dca4c62f4e0bce8d036e",
      "crates/wtime-decode/src/module.rs": "51b8fb2669a8323f36c7b199847a002f5b751dc04a382584a7112d7dd7dde71d",
      "crates/wtime-decode/src/reader.rs": "0c061e8df82b02c1d0fb25ad2744991a2a8f0db7893ae5d5818ba89772651e6e",
      "crates/wtime-runtime/Cargo.toml": "2ee79daf9485c056793d57b1886f75e0e74b12a6ddbf69f4b18f20ecffe8171b",
      "crates/wtime-runtime/examples/embedding.rs": "d9baac3b51bacbee00c42a109831e8bd1a845158b5a0ceae46a911307d8b0907",
      "crates/wtime-runtime/examples/suspension.rs": "097175e5cbbcf1be4716d4c1b584702042558d299360392fa742a85ead331e1d",
      "crates/wtime-runtime/src/arena.rs": "cd929b222d1ccf871f06684510df6767a1e69af4d338458c0e98b88cd16c8441",
      "crates/wtime-runtime/src/canonical.rs": "0bed54a47fbca8a39c7359a9140a5284f0513a6744505ddb5a0b92d531d0776d",
      "crates/wtime-runtime/src/continuation.rs": "c30b6fb4b89999b2e6f494b9258ce9ea3ea1e39a5971643b43147026e0fc8f46",
      "crates/wtime-runtime/src/coverage.rs": "fa735d933bf1334d23c262aac2d35282a743a49aaddb0c2bbc68da4c308f3fbe",
      "crates/wtime-runtime/src/exceptions.rs": "08e306b7f60b25077f31706bf55cc4b521b0b2b713a61e8656e36fcc34c326ba",
      "crates/wtime-runtime/src/execution.rs": "1baf160513d2daf53706f2bc03a54f7d01777b75da65656c5a6bedb4e995f4a5",
      "crates/wtime-runtime/src/execution/accounting.rs": "c86761a09e3c98301f54c971aa93e08da97140832c0f58e279aca4a75bef7127",
      "crates/wtime-runtime/src/execution/cursor.rs": "ff93a3f0056d109366d2b77ea93d459f456d048c05b4bf05369305a3b491d3d3",
      "crates/wtime-runtime/src/execution/effects.rs": "640bf290be1691465e44f188213b644030dab6f8ca1599790f5138816e192bcd",
      "crates/wtime-runtime/src/execution/plan.rs": "68e43e6484a6c198529b333a6e5122526845544d660191121755c3108e125d7e",
      "crates/wtime-runtime/src/gc.rs": "7dcfcc9aaff6a333041d9d8a7323d1ce0fad90b382b5023e9caeeb52815b3471",
      "crates/wtime-runtime/src/gc_exec.rs": "56cf9173994d267fa4ead0af137b8f9f941f6b86a52d458f31be3ce7a32280fc",
      "crates/wtime-runtime/src/lib.rs": "997a0ec8d835c83567e13aa4c755ed30ad0e44a0bdb9e689c14753abdd47bb45",
      "crates/wtime-runtime/src/linker.rs": "39a2068993bcb65fb4434f42d02b56687e8ae31914840487ee4c35de07677806",
      "crates/wtime-runtime/src/numeric.rs": "5acd2cc6a4e9e9c23fd4bb25ab8a9fbc1fa1658d63f4a95e207a00963650dc92",
      "crates/wtime-runtime/src/perf.rs": "f523ab7bc803da4fdcd5365c7c310b572ff7704252c245530e37a21ac740f59b",
      "crates/wtime-runtime/src/reference.rs": "5afabf5f234dd6339a84be2440794ef0e181891f801d78a65a5703ccb304d7f9",
      "crates/wtime-runtime/src/shared_memory.rs": "6b1a5a02e49d2d73ee04865af8d6bac68c60305f1afc754ca9b70252750bce44",
      "crates/wtime-runtime/src/simd.rs": "84c223e6a546aa8955ff5214b9add03df4c94d25f3ffcd45b33a808d299334b1",
      "crates/wtime-runtime/src/wide.rs": "e0347c76d5a00a56396bd641ce4beb4f0d6a7500739f6de57283c1cbacc4224a",
      "crates/wtime-runtime/tests/core.rs": "54f6e18cff27f3da96ae0736d7910013883742f0215d35abb551ce324c78af1f",
      "crates/wtime-runtime/tests/exceptions.rs": "3cc47afeeb6428154f6d0a30db6362a009c45d292278f5f03ca60c53c3342a93",
      "crates/wtime-runtime/tests/feature_profiles.rs": "dafe483c28f8683332d198935818fff885827dfc5cfd1ad62f27330490ebc14a",
      "crates/wtime-runtime/tests/performance.rs": "e1a13a992b35c6e52fb13db92e10a0ea921d410f94a51f58ee36b8d3436ed58d",
      "crates/wtime-runtime/tests/performance/p016.rs": "5ff37ef565ffc5c64c743723f1bfdabe85a315e784f1c81384f17b48a71dc21f",
      "crates/wtime-runtime/tests/performance/p017.rs": "4122d30ef0f042af886692d52de26c9a48d016f5f8e2a0bf67c6e9c9b84e9ef7",
      "crates/wtime-runtime/tests/performance/p018.rs": "e06da12e7539dbfc36bdfcc1981e87c8d926b08679f9bd2fce201987d5dafeae",
      "crates/wtime-runtime/tests/performance/p019.rs": "8294310dd4da5f9a7a05bf0bfee0d2be503ec52cdddbca4e21c6a3ee9d3b2715",
      "crates/wtime-runtime/tests/performance/p020.rs": "b6cb36f79e0d9702e7a8be4f81d254fcbd9d683d57c6baefe37f06951f611185",
      "crates/wtime-runtime/tests/performance/p021.rs": "fec19c012536565dbf2ba3e3f80f26b3fcc4e7d560a2218b5335552be2448a94",
      "crates/wtime-runtime/tests/performance/p023.rs": "6cbadd3bf64068c0ba819bed3be369c1334fb876075e9f3253c9d612666472f6",
      "crates/wtime-runtime/tests/proposals.rs": "8fffaa52d03e81fc7d88d0970617f1a0c8ba7602fc241ad9c65367133d2a9927",
      "crates/wtime-types/Cargo.toml": "e5c4d20d5d35fd4a15ce854f96602add82b86a1bbe4599aab69b0abcc7dc9031",
      "crates/wtime-types/src/error.rs": "a25659ee8aefcf08babfd2e4b027b698176adc100da5336a9a28f2f0664d8a2c",
      "crates/wtime-types/src/features.rs": "3c465ce7c325c9110d42a6df08ef20dfc90122e8a86a2fdbb85a6063e82b2dc5",
      "crates/wtime-types/src/indices.rs": "249359a286195771d58ad78daa775c8164de6419100fea38e6f6f172f92e2e40",
      "crates/wtime-types/src/lib.rs": "9f1ce2f00f37fba42288e5f4c78262234bf6ab453f7061d7b4907bbe4fdeca4d",
      "crates/wtime-types/src/types.rs": "d5d6a398f344494ada0d065afcab4bdee157f4803351a2ce6752ee3fc0d4e266",
      "crates/wtime-validate/Cargo.toml": "fed963beb5aa95d1c242f4131509edd95069e1b27c4e69f84f969357a04a0639",
      "crates/wtime-validate/src/acceptance.rs": "322c98e589f396f77097a7acf1dabc666f72ccbba8936703cb115ed7befd9621",
      "crates/wtime-validate/src/atomics.rs": "51d71f78adc9f140c79dad601afe64f4bc42fb2cf896a1d753a480f57f70bbc4",
      "crates/wtime-validate/src/continuations.rs": "185cf7f137c9dbd57062f95f3d36715e92420a44f5579fb09a34b7dd65029969",
      "crates/wtime-validate/src/exceptions.rs": "5afe91db3e03444019a0181f639f683c8969563ac20376c6485395c79f46c99b",
      "crates/wtime-validate/src/gc.rs": "e46a4440d3ffbea02c4905ffffa75e9582c3193697a8484ec464fdaa7a34f21f",
      "crates/wtime-validate/src/lib.rs": "6d6fd6442fdad200c494fc43775dc003ee18fbfa63632c7056e238777a283455",
      "crates/wtime-validate/src/types.rs": "2d6f8561b6d9d8eeff0bbb76510e9b9a6159d9646d7c01a01fda6f51207ec7a0",
      "crates/wtime-validate/tests/exceptions.rs": "061d15fe56d6028e54671c5dd5d86ee1069f2ac99183886a3c3441356d4da290",
      "crates/wtime-validate/tests/policy.rs": "4a4c47509bd9da3850b1ab76235d585db2c7aba826f1f89ec9055a2d888774c3",
      "tools/wtime-bench/Cargo.lock": "4d1a59eab7ae2ce8c6abcd203afb3a20d0787e6cd536b86a919fd133196c5431",
      "tools/wtime-bench/Cargo.toml": "389e92491b6caca59c924bd41a46bee79828b41459f2c3712411e3e40bcb5a3b",
      "tools/wtime-bench/fixtures/integration/expected_state.json": "126d72095766530e35a90ad864c8f90399c88c4bcac3bc039e02324bdd407ba9",
      "tools/wtime-bench/run.py": "f793c150ae38031467c91a70cddf66e85b6cd6d9782e227b773b4da6ff564bea",
      "tools/wtime-bench/src/continuation_driver.rs": "2cc5818b256463ef062ad1f36f8a858e3905ce961ffb8b56bae06b143965119a",
      "tools/wtime-bench/src/gc_exception_driver.rs": "63c192dd2f0f5a376dba96b66ac3e55c40303512374b02fe96b782e33ace369f",
      "tools/wtime-bench/src/main.rs": "9f9496929d3c60165fa061030336397d1825d9b0545a70aadeaf07208c3768e5",
      "tools/wtime-bench/src/memory_driver.rs": "1d3b6604fd98b2397906dbd085bf1bd0f5eadb79fde230dbaf22339b3690bc7c",
      "tools/wtime-bench/src/protocol.rs": "da72d0c29646d42cf0088c705371a6220a68832a27a95b181b32c7640b667901",
      "tools/wtime-bench/src/scalar_driver.rs": "c43a3369b6dfac8b57aaf789dbabd9da8dab4f28751571a683b970dd09225a58",
      "tools/wtime-bench/src/thread_driver.rs": "96801dbc985a214e335c2be69dedce48a1cd2acf37055547f3d5fc87a21f4ee3",
      "tools/wtime-bench/src/tier_state_driver.rs": "6215becc3a74f02bbea2dbb8d342afe879bd9b9787d2c3c777fea9b871678525",
      "tools/wtime-bench/tier_check.py": "680766b24821f50b47bfa43c8e1c757020b80257f962dd2ae2c06cdc37de96a5",
      "tools/wtime-bench/workloads.json": "37633e7b0fd99c729b74d408da76a2bb329e6ffb230c1bc75782a4af57b086c9",
      "tools/wtime-bench/workloads.py": "03e7e3370e5f1463361577f28612b184e1b02e64006f74be3ad8167133d907d8",
      "tools/wtime-conformance/Cargo.toml": "37c21479a09390a590f0e0ccba31e62ebfddf4096fa3c16039042af9c223c68e",
      "tools/wtime-conformance/build.rs": "1e37a8f6ec1af7964228b03abb3e9675c2c5218edf3bb43a94d76243cc893ef1",
      "tools/wtime-conformance/src/adapter.rs": "b99ae377d141ca6369569cbb1960f00910cc8d0b92fd5007ced206f4ebe0f997",
      "tools/wtime-conformance/src/bin/wtime-allocation-check.rs": "1a20e70ac081c7792732bc1cffded19935e19d0141bca5ea78a6b6086f4c6034",
      "tools/wtime-conformance/src/bin/wtime-probe.rs": "039e380652f8ef356d0995c35a7f14b289479db40f59e9f88d6ccfa9ef9df785",
      "tools/wtime-conformance/src/cli.rs": "ab8466680604fc36dc5e1aafbe06d7fbecd8e1cb1f5ad321099cb3a5094017ae",
      "tools/wtime-conformance/src/corpus.rs": "b1d2a7947b5904b9bd80c0109bc8b5e0f5f8329ba92a693db3f4409cd93cbc1e",
      "tools/wtime-conformance/src/main.rs": "ecf37373cd8988e4f45569d49a0a971fc85a5b687c091ae2ec7964ec8f35c169",
      "tools/wtime-conformance/src/report.rs": "a5bb757ec60513a95f0b9573328a9ee77159a2e061741bb695b52c1fef4348f5",
      "tools/wtime-conformance/src/script.rs": "8c11af05587c584895b9e945c1d0075181b9fb108ef60ae311416cb2b1971888",
      "tools/wtime-conformance/src/spectest.rs": "1403166f4be6ce443b47d4a7ba4a44da553c67e280dfa2876b3f2ad1e64e8883",
      "tools/wtime-conformance/tests/annotations.rs": "5eb54a28e57e7e929bcb2342d65c28e23c2ae112b7c10b36f707fa580f489b08",
      "tools/wtime-conformance/tests/interactions.rs": "fd36710c8cd2ef7f3a2883d3f0ab6cbfecace3ae80adf693095228e4f4641938"
    },
    "implementation_sources_sha256": "d0722f0c1eb2ac15ab05ad57173231df5c39e3247613a50fa60c3b1daab5749a",
    "report_time_workloads_json_sha256": "37633e7b0fd99c729b74d408da76a2bb329e6ffb230c1bc75782a4af57b086c9"
  },
  "source_stable": true,
  "state_checks": [
    {
      "case": "state_normal",
      "collection": {
        "live": 0,
        "reclaimed": 0
      },
      "global": "i32:11",
      "memory_byte_length": 65536,
      "memory_prefix_u32": 11,
      "memory_sha256": "9b7782a6552fb4e41537009e9f0ad735bca1de6eb3a3e0da6e3736bc6093ea07",
      "mode": "reference",
      "outcome": "returned",
      "remaining_memory_zero": true,
      "root": null,
      "table": [
        "function",
        "null"
      ],
      "trap": null,
      "values": [
        "i32:42"
      ]
    },
    {
      "case": "state_normal",
      "collection": {
        "live": 0,
        "reclaimed": 0
      },
      "global": "i32:11",
      "memory_byte_length": 65536,
      "memory_prefix_u32": 11,
      "memory_sha256": "9b7782a6552fb4e41537009e9f0ad735bca1de6eb3a3e0da6e3736bc6093ea07",
      "mode": "decoded",
      "outcome": "returned",
      "remaining_memory_zero": true,
      "root": null,
      "table": [
        "function",
        "null"
      ],
      "trap": null,
      "values": [
        "i32:42"
      ]
    },
    {
      "case": "state_trap",
      "collection": {
        "live": 0,
        "reclaimed": 0
      },
      "global": "i32:5",
      "memory_byte_length": 65536,
      "memory_prefix_u32": 5,
      "memory_sha256": "a0cef7e607271f0d81a0288935d5bf19bb031588b3221473d6b1fb9498dccdde",
      "mode": "reference",
      "outcome": "Trap",
      "remaining_memory_zero": true,
      "root": null,
      "table": [
        "function",
        "null"
      ],
      "trap": "MemoryOutOfBounds",
      "values": []
    },
    {
      "case": "state_trap",
      "collection": {
        "live": 0,
        "reclaimed": 0
      },
      "global": "i32:5",
      "memory_byte_length": 65536,
      "memory_prefix_u32": 5,
      "memory_sha256": "a0cef7e607271f0d81a0288935d5bf19bb031588b3221473d6b1fb9498dccdde",
      "mode": "decoded",
      "outcome": "Trap",
      "remaining_memory_zero": true,
      "root": null,
      "table": [
        "function",
        "null"
      ],
      "trap": "MemoryOutOfBounds",
      "values": []
    },
    {
      "case": "state_graph",
      "collection": {
        "live": 2,
        "reclaimed": 0
      },
      "global": "i32:33",
      "memory_byte_length": 65536,
      "memory_prefix_u32": 33,
      "memory_sha256": "5e9595c77555d6b0c3782705054e329e2fdd44e4074f96f19349bd2b0bfad509",
      "mode": "reference",
      "outcome": "returned",
      "remaining_memory_zero": true,
      "root": "gc",
      "table": [
        "function",
        "null"
      ],
      "trap": null,
      "values": [
        "i32:42"
      ]
    },
    {
      "case": "state_graph",
      "collection": {
        "live": 2,
        "reclaimed": 0
      },
      "global": "i32:33",
      "memory_byte_length": 65536,
      "memory_prefix_u32": 33,
      "memory_sha256": "5e9595c77555d6b0c3782705054e329e2fdd44e4074f96f19349bd2b0bfad509",
      "mode": "decoded",
      "outcome": "returned",
      "remaining_memory_zero": true,
      "root": "gc",
      "table": [
        "function",
        "null"
      ],
      "trap": null,
      "values": [
        "i32:42"
      ]
    }
  ],
  "status": "done",
  "task": "P014",
  "tests": {
    "artifact": {
      "bytes": 1420,
      "path": "/tmp/P014-tool-tests.log",
      "sha256": "a8658e24c977f28139c3f80dee12caa7447bf3029ff08f94e80befa26daf2d73"
    },
    "failed": 0,
    "passed": 9,
    "skipped": 0
  },
  "typed_cases": [
    {
      "case": "call_overhead",
      "equal": true,
      "expected": "i32:101",
      "fixture": {
        "wasm": "tools/wtime-bench/fixtures/call_overhead.wasm",
        "wasm_sha256": "0815ca7d6026206d0773fe10c6b9643862bfffe8ba361c9789369ef9668ee746",
        "wat": "tools/wtime-bench/fixtures/call_overhead.wat",
        "wat_sha256": "9493b8f751690a2fd6e8b19dcee5e2c1d50b717253d292ce34d5692b306663a5"
      },
      "input": "i32:100",
      "oracle": {
        "function": "oracle",
        "independent": true,
        "module": "run.py"
      },
      "runs": {
        "decoded": {
          "binary_sha256": "4101e0338828224718ca96c056893a926a38dfef140af5d0776453885f0735f5",
          "mode": "decoded",
          "ns_per_operation": 683.0,
          "result": "i32:101",
          "stage": "first_call"
        },
        "reference": {
          "binary_sha256": "4101e0338828224718ca96c056893a926a38dfef140af5d0776453885f0735f5",
          "mode": "reference",
          "ns_per_operation": 1075.0,
          "result": "i32:101",
          "stage": "first_call"
        }
      },
      "timing_classification": "diagnostic first_call values; not performance evidence"
    },
    {
      "case": "integer_loop",
      "equal": true,
      "expected": "i32:-205621359",
      "fixture": {
        "wasm": "tools/wtime-bench/fixtures/integer_loop.wasm",
        "wasm_sha256": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c",
        "wat": "tools/wtime-bench/fixtures/integer_loop.wat",
        "wat_sha256": "791730112109f3d5751a7b3e2c1134e7e0e6f71a7f7ee01ea8481eb989ee3fa4"
      },
      "input": "i32:10000",
      "oracle": {
        "function": "oracle",
        "independent": true,
        "module": "run.py"
      },
      "runs": {
        "decoded": {
          "binary_sha256": "4101e0338828224718ca96c056893a926a38dfef140af5d0776453885f0735f5",
          "mode": "decoded",
          "ns_per_operation": 8035803.0,
          "result": "i32:-205621359",
          "stage": "first_call"
        },
        "reference": {
          "binary_sha256": "4101e0338828224718ca96c056893a926a38dfef140af5d0776453885f0735f5",
          "mode": "reference",
          "ns_per_operation": 6362949.0,
          "result": "i32:-205621359",
          "stage": "first_call"
        }
      },
      "timing_classification": "diagnostic first_call values; not performance evidence"
    },
    {
      "case": "branch_calls",
      "equal": true,
      "expected": "i32:804656585",
      "fixture": {
        "wasm": "tools/wtime-bench/fixtures/branch_calls.wasm",
        "wasm_sha256": "9b58d26f201900b5b6cce684133a98ab68b3cf55d979b64ce895e914e9d5327b",
        "wat": "tools/wtime-bench/fixtures/branch_calls.wat",
        "wat_sha256": "d0af4fdf9ad5be38344267ae72fc1855edb5a8fa1b475fd0af4cb507730b2309"
      },
      "input": "i32:5000",
      "oracle": {
        "function": "oracle",
        "independent": true,
        "module": "run.py"
      },
      "runs": {
        "decoded": {
          "binary_sha256": "4101e0338828224718ca96c056893a926a38dfef140af5d0776453885f0735f5",
          "mode": "decoded",
          "ns_per_operation": 5849627.0,
          "result": "i32:804656585",
          "stage": "first_call"
        },
        "reference": {
          "binary_sha256": "4101e0338828224718ca96c056893a926a38dfef140af5d0776453885f0735f5",
          "mode": "reference",
          "ns_per_operation": 11763551.0,
          "result": "i32:804656585",
          "stage": "first_call"
        }
      },
      "timing_classification": "diagnostic first_call values; not performance evidence"
    }
  ]
}
```
