# P135 — Add a fresh-entry Register batch protocol

Status: **done**

Outcome: **contract_only**

Kind: `foundation` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P134 — Integrate opt-in lazy Register entry](P134-opt-in-lazy-register-entry.md)

P134 supplies the explicit eager and adaptive Register Engine construction paths, bounded shared preparation cache, and execution diagnostics. P135 adds a current-only driver protocol that measures a fresh compile, Store, instantiation, and exact invocation count as one cold-total observation. It does not select a threshold or make a performance claim. P066 remains the measured candidate gate and adds P135 as a prerequisite while retaining all existing gates. A later P136 is needed for settings forwarding, a source-bound Register adapter, and paired batch collection; P137 owns analysis of total-cost and cold-code distributions and cache churn.

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), and contracts **C1, C2, C3, C4, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md). Inspect the current `tools/wtime-bench/src/protocol.rs` single-case dispatcher and its `--protocol` capability output before implementation.

- `tools/wtime-bench/src/entry_batch_driver.rs` (new)
- `tools/wtime-bench/src/protocol.rs` (bounded dispatch and optional capability)
- `tools/wtime-bench/fixtures/p135/counter.wat` (new, MIT counter fixture)
- `tools/wtime-bench/fixtures/p135/counter.wasm` (encoded counter fixture)
- `tools/wtime-bench/fixtures/p135/late_bad_result.wat` (new, MIT oracle fixture)
- `tools/wtime-bench/fixtures/p135/late_bad_result.wasm` (encoded oracle fixture)
- `tools/wtime-bench/fixtures/p135/provenance.json` (fixture provenance and hashes)
- `tools/wtime-bench/fixtures/p135/LICENSE.txt` (MIT license)
- `tools/wtime-bench/tests/test_p135_entry_batch.py` (new)
- `tools/verification/tests/test_p135_entry_batch.py` (new source-bound verification)

Keep the Rust tests inline in `entry_batch_driver.rs`; do not create a separate Rust test file. The MIT counter WAT exports a one-page memory named `count`, initialized to zero, and increments the little-endian i32 at byte offset zero on every call. The separate `late_bad_result` WAT returns 12345 on its second call and the ordinary expected result on all other calls. `provenance.json` records both WAT/Wasm hashes, licenses, and encoding details. One integrator owns the new driver leaf, protocol registration, fixture, and its verification tests. Keep implementation to at most two focused patches: batch/diagnostic protocol and dispatch, then meaningful Rust/CLI/source-bound checks. Do not change the runtime library, execution policy semantics, workload registry, adapters, paired collection, or analyzers. Add no dependencies or public core APIs.

## Input/output contract

The MIT `counter.wat`/`.wasm` fixture exports a one-page memory named `count`; each call increments the little-endian i32 at byte offset zero. The separate `late_bad_result.wat`/`.wasm` fixture returns 12345 on its second call and the ordinary expected result on all others. `provenance.json` binds both WAT and Wasm hashes, license, and encoding details. The counter-memory assertion is fixture-specific verification, so future kernels need not export `count`.

Add the current-only command:

```text
--entry-batch FILE CASE INPUT EXPECTED CALLS register eager|adaptive THRESHOLD
```

Accept `CALLS` only from **1, 10, 100, 10000**. The execution mode is exactly `register`. `eager` requires threshold `0` and uses the existing eager Register Engine constructor; `adaptive` requires a positive threshold and uses the P134 opt-in constructor. Reject malformed, zero, negative, oversized, or unsupported values before guest execution. Use the existing MVP profile and omitted fuel configuration, with no implicit fuel budget or alternate profile selector.

For every batch invocation, build a fresh Engine before the timer, but start a new measured lifetime at `Engine::compile`. Inside one enclosing `total_ns` interval, compile the module, create a fresh Store, instantiate it, and invoke the same exported function exactly `CALLS` times. Report `prepare_ns` for `Engine::compile`, `instantiate_ns` for Store/linker/instance setup, and `invoke_ns` for the exact call loop; report actual invocation count and the strategy/threshold. `total_ns` measures the whole enclosed lifetime rather than the sum of separately rounded phases. Load the input file, construct Engine/configuration, and parse arguments before the timer. Do not preflight, warm up, calibrate, or invoke the measured Module before or after the timed loop. Check each individual result against `EXPECTED`; abort on the first mismatch. After the timed loop, read the admitted counter fixture’s exported `count` memory at offset zero without another guest invocation and require the little-endian i32 to equal `CALLS`, proving the protocol added no warmup or extra entry. Keep this fixture-specific assertion in the protocol verification so other future kernels need not export `count`. Keep total and phase timings in nanoseconds and report no divided `ns_per_operation` field.

