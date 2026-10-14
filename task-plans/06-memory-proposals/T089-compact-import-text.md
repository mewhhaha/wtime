# T089 — Support compact import text forms in tooling

Status: **done**  
Area: `wtime-conformance`  
Work unit: one focused behavior/contract and its tests; split into child tasks before exceeding two focused patches.

## Prerequisites

[T087 — Decode both compact import encodings](../06-memory-proposals/T087-compact-import-decode.md), [T012 — Implement spec-script parsing and outcome accounting](../00-foundation/T012-script-runner.md)

Use the predecessor's interfaces and evidence. Do not reimplement a second parser, type registry, store or execution stack.

## Deliverable

Verify wast's pinned text encoder supports both item syntaxes and choose a tool-only extension if necessary. Keep compact text grammar out of core dependencies and preserve source order. This task validates binary encoding and normalization; T088/T161 own engine-backed integration and complete corpus execution.

## Acceptance

Encode both forms to expected groups and confirm normalization preserves import order. Unsupported tool syntax is a blocking tooling gap, not a runtime skip. T161 executes the full compact-import scripts after integration is available.

Add focused tests for the changed behavior and relevant failure boundaries. Record the exact command, feature profile, corpus/source revision and executed assertion counts. Follow [the common definition of done](../EXECUTION.md#common-definition-of-done); a missing directive, zero-test run or unimplemented branch is not a pass.

## Boundaries

Follow [ARCHITECTURE.md](../ARCHITECTURE.md): no external core production dependency, no per-proposal Cargo-crate proliferation, no JS/browser implementation, and no invented semantics for draft TODOs. Coordinate changes to shared types/opcode/control contracts with other ready tasks. Runtime feature gates and implementation resource limits must remain explicit.

## Implementation instructions for Sol 6.1 Low

Read [the interface contracts](../IMPLEMENTATION_CONTRACTS.md) and the listed prerequisite outputs before coding. Use the predecessor's API; do not invent a second implementation to avoid a missing prerequisite.

### Files and ownership

Expected paths in `/workspace/wtime` (created by implementation, not by this plan):

- `tools/wtime-conformance/src/encode.rs`

Add focused tests in the relevant package's `tests/` directory, or its existing unit-test module. Name tests `t089_case_01`, `..._case_02`, etc. Reuse existing feature modules when present; coordinate edits to central dispatch/type tables.

### Ordered recipe

1. Probe both item text forms using the pinned tool parser/encoder.
2. Add a tool-only grammar adapter only if required and document the precise gap.
3. Check resulting import encoding and decoder normalization; runtime corpus integration belongs to the final proposal gate.
4. Add the cases below before considering the behavior finished. Keep malformed, invalid, trapping, host-error and resource-limit outcomes distinct.
5. Wire the change into the existing entry point/dispatch and update its feature-policy or ownership evidence where relevant. Run the commands below and record exact outcomes.

### Required cases

- `t089_case_01`: Both item syntaxes
- `t089_case_02`: Ordinary plus compact order
- `t089_case_03`: Expected 0x7e/0x7f encoding and malformed text.

For binary fixtures, keep a readable WAT/hex source and provenance alongside the `.wasm` or inline bytes. Generate fixtures with the separate encoder/wasm-tools; do not add `wast` to core tests. Expected outputs must come from the locked spec/rule or an independent calculation, not the engine being tested.

### Verification commands

Run from `/workspace/wtime` after activating the configured Rust environment:

```sh
. /workspace/.setup/rust-env.sh
cargo test --locked -p wtime-conformance t089_ -- --nocapture
cargo check --locked -p wtime-conformance
cargo fmt --all --check
cargo clippy --locked -p wtime-conformance --all-targets -- -D warnings
```

The filtered test invocation must execute the named cases with a positive count. If multiple packages are listed, at least each package with changed behavior must have an executed relevant test; a package's zero-match summary is not evidence for that package. Fix test placement/names rather than accepting a zero-test run.

### Handoff and scope limit

Record changed symbols/files, named test counts, corpus hash/profile if used, and remaining risks in Completion evidence. Stop at this task's interface; do not implement unrelated future features. If a prerequisite API is missing or pinned sources conflict, identify the exact missing contract/rule and keep the affected work blocked instead of guessing. Routine local coding choices do not require asking the user.

## Sources

- [Compact Import Section](https://github.com/WebAssembly/compact-import-section/tree/0624de829e2663da799e708444dec80337a3fe54)
- [Pinned integrated core specification](https://github.com/WebAssembly/spec/tree/7e9fb78c63d65018cc111422f657947b3df80d7b) is normative for finished core features; historical proposal repositories provide context.
- [Scope, source precedence and exclusions](../PROPOSALS.md).

## Completion evidence

- Changes: `wast261.0.0::Wat::encode`; `crates/wtime-decode/src/module.rs::decode import section2 normalization`.
- Focused acceptance: `cli::compact_text_tests::t089_case_01`; `cli::compact_text_tests::t089_case_02`; `cli::compact_text_tests::t089_case_03`. All named cases executed successfully in the final workspace test run (304 passed; conformance 31 unit + 3 annotations + 7 interactions = 41 passed).
- Commands and outcomes: `cargo test --locked --workspace --all-features` and workspace/all-target/all-feature strict Clippy passed. Exact corpus replay commands, source revisions/file SHA-256 values, binary fingerprint, implementation source fingerprints, and byte-identical archived reports are recorded in `/workspace/wtime/docs/architecture/conformance-gate-evidence.json`.
- Assertion counts / fixture hashes: `final-compact-import-section.json`: profile `all-phase3-core`, 35/35 executed, 0 failed/skipped/unsupported; report SHA-256 `ea2aa9511e2ed33912c5e61e74c4cf0abd5e608af9c6a7fe52d2961db9aadbec`; compatibility assertions 0.
- Follow-up or normative ambiguity: None for this locked scope; unsupported directives and zero-test selection remain errors.
