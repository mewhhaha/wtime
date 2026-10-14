# Integer register executor: rejected candidate

P061 supplies an opt-in safe integer executor for normally admitted numeric
whole functions. It executes i32/i64 operations in initialized u64 slots, uses
P130's checked edge schedules, and reconstructs typed locals, operands and
logical labels at diagnostic source boundaries. Fuel, cancellation and operand
limits use the existing original-instruction boundary; physical slot counts do
not replace logical stack limits. Unsupported functions decline before guest
work and use decoded execution. Preparation bounds both attempted work and
retained metadata, including unsuccessful candidates.

All 270 runtime cases passed, plus the default-policy case, combined
Compact/Register control case and 12 production-tooling checks. Integer tests
ran 8,448 decoded oracle comparisons over the 66 fixture exports. Source-state
checks cover division traps, exact fuel/operand limits and 126 control prefixes
including repeated loop visits, mixed multivalue results and nested if arms.
The combined-feature check also fixes P129's two remaining Compact tuple readers.

The focused gate measured 1,500 physical pairs in two independent runs, with 30
pairs per group per run and 10,000 bootstrap resamples. The targeted warm family
was 3.15% slower in run 1 and 3.09% slower in run 2. Several cold controls and
small/held-out workloads also regressed. The candidate is **rejected**; see the
[statistics](P061-statistics.json), [comparison](P061-comparison.json.gz) and
[records](P061-records.json). The measurement adapter uses the exact frozen
predecessor core with the updated protocol, preserving baseline identity.

The implementation remains behind the nondefault `register-executor` feature
as the executable interface required by the next register tasks. It is not
selected by Decoded or Portable. The default stripped CLI remains 1,000,712
bytes; the optional build is 1,057,072 bytes. Build and dependency gates passed.
Later register work must address these measured costs before promotion.
