# P091 — Export a verified numeric program for an offline Rust AOT tool

Status: **deferred**

Outcome: **not_selected**

Kind: `experiment` · Branch: `optional` · Model: **Sol 6.1 Low**

## Prerequisites

[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)

[P018 — Define source cursors and materialized guest state](../01-execution-contracts/P018-source-cursor.md)

[P019 — Define bounded optimizer effect and source expansion metadata](../01-execution-contracts/P019-effect-certificates.md)

[P057 — Define a small numeric register bytecode and eligibility rules](../05-register-executor/P057-register-ir-contract.md)

[P058 — Translate numeric stack code into direct slot operands](../05-register-executor/P058-stack-to-register-lowering.md)

[P060 — Implement safe parallel moves and register control transfers](../05-register-executor/P060-register-edge-moves.md)

[P061 — Execute checked integer register operations](../05-register-executor/P061-register-integer-execution.md)

[P067 — Evaluate register execution and adaptive break-even](../05-register-executor/P067-register-milestone.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/execution/aot_view.rs`
- `tools/wtime-aot/Cargo.toml`
- `tools/wtime-aot/src/main.rs`
- `tools/wtime-aot/README.md`
- `crates/wtime-runtime/tests/performance/p091.rs`
- `crates/wtime-runtime/Cargo.toml`
- `tools/wtime-bench/Cargo.toml`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

An isolated optional tool consumes a read-only validated numeric program view and emits a versioned manifest bound to original Wasm/features/limits/source event maps; it cannot deserialize unchecked executable state into the runtime.

## Implementation recipe

1. Define a narrow read-only numeric program export and trusted compiled-kernel registration contract that validates original source/module identity at load; no public raw-handle/unchecked-state constructors.
2. Limit initial AOT subset to pure numeric functions/loops with no memory/host/ref/capture effects and specify the normal source-accounting/mutable state ABI.
3. Create tools/wtime-aot as its own std-only/local-dependency workspace and CLI generate INPUT --output DIR; keep output and compiler work out of default runtime builds.
4. Document that generated code is trusted compiler output, not an untrusted plugin sandbox; validate names/indices and never insert guest names/text as Rust tokens.
5. Record go/no_go for this isolated AOT interface; a no_go defers P092/P093 with not_selected outcomes and no generated-code support claim.
6. Add nondefault aot-export and corresponding benchmark forwarding/registration hooks; generated kernels and source views compile out of default builds. The frozen-core adapter excludes these new APIs while still executing the same original Wasm bytes.

## Meaningful cases

- Wrong source/profile/spec/event manifest is rejected at registration.
- Malicious guest names cannot inject Rust source.
- Effectful/unsupported functions decline and default workspace dependency/build closure is unchanged.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime --features aot-export,execution-reference p091_ -- --nocapture
```

## Measurement and decision

Selection: `scalar`. Explicit execution setting: `aot-design`. Decision rule: `design_go_no_go`.

Complete the design/integrity checks and a concrete cost/scope decision artifact. Record go or no_go; this task establishes a conditional interface, not a measured speedup. Descendants remain unimplemented if no_go.

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. Executed evidence is recorded below.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/final-optional-dispositions-0002/proof.json": "5a2943b1b7f0e4fedf44a2d4e96153937683fc97d45e079e55c0ef43a5e3b97f"
  },
  "evidence": [
    {
      "bytes": 38990,
      "path": "/workspace/wtime/target/performance/final-optional-dispositions-0002/proof.json",
      "sha256": "5a2943b1b7f0e4fedf44a2d4e96153937683fc97d45e079e55c0ef43a5e3b97f"
    }
  ],
  "implementation_claimed": false,
  "outcome": "not_selected",
  "reason": "Conditional research is not selected for the current portfolio; this record claims no implementation, test execution, or speed result.",
  "schema": "wtime-task-disposition-record-v1",
  "status": "deferred",
  "task": "P091",
  "tests_claimed": 0,
  "timing_evidence": false
}
```
