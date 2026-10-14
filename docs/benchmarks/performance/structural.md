# Structural milestone: measurement policy

P035 measures the decoded production runtime after P034 restoration. P017 shares
immutable module metadata and remains adopted. P018–P022 supply execution
contracts. P023 was inconclusive; P024–P034 were rejected and their candidate
runtime changes were removed. The 20 production runtime source files must match
both P034-before and the restored P033 predecessor archive.

## Predeclared measurements

Capture a fresh P035-before archive after integrating and checking measurement
tooling. Compare the identical current runtime against that archive separately
for all 85 scalar, 35 memory, and nine representative registry inputs. Each
comparison contains two native runs and at least 30 pairs per stage/group, with
production instrumentation disabled. Current/current results establish cost and
repeatability; they do not establish an optimization gain.

The representative route compiles once per process, creates a fresh Store and
Instance before each invocation, and runs prepare before each guest timer. It
sums only guest intervals, calibrates the same batch on both sides to at least
20 ms, checks every scalar result, and compares the final invocation's complete
memory image with the independent Python oracle. Earlier batch invocations have
result checks; they do not have complete memory-image checks.

A separate original-mvp comparison uses exactly call_overhead, integer_loop,
and branch_calls against the verified P001 legacy adapter. Report the 10× compute
aspiration independently for each compatible case and stage with uncertainty.
No synthetic aggregate or current-only P001 row is permitted.

## Diagnostic scope

Counter runs use a separate instrumented executable and are excluded from
latency. Record all 129 selected IDs: three legacy scalar cases, memory_fill_scan,
and nine representative cases have declared counter routes. The remaining 82
scalar and 34 memory cases carry explicit unsupported attribution reasons.
These are diagnostic-route gaps, not evidence that execution is unsupported.
Probe perf and symbolization separately, retaining any actual capability gap.
Do not fill missing counters or profiles with zero.

Refresh Wasmtime representative capability and independent result/full-memory
integrity using the exact Wasm bytes. The existing batch-one Wasmtime endpoint
has no matched batched timer, so these observations support no Wasmtime speed
comparison for the new representative route.

## Integration checks

Run the decoded/reference correctness differential with real state/fuel and
proposal conformance suites, separately from latency. Complete default build
and dependency budgets with stable sources and zero external production
runtime dependencies. The milestone checker binds every native report,
analysis, journal, binary, fixture, and predecessor source identity.

## Completed measurements

P035 passed its final checker on 2026-10-12. The 129 current inputs produced
31,500 pairs across 1,050 groups. All 89 decoded/reference conformance checks
passed. The default clean build took 11.55 seconds, the CLI leaf rebuild took
0.36 seconds, and the stripped CLI occupied 1,000,600 bytes. Production retains
zero external dependencies.

The exact three P001 warm-call comparisons improved by roughly 1.7–2.6× across
both runs. None reached the 10× aspiration. The independent Wasmtime inventory
verified 71 inputs and recorded 58 explicit harness gaps; it provides capability
and result integrity evidence, without a matched Wasmtime timing comparison.

The next experiment targets compact loop dispatch. The integer-loop diagnostic
recorded 150,010 source dispatches and instruction clones for one frame entry.
Those counts do not establish CPU fractions; promotion requires new paired runs.

The complete stage costs, uncertainty intervals, recovery provenance and scope
are retained in `target/performance/P035-completion/attempt-20261012T011517300807Z/`.
The source-bound decision is `target/performance/P035/decision.json`. Failed
measurement-helper attempts remain retained separately.
