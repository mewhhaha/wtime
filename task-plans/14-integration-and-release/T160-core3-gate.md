# T160 — Pass the complete integrated Core 3.0 corpus

Status: **done**  
Area: `wtime-conformance`  
Work unit: one focused behavior/contract and its tests; split into child tasks before exceeding two focused patches.

## Prerequisites

[T058 — Pass the complete selected MVP corpus](../04-mvp-execution/T058-mvp-gate.md), [T059 — Implement sign-extension instructions end to end](../05-scalar-and-reference-proposals/T059-sign-extension.md), [T061 — Execute saturating float-to-int conversions](../05-scalar-and-reference-proposals/T061-sat-execute.md), [T063 — Complete multivalue execution and host results](../05-scalar-and-reference-proposals/T063-multivalue-execute.md), [T066 — Execute reference table operations](../05-scalar-and-reference-proposals/T066-refs-tables.md), [T068 — Execute bulk-memory operations](../05-scalar-and-reference-proposals/T068-bulk-memory.md), [T069 — Execute bulk-table operations](../05-scalar-and-reference-proposals/T069-bulk-tables.md), [T071 — Evaluate extended constant expressions](../05-scalar-and-reference-proposals/T071-extconst-evaluate.md), [T074 — Execute typed calls and reference refinements](../05-scalar-and-reference-proposals/T074-funcref-execution.md), [T076 — Execute tail calls by replacing guest frames](../05-scalar-and-reference-proposals/T076-tail-execute.md), [T079 — Execute modules with multiple memories](../06-memory-proposals/T079-multi-memory-execute.md), [T083 — Complete 64-bit growth, segments and bulk operations](../06-memory-proposals/T083-memory64-bulk-grow.md), [T101 — Pass the fixed-width SIMD conformance corpus](../07-vector-proposals/T101-simd-gate.md), [T105 — Pass relaxed SIMD conformance with allowed results](../07-vector-proposals/T105-relaxed-gate.md), [T120 — Pass GC/type-system conformance and collection stress](../08-gc-and-type-system/T120-gc-gate.md), [T126 — Pass standardized exception-handling conformance](../09-exception-handling/T126-eh-gate.md), [T158 — Decode and inspect branch hints without changing execution](../13-metadata-and-text-tooling/T158-branch-hints.md), [T159 — Support standard annotations in conformance text tooling](../13-metadata-and-text-tooling/T159-annotations.md)

Use the predecessor's interfaces and evidence. Do not reimplement a second parser, type registry, store or execution stack.

## Deliverable

Run every selected integrated Core 3.0 test with the implemented engine; verify counts and all finished core proposal families, not only historical proposal snapshots. Keep any genuine pinned spec defects separate and reproducible.

## Acceptance

All specified assertions and current directives execute; required unexplained failures/skips block completion. Reports include fixture hashes/profile/engine revision.

