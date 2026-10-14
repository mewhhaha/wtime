# T162 — Verify complete feature enable/disable behavior

Status: **done**  
Area: `core`  
Work unit: one focused behavior/contract and its tests; split into child tasks before exceeding two focused patches.

## Prerequisites

[T161 — Pass all active Phase 3–5 core proposal corpora](../14-integration-and-release/T161-phase3-gate.md), [T004 — Define feature dependencies and resource limits](../00-foundation/T004-features.md)

Use the predecessor's interfaces and evidence. Do not reimplement a second parser, type registry, store or execution stack.

## Deliverable

Test stable versus all_phase3_core, dependency closure and each proposal's disabled-policy behavior using representative binaries, section changes and host calls. No cargo-per-proposal build matrix.

## Acceptance

A feature cannot be executed or smuggled through const expressions/custom encodings when disabled; unrelated supported features remain usable.

Add focused tests for the changed behavior and relevant failure boundaries. Record the exact command, feature profile, corpus/source revision and executed assertion counts. Follow [the common definition of done](../EXECUTION.md#common-definition-of-done); a missing directive, zero-test run or unimplemented branch is not a pass.

## Boundaries

Follow [ARCHITECTURE.md](../ARCHITECTURE.md): no external core production dependency, no per-proposal Cargo-crate proliferation, no JS/browser implementation, and no invented semantics for draft TODOs. Coordinate changes to shared types/opcode/control contracts with other ready tasks. Runtime feature gates and implementation resource limits must remain explicit.

## Implementation instructions for Sol 6.1 Low

Read [the interface contracts](../IMPLEMENTATION_CONTRACTS.md) and the listed prerequisite outputs before coding. Use the predecessor's API; do not invent a second implementation to avoid a missing prerequisite.

### Files and ownership

Expected paths in `/workspace/wtime` (created by implementation, not by this plan):

- `crates/wtime-runtime/tests/feature_profiles.rs`

Add focused tests in the relevant package's `tests/` directory, or its existing unit-test module. Name tests `t162_case_01`, `..._case_02`, etc. Reuse existing feature modules when present; coordinate edits to central dispatch/type tables.

### Ordered recipe

1. Create a compact representative binary for each feature and each syntax-only delta.
2. Test enabled profile success and disabled rejection at compile/validate boundaries.
3. Exercise initializer/import/type encodings, not only function opcodes.
4. Add the cases below before considering the behavior finished. Keep malformed, invalid, trapping, host-error and resource-limit outcomes distinct.
5. Wire the change into the existing entry point/dispatch and update its feature-policy or ownership evidence where relevant. Run the commands below and record exact outcomes.

### Required cases

- `t162_case_01`: Disabled descriptor exact import
- `t162_case_02`: Disabled custom pages/type form
- `t162_case_03`: Core profile rejects active stack switch.

For binary fixtures, keep a readable WAT/hex source and provenance alongside the `.wasm` or inline bytes. Generate fixtures with the separate encoder/wasm-tools; do not add `wast` to core tests. Expected outputs must come from the locked spec/rule or an independent calculation, not the engine being tested.

### Verification commands

Run from `/workspace/wtime` after activating the configured Rust environment:

```sh
. /workspace/.setup/rust-env.sh
cargo test --locked -p wtime-runtime t162_ -- --nocapture
cargo check --locked -p wtime-runtime
cargo fmt --all --check
cargo clippy --locked -p wtime-runtime --all-targets -- -D warnings
```

The filtered test invocation must execute the named cases with a positive count. If multiple packages are listed, at least each package with changed behavior must have an executed relevant test; a package's zero-match summary is not evidence for that package. Fix test placement/names rather than accepting a zero-test run.

### Handoff and scope limit

Record changed symbols/files, named test counts, corpus hash/profile if used, and remaining risks in Completion evidence. Stop at this task's interface; do not implement unrelated future features. If a prerequisite API is missing or pinned sources conflict, identify the exact missing contract/rule and keep the affected work blocked instead of guessing. Routine local coding choices do not require asking the user.

## Sources

- [Frozen proposal scope](../PROPOSALS.md) and [architecture](../ARCHITECTURE.md).
- [Pinned integrated core specification](https://github.com/WebAssembly/spec/tree/7e9fb78c63d65018cc111422f657947b3df80d7b) is normative for finished core features; historical proposal repositories provide context.
- [Scope, source precedence and exclusions](../PROPOSALS.md).

## Completion evidence

- Changes: Exercised every feature gate with enabled and disabled profiles, syntax-only/import/type/initializer encodings, dependency closure, stable versus all-phase3 profiles, advisory branch hints and unrelated host calls. Feature fixture WAT/wasm pairs and encoder/source SHA provenance are retained.
- Source/tests: `crates/wtime-runtime/tests/feature_profiles.rs`, `crates/wtime-runtime/tests/fixtures/feature_profiles`.
- Commands and outcomes: cargo test --locked -p wtime-runtime --test feature_profiles: 28 passed, 0 failed (replayed 2026-10-11).
- Named tests: `t162_` matches 28 executed passing tests in the full relevant suite; zero-match summaries from other test binaries are not counted.
- Shared acceptance evidence: the named cases above cover the task-specific boundaries.
- Quality checks: feature-profile fixtures add no core dependencies; runtime package checks and formatting are maintained by the runtime/release owner.
- Fixture/source digest: `c3f5affe2e615e2394b31355b42a8d5cff68bf195bd4a330ee880dd197ca60f4` for `tests/fixtures/feature_profiles/provenance.json` (individual fixture hashes and encoder provenance).
- Formal/spec evidence: pinned integrated core `7e9fb78c63d65018cc111422f657947b3df80d7b`; `docs/architecture/validation-premise-audit.json` links inspected direct predicates to focused tests and exact locked corpus source/form offsets. Active proposal overlays remain explicit.
- Follow-up: final aggregate corpus source bindings are maintained by the conformance/release owners. This task has no pending source changes; T034 is reconciled separately.