The production batch executable must be uninstrumented. The additive JSON result includes schema/version, driver and case identity, input/expected/result, `mode:"register"`, strategy, threshold, requested and actual call counts, `instrumented:false`, and `total_ns`, `prepare_ns`, `instantiate_ns`, `invoke_ns`. It must not report a synthetic compile count or imply that engine construction is part of the timed total.

Add a separate diagnostic command using the same fixture and strategy inputs:

```text
--entry-diagnostic FILE CASE INPUT EXPECTED CALLS register eager|adaptive THRESHOLD
```

Build it with `execution-reference` instrumentation. Execute exactly the requested number of calls on a fresh module/store, take counter snapshots immediately around each call, and report per-call deltas for `register_dispatches`, `register_exits`, and `fallback_frames` (read from the existing `Store::register_fallback_frames` accessor), plus the one-based `first_register_call` whose dispatch delta is nonzero (or `null` when none). The diagnostic JSON is explicitly instrumented and contains no timing fields. Do not call a dispatch count a compilation count or label the field `promotion`; this driver observes dispatch only. Do not add any preflight invocation that changes entry hotness.

Keep existing `--single-case` behavior and ordinary mode/capability query response unchanged, except for an optional capability that advertises the new entry-batch and diagnostic protocols when their required features are compiled. No timing comparison, policy choice, analyzer, workload-registry edit, P001 adapter, or paired sample collection belongs here.

## Implementation recipe

1. Inspect current `protocol::try_main`, engine construction, module instantiation, invocation, mode reporting, and execution-reference counter APIs. Add a leaf `entry_batch_driver.rs` and route only the two new flags to it; preserve existing single-case and mode-query bytes/semantics except the optional protocol capability advertisement.
2. Parse the command completely before guest work. Enforce the exact call-count set, `register` mode, and strategy/threshold pairing. Construct a fresh configured Engine outside the measured interval; then enclose module compilation, a new Store and instance, and exactly N checked invocations in one total timer. Keep phase timings additive observations, make every call's result oracle-visible, and emit one complete JSON line.
3. Implement the separate diagnostic path under `execution-reference`: create a fresh execution lifetime, snapshot counters around each actual call, emit exact per-call deltas and the first dispatch call, and omit all timing fields. Avoid preflight calls and do not infer builder/compile/promotion counts from dispatch counters.
4. Add source-bound verification that binds the command argv, fixture hash, executable hash, requested settings, and parsed JSON to the same run; reject instrumented timing executables, missing/extra calls, stale output, result mismatches, phase fields in diagnostics, timing fields in diagnostic mode, and any divided per-operation timing.
5. Add at least four meaningful Rust/CLI checks and run real CLI probes for eager and adaptive strategies at each accepted call count **1, 10, 100, and 10000**, using adaptive threshold **3**. Prove each probe starts with a fresh module/Store/Engine lifecycle, records exact actual calls, validates every return value, and contains no warmup or preflight call. Test the diagnostic path separately with exact per-call counter deltas and no timing fields.

## Meaningful cases

- `p135_entry_batch_parser_rejects_unsupported_counts_and_strategy_pairs`: accept only the four declared call counts, the `register` mode, eager threshold zero, and adaptive positive thresholds; reject malformed and inconsistent settings before reading or executing guest code.
- `p135_fresh_batch_times_compile_instantiation_and_exact_calls`: on `counter.wasm`, a fresh Engine is created outside timing and one fresh compile/Store/instance plus exactly N oracle-checked calls are enclosed by total timing; the post-loop exported memory counter equals N, and JSON reports actual phase and call counts without per-operation division.
- `p135_oracle_checks_every_call_and_stops_on_mismatch`: each invocation is compared with the expected result; `late_bad_result.wasm` returns 12345 on call two and 8 on other calls for input 7, so a batch expecting 8 aborts on call two rather than allowing a later correct result to hide it.
- `p135_diagnostic_reports_dispatch_deltas_without_timing_or_promotion_claims`: fresh instrumented diagnostic runs expose exact per-call deltas and one-based first dispatch or null, with no timing fields and no compile/promotion labels.
- `p135_cli_matrix_proves_fresh_eager_and_adaptive_lifetimes`: source-bound executable probes run eager and adaptive threshold-three commands at calls 1/10/100/10000; each command is a separate process and fresh measured lifetime, and every row validates argv, fixture/executable hashes, exact calls, oracle result and uninstrumented timing output.

