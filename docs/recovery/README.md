# Recovered workspace

On 2026-10-14 shell access to the existing cloud workspace was restored.
All 113 current core source files match the hashes saved before P128 tooling
integration. Both original task-plan validators pass. This checkpoint saves
the Rust implementation, fixtures, tools, published result documents, and task
plans to Git.

The original plans remain in `../tasks`. The byte-preserving backup in
`task-plans/` is recorded in [task-plan-snapshot.json](task-plan-snapshot.json).
On a fresh clone, restore it only when the sibling directory is absent, using
the commands in the repository README. Future plan changes should update this
tracked backup before a push.

At the initial recovery checkpoint, the original plan had 167 done tasks and
two tasks awaiting native AArch64 evidence; the performance plan had 67 done
and 61 pending tasks. P050 was done with an inconclusive performance gate and
its inliner retained in opt-in Compact. Decoded remains the default. P128 is
now complete as a workload foundation; its reviewed proof and completion
records are under `docs/benchmarks/performance/P128-*.json`. P127 is also
complete: explicit source-fuel settings passed 38 named checks and 184 real
Wasm observations. Its reviewed records are under
`docs/benchmarks/performance/P127-*.json`; they do not claim a speedup. P053
is complete as a rejected experiment: its slower accounting variant was
removed and the restored core passed all 354 runtime tests. See
`docs/benchmarks/performance/P053-result.md` for the evidence and retained patch.
The current task snapshot records subsequent progress. These records do not
claim full conformance.

`/tmp` drafts did not survive reconnection. The archived sources and evidence
under the existing `target/performance` and `target/verification` directories
are still present. Build outputs, caches, and the complete raw benchmark
archives remain local and are ignored by Git. Selected recovery records are
copied here with original bytes and hashes; copying a record does not make its
referenced binaries or datasets available in a fresh clone.
