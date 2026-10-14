# T161 — Pass all active Phase 3–5 core proposal corpora

Status: **done**  
Area: `wtime-conformance`  
Work unit: one focused behavior/contract and its tests; split into child tasks before exceeding two focused patches.

## Prerequisites

[T136 — Pass Threads and combined-memory conformance](../10-threads-and-atomics/T136-threads-gate.md), [T089 — Support compact import text forms in tooling](../06-memory-proposals/T089-compact-import-text.md), [T088 — Validate and link normalized compact imports](../06-memory-proposals/T088-compact-import-integration.md), [T092 — Run the complete wide arithmetic corpus](../06-memory-proposals/T092-wide-conformance.md), [T086 — Implement per-memory page counts and growth](../06-memory-proposals/T086-page-execute.md), [T147 — Pass stack-switching conformance and lifecycle stress](../11-stack-switching/T147-cont-gate.md), [T157 — Pass custom-descriptor core conformance](../12-custom-descriptors/T157-desc-gate.md), [T160 — Pass the complete integrated Core 3.0 corpus](../14-integration-and-release/T160-core3-gate.md)

Use the predecessor's interfaces and evidence. Do not reimplement a second parser, type registry, store or execution stack.

## Deliverable

Run all active in-scope pinned proposal corpora with all_phase3_core and reconcile integrated overlaps. Compare implemented instruction/section ledger with each formal source; track unspecified draft extras without inventing behavior.

## Acceptance

All specified active core tests execute; no partial implementation is called complete. Draft-source ambiguities have explicit decisions or remain visible blockers.

