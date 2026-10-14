# P017 — Make every Module clone share immutable metadata

Status: **done**

Outcome: **adopted**

Kind: `candidate` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P002 — Compare separately built baseline and candidate binaries](../00-measurement/P002-paired-driver.md)

[P003 — Define workload selections and engine capability preflight](../00-measurement/P003-workload-schema.md)

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P007 — Capture CPU profiles and rank actionable costs](../00-measurement/P007-profile-hotpaths.md)

[P016 — Add explicit executor policies and test integration hooks](P016-execution-policy.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/lib.rs`
- `crates/wtime-runtime/src/execution.rs`
- `crates/wtime-runtime/src/continuation.rs`
- `crates/wtime-runtime/tests/performance/p017.rs`

- `crates/wtime-runtime/src/linker.rs`
- `crates/wtime-runtime/src/gc_exec.rs`
- `crates/wtime-runtime/src/exceptions.rs`
- `crates/wtime-runtime/src/reference.rs`
- `crates/wtime-runtime/src/perf.rs`
- `crates/wtime-runtime/tests/performance.rs`
- `tools/wtime-bench/compare.py`
- `tools/wtime-bench/tests/test_p017_compare.py`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

Module is a cheap cloneable handle to Arc<ModuleData>; engine identity, validation provenance, decoded code, canonical IDs and control tables share immutable ownership. No control/canonical Vec is copied by Module::clone.

## Implementation recipe

1. Move control/canonical/validated storage into one private immutable payload and change runtime access sites through private accessors or the payload field; do not leak a private type through public Deref.
2. Keep compile-time interning/lowering and Engine/Store checks unchanged; host-type context, instances, frames and continuation contexts retain the shared handle.
3. Review source_bytes/lowered_control_bytes and continuation retained-byte accounting for shared versus uniquely owned buffers; document physical retained ownership rather than charging duplicate table copies.
4. Use diagnostic counters and the archived binary to compare all original compute/preparation cases; retain the patch as contract_only only if immutable sharing is needed but speed is inconclusive.
5. Record the changed path with focused assertions/diagnostic counts and exact source/binary identities; compare the pre-edit archive, and revert a rejected structural optimization rather than adding a permanent toggle solely for testing.

## Meaningful cases

- Cloning a large module shares the exact payload/control/type storage rather than copying it.
- A retained host type context and suspended frame remain valid after the caller drops its Module handle.
- Foreign Engine instantiation is still rejected and all original workload outputs match.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime p017_ -- --nocapture
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml -j 4
python3 tools/wtime-bench/compare.py --baseline target/performance/P017-before --candidate tools/wtime-bench/target/release/wtime-bench --candidate-mode decoded --selection original-mvp --output target/performance/P017
python3 tools/wtime-bench/analyze.py --input target/performance/P017 --output target/performance/P017/analysis.json
python3 tools/checks/build_budget.py --output target/performance/P017/build.json
python3 tools/wtime-bench/gates.py --task P017 --comparison target/performance/P017 --build target/performance/P017/build.json --output target/performance/P017/decision.json
```

## Measurement and decision

Selection: `original-mvp`. Explicit execution setting: `decoded`. Decision rule: `measured_promotion`.

Use the paired protocol and family/control/cold/build rules in [MEASUREMENT.md](../MEASUREMENT.md). Predeclare the targeted family and held-out controls before changing the implementation. Keep instrumentation off in production timings. Adopt only with supported gains in both runs; otherwise record rejected/inconclusive and remove default-path cost. A necessary immutable-interface change may be contract_only with its separate justification.

## Before editing

Run these commands before changing the candidate sources. The archive is immutable and captures the immediate predecessor. Retain P001's original archive for milestone totals. For native work, use the named portable control and verify its actual policy/quantum; if this archive already exists with different sources, choose a new recorded run-specific directory and update the comparison path rather than overwriting it.

```sh
python3 tools/wtime-bench/baseline.py --output target/performance/P017-before
```

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. Executed evidence is recorded below.

## Executed evidence

```json
{
  "artifacts": [
    "/workspace/wtime/target/performance/P017/report.json",
    "/workspace/wtime/target/performance/P017/analysis.json",
    "/workspace/wtime/target/performance/P017/build.json",
    "/workspace/wtime/target/performance/P017/decision.json",
    "/workspace/wtime/target/performance/P017/runtime-tests.log",
    "/workspace/wtime/target/performance/P017-before/manifest.json"
  ],
  "commands": [
    {
      "command": "python3 tools/wtime-bench/baseline.py --output target/performance/P017-before",
      "exit_code": 0,
      "before_edit": true
    },
    {
      "command": "cargo test --locked -p wtime-runtime --features execution-reference,performance-counters p017_ -- --nocapture",
      "exit_code": 0,
      "passed": 5,
      "failed": 0,
      "skipped": 0
    },
    {
      "command": "cargo test --locked -p wtime-runtime --features performance-counters p006_ -- --nocapture",
      "exit_code": 0,
      "passed": 3,
      "failed": 0,
      "skipped": 0
    },
    {
      "command": "python3 -m unittest discover -s tools/wtime-bench/tests -p 'test_p017*.py' -v",
      "exit_code": 0,
      "passed": 4,
      "failed": 0,
      "skipped": 0
    },
    {
      "command": "python3 tools/wtime-bench/compare.py --baseline target/performance/P017-before --candidate tools/wtime-bench/target/release/wtime-bench --candidate-mode decoded --selection original-mvp --output target/performance/P017",
      "exit_code": 0,
      "runs": 2,
      "pairs": 1260
    },
    {
      "command": "python3 tools/wtime-bench/analyze.py --input target/performance/P017 --output target/performance/P017/analysis.json",
      "exit_code": 0
    },
    {
      "command": "python3 tools/checks/build_budget.py --output target/performance/P017/build.json",
      "exit_code": 0
    },
    {
      "command": "python3 tools/wtime-bench/gates.py --task P017 --comparison target/performance/P017 --build target/performance/P017/build.json --output target/performance/P017/decision.json",
      "exit_code": 0,
      "decision": "adopted"
    },
    {
      "command": "cargo test --locked -p wtime-runtime --features execution-reference,performance-counters",
      "exit_code": 0,
      "passed": 146,
      "failed": 0,
      "skipped": 0
    },
    {
      "command": "python3 tools/checks/dependency_budget.py",
      "exit_code": 0,
      "production_violations": 0
    }
  ],
  "hashes": {
    "target/performance/P017/report.json": "9ea5b6f0e17cd1d5d68a5170bf4cd2f4a84104d8dd186cdd1f90cf385b30baa3",
    "target/performance/P017/analysis.json": "764e9d47a9c50ca0bc0880d6cb8a79e644ace87fe106f5ca1d90a62154228da4",
    "target/performance/P017/build.json": "7b794ed309060ef5f40a64c47e69aea09c6b1219ab5c656e1a9e1d0c0e41eef7",
    "target/performance/P017/decision.json": "a00a8a40553c447e6397038c8c057637556302ffd4708a04d84209de35bad966",
    "target/performance/P017/runtime-tests.log": "4a9be8f3e15818d0e7a564fd13e220a6378f5ec9e381e6975b00771b05523b01",
    "target/performance/P017-before/manifest.json": "d51b49e5f5967e86de0d633181b55c63e3f3661b56b533444fe8914d3a156a2e"
  },
  "limitations": [
    "Payload bytes are physical shared ownership; per-clone/per-continuation duplicate charges are deliberately absent.",
    "Two private ownership unit tests complement the single registered performance integration target, without exposing private payload constructors.",
    "No Wasmtime-relative speed claim or later-tier completion is inferred from this structural gate."
  ]
}
```
