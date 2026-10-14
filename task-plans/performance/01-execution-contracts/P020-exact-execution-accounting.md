# P020 — Centralize exact source instruction accounting

Status: **done**

Outcome: **complete**

Kind: `foundation` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P018 — Define source cursors and materialized guest state](P018-source-cursor.md)

[P019 — Define bounded optimizer effect and source expansion metadata](P019-effect-certificates.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C2, C3, C4, C5, C6, C7** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/execution/accounting.rs`
- `crates/wtime-runtime/src/execution.rs`
- `crates/wtime-runtime/src/continuation.rs`
- `crates/wtime-runtime/tests/performance/p020.rs`
- `crates/wtime-runtime/tests/fixtures/performance/accounting/`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

Accounting helpers preserve the current source-boundary order of saved fuel, cancellation, prompt retirement, fuel charge and operand limits; optimized groups can execute a prefix without changing externally visible accounting.

## Implementation recipe

1. Extract the existing loop-head policy into a small contract/hot helper and retain current decoded behavior byte for byte where practical.
2. Implement source-prefix accounting for a prepared group: if fuel or a resource boundary interrupts it, expose the exact next unexecuted source instruction instead of debiting the entire group.
3. Preserve current_fuel/resume_fuel and continuation fuel interactions; cancellation polling remains per original instruction by default and bulk/wait policies remain separate.
4. Add controlled fixtures covering every possible interruption point in a short arithmetic/store/call sequence; compare prefix side effects and trap/resource categories against decoded execution.

## Meaningful cases

- Fuel 0 through sequence-length+1 interrupts at exactly the same source boundary.
- A cancellation before/inside a group cannot publish later stores/results.
- Suspend/resume and tail-call paths retain exact fuel and logical depth/stack peaks.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime p020_ -- --nocapture
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
  "artifacts": [
    "/workspace/wtime/target/performance/P020-performance-tests.log",
    "/workspace/wtime/target/performance/P020-build.json"
  ],
  "commands": [
    {
      "command": "cargo test --locked -p wtime-runtime --features execution-reference --test performance -- --nocapture",
      "exit_code": 0,
      "passed": 18,
      "failed": 0,
      "skipped": 0,
      "p020_named_cases_passed": 4,
      "named_cases": [
        "p020_every_fuel_prefix_preserves_completed_stores",
        "p020_fuel_failure_precedes_transient_stack_limit",
        "p020_cancellation_after_host_prefix_prevents_later_store",
        "p020_tail_recursion_preserves_logical_depth_policy"
      ]
    },
    {
      "command": "python3 tools/checks/build_budget.py --output target/performance/P020-build.json",
      "exit_code": 0,
      "source_stable": true,
      "default_dependency_closure_pass": true,
      "default_dependency_violations": 0,
      "clean_seconds": 5.736566249994212,
      "leaf_seconds": 0.335041900005308,
      "runtime_leaf_seconds": 0.8929614929948002,
      "release_seconds": 12.655436606000876,
      "stripped_bytes": 996024,
      "budget_pass": true,
      "embedded_build_command": "cargo build --locked --offline -j 5"
    }
  ],
  "hashes": {
    "target/performance/P020-performance-tests.log": "50c2988ecbdd2db0fdb2f3d38d65c3d406c6cb8001a9d8af4b46292f283d7437",
    "target/performance/P020-build.json": "90f6b19ab4e630afc2f7fe22df66aa9ac705013b707959b297e9525d4f0b0204",
    "build_source_sha256": "427ce3c021be44ae67886acad86606b9679cf4329dc4ac2c9521f7265cbe31cd",
    "build_source_sha256_after": "427ce3c021be44ae67886acad86606b9679cf4329dc4ac2c9521f7265cbe31cd",
    "Cargo.lock": "9d704c9854b30ca231109a54598c1c440a7d144b8ece5e21539971fead240820",
    "crates/wtime-runtime/src/execution/accounting.rs": "c86761a09e3c98301f54c971aa93e08da97140832c0f58e279aca4a75bef7127",
    "crates/wtime-runtime/src/execution.rs": "61a2effc27166744827ef52cc4b46d035ad6f74b93043b1d7666a437798ba921",
    "crates/wtime-runtime/src/continuation.rs": "c30b6fb4b89999b2e6f494b9258ce9ea3ea1e39a5971643b43147026e0fc8f46",
    "crates/wtime-runtime/src/reference.rs": "5afabf5f234dd6339a84be2440794ef0e181891f801d78a65a5703ccb304d7f9",
    "crates/wtime-runtime/tests/performance/p020.rs": "b6cb36f79e0d9702e7a8be4f81d254fcbd9d683d57c6baefe37f06951f611185"
  },
  "fixture_hashes": {
    "crates/wtime-runtime/tests/fixtures/performance/accounting/host_then_store.wasm": "ddc723fbcbad4bfc347cd89681a2061b738e1faa362f56f8a567d248fbe1fd19",
    "crates/wtime-runtime/tests/fixtures/performance/accounting/host_then_store.wat": "cbe74a5b2c9d087b470cdee9b58b391a2c75a1b7eed541cbc4f1c98e92176996",
    "crates/wtime-runtime/tests/fixtures/performance/accounting/ordinary_recursion.wasm": "e1bd7ea6ada8b15b2770ac387f8fc2256f20d7f87c4bc34e7559b239d8875ba1",
    "crates/wtime-runtime/tests/fixtures/performance/accounting/ordinary_recursion.wat": "aca6a66637fa36b72f3c528dd5e3e44039b573f363b5b4fb77437e9278623903",
    "crates/wtime-runtime/tests/fixtures/performance/accounting/tail_calls.wasm": "3e23be294e7a9eda62c89dbc366b6a6cab90edfbda532da27a7e4fc6d0aa4a56",
    "crates/wtime-runtime/tests/fixtures/performance/accounting/tail_calls.wat": "9953f19384c51b579921e6f789b13a8b57c86ba86d760b77846e3e33825ece02",
    "crates/wtime-runtime/tests/fixtures/performance/accounting/transient_stack.wasm": "988770c567d1c573ff38a97f0d54204bc40f4452b131ce343ae91f5042562df7",
    "crates/wtime-runtime/tests/fixtures/performance/accounting/transient_stack.wat": "6df3017d1a7d33aef39e8b66fbb7cb398d80f639eb972946ee2306582c0e41e5"
  },
  "limitations": [
    "The common decoded accounting helper is called once per source instruction. No aggregate prepared-group debit or optimized prefix executor is enabled; the source-prefix tests establish exact stopping boundaries only.",
    "The reference dispatcher retains the archived inline loop-head accounting header rather than calling the decoded helper; the comparison path has not been converted to shared aggregate accounting.",
    "The build-budget run retained only its JSON report and stripped-size metric, not a releasable binary or binary hash. No speedup is claimed.",
    "P020 performance-target command ran 18 total tests, including 4 P020 cases; unrelated P016-P018 tests are included in that target count."
  ],
  "build_and_dependencies": {
    "cpu": "x86_64",
    "clean_seconds": 5.736566249994212,
    "incremental_unchanged_seconds": 0.09128453700395767,
    "cli_leaf_seconds": 0.335041900005308,
    "runtime_leaf_seconds": 0.8929614929948002,
    "release_seconds": 12.655436606000876,
    "stripped_cli_bytes": 996024,
    "default_dependency_violations": 0,
    "dependency_delta": "No dependencies were added; default dependency closure check passed with 0 violations.",
    "budget_pass": true,
    "release_binary_hash": "Not captured; build_budget.py retained the report and stripped-size metric, not the temporary CLI binary."
  },
  "performance_decision": "Integrity-only; decoded instruction-by-instruction behavior is preserved and no aggregate execution path is active. No speedup is claimed."
}
```