## Verification

Use locked builds and tests in the exact feature configurations. The production executable uses `register-executor` without `execution-reference` or diagnostic instrumentation. The diagnostic executable includes `execution-reference`. Test both command paths and retain the pre-existing single-case and mode-query protocol checks. The CLI matrix is a protocol smoke check, not a timing result, and must run each eager/adaptive/count combination as a fresh process and fresh measured lifetime.

```sh
cargo test --locked --manifest-path tools/wtime-bench/Cargo.toml --features register-executor p135_ -- --nocapture
cargo test --locked --manifest-path tools/wtime-bench/Cargo.toml --features register-executor,execution-reference p135_ -- --nocapture
python3 -B tools/wtime-bench/tests/test_p135_entry_batch.py -v
python3 -B tools/verification/tests/test_p135_entry_batch.py -v
```

## Measurement and decision

Selection: `register`. Explicit execution setting: `register`. Decision rule: `integrity`.

P135 is a current-only cold-total protocol foundation. It makes no speed claim and does not choose an adaptive threshold. P066 owns candidate measurement and the full decision gate. P136 will bind settings into paired collection; P137 will analyze the new total metric and actual cold-code, cache churn, and call-count distributions. Existing `--single-case` per-call measurements remain unchanged.

## Before editing

Verify the immutable P001 archive and completed P134 correctness record. Create a fresh immutable Register archive for the immediate P134-successor source before edits, including exact runtime and driver sources, executable, fixture identities, and hashes. Record the baseline commands and selected CLI fixture/inputs; do not overwrite archives with different identities.

```sh
python3 -B tools/wtime-bench/baseline.py --output target/performance/P135-before --mode register
```

## Completion evidence

Record P001 and P135-before identities, runtime/driver/test/verification source hashes, executable and fixture hashes, exact argv and feature settings, all eight eager/adaptive call-count probes and their actual call counts, and separate diagnostic evidence. Preserve command logs and parsed outputs. Confirm production timing binaries are uninstrumented and diagnostics are timing-free. These smoke observations are not paired measurements or acceptance evidence; keep status pending until all cases and checks pass.

## Executed evidence

