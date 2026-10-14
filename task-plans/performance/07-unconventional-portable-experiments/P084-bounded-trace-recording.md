# P084 — Record short scalar loop traces at verified backedges

Status: **todo**

Outcome: **pending**

Kind: `experiment` · Branch: `optional` · Model: **Sol 6.1 Low**

## Prerequisites

[P002 — Compare separately built baseline and candidate binaries](../00-measurement/P002-paired-driver.md)

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P008 — Add scaling, branch and call benchmark families](../00-measurement/P008-scalar-workloads.md)

[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)

[P016 — Add explicit executor policies and test integration hooks](../01-execution-contracts/P016-execution-policy.md)

[P018 — Define source cursors and materialized guest state](../01-execution-contracts/P018-source-cursor.md)

[P019 — Define bounded optimizer effect and source expansion metadata](../01-execution-contracts/P019-effect-certificates.md)

[P020 — Centralize exact source instruction accounting](../01-execution-contracts/P020-exact-execution-accounting.md)

[P057 — Define a small numeric register bytecode and eligibility rules](../05-register-executor/P057-register-ir-contract.md)

[P060 — Implement safe parallel moves and register control transfers](../05-register-executor/P060-register-edge-moves.md)

[P061 — Execute checked integer register operations](../05-register-executor/P061-register-integer-execution.md)

[P067 — Evaluate register execution and adaptive break-even](../05-register-executor/P067-register-milestone.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/execution/experimental/trace_record.rs`
- `crates/wtime-runtime/tests/performance/p084.rs`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

A bounded recorder captures numeric local/control paths from a verified loop backedge, with original PCs, predicates, slot layout and side-exit points; no memory/ref/host effects are initially traced.

## Implementation recipe

1. Use the register/compact source map to identify an eligible numeric loop entry with one logical frame and a stable materializable stack layout.
2. Record at most 256 source operations/32 guards per trace and a bounded count per function; stop at barriers, repeated unsupported cycles or work/code limits.
3. Store actual branch predicates and invariant assumptions as guard descriptors, never specialize solely on benchmark inputs/names or unguarded constants.
4. Replay recorded paths in diagnostic verification against source execution before any optimized trace executor is selected.
5. Forward the explicit `trace-record` setting through the harness/policy integrator and record actual optimized/fallback counts. Persistent research selectors are nondefault; reversible changes to an existing tier use before/after artifacts and are selected only by their gate.

## Meaningful cases

- Branch-dependent paths record explicit guards and side exits.
- Barriers/oversized/irreducible paths decline without unbounded recording.
- Entry and every recorded prefix reconstruct exact numeric/source state.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime --features experimental-executors,execution-reference p084_ -- --nocapture
```

## Measurement and decision

Selection: `scalar`. Explicit execution setting: `trace-record`. Decision rule: `integrity`.

This is a tooling/contract deliverable. Require correct nonzero checks and bounded/source-bound output; do not invent a runtime speedup requirement. If it changes production work, record cold/build/retention deltas under [MEASUREMENT.md](../MEASUREMENT.md).

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. No commands in this planning file have been executed as implementation evidence.
