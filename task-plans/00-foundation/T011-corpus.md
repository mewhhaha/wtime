# T011 — Fetch immutable upstream test corpora

Status: **done**  
Area: `wtime-conformance`  
Work unit: one focused behavior/contract and its tests; split into child tasks before exceeding two focused patches.

## Prerequisites

[T001 — Freeze the specification and proposal test revisions](../00-foundation/T001-freeze-scope.md), [T003 — Create the minimal Rust workspace](../00-foundation/T003-workspace.md)

Use the predecessor's interfaces and evidence. Do not reimplement a second parser, type registry, store or execution stack.

## Deliverable

Fetch core/proposal test files by exact commit into an external or ignored cache with integrity/license metadata. Keep historical duplicates separate from normative integrated tests. Record every selected, missing and unsupported fixture.

## Acceptance

Offline reruns use the same corpus hash. A missing fetch or truncated corpus fails setup; tests do not silently track main.

Add focused tests for the changed behavior and relevant failure boundaries. Record the exact command, feature profile, corpus/source revision and executed assertion counts. Follow [the common definition of done](../EXECUTION.md#common-definition-of-done); a missing directive, zero-test run or unimplemented branch is not a pass.

## Boundaries

Follow [ARCHITECTURE.md](../ARCHITECTURE.md): no external core production dependency, no per-proposal Cargo-crate proliferation, no JS/browser implementation, and no invented semantics for draft TODOs. Coordinate changes to shared types/opcode/control contracts with other ready tasks. Runtime feature gates and implementation resource limits must remain explicit.

## Implementation instructions for Sol 6.1 Low

Read [the interface contracts](../IMPLEMENTATION_CONTRACTS.md) and the listed prerequisite outputs before coding. Use the predecessor's API; do not invent a second implementation to avoid a missing prerequisite.

### Files and ownership

Expected paths in `/workspace/wtime` (created by implementation, not by this plan):

- `tools/wtime-conformance/src/corpus.rs`
- `docs/architecture/corpus-lock.json`

Add focused tests in the relevant package's `tests/` directory, or its existing unit-test module. Name tests `t011_case_01`, `..._case_02`, etc. Reuse existing feature modules when present; coordinate edits to central dispatch/type tables.

### Ordered recipe

1. Use spec-lock entries to download/check out exact test trees outside default source compilation.
2. Hash selected files and retain upstream license/commit/path in a corpus manifest; avoid duplicate integrated/historical assertions.
3. Support offline verification of the manifest and fail missing/truncated trees.
4. Add the cases below before considering the behavior finished. Keep malformed, invalid, trapping, host-error and resource-limit outcomes distinct.
5. Wire the change into the existing entry point/dispatch and update its feature-policy or ownership evidence where relevant. Run the commands below and record exact outcomes.

### Required cases

- `t011_case_01`: Missing file fails selection
- `t011_case_02`: Offline replay uses identical bytes
- `t011_case_03`: Corpus update changes recorded checksum.

For binary fixtures, keep a readable WAT/hex source and provenance alongside the `.wasm` or inline bytes. Generate fixtures with the separate encoder/wasm-tools; do not add `wast` to core tests. Expected outputs must come from the locked spec/rule or an independent calculation, not the engine being tested.

### Verification commands

Run from `/workspace/wtime` after activating the configured Rust environment:

```sh
. /workspace/.setup/rust-env.sh
cargo test --locked -p wtime-conformance t011_ -- --nocapture
cargo check --locked -p wtime-conformance
cargo fmt --all --check
cargo clippy --locked -p wtime-conformance --all-targets -- -D warnings
```

The filtered test invocation must execute the named cases with a positive count. If multiple packages are listed, at least each package with changed behavior must have an executed relevant test; a package's zero-match summary is not evidence for that package. Fix test placement/names rather than accepting a zero-test run.

### Handoff and scope limit

Record changed symbols/files, named test counts, corpus hash/profile if used, and remaining risks in Completion evidence. Stop at this task's interface; do not implement unrelated future features. If a prerequisite API is missing or pinned sources conflict, identify the exact missing contract/rule and keep the affected work blocked instead of guessing. Routine local coding choices do not require asking the user.

## Sources

- [Frozen proposal scope](../PROPOSALS.md) and [architecture](../ARCHITECTURE.md).
- [Pinned integrated core specification](https://github.com/WebAssembly/spec/tree/7e9fb78c63d65018cc111422f657947b3df80d7b) is normative for finished core features; historical proposal repositories provide context.
- [Scope, source precedence and exclusions](../PROPOSALS.md).

## Completion evidence

- Changes: Immutable corpus fetch/verification records 382 upstream test files across eight repositories plus three local interaction fixtures with source/test licenses and SHA-256 identities.
- Commands and outcomes: Corpus integrity checks and three t011 tests passed; missing selection and hash changes fail explicitly.
- Assertion counts / fixture hashes: docs/architecture/corpus-lock.json; tools/checks/fetch_corpus.py and conformance/src/corpus.rs.
- Follow-up or normative ambiguity: No open implementation gap within this task; later integration gates remain separately tracked.
