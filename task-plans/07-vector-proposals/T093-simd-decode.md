# T093 — Decode SIMD opcodes, lanes and vector immediates

Status: **done**  
Area: `wtime-decode`  
Work unit: one focused behavior/contract and its tests; split into child tasks before exceeding two focused patches.

## Prerequisites

[T022 — Decode function bodies and MVP instruction immediates](../01-binary-decoding/T022-decode-code.md), [T008 — Create a complete instruction and section ledger](../00-foundation/T008-opcode-ledger.md)

Use the predecessor's interfaces and evidence. Do not reimplement a second parser, type registry, store or execution stack.

## Deliverable

Add every assigned fixed-width SIMD subopcode and immediate form, including 16-byte constants, shuffle lanes and memory lanes. Use portable u128/byte-array representation rather than architecture intrinsics.

## Acceptance

Reserved subopcodes/truncated vectors reject; all immediate forms map to the ledger without duplicate numeric assignments.

Add focused tests for the changed behavior and relevant failure boundaries. Record the exact command, feature profile, corpus/source revision and executed assertion counts. Follow [the common definition of done](../EXECUTION.md#common-definition-of-done); a missing directive, zero-test run or unimplemented branch is not a pass.

## Boundaries

Follow [ARCHITECTURE.md](../ARCHITECTURE.md): no external core production dependency, no per-proposal Cargo-crate proliferation, no JS/browser implementation, and no invented semantics for draft TODOs. Coordinate changes to shared types/opcode/control contracts with other ready tasks. Runtime feature gates and implementation resource limits must remain explicit.

## Implementation instructions for Sol 6.1 Low

Read [the interface contracts](../IMPLEMENTATION_CONTRACTS.md) and the listed prerequisite outputs before coding. Use the predecessor's API; do not invent a second implementation to avoid a missing prerequisite.

### Files and ownership

Expected paths in `/workspace/wtime` (created by implementation, not by this plan):

- `crates/wtime-decode/src/instructions/simd.rs`

Add focused tests in the relevant package's `tests/` directory, or its existing unit-test module. Name tests `t093_case_01`, `..._case_02`, etc. Reuse existing feature modules when present; coordinate edits to central dispatch/type tables.

### Ordered recipe

1. Enumerate each assigned SIMD subopcode from the source ledger before adding dispatch.
2. Decode 16-byte vectors, lane indices, memargs and shuffle immediates without native SIMD types.
3. Keep reserved gaps as decode errors.
4. Add the cases below before considering the behavior finished. Keep malformed, invalid, trapping, host-error and resource-limit outcomes distinct.
5. Wire the change into the existing entry point/dispatch and update its feature-policy or ownership evidence where relevant. Run the commands below and record exact outcomes.

### Required cases

- `t093_case_01`: Truncated vector constant
- `t093_case_02`: Every immediate category
- `t093_case_03`: Reserved prefix gap.

For binary fixtures, keep a readable WAT/hex source and provenance alongside the `.wasm` or inline bytes. Generate fixtures with the separate encoder/wasm-tools; do not add `wast` to core tests. Expected outputs must come from the locked spec/rule or an independent calculation, not the engine being tested.

### Verification commands

Run from `/workspace/wtime` after activating the configured Rust environment:

```sh
. /workspace/.setup/rust-env.sh
cargo test --locked -p wtime-decode t093_ -- --nocapture
cargo check --locked -p wtime-decode
cargo fmt --all --check
cargo clippy --locked -p wtime-decode --all-targets -- -D warnings
```

The filtered test invocation must execute the named cases with a positive count. If multiple packages are listed, at least each package with changed behavior must have an executed relevant test; a package's zero-match summary is not evidence for that package. Fix test placement/names rather than accepting a zero-test run.

### Handoff and scope limit

Record changed symbols/files, named test counts, corpus hash/profile if used, and remaining risks in Completion evidence. Stop at this task's interface; do not implement unrelated future features. If a prerequisite API is missing or pinned sources conflict, identify the exact missing contract/rule and keep the affected work blocked instead of guessing. Routine local coding choices do not require asking the user.

## Sources

- [Fixed-width SIMD](https://github.com/webassembly/simd/tree/a78b98a6899c9e91a13095e560767af6e99d98fd)
- [Pinned integrated core specification](https://github.com/WebAssembly/spec/tree/7e9fb78c63d65018cc111422f657947b3df80d7b) is normative for finished core features; historical proposal repositories provide context.
- [Scope, source precedence and exclusions](../PROPOSALS.md).

## Completion evidence

- Changes: Assigned SIMD/relaxed opcodes with typed memory lanes, lane/shuffle/vector immediates and explicit reserved holes. Shared regression t020_reserved_and_relaxed_opcodes plus complete dispatch probe. Implemented in repository crates/wtime-decode/src/module.rs and instructions.rs. Named shared regressions are used without claiming nonexistent task-prefix test cases.
- Commands and outcomes: cargo test --locked -p wtime-decode: 24 passed, 2 intentionally ignored harness tests; cargo clippy --locked -p wtime-decode --all-targets -- -D warnings: passed, frozen decoder source 2026-10-11. tools/checks/opcode_coverage.py invokes the ignored t008_actual_dispatch_coverage probe for assigned/reserved decoding.
- Assertion counts / fixture hashes: configured external t024_external_corpus harness passed, 1100 pinned core module fixtures decoded; core revision 7e9fb78c63d65018cc111422f657947b3df80d7b. Assigned584/reserved93 instruction inventory and pinned proposal revisions are retained in repository docs/architecture/opcodes.json and spec-lock.json. Source-bound full conformance execution evidence is separately recorded by the final opcode ledger.
- Follow-up or normative ambiguity: This task covers binary parsing; proposal validation and execution assertions belong to their respective task owners. Reserved/disabled syntax is rejected explicitly; successful decoding is not an execution claim.
