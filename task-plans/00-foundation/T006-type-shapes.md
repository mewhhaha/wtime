# T006 — Define portable value and module type shapes

Status: **done**  
Area: `wtime-types`  
Work unit: one focused behavior/contract and its tests; split into child tasks before exceeding two focused patches.

## Prerequisites

[T004 — Define feature dependencies and resource limits](../00-foundation/T004-features.md), [T005 — Define structured errors, traps and byte spans](../00-foundation/T005-diagnostics.md), [T148 — Resolve the pinned custom-descriptor core contract](../12-custom-descriptors/T148-desc-source-audit.md)

Use the predecessor's interfaces and evidence. Do not reimplement a second parser, type registry, store or execution stack.

## Deliverable

Define raw-bit scalar values, V128, typed indices, reference categories, recursive type-group syntax, exactness, memory/table widths, tags and continuation shapes. Use explicit handles and structural representations; defer actual interning to its own task.

## Acceptance

Type-shape tests include nullable/exact refs, recursive placeholders, 64-bit tables and one-byte pages. No Rust reference or native pointer is encoded as a guest reference.

Add focused tests for the changed behavior and relevant failure boundaries. Record the exact command, feature profile, corpus/source revision and executed assertion counts. Follow [the common definition of done](../EXECUTION.md#common-definition-of-done); a missing directive, zero-test run or unimplemented branch is not a pass.

## Boundaries

Follow [ARCHITECTURE.md](../ARCHITECTURE.md): no external core production dependency, no per-proposal Cargo-crate proliferation, no JS/browser implementation, and no invented semantics for draft TODOs. Coordinate changes to shared types/opcode/control contracts with other ready tasks. Runtime feature gates and implementation resource limits must remain explicit.

## Implementation instructions for Sol 6.1 Low

Read [the interface contracts](../IMPLEMENTATION_CONTRACTS.md) and the listed prerequisite outputs before coding. Use the predecessor's API; do not invent a second implementation to avoid a missing prerequisite.

### Files and ownership

Expected paths in `/workspace/wtime` (created by implementation, not by this plan):

- `crates/wtime-types/src/indices.rs`
- `crates/wtime-types/src/types.rs`

Add focused tests in the relevant package's `tests/` directory, or its existing unit-test module. Name tests `t006_case_01`, `..._case_02`, etc. Reuse existing feature modules when present; coordinate edits to central dispatch/type tables.

### Ordered recipe

1. Add typed module indices, u64 limits, address width and page-size types.
2. Represent recursive groups, abstract/defined/exact heap types, nullability, struct/array/function/continuation shapes without embedding runtime identities.
3. Keep scalar bit carriers separate from opaque runtime reference handles; add small construction/type-shape tests.
4. Add the cases below before considering the behavior finished. Keep malformed, invalid, trapping, host-error and resource-limit outcomes distinct.
5. Wire the change into the existing entry point/dispatch and update its feature-policy or ownership evidence where relevant. Run the commands below and record exact outcomes.

### Required cases

- `t006_case_01`: One-byte memory and table64 retain widths
- `t006_case_02`: Recursive references preserve group membership
- `t006_case_03`: Exact abstract heap types cannot be constructed as valid types.

For binary fixtures, keep a readable WAT/hex source and provenance alongside the `.wasm` or inline bytes. Generate fixtures with the separate encoder/wasm-tools; do not add `wast` to core tests. Expected outputs must come from the locked spec/rule or an independent calculation, not the engine being tested.

### Verification commands

Run from `/workspace/wtime` after activating the configured Rust environment:

```sh
. /workspace/.setup/rust-env.sh
cargo test --locked -p wtime-types t006_ -- --nocapture
cargo check --locked -p wtime-types
cargo fmt --all --check
cargo clippy --locked -p wtime-types --all-targets -- -D warnings
```

The filtered test invocation must execute the named cases with a positive count. If multiple packages are listed, at least each package with changed behavior must have an executed relevant test; a package's zero-match summary is not evidence for that package. Fix test placement/names rather than accepting a zero-test run.

### Handoff and scope limit

Record changed symbols/files, named test counts, corpus hash/profile if used, and remaining risks in Completion evidence. Stop at this task's interface; do not implement unrelated future features. If a prerequisite API is missing or pinned sources conflict, identify the exact missing contract/rule and keep the affected work blocked instead of guessing. Routine local coding choices do not require asking the user.

## Sources

- [MVP](https://github.com/WebAssembly/design/tree/6e43416ebc8becd6cdf5d91d9ae7d3712111df32)
- [Multi-value](https://github.com/WebAssembly/multi-value/tree/61975c83a286b96517659eded062f4e11dbac4c7)
- [Reference Types](https://github.com/WebAssembly/reference-types/tree/c7a3558bb5cbacb9c8879a91e08d0ac7276b740c)
- [Typed Function References](https://github.com/WebAssembly/function-references/tree/74d2ec81d15efd3c0f2fba46a023f376101d8e46)
- [Garbage collection](https://github.com/WebAssembly/gc/tree/756060f5816c7e2159f4817fbdee76cf52f9c923)
- [Memory64](https://github.com/WebAssembly/memory64/tree/9003cd5e24e53b84cd9027ea3dd7ae57159a6db1)
- [Custom Page Sizes](https://github.com/WebAssembly/custom-page-sizes/tree/61c873a1860acad2772539164f284cc5110f1b1b)
- [Custom Descriptors and JS Interop](https://github.com/WebAssembly/custom-descriptors/tree/4aa477a4c1ca064fe3592b3a0307509e9059d0f9)
- [Stack Switching](https://github.com/WebAssembly/stack-switching/tree/c7be390539e5fc10669c88cadc4fa4b32fba1d47)
- [Pinned integrated core specification](https://github.com/WebAssembly/spec/tree/7e9fb78c63d65018cc111422f657947b3df80d7b) is normative for finished core features; historical proposal repositories provide context.
- [Scope, source precedence and exclusions](../PROPOSALS.md).

## Completion evidence

- Changes: Portable raw-bit scalars/V128, distinct local indices, recursive group/type syntax, exact defined refs, memory/table widths and page sizes.
- Commands and outcomes: Shared types and feature-profile tests passed; memory64/custom-page and recursive type fixtures compile and execute.
- Assertion counts / fixture hashes: crates/wtime-types/src/types.rs; runtime/tests/fixtures/feature_profiles with SHA-256 provenance.
- Follow-up or normative ambiguity: No open implementation gap within this task; later integration gates remain separately tracked.
