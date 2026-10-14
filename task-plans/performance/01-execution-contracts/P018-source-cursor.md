# P018 — Define source cursors and materialized guest state

Status: **done**

Outcome: **complete**

Kind: `foundation` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P016 — Add explicit executor policies and test integration hooks](P016-execution-policy.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C6, C7** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/execution/cursor.rs`
- `crates/wtime-runtime/src/execution.rs`
- `crates/wtime-runtime/src/continuation.rs`
- `crates/wtime-runtime/tests/performance/p018.rs`
- `crates/wtime-runtime/src/lib.rs`

- `crates/wtime-runtime/tests/performance.rs`
- `crates/wtime-runtime/tests/fixtures/performance/cursor/*.wat`
- `crates/wtime-runtime/tests/fixtures/performance/cursor/*.wasm`
- `tools/wtime-bench/src/protocol.rs`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

ExecutionCursor names original function/PC plus prepared PC; MaterializedState contains logical frames, locals, operands, labels/catches, prompts, canonical contexts and remaining fuel with the next original operation unexecuted.

## Implementation recipe

1. Describe the current frame.pc increment and branch/exception/continuation conventions precisely; add a lossless cursor conversion for decoded execution first.
2. Implement snapshot/restore helpers at development-tier boundaries without making a public unchecked state constructor; keep references owned/rooted and record logical heights/depths.
3. Add equivalence assertions for execution resuming from a materialized point; preserve earlier side effects and never replay a completed store/call/allocation.
4. Make source-PC attribution available to the diagnostic runner and reserve prepared-PC mapping interfaces for compact/register tasks.
5. Expose a bounded read-only diagnostic snapshot under execution-reference for the tier runner: source cursor, normalized frame/operand/root shape and observable state, with no unchecked executable-state/handle constructors. Snapshot hooks compile out of production measurements; the old frozen-core adapter uses public outputs/digests instead.

## Meaningful cases

- Restoring after a store resumes at its successor without duplicating the store.
- A multivalue branch/exception state restores operand order and labels.
- A suspended cross-module frame retains nominal context and saved remaining fuel.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime --features execution-reference p018_ -- --nocapture
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
    "/workspace/wtime/target/performance/P018-snapshot.json",
    "/workspace/wtime/target/performance/P018-runtime-tests.log"
  ],
  "commands": [
    {
      "command": "cargo test --locked -p wtime-runtime --features execution-reference --test performance p018_ -- --nocapture",
      "exit_code": 0,
      "passed": 7,
      "failed": 0,
      "skipped": 0
    },
    {
      "command": "cargo test --locked -p wtime-runtime --features execution-reference --test performance",
      "exit_code": 0,
      "passed": 14,
      "failed": 0,
      "skipped": 0
    },
    {
      "command": "cargo build --locked --release --manifest-path tools/wtime-bench/profiling/Cargo.toml --features execution-reference",
      "exit_code": 0
    },
    {
      "command": "cargo build --locked -p wtime-cli",
      "exit_code": 0
    },
    {
      "command": "wtime-bench --snapshot store_successor.wasm - i32:7 3 decoded all-phase3-core store_successor",
      "exit_code": 0,
      "passed": 1,
      "source_pc": 3,
      "roundtrips": 1
    }
  ],
  "hashes": {
    "target/performance/P018-snapshot.json": "ca160c1bcced6cc33e9f9caa43482d3bd26047d3c832cbd178bb1fb39e2d1423",
    "target/performance/P018-runtime-tests.log": "b07d77e712bf99a152130e85b9ca5453adbe9aee3623b105f0892abb9cba2266"
  },
  "fixture_hashes": {
    "crates/wtime-runtime/tests/fixtures/performance/cursor/root_after_restore.wasm": "9d681c27087d43d3a3c43780fa981d492d1fd7000f61ef551a6bdc1fe6ccc520",
    "crates/wtime-runtime/tests/fixtures/performance/cursor/multivalue_branch.wasm": "251acf5ea9d683baa9671ab18b7e1a053e2bb1ebaedff8c77f215dc5fd32502d",
    "crates/wtime-runtime/tests/fixtures/performance/cursor/host_prefix.wasm": "672366f323679c7a45df7dfa0e7e1269ad180b3b87a565f812bb891f07474eeb",
    "crates/wtime-runtime/tests/fixtures/performance/cursor/store_successor.wasm": "e05575761936f326eebf004bcfc6b015cb6dbf8ab14412aa7daae268cff3ce96",
    "crates/wtime-runtime/tests/fixtures/performance/cursor/cross_callee.wasm": "8a9be008deb3df26866dd34688447acac9b3a3f46e47c2e4f21fe109148791b6",
    "crates/wtime-runtime/tests/fixtures/performance/cursor/cross_caller.wasm": "fc047eb589e9668f918a8e44db0c7ddaa3aad80ee748c68e584707e5632c6f6f"
  },
  "limitations": [
    "State is privately moved/restored at the charged-before-fetch boundary within the active loop; no public arbitrary executable-state constructor or public invoke restart.",
    "Read-only snapshots cap values/frames/memory/table prefixes and explicitly mark truncation; they describe normalized state shape, not a full heap/memory digest. Future P014/P022 add broader tier differential orchestration.",
    "Snapshot hooks compile out without execution-reference; that feature now reports instrumented=true so production comparison rejects it.",
    "Snapshot runner prints source/prepared PCs and a normalized debug description; the read-only Rust view exposes structured fields. No performance gain is claimed for this development-only contract."
  ]
}
```
