# T136 — Pass Threads and combined-memory conformance

Status: **done**  
Area: `wtime-conformance`  
Work unit: one focused behavior/contract and its tests; split into child tasks before exceeding two focused patches.

## Prerequisites

[T135 — Support thread assertions and stress scheduling](../10-threads-and-atomics/T135-threads-runner.md), [T086 — Implement per-memory page counts and growth](../06-memory-proposals/T086-page-execute.md), [T083 — Complete 64-bit growth, segments and bulk operations](../06-memory-proposals/T083-memory64-bulk-grow.md), [T129 — Route normal accesses to shared memory safely](../10-threads-and-atomics/T129-shared-regular-access.md)

Use the predecessor's interfaces and evidence. Do not reimplement a second parser, type registry, store or execution stack.

## Deliverable

Run full threads assertions plus combinations with memory64/custom pages/multimemory when the pinned specifications permit them. Review synchronization invariants and optional Miri coverage of applicable code.

## Acceptance

All assigned atomics execute; regular/atomic mixed access and lost-wakeup tests pass repeatedly; excluded shared-everything threads are not accidentally enabled.

Add focused tests for the changed behavior and relevant failure boundaries. Record the exact command, feature profile, corpus/source revision and executed assertion counts. Follow [the common definition of done](../EXECUTION.md#common-definition-of-done); a missing directive, zero-test run or unimplemented branch is not a pass.

## Boundaries

Follow [ARCHITECTURE.md](../ARCHITECTURE.md): no external core production dependency, no per-proposal Cargo-crate proliferation, no JS/browser implementation, and no invented semantics for draft TODOs. Coordinate changes to shared types/opcode/control contracts with other ready tasks. Runtime feature gates and implementation resource limits must remain explicit.

## Implementation instructions for Sol 6.1 Low

Read [the interface contracts](../IMPLEMENTATION_CONTRACTS.md) and the listed prerequisite outputs before coding. Use the predecessor's API; do not invent a second implementation to avoid a missing prerequisite.

### Files and ownership

Expected paths in `/workspace/wtime` (created by implementation, not by this plan):

- `tools/wtime-conformance/tests/threads.rs`

Add focused tests in the relevant package's `tests/` directory, or its existing unit-test module. Name tests `t136_case_01`, `..._case_02`, etc. Reuse existing feature modules when present; coordinate edits to central dispatch/type tables.

### Ordered recipe

1. Run locked thread tests and full SC/wait stress cases.
2. Add supported width/page-size/multimemory combinations and ordinary/atomic shared accesses.
3. Check synchronization invariants in review and optional supported Miri lane.
4. Add the cases below before considering the behavior finished. Keep malformed, invalid, trapping, host-error and resource-limit outcomes distinct.
5. Wire the change into the existing entry point/dispatch and update its feature-policy or ownership evidence where relevant. Run the commands below and record exact outcomes.

### Required cases

- `t136_case_01`: All atomic families execute
- `t136_case_02`: No lost wakeups under stress
- `t136_case_03`: Shared-everything types remain disabled.

For binary fixtures, keep a readable WAT/hex source and provenance alongside the `.wasm` or inline bytes. Generate fixtures with the separate encoder/wasm-tools; do not add `wast` to core tests. Expected outputs must come from the locked spec/rule or an independent calculation, not the engine being tested.

### Verification commands

Run from `/workspace/wtime` after activating the configured Rust environment:

```sh
. /workspace/.setup/rust-env.sh
cargo test --locked -p wtime-conformance t136_ -- --nocapture
cargo check --locked -p wtime-conformance
cargo fmt --all --check
cargo clippy --locked -p wtime-conformance --all-targets -- -D warnings
cargo run --locked -p wtime-conformance -- run --profile all-phase3-core --suite threads --filter all --report target/conformance/T136.json
```

The filtered test invocation must execute the named cases with a positive count. If multiple packages are listed, at least each package with changed behavior must have an executed relevant test; a package's zero-match summary is not evidence for that package. Fix test placement/names rather than accepting a zero-test run.

The corpus command must report every selected assertion as executed, with no unexplained failure, required skip or unsupported directive. A unit-test pass does not replace this gate.

### Handoff and scope limit

Record changed symbols/files, named test counts, corpus hash/profile if used, and remaining risks in Completion evidence. Stop at this task's interface; do not implement unrelated future features. If a prerequisite API is missing or pinned sources conflict, identify the exact missing contract/rule and keep the affected work blocked instead of guessing. Routine local coding choices do not require asking the user.

## Sources

- [Threads](https://github.com/webassembly/threads/tree/cc535ada1aa21cfaa3cabf3ac73b89acef78a0a0)
- [Memory64](https://github.com/WebAssembly/memory64/tree/9003cd5e24e53b84cd9027ea3dd7ae57159a6db1)
- [Custom Page Sizes](https://github.com/WebAssembly/custom-page-sizes/tree/61c873a1860acad2772539164f284cc5110f1b1b)
- [Pinned integrated core specification](https://github.com/WebAssembly/spec/tree/7e9fb78c63d65018cc111422f657947b3df80d7b) is normative for finished core features; historical proposal repositories provide context.
- [Scope, source precedence and exclusions](../PROPOSALS.md).

## Completion evidence

- Changes: `crates/wtime-runtime/src/shared_memory.rs`; `crates/wtime-runtime/src/execution.rs::atomic`.
- Focused acceptance: `shared_memory::tests::t133_aliases_counted_notification_and_registration_races`; `shared_memory::tests::t130_two_memory_sc_store_buffering_and_fence_visibility`; `script::gate_acceptance_tests::t136_case_03`; `tests/interactions.rs::t163_case_03`. All named cases executed successfully in the final workspace test run (304 passed; conformance 31 unit + 3 annotations + 7 interactions = 41 passed).
- Commands and outcomes: `cargo test --locked --workspace --all-features` and workspace/all-target/all-feature strict Clippy passed. Exact corpus replay commands, source revisions/file SHA-256 values, binary fingerprint, implementation source fingerprints, and byte-identical archived reports are recorded in `/workspace/wtime/docs/architecture/conformance-gate-evidence.json`.
- Assertion counts / fixture hashes: `final-threads.json`: profile `all-phase3-core`, 405/405 executed, 0 failed/skipped/unsupported; report SHA-256 `db85762cf493e1087603f482d160385c83581a52013bdfca06a71f40b7ecca71`; compatibility assertions 0. `final-interactions.json`: profile `all-phase3-core`, 15/15 executed, 0 failed/skipped/unsupported; report SHA-256 `7b5dbeb1fc0a0c502eece8d9e5e975574f410ae53c4d1d735d986449c1547119`; compatibility assertions 0.
- Follow-up or normative ambiguity: None for this locked scope; unsupported directives and zero-test selection remain errors.
