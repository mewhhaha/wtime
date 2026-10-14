# T008 — Create a complete instruction and section ledger

Status: **done**  
Area: `tooling`  
Work unit: one focused behavior/contract and its tests; split into child tasks before exceeding two focused patches.

## Prerequisites

[T001 — Freeze the specification and proposal test revisions](../00-foundation/T001-freeze-scope.md), [T006 — Define portable value and module type shapes](../00-foundation/T006-type-shapes.md)

Use the predecessor's interfaces and evidence. Do not reimplement a second parser, type registry, store or execution stack.

## Deliverable

Inventory all in-scope instructions, prefixed opcodes, immediates, sections and validation rules, including non-opcode changes. Assign each to decode/validate/execute tasks. Prefer compact tables and small local macros; avoid a global code-generating macro.

## Acceptance

A checker detects a missing handler or test owner; reserved opcodes are excluded intentionally, not accidentally accepted.

Add focused tests for the changed behavior and relevant failure boundaries. Record the exact command, feature profile, corpus/source revision and executed assertion counts. Follow [the common definition of done](../EXECUTION.md#common-definition-of-done); a missing directive, zero-test run or unimplemented branch is not a pass.

## Boundaries

Follow [ARCHITECTURE.md](../ARCHITECTURE.md): no external core production dependency, no per-proposal Cargo-crate proliferation, no JS/browser implementation, and no invented semantics for draft TODOs. Coordinate changes to shared types/opcode/control contracts with other ready tasks. Runtime feature gates and implementation resource limits must remain explicit.

## Implementation instructions for Sol 6.1 Low

Read [the interface contracts](../IMPLEMENTATION_CONTRACTS.md) and the listed prerequisite outputs before coding. Use the predecessor's API; do not invent a second implementation to avoid a missing prerequisite.

### Files and ownership

Expected paths in `/workspace/wtime` (created by implementation, not by this plan):

- `docs/architecture/opcodes.json`
- `tools/checks/opcode_coverage.py`

Add stdlib-only checker tests under `tools/checks/tests/test_opcode_ledger.py`, including an intentionally failing input. Do not test a checker solely against its own happy-path output.

### Ordered recipe

1. Extract assigned opcodes/immediates and non-opcode grammar changes from the locked sources.
2. Write a machine-readable ledger mapping feature to decode/validate/execute/test owners and reserved encodings.
3. Add a checker comparing enumerated dispatch entries/coverage against the ledger without generating an entire interpreter.
4. Add the cases below before considering the behavior finished. Keep malformed, invalid, trapping, host-error and resource-limit outcomes distinct.
5. Wire the change into the existing entry point/dispatch and update its feature-policy or ownership evidence where relevant. Run the commands below and record exact outcomes.

### Required cases

- `t008_case_01`: Missing ownership row fails check
- `t008_case_02`: Reserved opcode does not acquire a handler
- `t008_case_03`: Every non-opcode proposal has a row.

For binary fixtures, keep a readable WAT/hex source and provenance alongside the `.wasm` or inline bytes. Generate fixtures with the separate encoder/wasm-tools; do not add `wast` to core tests. Expected outputs must come from the locked spec/rule or an independent calculation, not the engine being tested.

### Verification commands

Run from `/workspace/wtime` after activating the configured Rust environment:

```sh
. /workspace/.setup/rust-env.sh
python3 -m unittest discover -s tools/checks/tests -p 'test_opcode_ledger.py' -v
git diff --check
```

### Handoff and scope limit

Record changed symbols/files, named test counts, corpus hash/profile if used, and remaining risks in Completion evidence. Stop at this task's interface; do not implement unrelated future features. If a prerequisite API is missing or pinned sources conflict, identify the exact missing contract/rule and keep the affected work blocked instead of guessing. Routine local coding choices do not require asking the user.

## Sources

- [Frozen proposal scope](../PROPOSALS.md) and [architecture](../ARCHITECTURE.md).
- [Pinned integrated core specification](https://github.com/WebAssembly/spec/tree/7e9fb78c63d65018cc111422f657947b3df80d7b) is normative for finished core features; historical proposal repositories provide context.
- [Scope, source precedence and exclusions](../PROPOSALS.md).

## Completion evidence

- Changes: Repository docs/architecture/opcodes.json inventories584 assigned instructions,93 explicit reserved encodings,25 nonopcode feature surfaces and14 section forms. tools/checks/opcode_coverage.py verifies actual decoder dispatch, source-handler ownership, digest-bound executed traces, focused test execution and formal direct-premise evidence. Comment-aware normative inventory retains263 active rules; two commented historical load/store aliases are preserved separately. Source revisions and proposal assignments follow spec-lock.json.
- Commands and outcomes: python3 tools/checks/reconcile_opcode_evidence.py --reports target/conformance/final-core3.json target/conformance/final-phase3.json target/conformance/final-mvp.json target/conformance/final-interactions.json; python3 tools/checks/opcode_coverage.py --require-complete --report target/conformance/ledger-final.json: passed with0 errors and0 pending coverage owners. python3 -m unittest discover -s tools/checks/tests -p test_opcode_ledger.py -q:10 passed.
- Assertion counts / fixture hashes: final mvp18480,core362968,phase364297,interactions15 executed; each has0 failed/skipped/unsupported. Final binary SHA2562d3af21d59ef51743662a31c76bc54f063d7c91f5c1514dfa8441d718ece2d08; source maps match the frozen implementation. Immutable report snapshots in repository docs/architecture/opcode-evidence bind every assigned instruction to actual execution; focused Rust reports bind metadata and annotation tests. Formal audits merge validator170 and types96 entries minus one overlap and two inactive aliases; all263 active direct-rule premises have named source/test evidence. Types audit includes271 exact active corpus forms verified independently against their byte offsets/hashes.
- Follow-up or normative ambiguity: This is complete recorded instruction/feature/direct-rule acceptance coverage, not an exhaustive proof of every execution path or negative boundary. Audit artifacts explicitly preserve their test-boundary limits; no pending unsupported semantics are hidden by a generic source link.
