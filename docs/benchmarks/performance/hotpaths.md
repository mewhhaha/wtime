# Hot-path profiling hypotheses

This is a ranked investigation list based on visible source operations, not a
profile result. The order reflects how directly each operation appears in the
current execution path and how easily the scalar selection can exercise it.
No percentage of CPU time or allocation volume is inferred here.

| Rank | Evidence to check | Hypothesis | Candidate family |
| --- | --- | --- | --- |
| 1 | `crates/wtime-runtime/src/execution.rs`, `Store::invoke` | The decoded dispatch path clones the active module and instruction as it advances. The measured contribution depends on clone sizes and frequency; an observed clone does not establish its CPU share. | P017 immutable module payload; P023 instruction access |
| 2 | `crates/wtime-runtime/src/execution.rs`, `load_store`; `crates/wtime-runtime/src/lib.rs`, `Mem::read` | Scalar loads may allocate a temporary byte vector before copying a fixed-width result. The scalar selection has no memory workload, so the separate `memory` selection is needed to assess it. | P025 fixed scalar reads; P026 fixed writes |
| 3 | `crates/wtime-runtime/src/execution.rs`, `Store::frame` and call opcode handling | Guest calls may clone function/signature data and allocate locals and argument storage. `call_overhead` and `branch_calls` can distinguish call-heavy behavior from a simple arithmetic loop. | P029–P032 call-frame and argument handling |
| 4 | `crates/wtime-runtime/src/execution.rs`, `keep` and branch handling | Branch result extraction may allocate through `split_off`; `branch_calls` is a first probe, while a branch-result-specific fixture would make the attribution clearer. | P024 in-place branch results |
| 5 | `crates/wtime-runtime/src/execution.rs`, host-call root handling; `crates/wtime-runtime/src/canonical.rs`, `is_subtype` | Root snapshots and subtype walks may matter for reference and host-call workloads. The initial scalar selection does not exercise enough of those paths to rank them by observed cost. | P033, P068–P069 |

The P007 tool records CPU samples only when the host's existing `perf` access
allows them. Its performance-counter build is separate from the release build
used for sampling, and both are separate from production timing binaries.
Counter events can show how often a source operation occurred; they cannot
establish CPU percentages or production latency. Sampling and counter results
must be interpreted against the exact workload, fixture hash, executable hash,
and build flags recorded in `profile.json`.

The initial profiling selection is `scalar`: `call_overhead`, `integer_loop`,
and `branch_calls`. The memory hypothesis requires the declared `memory`
selection. These hypotheses should be reordered only after symbolized sampling
or explicit counter evidence identifies a more useful next investigation.

## Executed diagnostic evidence

`target/performance/P007-profile-runtime/profile.json` passed three scalar cases
with distinct source-bound sampling and counter binaries. The host has no
`perf` executable, so hardware percentages are unavailable. The standalone
profiler links only the four internal crates, avoiding comparator compilation.

At input 10,000, `integer_loop` records 150,010 dispatches and module copies,
with 88,805,920 explicit copied metadata bytes. `branch_calls` at input 5,000
records 127,510 module copies, 5,001 frame entries, 5,001 argument allocations,
and 10,001 branch-result allocations. These counts support investigating P017
first, followed by call frames and branch results; they do not assign CPU shares.
Preparation and memory hypotheses remain separately scoped.
