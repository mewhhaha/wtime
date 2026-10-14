# Risks and decisions

All choices here are planning decisions, not verified implementation claims. Source pins and exclusions are in [PROPOSALS.md](PROPOSALS.md).

| Risk or tension | Decision and mitigation | Owning tasks |
| --- | --- | --- |
| “All proposals” changes over time | Freeze the catalog, integrated spec and active proposal revisions. Reconcile normative text/tests before implementation; update scope explicitly when a draft changes. | T001, T011, T169 |
| Phase 3 documents are not fully settled | Implement specified core semantics. Record formal inconsistencies; do not invent a rule to make tests pass. Custom-descriptor field deduplication is presently an unspecified TODO; JS prototype behavior is excluded. | T001, T085, T148 |
| Full scope is large despite a small implementation | Use 169 focused tasks and explicit milestones. A small interpreter/dependency graph does not imply a short project. Re-estimate after working MVP and the first GC/continuation slices. | T058, T120, T147 |
| GC, exceptions and continuations require compatible ownership | Start with explicit guest stacks, typed handles, deliberate root/safepoint contracts and prompt/handler records. Avoid later conversion from native recursion. | T007, T045, T113–T114, T123–T125, T140–T146 |
| Recursive/exact type matching can be unsound or expensive | Canonicalize bounded recursion groups; keep declared subtyping distinct from equivalence; include descriptor graph/exact import constraints. Add adversarial graph tests. | T107–T109, T150–T152 |
| Threads can cause Rust UB or deadlocks | Begin with safe synchronized shared byte storage and global atomic ordering; never alias mixed-width atomics over unprotected bytes. Document lock order and atomic waiter registration. Trade throughput for correctness initially. | T128–T136 |
| Phase combinations can hide width/identity assumptions | Carry u64 memory/table indices, page size and exactness in shared shapes early. Test combinations explicitly, including GC roots across suspension and exact compact imports. | T006, T080–T086, T151–T157, T163 |
| Float and SIMD semantics differ subtly from native operators | Test NaNs, signed zero, saturating/trapping bounds, relaxed-result sets and lane ordering. Use a portable software implementation before architecture optimizations. | T050–T052, T093–T105 |
| Zero production dependencies moves correctness work into this project | Implement only required binary decoding/validation/runtime logic, but rely on maintained text tooling in a separate package. Do not write a second WAT parser merely to claim zero dependencies. | T003, T010, T012, T034 |
| Upstream WAST tooling may lag a draft | Treat missing syntax/assertion support as a real tool task. Pin a compatible tool revision or make an isolated tested tool extension; never skip required scripts silently. | T012, T089, T092, T147, T157, T159 |
| Extra crates can hurt rather than help builds | Keep four core libraries and internal Rust modules. Measure concrete codegen and fan-out before adding a crate or generic framework; exclude the conformance graph from default builds. | T002, T003, T009, T167 |
| Shared-file edits reduce practical parallelism | Land type/feature/opcode/control contracts first. Assign file/module ownership; parallelize independent implementations and source/test chores once interfaces are stable. | T002, T006, T008 and [EXECUTION.md](EXECUTION.md) |
| Host reentry and cancellation can leak roots/stacks/locks | Make host transitions explicit, forbid captures of arbitrary Rust frames, and test nested callbacks, pending results, cancellation and dropped continuations. | T040, T046, T125, T133, T146, T166 |
| Build targets are provisional | Measure the baseline before enforcing budgets. Record hardware/toolchain/cache state; any budget adjustment must be visible and justified. | T009, T167 |
| A passing subset could be mistaken for completion | Require exact corpus counts, no unexplained required skips, complete opcode/rule coverage, feature-policy negatives and release evidence. External oracle gaps are not proof of correctness. | T160–T169 |
| The plan is outside the Git repository | The requested sibling directory is intentional. Preserve or move it explicitly before checkout/environment replacement; it is not part of a wtime commit automatically. | [README.md](README.md) |

No embedded JavaScript engine, browser integration, WASI layer, native JIT, shared-everything GC or general async executor is needed for the agreed core-only goal. Adding one is a separate scope decision, not an implementation shortcut for an included proposal.
