# T021 — Decode data and element segment forms

Status: **done**  
Area: `wtime-decode`  
Work unit: one focused behavior/contract and its tests; split into child tasks before exceeding two focused patches.

## Prerequisites

[T020 — Decode globals, tables and memory declarations](../01-binary-decoding/T020-decode-state.md), [T019 — Decode imports, functions, exports and start](../01-binary-decoding/T019-decode-imports.md)

Use the predecessor's interfaces and evidence. Do not reimplement a second parser, type registry, store or execution stack.

## Deliverable

Decode active/passive/declarative segments, explicit memory/table indices, reference-expression elements and data-count syntax. Validate counts only where binary grammar requires it.

## Acceptance

Cover every segment flag form, zero-length payloads, declared counts and truncation; segment payload bytes remain unmodified.

Add focused tests for the changed behavior and relevant failure boundaries. Record the exact command, feature profile, corpus/source revision and executed assertion counts. Follow [the common definition of done](../EXECUTION.md#common-definition-of-done); a missing directive, zero-test run or unimplemented branch is not a pass.

## Boundaries

Follow [ARCHITECTURE.md](../ARCHITECTURE.md): no external core production dependency, no per-proposal Cargo-crate proliferation, no JS/browser implementation, and no invented semantics for draft TODOs. Coordinate changes to shared types/opcode/control contracts with other ready tasks. Runtime feature gates and implementation resource limits must remain explicit.

## Implementation instructions for Sol 6.1 Low

Read [the interface contracts](../IMPLEMENTATION_CONTRACTS.md) and the listed prerequisite outputs before coding. Use the predecessor's API; do not invent a second implementation to avoid a missing prerequisite.

### Files and ownership

Expected paths in `/workspace/wtime` (created by implementation, not by this plan):

- `crates/wtime-decode/src/sections/segments.rs`

Add focused tests in the relevant package's `tests/` directory, or its existing unit-test module. Name tests `t021_case_01`, `..._case_02`, etc. Reuse existing feature modules when present; coordinate edits to central dispatch/type tables.

### Ordered recipe

1. Dispatch segment flags to active/passive/declarative forms and parse explicit/default targets.
2. Preserve element expressions and payload byte ranges; parse data-count separately.
3. Check vector lengths against remaining bytes and policy before reserving.
4. Add the cases below before considering the behavior finished. Keep malformed, invalid, trapping, host-error and resource-limit outcomes distinct.
5. Wire the change into the existing entry point/dispatch and update its feature-policy or ownership evidence where relevant. Run the commands below and record exact outcomes.

### Required cases

- `t021_case_01`: Each legal segment flag
- `t021_case_02`: Reserved flag/truncated payload
- `t021_case_03`: Empty segment at boundary with explicit target.

For binary fixtures, keep a readable WAT/hex source and provenance alongside the `.wasm` or inline bytes. Generate fixtures with the separate encoder/wasm-tools; do not add `wast` to core tests. Expected outputs must come from the locked spec/rule or an independent calculation, not the engine being tested.

### Verification commands

Run from `/workspace/wtime` after activating the configured Rust environment:

```sh
. /workspace/.setup/rust-env.sh
cargo test --locked -p wtime-decode t021_ -- --nocapture
cargo check --locked -p wtime-decode
cargo fmt --all --check
cargo clippy --locked -p wtime-decode --all-targets -- -D warnings
```

The filtered test invocation must execute the named cases with a positive count. If multiple packages are listed, at least each package with changed behavior must have an executed relevant test; a package's zero-match summary is not evidence for that package. Fix test placement/names rather than accepting a zero-test run.

### Handoff and scope limit

Record changed symbols/files, named test counts, corpus hash/profile if used, and remaining risks in Completion evidence. Stop at this task's interface; do not implement unrelated future features. If a prerequisite API is missing or pinned sources conflict, identify the exact missing contract/rule and keep the affected work blocked instead of guessing. Routine local coding choices do not require asking the user.

## Sources

- [MVP](https://github.com/WebAssembly/design/tree/6e43416ebc8becd6cdf5d91d9ae7d3712111df32)
- [Bulk memory operations](https://github.com/WebAssembly/bulk-memory-operations/tree/dcaa1b6791401c29b67e8cd7929ec80949f1f849)
- [Reference Types](https://github.com/WebAssembly/reference-types/tree/c7a3558bb5cbacb9c8879a91e08d0ac7276b740c)
- [Multiple memories](https://github.com/WebAssembly/multi-memory/tree/cf8b5aa27257311b8eac80ae83f4ba22ee308064)
- [Pinned integrated core specification](https://github.com/WebAssembly/spec/tree/7e9fb78c63d65018cc111422f657947b3df80d7b) is normative for finished core features; historical proposal repositories provide context.
- [Scope, source precedence and exclusions](../PROPOSALS.md).

## Completion evidence

- Changes: Active/passive/declarative element/data segments, typed reference expressions and source spans. Shared decoder unit tests cover these APIs in reader.rs, module.rs and instructions.rs; test prefixes reflect shared regression ownership.
- Commands and outcomes: cargo test --locked -p wtime-decode: 24 passed, 2 intentionally ignored external/ledger harness tests; cargo clippy --locked -p wtime-decode --all-targets -- -D warnings: passed. Frozen source verified 2026-10-11.
- Assertion counts / fixture hashes: WTIME_DECODE_CORPUS=/tmp/decoder-pinned cargo test --locked -p wtime-decode t024_external_corpus -- --ignored --nocapture: 1 harness test passed, 1100 extracted pinned core module fixtures decoded. Core revision 7e9fb78c63d65018cc111422f657947b3df80d7b; corpus provenance in repository docs/architecture/corpus-lock.json. Strict assigned/reserved instruction dispatch is additionally checked by tools/checks/opcode_coverage.py.
- Follow-up or normative ambiguity: Full validation/execution conformance and per-rule semantic audits are separately tracked; these decoder tasks do not claim runtime behavior from successful parsing.
