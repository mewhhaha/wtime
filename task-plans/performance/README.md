# Performance climb for wtime

This is a new performance plan, dated 2026-10-11, for the existing Rust runtime in `/workspace/wtime`. It supplements the [original implementation plan](../EXECUTION.md); it does not change its specification snapshot, task statuses, or outstanding native AArch64 verification. All performance tasks start unimplemented. The intended implementation model is **Sol 6.1 Low**.

Start with [INDEX.md](INDEX.md) and use [TASKS.json](TASKS.json) as the dependency graph. Read [EXECUTION.md](EXECUTION.md), [CONTRACTS.md](CONTRACTS.md), [BACKEND_CONTRACTS.md](BACKEND_CONTRACTS.md), [MEASUREMENT.md](MEASUREMENT.md), and [ARCHITECTURE.md](ARCHITECTURE.md) before implementing a task. Individual files supply the owned paths, ordered recipe, fixtures, pre-edit baseline and verification commands, measurement selection, and decision rule. Commands referring to new tools are deliverables of explicitly linked predecessor tasks, not commands claimed to exist today.

The order is deliberate:

1. Preserve the current executable and establish reproducible before/after comparisons, profiling, and broader workloads.
2. Remove known structural costs: deep module copies in dispatch, tiny heap allocations for loads, branch-result allocation, and repeated call metadata work.
3. Introduce compact execution instructions, resolved control and call plans, then carefully bounded fusions.
4. Try register bytecode and specialize memory, roots, GC metadata, continuations, and portable SIMD.
5. Explore optional traces, proof-guarded loop acceleration, batching, offline AOT, and a small native backend. Keep experiments that earn their complexity; record negative results.

The existing benchmarks are evidence, not a profile. Against Wasmtime 49.0.2, the current interpreter prepares the small modules in about 14–23 microseconds, versus 0.71–2.73 milliseconds for Wasmtime. Warm synthetic integer, memory, and call workloads are approximately 697–739x, 3,121–5,644x, and 944–992x slower. The [benchmark report](../../wtime/docs/benchmarks/README.md) describes the different preparation products, dynamic API costs, and cloud noise. No attribution or speedup for the proposed changes has been measured.

The first milestone aims for a **10x reduction in the three compute workload times** while preserving the small cold-start advantage. The portable interpreter milestone aims for **100x over today's interpreter** on those workloads and broader representative cases. An optional optimized tier aims to approach **within 2–10x of Wasmtime** on its declared eligible workloads. These are investigation goals, not promises or conditions that justify weakening correctness. Each candidate has a smaller measurable promotion rule; an unmet milestone produces evidence and follow-up tasks.

Default production dependencies stay at zero external crates. Existing core crates continue to forbid unsafe Rust. Experiments remain disabled by default. Native code and architecture intrinsics require a separate optional boundary with an explicit safety contract, supported-target list, and evidence; they cannot silently enter the portable interpreter. The original browser/JS-only exclusions remain. Performance work does not add WASI, a component model, or newer proposal revisions.

This directory is a sibling planning artifact under `/workspace/tasks`, outside the repository's Git tracking. Preserve it when moving the checkout. Validate both plans with:

```sh
python3 /workspace/tasks/performance/check_plan.py
python3 /workspace/tasks/check_plan.py
```

Use Luna subagents for exploration, inventories, index generation, and mechanical checks, as requested. Keep semantic changes with the implementer and shared interfaces with one integrator. The optional research branches are alternatives, not a demand to ship every design simultaneously.
