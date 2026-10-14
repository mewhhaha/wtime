# Transactional per-function preparation

Register preparation now has a single-function boundary in
`execution/register/execute.rs`. The caller supplies the admitted module/body,
retained-metadata budget and remaining attempted work. The result distinguishes
a complete artifact, an ordinary decline, exhausted attempted work, and invalid
metadata or arithmetic.

The module wrapper keeps the original index allocation, ordered body traversal
and aggregate limits. Exhaustion stops further attempts; invalid metadata
refuses the module artifact. Unsupported functions and failed allocations
decline individually. Each attempt spends its conservative work charge before
eligibility checks, while retained metadata commits only after complete lowering,
operation checks and edge-move preparation. The existing 100,000-work, 1-MiB
metadata and 2-MiB activation limits remain unchanged.

This extraction adds no cache, policy change or Arc allocation. P066 can use the
same function transaction at an entry boundary without recompiling an entire
module. Cache synchronization, eviction and promotion remain separate work.

P132 compares 1,133 preparation records against a separately compiled immutable
P065 predecessor: 20 fixtures, 181 prepared functions and 15 declines, 805 source
PCs, 68 constants, 33 memory arguments and ten edge records. All selected fields
match byte for byte. Additional cases check invalid body indices, arithmetic
overflow, unsupported layouts, work exhaustion, metadata refusal and aggregate
commit behavior. Full Register/reference and combined-feature suites plus the
default policy check passed 715 test executions. No performance claim is made.
The records are indexed by
[P132-records.json](../benchmarks/performance/P132-records.json).
