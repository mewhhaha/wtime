# P002 — Compare separately built baseline and candidate binaries

Status: **done**

Outcome: **contract_only**

Kind: `foundation` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P001 — Archive the current runtime and benchmark baseline](P001-freeze-baseline.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `tools/wtime-bench/compare.py`
- `tools/wtime-bench/src/main.rs`
- `tools/wtime-bench/tests/test_p002_compare.py`
- `tools/wtime-bench/baseline.py`
- `tools/wtime-bench/legacy_adapter/Cargo.toml`
- `tools/wtime-bench/legacy_adapter/src/main.rs`
- `tools/wtime-bench/src/protocol.rs`

- `tools/wtime-bench/checkpoint.py`

- `tools/wtime-bench/tests/test_p002_checkpoint.py`

- `docs/benchmarks/performance/P002-checkpoint-evidence.json`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

compare.py --baseline DIR --candidate EXE --selection NAME --output DIR drives two processes over identical bytes and emits versioned raw A/B pairs; initial original-mvp selection needs no new runtime API. The historical executable has only its six-argument CLI; paired single-case execution uses a separately hashed compatible adapter built against immutable frozen core sources, not an assumed new CLI on that old binary.

## Implementation recipe

1. Preserve the current six-argument harness CLI; add an explicit protocol/version query and single-case execution adapter without changing historical timing boundaries.
2. Add baseline.py --adapt-harness HARNESS --baseline ARCHIVE --output DIR: compile the shared single-case protocol against the archived original core source in an isolated legacy-api adapter. Never overwrite the original binary/source/report; bind the new adapter hash, frozen-core hash and Cargo paths. It keeps Engine::new and rejects unavailable new policies.
3. Verify baseline archive and candidate identities, generate one shared fixture set, and launch each executable in deterministic randomized A/B order under the same allowed affinity.
4. Record pair ID, process/engine identity, stage, input, batch count, nanoseconds, full settings and binary/source provenance; reject missing, duplicated, negative or non-finite observations.
5. Finish with at least 30 pairs in each of two independent seeded runs; prohibit concurrent builds and fail if candidate source/executable or either fixture changes mid-run.

## Meaningful cases

- A process crash or missing sample produces a nonzero exit and partial-error report.
- A deliberately wrong result aborts before its timings are accepted.
- Swapping randomized order preserves pair IDs and the exact workload bytes.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
python3 -m unittest discover -s tools/wtime-bench/tests -p 'test_p002*.py' -v
python3 tools/wtime-bench/baseline.py --adapt-harness tools/wtime-bench --baseline target/performance/baseline --output target/performance/baseline-adapter
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml -j 4
python3 tools/wtime-bench/compare.py --baseline target/performance/baseline-adapter --candidate tools/wtime-bench/target/release/wtime-bench --selection original-mvp --output target/performance/P002
```

## Measurement and decision

Selection: `original-mvp`. Explicit execution setting: `decoded`. Decision rule: `integrity`.

This is a tooling/contract deliverable. Require correct nonzero checks and bounded/source-bound output; do not invent a runtime speedup requirement. If it changes production work, record cold/build/retention deltas under [MEASUREMENT.md](../MEASUREMENT.md).

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. Executed evidence is recorded below.

## Executed evidence

```json
{
  "artifacts": [
    "/workspace/wtime/target/performance/P002/report.json",
    "/workspace/wtime/target/performance/baseline-adapter/manifest.json",
    "/workspace/wtime/target/performance/P002-checkpoint/report.json",
    "/workspace/wtime/target/performance/P002-checkpoint/checkpoint.jsonl",
    "/workspace/wtime/docs/benchmarks/performance/P002-checkpoint-evidence.json"
  ],
  "existing_completion_record": {
    "artifacts": [
      "/workspace/wtime/target/performance/P002/report.json",
      "/workspace/wtime/target/performance/baseline-adapter/manifest.json"
    ],
    "baseline_binary_sha256": "9e0c3a24e07ee08ededc868120944c4b58727e4dbd0fb42d2d6910f0c3acf70c",
    "candidate_binary_sha256": "ba29196fb42d3c0c83d42e7c3976245310e0960f3bd53d8f537d6b4ddf1e3b58",
    "commands": [
      {
        "command": "python3 -m unittest discover -s tools/wtime-bench/tests -p test_p002*.py -v",
        "exit_code": 0,
        "failed": 0,
        "passed": 9,
        "skipped": 0
      },
      {
        "command": "cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml -j 4",
        "exit_code": 0
      },
      {
        "command": "python3 tools/wtime-bench/compare.py --baseline target/performance/baseline-adapter --candidate tools/wtime-bench/target/release/wtime-bench --selection original-mvp --output target/performance/P002",
        "exit_code": 0,
        "groups_per_run": 21,
        "paired_observations": 1260,
        "pairs_per_group": 30,
        "runs": 2,
        "timing_rows": 2520
      }
    ],
    "limitations": [
      "Synthetic MVP fixture set only; proposal families arrive in later tasks.",
      "Shared-cloud affinity cannot guarantee an idle physical core.",
      "Pre-measure packaging defect in adapter executable mode and binary manifest field repaired; frozen baseline bytes unchanged."
    ],
    "report_sha256": "4d3293e10166ddf6fb148da5b66b7c999e8d14b5122b7cd75ed97b74b41eefba"
  },
  "existing_completion_record_sha256_canonical": "b53f217654086e13ece7bbe6422729e94c1d6e880e303a32dcce1dce050e0336",
  "integrity": {
    "all_103_archive_files_verified": true,
    "archive_manifest_matches_report": true,
    "journal_pair_count": 1260,
    "journal_seeds_match_report_runs": true,
    "ordered_journal_pairs_equal_report_pairs": true,
    "report_fixture_hashes_verified": true,
    "report_pair_count": 1260,
    "report_passed_and_source_stable": true
  },
  "kind": "supplemental_completion_evidence",
  "logs": {
    "P002-checkpoint-archive.log": {
      "path": "/workspace/wtime/target/performance/P002-checkpoint-archive.log",
      "sha256": "4f6223231a8bd3d0a5c1fcc13883e2594b98e409419a1eb112568cd597a7be06",
      "size_bytes": 254
    },
    "P002-checkpoint-run.log": {
      "path": "/workspace/wtime/target/performance/P002-checkpoint-run.log",
      "sha256": "4dcd124d5873f9583740abf0b6185bfe6e36b88f780da6969dd5a773317fc1f0",
      "size_bytes": 1978
    },
    "P002-checkpoint-tests.log": {
      "path": "/workspace/wtime/target/performance/P002-checkpoint-tests.log",
      "sha256": "1a11681d66fba9ce231ccdba34e674813613d35183f2a1a58874ea2c2395c9f4",
      "size_bytes": 2705
    },
    "P002-checkpoint-walltime.txt": {
      "path": "/workspace/wtime/target/performance/P002-checkpoint-walltime.txt",
      "sha256": "ae9b28eaa38d346fdb0085d62b13c1931512577c2275e85f93b03732a4e2d6e1",
      "size_bytes": 18
    }
  },
  "measurement": {
    "groups_per_run": 21,
    "paired_observations": 1260,
    "pairs_per_group": 30,
    "recorded_at_utc": "2026-10-11T12:23:14.600674+00:00",
    "runs": [
      {
        "groups": 21,
        "pairs": 630,
        "pairs_per_group": 30,
        "run_id": 0,
        "seed": 20261011
      },
      {
        "groups": 21,
        "pairs": 630,
        "pairs_per_group": 30,
        "run_id": 1,
        "seed": 21261014
      }
    ],
    "settings": {
      "baseline_mode": "decoded",
      "cancellation_quantum": 1,
      "candidate_mode": "decoded",
      "feature_profiles": [],
      "features": "mvp",
      "fuel": false,
      "instrumented": false,
      "profile": "release"
    },
    "stages": [
      "prepare",
      "instantiate",
      "first_call",
      "warm_call",
      "load_and_call"
    ],
    "timing_rows": 2520,
    "total_groups": 42,
    "wall_time_seconds": 95.19049985399761,
    "workloads": 5
  },
  "preserves_existing_completion_record": true,
  "result": {
    "decision": "integrity validation only; no speedup claim",
    "failed_pretrial_attempt": "The initially wrong selection failed before writing output; excluded from evidence.",
    "passed": true,
    "process_model": "Fresh processes for each baseline/candidate observation. Checkpoint resume stores pair rows only and does not keep a runtime alive.",
    "selection": "original-mvp",
    "source_stable": true
  },
  "schema": "wtime-p002-checkpoint-extension-v1",
  "task": "P002",
  "tests": {
    "command": "python3 -m unittest discover -s tools/wtime-bench/tests -p 'test_p002*.py' -v",
    "exit_code": 0,
    "failed": 0,
    "log": {
      "path": "/workspace/wtime/target/performance/P002-checkpoint-tests.log",
      "sha256": "1a11681d66fba9ce231ccdba34e674813613d35183f2a1a58874ea2c2395c9f4",
      "size_bytes": 2705
    },
    "passed": 17,
    "skipped": 0,
    "test_names": [
      "test_p002_adapter_binary_permission_tampering_fails_verification (test_p002_adapter.LegacyAdapterArchiveTests.test_p002_adapter_binary_permission_tampering_fails_verification) ... ok",
      "test_p002_adapter_binary_tampering_fails_manifest_hash (test_p002_adapter.LegacyAdapterArchiveTests.test_p002_adapter_binary_tampering_fails_manifest_hash) ... ok",
      "test_p002_changed_frozen_archive_is_rejected_without_rebuild (test_p002_adapter.LegacyAdapterArchiveTests.test_p002_changed_frozen_archive_is_rejected_without_rebuild) ... ok",
      "test_p002_changed_shared_protocol_rejects_existing_adapter (test_p002_adapter.LegacyAdapterArchiveTests.test_p002_changed_shared_protocol_rejects_existing_adapter) ... ok",
      "test_p002_changed_shared_runner_rejects_existing_adapter (test_p002_adapter.LegacyAdapterArchiveTests.test_p002_changed_shared_runner_rejects_existing_adapter) ... ok",
      "test_p002_matching_frozen_adapter_is_reused_and_verified (test_p002_adapter.LegacyAdapterArchiveTests.test_p002_matching_frozen_adapter_is_reused_and_verified) ... ok",
      "test_complete_malformed_line_rejected (test_p002_checkpoint.JournalTests.test_complete_malformed_line_rejected) ... ok",
      "test_duplicate_pair_and_conflicting_run_id_are_rejected (test_p002_checkpoint.JournalTests.test_duplicate_pair_and_conflicting_run_id_are_rejected) ... ok",
      "test_header_and_seed_conflicts_are_rejected (test_p002_checkpoint.JournalTests.test_header_and_seed_conflicts_are_rejected) ... ok",
      "test_only_malformed_unterminated_tail_is_repaired (test_p002_checkpoint.JournalTests.test_only_malformed_unterminated_tail_is_repaired) ... ok",
      "test_partial_header_and_valid_unterminated_tail_are_rejected (test_p002_checkpoint.JournalTests.test_partial_header_and_valid_unterminated_tail_are_rejected) ... ok",
      "test_passed_report_is_never_overwritten (test_p002_checkpoint.JournalTests.test_passed_report_is_never_overwritten) ... ok",
      "test_recovery_keeps_all_pairs_and_raw_values (test_p002_checkpoint.JournalTests.test_recovery_keeps_all_pairs_and_raw_values) ... ok",
      "test_source_and_binary_mismatch_and_missing_checkpoint_fail_closed (test_p002_checkpoint.JournalTests.test_source_and_binary_mismatch_and_missing_checkpoint_fail_closed) ... ok",
      "test_p002_missing_wrong_oracle_and_nonfinite_fail (test_p002_compare.CompareTests.test_p002_missing_wrong_oracle_and_nonfinite_fail) ... ok",
      "test_p002_order_is_reproducible_and_contains_both_orders (test_p002_compare.CompareTests.test_p002_order_is_reproducible_and_contains_both_orders) ... ok",
      "test_p002_process_crash_and_missing_response_fail (test_p002_compare.CompareTests.test_p002_process_crash_and_missing_response_fail) ... ok"
    ]
  },
  "artifact_details": {
    "baseline_archive": {
      "manifest": {
        "path": "/workspace/wtime/target/performance/P002-checkpoint-before/manifest.json",
        "sha256": "6a284d5ed122e8bfe7cd3359f941441bc91835e8de846e00e84027821ff31a81",
        "size_bytes": 28007
      },
      "manifest_records": {
        "all_files_verified": true,
        "file_count": 103,
        "files": [
          {
            "path": "binary/wtime-bench",
            "role": "release_benchmark_executable",
            "sha256": "489cbfdc0690c4f2d52e30df4019a1cca356b27d2d2d818b80e22176b4f48f43",
            "size_bytes": 16743080
          },
          {
            "path": "binary/wtime-conformance",
            "role": "fixture_encoder_executable",
            "sha256": "6616cabefdaf1cb99514a43fc27b63a039210727ac0ee22e2dab3cfb2f5b4ce7",
            "size_bytes": 23921656
          },
          {
            "path": "fixtures/branch_calls.wasm",
            "role": "deterministic_fixture",
            "sha256": "9b58d26f201900b5b6cce684133a98ab68b3cf55d979b64ce895e914e9d5327b",
            "size_bytes": 157
          },
          {
            "path": "fixtures/branch_calls.wat",
            "role": "deterministic_fixture",
            "sha256": "d0af4fdf9ad5be38344267ae72fc1855edb5a8fa1b475fd0af4cb507730b2309",
            "size_bytes": 604
          },
          {
            "path": "fixtures/call_overhead.wasm",
            "role": "deterministic_fixture",
            "sha256": "0815ca7d6026206d0773fe10c6b9643862bfffe8ba361c9789369ef9668ee746",
            "size_bytes": 40
          },
          {
            "path": "fixtures/call_overhead.wat",
            "role": "deterministic_fixture",
            "sha256": "9493b8f751690a2fd6e8b19dcee5e2c1d50b717253d292ce34d5692b306663a5",
            "size_bytes": 88
          },
          {
            "path": "fixtures/integer_loop.wasm",
            "role": "deterministic_fixture",
            "sha256": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c",
            "size_bytes": 122
          },
          {
            "path": "fixtures/integer_loop.wat",
            "role": "deterministic_fixture",
            "sha256": "791730112109f3d5751a7b3e2c1134e7e0e6f71a7f7ee01ea8481eb989ee3fa4",
            "size_bytes": 391
          },
          {
            "path": "fixtures/memory_fill_scan.wasm",
            "role": "deterministic_fixture",
            "sha256": "1c933d404fe9feb93bb664af3bbb60a50319cdd6a2c29d442aa9ab7799f5e65f",
            "size_bytes": 171
          },
          {
            "path": "fixtures/memory_fill_scan.wat",
            "role": "deterministic_fixture",
            "sha256": "c0a222245af932b6d43ca964fa710f56476dc90ebaca0b20915d35b242a6c87d",
            "size_bytes": 644
          },
          {
            "path": "fixtures/prepare_large.wasm",
            "role": "deterministic_fixture",
            "sha256": "c489fe06970f8e1c60e4992774aaadb571f676833d6579de1f67a5facf3fe9b0",
            "size_bytes": 215036
          },
          {
            "path": "fixtures/prepare_large.wat",
            "role": "deterministic_fixture",
            "sha256": "aefc7237fa17bfbccd8c318bc104111c8e3296933a94017837d2c02130d638fd",
            "size_bytes": 924025
          },
          {
            "path": "historical_reports/wasmtime-comparison-run2.json",
            "role": "historical_report",
            "sha256": "6818fa194027b57b183ca3a42a55627c676ba857392497fa325345e0f3fd090b",
            "size_bytes": 174851
          },
          {
            "path": "historical_reports/wasmtime-comparison.json",
            "role": "historical_report",
            "sha256": "b1b69f3944b488a037c08305a43bd8de474837958c53217f1c3cb94b7d04354c",
            "size_bytes": 174792
          },
          {
            "path": "runs/historical-format.json",
            "role": "fresh_historical_format_run",
            "sha256": "aad005f6c0f950b6871734b586ab4d80dd2f6921f7750a38fd7b5ee90ece428d",
            "size_bytes": 57706
          },
          {
            "path": "runs/mode-smoke.json",
            "role": "mode_aware_single_case_run",
            "sha256": "895cd349f44d590554628fd95b326f0df9edc708d235e6ceb0c29ff0afbf46f6",
            "size_bytes": 187
          },
          {
            "path": "source/Cargo.lock",
            "role": "source",
            "sha256": "9d704c9854b30ca231109a54598c1c440a7d144b8ece5e21539971fead240820",
            "size_bytes": 5482
          },
          {
            "path": "source/Cargo.toml",
            "role": "source",
            "sha256": "9656851608b4f5ecb5f973a5ad370c67aef45fc557d4c132469bbf012662efff",
            "size_bytes": 587
          },
          {
            "path": "source/LICENSE-APACHE",
            "role": "source",
            "sha256": "a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2",
            "size_bytes": 10847
          },
          {
            "path": "source/LICENSE-MIT",
            "role": "source",
            "sha256": "a43aeeeebe46eda1a6fea77337eb7f9f37341b94afe7251d1f6d14fe03e702c5",
            "size_bytes": 1062
          },
          {
            "path": "source/crates/wtime-cli/Cargo.toml",
            "role": "source",
            "sha256": "67dc333182e863f6bbe2a09d9c99e7519ff35a1bc41c551c49273cc1c9f935e8",
            "size_bytes": 209
          },
          {
            "path": "source/crates/wtime-cli/src/main.rs",
            "role": "source",
            "sha256": "760feeee1801d17c5675ba1f35e5fed10b3e2a02eb58458435e7d9ad355e6dfe",
            "size_bytes": 5433
          },
          {
            "path": "source/crates/wtime-cli/tests/metadata.rs",
            "role": "source",
            "sha256": "38015f945e99ffca1c9dc18577575057050a91e4df215314d7636f3c9cc7141f",
            "size_bytes": 1847
          },
          {
            "path": "source/crates/wtime-decode/Cargo.toml",
            "role": "source",
            "sha256": "1ae571b0eb1a293cd0ef6a181e33d267c40b228cd1afd5dd44ef9a309d113fa1",
            "size_bytes": 181
          },
          {
            "path": "source/crates/wtime-decode/src/instructions.rs",
            "role": "source",
            "sha256": "d352215a69fc6d33b188850974c364023510056c347d0ecf95ce9ba96cd05168",
            "size_bytes": 12354
          },
          {
            "path": "source/crates/wtime-decode/src/lib.rs",
            "role": "source",
            "sha256": "9a83e26510e41a1d73556ebc74f4402f0aecb3493068dd7f66d4851d451524d9",
            "size_bytes": 304
          },
          {
            "path": "source/crates/wtime-decode/src/metadata.rs",
            "role": "source",
            "sha256": "e9b8d4d88fc5b34dccd0a1918346cfed3a03dd256129dca4c62f4e0bce8d036e",
            "size_bytes": 5900
          },
          {
            "path": "source/crates/wtime-decode/src/module.rs",
            "role": "source",
            "sha256": "51b8fb2669a8323f36c7b199847a002f5b751dc04a382584a7112d7dd7dde71d",
            "size_bytes": 29971
          },
          {
            "path": "source/crates/wtime-decode/src/reader.rs",
            "role": "source",
            "sha256": "0c061e8df82b02c1d0fb25ad2744991a2a8f0db7893ae5d5818ba89772651e6e",
            "size_bytes": 5875
          },
          {
            "path": "source/crates/wtime-runtime/Cargo.toml",
            "role": "source",
            "sha256": "2ee79daf9485c056793d57b1886f75e0e74b12a6ddbf69f4b18f20ecffe8171b",
            "size_bytes": 634
          },
          {
            "path": "source/crates/wtime-runtime/examples/embedding.rs",
            "role": "source",
            "sha256": "d9baac3b51bacbee00c42a109831e8bd1a845158b5a0ceae46a911307d8b0907",
            "size_bytes": 1091
          },
          {
            "path": "source/crates/wtime-runtime/examples/suspension.rs",
            "role": "source",
            "sha256": "097175e5cbbcf1be4716d4c1b584702042558d299360392fa742a85ead331e1d",
            "size_bytes": 1227
          },
          {
            "path": "source/crates/wtime-runtime/src/arena.rs",
            "role": "source",
            "sha256": "cd929b222d1ccf871f06684510df6767a1e69af4d338458c0e98b88cd16c8441",
            "size_bytes": 3852
          },
          {
            "path": "source/crates/wtime-runtime/src/canonical.rs",
            "role": "source",
            "sha256": "0bed54a47fbca8a39c7359a9140a5284f0513a6744505ddb5a0b92d531d0776d",
            "size_bytes": 17695
          },
          {
            "path": "source/crates/wtime-runtime/src/continuation.rs",
            "role": "source",
            "sha256": "c30b6fb4b89999b2e6f494b9258ce9ea3ea1e39a5971643b43147026e0fc8f46",
            "size_bytes": 56170
          },
          {
            "path": "source/crates/wtime-runtime/src/coverage.rs",
            "role": "source",
            "sha256": "fa735d933bf1334d23c262aac2d35282a743a49aaddb0c2bbc68da4c308f3fbe",
            "size_bytes": 796
          },
          {
            "path": "source/crates/wtime-runtime/src/exceptions.rs",
            "role": "source",
            "sha256": "08e306b7f60b25077f31706bf55cc4b521b0b2b713a61e8656e36fcc34c326ba",
            "size_bytes": 9186
          },
          {
            "path": "source/crates/wtime-runtime/src/execution.rs",
            "role": "source",
            "sha256": "1baf160513d2daf53706f2bc03a54f7d01777b75da65656c5a6bedb4e995f4a5",
            "size_bytes": 53186
          },
          {
            "path": "source/crates/wtime-runtime/src/execution/accounting.rs",
            "role": "source",
            "sha256": "c86761a09e3c98301f54c971aa93e08da97140832c0f58e279aca4a75bef7127",
            "size_bytes": 1294
          },
          {
            "path": "source/crates/wtime-runtime/src/execution/cursor.rs",
            "role": "source",
            "sha256": "ff93a3f0056d109366d2b77ea93d459f456d048c05b4bf05369305a3b491d3d3",
            "size_bytes": 12372
          },
          {
            "path": "source/crates/wtime-runtime/src/execution/effects.rs",
            "role": "source",
            "sha256": "640bf290be1691465e44f188213b644030dab6f8ca1599790f5138816e192bcd",
            "size_bytes": 5198
          },
          {
            "path": "source/crates/wtime-runtime/src/execution/plan.rs",
            "role": "source",
            "sha256": "68e43e6484a6c198529b333a6e5122526845544d660191121755c3108e125d7e",
            "size_bytes": 7410
          },
          {
            "path": "source/crates/wtime-runtime/src/gc.rs",
            "role": "source",
            "sha256": "7dcfcc9aaff6a333041d9d8a7323d1ce0fad90b382b5023e9caeeb52815b3471",
            "size_bytes": 14785
          },
          {
            "path": "source/crates/wtime-runtime/src/gc_exec.rs",
            "role": "source",
            "sha256": "56cf9173994d267fa4ead0af137b8f9f941f6b86a52d458f31be3ce7a32280fc",
            "size_bytes": 34395
          },
          {
            "path": "source/crates/wtime-runtime/src/lib.rs",
            "role": "source",
            "sha256": "997a0ec8d835c83567e13aa4c755ed30ad0e44a0bdb9e689c14753abdd47bb45",
            "size_bytes": 71696
          },
          {
            "path": "source/crates/wtime-runtime/src/linker.rs",
            "role": "source",
            "sha256": "39a2068993bcb65fb4434f42d02b56687e8ae31914840487ee4c35de07677806",
            "size_bytes": 22669
          },
          {
            "path": "source/crates/wtime-runtime/src/numeric.rs",
            "role": "source",
            "sha256": "5acd2cc6a4e9e9c23fd4bb25ab8a9fbc1fa1658d63f4a95e207a00963650dc92",
            "size_bytes": 10611
          },
          {
            "path": "source/crates/wtime-runtime/src/perf.rs",
            "role": "source",
            "sha256": "f523ab7bc803da4fdcd5365c7c310b572ff7704252c245530e37a21ac740f59b",
            "size_bytes": 6496
          },
          {
            "path": "source/crates/wtime-runtime/src/reference.rs",
            "role": "source",
            "sha256": "5afabf5f234dd6339a84be2440794ef0e181891f801d78a65a5703ccb304d7f9",
            "size_bytes": 29692
          },
          {
            "path": "source/crates/wtime-runtime/src/shared_memory.rs",
            "role": "source",
            "sha256": "6b1a5a02e49d2d73ee04865af8d6bac68c60305f1afc754ca9b70252750bce44",
            "size_bytes": 16933
          },
          {
            "path": "source/crates/wtime-runtime/src/simd.rs",
            "role": "source",
            "sha256": "84c223e6a546aa8955ff5214b9add03df4c94d25f3ffcd45b33a808d299334b1",
            "size_bytes": 54133
          },
          {
            "path": "source/crates/wtime-runtime/src/wide.rs",
            "role": "source",
            "sha256": "e0347c76d5a00a56396bd641ce4beb4f0d6a7500739f6de57283c1cbacc4224a",
            "size_bytes": 2260
          },
          {
            "path": "source/crates/wtime-runtime/tests/core.rs",
            "role": "source",
            "sha256": "54f6e18cff27f3da96ae0736d7910013883742f0215d35abb551ce324c78af1f",
            "size_bytes": 4617
          },
          {
            "path": "source/crates/wtime-runtime/tests/exceptions.rs",
            "role": "source",
            "sha256": "3cc47afeeb6428154f6d0a30db6362a009c45d292278f5f03ca60c53c3342a93",
            "size_bytes": 2533
          },
          {
            "path": "source/crates/wtime-runtime/tests/feature_profiles.rs",
            "role": "source",
            "sha256": "dafe483c28f8683332d198935818fff885827dfc5cfd1ad62f27330490ebc14a",
            "size_bytes": 10000
          },
          {
            "path": "source/crates/wtime-runtime/tests/performance.rs",
            "role": "source",
            "sha256": "e1a13a992b35c6e52fb13db92e10a0ea921d410f94a51f58ee36b8d3436ed58d",
            "size_bytes": 210
          },
          {
            "path": "source/crates/wtime-runtime/tests/performance/p016.rs",
            "role": "source",
            "sha256": "5ff37ef565ffc5c64c743723f1bfdabe85a315e784f1c81384f17b48a71dc21f",
            "size_bytes": 2678
          },
          {
            "path": "source/crates/wtime-runtime/tests/performance/p017.rs",
            "role": "source",
            "sha256": "4122d30ef0f042af886692d52de26c9a48d016f5f8e2a0bf67c6e9c9b84e9ef7",
            "size_bytes": 2343
          },
          {
            "path": "source/crates/wtime-runtime/tests/performance/p018.rs",
            "role": "source",
            "sha256": "e06da12e7539dbfc36bdfcc1981e87c8d926b08679f9bd2fce201987d5dafeae",
            "size_bytes": 7025
          },
          {
            "path": "source/crates/wtime-runtime/tests/performance/p019.rs",
            "role": "source",
            "sha256": "8294310dd4da5f9a7a05bf0bfee0d2be503ec52cdddbca4e21c6a3ee9d3b2715",
            "size_bytes": 2163
          },
          {
            "path": "source/crates/wtime-runtime/tests/performance/p020.rs",
            "role": "source",
            "sha256": "b6cb36f79e0d9702e7a8be4f81d254fcbd9d683d57c6baefe37f06951f611185",
            "size_bytes": 4180
          },
          {
            "path": "source/crates/wtime-runtime/tests/performance/p021.rs",
            "role": "source",
            "sha256": "fec19c012536565dbf2ba3e3f80f26b3fcc4e7d560a2218b5335552be2448a94",
            "size_bytes": 3340
          },
          {
            "path": "source/crates/wtime-runtime/tests/performance/p023.rs",
            "role": "source",
            "sha256": "6cbadd3bf64068c0ba819bed3be369c1334fb876075e9f3253c9d612666472f6",
            "size_bytes": 2940
          },
          {
            "path": "source/crates/wtime-runtime/tests/proposals.rs",
            "role": "source",
            "sha256": "8fffaa52d03e81fc7d88d0970617f1a0c8ba7602fc241ad9c65367133d2a9927",
            "size_bytes": 3397
          },
          {
            "path": "source/crates/wtime-types/Cargo.toml",
            "role": "source",
            "sha256": "e5c4d20d5d35fd4a15ce854f96602add82b86a1bbe4599aab69b0abcc7dc9031",
            "size_bytes": 151
          },
          {
            "path": "source/crates/wtime-types/src/error.rs",
            "role": "source",
            "sha256": "a25659ee8aefcf08babfd2e4b027b698176adc100da5336a9a28f2f0664d8a2c",
            "size_bytes": 2968
          },
          {
            "path": "source/crates/wtime-types/src/features.rs",
            "role": "source",
            "sha256": "3c465ce7c325c9110d42a6df08ef20dfc90122e8a86a2fdbb85a6063e82b2dc5",
            "size_bytes": 7970
          },
          {
            "path": "source/crates/wtime-types/src/indices.rs",
            "role": "source",
            "sha256": "249359a286195771d58ad78daa775c8164de6419100fea38e6f6f172f92e2e40",
            "size_bytes": 4919
          },
          {
            "path": "source/crates/wtime-types/src/lib.rs",
            "role": "source",
            "sha256": "9f1ce2f00f37fba42288e5f4c78262234bf6ab453f7061d7b4907bbe4fdeca4d",
            "size_bytes": 1860
          },
          {
            "path": "source/crates/wtime-types/src/types.rs",
            "role": "source",
            "sha256": "d5d6a398f344494ada0d065afcab4bdee157f4803351a2ce6752ee3fc0d4e266",
            "size_bytes": 6611
          },
          {
            "path": "source/crates/wtime-validate/Cargo.toml",
            "role": "source",
            "sha256": "fed963beb5aa95d1c242f4131509edd95069e1b27c4e69f84f969357a04a0639",
            "size_bytes": 213
          },
          {
            "path": "source/crates/wtime-validate/src/acceptance.rs",
            "role": "source",
            "sha256": "322c98e589f396f77097a7acf1dabc666f72ccbba8936703cb115ed7befd9621",
            "size_bytes": 12645
          },
          {
            "path": "source/crates/wtime-validate/src/atomics.rs",
            "role": "source",
            "sha256": "51d71f78adc9f140c79dad601afe64f4bc42fb2cf896a1d753a480f57f70bbc4",
            "size_bytes": 1933
          },
          {
            "path": "source/crates/wtime-validate/src/continuations.rs",
            "role": "source",
            "sha256": "185cf7f137c9dbd57062f95f3d36715e92420a44f5579fb09a34b7dd65029969",
            "size_bytes": 7014
          },
          {
            "path": "source/crates/wtime-validate/src/exceptions.rs",
            "role": "source",
            "sha256": "5afe91db3e03444019a0181f639f683c8969563ac20376c6485395c79f46c99b",
            "size_bytes": 5080
          },
          {
            "path": "source/crates/wtime-validate/src/gc.rs",
            "role": "source",
            "sha256": "e46a4440d3ffbea02c4905ffffa75e9582c3193697a8484ec464fdaa7a34f21f",
            "size_bytes": 18820
          },
          {
            "path": "source/crates/wtime-validate/src/lib.rs",
            "role": "source",
            "sha256": "6d6fd6442fdad200c494fc43775dc003ee18fbfa63632c7056e238777a283455",
            "size_bytes": 72331
          },
          {
            "path": "source/crates/wtime-validate/src/types.rs",
            "role": "source",
            "sha256": "2d6f8561b6d9d8eeff0bbb76510e9b9a6159d9646d7c01a01fda6f51207ec7a0",
            "size_bytes": 19700
          },
          {
            "path": "source/crates/wtime-validate/tests/exceptions.rs",
            "role": "source",
            "sha256": "061d15fe56d6028e54671c5dd5d86ee1069f2ac99183886a3c3441356d4da290",
            "size_bytes": 1022
          },
          {
            "path": "source/crates/wtime-validate/tests/policy.rs",
            "role": "source",
            "sha256": "4a4c47509bd9da3850b1ab76235d585db2c7aba826f1f89ec9055a2d888774c3",
            "size_bytes": 942
          },
          {
            "path": "source/rust-toolchain.toml",
            "role": "source",
            "sha256": "10c67ce85265f8fc5b1a52b416af1a553e56ad96ca9843da6579e2854741e084",
            "size_bytes": 66
          },
          {
            "path": "source/tools/wtime-bench/Cargo.lock",
            "role": "source",
            "sha256": "4d1a59eab7ae2ce8c6abcd203afb3a20d0787e6cd536b86a919fd133196c5431",
            "size_bytes": 26657
          },
          {
            "path": "source/tools/wtime-bench/Cargo.toml",
            "role": "source",
            "sha256": "389e92491b6caca59c924bd41a46bee79828b41459f2c3712411e3e40bcb5a3b",
            "size_bytes": 991
          },
          {
            "path": "source/tools/wtime-bench/README.md",
            "role": "source",
            "sha256": "4a679dbd985d5534f4cbaf26aaa92dc397852b12a4af0687c73aea085c3608c7",
            "size_bytes": 3669
          },
          {
            "path": "source/tools/wtime-bench/baseline.py",
            "role": "source",
            "sha256": "23374f433ac86a0df27876699f8e6942d54bde0c6ae6cd73cd424c01e9a0bb1e",
            "size_bytes": 28057
          },
          {
            "path": "source/tools/wtime-bench/run.py",
            "role": "source",
            "sha256": "f793c150ae38031467c91a70cddf66e85b6cd6d9782e227b773b4da6ff564bea",
            "size_bytes": 10152
          },
          {
            "path": "source/tools/wtime-bench/src/main.rs",
            "role": "source",
            "sha256": "9f9496929d3c60165fa061030336397d1825d9b0545a70aadeaf07208c3768e5",
            "size_bytes": 9474
          },
          {
            "path": "source/tools/wtime-bench/src/memory_driver.rs",
            "role": "source",
            "sha256": "1d3b6604fd98b2397906dbd085bf1bd0f5eadb79fde230dbaf22339b3690bc7c",
            "size_bytes": 11116
          },
          {
            "path": "source/tools/wtime-bench/src/protocol.rs",
            "role": "source",
            "sha256": "b7a9e62658fa6e78d400ade48252dd2fd45795eaf3d5afa8ce6fe8daf5f8695e",
            "size_bytes": 28371
          },
          {
            "path": "source/tools/wtime-bench/src/scalar_driver.rs",
            "role": "source",
            "sha256": "c43a3369b6dfac8b57aaf789dbabd9da8dab4f28751571a683b970dd09225a58",
            "size_bytes": 21137
          },
          {
            "path": "source/tools/wtime-conformance/Cargo.toml",
            "role": "source",
            "sha256": "b5ecbd6c6fa3f2081f1b600433143b2458e1c359c0bda3968cc7d3201361b141",
            "size_bytes": 422
          },
          {
            "path": "source/tools/wtime-conformance/build.rs",
            "role": "source",
            "sha256": "1e37a8f6ec1af7964228b03abb3e9675c2c5218edf3bb43a94d76243cc893ef1",
            "size_bytes": 1428
          },
          {
            "path": "source/tools/wtime-conformance/src/adapter.rs",
            "role": "source",
            "sha256": "d099a8b490b9b16eac942988ec32b6e59606da4be2faadfde79cd930e381070b",
            "size_bytes": 6725
          },
          {
            "path": "source/tools/wtime-conformance/src/bin/wtime-allocation-check.rs",
            "role": "source",
            "sha256": "1a20e70ac081c7792732bc1cffded19935e19d0141bca5ea78a6b6086f4c6034",
            "size_bytes": 1414
          },
          {
            "path": "source/tools/wtime-conformance/src/bin/wtime-probe.rs",
            "role": "source",
            "sha256": "8581162a05834657c2715c6593069a9d1f9421029c5e09a993e6bb4d02419de5",
            "size_bytes": 4424
          },
          {
            "path": "source/tools/wtime-conformance/src/cli.rs",
            "role": "source",
            "sha256": "fa89e465b879d94a185072e6587a1323b242390a5c5e62b388c147dc679e20e2",
            "size_bytes": 8939
          },
          {
            "path": "source/tools/wtime-conformance/src/corpus.rs",
            "role": "source",
            "sha256": "b1d2a7947b5904b9bd80c0109bc8b5e0f5f8329ba92a693db3f4409cd93cbc1e",
            "size_bytes": 2079
          },
          {
            "path": "source/tools/wtime-conformance/src/main.rs",
            "role": "source",
            "sha256": "ecf37373cd8988e4f45569d49a0a971fc85a5b687c091ae2ec7964ec8f35c169",
            "size_bytes": 182
          },
          {
            "path": "source/tools/wtime-conformance/src/report.rs",
            "role": "source",
            "sha256": "5621e3ced6bb5b68fcd1151d54d7f1f5c410e328cec2a752e46d2de77b20ba60",
            "size_bytes": 2830
          },
          {
            "path": "source/tools/wtime-conformance/src/script.rs",
            "role": "source",
            "sha256": "8c11af05587c584895b9e945c1d0075181b9fb108ef60ae311416cb2b1971888",
            "size_bytes": 38455
          },
          {
            "path": "source/tools/wtime-conformance/src/spectest.rs",
            "role": "source",
            "sha256": "1403166f4be6ce443b47d4a7ba4a44da553c67e280dfa2876b3f2ad1e64e8883",
            "size_bytes": 2698
          },
          {
            "path": "source/tools/wtime-conformance/tests/annotations.rs",
            "role": "source",
            "sha256": "5eb54a28e57e7e929bcb2342d65c28e23c2ae112b7c10b36f707fa580f489b08",
            "size_bytes": 2316
          },
          {
            "path": "source/tools/wtime-conformance/tests/interactions.rs",
            "role": "source",
            "sha256": "fd36710c8cd2ef7f3a2883d3f0ab6cbfecace3ae80adf693095228e4f4641938",
            "size_bytes": 15026
          }
        ],
        "manifest_sha256": "6a284d5ed122e8bfe7cd3359f941441bc91835e8de846e00e84027821ff31a81"
      },
      "path": "/workspace/wtime/target/performance/P002-checkpoint-before"
    },
    "baseline_binary": {
      "sha256": "489cbfdc0690c4f2d52e30df4019a1cca356b27d2d2d818b80e22176b4f48f43"
    },
    "baseline_candidate_binary_hash_equal": true,
    "candidate_binary": {
      "sha256": "489cbfdc0690c4f2d52e30df4019a1cca356b27d2d2d818b80e22176b4f48f43"
    },
    "candidate_source_map": {
      "canonical_sha256": "97b3f946b13c7db006b2a95b9db3fae8b38e286cf1db82a6003ba238a9cdca67",
      "coverage_note": "Candidate source map and frozen archive have different declared file coverage; common paths match exactly.",
      "file_count": 85,
      "shared_path_count": 68,
      "shared_path_hash_mismatches": []
    },
    "fixtures": [
      {
        "id": "call_overhead",
        "path": "/workspace/wtime/target/performance/P002-checkpoint-before/fixtures/call_overhead.wasm",
        "sha256": "0815ca7d6026206d0773fe10c6b9643862bfffe8ba361c9789369ef9668ee746",
        "size_bytes": 40
      },
      {
        "id": "integer_loop",
        "path": "/workspace/wtime/target/performance/P002-checkpoint-before/fixtures/integer_loop.wasm",
        "sha256": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c",
        "size_bytes": 122
      },
      {
        "id": "memory_fill_scan",
        "path": "/workspace/wtime/target/performance/P002-checkpoint-before/fixtures/memory_fill_scan.wasm",
        "sha256": "1c933d404fe9feb93bb664af3bbb60a50319cdd6a2c29d442aa9ab7799f5e65f",
        "size_bytes": 171
      },
      {
        "id": "branch_calls",
        "path": "/workspace/wtime/target/performance/P002-checkpoint-before/fixtures/branch_calls.wasm",
        "sha256": "9b58d26f201900b5b6cce684133a98ab68b3cf55d979b64ce895e914e9d5327b",
        "size_bytes": 157
      },
      {
        "id": "prepare_large",
        "path": "/workspace/wtime/target/performance/P002-checkpoint-before/fixtures/prepare_large.wasm",
        "sha256": "c489fe06970f8e1c60e4992774aaadb571f676833d6579de1f67a5facf3fe9b0",
        "size_bytes": 215036
      }
    ],
    "journal": {
      "header_metadata_equals_report_metadata": true,
      "identity_digest_valid": true,
      "identity_sha256": "d833b584bf6a17892669adad8dd6d088a8289080c7e0c9ba4f71b9dcf9b6be86",
      "ordered_report_pair_equivalence": true,
      "path": "/workspace/wtime/target/performance/P002-checkpoint/checkpoint.jsonl",
      "record_count": 1260,
      "schema": "wtime-compare-checkpoint-v1",
      "sha256": "eb8d173df1133e1b0fa2ad8f0e9d05a6e5ded6e86e5d5958b9639d3607a19e89",
      "size_bytes": 831892,
      "unique_pair_ids": 1260
    },
    "protocol_sha256": "b7a9e62658fa6e78d400ade48252dd2fd45795eaf3d5afa8ce6fe8daf5f8695e",
    "report": {
      "path": "/workspace/wtime/target/performance/P002-checkpoint/report.json",
      "sha256": "2ae39c4a66a10efa6ce68ca9e0f6dc4951e8494021724d8dbef5f4496867fd70",
      "size_bytes": 1321858
    }
  },
  "commands": [
    {
      "command": "python3 -m unittest discover -s tools/wtime-bench/tests -p test_p002*.py -v",
      "exit_code": 0,
      "failed": 0,
      "passed": 9,
      "skipped": 0
    },
    {
      "command": "cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml -j 4",
      "exit_code": 0
    },
    {
      "command": "python3 tools/wtime-bench/compare.py --baseline target/performance/baseline-adapter --candidate tools/wtime-bench/target/release/wtime-bench --selection original-mvp --output target/performance/P002",
      "exit_code": 0,
      "groups_per_run": 21,
      "paired_observations": 1260,
      "pairs_per_group": 30,
      "runs": 2,
      "timing_rows": 2520
    },
    {
      "command": "python3 -m unittest discover -s tools/wtime-bench/tests -p 'test_p002*.py' -v",
      "exit_code": 0,
      "passed": 17,
      "failed": 0,
      "skipped": 0
    },
    {
      "command": "python3 tools/wtime-bench/compare.py --baseline target/performance/P002-checkpoint-before --candidate tools/wtime-bench/target/release/wtime-bench --candidate-mode decoded --selection original-mvp --output target/performance/P002-checkpoint",
      "exit_code": 0,
      "pairs": 1260,
      "runs": 2
    }
  ]
}
```
