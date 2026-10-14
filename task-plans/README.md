# wtime runtime implementation plan

**169 focused tasks** for a small Rust Core WebAssembly interpreter implementing the pinned finished + Phase 3–5 core proposals. Implementation is in progress in `/workspace/wtime`; task status and completion evidence distinguish implemented code from verified release gates.

- [Architecture and crate boundaries](ARCHITECTURE.md)
- [Proposal coverage, source pins and explicit exclusions](PROPOSALS.md)
- [Milestones and parallel work](ROADMAP.md)
- [Complete task index](INDEX.md)
- [Execution rules, Luna delegation and definition of done](EXECUTION.md)
- [Implementation interfaces and conventions](IMPLEMENTATION_CONTRACTS.md)
- [Detailed contracts for frames, roots, atomics and continuations](HARD_TASK_CONTRACTS.md)
- [Risks and decisions](RISKS.md)
- [Machine-readable dependency graph](TASKS.json) and [source manifest](SOURCES.json)

Check the plan's files, links, coverage and dependency graph with `python3 /workspace/tasks/check_plan.py`.

The design uses four small libraries, a thin CLI and an isolated conformance tool. Core production dependency target: **zero external crates**. Use an explicit-stack interpreter, safe generational references, a nonmoving tracing GC and a conservative synchronized shared-memory backend. Avoid JIT/LLVM and per-proposal crates.

Every task includes target files, prerequisite interfaces, an ordered recipe, named acceptance cases and exact verification commands for **Sol 6.1 Low**. Luna remains the choice for exploration and chores. The harder shared algorithms have additional ownership and state-transition contracts.

The task directory is **`/workspace/tasks`**, the requested `../tasks` from **`/workspace/wtime`**. It lives outside the Git checkout and must be preserved separately or moved deliberately when implementation begins.

## Start here

[T001 — Freeze the specification and proposal test revisions](00-foundation/T001-freeze-scope.md) is the initial ready task. Work through prerequisites, not numeric order; disjoint tasks can proceed once their shared contracts land. Use Luna subagents for exploration and chores as requested. Implementation files belong in the repository, not this planning directory.

## Task groups

| Group | Count | Purpose |
| --- | ---: | --- |
| [00-foundation](INDEX.md#00-foundation) | 14 | Scope, contracts, crate setup, dependencies and test infrastructure |
| [01-binary-decoding](INDEX.md#01-binary-decoding) | 10 | Bounded binary parsing and immutable module representation |
| [02-validation](INDEX.md#02-validation) | 10 | Type, stack, control-flow and module validation |
| [03-runtime-foundations](INDEX.md#03-runtime-foundations) | 12 | Engine, Store, linking, state and explicit execution stacks |
| [04-mvp-execution](INDEX.md#04-mvp-execution) | 12 | Scalar/control/call semantics, CLI and MVP conformance |
| [05-scalar-and-reference-proposals](INDEX.md#05-scalar-and-reference-proposals) | 18 | Scalar extensions, references, bulk operations, typed calls and tail calls |
| [06-memory-proposals](INDEX.md#06-memory-proposals) | 16 | Multiple/64-bit memories and tables, custom pages, compact imports and wide arithmetic |
| [07-vector-proposals](INDEX.md#07-vector-proposals) | 13 | Portable fixed-width and relaxed SIMD |
| [08-gc-and-type-system](INDEX.md#08-gc-and-type-system) | 15 | Recursive types, canonical identities, tracing GC and reference operations |
| [09-exception-handling](INDEX.md#09-exception-handling) | 6 | Exception types, objects, handlers and host boundaries |
| [10-threads-and-atomics](INDEX.md#10-threads-and-atomics) | 10 | Shared memory, sequentially consistent atomics, wait/notify and host workers |
| [11-stack-switching](INDEX.md#11-stack-switching) | 11 | Typed one-shot continuations, prompts and suspension/resumption |
| [12-custom-descriptors](INDEX.md#12-custom-descriptors) | 10 | Exact types/imports, descriptor graphs, allocation and casts |
| [13-metadata-and-text-tooling](INDEX.md#13-metadata-and-text-tooling) | 2 | Branch hints and annotation grammar |
| [14-integration-and-release](INDEX.md#14-integration-and-release) | 10 | Full conformance, interactions, fuzzing, hardening and build budgets |

Completion requires T169 and recorded evidence, not merely implemented opcodes. Browser/JS APIs, WASI, a JIT and lower-phase proposals are explicitly outside this goal.
