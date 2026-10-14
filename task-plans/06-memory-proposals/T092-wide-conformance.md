# T092 — Run the complete wide arithmetic corpus

Status: **done**  
Area: `wtime-conformance`  
Work unit: one focused behavior/contract and its tests; split into child tasks before exceeding two focused patches.

## Prerequisites

[T091 — Execute 128-bit arithmetic with Rust primitives](../06-memory-proposals/T091-wide-execute.md), [T013 — Implement spectest imports and module registration](../00-foundation/T013-spectest-host.md)

Use the predecessor's interfaces and evidence. Do not reimplement a second parser, type registry, store or execution stack.

## Deliverable

Add proposal text-tool support if missing; execute every selected arithmetic assertion and randomized paired-limb cases against native Rust reference arithmetic.

## Acceptance

All low/high results, decode negatives and validation negatives pass with counts; no unsupported directive remains.

Add focused tests for the changed behavior and relevant failure boundaries. Record the exact command, feature profile, corpus/source revision and executed assertion counts. Follow [the common definition of done](../EXECUTION.md#common-definition-of-done); a missing directive, zero-test run or unimplemented branch is not a pass.

## Boundaries

Follow [ARCHITECTURE.md](../ARCHITECTURE.md): no external core production dependency, no per-proposal Cargo-crate proliferation, no JS/browser implementation, and no invented semantics for draft TODOs. Coordinate changes to shared types/opcode/control contracts with other ready tasks. Runtime feature gates and implementation resource limits must remain explicit.

## Implementation instructions for Sol 6.1 Low

Read [the interface contracts](../IMPLEMENTATION_CONTRACTS.md) and the listed prerequisite outputs before coding. Use the predecessor's API; do not invent a second implementation to avoid a missing prerequisite.

### Files and ownership

Expected paths in `/workspace/wtime` (created by implementation, not by this plan):

- `tools/wtime-conformance/tests/wide_arithmetic.rs`

Add focused tests in the relevant package's `tests/` directory, or its existing unit-test module. Name tests `t092_case_01`, `..._case_02`, etc. Reuse existing feature modules when present; coordinate edits to central dispatch/type tables.

### Ordered recipe

1. Execute every locked wide arithmetic assertion and invalid case.
2. Add seeded limb vectors checked against independent Rust bit arithmetic.
3. Verify text tooling produces the pinned assignments and reports counts.
4. Add the cases below before considering the behavior finished. Keep malformed, invalid, trapping, host-error and resource-limit outcomes distinct.
5. Wire the change into the existing entry point/dispatch and update its feature-policy or ownership evidence where relevant. Run the commands below and record exact outcomes.

### Required cases

- `t092_case_01`: Carry into high limb
- `t092_case_02`: High limb wraps
- `t092_case_03`: Swapped result limbs are detected.

For binary fixtures, keep a readable WAT/hex source and provenance alongside the `.wasm` or inline bytes. Generate fixtures with the separate encoder/wasm-tools; do not add `wast` to core tests. Expected outputs must come from the locked spec/rule or an independent calculation, not the engine being tested.

### Verification commands

Run from `/workspace/wtime` after activating the configured Rust environment:

```sh
. /workspace/.setup/rust-env.sh
cargo test --locked -p wtime-conformance t092_ -- --nocapture
cargo check --locked -p wtime-conformance
cargo fmt --all --check
cargo clippy --locked -p wtime-conformance --all-targets -- -D warnings
cargo run --locked -p wtime-conformance -- run --profile all-phase3-core --suite wide-arithmetic --filter all --report target/conformance/T092.json
```

The filtered test invocation must execute the named cases with a positive count. If multiple packages are listed, at least each package with changed behavior must have an executed relevant test; a package's zero-match summary is not evidence for that package. Fix test placement/names rather than accepting a zero-test run.

The corpus command must report every selected assertion as executed, with no unexplained failure, required skip or unsupported directive. A unit-test pass does not replace this gate.

### Handoff and scope limit

Record changed symbols/files, named test counts, corpus hash/profile if used, and remaining risks in Completion evidence. Stop at this task's interface; do not implement unrelated future features. If a prerequisite API is missing or pinned sources conflict, identify the exact missing contract/rule and keep the affected work blocked instead of guessing. Routine local coding choices do not require asking the user.

## Sources

- [Wide Arithmetic](https://github.com/WebAssembly/wide-arithmetic/tree/024d4ce1b28cee7fd006d9c5175e3b7ece52b429)
- [Pinned integrated core specification](https://github.com/WebAssembly/spec/tree/7e9fb78c63d65018cc111422f657947b3df80d7b) is normative for finished core features; historical proposal repositories provide context.
- [Scope, source precedence and exclusions](../PROPOSALS.md).

## Completion evidence

- Changes: `crates/wtime-runtime/src/execution.rs::0xfc13..=0xfc16`; `crates/wtime-runtime/src/wide.rs::execute`.
- Focused acceptance: `script::gate_acceptance_tests::t092_case_01`; `script::gate_acceptance_tests::t092_case_02`; `script::gate_acceptance_tests::t092_case_03`. All named cases executed successfully in the final workspace test run (304 passed; conformance 31 unit + 3 annotations + 7 interactions = 41 passed).
- Commands and outcomes: `cargo test --locked --workspace --all-features` and workspace/all-target/all-feature strict Clippy passed. Exact corpus replay commands, source revisions/file SHA-256 values, binary fingerprint, implementation source fingerprints, and byte-identical archived reports are recorded in `/workspace/wtime/docs/architecture/conformance-gate-evidence.json`.
- Assertion counts / fixture hashes: `final-wide-arithmetic.json`: profile `all-phase3-core`, 107/107 executed, 0 failed/skipped/unsupported; report SHA-256 `d547373fd5f6b94cf83e6c9dd02b1f0878c290fc9992e6838a842526a2bd6281`; compatibility assertions 0.
- Follow-up or normative ambiguity: None for this locked scope; unsupported directives and zero-test selection remain errors.
