# T158 — Decode and inspect branch hints without changing execution

Status: **done**  
Area: `core`  
Work unit: one focused behavior/contract and its tests; split into child tasks before exceeding two focused patches.

## Prerequisites

[T023 — Preserve custom sections and optional diagnostic names](../01-binary-decoding/T023-decode-custom.md), [T044 — Lower validated functions into compact interpreter code](../03-runtime-foundations/T044-compile-ir.md)

Use the predecessor's interfaces and evidence. Do not reimplement a second parser, type registry, store or execution stack.

## Deliverable

Implement specified custom branch-hint metadata recognition and required offset/association policy. An interpreter may ignore optimization hints; retain useful inspection information without treating hints as executable instructions.

## Acceptance

Hinted/unhinted modules execute identically; malformed/unknown metadata follows pinned optional-section semantics and cannot bypass validation.

Add focused tests for the changed behavior and relevant failure boundaries. Record the exact command, feature profile, corpus/source revision and executed assertion counts. Follow [the common definition of done](../EXECUTION.md#common-definition-of-done); a missing directive, zero-test run or unimplemented branch is not a pass.

## Boundaries

Follow [ARCHITECTURE.md](../ARCHITECTURE.md): no external core production dependency, no per-proposal Cargo-crate proliferation, no JS/browser implementation, and no invented semantics for draft TODOs. Coordinate changes to shared types/opcode/control contracts with other ready tasks. Runtime feature gates and implementation resource limits must remain explicit.

## Implementation instructions for Sol 6.1 Low

Read [the interface contracts](../IMPLEMENTATION_CONTRACTS.md) and the listed prerequisite outputs before coding. Use the predecessor's API; do not invent a second implementation to avoid a missing prerequisite.

### Files and ownership

Expected paths in `/workspace/wtime` (created by implementation, not by this plan):

- `crates/wtime-decode/src/sections/custom.rs`
- `crates/wtime-cli/src/main.rs`

Add focused tests in the relevant package's `tests/` directory, or its existing unit-test module. Name tests `t158_case_01`, `..._case_02`, etc. Reuse existing feature modules when present; coordinate edits to central dispatch/type tables.

### Ordered recipe

1. Read recognized hint payloads through bounded custom-section readers.
2. Associate valid hints with instruction/function offsets under pinned optional-metadata rules.
3. Expose inspection data but never use hint contents to bypass validation or alter results.
4. Add the cases below before considering the behavior finished. Keep malformed, invalid, trapping, host-error and resource-limit outcomes distinct.
5. Wire the change into the existing entry point/dispatch and update its feature-policy or ownership evidence where relevant. Run the commands below and record exact outcomes.

### Required cases

- `t158_case_01`: Hinted/unhinted identical results
- `t158_case_02`: Malformed optional hint handling
- `t158_case_03`: Unknown hint category safely ignored as specified.

For binary fixtures, keep a readable WAT/hex source and provenance alongside the `.wasm` or inline bytes. Generate fixtures with the separate encoder/wasm-tools; do not add `wast` to core tests. Expected outputs must come from the locked spec/rule or an independent calculation, not the engine being tested.

### Verification commands

Run from `/workspace/wtime` after activating the configured Rust environment:

```sh
. /workspace/.setup/rust-env.sh
cargo test --locked -p wtime-decode -p wtime-cli t158_ -- --nocapture
cargo check --locked -p wtime-decode -p wtime-cli
cargo fmt --all --check
cargo clippy --locked -p wtime-decode -p wtime-cli --all-targets -- -D warnings
```

The filtered test invocation must execute the named cases with a positive count. If multiple packages are listed, at least each package with changed behavior must have an executed relevant test; a package's zero-match summary is not evidence for that package. Fix test placement/names rather than accepting a zero-test run.

### Handoff and scope limit

Record changed symbols/files, named test counts, corpus hash/profile if used, and remaining risks in Completion evidence. Stop at this task's interface; do not implement unrelated future features. If a prerequisite API is missing or pinned sources conflict, identify the exact missing contract/rule and keep the affected work blocked instead of guessing. Routine local coding choices do not require asking the user.

## Sources

- [Branch Hinting](https://github.com/WebAssembly/branch-hinting/tree/23ce0bb32bbb690ce08877154e3879af060fb1d4)
- [Pinned integrated core specification](https://github.com/WebAssembly/spec/tree/7e9fb78c63d65018cc111422f657947b3df80d7b) is normative for finished core features; historical proposal repositories provide context.
- [Scope, source precedence and exclusions](../PROPOSALS.md).

## Completion evidence

- Changes: Optional hints preserve execution and cannot bypass validation.
- Commands and outcomes: Decoder metadata 3 passed; CLI hints integration 1 passed.
- Evidence: `/workspace/wtime/docs/verification/branch-hints-decode.json`; `/workspace/wtime/docs/verification/branch-hints-cli.json`.
- Follow-up or normative ambiguity: No additional behavior claimed beyond the pinned core scope; final release also requires native aarch64 evidence.
