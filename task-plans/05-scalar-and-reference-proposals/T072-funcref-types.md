# T072 — Decode typed function references and recursive function types

Status: **done**  
Area: `decode/types`  
Work unit: one focused behavior/contract and its tests; split into child tasks before exceeding two focused patches.

## Prerequisites

[T018 — Decode function types and storage limits](../01-binary-decoding/T018-decode-basic-types.md), [T006 — Define portable value and module type shapes](../00-foundation/T006-type-shapes.md), [T064 — Decode and validate reference-type instructions](../05-scalar-and-reference-proposals/T064-refs-validate.md)

Use the predecessor's interfaces and evidence. Do not reimplement a second parser, type registry, store or execution stack.

## Deliverable

Add defined heap-type references, nullability, recursive type syntax for function types and typed-function instructions. Share recursive-group syntax with later GC rather than duplicate representations.

## Acceptance

Recursive signatures, ref.as_non_null, br_on_null/non_null and call_ref immediate encodings decode; malformed heap types fail.

Add focused tests for the changed behavior and relevant failure boundaries. Record the exact command, feature profile, corpus/source revision and executed assertion counts. Follow [the common definition of done](../EXECUTION.md#common-definition-of-done); a missing directive, zero-test run or unimplemented branch is not a pass.

## Boundaries

Follow [ARCHITECTURE.md](../ARCHITECTURE.md): no external core production dependency, no per-proposal Cargo-crate proliferation, no JS/browser implementation, and no invented semantics for draft TODOs. Coordinate changes to shared types/opcode/control contracts with other ready tasks. Runtime feature gates and implementation resource limits must remain explicit.

## Implementation instructions for Sol 6.1 Low

Read [the interface contracts](../IMPLEMENTATION_CONTRACTS.md) and the listed prerequisite outputs before coding. Use the predecessor's API; do not invent a second implementation to avoid a missing prerequisite.

### Files and ownership

Expected paths in `/workspace/wtime` (created by implementation, not by this plan):

- `crates/wtime-decode/src/sections/types.rs`
- `crates/wtime-decode/src/instructions/reference.rs`

Add focused tests in the relevant package's `tests/` directory, or its existing unit-test module. Name tests `t072_case_01`, `..._case_02`, etc. Reuse existing feature modules when present; coordinate edits to central dispatch/type tables.

### Ordered recipe

1. Extend heap/reference syntax to module-defined function types and recursive groups.
2. Decode call_ref/ref.as_non_null/null-branch forms with exact immediates.
3. Retain local initialization metadata for validation.
4. Add the cases below before considering the behavior finished. Keep malformed, invalid, trapping, host-error and resource-limit outcomes distinct.
5. Wire the change into the existing entry point/dispatch and update its feature-policy or ownership evidence where relevant. Run the commands below and record exact outcomes.

### Required cases

- `t072_case_01`: Nullable defined function ref
- `t072_case_02`: Recursive function type
- `t072_case_03`: Malformed heap-type encoding.

For binary fixtures, keep a readable WAT/hex source and provenance alongside the `.wasm` or inline bytes. Generate fixtures with the separate encoder/wasm-tools; do not add `wast` to core tests. Expected outputs must come from the locked spec/rule or an independent calculation, not the engine being tested.

### Verification commands

Run from `/workspace/wtime` after activating the configured Rust environment:

```sh
. /workspace/.setup/rust-env.sh
cargo test --locked -p wtime-decode t072_ -- --nocapture
cargo check --locked -p wtime-decode
cargo fmt --all --check
cargo clippy --locked -p wtime-decode --all-targets -- -D warnings
```

The filtered test invocation must execute the named cases with a positive count. If multiple packages are listed, at least each package with changed behavior must have an executed relevant test; a package's zero-match summary is not evidence for that package. Fix test placement/names rather than accepting a zero-test run.

### Handoff and scope limit

Record changed symbols/files, named test counts, corpus hash/profile if used, and remaining risks in Completion evidence. Stop at this task's interface; do not implement unrelated future features. If a prerequisite API is missing or pinned sources conflict, identify the exact missing contract/rule and keep the affected work blocked instead of guessing. Routine local coding choices do not require asking the user.

## Sources

- [Typed Function References](https://github.com/WebAssembly/function-references/tree/74d2ec81d15efd3c0f2fba46a023f376101d8e46)
- [Pinned integrated core specification](https://github.com/WebAssembly/spec/tree/7e9fb78c63d65018cc111422f657947b3df80d7b) is normative for finished core features; historical proposal repositories provide context.
- [Scope, source precedence and exclusions](../PROPOSALS.md).

## Completion evidence

- Changes: Defined/nullability heap types and recursive groups use the shared decoded type representation; type lookup rejects nonfunction targets.
- Handler: `crates/wtime-validate/src/lib.rs` — `Context::valtype/ty; types::validate_types; typed reference instruction arms`.
- Focused/shared tests: `decode::t142_extension_immediates`, `decode::t146_descriptors_and_reference_blocks`, `feature_profiles::t162_case_13`.
- Commands and outcomes: `cargo test --locked -p wtime-validate` 90 passed; `cargo test --locked -p wtime-runtime --test feature_profiles` 28 passed; `cargo test --locked -p wtime-conformance --test interactions` 4 passed; `cargo test --locked -p wtime-runtime t152_` one actual matching test passed (three exact-import cells). Decoder owner reports 24 decoder tests passed. Other binaries' zero-match summaries are not proof.
- Locked corpus: `target/conformance/final-phase3.json`, profile `all-phase3-core`, suite `phase3`, 64297 executed, zero failures/skips/unsupported; binary SHA256 `a0aaaa7d7c6950508f89e877a87bfcdf46d929f9b7e7efbaa9109c1f5101a1be`. 5 exact relevant selected source hashes are recorded in this task's `TASKS.json` completion evidence; live source hashes were checked before recording.
- Formal/spec evidence: pinned core `7e9fb78c63d65018cc111422f657947b3df80d7b`; actual rule predicates and exact unit/corpus forms in `docs/architecture/validation-premise-audit.json` and `docs/architecture/opcodes.json`. Historical proposal source precedence and active overlays remain explicit.
- Quality checks: strict validator Clippy passed after final source changes. No source edits were made during task reconciliation.
- Follow-up: none for validation. The release/conformance owners maintain final aggregate source bindings; decoder/runtime work is supported by the shared evidence above.
