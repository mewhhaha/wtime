# P097 — Emit a tiny verified x86_64 integer block subset

Status: **deferred**

Outcome: **not_selected**

Kind: `foundation` · Branch: `optional` · Model: **Sol 6.1 Low**

## Prerequisites

[P061 — Execute checked integer register operations](../05-register-executor/P061-register-integer-execution.md)

[P095 — Create the minimal optional numeric backend interface](P095-native-ir-and-abi.md)

[P096 — Implement one audited W^X mapping owner for x86 Linux](P096-executable-mapping-owner.md)

Conditional branch: `P094.outcome == go`. If its decision is declined, record this task as deferred/not_selected; do not claim it implemented.

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-native/src/x86/encode.rs`
- `crates/wtime-native/src/x86/blocks.rs`
- `crates/wtime-native/tests/x86_blocks.rs`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

The first emitter handles const/move/i32 add/sub/mul/xor/compare over verified slots with fixed ABI scratch registers; generated instructions/relocations are private bounded output, not an arbitrary bytecode loader.

## Implementation recipe

1. Implement a small checked encoder for the named forms using integer bytes/relocation records, not a general assembler dependency or unrestricted instruction selection.
2. Load/store only statically verified slot offsets and use the ABI's fixed scratch register set; preserve callee-saved registers and stack alignment without a native guest call stack.
3. Validate template boundaries/slot offsets/branch-free extent before sealing, and decline all unsupported instructions with no guest work consumed.
4. Differential-test straight blocks including wrap/boundary inputs, retain byte/decoded-instruction evidence where tools exist, and measure emission/code bytes separately.

## Meaningful cases

- Random/boundary integer blocks equal the safe register executor.
- Invalid/overflowing slot offsets or truncated templates are rejected before RX publication.
- Callee-saved registers/ABI and unsupported op fallback are correct on native x86.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-native p097_ -- --nocapture
```

## Measurement and decision

Selection: `scalar`. Explicit execution setting: `native`. Decision rule: `integrity`.

This is a tooling/contract deliverable. Require correct nonzero checks and bounded/source-bound output; do not invent a runtime speedup requirement. If it changes production work, record cold/build/retention deltas under [MEASUREMENT.md](../MEASUREMENT.md).

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
  "task": "P097",
  "tests_claimed": 0,
  "timing_evidence": false
}
```
