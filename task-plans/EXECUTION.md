# How to execute this task set

This directory tracks implementation of the 169-task plan. See `TASKS.json` for current statuses and individual task completion evidence; uncompleted gates are not conformance claims. Read [ARCHITECTURE.md](ARCHITECTURE.md) and [PROPOSALS.md](PROPOSALS.md) first.

The intended implementation model is **Sol 6.1 Low** (`gpt-6.1-sol`, reasoning `low`). Read [IMPLEMENTATION_CONTRACTS.md](IMPLEMENTATION_CONTRACTS.md) before beginning; tasks for frames, roots, atomics, continuations and descriptors also reference [HARD_TASK_CONTRACTS.md](HARD_TASK_CONTRACTS.md). Each task supplies file ownership, concrete steps, fixture cases and commands. These are planned commands, not claims that a runtime exists or tests already passed.

## Work selection

- Work in the existing checkout at `/workspace/wtime`. This task list is its sibling `/workspace/tasks`; it is not automatically committed with the repository. Preserve/copy the directory intentionally if moving to a new environment.
- Use `TASKS.json` as the dependency graph. A task is ready when all its listed predecessors are done. IDs organize the directory; they are not an instruction to finish every lower number first. Never remove an edge solely to claim parallelism.
- Each file describes one behavior or contract and focused acceptance checks. Target one reviewable patch. If it expands beyond two focused patches, add child tasks, keep its acceptance gate, and update the JSON and indexes together.
- Keep status/evidence synchronized in task files and TASKS.json. Record exact commands, profile, test/corpus revision, executed counts and remaining failures. Do not mark an upstream task done from a downstream workaround.

## Delegation and integration

The user explicitly requested **Luna subagents for exploration and chores**. Use `gpt-6-luna` with a fresh, focused context for source exploration, corpus inventory, dependency checks, documentation/index generation and mechanical reviews. Do not delegate architectural or semantic decisions without reviewing the evidence. Give each subagent exact paths, expected output and an edit boundary; retain one integrator for shared contracts. Planning subagents performed read-only audits; they did not implement the runtime.

Parallelize disjoint work once its contracts land: decoding/validation numeric families, scalar execution families, memory/vector/GC/metadata work. Freeze shared enums/interfaces or reserve extension ranges before overlapping changes to the opcode/type/control files. Independent tasks may still edit the same crate: ownership is at the Rust module/file level, not simply crate level. No requirement to create a worktree.

Never paper over draft inconsistencies: report source text and isolate the affected behavior. Updating a normative source changes scope and needs an explicit reviewed task delta, not a silent rebase of all conformance expectations.

## Tool activation

Current environment activation:
```sh
. /workspace/.setup/rust-env.sh
cd /workspace/wtime
```

Rust 1.99.0, rustfmt, Clippy, wasm-tools 1.261.0 and the wasm32-unknown-unknown target were validated during environment setup. T003 created the Cargo project in `/workspace/wtime`. The current environment smoke project is not evidence of a runtime implementation.

Expected fast commands **after the corresponding Cargo packages exist**:
```sh
cargo check --locked -p wtime-runtime -p wtime-cli
cargo test --locked -p wtime-types -p wtime-decode -p wtime-validate -p wtime-runtime
cargo fmt --all --check
cargo clippy --locked -p wtime-runtime -p wtime-cli --all-targets -- -D warnings
```

The conformance runner command line is deliberately specified by T012 rather than assumed to exist. It must accept corpus revision/profile/selection and produce machine-readable assertion counts and replay artifacts. Build it explicitly; don't make core tests depend on it. Frozen dependency installation follows creation of the initial committed Cargo.lock.

## Common definition of done

1. Deliver the task's named API/behavior within the architecture and dependency budget.
2. Add meaningful positive, negative, boundary and failure-path tests specified in its file; retain exact float/NaN and nondeterminism rules.
3. Run the smallest relevant crate tests and selected normative corpus. Run broader gates when integrating a completed feature or shared contract change.
4. Cover decoding, validation, execution and feature-disabled behavior wherever applicable, either in this task or its linked counterpart tasks. No unreachable todo!(), silent stub or skipped directive may count as support.
5. Update the opcode/encoding ledger, proposal coverage and ownership docs for changed shared contracts.
6. Record evidence and failures without changing tests merely to get a pass. Keep repository defects, tool gaps, resource policy and normative ambiguities separate.

The final gate is T169, after required features, interaction checks, hardening, build budgets and documentation. A source TODO without formal semantics is recorded as unspecified, never claimed implemented.

## Scope stability

This snapshot is dated 2026-10-11 in Europe/Stockholm. It includes finished proposals and active phases 3, 4 and 5 from the pinned official catalog. Phase 3 drafts are moving targets; this plan promises a **versioned core specification snapshot**, not permanent support for all future changes. Excluded JS/browser behaviors, lower-phase proposals, WASI and a JIT do not become hidden acceptance criteria.
