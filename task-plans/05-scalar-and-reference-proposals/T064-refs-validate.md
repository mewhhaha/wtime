# T064 — Decode and validate reference-type instructions

Status: **done**  
Area: `decode/validate`  
Work unit: one focused behavior/contract and its tests; split into child tasks before exceeding two focused patches.

## Prerequisites

[T018 — Decode function types and storage limits](../01-binary-decoding/T018-decode-basic-types.md), [T028 — Implement operand/control stack validation](../02-validation/T028-validate-stack.md), [T032 — Validate memory, table and segment rules](../02-validation/T032-validate-memory.md), [T030 — Validate locals, globals and call instructions](../02-validation/T030-validate-calls.md)

Use the predecessor's interfaces and evidence. Do not reimplement a second parser, type registry, store or execution stack.

## Deliverable

Complete externref/funcref, typed select, ref.null/ref.is_null/ref.func and table instruction types. Enforce declared-function rules and reference-compatible element/global initializers.

## Acceptance

Invalid undeclared refs and type-incompatible tables reject; legal externrefs and reference-valued globals/segments validate.

Add focused tests for the changed behavior and relevant failure boundaries. Record the exact command, feature profile, corpus/source revision and executed assertion counts. Follow [the common definition of done](../EXECUTION.md#common-definition-of-done); a missing directive, zero-test run or unimplemented branch is not a pass.

## Boundaries

Follow [ARCHITECTURE.md](../ARCHITECTURE.md): no external core production dependency, no per-proposal Cargo-crate proliferation, no JS/browser implementation, and no invented semantics for draft TODOs. Coordinate changes to shared types/opcode/control contracts with other ready tasks. Runtime feature gates and implementation resource limits must remain explicit.

## Implementation instructions for Sol 6.1 Low

Read [the interface contracts](../IMPLEMENTATION_CONTRACTS.md) and the listed prerequisite outputs before coding. Use the predecessor's API; do not invent a second implementation to avoid a missing prerequisite.

### Files and ownership

Expected paths in `/workspace/wtime` (created by implementation, not by this plan):

- `crates/wtime-decode/src/instructions/reference.rs`
- `crates/wtime-validate/src/function/reference.rs`

Add focused tests in the relevant package's `tests/` directory, or its existing unit-test module. Name tests `t064_case_01`, `..._case_02`, etc. Reuse existing feature modules when present; coordinate edits to central dispatch/type tables.

### Ordered recipe

1. Add nullable function/external refs, reference instructions and typed select signatures.
2. Track declared function usage for ref.func and initializer constraints.
3. Validate each table instruction's element/address type.
4. Add the cases below before considering the behavior finished. Keep malformed, invalid, trapping, host-error and resource-limit outcomes distinct.
5. Wire the change into the existing entry point/dispatch and update its feature-policy or ownership evidence where relevant. Run the commands below and record exact outcomes.

### Required cases

- `t064_case_01`: Undeclared ref.func
- `t064_case_02`: Externref table with funcref set
- `t064_case_03`: Typed select with matching refs.

For binary fixtures, keep a readable WAT/hex source and provenance alongside the `.wasm` or inline bytes. Generate fixtures with the separate encoder/wasm-tools; do not add `wast` to core tests. Expected outputs must come from the locked spec/rule or an independent calculation, not the engine being tested.

### Verification commands

Run from `/workspace/wtime` after activating the configured Rust environment:

```sh
. /workspace/.setup/rust-env.sh
cargo test --locked -p wtime-decode -p wtime-validate t064_ -- --nocapture
cargo check --locked -p wtime-decode -p wtime-validate
cargo fmt --all --check
cargo clippy --locked -p wtime-decode -p wtime-validate --all-targets -- -D warnings
```

The filtered test invocation must execute the named cases with a positive count. If multiple packages are listed, at least each package with changed behavior must have an executed relevant test; a package's zero-match summary is not evidence for that package. Fix test placement/names rather than accepting a zero-test run.

### Handoff and scope limit

Record changed symbols/files, named test counts, corpus hash/profile if used, and remaining risks in Completion evidence. Stop at this task's interface; do not implement unrelated future features. If a prerequisite API is missing or pinned sources conflict, identify the exact missing contract/rule and keep the affected work blocked instead of guessing. Routine local coding choices do not require asking the user.

## Sources

- [Reference Types](https://github.com/WebAssembly/reference-types/tree/c7a3558bb5cbacb9c8879a91e08d0ac7276b740c)
- [Pinned integrated core specification](https://github.com/WebAssembly/spec/tree/7e9fb78c63d65018cc111422f657947b3df80d7b) is normative for finished core features; historical proposal repositories provide context.
- [Scope, source precedence and exclusions](../PROPOSALS.md).

## Completion evidence

- Changes: Reference stack types, typed select and declared ref.func rules include element/global/table initializers and exports.
- Handler: `crates/wtime-validate/src/lib.rs` — `Context::instruction reference/select/table arms; Context::body declared function set`.
- Focused/shared tests: `feature_profiles::t162_case_08`, `corpus_validation_regressions::t033_nonnullable_table_legacy_element`.
- Commands and outcomes: `cargo test --locked -p wtime-validate` 90 passed; `cargo test --locked -p wtime-runtime --test feature_profiles` 28 passed; `cargo test --locked -p wtime-conformance --test interactions` 4 passed; `cargo test --locked -p wtime-runtime t152_` one actual matching test passed (three exact-import cells). Decoder owner reports 24 decoder tests passed. Other binaries' zero-match summaries are not proof.
- Locked corpus: `target/conformance/final-phase3.json`, profile `all-phase3-core`, suite `phase3`, 64297 executed, zero failures/skips/unsupported; binary SHA256 `a0aaaa7d7c6950508f89e877a87bfcdf46d929f9b7e7efbaa9109c1f5101a1be`. 7 exact relevant selected source hashes are recorded in this task's `TASKS.json` completion evidence; live source hashes were checked before recording.
- Formal/spec evidence: pinned core `7e9fb78c63d65018cc111422f657947b3df80d7b`; actual rule predicates and exact unit/corpus forms in `docs/architecture/validation-premise-audit.json` and `docs/architecture/opcodes.json`. Historical proposal source precedence and active overlays remain explicit.
- Quality checks: strict validator Clippy passed after final source changes. No source edits were made during task reconciliation.
- Follow-up: none for validation. The release/conformance owners maintain final aggregate source bindings; decoder/runtime work is supported by the shared evidence above.
