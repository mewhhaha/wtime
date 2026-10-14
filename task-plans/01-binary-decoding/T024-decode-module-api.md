# T024 — Expose owned decoded modules without unsafe borrowing

Status: **done**  
Area: `wtime-decode`  
Work unit: one focused behavior/contract and its tests; split into child tasks before exceeding two focused patches.

## Prerequisites

[T019 — Decode imports, functions, exports and start](../01-binary-decoding/T019-decode-imports.md), [T020 — Decode globals, tables and memory declarations](../01-binary-decoding/T020-decode-state.md), [T021 — Decode data and element segment forms](../01-binary-decoding/T021-decode-segments.md), [T022 — Decode function bodies and MVP instruction immediates](../01-binary-decoding/T022-decode-code.md), [T023 — Preserve custom sections and optional diagnostic names](../01-binary-decoding/T023-decode-custom.md)

Use the predecessor's interfaces and evidence. Do not reimplement a second parser, type registry, store or execution stack.

## Deliverable

Use immutable owned bytes/shared Arc storage and range-based views to avoid self-referential structs. Expose body and metadata access behind stable APIs; keep module construction internal.

## Acceptance

Clone/drop/view lifetimes are safe; callers cannot create impossible ranges or mutate decoded bytes. Parsing and retained-memory limits are measured.

Add focused tests for the changed behavior and relevant failure boundaries. Record the exact command, feature profile, corpus/source revision and executed assertion counts. Follow [the common definition of done](../EXECUTION.md#common-definition-of-done); a missing directive, zero-test run or unimplemented branch is not a pass.

## Boundaries

Follow [ARCHITECTURE.md](../ARCHITECTURE.md): no external core production dependency, no per-proposal Cargo-crate proliferation, no JS/browser implementation, and no invented semantics for draft TODOs. Coordinate changes to shared types/opcode/control contracts with other ready tasks. Runtime feature gates and implementation resource limits must remain explicit.

## Implementation instructions for Sol 6.1 Low

Read [the interface contracts](../IMPLEMENTATION_CONTRACTS.md) and the listed prerequisite outputs before coding. Use the predecessor's API; do not invent a second implementation to avoid a missing prerequisite.

### Files and ownership

Expected paths in `/workspace/wtime` (created by implementation, not by this plan):

- `crates/wtime-decode/src/module.rs`

Add focused tests in the relevant package's `tests/` directory, or its existing unit-test module. Name tests `t024_case_01`, `..._case_02`, etc. Reuse existing feature modules when present; coordinate edits to central dispatch/type tables.

### Ordered recipe

1. Wrap the original bytes in Arc-backed storage and keep ranges rather than self-borrows.
2. Make constructors internal and expose immutable declarations/body iterators.
3. Audit every retained range and ensure partially decoded modules cannot escape on failure.
4. Add the cases below before considering the behavior finished. Keep malformed, invalid, trapping, host-error and resource-limit outcomes distinct.
5. Wire the change into the existing entry point/dispatch and update its feature-policy or ownership evidence where relevant. Run the commands below and record exact outcomes.

### Required cases

- `t024_case_01`: Clone then drop original owner
- `t024_case_02`: Empty body/custom section ranges
- `t024_case_03`: Mutation and invalid range construction unavailable publicly.

For binary fixtures, keep a readable WAT/hex source and provenance alongside the `.wasm` or inline bytes. Generate fixtures with the separate encoder/wasm-tools; do not add `wast` to core tests. Expected outputs must come from the locked spec/rule or an independent calculation, not the engine being tested.

### Verification commands

Run from `/workspace/wtime` after activating the configured Rust environment:

```sh
. /workspace/.setup/rust-env.sh
cargo test --locked -p wtime-decode t024_ -- --nocapture
cargo check --locked -p wtime-decode
cargo fmt --all --check
cargo clippy --locked -p wtime-decode --all-targets -- -D warnings
```

The filtered test invocation must execute the named cases with a positive count. If multiple packages are listed, at least each package with changed behavior must have an executed relevant test; a package's zero-match summary is not evidence for that package. Fix test placement/names rather than accepting a zero-test run.

### Handoff and scope limit

Record changed symbols/files, named test counts, corpus hash/profile if used, and remaining risks in Completion evidence. Stop at this task's interface; do not implement unrelated future features. If a prerequisite API is missing or pinned sources conflict, identify the exact missing contract/rule and keep the affected work blocked instead of guessing. Routine local coding choices do not require asking the user.

## Sources

- [MVP](https://github.com/WebAssembly/design/tree/6e43416ebc8becd6cdf5d91d9ae7d3712111df32)
- [Pinned integrated core specification](https://github.com/WebAssembly/spec/tree/7e9fb78c63d65018cc111422f657947b3df80d7b) is normative for finished core features; historical proposal repositories provide context.
- [Scope, source precedence and exclusions](../PROPOSALS.md).

## Completion evidence

- Changes: Public feature/limit-aware decode API returns Arc-backed immutable modules and precise diagnostics. Shared decoder unit tests cover these APIs in reader.rs, module.rs and instructions.rs; test prefixes reflect shared regression ownership.
- Commands and outcomes: cargo test --locked -p wtime-decode: 24 passed, 2 intentionally ignored external/ledger harness tests; cargo clippy --locked -p wtime-decode --all-targets -- -D warnings: passed. Frozen source verified 2026-10-11.
- Assertion counts / fixture hashes: WTIME_DECODE_CORPUS=/tmp/decoder-pinned cargo test --locked -p wtime-decode t024_external_corpus -- --ignored --nocapture: 1 harness test passed, 1100 extracted pinned core module fixtures decoded. Core revision 7e9fb78c63d65018cc111422f657947b3df80d7b; corpus provenance in repository docs/architecture/corpus-lock.json. Strict assigned/reserved instruction dispatch is additionally checked by tools/checks/opcode_coverage.py.
- Follow-up or normative ambiguity: Full validation/execution conformance and per-rule semantic audits are separately tracked; these decoder tasks do not claim runtime behavior from successful parsing.
