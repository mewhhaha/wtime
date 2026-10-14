# T138 — Validate continuation types and result-bearing tags

Status: **done**  
Area: `wtime-validate`  
Work unit: one focused behavior/contract and its tests; split into child tasks before exceeding two focused patches.

## Prerequisites

[T137 — Decode continuation types, tags and stack-switching instructions](../11-stack-switching/T137-cont-decode.md), [T122 — Validate exception tags and handler label signatures](../09-exception-handling/T122-eh-validate.md), [T107 — Validate recursive types, subtyping and defaultability](../08-gc-and-type-system/T107-gc-type-validate.md)

Use the predecessor's interfaces and evidence. Do not reimplement a second parser, type registry, store or execution stack.

## Deliverable

Extend recursive type matching, continuation subtyping and tag validation with pinned input/output variance and restrictions. Retain EH rules where result-bearing tags are not legal. Amend existing ref.test/ref.cast/br_on_cast/br_on_cast_fail validation: target reference types must not be subtypes of (ref null cont), per the pinned proposal.

## Acceptance

Wrong function-type targets and invalid tag use reject; valid recursive cont types and result-bearing control tags validate. Include all four prohibited continuation-target cast forms and valid unrelated GC casts, so adding continuations cannot silently loosen GC validation.

Add focused tests for the changed behavior and relevant failure boundaries. Record the exact command, feature profile, corpus/source revision and executed assertion counts. Follow [the common definition of done](../EXECUTION.md#common-definition-of-done); a missing directive, zero-test run or unimplemented branch is not a pass.

## Boundaries

Follow [ARCHITECTURE.md](../ARCHITECTURE.md): no external core production dependency, no per-proposal Cargo-crate proliferation, no JS/browser implementation, and no invented semantics for draft TODOs. Coordinate changes to shared types/opcode/control contracts with other ready tasks. Runtime feature gates and implementation resource limits must remain explicit.

## Implementation instructions for Sol 6.1 Low

Read [the interface contracts](../IMPLEMENTATION_CONTRACTS.md) and the listed prerequisite outputs before coding. Use the predecessor's API; do not invent a second implementation to avoid a missing prerequisite.

### Files and ownership

Expected paths in `/workspace/wtime` (created by implementation, not by this plan):

- `crates/wtime-validate/src/types.rs`
- `crates/wtime-validate/src/function/gc.rs`

Add focused tests in the relevant package's `tests/` directory, or its existing unit-test module. Name tests `t138_case_01`, `..._case_02`, etc. Reuse existing feature modules when present; coordinate edits to central dispatch/type tables.

### Ordered recipe

1. Check continuation input/output type matching and control-tag result rules with a bounded worklist.
2. Extend canonical-type shape compatibility without enabling casts to continuation refs.
3. Explicitly reject ref.test/ref.cast/br_on_cast/br_on_cast_fail targets that are subtypes of (ref null cont).
4. Add the cases below before considering the behavior finished. Keep malformed, invalid, trapping, host-error and resource-limit outcomes distinct.
5. Wire the change into the existing entry point/dispatch and update its feature-policy or ownership evidence where relevant. Run the commands below and record exact outcomes.

### Required cases

- `t138_case_01`: Each forbidden cast target variant
- `t138_case_02`: Valid unrelated GC cast remains valid
- `t138_case_03`: Wrong continuation function signature.

For binary fixtures, keep a readable WAT/hex source and provenance alongside the `.wasm` or inline bytes. Generate fixtures with the separate encoder/wasm-tools; do not add `wast` to core tests. Expected outputs must come from the locked spec/rule or an independent calculation, not the engine being tested.

### Verification commands

Run from `/workspace/wtime` after activating the configured Rust environment:

```sh
. /workspace/.setup/rust-env.sh
cargo test --locked -p wtime-validate t138_ -- --nocapture
cargo check --locked -p wtime-validate
cargo fmt --all --check
cargo clippy --locked -p wtime-validate --all-targets -- -D warnings
```

The filtered test invocation must execute the named cases with a positive count. If multiple packages are listed, at least each package with changed behavior must have an executed relevant test; a package's zero-match summary is not evidence for that package. Fix test placement/names rather than accepting a zero-test run.

### Handoff and scope limit

Record changed symbols/files, named test counts, corpus hash/profile if used, and remaining risks in Completion evidence. Stop at this task's interface; do not implement unrelated future features. If a prerequisite API is missing or pinned sources conflict, identify the exact missing contract/rule and keep the affected work blocked instead of guessing. Routine local coding choices do not require asking the user.

## Sources

- [Stack Switching](https://github.com/WebAssembly/stack-switching/tree/c7be390539e5fc10669c88cadc4fa4b32fba1d47)
- [Pinned integrated core specification](https://github.com/WebAssembly/spec/tree/7e9fb78c63d65018cc111422f657947b3df80d7b) is normative for finished core features; historical proposal repositories provide context.
- [Scope, source precedence and exclusions](../PROPOSALS.md).

## Completion evidence

- Changes: Continuation targets must be function types; continuation subtyping follows function input/output variance. Control tags may return values. All ref.test/ref.cast/br_on_cast/br_on_cast_fail continuation-target subtypes are explicitly rejected while unrelated GC casts remain valid.
- Handler: `crates/wtime-validate/src/types.rs` — `types::validate_types continuation composite; Context::add_tag; Context::gc cast checks`.
- Focused/shared tests: `feature_profiles::t162_case_03`, `continuation_tests::t139_generator_bind`, `types::premise_tests::t107_reference_nullability_and_bottom_lattice_matrix`.
- Commands and outcomes: `cargo test --locked -p wtime-validate` 90 passed; `cargo test --locked -p wtime-runtime --test feature_profiles` 28 passed; `cargo test --locked -p wtime-conformance --test interactions` 4 passed; `cargo test --locked -p wtime-runtime t152_` one actual matching test passed (three exact-import cells). Decoder owner reports 24 decoder tests passed. Other binaries' zero-match summaries are not proof.
- Locked corpus: `target/conformance/final-phase3.json`, profile `all-phase3-core`, suite `phase3`, 64297 executed, zero failures/skips/unsupported; binary SHA256 `a0aaaa7d7c6950508f89e877a87bfcdf46d929f9b7e7efbaa9109c1f5101a1be`. 3 exact relevant selected source hashes are recorded in this task's `TASKS.json` completion evidence; live source hashes were checked before recording.
- Continuation cast boundaries: all four operation forms (`ref.test`, `ref.cast`, `br_on_cast`, `br_on_cast_fail`) have three locked invalid target variants each (abstract cont, bottom cont, defined continuation); 12 exact source/form SHA and byte-offset references are retained in `TASKS.json` (`validation.wast`, lines 802–902).
- Formal/spec evidence: pinned core `7e9fb78c63d65018cc111422f657947b3df80d7b`; actual rule predicates and exact unit/corpus forms in `docs/architecture/validation-premise-audit.json` and `docs/architecture/opcodes.json`. Historical proposal source precedence and active overlays remain explicit.
- Quality checks: strict validator Clippy passed after final source changes. No source edits were made during task reconciliation.
- Follow-up: none for validation. The release/conformance owners maintain final aggregate source bindings; decoder/runtime work is supported by the shared evidence above.
