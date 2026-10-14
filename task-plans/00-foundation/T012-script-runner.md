# T012 — Implement spec-script parsing and outcome accounting

Status: **done**  
Area: `wtime-conformance`  
Work unit: one focused behavior/contract and its tests; split into child tasks before exceeding two focused patches.

## Prerequisites

[T011 — Fetch immutable upstream test corpora](../00-foundation/T011-corpus.md), [T005 — Define structured errors, traps and byte spans](../00-foundation/T005-diagnostics.md)

Use the predecessor's interfaces and evidence. Do not reimplement a second parser, type registry, store or execution stack.

## Deliverable

Use wast to parse module/binary/quote forms and every script assertion/directive present in the corpus. Preserve NaN patterns, trap categories, exhaustion, malformed, invalid, unlinkable and uninstantiable outcomes separately. Text-tool support is independently reported.

## Acceptance

Runner fixtures intentionally fail each assertion class; zero-test and unsupported-directive runs cannot pass. Emit counts and replay commands with underlying exit status.

Add focused tests for the changed behavior and relevant failure boundaries. Record the exact command, feature profile, corpus/source revision and executed assertion counts. Follow [the common definition of done](../EXECUTION.md#common-definition-of-done); a missing directive, zero-test run or unimplemented branch is not a pass.

## Boundaries

Follow [ARCHITECTURE.md](../ARCHITECTURE.md): no external core production dependency, no per-proposal Cargo-crate proliferation, no JS/browser implementation, and no invented semantics for draft TODOs. Coordinate changes to shared types/opcode/control contracts with other ready tasks. Runtime feature gates and implementation resource limits must remain explicit.

## Implementation instructions for Sol 6.1 Low

Read [the interface contracts](../IMPLEMENTATION_CONTRACTS.md) and the listed prerequisite outputs before coding. Use the predecessor's API; do not invent a second implementation to avoid a missing prerequisite.

### Files and ownership

Expected paths in `/workspace/wtime` (created by implementation, not by this plan):

- `tools/wtime-conformance/src/cli.rs`
- `tools/wtime-conformance/src/script.rs`
- `tools/wtime-conformance/src/report.rs`

Add focused tests in the relevant package's `tests/` directory, or its existing unit-test module. Name tests `t012_case_01`, `..._case_02`, etc. Reuse existing feature modules when present; coordinate edits to central dispatch/type tables.

### Ordered recipe

1. Implement the run/encode CLI from IMPLEMENTATION_CONTRACTS.md and parse wast script directives.
2. Represent expected results without losing NaN patterns and distinguish each assertion class before wiring runtime.
3. Add a fake adapter that returns deliberate successes/failures so accounting can be tested independently.
4. Add the cases below before considering the behavior finished. Keep malformed, invalid, trapping, host-error and resource-limit outcomes distinct.
5. Wire the change into the existing entry point/dispatch and update its feature-policy or ownership evidence where relevant. Run the commands below and record exact outcomes.

### Required cases

- `t012_case_01`: Unknown directive fails
- `t012_case_02`: Zero selected assertions fails
- `t012_case_03`: assert_invalid does not accept a runtime trap.

For binary fixtures, keep a readable WAT/hex source and provenance alongside the `.wasm` or inline bytes. Generate fixtures with the separate encoder/wasm-tools; do not add `wast` to core tests. Expected outputs must come from the locked spec/rule or an independent calculation, not the engine being tested.

### Verification commands

Run from `/workspace/wtime` after activating the configured Rust environment:

```sh
. /workspace/.setup/rust-env.sh
cargo test --locked -p wtime-conformance t012_ -- --nocapture
cargo check --locked -p wtime-conformance
cargo fmt --all --check
cargo clippy --locked -p wtime-conformance --all-targets -- -D warnings
```

The filtered test invocation must execute the named cases with a positive count. If multiple packages are listed, at least each package with changed behavior must have an executed relevant test; a package's zero-match summary is not evidence for that package. Fix test placement/names rather than accepting a zero-test run.

### Handoff and scope limit

Record changed symbols/files, named test counts, corpus hash/profile if used, and remaining risks in Completion evidence. Stop at this task's interface; do not implement unrelated future features. If a prerequisite API is missing or pinned sources conflict, identify the exact missing contract/rule and keep the affected work blocked instead of guessing. Routine local coding choices do not require asking the user.

## Sources

- [MVP](https://github.com/WebAssembly/design/tree/6e43416ebc8becd6cdf5d91d9ae7d3712111df32)
- [Custom Annotation Syntax in the Text Format](https://github.com/WebAssembly/annotations/tree/20a8e4f125d5d856aac392105c35055af9ed49f7)
- [Pinned integrated core specification](https://github.com/WebAssembly/spec/tree/7e9fb78c63d65018cc111422f657947b3df80d7b) is normative for finished core features; historical proposal repositories provide context.
- [Scope, source precedence and exclusions](../PROPOSALS.md).

## Completion evidence

- Changes: wast 261 runner executes all selected assertion/directive classes, preserving categories and reporting zero/unsupported/text failures and source-bound replay.
- Commands and outcomes: Runner unit tests including all three t012 cases passed in workspace test run; no unsupported directive silently passes.
- Assertion counts / fixture hashes: tools/wtime-conformance/src/script.rs, report.rs; pending full corpus gate is T160/T161, not hidden here.
- Follow-up or normative ambiguity: No open implementation gap within this task; later integration gates remain separately tracked.
