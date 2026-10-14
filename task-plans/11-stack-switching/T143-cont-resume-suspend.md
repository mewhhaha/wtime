# T143 — Execute resume and suspend

Status: **done**  
Area: `wtime-runtime`  
Work unit: one focused behavior/contract and its tests; split into child tasks before exceeding two focused patches.

## Prerequisites

[T141 — Execute continuation creation and partial binding](../11-stack-switching/T141-cont-new-bind.md), [T142 — Implement prompt/handler delimiter stacks](../11-stack-switching/T142-cont-prompts.md), [T046 — Define run outcomes and cancellation checkpoints](../03-runtime-foundations/T046-execution-results.md)

Use the predecessor's interfaces and evidence. Do not reimplement a second parser, type registry, store or execution stack.

## Deliverable

Splice/resume owned stacks and capture the exact delimited suspended suffix, payload and continuation. Restore operand/result layout and budget state without native-stack suspension.

## Acceptance

Generator examples yield/resume correct values; nested resumptions, unhandled suspension and one-shot use behave as specified with forced GC.

Add focused tests for the changed behavior and relevant failure boundaries. Record the exact command, feature profile, corpus/source revision and executed assertion counts. Follow [the common definition of done](../EXECUTION.md#common-definition-of-done); a missing directive, zero-test run or unimplemented branch is not a pass.

## Boundaries

Follow [ARCHITECTURE.md](../ARCHITECTURE.md): no external core production dependency, no per-proposal Cargo-crate proliferation, no JS/browser implementation, and no invented semantics for draft TODOs. Coordinate changes to shared types/opcode/control contracts with other ready tasks. Runtime feature gates and implementation resource limits must remain explicit.

## Implementation instructions for Sol 6.1 Low

Read [the interface contracts](../IMPLEMENTATION_CONTRACTS.md) and the listed prerequisite outputs before coding. Use the predecessor's API; do not invent a second implementation to avoid a missing prerequisite.

### Files and ownership

Expected paths in `/workspace/wtime` (created by implementation, not by this plan):

- `crates/wtime-runtime/src/exec/continuation.rs`
- `crates/wtime-runtime/src/exec/prompt.rs`

Add focused tests in the relevant package's `tests/` directory, or its existing unit-test module. Name tests `t143_case_01`, `..._case_02`, etc. Reuse existing feature modules when present; coordinate edits to central dispatch/type tables.

### Ordered recipe

1. Before resume, consume the Ready record and validate arguments, then install its owned suffix under supplied prompts.
2. On suspend find nearest matching suspend prompt and split only the delimited suffix; publish a fresh one-shot continuation plus payload.
3. Move operand/local roots and fuel/cancellation ownership atomically with the suffix; clean temporary owners on trap/failure.
4. Add the cases below before considering the behavior finished. Keep malformed, invalid, trapping, host-error and resource-limit outcomes distinct.
5. Wire the change into the existing entry point/dispatch and update its feature-policy or ownership evidence where relevant. Run the commands below and record exact outcomes.

**Required algorithm contract:** read the section named `T140–T145 — continuation ownership` in [HARD_TASK_CONTRACTS.md](../HARD_TASK_CONTRACTS.md) before implementing. It fixes the state/ownership/validation rules that this task must not redesign.

### Required cases

- `t143_case_01`: Generator yield/resume values
- `t143_case_02`: Nested same-tag prompts
- `t143_case_03`: Normal return exits resumed delimiter
- `t143_case_04`: Forced GC in detached stack
- `t143_case_05`: Second use and unmatched suspension produce correct outcome.

For binary fixtures, keep a readable WAT/hex source and provenance alongside the `.wasm` or inline bytes. Generate fixtures with the separate encoder/wasm-tools; do not add `wast` to core tests. Expected outputs must come from the locked spec/rule or an independent calculation, not the engine being tested.

### Verification commands

Run from `/workspace/wtime` after activating the configured Rust environment:

```sh
. /workspace/.setup/rust-env.sh
cargo test --locked -p wtime-runtime t143_ -- --nocapture
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
- Changes and boundary proof: Suspend payload/handler transfer: generator/bind, handler-kind skip, unhandled resumable suspension and official stack-switching corpus; guest segment roots retained.
- Validation: 301 workspace Rust tests and strict workspace all-target/all-feature Clippy pass. Six interaction integration tests pass; focused runtime resource, root and canonical tests pass.
- Corpus: source-frozen `target/conformance/final-{mvp,core3,phase3,interactions}.json`, executed counts 18480/62968/64297/15, zero failed or unsupported; exact source/corpus hashes are recorded in those reports.
- Formal evidence: `docs/architecture/types-premise-audit.json` contains96 audited rules and271 active form records.
- Scope: pinned normative semantics; descriptor field-deduplication TODO has no encoding. Shared-everything and JS/browser APIs excluded. Rust borrowed native-frame capture is statically unrepresentable; see host-suspension policy.
