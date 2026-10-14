# T010 — Enforce the dependency and unsafe-code budget

Status: **done**  
Area: `tooling`  
Work unit: one focused behavior/contract and its tests; split into child tasks before exceeding two focused patches.

## Prerequisites

[T003 — Create the minimal Rust workspace](../00-foundation/T003-workspace.md)

Use the predecessor's interfaces and evidence. Do not reimplement a second parser, type registry, store or execution stack.

## Deliverable

Check core production cargo trees, lockfile license/provenance metadata and unsafe inventory in CI. Budget zero external core production crates; use wast only in separate tooling and wasmparser only behind an oracle switch. Require justification for any later exception.

## Acceptance

Inject a fixture dependency/unsafe block into the checker's test input and verify detection without changing production manifests. Tool-only transitive trees are reported separately.

Add focused tests for the changed behavior and relevant failure boundaries. Record the exact command, feature profile, corpus/source revision and executed assertion counts. Follow [the common definition of done](../EXECUTION.md#common-definition-of-done); a missing directive, zero-test run or unimplemented branch is not a pass.

## Boundaries

Follow [ARCHITECTURE.md](../ARCHITECTURE.md): no external core production dependency, no per-proposal Cargo-crate proliferation, no JS/browser implementation, and no invented semantics for draft TODOs. Coordinate changes to shared types/opcode/control contracts with other ready tasks. Runtime feature gates and implementation resource limits must remain explicit.

## Implementation instructions for Sol 6.1 Low

Read [the interface contracts](../IMPLEMENTATION_CONTRACTS.md) and the listed prerequisite outputs before coding. Use the predecessor's API; do not invent a second implementation to avoid a missing prerequisite.

### Files and ownership

Expected paths in `/workspace/wtime` (created by implementation, not by this plan):

- `tools/checks/dependency_budget.py`
- `docs/architecture/dependencies.md`

Add stdlib-only checker tests under `tools/checks/tests/test_dependency_policy.py`, including an intentionally failing input. Do not test a checker solely against its own happy-path output.

### Ordered recipe

1. Read cargo metadata/tree and classify normal/build/dev/tool edges.
2. Fail core/CLI production checks on an external edge or prohibited proc macro; list tool transitive deps and licenses separately.
3. Scan unsafe inventory with explicit reviewed entries rather than suppressing findings.
4. Add the cases below before considering the behavior finished. Keep malformed, invalid, trapping, host-error and resource-limit outcomes distinct.
5. Wire the change into the existing entry point/dispatch and update its feature-policy or ownership evidence where relevant. Run the commands below and record exact outcomes.

### Required cases

- `t010_case_01`: Injected external core edge fails
- `t010_case_02`: Tool-only wast stays allowed
- `t010_case_03`: New unreviewed unsafe location fails.

For binary fixtures, keep a readable WAT/hex source and provenance alongside the `.wasm` or inline bytes. Generate fixtures with the separate encoder/wasm-tools; do not add `wast` to core tests. Expected outputs must come from the locked spec/rule or an independent calculation, not the engine being tested.

### Verification commands

Run from `/workspace/wtime` after activating the configured Rust environment:

```sh
. /workspace/.setup/rust-env.sh
python3 -m unittest discover -s tools/checks/tests -p 'test_dependency_policy.py' -v
git diff --check
```

### Handoff and scope limit

Record changed symbols/files, named test counts, corpus hash/profile if used, and remaining risks in Completion evidence. Stop at this task's interface; do not implement unrelated future features. If a prerequisite API is missing or pinned sources conflict, identify the exact missing contract/rule and keep the affected work blocked instead of guessing. Routine local coding choices do not require asking the user.

## Sources

- [Frozen proposal scope](../PROPOSALS.md) and [architecture](../ARCHITECTURE.md).
- [Pinned integrated core specification](https://github.com/WebAssembly/spec/tree/7e9fb78c63d65018cc111422f657947b3df80d7b) is normative for finished core features; historical proposal repositories provide context.
- [Scope, source precedence and exclusions](../PROPOSALS.md).

## Completion evidence

- Changes: tools/checks/dependency_budget.py traverses normal/build dependency closure and inventories unsafe; documents all tool dependency licenses.
- Commands and outcomes: python3 -m unittest discover -s tools/checks/tests -p test_dependency_policy.py -v:4 passed; python3 tools/checks/dependency_budget.py:0 violations.
- Assertion counts / fixture hashes: Injected external edge and unsafe block rejected; tool-only edge allowed. See docs/verification/dependency-budget.txt.
- Follow-up or normative ambiguity: No open issue within this task; feature gates remain separate tasks.
