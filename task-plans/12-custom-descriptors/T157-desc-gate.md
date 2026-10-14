# T157 — Pass custom-descriptor core conformance

Status: **done**  
Area: `wtime-conformance`  
Work unit: one focused behavior/contract and its tests; split into child tasks before exceeding two focused patches.

## Prerequisites

[T156 — Execute descriptor-equality casts and branches](../12-custom-descriptors/T156-desc-cast.md), [T120 — Pass GC/type-system conformance and collection stress](../08-gc-and-type-system/T120-gc-gate.md)

Use the predecessor's interfaces and evidence. Do not reimplement a second parser, type registry, store or execution stack.

## Deliverable

Run all specified core tests including exact function imports, recursive groups, constant allocations, metadescriptors and cross-module linking. Add tool-only text support if required; keep JS prototype API tests explicitly excluded.

## Acceptance

Specified core assertions execute with counts; unsound examples reject; unspecified field deduplication is recorded separately rather than marked implemented.

Add focused tests for the changed behavior and relevant failure boundaries. Record the exact command, feature profile, corpus/source revision and executed assertion counts. Follow [the common definition of done](../EXECUTION.md#common-definition-of-done); a missing directive, zero-test run or unimplemented branch is not a pass.

## Boundaries

Follow [ARCHITECTURE.md](../ARCHITECTURE.md): no external core production dependency, no per-proposal Cargo-crate proliferation, no JS/browser implementation, and no invented semantics for draft TODOs. Coordinate changes to shared types/opcode/control contracts with other ready tasks. Runtime feature gates and implementation resource limits must remain explicit.

## Implementation instructions for Sol 6.1 Low

Read [the interface contracts](../IMPLEMENTATION_CONTRACTS.md) and the listed prerequisite outputs before coding. Use the predecessor's API; do not invent a second implementation to avoid a missing prerequisite.

### Files and ownership

Expected paths in `/workspace/wtime` (created by implementation, not by this plan):

- `tools/wtime-conformance/tests/descriptors.rs`

Add focused tests in the relevant package's `tests/` directory, or its existing unit-test module. Name tests `t157_case_01`, `..._case_02`, etc. Reuse existing feature modules when present; coordinate edits to central dispatch/type tables.

### Ordered recipe

1. Run specified core descriptor corpus and encode missing draft syntax only in tooling.
2. Add exact import, metadescriptor, constant constructor and cross-module regressions.
3. List JS-only excluded assertions and unspecified field-dedup separately from core pass counts.
4. Add the cases below before considering the behavior finished. Keep malformed, invalid, trapping, host-error and resource-limit outcomes distinct.
5. Wire the change into the existing entry point/dispatch and update its feature-policy or ownership evidence where relevant. Run the commands below and record exact outcomes.

### Required cases

- `t157_case_01`: Every specified core op exercised
- `t157_case_02`: Unsound import example rejected
- `t157_case_03`: Forced GC preserves described objects.

For binary fixtures, keep a readable WAT/hex source and provenance alongside the `.wasm` or inline bytes. Generate fixtures with the separate encoder/wasm-tools; do not add `wast` to core tests. Expected outputs must come from the locked spec/rule or an independent calculation, not the engine being tested.

### Verification commands

Run from `/workspace/wtime` after activating the configured Rust environment:

```sh
. /workspace/.setup/rust-env.sh
cargo test --locked -p wtime-conformance t157_ -- --nocapture
cargo check --locked -p wtime-conformance
cargo fmt --all --check
cargo clippy --locked -p wtime-conformance --all-targets -- -D warnings
cargo run --locked -p wtime-conformance -- run --profile all-phase3-core --suite custom-descs --filter all --report target/conformance/T157.json
```

The filtered test invocation must execute the named cases with a positive count. If multiple packages are listed, at least each package with changed behavior must have an executed relevant test; a package's zero-match summary is not evidence for that package. Fix test placement/names rather than accepting a zero-test run.

The corpus command must report every selected assertion as executed, with no unexplained failure, required skip or unsupported directive. A unit-test pass does not replace this gate.

### Handoff and scope limit

Record changed symbols/files, named test counts, corpus hash/profile if used, and remaining risks in Completion evidence. Stop at this task's interface; do not implement unrelated future features. If a prerequisite API is missing or pinned sources conflict, identify the exact missing contract/rule and keep the affected work blocked instead of guessing. Routine local coding choices do not require asking the user.

## Sources

- [Custom Descriptors and JS Interop](https://github.com/WebAssembly/custom-descriptors/tree/4aa477a4c1ca064fe3592b3a0307509e9059d0f9)
- [Pinned integrated core specification](https://github.com/WebAssembly/spec/tree/7e9fb78c63d65018cc111422f657947b3df80d7b) is normative for finished core features; historical proposal repositories provide context.
- [Scope, source precedence and exclusions](../PROPOSALS.md).

## Completion evidence

- Changes: `crates/wtime-runtime/src/gc_exec.rs::descriptor operations`; `crates/wtime-runtime/src/gc.rs::tracing`.
- Focused acceptance: `tests/interactions.rs::t163_case_01`; `tests/interactions.rs::t163_case_02`; `gc::tests::t154_shared_meta_descriptor_chain_and_cycle_trace`. All named cases executed successfully in the final workspace test run (304 passed; conformance 31 unit + 3 annotations + 7 interactions = 41 passed).
- Commands and outcomes: `cargo test --locked --workspace --all-features` and workspace/all-target/all-feature strict Clippy passed. Exact corpus replay commands, source revisions/file SHA-256 values, binary fingerprint, implementation source fingerprints, and byte-identical archived reports are recorded in `/workspace/wtime/docs/architecture/conformance-gate-evidence.json`.
- Assertion counts / fixture hashes: `final-custom-descs.json`: profile `all-phase3-core`, 613/613 executed, 0 failed/skipped/unsupported; report SHA-256 `ab6139231505196a91354573f0789f898ceae41431d92c935eafd7022377596c`; compatibility assertions 0. `final-interactions.json`: profile `all-phase3-core`, 15/15 executed, 0 failed/skipped/unsupported; report SHA-256 `7b5dbeb1fc0a0c502eece8d9e5e975574f410ae53c4d1d735d986449c1547119`; compatibility assertions 0.
- Follow-up or normative ambiguity: None for this locked scope; unsupported directives and zero-test selection remain errors.