The successful shared closure and its durable records bind all probe outputs, test executions, source identities, and the reused P134 build-budget checks. These are protocol correctness smoke results; they make no performance claim.

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/P135/shared-finalization/attempt-0002/evidence.json": "c62356a8b997bba379dfd39859676597e5f5379e3ebeac7c33cff5039c2bbc79"
  },
  "durable_export": {
    "architecture": {
      "bytes": 2516,
      "path": "/workspace/wtime/docs/architecture/register-entry-benchmark.md",
      "sha256": "b49d27bdbf15007ce50c338dd8e174f5d110de3821f858e6af3c7c03c88a522d"
    },
    "build_budget_reuse": {
      "core_build_budgets_reused": true,
      "new_build_timing_recorded": false,
      "reports": {
        "default": {
          "budget_pass": true,
          "default_dependency_closure_pass": true,
          "features": [],
          "historical_build_reused": true,
          "report": {
            "bytes": 330120,
            "path": "/workspace/wtime/target/verification/runs/P134/verify/attempt-0001/default-build.json",
            "sha256": "800490448cea73d37b80ef6cae11b84cbbb0ff5443ecf6f12fe53a3baa828c05"
          },
          "source_file_count": 730,
          "source_sha256": "8daf832a4cbe4f63b4aba57254018343e88fd7bcc6013bc3cd9e8f93353158cb",
          "stripped_bytes": 1000960
        },
        "register": {
          "budget_pass": true,
          "default_dependency_closure_pass": true,
          "features": [
            "wtime-runtime/register-executor"
          ],
          "historical_build_reused": true,
          "report": {
            "bytes": 330283,
            "path": "/workspace/wtime/target/verification/runs/P134/verify/attempt-0001/register-build.json",
            "sha256": "a7576f9e8c2abc64372833064a6802807ed73b9a4a9b073c00bd0ef9f13f7796"
          },
          "source_file_count": 730,
          "source_sha256": "8daf832a4cbe4f63b4aba57254018343e88fd7bcc6013bc3cd9e8f93353158cb",
          "stripped_bytes": 1090512
        }
      },
      "source_file_count": 730,
      "source_inputs_sha256": "71f313988ae1130c99950ca4a15d898920dc89987e9590c408812539a423bc0c"
    },
    "canonical_artifacts": {
      "P135-completion.json": {
        "bytes": 503,
        "path": "/workspace/wtime/docs/benchmarks/performance/P135-completion.json",
        "sha256": "7844180c45bfb71527f74a5a1ff63a9bf2c2236ef33fd302b723198ef4872903"
      },
      "P135-evidence.json": {
        "bytes": 1539,
        "path": "/workspace/wtime/docs/benchmarks/performance/P135-evidence.json",
        "sha256": "c62356a8b997bba379dfd39859676597e5f5379e3ebeac7c33cff5039c2bbc79"
      },
      "P135-freeze.json": {
        "bytes": 20069,
        "path": "/workspace/wtime/docs/benchmarks/performance/P135-freeze.json",
        "sha256": "770aaec64afa3f6ead630852834bf205b96e53fbf07085f9388a279e81654356"
      },
      "P135-observations.json.gz": {
        "bytes": 59359,
        "path": "/workspace/wtime/docs/benchmarks/performance/P135-observations.json.gz",
        "sha256": "6658740121c5f73fafc27cb6850e84a11b8d1c274c2419f23cac4b6e8ce7f75a"
      },
      "P135-proof.json": {
        "bytes": 106679,
        "path": "/workspace/wtime/docs/benchmarks/performance/P135-proof.json",
        "sha256": "0c7081bbb2948f7c2bb34da7933b46f141024a443c98ac770d13f1a7ab9477c0"
      },
      "P135-records.json": {
        "bytes": 41014,
        "path": "/workspace/wtime/docs/benchmarks/performance/P135-records.json",
        "sha256": "cfd84839aad6df2913ca6ee31ecf98ad50f991405d6e156c64722684f6c05f71"
      }
    },
    "child_runs": {
      "all_stdout_stderr_preserved": true,
      "argv_bound_rows": 27,
      "diagnostic": 8,
      "negative": 11,
      "production": 8
    },
    "default_core": {
      "added_sources": [],
      "changed_sources": [],
      "runtime_source_file_count": 150,
      "runtime_source_map_sha256": "88bdcaa3e3acbfbc0a9443a59ae58131686c4809a73ef36b3a790c840866dca9",
      "unchanged_source_count": 150
    },
    "diagnostic_binary": {
      "bytes": 17108768,
      "path": "target/verification/runs/P135/verify/attempt-0002/diagnostic-wtime-bench",
      "sha256": "3bfadf9201c627909888c9b77e9384c8596b7f7e779b22a7dd1b0db8fa329c42"
    },
    "fixtures": {
      "counter": {
        "bytes": 84,
        "path": "tools/wtime-bench/fixtures/p135/counter.wasm",
        "sha256": "6b365cbdbe8c189ad1b19a8cbdc6304012207dbf0f83c0de6824f8c8b8f07776"
      },
      "late_bad_result": {
        "bytes": 100,
        "path": "tools/wtime-bench/fixtures/p135/late_bad_result.wasm",
        "sha256": "71fb4bd0e45f07dd962b5513f36ef4c1bf4819820595585c872d0961df3763b3"
      },
      "license": {
        "bytes": 1079,
        "path": "tools/wtime-bench/fixtures/p135/LICENSE.txt",
        "sha256": "c28998034a6eb55fc54c810c8fbf560449a43e4057600b77d5ff9aed1b4f7eca"
      },
      "provenance": {
        "bytes": 3028,
        "path": "tools/wtime-bench/fixtures/p135/provenance.json",
        "sha256": "893200b8fb8df8edfac1a12f6e921f5f1543d4c539b74baef5d2be99ba481829"
      }
    },
    "helper_archive": {
      "bytes": 49978,
      "command": {
        "bytes": 187,
        "path": "target/recovery/P135-export-0001/attempt-0002/command.json",
        "sha256": "1d41dac8ed5146d6e6d25f937b31b643ba474d7c0c73256a6a06d91c8672952d"
      },
      "invocation": {
        "bytes": 436,
        "path": "target/recovery/P135-export-0001/attempt-0002/invocation.json",
        "sha256": "ac8127698a1ae0b5d9bdbcc9e74453308f562e109045e1addee09d5b7c55e448"
      },
      "sha256": "cea72c11cbe2529bc9ef8624f752aaf6ffeff65e9660400e9bc1542e540cb63a",
      "stderr_log": {
        "bytes": 0,
        "path": "target/recovery/P135-export-0001/attempt-0002/stderr.log",
        "sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
      },
      "stdout_log": {
        "bytes": 135,
        "path": "target/recovery/P135-export-0001/attempt-0002/stdout.log",
        "sha256": "973dddea9139aabd47fbf5449a345d44c51a73fdd292eb872b3f3f3e025259b4"
      }
    },
    "interpretation": "CLI smoke protocol correctness only; no paired timings, threshold selection, or performance promotion",
    "observations_archive": {
      "compressed_bytes": 59359,
      "compressed_sha256": "6658740121c5f73fafc27cb6850e84a11b8d1c274c2419f23cac4b6e8ce7f75a",
      "copy_path": "docs/benchmarks/performance/P135-observations.json.gz",
      "gzip_mtime": 0,
      "source": {
        "bytes": 1767354,
        "path": "target/verification/runs/P135/verify/attempt-0002/proof/observations.json",
        "sha256": "c6c26f3df5409cd90fd4417d82624e55b234c71afa2c6ade105bb2b9c192cb12"
      },
      "uncompressed_bytes": 1767354,
      "uncompressed_sha256": "c6c26f3df5409cd90fd4417d82624e55b234c71afa2c6ade105bb2b9c192cb12"
    },
    "phase_receipts": {
      "freeze": {
        "bytes": 394876,
        "path": "target/verification/runs/P135/freeze/attempt-0002/receipt.json",
        "sha256": "a66f6957cb5dc95afe9f60879371ab75e1aba9396f00b779357f8a0f06cc263c"
      },
      "verify": {
        "bytes": 154612,
        "path": "target/verification/runs/P135/verify/attempt-0002/receipt.json",
        "sha256": "65132a83b9c73b041755ab6a1b7f6a911204e04f1ef7a18b7dc5541aacf31bc5"
      }
    },
    "production_binary": {
      "bytes": 17499768,
      "path": "target/verification/runs/P135/verify/attempt-0002/production-wtime-bench",
      "sha256": "9a8138721940cd362308ed671fd41f22d53dcd1a457250f3297fdc946b5fb7c7"
    },
    "proof_counts": {
      "cli_matrix_rows": 8,
      "diagnostic_rows": 8,
      "named_cases": 5,
      "negative_rows": 11
    },
    "records": {
      "bytes": 41014,
      "path": "/workspace/wtime/docs/benchmarks/performance/P135-records.json",
      "sha256": "cfd84839aad6df2913ca6ee31ecf98ad50f991405d6e156c64722684f6c05f71"
    },
    "tests": {
      "groups": {
        "diagnostic-tests": {
          "failed": 0,
          "ignored": 0,
          "passed": 4
        },
        "production-tests": {
          "failed": 0,
          "ignored": 0,
          "passed": 4
        },
        "source-bound-unit-tests": {
          "failed": 0,
          "ignored": 0,
          "passed": 4
        }
      },
      "python_cli": {
        "failed": 0,
        "ignored": 0,
        "names": [
          "test_p135_cli_matrix_proves_fresh_eager_and_adaptive_lifetimes",
          "test_p135_fixture_bytes_are_bound_by_provenance",
          "test_p135_fixture_profile_is_mvp_compatible"
        ],
        "passed": 3
      },
      "totals": {
        "failed": 0,
        "ignored": 0,
        "passed": 15
      }
    }
  },
  "evidence": [
    {
      "bytes": 1539,
      "path": "/workspace/wtime/target/performance/P135/shared-finalization/attempt-0002/evidence.json",
      "sha256": "c62356a8b997bba379dfd39859676597e5f5379e3ebeac7c33cff5039c2bbc79"
    }
  ],
  "outcome": "contract_only",
  "status": "done",
  "task": "P135"
}
```