Add focused tests for the changed behavior and relevant failure boundaries. Record the exact command, feature profile, corpus/source revision and executed assertion counts. Follow [the common definition of done](../EXECUTION.md#common-definition-of-done); a missing directive, zero-test run or unimplemented branch is not a pass.

## Boundaries

Follow [ARCHITECTURE.md](../ARCHITECTURE.md): no external core production dependency, no per-proposal Cargo-crate proliferation, no JS/browser implementation, and no invented semantics for draft TODOs. Coordinate changes to shared types/opcode/control contracts with other ready tasks. Runtime feature gates and implementation resource limits must remain explicit.

## Implementation instructions for Sol 6.1 Low

Read [the interface contracts](../IMPLEMENTATION_CONTRACTS.md) and the listed prerequisite outputs before coding. Use the predecessor's API; do not invent a second implementation to avoid a missing prerequisite.

### Files and ownership

Expected paths in `/workspace/wtime` (created by implementation, not by this plan):

- `tools/wtime-conformance/tests/core3.rs`

Add focused tests in the relevant package's `tests/` directory, or its existing unit-test module. Name tests `t160_case_01`, `..._case_02`, etc. Reuse existing feature modules when present; coordinate edits to central dispatch/type tables.

### Ordered recipe

1. Run immutable integrated Core 3.0 corpus with exact profile/filter manifest.
2. Compare selected/executed counts and account for every current directive.
3. Convert any corrected failure into a focused regression while retaining original normative fixture.
4. Add the cases below before considering the behavior finished. Keep malformed, invalid, trapping, host-error and resource-limit outcomes distinct.
5. Wire the change into the existing entry point/dispatch and update its feature-policy or ownership evidence where relevant. Run the commands below and record exact outcomes.

### Required cases

- `t160_case_01`: All finished core families exercised
- `t160_case_02`: No unexpected skip
- `t160_case_03`: Wrong trap/exception class fails assertion.

For binary fixtures, keep a readable WAT/hex source and provenance alongside the `.wasm` or inline bytes. Generate fixtures with the separate encoder/wasm-tools; do not add `wast` to core tests. Expected outputs must come from the locked spec/rule or an independent calculation, not the engine being tested.

### Verification commands

Run from `/workspace/wtime` after activating the configured Rust environment:

```sh
. /workspace/.setup/rust-env.sh
cargo test --locked -p wtime-conformance t160_ -- --nocapture
cargo check --locked -p wtime-conformance
cargo fmt --all --check
cargo clippy --locked -p wtime-conformance --all-targets -- -D warnings
cargo run --locked -p wtime-conformance -- run --profile stable-core --suite core3 --filter all --report target/conformance/T160.json
```

The filtered test invocation must execute the named cases with a positive count. If multiple packages are listed, at least each package with changed behavior must have an executed relevant test; a package's zero-match summary is not evidence for that package. Fix test placement/names rather than accepting a zero-test run.

The corpus command must report every selected assertion as executed, with no unexplained failure, required skip or unsupported directive. A unit-test pass does not replace this gate.

### Handoff and scope limit

Record changed symbols/files, named test counts, corpus hash/profile if used, and remaining risks in Completion evidence. Stop at this task's interface; do not implement unrelated future features. If a prerequisite API is missing or pinned sources conflict, identify the exact missing contract/rule and keep the affected work blocked instead of guessing. Routine local coding choices do not require asking the user.

## Sources

- [MVP](https://github.com/WebAssembly/design/tree/6e43416ebc8becd6cdf5d91d9ae7d3712111df32)
- [Pinned integrated core specification](https://github.com/WebAssembly/spec/tree/7e9fb78c63d65018cc111422f657947b3df80d7b) is normative for finished core features; historical proposal repositories provide context.
- [Scope, source precedence and exclusions](../PROPOSALS.md).

## Completion evidence

- Changes: `tools/wtime-conformance/src/corpus.rs::select`; `tools/wtime-conformance/src/report.rs::ok`; `tools/wtime-conformance/src/script.rs::typed outcomes`.
- Focused acceptance: `script::gate_acceptance_tests::t160_case_03`; `script::tests::t012_case_01`; `script::tests::t012_case_02`. All named cases executed successfully in final workspace tests (304 passed; conformance 41 passed). The locked full corpus covers all selected families and directive outcomes, not only filtered smoke cases.
- Commands and outcomes: final optimized `opcode-coverage` corpus replays passed; replay commands/source revisions/file hashes and current implementation source fingerprint map are recorded in `/workspace/wtime/docs/architecture/conformance-gate-evidence.json`. Strict ledger passed: 584 assigned instructions, 93 reserved entries, 25 non-opcode entries, 263 active formal rules; 0 pending, 0 errors. Workspace tests and strict Clippy passed.
- Assertion counts / fixture hashes: `final-core3.json`: profile `stable-core`, 62968/62968 executed, 0 failed/skipped/unsupported, 0 explicitly reported compatibility assertions; report SHA-256 `2d1b1e665cf253f58ab3ffe305e6f77d60f2776283db2d9cb7bd0509d6bb1a4c`. `final-interactions.json`: profile `all-phase3-core`, 15/15 executed, 0 failed/skipped/unsupported, 0 explicitly reported compatibility assertions; report SHA-256 `7b5dbeb1fc0a0c502eece8d9e5e975574f410ae53c4d1d735d986449c1547119`.
- Follow-up or normative ambiguity: No compatibility rewrites for stable-core. Reports and ledger bind the frozen implementation and immutable catalog; no future draft revisions are claimed.
