# P026 — Use fixed buffers for SIMD and lane memory access

Status: **done**

Outcome: **rejected**

Kind: `candidate` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P002 — Compare separately built baseline and candidate binaries](../00-measurement/P002-paired-driver.md)

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P010 — Add SIMD and strict floating-point benchmarks](../00-measurement/P010-simd-float-workloads.md)

[P016 — Add explicit executor policies and test integration hooks](../01-execution-contracts/P016-execution-policy.md)

[P025 — Remove per-load heap allocation for all scalar widths](P025-fixed-scalar-reads.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C2, C3, C5, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/execution.rs`
- `crates/wtime-runtime/tests/performance/p026.rs`

- `crates/wtime-runtime/src/lib.rs`

- `crates/wtime-runtime/src/shared_memory.rs`

- `crates/wtime-runtime/tests/fixtures/performance/simd/`

- `tools/wtime-bench/compare.py`

- `tools/wtime-bench/gates.py`

- `tools/wtime-bench/tests/test_p026_compare.py`

- `tools/wtime-bench/tests/test_source_binding.py`

- `docs/benchmarks/performance/P026-contract.json`

- `docs/benchmarks/performance/P026-build-decision.md`

- `tools/wtime-bench/tests/test_p026_calibration.py`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

simd_memory uses fixed [u8;16] or smaller buffers through read_into for full, extending, splat, zero and lane loads; it never allocates a temporary Vec for a vector load.

## Implementation recipe

1. Enumerate each SIMD memory width from current simd_memory and preserve the existing effective-address/lane-immediate validation.
2. Read directly into appropriately initialized fixed buffers; apply extension/splat/lane writes with explicit little-endian semantics and no stale bytes.
3. Keep shared-memory dispatch through the synchronized read_into backend and stores through existing checked write APIs.
4. Measure vector memory kernels and diagnostic read events; use the scalar path as a control rather than assuming identical SIMD speed gains.
5. Record the changed path with focused assertions/diagnostic counts and exact source/binary identities; compare the pre-edit archive, and revert a rejected structural optimization rather than adding a permanent toggle solely for testing.

## Meaningful cases

- Zero/splat/extending loads populate every lane exactly.
- Lane loads preserve untouched bytes and trap at full accessed width.
- Shared vector loads and ordinary scalar loads retain existing synchronization/results.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime p026_ -- --nocapture
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml -j 4
python3 tools/wtime-bench/compare.py --baseline target/performance/P026-before-calibrated --candidate tools/wtime-bench/target/release/wtime-bench --candidate-mode decoded --selection simd --pairs 30 --build-growth-explanation-file docs/benchmarks/performance/P026-build-decision.md --output target/performance/P026
python3 tools/wtime-bench/analyze.py --input target/performance/P026 --output target/performance/P026/analysis.json
python3 tools/checks/build_budget.py --output target/performance/P026/build.json
python3 tools/wtime-bench/gates.py --task P026 --comparison target/performance/P026 --build target/performance/P026/build.json --contract docs/benchmarks/performance/P026-contract.json --output target/performance/P026/decision.json
```

## Measurement and decision

Selection: `simd`. Explicit execution setting: `decoded`. Decision rule: `measured_promotion`.

Use the paired protocol and family/control/cold/build rules in [MEASUREMENT.md](../MEASUREMENT.md). Predeclare the targeted family and held-out controls before changing the implementation. Keep instrumentation off in production timings. Adopt only with supported gains in both runs; otherwise record rejected/inconclusive and remove default-path cost. A necessary immutable-interface change may be contract_only with its separate justification.

## Before editing

Run these commands before changing the candidate sources. The archive is immutable and captures the immediate predecessor. Retain P001's original archive for milestone totals. For native work, use the named portable control and verify its actual policy/quantum; if this archive already exists with different sources, choose a new recorded run-specific directory and update the comparison path rather than overwriting it.

```sh
python3 tools/wtime-bench/baseline.py --output target/performance/P026-before-calibrated
```

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. Executed evidence is recorded below.

## Executed evidence

```json
{
  "task": "P026",
  "outcome": "rejected",
  "artifacts": [
    "/workspace/wtime/target/performance/P026/report.json",
    "/workspace/wtime/target/performance/P026/checkpoint.jsonl",
    "/workspace/wtime/target/performance/P026/extraction.json",
    "/workspace/wtime/target/performance/P026/analysis.json",
    "/workspace/wtime/target/performance/P026/decision.json",
    "/workspace/wtime/target/performance/P026/build.json",
    "/workspace/wtime/docs/benchmarks/performance/P026-contract.json",
    "/workspace/wtime/docs/benchmarks/performance/P026-candidate.patch",
    "/workspace/wtime/target/performance/P026-incremental-build-decision.json",
    "/workspace/wtime/target/performance/P026-rollback-identities.json",
    "/workspace/wtime/target/performance/P026-rollback-build.json",
    "/workspace/wtime/target/performance/P026-rollback-runtime-tests.log",
    "/workspace/wtime/target/performance/P026-rollback-performance-tests.log",
    "/workspace/wtime/AGENTS.md",
    "/workspace/wtime/docs/benchmarks/performance/P026-evidence.json",
    "/workspace/wtime/target/performance/P026/promotion-decision.json",
    "/workspace/wtime/target/performance/P026/complete_executed.py",
    "/workspace/wtime/target/performance/P026/extract_completed.py",
    "/workspace/wtime/target/performance/P026/task-definition-before-completion.md",
    "/workspace/wtime/target/performance/P026-finalization.log"
  ],
  "artifact_hashes": {
    "/workspace/wtime/target/performance/P026/report.json": {
      "sha256": "fa628e3fa042e7c2ed00d45830e1b0dcd13c468ec38b39c94d0afdb9bf9517a7",
      "bytes": 49825330
    },
    "/workspace/wtime/target/performance/P026/checkpoint.jsonl": {
      "sha256": "532757b32b2bc58b3191cf4e2edd93d60fe7fbc1f0b5cdec64673135a0b17efc",
      "bytes": 29249532
    },
    "/workspace/wtime/target/performance/P026/extraction.json": {
      "sha256": "666cc4a2c70806e2053389c676f85d79e54a46e68d370d43ed3cd0a5cd062c76",
      "bytes": 2030
    },
    "/workspace/wtime/target/performance/P026/analysis.json": {
      "sha256": "0345619c41eec4c4d09a3fad633cffeaa65a3cf2585ead007ba60934cc138a5d",
      "bytes": 1705571
    },
    "/workspace/wtime/target/performance/P026/decision.json": {
      "sha256": "553b021ed57d73e1317f569cc8b0a898c76af2a44d8e16246041b7a302e63285",
      "bytes": 2270828
    },
    "/workspace/wtime/target/performance/P026/build.json": {
      "sha256": "4e8dbb2ce9ef63c2075464db4042a8a8d3b59787b1786fb094009dee48a77235",
      "bytes": 84251
    },
    "/workspace/wtime/docs/benchmarks/performance/P026-contract.json": {
      "sha256": "1482c6ff7feea38d0a247dbff91c80d8443f2f217ed126fbc95a879a90f34cf0",
      "bytes": 4265
    },
    "/workspace/wtime/docs/benchmarks/performance/P026-candidate.patch": {
      "sha256": "d9fa4aec88dc94a75658e6eeed3af53ec131bdaff7ea51a893521f08665306c5",
      "bytes": 4166
    },
    "/workspace/wtime/target/performance/P026-incremental-build-decision.json": {
      "sha256": "69606f734b4b7398f036eabf72ce68c06dd79fcc7c8ba10501713974deb54f82",
      "bytes": 632
    },
    "/workspace/wtime/target/performance/P026-rollback-identities.json": {
      "sha256": "921866f97d5066b67afa1b3b9d5903e45a48d9715ee525fe5adee1b26b86f7a4",
      "bytes": 1196
    },
    "/workspace/wtime/target/performance/P026-rollback-build.json": {
      "sha256": "cf1ecc7cf3f06e690829249ab243a3d904f681388b085239435210552923c04e",
      "bytes": 84254
    },
    "/workspace/wtime/target/performance/P026-rollback-runtime-tests.log": {
      "sha256": "4bea0e4bb141677515545216150216b9f3612db5b1d6902f2b60a59b53b3253d",
      "bytes": 13339
    },
    "/workspace/wtime/target/performance/P026-rollback-performance-tests.log": {
      "sha256": "ab6f758e404d120506bef29b87705ce36ece5f7e7667dd2be227d72f356bd277",
      "bytes": 3537
    },
    "/workspace/wtime/AGENTS.md": {
      "sha256": "3950e84c8f48c9ae6d78dfdfde90aae04e9dc4e048b9acdb7e78bb9c5af931a1",
      "bytes": 2110
    },
    "/workspace/wtime/docs/benchmarks/performance/P026-evidence.json": {
      "sha256": "f8a83b60a2419a5f91dbf1c216f1315a4061257832eab41665fe9aaf6ac3dfcd",
      "bytes": 42698
    },
    "/workspace/wtime/target/performance/P026/promotion-decision.json": {
      "sha256": "1de464371afcca4f5af8ab59f41a48bb4d7d1f530b7d65b2ba7a6313f87a86ec",
      "bytes": 6013
    },
    "/workspace/wtime/target/performance/P026/complete_executed.py": {
      "sha256": "c97559af5e9b698f02df9bf4b5df8128aa5b65efe1f5cc0f1cd7b0bf43f9ba56",
      "bytes": 35962
    },
    "/workspace/wtime/target/performance/P026/extract_completed.py": {
      "sha256": "4560f638abedc94b23c2cb59f0a0e917a93b52ff603db89d95b3262b4bfb8d67",
      "bytes": 8614
    },
    "/workspace/wtime/target/performance/P026/task-definition-before-completion.md": {
      "sha256": "2225a547c8f7381459ab70f8d621f406b999dcd2e44dd790c4aa852cb324aa10",
      "bytes": 5705
    },
    "/workspace/wtime/target/performance/P026-finalization.log": {
      "sha256": "f1cea9c7ed804524182ee0f6ca77b123aa3cb16cd692d3dd31648b6a776cfb6b",
      "bytes": 86
    }
  },
  "settings": {
    "selection": "simd",
    "baseline_mode": "decoded",
    "candidate_mode": "decoded",
    "instrumented": false,
    "pairs_per_group": 30,
    "runs": 2,
    "baseline_archive": "target/performance/P026-before-calibrated"
  },
  "counts": {
    "workloads": 114,
    "stages": 5,
    "paired_groups": 1140,
    "pairs": 34200,
    "correctness_records": 114,
    "correctness_side_checks": 228,
    "candidate_default_focused": 4,
    "candidate_diagnostic_focused": 5,
    "candidate_full_default": 183,
    "candidate_feature_performance": 48,
    "adapter_tests": 8,
    "calibration_tests": 5,
    "source_binding_tests": 5,
    "restored_full_default": 183,
    "restored_feature_performance": 48,
    "restored_failed": 0,
    "restored_skipped": 0
  },
  "commands": {
    "comparison": "python3 tools/wtime-bench/compare.py --baseline target/performance/P026-before-calibrated --candidate tools/wtime-bench/target/release/wtime-bench --candidate-mode decoded --selection simd --pairs 30 --build-growth-explanation-file docs/benchmarks/performance/P026-build-decision.md --output target/performance/P026",
    "analysis": "python3 tools/wtime-bench/analyze.py --input target/performance/P026 --output target/performance/P026/analysis.json",
    "gate": "python3 tools/wtime-bench/gates.py --task P026 --comparison target/performance/P026 --build target/performance/P026/build.json --contract docs/benchmarks/performance/P026-contract.json --output target/performance/P026/decision.json",
    "restored_default": "cargo test --locked -p wtime-runtime",
    "restored_diagnostic": "cargo test --locked -p wtime-runtime --features execution-reference,performance-counters --test performance",
    "restored_build": "cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml -j 4",
    "restored_budget": "python3 tools/checks/build_budget.py --output target/performance/P026-rollback-build.json"
  },
  "exit_statuses": {
    "comparison": 0,
    "extraction_corrected": 0,
    "analysis": 0,
    "gate": 1,
    "restored_default_corrected": 0,
    "restored_diagnostic_corrected": 0,
    "restored_build": 0,
    "restored_budget": 0
  },
  "decision": "Rejected: primary family inconclusive in both runs, protected/cold failures; six immediate-predecessor build ratios passed. Production sources restored byte-identically; candidate retained.",
  "limitations": [
    "No supported speed gain and no Wasmtime speed claim.",
    "Historical report source_stable describes measurement time; later restoration is separate.",
    "Initial extractor schema failure, invalid feature name, and stale Cargo artifacts are retained as development/setup history."
  ],
  "primary_family": [
    {
      "bootstrap_resamples": 10000,
      "bootstrap_seed": 2329226862576669797,
      "ci95": {
        "lower": 0.9464771181970228,
        "upper": 1.0261494496590255
      },
      "input_fixture_groups": 5,
      "median_ratio": 0.9819758484758658,
      "required_median_max": 0.95,
      "required_upper_max_exclusive": 1.0,
      "run": 1,
      "workloads": 5
    },
    {
      "bootstrap_resamples": 10000,
      "bootstrap_seed": 9928753566573701063,
      "ci95": {
        "lower": 0.975030280438631,
        "upper": 1.0082770990368644
      },
      "input_fixture_groups": 5,
      "median_ratio": 0.9889897417400478,
      "required_median_max": 0.95,
      "required_upper_max_exclusive": 1.0,
      "run": 2,
      "workloads": 5
    }
  ]
}
```
