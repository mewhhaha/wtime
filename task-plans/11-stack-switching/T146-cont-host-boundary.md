# T146 — Define suspension across host-call boundaries

Status: **done**  
Area: `wtime-runtime`  
Work unit: one focused behavior/contract and its tests; split into child tasks before exceeding two focused patches.

## Prerequisites

[T144 — Execute symmetric continuation switching](../11-stack-switching/T144-cont-switch.md), [T145 — Execute throwing resumes and EH interactions](../11-stack-switching/T145-cont-throw.md), [T040 — Implement explicit typed host-call boundaries](../03-runtime-foundations/T040-host-functions.md)

Use the predecessor's interfaces and evidence. Do not reimplement a second parser, type registry, store or execution stack.

## Deliverable

Define and document this Rust embedding API's suspension/reentry policy. Keep it separate from Core proposal semantics: the pinned stack-switching proposal does not specify a Rust host protocol. Prefer synchronous boundaries unless an explicit resumable host token is needed; never capture arbitrary Rust frames or retain borrowed Store state.

## Acceptance

Supported host resumptions preserve state; unsupported captures return this API's documented error without retaining stack borrows. Cancellation/drop cleans guest continuation and host tokens. These are embedding safety tests, not normative Core conformance assertions.

Add focused tests for the changed behavior and relevant failure boundaries. Record the exact command, feature profile, corpus/source revision and executed assertion counts. Follow [the common definition of done](../EXECUTION.md#common-definition-of-done); a missing directive, zero-test run or unimplemented branch is not a pass.

## Boundaries

Follow [ARCHITECTURE.md](../ARCHITECTURE.md): no external core production dependency, no per-proposal Cargo-crate proliferation, no JS/browser implementation, and no invented semantics for draft TODOs. Coordinate changes to shared types/opcode/control contracts with other ready tasks. Runtime feature gates and implementation resource limits must remain explicit.

## Implementation instructions for Sol 6.1 Low

Read [the interface contracts](../IMPLEMENTATION_CONTRACTS.md) and the listed prerequisite outputs before coding. Use the predecessor's API; do not invent a second implementation to avoid a missing prerequisite.

### Files and ownership

Expected paths in `/workspace/wtime` (created by implementation, not by this plan):

- `crates/wtime-runtime/src/host.rs`
- `docs/architecture/host-suspension.md`

Add focused tests in the relevant package's `tests/` directory, or its existing unit-test module. Name tests `t146_case_01`, `..._case_02`, etc. Reuse existing feature modules when present; coordinate edits to central dispatch/type tables.

### Ordered recipe

1. Document synchronous host boundary policy as this API's choice, not a Core rule.
2. Permit resumable host tokens only with owned state; never retain a native-frame borrow across suspension.
3. Return an explicit API error for unsupported boundary crossing and clean cancellation tokens.
4. Add the cases below before considering the behavior finished. Keep malformed, invalid, trapping, host-error and resource-limit outcomes distinct.
5. Wire the change into the existing entry point/dispatch and update its feature-policy or ownership evidence where relevant. Run the commands below and record exact outcomes.

### Required cases

- `t146_case_01`: Nested supported reentry
- `t146_case_02`: Unsupported borrowed capture rejected safely
- `t146_case_03`: Drop/cancel releases roots and host token.

For binary fixtures, keep a readable WAT/hex source and provenance alongside the `.wasm` or inline bytes. Generate fixtures with the separate encoder/wasm-tools; do not add `wast` to core tests. Expected outputs must come from the locked spec/rule or an independent calculation, not the engine being tested.

### Verification commands

Run from `/workspace/wtime` after activating the configured Rust environment:

```sh
. /workspace/.setup/rust-env.sh
cargo test --locked -p wtime-runtime t146_ -- --nocapture
cargo check --locked -p wtime-runtime
cargo fmt --all --check
cargo clippy --locked -p wtime-runtime --all-targets -- -D warnings
```

The filtered test invocation must execute the named cases with a positive count. If multiple packages are listed, at least each package with changed behavior must have an executed relevant test; a package's zero-match summary is not evidence for that package. Fix test placement/names rather than accepting a zero-test run.

### Handoff and scope limit

Record changed symbols/files, named test counts, corpus hash/profile if used, and remaining risks in Completion evidence. Stop at this task's interface; do not implement unrelated future features. If a prerequisite API is missing or pinned sources conflict, identify the exact missing contract/rule and keep the affected work blocked instead of guessing. Routine local coding choices do not require asking the user.

## Sources

- [Stack Switching](https://github.com/WebAssembly/stack-switching/tree/c7be390539e5fc10669c88cadc4fa4b32fba1d47)
- [Pinned integrated core specification](https://github.com/WebAssembly/spec/tree/7e9fb78c63d65018cc111422f657947b3df80d7b) is normative for finished core features; historical proposal repositories provide context.
- [Scope, source precedence and exclusions](../PROPOSALS.md).

## Completion evidence

- Status: done.
- Changes and boundary proof: Documented docs/architecture/host-suspension.md synchronous Rust boundary and owned guest-outcome forwarding. t040 nested reentry and owned suspension tests; borrowed native capture is excluded by Caller lifetime and static callback API, not an invented runtime error operation.
- Validation: 301 workspace Rust tests and strict workspace all-target/all-feature Clippy pass. Six interaction integration tests pass; focused runtime resource, root and canonical tests pass.
- Corpus: source-frozen `target/conformance/final-{mvp,core3,phase3,interactions}.json`, executed counts 18480/62968/64297/15, zero failed or unsupported; exact source/corpus hashes are recorded in those reports.
- Formal evidence: `docs/architecture/types-premise-audit.json` contains96 audited rules and271 active form records.
- Scope: pinned normative semantics; descriptor field-deduplication TODO has no encoding. Shared-everything and JS/browser APIs excluded. Rust borrowed native-frame capture is statically unrepresentable; see host-suspension policy.

- Final T146 boundary proof: Documented docs/architecture/host-suspension.md synchronous Rust boundary and owned guest-outcome forwarding. t040 nested reentry and owned suspension tests; borrowed native capture is excluded by Caller lifetime and static callback API, not an invented runtime error operation. Actual t146_host_suspension_forward_validates_capability_and_payload validates affinity, tag, payload, ready continuation and signature before publication; t146_guest_call_rejects_host_suspension_without_capturing_outer_frames rejects active-guest host suspension. Actual integration t146_cross_module_suspension_preserves_nominal_context_and_exactness preserves provider struct1 versus consumer struct0 contexts, rejects exact/inexact mismatch, and resumes with struct77 to return77.
- Final focused verification: 134 runtime unit tests, strict runtime Clippy and all7 interaction integration tests pass.
