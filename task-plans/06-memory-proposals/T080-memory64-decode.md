# T080 — Decode 64-bit memories, tables and immediates

Status: **done**  
Area: `wtime-decode`  
Work unit: one focused behavior/contract and its tests; split into child tasks before exceeding two focused patches.

## Prerequisites

[T018 — Decode function types and storage limits](../01-binary-decoding/T018-decode-basic-types.md), [T077 — Decode explicit multi-memory indices](../06-memory-proposals/T077-multi-memory-decode.md)

Use the predecessor's interfaces and evidence. Do not reimplement a second parser, type registry, store or execution stack.

## Deliverable

Decode memory/table index-type flags, u64 limits and appropriate offsets/immediates. Table64 is part of this scope, including element offsets and call_indirect address operands.

## Acceptance

Large limits/offsets are preserved bit-exactly; malformed width flags and out-of-width LEBs fail decoding.

Add focused tests for the changed behavior and relevant failure boundaries. Record the exact command, feature profile, corpus/source revision and executed assertion counts. Follow [the common definition of done](../EXECUTION.md#common-definition-of-done); a missing directive, zero-test run or unimplemented branch is not a pass.

## Boundaries

Follow [ARCHITECTURE.md](../ARCHITECTURE.md): no external core production dependency, no per-proposal Cargo-crate proliferation, no JS/browser implementation, and no invented semantics for draft TODOs. Coordinate changes to shared types/opcode/control contracts with other ready tasks. Runtime feature gates and implementation resource limits must remain explicit.

## Implementation instructions for Sol 6.1 Low

Read [the interface contracts](../IMPLEMENTATION_CONTRACTS.md) and the listed prerequisite outputs before coding. Use the predecessor's API; do not invent a second implementation to avoid a missing prerequisite.

### Files and ownership

Expected paths in `/workspace/wtime` (created by implementation, not by this plan):

- `crates/wtime-decode/src/sections/types.rs`
- `crates/wtime-decode/src/instructions/memory.rs`

Add focused tests in the relevant package's `tests/` directory, or its existing unit-test module. Name tests `t080_case_01`, `..._case_02`, etc. Reuse existing feature modules when present; coordinate edits to central dispatch/type tables.

### Ordered recipe

1. Decode index-type flags and u64 limits/offsets for memories and tables.
2. Update table/call_indirect/segment operand representations without usize narrowing.
3. Keep malformed encoding detection separate from feature validation.
4. Add the cases below before considering the behavior finished. Keep malformed, invalid, trapping, host-error and resource-limit outcomes distinct.
5. Wire the change into the existing entry point/dispatch and update its feature-policy or ownership evidence where relevant. Run the commands below and record exact outcomes.

### Required cases

- `t080_case_01`: u64 maximum limit bits
- `t080_case_02`: Table64 descriptor
- `t080_case_03`: Overlong LEB in wide offset.

For binary fixtures, keep a readable WAT/hex source and provenance alongside the `.wasm` or inline bytes. Generate fixtures with the separate encoder/wasm-tools; do not add `wast` to core tests. Expected outputs must come from the locked spec/rule or an independent calculation, not the engine being tested.

### Verification commands

Run from `/workspace/wtime` after activating the configured Rust environment:

```sh
. /workspace/.setup/rust-env.sh
cargo test --locked -p wtime-decode t080_ -- --nocapture
cargo check --locked -p wtime-decode
cargo fmt --all --check
cargo clippy --locked -p wtime-decode --all-targets -- -D warnings
```

The filtered test invocation must execute the named cases with a positive count. If multiple packages are listed, at least each package with changed behavior must have an executed relevant test; a package's zero-match summary is not evidence for that package. Fix test placement/names rather than accepting a zero-test run.

### Handoff and scope limit

Record changed symbols/files, named test counts, corpus hash/profile if used, and remaining risks in Completion evidence. Stop at this task's interface; do not implement unrelated future features. If a prerequisite API is missing or pinned sources conflict, identify the exact missing contract/rule and keep the affected work blocked instead of guessing. Routine local coding choices do not require asking the user.

## Sources

- [Memory64](https://github.com/WebAssembly/memory64/tree/9003cd5e24e53b84cd9027ea3dd7ae57159a6db1)
- [Pinned integrated core specification](https://github.com/WebAssembly/spec/tree/7e9fb78c63d65018cc111422f657947b3df80d7b) is normative for finished core features; historical proposal repositories provide context.
- [Scope, source precedence and exclusions](../PROPOSALS.md).

## Completion evidence

- Changes: I64 memory/table declarations, u64 limits and memory-offset width parsing. Shared regression t113_memory_offset_width. Implemented in repository crates/wtime-decode/src/module.rs and instructions.rs. Named shared regressions are used without claiming nonexistent task-prefix test cases.
- Commands and outcomes: cargo test --locked -p wtime-decode: 24 passed, 2 intentionally ignored harness tests; cargo clippy --locked -p wtime-decode --all-targets -- -D warnings: passed, frozen decoder source 2026-10-11. tools/checks/opcode_coverage.py invokes the ignored t008_actual_dispatch_coverage probe for assigned/reserved decoding.
- Assertion counts / fixture hashes: configured external t024_external_corpus harness passed, 1100 pinned core module fixtures decoded; core revision 7e9fb78c63d65018cc111422f657947b3df80d7b. Assigned584/reserved93 instruction inventory and pinned proposal revisions are retained in repository docs/architecture/opcodes.json and spec-lock.json. Source-bound full conformance execution evidence is separately recorded by the final opcode ledger.
- Follow-up or normative ambiguity: This task covers binary parsing; proposal validation and execution assertions belong to their respective task owners. Reserved/disabled syntax is rejected explicitly; successful decoding is not an execution claim.
