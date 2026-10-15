# P094 native-backend boundary decision

**Decision: `no_go`. Native implementation: none. Native measurements: none.** The completed [Register milestone](../benchmarks/performance/P067-proof.json) rejected promotion against both portable comparators, and the [specialist milestone](../benchmarks/performance/P080-proof.json) retained no specialist optimization. These workload-specific portable results do not establish an accepted native workload, native benefit, or native break-even point. Keep decoded execution as the default, with Compact and Register available only as explicit experimental opt-ins. The source-bound P094 proof records the reused per-run costs and this design decision; it makes no claim that native execution was implemented or shown to be slower.

The current release keeps `native_enabled` and `aot_enabled` false, uses decoded execution by default, has zero default external dependencies, and retains the default safe-Rust boundary. No native crate, Cargo feature, unsafe scanner exception, dispatch path, executable mapping, or native descendant is enabled by this document. P095–P107 remain deferred and not selected. A future go decision must be separately reviewed and source-bound; it cannot inherit safety or performance claims from this no-go.

## Conditional scope if the decision is reopened

A future first implementation would be limited to x86_64 Linux and a numeric i32/i64 subset. It would exclude host calls, references, and a native guest stack. Unsupported operations must decline before guest execution and use the decoded engine, preserving exact source-level fuel, cancellation, source cursor, and guest state. Fallback must begin before native execution changes guest-visible state.

The proposed dependency boundary permits at most two additional internal crates. `wtime-exec-ir` has no dependencies. `wtime-native` depends only on `wtime-exec-ir`; the default package and dependency closure remains unchanged. The runtime's future native feature would be opt-in and depend on the optional native crate. Neither optional crate enters default members or the default build. The default remains safe Rust with zero external dependencies.

The native unsafe scanner exception is **not activated**. Any future exception would require a separate reviewed change limited to `crates/wtime-native/src/**`; all other default and optional Rust remains subject to the current unsafe policy. This proposal itself grants no unsafe exemption.

## Provisional finite resource policy

The following are explicit design ceilings to evaluate, not measured capacities or performance results: 16 KiB of generated code per function, 1 MiB per module, 1 MiB of executable mapping, 2 MiB of temporary compile storage, at most 65,536 attempted IR instructions, and at most 4,096 source-event expansions. Compilation is synchronous and has no worker pool. Counters use checked arithmetic. The compiler reserves bounded storage before emission; exhaustion declines compilation transactionally before guest execution and preserves the decoded fallback path. A later go proposal must justify these ceilings against the selected workloads and target, or replace them with newly frozen finite values before implementation.

Executable memory must never be simultaneously writable and executable. Code is fully emitted and validated before atomic immutable publication. Mapping lifetime references require audit and tests. Platform-specific mapping is an optional path, isolated from the default build. The x86_64 System V ABI boundary must define 16-byte stack alignment, preserve callee-saved registers, version entry/return layouts, and prohibit unwinding across native entry. Host-function ABI exposure is out of scope.

## Reopen evidence and thresholds

Reopen only when source-current P067 and P080 evidence identifies a specific user-relevant residual workload where the selected portable path misses its predeclared target. A future proposal must freeze a matched portable comparator, independent semantic oracle, source identity, and two independent uninstrumented timing runs before implementing native execution.

Every declared primary family must show at least a 5% point-estimate improvement with its 95% interval strictly below a candidate/baseline ratio of 1.00 in each run. Protected controls must have a 95% upper ratio at or below 1.05. Preparation must remain at or below 1.25 relative to its matched portable baseline, and tiny first use at or below 1.05. These are admission thresholds, not native measurements.

Optional native cost must be reported separately and stay within the existing absolute limits: clean build at most 60 seconds, touched CLI leaf at most 5 seconds, and stripped CLI at most 5 MiB. Relative clean-build, leaf-build, stripped-size, and RSS ratios above 1.25 are a no-go unless a later decision explicitly justifies the excess. Cross-compilation alone does not establish support for a target; actual execution and independent safety review are required for each claimed platform.
