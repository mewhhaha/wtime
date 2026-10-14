# P019 — Define bounded optimizer effect and source expansion metadata

Status: **done**

Outcome: **complete**

Kind: `foundation` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P018 — Define source cursors and materialized guest state](P018-source-cursor.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C5, C6, C7** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/execution/effects.rs`
- `crates/wtime-runtime/src/execution/cursor.rs`
- `crates/wtime-runtime/src/execution.rs`
- `crates/wtime-runtime/tests/performance/p019.rs`

The tests inspect private `PreparedEffect` metadata. `execution/effects.rs` includes `tests/performance/p019.rs` under `cfg(test)`, so the focused command runs these cases in the library unit-test target rather than the shared performance integration target. This private-test arrangement preserves the non-public certificate constructor and verifier boundary.

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

A private PreparedEffect record includes original operation expansion, ordered side effects, possible traps, root/materialization points, transient stack peak and logical-call events; only validated lowering can construct it.

## Implementation recipe

1. Classify every original opcode into a conservative effect family using the existing ledger; unknown classifications are barriers, never assumed pure.
2. Define source expansion as ordered source-PC/event entries so fusion/inlining/folding cannot erase fuel, cancellation or limit events.
3. Set aggregate metadata/work limits with checked arithmetic, fallible reservation and transactional publication; do not create a second unbounded semantic analyzer.
4. Provide a verifier that rejects missing origins, reordered effects, invalid branch targets and root-relevant operations without a materialization point.

## Meaningful cases

- An unclassified opcode blocks optimization safely.
- A reordered store/trap or missing source event is rejected.
- Metadata overflow/budget exhaustion leaves a usable decoded module and publishes no partial optimized artifact.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime p019_ -- --nocapture
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
    "/workspace/wtime/target/performance/P019-tests.log"
  ],
  "commands": [
    {
      "command": "cargo test --locked -p wtime-runtime p019_ -- --nocapture",
      "exit_code": 0,
      "passed": 5,
      "failed": 0,
      "skipped": 0,
      "test_target": "library unit tests; private PreparedEffect tests are included from tests/performance/p019.rs by execution/effects.rs"
    }
  ],
  "hashes": {
    "target/performance/P019-tests.log": "451c1a0694385faa3e69537c8628bfddc50fd61bbcd00a514efe3913867fc3b7",
    "crates/wtime-runtime/src/execution/effects.rs": "0d3a56c4a8714b80d9542ecbf1e14ae17ebffb43052d67052e17ae046ba28478",
    "crates/wtime-runtime/src/execution/cursor.rs": "ff93a3f0056d109366d2b77ea93d459f456d048c05b4bf05369305a3b491d3d3",
    "crates/wtime-runtime/src/execution.rs": "373caa16454247f85911714b2f9bc340929dda950b3adcd81f5570af1a12ccea",
    "crates/wtime-runtime/tests/performance/p019.rs": "8294310dd4da5f9a7a05bf0bfee0d2be503ec52cdddbca4e21c6a3ee9d3b2715"
  },
  "fixture_hashes": {
    "crates/wtime-runtime/tests/fixtures/performance/cursor/store_successor.wat": "366ad9a071ac01995edfcb5e7291c5bc3c3de4a33c7c52b63edfce3403d1fcbd",
    "crates/wtime-runtime/tests/fixtures/performance/cursor/store_successor.wasm": "e05575761936f326eebf004bcfc6b015cb6dbf8ab14412aa7daae268cff3ce96"
  },
  "limitations": [
    "PreparedEffect remains private conservative metadata. Unclassified opcodes are barriers; transient_stack_peak is always None until a trusted function-plan producer exists, so numeric_region_eligible always declines. No optimized executor or speedup is claimed.",
    "P019 tests inspect private PreparedEffect and are compiled into the library unit-test target through an include from execution/effects.rs; they are not tests in the shared performance integration target.",
    "P019 test output does not bind a source snapshot or retain a test-binary hash. Source identities above are captured at evidence finalization, not claimed as a separately archived pre-test source bundle."
  ],
  "build_and_dependencies": {
    "build_budget_artifact": null,
    "dependency_delta": "No dependency or Cargo feature changes were part of P019; no dependency delta was measured.",
    "binary_hash": "Not captured; the test log records the executed library unit-test target but does not bind its executable hash."
  },
  "performance_decision": "Foundation integrity check only; no optimized execution path is active and no speedup is claimed."
}
```
