# T124 — Execute throws, try_table and exception dispatch

Status: **done**  
Area: `wtime-runtime`  
Work unit: one focused behavior/contract and its tests; split into child tasks before exceeding two focused patches.

## Prerequisites

[T123 — Represent rooted exceptions and payload identities](../09-exception-handling/T123-eh-objects.md), [T053 — Execute structured control and branches](../04-mvp-execution/T053-control-execution.md), [T054 — Execute direct, indirect and host calls](../04-mvp-execution/T054-call-execution.md), [T046 — Define run outcomes and cancellation checkpoints](../03-runtime-foundations/T046-execution-results.md)

Use the predecessor's interfaces and evidence. Do not reimplement a second parser, type registry, store or execution stack.

## Deliverable

Use explicit handler records to unwind guest frames, restore stack heights and transfer matching payloads to labels. Execute all current catches and rethrows, preserving tail-call behavior.

## Acceptance

Nested/cross-function catches, catch_all, uncaught exceptions and traps remain distinct; no native stack unwinding implements guest control.

Add focused tests for the changed behavior and relevant failure boundaries. Record the exact command, feature profile, corpus/source revision and executed assertion counts. Follow [the common definition of done](../EXECUTION.md#common-definition-of-done); a missing directive, zero-test run or unimplemented branch is not a pass.

## Boundaries

Follow [ARCHITECTURE.md](../ARCHITECTURE.md): no external core production dependency, no per-proposal Cargo-crate proliferation, no JS/browser implementation, and no invented semantics for draft TODOs. Coordinate changes to shared types/opcode/control contracts with other ready tasks. Runtime feature gates and implementation resource limits must remain explicit.

## Implementation instructions for Sol 6.1 Low

Read [the interface contracts](../IMPLEMENTATION_CONTRACTS.md) and the listed prerequisite outputs before coding. Use the predecessor's API; do not invent a second implementation to avoid a missing prerequisite.

### Files and ownership

Expected paths in `/workspace/wtime` (created by implementation, not by this plan):

- `crates/wtime-runtime/src/exec/exception.rs`

Add focused tests in the relevant package's `tests/` directory, or its existing unit-test module. Name tests `t124_case_01`, `..._case_02`, etc. Reuse existing feature modules when present; coordinate edits to central dispatch/type tables.

### Ordered recipe

1. Push explicit handler records with frame/operand boundaries at try_table.
2. On throw search matching tag/all handler, pop guest frames and restore exact stack height.
3. Deliver payload or exnref according to catch form; uncaught outcome stays an exception.
4. Add the cases below before considering the behavior finished. Keep malformed, invalid, trapping, host-error and resource-limit outcomes distinct.
5. Wire the change into the existing entry point/dispatch and update its feature-policy or ownership evidence where relevant. Run the commands below and record exact outcomes.

### Required cases

- `t124_case_01`: Cross-function catch
- `t124_case_02`: Catch_all_ref shape
- `t124_case_03`: Trap bypasses guest exception catch.

For binary fixtures, keep a readable WAT/hex source and provenance alongside the `.wasm` or inline bytes. Generate fixtures with the separate encoder/wasm-tools; do not add `wast` to core tests. Expected outputs must come from the locked spec/rule or an independent calculation, not the engine being tested.

### Verification commands

Run from `/workspace/wtime` after activating the configured Rust environment:

```sh
. /workspace/.setup/rust-env.sh
cargo test --locked -p wtime-runtime t124_ -- --nocapture
cargo check --locked -p wtime-runtime
cargo fmt --all --check
cargo clippy --locked -p wtime-runtime --all-targets -- -D warnings
```

The filtered test invocation must execute the named cases with a positive count. If multiple packages are listed, at least each package with changed behavior must have an executed relevant test; a package's zero-match summary is not evidence for that package. Fix test placement/names rather than accepting a zero-test run.

### Handoff and scope limit

Record changed symbols/files, named test counts, corpus hash/profile if used, and remaining risks in Completion evidence. Stop at this task's interface; do not implement unrelated future features. If a prerequisite API is missing or pinned sources conflict, identify the exact missing contract/rule and keep the affected work blocked instead of guessing. Routine local coding choices do not require asking the user.

## Sources

- [Exception handling](https://github.com/WebAssembly/exception-handling/tree/af287a73d8f3bf7ea216c10592f9e350b947c4f2)
- [Pinned integrated core specification](https://github.com/WebAssembly/spec/tree/7e9fb78c63d65018cc111422f657947b3df80d7b) is normative for finished core features; historical proposal repositories provide context.
- [Scope, source precedence and exclusions](../PROPOSALS.md).

## Completion evidence

- Changes: Explicit exception unwinding and tracing/reclamation across GC payload cycles. Actual implementation paths: `crates/wtime-runtime/src/lib.rs`, `execution.rs`, `linker.rs`, `numeric.rs`, and the focused arena/shared-memory/exception/wide helpers; all production runtime code forbids unsafe Rust.
- Commands and outcomes: `. /workspace/.setup/rust-env.sh; cargo test --locked -p wtime-runtime --all-features` passed 134 executed tests (89 unit + 45 integration), captured in `/tmp/runtime-final-acceptance.log`. `cargo clippy -p wtime-runtime --all-targets --all-features -- -D warnings` passed. Focused executed evidence for this boundary (4 shared tests): exceptions::tests::t124_host_published_exception_preserves_payload, exceptions::tests::t124_unreachable_exception_payload_cycles_reclaimed, exceptions::tests::t124_rooted_gc_reaches_exception_payload, t124_uncaught_identity_payload
- Assertion counts / fixture hashes: final-mvp.json (mvp): 18480 executed, 0 failed, 0 unsupported, report SHA256 62a48c4ed416a42f2a965a2be4b1584f271685a856c2bfd53bb21cd3970280a8; final-core3.json (stable-core): 62968 executed, 0 failed, 0 unsupported, report SHA256 2d1b1e665cf253f58ab3ffe305e6f77d60f2776283db2d9cb7bd0509d6bb1a4c; final-phase3.json (all-phase3-core): 64297 executed, 0 failed, 0 unsupported, report SHA256 6a675490cd218d3ec0c5e4917d26074221da49aa59df3b6b4ecefdd313b4ac21; final-interactions.json (all-phase3-core): 15 executed, 0 failed, 0 unsupported, report SHA256 7b5dbeb1fc0a0c502eece8d9e5e975574f410ae53c4d1d735d986449c1547119. Normative core revision `7e9fb78c63d65018cc111422f657947b3df80d7b`; reports retain complete locked source hashes and executable/source fingerprints.
- Follow-up or normative ambiguity: No known unfinished semantic branch within this task's implemented scope. Evidence is shared across related boundaries; positive test totals are not a claim that every prescribed test-name label exists. Host-created concrete reference globals/tables are explicitly rejected without canonical context; module-bound function/tag factories provide typed import contexts.
