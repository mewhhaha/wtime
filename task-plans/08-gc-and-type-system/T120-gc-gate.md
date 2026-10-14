# T120 — Pass GC/type-system conformance and collection stress

Status: **done**  
Area: `wtime-conformance`  
Work unit: one focused behavior/contract and its tests; split into child tasks before exceeding two focused patches.

## Prerequisites

[T115 — Execute struct allocation, access and mutation](../08-gc-and-type-system/T115-gc-structs.md), [T117 — Execute array bulk, data and element operations](../08-gc-and-type-system/T117-gc-array-bulk.md), [T119 — Execute reference tests, casts and cast branches](../08-gc-and-type-system/T119-gc-casts.md), [T013 — Implement spectest imports and module registration](../00-foundation/T013-spectest-host.md)

Use the predecessor's interfaces and evidence. Do not reimplement a second parser, type registry, store or execution stack.

## Deliverable

Execute the full selected GC/typed-reference corpus and force collections at allocation points and host boundaries. Add adversarial type graph and root-loss regressions.

## Acceptance

No required assertion is skipped; cyclic heaps are reclaimed, roots survive forced GC, and subtype validation/engine affinity failures are diagnosed.

Add focused tests for the changed behavior and relevant failure boundaries. Record the exact command, feature profile, corpus/source revision and executed assertion counts. Follow [the common definition of done](../EXECUTION.md#common-definition-of-done); a missing directive, zero-test run or unimplemented branch is not a pass.

## Boundaries

Follow [ARCHITECTURE.md](../ARCHITECTURE.md): no external core production dependency, no per-proposal Cargo-crate proliferation, no JS/browser implementation, and no invented semantics for draft TODOs. Coordinate changes to shared types/opcode/control contracts with other ready tasks. Runtime feature gates and implementation resource limits must remain explicit.

## Implementation instructions for Sol 6.1 Low

Read [the interface contracts](../IMPLEMENTATION_CONTRACTS.md) and the listed prerequisite outputs before coding. Use the predecessor's API; do not invent a second implementation to avoid a missing prerequisite.

### Files and ownership

Expected paths in `/workspace/wtime` (created by implementation, not by this plan):

- `tools/wtime-conformance/tests/gc.rs`

Add focused tests in the relevant package's `tests/` directory, or its existing unit-test module. Name tests `t120_case_01`, `..._case_02`, etc. Reuse existing feature modules when present; coordinate edits to central dispatch/type tables.

### Ordered recipe

1. Run every selected GC/function-ref script against the real engine.
2. Enable forced-collection mode at each legal allocation/callback safepoint.
3. Add adversarial type graphs and root-only-reference regressions.
4. Add the cases below before considering the behavior finished. Keep malformed, invalid, trapping, host-error and resource-limit outcomes distinct.
5. Wire the change into the existing entry point/dispatch and update its feature-policy or ownership evidence where relevant. Run the commands below and record exact outcomes.

### Required cases

- `t120_case_01`: All required scripts executed
- `t120_case_02`: Cycle reclaimed
- `t120_case_03`: Wrong subtype does not pass after interning.

For binary fixtures, keep a readable WAT/hex source and provenance alongside the `.wasm` or inline bytes. Generate fixtures with the separate encoder/wasm-tools; do not add `wast` to core tests. Expected outputs must come from the locked spec/rule or an independent calculation, not the engine being tested.

### Verification commands

Run from `/workspace/wtime` after activating the configured Rust environment:

```sh
. /workspace/.setup/rust-env.sh
cargo test --locked -p wtime-conformance t120_ -- --nocapture
cargo check --locked -p wtime-conformance
cargo fmt --all --check
cargo clippy --locked -p wtime-conformance --all-targets -- -D warnings
cargo run --locked -p wtime-conformance -- run --profile stable-core --suite garbage_collection --filter all --report target/conformance/T120.json
```

The filtered test invocation must execute the named cases with a positive count. If multiple packages are listed, at least each package with changed behavior must have an executed relevant test; a package's zero-match summary is not evidence for that package. Fix test placement/names rather than accepting a zero-test run.

The corpus command must report every selected assertion as executed, with no unexplained failure, required skip or unsupported directive. A unit-test pass does not replace this gate.

### Handoff and scope limit

Record changed symbols/files, named test counts, corpus hash/profile if used, and remaining risks in Completion evidence. Stop at this task's interface; do not implement unrelated future features. If a prerequisite API is missing or pinned sources conflict, identify the exact missing contract/rule and keep the affected work blocked instead of guessing. Routine local coding choices do not require asking the user.

## Sources

- [Garbage collection](https://github.com/WebAssembly/gc/tree/756060f5816c7e2159f4817fbdee76cf52f9c923)
- [Typed Function References](https://github.com/WebAssembly/function-references/tree/74d2ec81d15efd3c0f2fba46a023f376101d8e46)
- [Pinned integrated core specification](https://github.com/WebAssembly/spec/tree/7e9fb78c63d65018cc111422f657947b3df80d7b) is normative for finished core features; historical proposal repositories provide context.
- [Scope, source precedence and exclusions](../PROPOSALS.md).

## Completion evidence

- Changes: `crates/wtime-runtime/src/gc.rs::collect`; `crates/wtime-runtime/src/gc_exec.rs`; `crates/wtime-runtime/src/canonical.rs::intern_group`.
- Focused acceptance: `gc::tests::t112_cycles_and_stale_references`; `canonical::tests::t108_collision_and_nonequivalent_member_preserve_identity`; `canonical::tests::t154_exact_type_excludes_proper_subtypes`. All named cases executed successfully in the final workspace test run (304 passed; conformance 31 unit + 3 annotations + 7 interactions = 41 passed).
- Commands and outcomes: `cargo test --locked --workspace --all-features` and workspace/all-target/all-feature strict Clippy passed. Exact corpus replay commands, source revisions/file SHA-256 values, binary fingerprint, implementation source fingerprints, and byte-identical archived reports are recorded in `/workspace/wtime/docs/architecture/conformance-gate-evidence.json`.
- Assertion counts / fixture hashes: `final-garbage_collection.json`: profile `all-phase3-core`, 678/678 executed, 0 failed/skipped/unsupported; report SHA-256 `bae1f6903ff9e2c46931aa0428215219094b35f7b34493f043189e43c9f2f020`; compatibility assertions 0.
- Follow-up or normative ambiguity: None for this locked scope; unsupported directives and zero-test selection remain errors.
