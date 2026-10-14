# T145 — Execute throwing resumes and EH interactions

Status: **done**  
Area: `wtime-runtime`  
Work unit: one focused behavior/contract and its tests; split into child tasks before exceeding two focused patches.

## Prerequisites

[T143 — Execute resume and suspend](../11-stack-switching/T143-cont-resume-suspend.md), [T125 — Propagate exceptions across host calls and reentry](../09-exception-handling/T125-eh-host.md)

Use the predecessor's interfaces and evidence. Do not reimplement a second parser, type registry, store or execution stack.

## Deliverable

Implement resume_throw/resume_throw_ref and exception injection into saved stacks. Preserve exception roots, handler scope, unwinding, tail-call boundaries and continuation consumption.

## Acceptance

Exceptions injected into suspended coroutines are caught at the specified location; uncaught cases clean up owned stacks and retain rooted payloads.

Add focused tests for the changed behavior and relevant failure boundaries. Record the exact command, feature profile, corpus/source revision and executed assertion counts. Follow [the common definition of done](../EXECUTION.md#common-definition-of-done); a missing directive, zero-test run or unimplemented branch is not a pass.

## Boundaries

Follow [ARCHITECTURE.md](../ARCHITECTURE.md): no external core production dependency, no per-proposal Cargo-crate proliferation, no JS/browser implementation, and no invented semantics for draft TODOs. Coordinate changes to shared types/opcode/control contracts with other ready tasks. Runtime feature gates and implementation resource limits must remain explicit.

## Implementation instructions for Sol 6.1 Low

Read [the interface contracts](../IMPLEMENTATION_CONTRACTS.md) and the listed prerequisite outputs before coding. Use the predecessor's API; do not invent a second implementation to avoid a missing prerequisite.

### Files and ownership

Expected paths in `/workspace/wtime` (created by implementation, not by this plan):

- `crates/wtime-runtime/src/exec/continuation.rs`
- `crates/wtime-runtime/src/exec/exception.rs`

Add focused tests in the relevant package's `tests/` directory, or its existing unit-test module. Name tests `t145_case_01`, `..._case_02`, etc. Reuse existing feature modules when present; coordinate edits to central dispatch/type tables.

### Ordered recipe

1. Consume target continuation using the same one-shot transfer path as resume.
2. Inject new tag/payload or an existing exnref at the captured point and dispatch existing EH handlers.
3. On uncaught error release captured ownership but preserve any returned rooted exception.
4. Add the cases below before considering the behavior finished. Keep malformed, invalid, trapping, host-error and resource-limit outcomes distinct.
5. Wire the change into the existing entry point/dispatch and update its feature-policy or ownership evidence where relevant. Run the commands below and record exact outcomes.

**Required algorithm contract:** read the section named `T140–T145 — continuation ownership` in [HARD_TASK_CONTRACTS.md](../HARD_TASK_CONTRACTS.md) before implementing. It fixes the state/ownership/validation rules that this task must not redesign.

### Required cases

- `t145_case_01`: Caught inside resumed stack
- `t145_case_02`: Existing exnref identity preserved
- `t145_case_03`: Second use and uncaught cleanup.

For binary fixtures, keep a readable WAT/hex source and provenance alongside the `.wasm` or inline bytes. Generate fixtures with the separate encoder/wasm-tools; do not add `wast` to core tests. Expected outputs must come from the locked spec/rule or an independent calculation, not the engine being tested.

### Verification commands

Run from `/workspace/wtime` after activating the configured Rust environment:

```sh
. /workspace/.setup/rust-env.sh
cargo test --locked -p wtime-runtime t145_ -- --nocapture
cargo check --locked -p wtime-runtime
cargo fmt --all --check
cargo clippy --locked -p wtime-runtime --all-targets -- -D warnings
```

The filtered test invocation must execute the named cases with a positive count. If multiple packages are listed, at least each package with changed behavior must have an executed relevant test; a package's zero-match summary is not evidence for that package. Fix test placement/names rather than accepting a zero-test run.

### Handoff and scope limit

Record changed symbols/files, named test counts, corpus hash/profile if used, and remaining risks in Completion evidence. Stop at this task's interface; do not implement unrelated future features. If a prerequisite API is missing or pinned sources conflict, identify the exact missing contract/rule and keep the affected work blocked instead of guessing. Routine local coding choices do not require asking the user.

## Sources

- [Stack Switching](https://github.com/WebAssembly/stack-switching/tree/c7be390539e5fc10669c88cadc4fa4b32fba1d47)
- [Exception handling](https://github.com/WebAssembly/exception-handling/tree/af287a73d8f3bf7ea216c10592f9e350b947c4f2)
- [Pinned integrated core specification](https://github.com/WebAssembly/spec/tree/7e9fb78c63d65018cc111422f657947b3df80d7b) is normative for finished core features; historical proposal repositories provide context.
- [Scope, source precedence and exclusions](../PROPOSALS.md).

## Completion evidence

- Status: done.
- Changes and boundary proof: resume_throw/ref: t145_resume_throw_ref_uncaught_consumes_and_cleans_segment, preserved exception identity, EH integration and fuel retention tests; actual T142 post-unwind suspension.
- Validation: 301 workspace Rust tests and strict workspace all-target/all-feature Clippy pass. Six interaction integration tests pass; focused runtime resource, root and canonical tests pass.
- Corpus: source-frozen `target/conformance/final-{mvp,core3,phase3,interactions}.json`, executed counts 18480/62968/64297/15, zero failed or unsupported; exact source/corpus hashes are recorded in those reports.
- Formal evidence: `docs/architecture/types-premise-audit.json` contains96 audited rules and271 active form records.
- Scope: pinned normative semantics; descriptor field-deduplication TODO has no encoding. Shared-everything and JS/browser APIs excluded. Rust borrowed native-frame capture is statically unrepresentable; see host-suspension policy.
