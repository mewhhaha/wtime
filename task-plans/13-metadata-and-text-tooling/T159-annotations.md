# T159 — Support standard annotations in conformance text tooling

Status: **done**  
Area: `wtime-conformance`  
Work unit: one focused behavior/contract and its tests; split into child tasks before exceeding two focused patches.

## Prerequisites

[T012 — Implement spec-script parsing and outcome accounting](../00-foundation/T012-script-runner.md), [T023 — Preserve custom sections and optional diagnostic names](../01-binary-decoding/T023-decode-custom.md)

Use the predecessor's interfaces and evidence. Do not reimplement a second parser, type registry, store or execution stack.

## Deliverable

Verify annotation grammar support in wast and document tool-only text assembly. Handle known/unknown annotations with their specified placement/encoding/ignore rules; runtime CLI remains binary-first.

## Acceptance

Annotated and equivalent plain WAT encode/execute equivalently where required; malformed annotation syntax fails tooling. No production dependency is added.

Add focused tests for the changed behavior and relevant failure boundaries. Record the exact command, feature profile, corpus/source revision and executed assertion counts. Follow [the common definition of done](../EXECUTION.md#common-definition-of-done); a missing directive, zero-test run or unimplemented branch is not a pass.

## Boundaries

Follow [ARCHITECTURE.md](../ARCHITECTURE.md): no external core production dependency, no per-proposal Cargo-crate proliferation, no JS/browser implementation, and no invented semantics for draft TODOs. Coordinate changes to shared types/opcode/control contracts with other ready tasks. Runtime feature gates and implementation resource limits must remain explicit.

## Implementation instructions for Sol 6.1 Low

Read [the interface contracts](../IMPLEMENTATION_CONTRACTS.md) and the listed prerequisite outputs before coding. Use the predecessor's API; do not invent a second implementation to avoid a missing prerequisite.

### Files and ownership

Expected paths in `/workspace/wtime` (created by implementation, not by this plan):

- `tools/wtime-conformance/src/encode.rs`
- `tools/wtime-conformance/tests/annotations.rs`

Add focused tests in the relevant package's `tests/` directory, or its existing unit-test module. Name tests `t159_case_01`, `..._case_02`, etc. Reuse existing feature modules when present; coordinate edits to central dispatch/type tables.

### Ordered recipe

1. Verify pinned wast accepts allowed annotations at every specified grammar site.
2. Add isolated tool grammar support only for any missing forms.
3. Check encoded custom payloads/ignore behavior while keeping core binary interpreter independent.
4. Add the cases below before considering the behavior finished. Keep malformed, invalid, trapping, host-error and resource-limit outcomes distinct.
5. Wire the change into the existing entry point/dispatch and update its feature-policy or ownership evidence where relevant. Run the commands below and record exact outcomes.

### Required cases

- `t159_case_01`: Unknown allowed annotation
- `t159_case_02`: Malformed annotation rejects in tooling
- `t159_case_03`: Equivalent annotated/plain module results.

For binary fixtures, keep a readable WAT/hex source and provenance alongside the `.wasm` or inline bytes. Generate fixtures with the separate encoder/wasm-tools; do not add `wast` to core tests. Expected outputs must come from the locked spec/rule or an independent calculation, not the engine being tested.

### Verification commands

Run from `/workspace/wtime` after activating the configured Rust environment:

```sh
. /workspace/.setup/rust-env.sh
cargo test --locked -p wtime-conformance t159_ -- --nocapture
cargo check --locked -p wtime-conformance
cargo fmt --all --check
cargo clippy --locked -p wtime-conformance --all-targets -- -D warnings
```

The filtered test invocation must execute the named cases with a positive count. If multiple packages are listed, at least each package with changed behavior must have an executed relevant test; a package's zero-match summary is not evidence for that package. Fix test placement/names rather than accepting a zero-test run.

### Handoff and scope limit

Record changed symbols/files, named test counts, corpus hash/profile if used, and remaining risks in Completion evidence. Stop at this task's interface; do not implement unrelated future features. If a prerequisite API is missing or pinned sources conflict, identify the exact missing contract/rule and keep the affected work blocked instead of guessing. Routine local coding choices do not require asking the user.

## Sources

- [Custom Annotation Syntax in the Text Format](https://github.com/WebAssembly/annotations/tree/20a8e4f125d5d856aac392105c35055af9ed49f7)
- [Pinned integrated core specification](https://github.com/WebAssembly/spec/tree/7e9fb78c63d65018cc111422f657947b3df80d7b) is normative for finished core features; historical proposal repositories provide context.
- [Scope, source precedence and exclusions](../PROPOSALS.md).

## Completion evidence

- Changes: Text annotations are handled solely by the isolated text tool; equivalent binaries and malformed syntax are checked.
- Commands and outcomes: Annotations integration 3 passed.
- Evidence: `/workspace/wtime/docs/verification/annotations.json`.
- Follow-up or normative ambiguity: No additional behavior claimed beyond the pinned core scope; final release also requires native aarch64 evidence.
