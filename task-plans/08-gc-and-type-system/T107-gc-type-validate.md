# T107 — Validate recursive types, subtyping and defaultability

Status: **done**  
Area: `wtime-validate`  
Work unit: one focused behavior/contract and its tests; split into child tasks before exceeding two focused patches.

## Prerequisites

[T106 — Decode recursive groups, structs, arrays and heap types](../08-gc-and-type-system/T106-gc-type-decode.md), [T073 — Validate typed function-reference operations](../05-scalar-and-reference-proposals/T073-funcref-validation.md)

Use the predecessor's interfaces and evidence. Do not reimplement a second parser, type registry, store or execution stack.

## Deliverable

Check group well-formedness, supertype ordering/finality and declared structural subtyping with mutable-field invariance and immutable-field covariance. Use bounded iterative algorithms for recursive graphs.

## Acceptance

Invalid recursive groups, incompatible mutable fields and final-type extension reject; equivalent valid recursive graphs terminate without recursive Rust-stack overflow.

Add focused tests for the changed behavior and relevant failure boundaries. Record the exact command, feature profile, corpus/source revision and executed assertion counts. Follow [the common definition of done](../EXECUTION.md#common-definition-of-done); a missing directive, zero-test run or unimplemented branch is not a pass.

## Boundaries

Follow [ARCHITECTURE.md](../ARCHITECTURE.md): no external core production dependency, no per-proposal Cargo-crate proliferation, no JS/browser implementation, and no invented semantics for draft TODOs. Coordinate changes to shared types/opcode/control contracts with other ready tasks. Runtime feature gates and implementation resource limits must remain explicit.

## Implementation instructions for Sol 6.1 Low

Read [the interface contracts](../IMPLEMENTATION_CONTRACTS.md) and the listed prerequisite outputs before coding. Use the predecessor's API; do not invent a second implementation to avoid a missing prerequisite.

### Files and ownership

Expected paths in `/workspace/wtime` (created by implementation, not by this plan):

- `crates/wtime-validate/src/types.rs`

Add focused tests in the relevant package's `tests/` directory, or its existing unit-test module. Name tests `t107_case_01`, `..._case_02`, etc. Reuse existing feature modules when present; coordinate edits to central dispatch/type tables.

### Ordered recipe

1. Check referenced group members, composite kinds, supertype ordering and finality first.
2. Compare structural subtype obligations with immutable covariance, mutable invariance and function parameter/result variance.
3. Use an explicit memoized worklist and a work budget; do not recurse on the Rust stack.
4. Add the cases below before considering the behavior finished. Keep malformed, invalid, trapping, host-error and resource-limit outcomes distinct.
5. Wire the change into the existing entry point/dispatch and update its feature-policy or ownership evidence where relevant. Run the commands below and record exact outcomes.

### Required cases

- `t107_case_01`: Valid immutable width subtype
- `t107_case_02`: Invalid mutable field widening
- `t107_case_03`: Invalid final parent/cycle and function variance reversal.

For binary fixtures, keep a readable WAT/hex source and provenance alongside the `.wasm` or inline bytes. Generate fixtures with the separate encoder/wasm-tools; do not add `wast` to core tests. Expected outputs must come from the locked spec/rule or an independent calculation, not the engine being tested.

### Verification commands

Run from `/workspace/wtime` after activating the configured Rust environment:

```sh
. /workspace/.setup/rust-env.sh
cargo test --locked -p wtime-validate t107_ -- --nocapture
cargo check --locked -p wtime-validate
cargo fmt --all --check
cargo clippy --locked -p wtime-validate --all-targets -- -D warnings
```

The filtered test invocation must execute the named cases with a positive count. If multiple packages are listed, at least each package with changed behavior must have an executed relevant test; a package's zero-match summary is not evidence for that package. Fix test placement/names rather than accepting a zero-test run.

### Handoff and scope limit

Record changed symbols/files, named test counts, corpus hash/profile if used, and remaining risks in Completion evidence. Stop at this task's interface; do not implement unrelated future features. If a prerequisite API is missing or pinned sources conflict, identify the exact missing contract/rule and keep the affected work blocked instead of guessing. Routine local coding choices do not require asking the user.

## Sources

- [Garbage collection](https://github.com/WebAssembly/gc/tree/756060f5816c7e2159f4817fbdee76cf52f9c923)
- [Typed Function References](https://github.com/WebAssembly/function-references/tree/74d2ec81d15efd3c0f2fba46a023f376101d8e46)
- [Pinned integrated core specification](https://github.com/WebAssembly/spec/tree/7e9fb78c63d65018cc111422f657947b3df80d7b) is normative for finished core features; historical proposal repositories provide context.
- [Scope, source precedence and exclusions](../PROPOSALS.md).

## Completion evidence

- Changes: Recursive groups/finality/ordering/defaultability and structural variance use iterative equivalence/work limits; empty recursion groups are valid. Fifty-thousand declared subtype chain is stack-safe and budgeted.
- Handler: `crates/wtime-validate/src/types.rs` — `types::validate_types/value_subtype/heap_subtype/equivalent`.
- Focused/shared tests: `proposal_tests::t107_case_01`, `proposal_tests::t107_case_02`, `proposal_tests::t107_case_03`, `corpus_validation_regressions::t107_internal_external_distinction`, `empty_rec_regression::t107_empty_rec_group_valid`, `types::premise_tests::t107_reference_nullability_and_bottom_lattice_matrix`, `types::premise_tests::t107_array_storage_covariance_and_mutable_invariance`, `acceptance::t107_function_parameter_contravariance_and_result_covariance`, `acceptance::t107_declared_array_const_covariance_and_mutable_invariance`, `acceptance::t028_case03_deep_declared_subtyping_is_iterative_and_budgeted`.
- Commands and outcomes: `cargo test --locked -p wtime-validate` 90 passed; `cargo test --locked -p wtime-runtime --test feature_profiles` 28 passed; `cargo test --locked -p wtime-conformance --test interactions` 4 passed; `cargo test --locked -p wtime-runtime t152_` one actual matching test passed (three exact-import cells). Decoder owner reports 24 decoder tests passed. Other binaries' zero-match summaries are not proof.
- Locked corpus: `target/conformance/final-phase3.json`, profile `all-phase3-core`, suite `phase3`, 64297 executed, zero failures/skips/unsupported; binary SHA256 `a0aaaa7d7c6950508f89e877a87bfcdf46d929f9b7e7efbaa9109c1f5101a1be`. 4 exact relevant selected source hashes are recorded in this task's `TASKS.json` completion evidence; live source hashes were checked before recording.
- Formal/spec evidence: pinned core `7e9fb78c63d65018cc111422f657947b3df80d7b`; actual rule predicates and exact unit/corpus forms in `docs/architecture/validation-premise-audit.json` and `docs/architecture/opcodes.json`. Historical proposal source precedence and active overlays remain explicit.
- Quality checks: strict validator Clippy passed after final source changes. No source edits were made during task reconciliation.
- Follow-up: none for validation. The release/conformance owners maintain final aggregate source bindings; decoder/runtime work is supported by the shared evidence above.
