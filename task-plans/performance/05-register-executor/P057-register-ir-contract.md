# P057 — Define a small numeric register bytecode and eligibility rules

Status: **todo**

Outcome: **pending**

Kind: `foundation` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P018 — Define source cursors and materialized guest state](../01-execution-contracts/P018-source-cursor.md)

[P019 — Define bounded optimizer effect and source expansion metadata](../01-execution-contracts/P019-effect-certificates.md)

[P020 — Centralize exact source instruction accounting](../01-execution-contracts/P020-exact-execution-accounting.md)

[P021 — Compute bounded per-function execution metadata](../01-execution-contracts/P021-function-execution-plans.md)

[P056 — Select fusions and keep the portable hot path small](../04-specialization-and-fusion/P056-specialization-milestone.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/execution/register/ir.rs`
- `crates/wtime-runtime/src/execution/register/mod.rs`
- `crates/wtime-runtime/tests/performance/p057.rs`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

RegisterFunction is a nondefault bounded typed-slot program for numeric functions, with explicit source events, control successors and a decline reason for unsupported effects. It is not a general SSA compiler.

## Implementation recipe

1. Define simple Move/Const/Unary/Binary/Jump/Branch/Memory/Exit tags with u32 slot indices and bounded side tables, initially excluding refs/V128/calls/GC/exception/continuation execution.
2. Specify locals plus stack-height temporary slots as the initial deterministic layout; preserve logical operand heights separately from physical slot count.
3. Retain exact source map/materialization interfaces and immutable module ownership, with private validated construction and aggregate code/work limits.
4. Wire register-executor as nondefault and record unsupported-function entry fallback, avoiding a new internal crate until a measured boundary requires it.

## Meaningful cases

- Eligible numeric loops produce a valid bounded slot program.
- Reference/effectful/wide unsupported forms decline at entry with no guest work consumed.
- Malformed slot/source/event metadata is rejected and default builds exclude the executor.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime --features register-executor,execution-reference p057_ -- --nocapture
```

## Measurement and decision

Selection: `scalar`. Explicit execution setting: `register`. Decision rule: `integrity`.

This is a tooling/contract deliverable. Require correct nonzero checks and bounded/source-bound output; do not invent a runtime speedup requirement. If it changes production work, record cold/build/retention deltas under [MEASUREMENT.md](../MEASUREMENT.md).

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. No commands in this planning file have been executed as implementation evidence.
