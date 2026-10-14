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

Implementation is still in progress: the original plan has 167 done tasks and
two tasks awaiting native AArch64 evidence; the performance plan has 67 done
and 61 pending tasks. P050 is done with an inconclusive performance gate and
its inliner retained in opt-in Compact. Decoded remains the default. P128 is
integrated but awaits its verification retry; P127 is pending. This checkpoint
does not promote either task or claim full conformance.

`/tmp` drafts did not survive reconnection. The archived sources and evidence
under the existing `target/performance` and `target/verification` directories
are still present. Build outputs, caches, and the complete raw benchmark
archives remain local and are ignored by Git. Selected recovery records are
copied here with original bytes and hashes; copying a record does not make its
referenced binaries or datasets available in a fresh clone.
