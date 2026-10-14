# P027 — Use slice operations for unshared bulk memory and table copies

Status: **done**

Outcome: **rejected**

Kind: `candidate` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P002 — Compare separately built baseline and candidate binaries](../00-measurement/P002-paired-driver.md)

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P009 — Add memory width, bulk and growth benchmark families](../00-measurement/P009-memory-workloads.md)

[P016 — Add explicit executor policies and test integration hooks](../01-execution-contracts/P016-execution-policy.md)

[P025 — Remove per-load heap allocation for all scalar widths](P025-fixed-scalar-reads.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C5, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/execution.rs`
- `crates/wtime-runtime/src/arena.rs`
- `crates/wtime-runtime/src/lib.rs`
- `crates/wtime-runtime/src/perf.rs`
- `crates/wtime-runtime/tests/performance.rs`
- `crates/wtime-runtime/tests/performance/p027.rs`
- `crates/wtime-runtime/tests/fixtures/performance/p027/`
- `tools/wtime-bench/compare.py`
- `tools/wtime-bench/memory_scaling.py`
- `tools/wtime-bench/memory_workloads.py`
- `tools/wtime-bench/p027_memory_workloads.py`
- `tools/wtime-bench/profile.py`
- `tools/wtime-bench/src/memory_driver.rs`
- `tools/wtime-bench/workloads.json`
- `tools/wtime-bench/tests/test_p027_calibration.py`
- `tools/wtime-bench/tests/test_p027_harness.py`
- `tools/wtime-bench/fixtures/memory/memory_copy_distinct_seeded.wat`
- `tools/wtime-bench/fixtures/memory/memory_copy_distinct_seeded.wasm`
- `tools/wtime-bench/fixtures/memory/memory_fill_seeded.wat`
- `tools/wtime-bench/fixtures/memory/memory_fill_seeded.wasm`
- `scripts/merge_registry.py`
- `docs/benchmarks/performance/P027-contract.json`
- `docs/benchmarks/performance/P027-build-decision.md`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

Unshared memory.copy/fill and distinct-table copy use safe disjoint slice borrowing and existing chunk checkpoints; shared/aliased backing retains its coordinated path.

## Implementation recipe

1. Precheck complete source/destination ranges before mutation, detect same unshared memory for copy_within and distinct buffers for split_at_mut borrowing.
2. Implement direct fill/copy per existing cancellation chunk size and overlap direction; do not remove chunk cancellation to make a bulk benchmark win.
3. Apply the same safe split borrowing to distinct table buffers, retaining reference values/roots and table64 range checks; keep shared backing aliases on the original synchronized path.
4. Compare 64B/4KiB/1MiB throughput plus cancellation/OOB effects and confirm no chunk snapshot Vec remains on the unshared path.
5. Record the changed path with focused assertions/diagnostic counts and exact source/binary identities; compare the pre-edit archive, and revert a rejected structural optimization rather than adding a permanent toggle solely for testing.
6. Use the predeclared P027 selection unchanged: six seeded primary rows, 29 protected rows, and 35 selected rows. Keep host seeding outside timed intervals, verify complete memory images, and use adaptive warm batches targeting at least 20 ms.

## Meaningful cases

- Forward/backward overlap equals memmove and alias detection is correct.
- OOB full-range precheck prevents partial copy, while cancellation preserves the documented completed prefix.
- Distinct-table reference copies survive collection and memory64/custom-page ranges remain correct.
- Distinct seeded copy at 64 B, 4 KiB, and 1 MiB changes only the destination image; host seeding stays outside invocation timing.
- Seeded fill at 64 B, 4 KiB, and 1 MiB matches the complete expected memory image, including distinct initial bytes.
- Seeded zero-length and 17-byte copy/fill controls retain correct results and remain protected from regression.
- The P027 private cancellation-after-candidate case confirms chunk checkpoints preserve the documented completed prefix without a snapshot vector on the unshared path.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **seven** named P027 cases, and optimized-path tests must prove the requested path ran. Also retain the private cancellation-after-candidate check when the candidate path is staged.

```sh
cargo test --locked -p wtime-runtime --features performance-counters --test performance p027_ -- --nocapture
python3 tools/wtime-bench/tests/test_p027_calibration.py
python3 tools/wtime-bench/tests/test_p027_harness.py
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml -j 4
python3 tools/wtime-bench/compare.py --baseline target/performance/P027-before --candidate tools/wtime-bench/target/release/wtime-bench --candidate-mode decoded --selection memory --pairs 30 --build-growth-explanation-file docs/benchmarks/performance/P027-build-decision.md --output target/performance/P027
python3 tools/wtime-bench/analyze.py --input target/performance/P027 --output target/performance/P027/analysis.json
python3 tools/checks/build_budget.py --output target/performance/P027/build.json
python3 tools/wtime-bench/gates.py --task P027 --comparison target/performance/P027 --build target/performance/P027/build.json --contract docs/benchmarks/performance/P027-contract.json --output target/performance/P027/decision.json
```

## Measurement and decision

Selection: `memory`. Explicit execution setting: `decoded`. Decision rule: `measured_promotion`. The predeclared contract selects six seeded copy/fill primary workloads, 29 protected controls, and all 35 rows in each run. Host seeding is excluded from invocation timing and reported separately; correctness checks compare complete memory images. Warm batch calibration adapts to a minimum 20 ms target.

Use the paired protocol and family/control/cold/build rules in [MEASUREMENT.md](../MEASUREMENT.md). Predeclare the targeted family and held-out controls before changing the implementation. Keep instrumentation off in production timings. Adopt only with supported gains in both runs; otherwise record rejected/inconclusive and remove default-path cost. A necessary immutable-interface change may be contract_only with its separate justification.

## Before editing

Run these commands before changing the candidate sources. The archive is immutable and captures the immediate predecessor. Retain P001's original archive for milestone totals. For native work, use the named portable control and verify its actual policy/quantum; if this archive already exists with different sources, choose a new recorded run-specific directory and update the comparison path rather than overwriting it.

```sh
python3 tools/wtime-bench/baseline.py --output target/performance/P027-before
```

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least seven named focused cases plus the private cancellation-after-candidate check, no stale or zero-work pass; speed claims need MEASUREMENT.md. Planned evidence may be recorded in `docs/benchmarks/performance/P027-evidence.json` after those checks execute; this task remains todo/pending until then.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. Executed evidence is recorded below.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/docs/benchmarks/performance/P027-candidate.patch": {
      "bytes": 9956,
      "sha256": "9fc314b9dbc70ddf8c6cc866a2e387a6f1593e04fdb7f8d4d7bf71416c2ffbcb"
    },
    "/workspace/wtime/docs/benchmarks/performance/P027-contract.json": {
      "bytes": 1247,
      "sha256": "608ef4386d4f85750397c28282d5f7409a3ad4e93d4aca298a4390646f05b2fe"
    },
    "/workspace/wtime/docs/benchmarks/performance/P027-evidence.json": {
      "bytes": 122362,
      "sha256": "0abcf7e4540cc69fff74dfe07b9c765b8055523399f9c0b284cbd9e3166c8dfa"
    },
    "/workspace/wtime/target/performance/P027-before/manifest.json": {
      "bytes": 36484,
      "sha256": "b1de0814e94fca5e7680a0707af512def8b3a3abbcac25f8d074e2858a4be1e1"
    },
    "/workspace/wtime/target/performance/P027-build.json": {
      "bytes": 89496,
      "sha256": "237e9acf8fd782e714151f8d99f36dab0ecda1c2e232d24255f990728256d89d"
    },
    "/workspace/wtime/target/performance/P027-candidate-retained/source-manifest.json": {
      "bytes": 16554,
      "sha256": "bbdbf2a11f7fde1c9a261885ae26b1294d2429ebb591d0d5dd92175bd3fbb7d4"
    },
    "/workspace/wtime/target/performance/P027-incremental-build-decision.json": {
      "bytes": 631,
      "sha256": "ff4c6ffb7b8187b8a50512e3b775e51246e7f95c1a634effab05e67fed93a596"
    },
    "/workspace/wtime/target/performance/P027/analysis.json": {
      "bytes": 320153,
      "sha256": "b70a38a9969e19a344a8b71d7568ecfc153ea382e0800baf65eb61ccb9e4d6c0"
    },
    "/workspace/wtime/target/performance/P027/checkpoint.jsonl": {
      "bytes": 13787423,
      "sha256": "da508af08914042d546831397f078baf0563e3572664310173ec1fb9354c59e3"
    },
    "/workspace/wtime/target/performance/P027/comparison-build.json": {
      "bytes": 89496,
      "sha256": "237e9acf8fd782e714151f8d99f36dab0ecda1c2e232d24255f990728256d89d"
    },
    "/workspace/wtime/target/performance/P027/decision.json": {
      "bytes": 423462,
      "sha256": "9b4ee65c8b11fe57242cb71dc2a10001b938f9cf7ebbf58983003e161b1fa0d0"
    },
    "/workspace/wtime/target/performance/P027/existing-build-verification.json": {
      "bytes": 4154,
      "sha256": "e244ca15edc800a68de1f51d643445752778f7e494d64c15e270da897273d6e5"
    },
    "/workspace/wtime/target/performance/P027/extraction.json": {
      "bytes": 1953,
      "sha256": "42186e700fffb7784d41d1d85de199eda2e29c001adb5e3f2f457bf7691bccba"
    },
    "/workspace/wtime/target/performance/P027/final-decision.json": {
      "bytes": 8167,
      "sha256": "a86b7035a85144efa5c9d5c0dc4ed4bac3f7f779d8e050f76f3e7ee81d435601"
    },
    "/workspace/wtime/target/performance/P027/report.json": {
      "bytes": 23578883,
      "sha256": "4bf5748817b16831dfe1f3e8f9a598a3c7813a6b842d1e36042c873110f10f5c"
    },
    "/workspace/wtime/target/performance/P027/task-precompletion/P027-unshared-bulk-slices.md": {
      "bytes": 7918,
      "sha256": "4bccb6f16bbc86c59f0beb466cdd9ad900b1e39fafb1b3e64692ae6fb7fcf075"
    },
    "/workspace/wtime/target/performance/P027/task-precompletion/TASKS.json": {
      "bytes": 880323,
      "sha256": "292ef61856f54a03c7a488c6e739b39fd2ea3b717932470c0ca7d130226a1fe4"
    }
  },
  "artifacts": [
    "/workspace/wtime/docs/benchmarks/performance/P027-candidate.patch",
    "/workspace/wtime/docs/benchmarks/performance/P027-contract.json",
    "/workspace/wtime/docs/benchmarks/performance/P027-evidence.json",
    "/workspace/wtime/target/performance/P027-before/manifest.json",
    "/workspace/wtime/target/performance/P027-build.json",
    "/workspace/wtime/target/performance/P027-candidate-retained/source-manifest.json",
    "/workspace/wtime/target/performance/P027-incremental-build-decision.json",
    "/workspace/wtime/target/performance/P027/analysis.json",
    "/workspace/wtime/target/performance/P027/checkpoint.jsonl",
    "/workspace/wtime/target/performance/P027/comparison-build.json",
    "/workspace/wtime/target/performance/P027/decision.json",
    "/workspace/wtime/target/performance/P027/existing-build-verification.json",
    "/workspace/wtime/target/performance/P027/extraction.json",
    "/workspace/wtime/target/performance/P027/final-decision.json",
    "/workspace/wtime/target/performance/P027/report.json",
    "/workspace/wtime/target/performance/P027/task-precompletion/P027-unshared-bulk-slices.md",
    "/workspace/wtime/target/performance/P027/task-precompletion/TASKS.json"
  ],
  "build_ratio_schemas": {
    "distinction": "The retained strict decision artifact has a custom six-field vector including runtime_leaf_seconds; gates.RELATIVE_LIMITS is the complete project vector and uses unchanged_peak_rss_kib instead.",
    "gates.RELATIVE_LIMITS_complete": {
      "failed": [
        "clean_seconds",
        "leaf_seconds",
        "release_seconds"
      ],
      "fields": [
        "clean_peak_rss_kib",
        "clean_seconds",
        "leaf_seconds",
        "release_seconds",
        "stripped_bytes",
        "unchanged_peak_rss_kib"
      ],
      "limits": {
        "clean_peak_rss_kib": 1.25,
        "clean_seconds": 1.25,
        "leaf_seconds": 1.25,
        "release_seconds": 1.25,
        "stripped_bytes": 1.25,
        "unchanged_peak_rss_kib": 1.25
      },
      "ratios": {
        "clean_peak_rss_kib": 1.035457963089542,
        "clean_seconds": 1.960294812516361,
        "leaf_seconds": 1.4731083231028161,
        "release_seconds": 1.2939528915658733,
        "stripped_bytes": 1.0009037614069884,
        "unchanged_peak_rss_kib": 1.0057394554191137
      }
    },
    "strict_artifact_custom": {
      "failed": [
        "clean_seconds",
        "leaf_seconds",
        "release_seconds"
      ],
      "fields": [
        "clean_peak_rss_kib",
        "clean_seconds",
        "leaf_seconds",
        "release_seconds",
        "runtime_leaf_seconds",
        "stripped_bytes"
      ],
      "ratios": {
        "clean_peak_rss_kib": 1.035457963089542,
        "clean_seconds": 1.960294812516361,
        "leaf_seconds": 1.4731083231028161,
        "release_seconds": 1.2939528915658733,
        "runtime_leaf_seconds": 0.8374312905771024,
        "stripped_bytes": 1.0009037614069884
      }
    },
    "verification_path": "/workspace/wtime/target/performance/P027/existing-build-verification.json",
    "verification_sha256": "e244ca15edc800a68de1f51d643445752778f7e494d64c15e270da897273d6e5"
  },
  "candidate_promoted": false,
  "command_provenance_limitations": [
    "The baseline probe report retains exact child CLI request arrays and all observations, but its outer shell invocation was not retained; no command line is inferred for it."
  ],
  "commands": [
    {
      "command": ". /workspace/.setup/rust-env.sh\npython3 tools/wtime-bench/compare.py --baseline target/performance/P027-before --candidate tools/wtime-bench/target/release/wtime-bench --candidate-mode decoded --selection memory --pairs 30 --build-growth-explanation-file docs/benchmarks/performance/P027-build-decision.md --output target/performance/P027 > target/performance/P027-run.log 2>&1",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P027-run.log",
      "log_sha256": "2f061044e27c1859e44856a8923c21da4452997f02de83151d1a24c93dee3f3f",
      "name": "comparison"
    },
    {
      "command": ". /workspace/.setup/rust-env.sh\npython3 /tmp/extract_p027.py --report /workspace/wtime/target/performance/P027/report.json --checkpoint /workspace/wtime/target/performance/P027/checkpoint.jsonl --archive /workspace/wtime/target/performance/P027-before --tool-dir /workspace/wtime/tools/wtime-bench --candidate-dir /workspace/wtime/target/performance/P027-candidate-retained --output /workspace/wtime/target/performance/P027/extraction.json",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P027/extraction.log",
      "log_sha256": "0d5a7184c31d5ae6faadc61a7953116088487e9e365e3f9b5ff079611fe644a6",
      "name": "extraction",
      "stdout_path": "/workspace/wtime/target/performance/P027/extraction.log",
      "stdout_sha256": "0d5a7184c31d5ae6faadc61a7953116088487e9e365e3f9b5ff079611fe644a6"
    },
    {
      "command": ". /workspace/.setup/rust-env.sh\npython3 /tmp/P027-verify-strict-build.py --output /workspace/wtime/target/performance/P027/existing-build-verification.json",
      "executed_script_sha256": "74607a970e8c94c7c085becc28a5b6d9256657c91399baaa8dc1fd095ca83d07",
      "exit_code": 1,
      "failure_reason": "Checker required absent relative_build_growth_explanation_source; actual schema contains matching policy_sha256 only",
      "log_path": "/workspace/wtime/target/performance/P027/existing-build-verification.log",
      "log_sha256": "55307618c856bc7510fd0db2726c3e35b2d92c6b72d5bd29c931487f6b4ca4b8",
      "name": "strict_build_gate_schema_failure",
      "purpose": "verify existing rejected samples; no new budget sample",
      "stdout_path": "/workspace/wtime/target/performance/P027/existing-build-verification.log",
      "stdout_sha256": "55307618c856bc7510fd0db2726c3e35b2d92c6b72d5bd29c931487f6b4ca4b8"
    },
    {
      "command": ". /workspace/.setup/rust-env.sh\npython3 /tmp/P027-verify-strict-build.py --output /workspace/wtime/target/performance/P027/existing-build-verification.json",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P027/existing-build-verification.json",
      "log_sha256": "e244ca15edc800a68de1f51d643445752778f7e494d64c15e270da897273d6e5",
      "name": "strict_build_gate",
      "purpose": "verify existing rejected samples; no new budget sample",
      "stdout_path": "/workspace/wtime/target/performance/P027/existing-build-verification-retry.log",
      "stdout_sha256": "99931f87a469e0efd55bfb19b4bc36d2eb80705f6f2ec63b606279c3b7c19cc9"
    },
    {
      "command": ". /workspace/.setup/rust-env.sh\npython3 tools/wtime-bench/analyze.py --input target/performance/P027 --output target/performance/P027/analysis.json",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P027/analysis.log",
      "log_sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
      "name": "analysis",
      "stdout_path": "/workspace/wtime/target/performance/P027/analysis.log",
      "stdout_sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    },
    {
      "command": ". /workspace/.setup/rust-env.sh\npython3 tools/wtime-bench/gates.py --task P027 --comparison target/performance/P027 --build target/performance/P027/comparison-build.json --contract docs/benchmarks/performance/P027-contract.json --output target/performance/P027/decision.json",
      "exit_code": 1,
      "log_path": "/workspace/wtime/target/performance/P027/raw-gate.log",
      "log_sha256": "9b4ee65c8b11fe57242cb71dc2a10001b938f9cf7ebbf58983003e161b1fa0d0",
      "name": "raw_gate",
      "stdout_path": "/workspace/wtime/target/performance/P027/raw-gate.log",
      "stdout_sha256": "9b4ee65c8b11fe57242cb71dc2a10001b938f9cf7ebbf58983003e161b1fa0d0"
    },
    {
      "command": ". /workspace/.setup/rust-env.sh\ncargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml -j 4",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P027/candidate-build-verification.log",
      "log_sha256": "9ba21e9ae2f9eaebeb075bb454cb4e4f2f949f247ac35e99e04f280f42ea9df4",
      "name": "candidate_build",
      "original_command_flags_retained": false,
      "purpose": "unchanged-build binary verification after timing; not replacement of historical 43.31s build or budget sample",
      "stdout_path": "/workspace/wtime/target/performance/P027/candidate-build-verification.log",
      "stdout_sha256": "9ba21e9ae2f9eaebeb075bb454cb4e4f2f949f247ac35e99e04f280f42ea9df4"
    },
    {
      "command": ". /workspace/.setup/rust-env.sh\ncargo test --locked -p wtime-runtime",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P027/restored-default-tests.log",
      "log_sha256": "310b2c662234389bf09f400655b70d668ce5a3fef3dd24e71f8479f1e21a81a8",
      "name": "restored_default_tests"
    },
    {
      "command": ". /workspace/.setup/rust-env.sh\ncargo test --locked -p wtime-runtime --features execution-reference,performance-counters --test performance",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P027/restored-diagnostic-tests.log",
      "log_sha256": "d6e71f35b0cd9b6abab20948f7c4041e3899a2915e1790276c6c71aca1b8c55d",
      "name": "restored_diagnostic_tests"
    },
    {
      "command": ". /workspace/.setup/rust-env.sh\ncargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml -j 4",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P027/restored-benchmark-build.log",
      "log_sha256": "cd063f699e5d52122d1eaab444eda6904cfe8f595fd925e2795c77643e5926c6",
      "name": "rollback_benchmark_build"
    },
    {
      "command": ". /workspace/.setup/rust-env.sh\npython3 tools/checks/build_budget.py --output target/performance/P027/restored-build.json",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P027/restored-build.log",
      "log_sha256": "37a7c3332d512cb7ba1a8af6dbffc8a64e2aa854d75e62915311fa83d9aa7806",
      "name": "rollback_build"
    }
  ],
  "comparison_exit_code": 0,
  "counts": {
    "correctness_probe_records": 36,
    "default_restored_tests": 190,
    "diagnostic_restored_tests": 56,
    "groups": 182,
    "paired_observations": 5460,
    "workloads": 35
  },
  "historical_task_snapshot": [
    {
      "bytes": 880323,
      "path": "/workspace/wtime/target/performance/P027/task-precompletion/TASKS.json",
      "sha256": "292ef61856f54a03c7a488c6e739b39fd2ea3b717932470c0ca7d130226a1fe4",
      "source": "/workspace/tasks/performance/TASKS.json"
    },
    {
      "bytes": 7918,
      "path": "/workspace/wtime/target/performance/P027/task-precompletion/P027-unshared-bulk-slices.md",
      "sha256": "4bccb6f16bbc86c59f0beb466cdd9ad900b1e39fafb1b3e64692ae6fb7fcf075",
      "source": "/workspace/tasks/performance/02-structural-wins/P027-unshared-bulk-slices.md"
    }
  ],
  "outcome": "rejected",
  "raw_gate_decision": "rejected",
  "raw_gate_exit_code": 1,
  "reason": "strict immediate-predecessor build gate failed its predeclared 1.25 ratio limit",
  "rollback_verified": {
    "expect_direct_bulk": false,
    "freshness_proven": true,
    "post_restore_build": {
      "budget_pass": true,
      "default_dependency_closure_pass": true,
      "path": "/workspace/wtime/target/performance/P027/restored-build.json",
      "sha256": "4cc443d54f34be54c54670541b2f3e48be23906e9282396800edbe811d13705b",
      "source_stable": true
    },
    "post_restore_tests": {
      "default": {
        "failed": 0,
        "ignored": 0,
        "passed": 190,
        "path": "/workspace/wtime/target/performance/P027/restored-default-tests.log",
        "sha256": "310b2c662234389bf09f400655b70d668ce5a3fef3dd24e71f8479f1e21a81a8",
        "test_suites": 7
      },
      "feature_diagnostic": {
        "failed": 0,
        "ignored": 0,
        "passed": 56,
        "path": "/workspace/wtime/target/performance/P027/restored-diagnostic-tests.log",
        "sha256": "d6e71f35b0cd9b6abab20948f7c4041e3899a2915e1790276c6c71aca1b8c55d",
        "test_suites": 1
      }
    },
    "predecessor_archive": "/workspace/wtime/target/performance/P027-before",
    "predecessor_manifest_sha256": "b1de0814e94fca5e7680a0707af512def8b3a3abbcac25f8d074e2858a4be1e1",
    "production_restored": true,
    "reverted": true,
    "source_mtime_ns_max": 1791740226630392265,
    "verified_restored_sources": {
      "crates/wtime-runtime/src/arena.rs": {
        "bytes": 3852,
        "sha256": "cd929b222d1ccf871f06684510df6767a1e69af4d338458c0e98b88cd16c8441"
      },
      "crates/wtime-runtime/src/execution.rs": {
        "bytes": 53349,
        "sha256": "7516480024d6d9a4037374cd9434614cf9fefe042ffdaed02852700b478f18af"
      },
      "crates/wtime-runtime/src/lib.rs": {
        "bytes": 71696,
        "sha256": "997a0ec8d835c83567e13aa4c755ed30ad0e44a0bdb9e689c14753abdd47bb45"
      }
    }
  },
  "schema": "wtime-p027-completion-v1",
  "status": "done",
  "strict_build_gate_passed": false,
  "task": "P027",
  "task_index_update_performed": false
}
```
