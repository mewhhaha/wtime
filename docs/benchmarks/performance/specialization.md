# Portable specialization portfolio (P056)

P056 is complete with a **rejected** outcome. The combined Compact portfolio
did not establish the required family gains and exceeded protected regression
limits. No variants were enabled. Decoded remains the default; Compact remains
opt-in. The four core crates use safe Rust and zero external dependencies.

The [completion record](P056-completion.json), [decision](P056-decision.json),
and [durable records](P056-records.json) bind the executed evidence. The
[analysis archive](P056-analysis-artifacts.tar.gz) retains the exact portfolio
and independent statistics, inputs, command logs, and executed helpers.
It contains `statistics.json` and `variants-statistics.json`; their original
byte lengths and SHA-256 hashes are recorded in the durable records.

| Variant | Historical outcome | Current implementation |
| --- | --- | --- |
| P047 local arithmetic fusions | Inconclusive | Retained inside opt-in Compact |
| P050 bounded direct-leaf inlining | Inconclusive | Retained inside opt-in Compact |
| P048 compare/branch fusion | Rejected | Removed |
| P049 address/memory fusion | Rejected | Removed |
| P053 accounting specialization | Rejected | Removed |
| P054 integer peepholes | Inconclusive | Removed |
| P055 split/table dispatch | Inconclusive | Removed |
| P051 operand cache | Unimplemented optional experiment | No claim |
| P052 numeric activation storage | Unimplemented optional experiment | No claim |

Verification passed 487 named tests with zero failures or ignored tests,
145,760 corpus cases, 89 integration checks, and 68 source-fuel probes.
Three additional archive-revalidation tests passed in the analysis phase.
The admitted P001 capability probe checked 261 workloads and 1,185 stages;
those probes carry correctness evidence only.

The exhaustive calibration preflight passed all 490 warm groups in 88.344 s.
Each group confirmed the same batch on both sides with at least 20 ms measured
per side, within eight attempts. All five actual baseline/candidate binaries
accepted the numeric 100,001-call boundary, rejected 1,000,001 numeric calls,
and rejected 100,001 memory-sensitive calls. Diagnostic calibration produced
zero paired samples and zero statistics computations. The exact report is
preserved as [P056-calibration.json.gz](P056-calibration.json.gz).

The milestone analyzed 144,720 physical pairs across eight family/baseline
datasets and two original-MVP cold controls: 71,100 pairs per baseline plus
2,520 cold-control pairs. The baselines are P001 and the structural predecessor,
not Wasmtime. Each declared group retains two independent runs of 30 pairs.
All ten source-current datasets were reused from the immutable dataset cache.
The independent P047 and P050 refreshes added 1,500 and 3,360 physical pairs;
preparation aliases share those samples and their uncertainty. The refreshes
do not change historical variant outcomes.

Analysis computed statistics once for each of the ten milestone datasets and
once for each independent refresh, with one portfolio gate decision.
A misspelled post-analysis contract field initially prevented finalization.
The repaired [contract](P056-verification.json) revalidated the complete sealed
analysis, unchanged sources and dependency receipts, and final command streams.
It copied completed results without repeating timings or bootstrap calculations;
see [P056-analysis-revalidation.json](P056-analysis-revalidation.json).

Default and Compact clean builds took 2.7925 s and 3.6076 s; stripped artifacts
were 1,000,712 and 1,091,296 bytes. Both build and dependency budgets passed.
The 57-row retained-products check passed; aggregate requested Engine/Module
graph storage was 10,571,652 bytes for Decoded and 10,810,386 for Compact.
These allocator figures do not attribute physical code pages.

Earlier failed and partial measurement attempts remain preserved. Original
raw timing datasets remain in the cloud artifact store, with hashes retained
in the analysis. Native AArch64 execution evidence remains unresolved.
P057 can now proceed after this completed milestone.
