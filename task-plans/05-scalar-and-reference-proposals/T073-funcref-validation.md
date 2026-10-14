# T073 — Validate typed function-reference operations

Status: **done**  
Area: `wtime-validate`  
Work unit: one focused behavior/contract and its tests; split into child tasks before exceeding two focused patches.

## Prerequisites

[T072 — Decode typed function references and recursive function types](../05-scalar-and-reference-proposals/T072-funcref-types.md), [T062 — Complete multivalue decoding and validation](../05-scalar-and-reference-proposals/T062-multivalue-validate.md), [T030 — Validate locals, globals and call instructions](../02-validation/T030-validate-calls.md)

Use the predecessor's interfaces and evidence. Do not reimplement a second parser, type registry, store or execution stack.

## Deliverable

Validate function subtyping/nullability, call_ref, reference refinements and nondefaultable locals' definite initialization. Keep type matching extensible for GC/exact types.

## Acceptance

Uninitialized nonnull locals and wrong call_ref signatures reject; branches refine ref types and preserve label values.

Add focused tests for the changed behavior and relevant failure boundaries. Record the exact command, feature profile, corpus/source revision and executed assertion counts. Follow [the common definition of done](../EXECUTION.md#common-definition-of-done); a missing directive, zero-test run or unimplemented branch is not a pass.

## Boundaries

Follow [ARCHITECTURE.md](../ARCHITECTURE.md): no external core production dependency, no per-proposal Cargo-crate proliferation, no JS/browser implementation, and no invented semantics for draft TODOs. Coordinate changes to shared types/opcode/control contracts with other ready tasks. Runtime feature gates and implementation resource limits must remain explicit.

## Implementation instructions for Sol 6.1 Low

Read [the interface contracts](../IMPLEMENTATION_CONTRACTS.md) and the listed prerequisite outputs before coding. Use the predecessor's API; do not invent a second implementation to avoid a missing prerequisite.

### Files and ownership

Expected paths in `/workspace/wtime` (created by implementation, not by this plan):

- `crates/wtime-validate/src/types.rs`
- `crates/wtime-validate/src/function/reference.rs`

Add focused tests in the relevant package's `tests/` directory, or its existing unit-test module. Name tests `t073_case_01`, `..._case_02`, etc. Reuse existing feature modules when present; coordinate edits to central dispatch/type tables.

### Ordered recipe

1. Apply function parameter contravariance/result covariance where allowed.
2. Track nondefaultable locals' initialization across structured control flow.
3. Refine branch/null paths and check call_ref target signature.
4. Add the cases below before considering the behavior finished. Keep malformed, invalid, trapping, host-error and resource-limit outcomes distinct.
5. Wire the change into the existing entry point/dispatch and update its feature-policy or ownership evidence where relevant. Run the commands below and record exact outcomes.

### Required cases

- `t073_case_01`: Uninitialized nonnull local
- `t073_case_02`: Wrong call_ref type
- `t073_case_03`: Null branch refinement plus multivalue.

For binary fixtures, keep a readable WAT/hex source and provenance alongside the `.wasm` or inline bytes. Generate fixtures with the separate encoder/wasm-tools; do not add `wast` to core tests. Expected outputs must come from the locked spec/rule or an independent calculation, not the engine being tested.

### Verification commands

Run from `/workspace/wtime` after activating the configured Rust environment:

```sh
. /workspace/.setup/rust-env.sh
cargo test --locked -p wtime-validate t073_ -- --nocapture
cargo check --locked -p wtime-validate
cargo fmt --all --check
cargo clippy --locked -p wtime-validate --all-targets -- -D warnings
```

The filtered test invocation must execute the named cases with a positive count. If multiple packages are listed, at least each package with changed behavior must have an executed relevant test; a package's zero-match summary is not evidence for that package. Fix test placement/names rather than accepting a zero-test run.

### Handoff and scope limit

Record changed symbols/files, named test counts, corpus hash/profile if used, and remaining risks in Completion evidence. Stop at this task's interface; do not implement unrelated future features. If a prerequisite API is missing or pinned sources conflict, identify the exact missing contract/rule and keep the affected work blocked instead of guessing. Routine local coding choices do not require asking the user.

## Sources

- [Typed Function References](https://github.com/WebAssembly/function-references/tree/74d2ec81d15efd3c0f2fba46a023f376101d8e46)
- [Pinned integrated core specification](https://github.com/WebAssembly/spec/tree/7e9fb78c63d65018cc111422f657947b3df80d7b) is normative for finished core features; historical proposal repositories provide context.
- [Scope, source precedence and exclusions](../PROPOSALS.md).

## Completion evidence

- Changes: Typed function subtyping/nullability, branch refinements and definite initialization share the bounded type environment and local undo log.
- Handler: `crates/wtime-validate/src/lib.rs` — `Stack::initialize/restore_locals; Context::instruction call_ref/refinement arms`.
- Focused/shared tests: `more_proposal_tests::t111_nondefaultable_local`, `more_proposal_tests::t111_initialized_local`, `corpus_regressions::t073_tail_result_covariance`, `feature_profiles::t162_case_28_typed_indirect_calls`.
- Commands and outcomes: `cargo test --locked -p wtime-validate` 90 passed; `cargo test --locked -p wtime-runtime --test feature_profiles` 28 passed; `cargo test --locked -p wtime-conformance --test interactions` 4 passed; `cargo test --locked -p wtime-runtime t152_` one actual matching test passed (three exact-import cells). Decoder owner reports 24 decoder tests passed. Other binaries' zero-match summaries are not proof.
- Locked corpus: `target/conformance/final-phase3.json`, profile `all-phase3-core`, suite `phase3`, 64297 executed, zero failures/skips/unsupported; binary SHA256 `a0aaaa7d7c6950508f89e877a87bfcdf46d929f9b7e7efbaa9109c1f5101a1be`. 4 exact relevant selected source hashes are recorded in this task's `TASKS.json` completion evidence; live source hashes were checked before recording.
- Formal/spec evidence: pinned core `7e9fb78c63d65018cc111422f657947b3df80d7b`; actual rule predicates and exact unit/corpus forms in `docs/architecture/validation-premise-audit.json` and `docs/architecture/opcodes.json`. Historical proposal source precedence and active overlays remain explicit.
- Quality checks: strict validator Clippy passed after final source changes. No source edits were made during task reconciliation.
- Follow-up: none for validation. The release/conformance owners maintain final aggregate source bindings; decoder/runtime work is supported by the shared evidence above.
