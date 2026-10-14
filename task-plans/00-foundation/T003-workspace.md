# T003 — Create the minimal Rust workspace

Status: **done**  
Area: `workspace`  
Work unit: one focused behavior/contract and its tests; split into child tasks before exceeding two focused patches.

## Prerequisites

[T002 — Write crate contracts and semantic invariants](../00-foundation/T002-crate-contracts.md)

Use the predecessor's interfaces and evidence. Do not reimplement a second parser, type registry, store or execution stack.

## Deliverable

Create wtime-types, wtime-decode, wtime-validate, wtime-runtime, wtime-cli and tools/wtime-conformance. Pin Rust initially to the verified 1.99.0 unless MSRV review justifies another pin; default-members exclude tooling. Use std, manual errors and no proc-macro frameworks.

## Acceptance

cargo check of default members succeeds; cargo tree for the core shows only internal crates. Runtime tests do not build the conformance tool transitively.

Add focused tests for the changed behavior and relevant failure boundaries. Record the exact command, feature profile, corpus/source revision and executed assertion counts. Follow [the common definition of done](../EXECUTION.md#common-definition-of-done); a missing directive, zero-test run or unimplemented branch is not a pass.

## Boundaries

Follow [ARCHITECTURE.md](../ARCHITECTURE.md): no external core production dependency, no per-proposal Cargo-crate proliferation, no JS/browser implementation, and no invented semantics for draft TODOs. Coordinate changes to shared types/opcode/control contracts with other ready tasks. Runtime feature gates and implementation resource limits must remain explicit.

## Implementation instructions for Sol 6.1 Low

Read [the interface contracts](../IMPLEMENTATION_CONTRACTS.md) and the listed prerequisite outputs before coding. Use the predecessor's API; do not invent a second implementation to avoid a missing prerequisite.

### Files and ownership

Expected paths in `/workspace/wtime` (created by implementation, not by this plan):

- `Cargo.toml`
- `Cargo.lock`
- `rust-toolchain.toml`
- `.gitignore`

This is a workspace/bootstrap task. Verify the manifests, dependency graph and build commands below; do not add meaningless runtime tests to empty crate entry points. The named cases are checklist items, not required Rust test functions.

### Ordered recipe

1. Create the six manifests and empty lib/bin entry points at the contract paths; use workspace package metadata and resolver 3.
2. Set default-members to four core libs and CLI; tools/wtime-conformance opts into wast separately.
3. Write rust-toolchain.toml and initial Cargo.lock; add target/corpus caches to .gitignore without ignoring test fixtures.
4. Add the cases below before considering the behavior finished. Keep malformed, invalid, trapping, host-error and resource-limit outcomes distinct.
5. Wire the change into the existing entry point/dispatch and update its feature-policy or ownership evidence where relevant. Run the commands below and record exact outcomes.

### Required cases

- `t003_case_01`: Core cargo tree contains only internal packages
- `t003_case_02`: cargo check default members passes
- `t003_case_03`: Explicit tooling build succeeds independently.

For binary fixtures, keep a readable WAT/hex source and provenance alongside the `.wasm` or inline bytes. Generate fixtures with the separate encoder/wasm-tools; do not add `wast` to core tests. Expected outputs must come from the locked spec/rule or an independent calculation, not the engine being tested.

### Verification commands

Run from `/workspace/wtime` after activating the configured Rust environment:

```sh
. /workspace/.setup/rust-env.sh
cargo generate-lockfile
cargo check --locked
cargo check --locked -p wtime-conformance
cargo tree --locked -p wtime-runtime
```

### Handoff and scope limit

Record changed symbols/files, named test counts, corpus hash/profile if used, and remaining risks in Completion evidence. Stop at this task's interface; do not implement unrelated future features. If a prerequisite API is missing or pinned sources conflict, identify the exact missing contract/rule and keep the affected work blocked instead of guessing. Routine local coding choices do not require asking the user.

## Sources

- [Frozen proposal scope](../PROPOSALS.md) and [architecture](../ARCHITECTURE.md).
- [Pinned integrated core specification](https://github.com/WebAssembly/spec/tree/7e9fb78c63d65018cc111422f657947b3df80d7b) is normative for finished core features; historical proposal repositories provide context.
- [Scope, source precedence and exclusions](../PROPOSALS.md).

## Completion evidence

- Changes: Created six-package Rust 2024 workspace, resolver 3, pinned Rust1.99, core-only default members and Cargo.lock.
- Commands and outcomes: cargo test --locked compiled default members; cargo test --locked -p wtime-conformance compiled tool separately; cargo metadata dependency audit passed.
- Assertion counts / fixture hashes: Initial39 Rust tests passed; tool tests separately11; later feature runs tracked separately.
- Follow-up or normative ambiguity: No open issue within this task; feature gates remain separate tasks.