Add focused tests for the changed behavior and relevant failure boundaries. Record the exact command, feature profile, corpus/source revision and executed assertion counts. Follow [the common definition of done](../EXECUTION.md#common-definition-of-done); a missing directive, zero-test run or unimplemented branch is not a pass.

## Boundaries

Follow [ARCHITECTURE.md](../ARCHITECTURE.md): no external core production dependency, no per-proposal Cargo-crate proliferation, no JS/browser implementation, and no invented semantics for draft TODOs. Coordinate changes to shared types/opcode/control contracts with other ready tasks. Runtime feature gates and implementation resource limits must remain explicit.

## Implementation instructions for Sol 6.1 Low

Read [the interface contracts](../IMPLEMENTATION_CONTRACTS.md) and the listed prerequisite outputs before coding. Use the predecessor's API; do not invent a second implementation to avoid a missing prerequisite.

### Files and ownership

Expected paths in `/workspace/wtime` (created by implementation, not by this plan):

- `tools/wtime-conformance/tests/phase3.rs`

Add focused tests in the relevant package's `tests/` directory, or its existing unit-test module. Name tests `t161_case_01`, `..._case_02`, etc. Reuse existing feature modules when present; coordinate edits to central dispatch/type tables.

### Ordered recipe

1. Run all active included proposal corpora against all-phase3-core.
2. Reconcile overlapping core cases and compare opcode/rule ledger coverage.
3. Execute compact import text+normalization together and keep undefined draft clauses visible.
4. Add the cases below before considering the behavior finished. Keep malformed, invalid, trapping, host-error and resource-limit outcomes distinct.
5. Wire the change into the existing entry point/dispatch and update its feature-policy or ownership evidence where relevant. Run the commands below and record exact outcomes.

### Required cases

- `t161_case_01`: All six active core entries covered
- `t161_case_02`: Every assigned new opcode exercised
- `t161_case_03`: No tool gap hidden as pass.

For binary fixtures, keep a readable WAT/hex source and provenance alongside the `.wasm` or inline bytes. Generate fixtures with the separate encoder/wasm-tools; do not add `wast` to core tests. Expected outputs must come from the locked spec/rule or an independent calculation, not the engine being tested.

### Verification commands

Run from `/workspace/wtime` after activating the configured Rust environment:

```sh
. /workspace/.setup/rust-env.sh
cargo test --locked -p wtime-conformance t161_ -- --nocapture
cargo check --locked -p wtime-conformance
cargo fmt --all --check
cargo clippy --locked -p wtime-conformance --all-targets -- -D warnings
cargo run --locked -p wtime-conformance -- run --profile all-phase3-core --suite phase3 --filter all --report target/conformance/T161.json
```

The filtered test invocation must execute the named cases with a positive count. If multiple packages are listed, at least each package with changed behavior must have an executed relevant test; a package's zero-match summary is not evidence for that package. Fix test placement/names rather than accepting a zero-test run.

The corpus command must report every selected assertion as executed, with no unexplained failure, required skip or unsupported directive. A unit-test pass does not replace this gate.

### Handoff and scope limit

Record changed symbols/files, named test counts, corpus hash/profile if used, and remaining risks in Completion evidence. Stop at this task's interface; do not implement unrelated future features. If a prerequisite API is missing or pinned sources conflict, identify the exact missing contract/rule and keep the affected work blocked instead of guessing. Routine local coding choices do not require asking the user.

## Sources

- [Threads](https://github.com/webassembly/threads/tree/cc535ada1aa21cfaa3cabf3ac73b89acef78a0a0)
- [Compact Import Section](https://github.com/WebAssembly/compact-import-section/tree/0624de829e2663da799e708444dec80337a3fe54)
- [Wide Arithmetic](https://github.com/WebAssembly/wide-arithmetic/tree/024d4ce1b28cee7fd006d9c5175e3b7ece52b429)
- [Custom Page Sizes](https://github.com/WebAssembly/custom-page-sizes/tree/61c873a1860acad2772539164f284cc5110f1b1b)
- [Stack Switching](https://github.com/WebAssembly/stack-switching/tree/c7be390539e5fc10669c88cadc4fa4b32fba1d47)
- [Custom Descriptors and JS Interop](https://github.com/WebAssembly/custom-descriptors/tree/4aa477a4c1ca064fe3592b3a0307509e9059d0f9)
- [Pinned integrated core specification](https://github.com/WebAssembly/spec/tree/7e9fb78c63d65018cc111422f657947b3df80d7b) is normative for finished core features; historical proposal repositories provide context.
- [Scope, source precedence and exclusions](../PROPOSALS.md).

## Completion evidence

- Changes: `docs/architecture/corpus-lock.json::phase3`; `tools/checks/fetch_corpus.py`; `tools/wtime-conformance/src/script.rs::fail-closed directive dispatch`.
- Focused acceptance: `script::tests::t012_case_01`; `script::gate_acceptance_tests::t160_case_03`; `tests/fixtures/interactions/opcode-coverage.wast`. All named cases executed successfully in final workspace tests (304 passed; conformance 41 passed). The locked full corpus covers all selected families and directive outcomes, not only filtered smoke cases.
- Commands and outcomes: final optimized `opcode-coverage` corpus replays passed; replay commands/source revisions/file hashes and current implementation source fingerprint map are recorded in `/workspace/wtime/docs/architecture/conformance-gate-evidence.json`. Strict ledger passed: 584 assigned instructions, 93 reserved entries, 25 non-opcode entries, 263 active formal rules; 0 pending, 0 errors. Workspace tests and strict Clippy passed.
- Assertion counts / fixture hashes: `final-phase3.json`: profile `all-phase3-core`, 64297/64297 executed, 0 failed/skipped/unsupported, 2 explicitly reported compatibility assertions; report SHA-256 `6a675490cd218d3ec0c5e4917d26074221da49aa59df3b6b4ecefdd313b4ac21`. `final-interactions.json`: profile `all-phase3-core`, 15/15 executed, 0 failed/skipped/unsupported, 0 explicitly reported compatibility assertions; report SHA-256 `7b5dbeb1fc0a0c502eece8d9e5e975574f410ae53c4d1d735d986449c1547119`.
- Follow-up or normative ambiguity: Historical MVP executes 60 explicitly reported current-normative compatibility checks and enables mutable globals for the locked WG1.0 catalog; this is distinct from strict CLI mvp. Phase3 executes 2 explicitly reported nonempty-tag-result current stack-switching overlay checks; stable-core retains original invalid expectations. No assertions are skipped.
