# P022 — Add adversarial state and interaction tier fixtures

Status: **done**

Outcome: **contract_only**

Kind: `foundation` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P011 — Measure reference, GC and exception paths](../00-measurement/P011-gc-exception-workloads.md)

[P012 — Measure continuation capture, resume and accounting](../00-measurement/P012-continuation-workloads.md)

[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)

[P021 — Compute bounded per-function execution metadata](P021-function-execution-plans.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C5, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/tests/performance/p022.rs`
- `tools/wtime-bench/fixtures/integration/*.wat`
- `tools/wtime-bench/workloads.json`

- `crates/wtime-runtime/tests/performance.rs`

- `tools/wtime-bench/fixtures/integration/*.wasm`

- `docs/benchmarks/performance/P022-evidence.json`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

Integration selection exercises optimized-to-decoded state transitions and observable outcomes across calls, traps, roots, exceptions and continuations; corresponding Store handles are normalized rather than compared numerically.

## Implementation recipe

1. Build one mixed fixture for source-prefix side effects/fuel, one for host reentry/collection and one for exception/continuation context, reusing existing hard-task cases.
2. Compare globals, complete relevant memory ranges, table target identities and normalized object graphs in addition to returned values.
3. Inject low call/operand/GC/continuation budgets and stale/foreign dynamic handles; assert optimized paths cannot waive checks or root providers.
4. Register tier/fallback counters so future gates prove each eligible adapter actually executed; retain replay seeds and exact fixture identities.

## Meaningful cases

- Equal scalar output with a wrong memory effect fails comparison.
- A reference held only in an outer host-reentered frame survives collection.
- Cross-module suspension preserves exactness/signature contexts and budget/fuel failures occur at the same source point.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime p022_ -- --nocapture
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
  "task": "P022",
  "scope": "adversarial state and interaction fixtures",
  "selection": "integration",
  "execution_setting": "decoded default; reference feature comparison; portable is an alias for decoded",
  "production_timing": false,
  "verification": [
    {
      "command": "cargo test --locked -p wtime-runtime p022_ -- --nocapture",
      "features": [],
      "settings": "default decoded policy",
      "artifact": "target/performance/P022-default-tests-final.log",
      "test_binary": "target/debug/deps/performance-f1f918c868572e47",
      "test_binary_sha256": "491646f2f94286fae5ba503962b7cfef69078aa2db0162c0bf044ce71ebaf840",
      "log_sha256": "cd5894d70369efadb14cdc09a32ac81a50849f10ac8f60d8e9cf1a112c6955a6",
      "result": "8 passed, 0 failed, 0 ignored; 14 filtered out; 0.05 s",
      "retained_binary": "target/performance/P022/binary/performance-default"
    },
    {
      "command": "cargo test --locked -p wtime-runtime --features execution-reference p022_ -- --nocapture",
      "features": [
        "execution-reference"
      ],
      "settings": "decoded, portable alias, and reference policies",
      "artifact": "target/performance/P022-reference-tests-completion.log",
      "test_binary": "target/debug/deps/performance-b13572b815d07186",
      "test_binary_sha256": "6bb43de042ca8c1f52057cfa4d7d99189766fa5028233b3a100ab03e116ecece",
      "log_sha256": "e3da1f70dc49e2dd0f2fbaa2b5e2f747d7e9e15986da19a3714f188ca444af03",
      "result": "8 passed, 0 failed, 0 ignored; 23 filtered out; 0.05 s",
      "retained_binary": "target/performance/P022/binary/performance-reference"
    },
    {
      "command": "cargo test --locked -p wtime-runtime --features execution-reference,performance-counters p022_ -- --nocapture",
      "features": [
        "execution-reference",
        "performance-counters"
      ],
      "settings": "decoded, portable alias, and reference policies with source/reference dispatch counter assertions",
      "artifact": "target/performance/P022-counters-tests.log",
      "test_binary": "target/debug/deps/performance-990ca3e0de8de9bc",
      "test_binary_sha256": "f0892fa911a1118ee00f7da1375dc49d7e4944b512350f53b5b4baf86f50f9d5",
      "log_sha256": "e16535bb8b14c55d44e4f9b2d08abc0f1ccf710fc1da88f76fccd4286a8594de",
      "result": "8 passed, 0 failed, 0 ignored; 23 filtered out; 0.05 s",
      "retained_binary": "target/performance/P022/binary/performance-counters"
    }
  ],
  "named_cases": [
    "p022_prefix_effects_and_fuel_match_all_policies",
    "p022_host_reentry_collection_preserves_outer_root_and_state",
    "p022_cross_module_continuation_preserves_context_roots_and_one_shot",
    "p022_budget_priority_and_continuation_cleanup_match_policies",
    "p022_table_mutations_and_foreign_handles_are_checked",
    "p022_exception_unwind_and_stale_foreign_gc_values_are_checked",
    "p022_call_depth_and_tail_call_controls_match_policies",
    "p022_gc_budget_failure_preserves_prefix_and_heap"
  ],
  "behavior_coverage": {
    "fuel": "0..7 for every policy: exact fuel exhaustion below threshold 6, returned i32 7 at/above threshold, full 64 KiB memory checked for prefix effects and zero remainder",
    "call_depth_and_operand_stack": "tail recursion completes while ordinary recursion hits configured call-depth ResourceLimit; fuel=4 and fuel=5 distinguish fuel exhaustion from operand stack limit",
    "gc_byte_budget": "zero-byte guest heap budget fails before effects; verifies exact error, global phase 0, full memory remains zero, heap payload bytes zero; successful nested path still runs",
    "continuation_byte_budget": "zero-byte continuation budget fails with ResourceLimit and continuation byte accounting returns to zero after collection",
    "host_reentry_rooting": "host callback collects before/after nested call; sole outer root remains live; result 77, nested result 19, phase 2, full 64 KiB memory prefix and zero remainder checked; later collection reaches zero live objects",
    "cross_module_suspension": "provider and consumer Modules dropped before continuation resume; payload empty; published continuation root transferred; collection retains it; resume returns 77 exactly once, second resume traps; after root drop collection returns GC payload bytes to zero",
    "tables_and_handles": "table slot mutations switch calls between functions returning 17 and 29; slot identities/null are inspected; foreign-store function rejected",
    "exceptions_and_gc_values": "cross-module exception context survives Module drop and returns 42; foreign and stale GC values are rejected; post-release collection reaches zero live values"
  },
  "path_evidence": "with performance-counters, non-reference policy runs assert source_dispatches > 0; with execution-reference, reference policy runs assert reference_dispatches > 0",
  "limitations": [
    "No compact/register/native optimized adapter or optimized-to-decoded transition exists in this integration test; no optimized-tier performance or parity claim is made.",
    "ExecutionPolicy::Portable currently resolves to decoded, as recorded in acceptance_checks.",
    "The P022 task file and TASKS.json still report todo/pending; this file is an evidence draft, not a status update.",
    "Registry identities describe the completion-time acceptance-check registration; Rust tests consume the pinned Wasm fixtures directly and do not load the JSON registry. Subsequent benchmark tooling extensions retain earlier test evidence as historical."
  ],
  "source_hashes": {
    "crates/wtime-runtime/tests/performance/p022.rs": "e688034612986f5ccab2640e56c2ef0cadf4ad661e8b362a85455b29bffacad6",
    "crates/wtime-runtime/tests/performance.rs": "e1db973360cd5b90a7fe36db55da7e8375baa4a25db8d23df3ae596c31dd8a24",
    "tools/wtime-bench/workloads.json": "ced1971bc2c1bd67677237cad5fd0ce9c6a0ee1a09d5349d58c6a91615e37990"
  },
  "fixture_hashes": {
    "tools/wtime-bench/fixtures/integration/host_reentry.wat": "a639719b905239188e6d6baeb5cbe8a7ff5f18e2350d06ab9b7aa9daefb23cd9",
    "tools/wtime-bench/fixtures/integration/host_reentry.wasm": "6c3c8ac8ff596cdc4c8320a542fe9a36ac82a000326a12f38638ec82328a18d3",
    "tools/wtime-bench/fixtures/integration/table_targets.wat": "32d5bd9d4f8279245b8b20b938ddfb6a6218dc04137d91550e0581882d33a9aa",
    "tools/wtime-bench/fixtures/integration/table_targets.wasm": "725c3d612c152cd88909eaae911dc88b558af6e138108d2e8977f9251aad44d0",
    "tools/wtime-bench/fixtures/integration/cross_module_provider.wat": "4e69dd91d7bc14d7e8ba630138ab2ae4c58accdf9de0f284bcfdfcda13b6e651",
    "tools/wtime-bench/fixtures/integration/cross_module_provider.wasm": "d6d7298a08c1888fe15c44b430187f10b5b8734ca04dc25b627b2149860c2290",
    "tools/wtime-bench/fixtures/integration/cross_module_consumer.wat": "6c9a7c8e869e9f3c1d931b63251032bae765c20c78d06f11aa5ff9e1fb24256d",
    "tools/wtime-bench/fixtures/integration/cross_module_consumer.wasm": "85d6081efdbf66f3b825d6847fd0831e25c0be110e51feccca99b56595adb2ab",
    "tools/wtime-bench/fixtures/integration/expected_state.json": "126d72095766530e35a90ad864c8f90399c88c4bcac3bc039e02324bdd407ba9"
  },
  "status": "done",
  "outcome": "contract_only",
  "commands": [
    "cargo test --locked -p wtime-runtime p022_ -- --nocapture",
    "cargo test --locked -p wtime-runtime --features execution-reference p022_ -- --nocapture",
    "cargo test --locked -p wtime-runtime --features execution-reference,performance-counters p022_ -- --nocapture"
  ],
  "artifacts": [
    "target/performance/P022-default-tests-final.log",
    "target/performance/P022/binary/performance-default",
    "target/performance/P022-reference-tests-completion.log",
    "target/performance/P022/binary/performance-reference",
    "target/performance/P022-counters-tests.log",
    "target/performance/P022/binary/performance-counters",
    "crates/wtime-runtime/tests/performance/p022.rs",
    "crates/wtime-runtime/tests/performance.rs",
    "tools/wtime-bench/workloads.json",
    "tools/wtime-bench/fixtures/integration/host_reentry.wat",
    "tools/wtime-bench/fixtures/integration/host_reentry.wasm",
    "tools/wtime-bench/fixtures/integration/table_targets.wat",
    "tools/wtime-bench/fixtures/integration/table_targets.wasm",
    "tools/wtime-bench/fixtures/integration/cross_module_provider.wat",
    "tools/wtime-bench/fixtures/integration/cross_module_provider.wasm",
    "tools/wtime-bench/fixtures/integration/cross_module_consumer.wat",
    "tools/wtime-bench/fixtures/integration/cross_module_consumer.wasm",
    "tools/wtime-bench/fixtures/integration/expected_state.json"
  ],
  "artifact_hashes": {
    "target/performance/P022-default-tests-final.log": "cd5894d70369efadb14cdc09a32ac81a50849f10ac8f60d8e9cf1a112c6955a6",
    "target/performance/P022/binary/performance-default": "491646f2f94286fae5ba503962b7cfef69078aa2db0162c0bf044ce71ebaf840",
    "target/performance/P022-reference-tests-completion.log": "e3da1f70dc49e2dd0f2fbaa2b5e2f747d7e9e15986da19a3714f188ca444af03",
    "target/performance/P022/binary/performance-reference": "6bb43de042ca8c1f52057cfa4d7d99189766fa5028233b3a100ab03e116ecece",
    "target/performance/P022-counters-tests.log": "e16535bb8b14c55d44e4f9b2d08abc0f1ccf710fc1da88f76fccd4286a8594de",
    "target/performance/P022/binary/performance-counters": "f0892fa911a1118ee00f7da1375dc49d7e4944b512350f53b5b4baf86f50f9d5",
    "crates/wtime-runtime/tests/performance/p022.rs": "e688034612986f5ccab2640e56c2ef0cadf4ad661e8b362a85455b29bffacad6",
    "crates/wtime-runtime/tests/performance.rs": "e1db973360cd5b90a7fe36db55da7e8375baa4a25db8d23df3ae596c31dd8a24",
    "tools/wtime-bench/workloads.json": "ced1971bc2c1bd67677237cad5fd0ce9c6a0ee1a09d5349d58c6a91615e37990",
    "tools/wtime-bench/fixtures/integration/host_reentry.wat": "a639719b905239188e6d6baeb5cbe8a7ff5f18e2350d06ab9b7aa9daefb23cd9",
    "tools/wtime-bench/fixtures/integration/host_reentry.wasm": "6c3c8ac8ff596cdc4c8320a542fe9a36ac82a000326a12f38638ec82328a18d3",
    "tools/wtime-bench/fixtures/integration/table_targets.wat": "32d5bd9d4f8279245b8b20b938ddfb6a6218dc04137d91550e0581882d33a9aa",
    "tools/wtime-bench/fixtures/integration/table_targets.wasm": "725c3d612c152cd88909eaae911dc88b558af6e138108d2e8977f9251aad44d0",
    "tools/wtime-bench/fixtures/integration/cross_module_provider.wat": "4e69dd91d7bc14d7e8ba630138ab2ae4c58accdf9de0f284bcfdfcda13b6e651",
    "tools/wtime-bench/fixtures/integration/cross_module_provider.wasm": "d6d7298a08c1888fe15c44b430187f10b5b8734ca04dc25b627b2149860c2290",
    "tools/wtime-bench/fixtures/integration/cross_module_consumer.wat": "6c9a7c8e869e9f3c1d931b63251032bae765c20c78d06f11aa5ff9e1fb24256d",
    "tools/wtime-bench/fixtures/integration/cross_module_consumer.wasm": "85d6081efdbf66f3b825d6847fd0831e25c0be110e51feccca99b56595adb2ab",
    "tools/wtime-bench/fixtures/integration/expected_state.json": "126d72095766530e35a90ad864c8f90399c88c4bcac3bc039e02324bdd407ba9"
  }
}
```
