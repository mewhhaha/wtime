# T057 — Add a small CLI for inspect, validate and invoke

Status: **done**  
Area: `wtime-cli`  
Work unit: one focused behavior/contract and its tests; split into child tasks before exceeding two focused patches.

## Prerequisites

[T043 — Execute start functions with correct failure semantics](../03-runtime-foundations/T043-start-function.md), [T046 — Define run outcomes and cancellation checkpoints](../03-runtime-foundations/T046-execution-results.md), [T052 — Execute numeric conversions and reinterpretations](../04-mvp-execution/T052-numeric-conversions.md), [T049 — Execute integer bit operations and shifts](../04-mvp-execution/T049-integer-bits.md), [T051 — Execute floating rounding instructions](../04-mvp-execution/T051-float-rounding.md), [T055 — Execute scalar loads and stores](../04-mvp-execution/T055-load-store.md), [T056 — Execute memory.size and memory.grow](../04-mvp-execution/T056-memory-growth.md)

Use the predecessor's interfaces and evidence. Do not reimplement a second parser, type registry, store or execution stack.

## Deliverable

Implement hand-written argument parsing and binary-file inspect/validate/invoke commands with explicit profile/limit switches. Format traps and raw values; omit WASI and ambient imports by default.

## Acceptance

An add module, import error, trapping module and invalid binary return documented stdout/stderr and exit statuses. Help does not pull CLI frameworks.

Add focused tests for the changed behavior and relevant failure boundaries. Record the exact command, feature profile, corpus/source revision and executed assertion counts. Follow [the common definition of done](../EXECUTION.md#common-definition-of-done); a missing directive, zero-test run or unimplemented branch is not a pass.

## Boundaries

Follow [ARCHITECTURE.md](../ARCHITECTURE.md): no external core production dependency, no per-proposal Cargo-crate proliferation, no JS/browser implementation, and no invented semantics for draft TODOs. Coordinate changes to shared types/opcode/control contracts with other ready tasks. Runtime feature gates and implementation resource limits must remain explicit.

## Implementation instructions for Sol 6.1 Low

Read [the interface contracts](../IMPLEMENTATION_CONTRACTS.md) and the listed prerequisite outputs before coding. Use the predecessor's API; do not invent a second implementation to avoid a missing prerequisite.

### Files and ownership

Expected paths in `/workspace/wtime` (created by implementation, not by this plan):

- `crates/wtime-cli/src/main.rs`

Add focused tests in the relevant package's `tests/` directory, or its existing unit-test module. Name tests `t057_case_01`, `..._case_02`, etc. Reuse existing feature modules when present; coordinate edits to central dispatch/type tables.

### Ordered recipe

1. Implement inspect/validate/invoke with hand-parsed args and stable exit categories.
2. Wire Engine/Store/Linker without implicit WASI/system imports.
3. Render raw numeric results, useful errors and selected feature profile.
4. Add the cases below before considering the behavior finished. Keep malformed, invalid, trapping, host-error and resource-limit outcomes distinct.
5. Wire the change into the existing entry point/dispatch and update its feature-policy or ownership evidence where relevant. Run the commands below and record exact outcomes.

### Required cases

- `t057_case_01`: Add function returns 42
- `t057_case_02`: Invalid binary nonzero
- `t057_case_03`: Missing import and trapped invocation differ.

For binary fixtures, keep a readable WAT/hex source and provenance alongside the `.wasm` or inline bytes. Generate fixtures with the separate encoder/wasm-tools; do not add `wast` to core tests. Expected outputs must come from the locked spec/rule or an independent calculation, not the engine being tested.

### Verification commands

Run from `/workspace/wtime` after activating the configured Rust environment:

```sh
. /workspace/.setup/rust-env.sh
cargo test --locked -p wtime-cli t057_ -- --nocapture
cargo check --locked -p wtime-cli
cargo fmt --all --check
cargo clippy --locked -p wtime-cli --all-targets -- -D warnings
```

The filtered test invocation must execute the named cases with a positive count. If multiple packages are listed, at least each package with changed behavior must have an executed relevant test; a package's zero-match summary is not evidence for that package. Fix test placement/names rather than accepting a zero-test run.

### Handoff and scope limit

Record changed symbols/files, named test counts, corpus hash/profile if used, and remaining risks in Completion evidence. Stop at this task's interface; do not implement unrelated future features. If a prerequisite API is missing or pinned sources conflict, identify the exact missing contract/rule and keep the affected work blocked instead of guessing. Routine local coding choices do not require asking the user.

## Sources

- [MVP](https://github.com/WebAssembly/design/tree/6e43416ebc8becd6cdf5d91d9ae7d3712111df32)
- [Pinned integrated core specification](https://github.com/WebAssembly/spec/tree/7e9fb78c63d65018cc111422f657947b3df80d7b) is normative for finished core features; historical proposal repositories provide context.
- [Scope, source precedence and exclusions](../PROPOSALS.md).

## Completion evidence

- Changes: CLI supports check/exports/hints/run, explicit profiles and fuel, checked scalar arguments and structured errors.
- Commands and outcomes: cargo test --locked -p wtime-cli: 2 passed (argument/error unit and hints/execution integration). `python3 tools/checks/cli_smoke.py`: four outcomes pass: add42, invalid Decode/nonzero, missing-import Link/nonzero, unreachable Trap/nonzero. Original WAT and hashes are retained in tools/checks/fixtures/cli and docs/verification/cli.json. Native memory64/table64 CLI examples return I32(42) and I64(1).
- Evidence: `/workspace/wtime//tmp/wtime-cli-final.log`; `/workspace/wtime/docs/verification/api-docs.json`.
- Follow-up or normative ambiguity: No additional behavior claimed beyond the pinned core scope; final release also requires native aarch64 evidence.
