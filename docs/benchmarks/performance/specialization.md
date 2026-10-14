# Portable specialization portfolio (P056)

P056 is pending. The complete milestone comparison and independent variant
refresh have not finished; this document makes no new speed or promotion claim.

Decoded remains the default. Compact is opt-in. The four core crates use safe
Rust and have no external production dependencies.

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

The milestone contract is [P056-verification.json](P056-verification.json).
It requires full runtime and corpus checks, exact source fuel and cancellation
checks, build and retained-size limits, eight full family comparisons, and two
original-MVP cold-control comparisons. The current registry requires 71,100
pairs per baseline, plus 2,520 cold-control pairs: 144,720 pairs in total.
Independent P047/P050 refreshes compare frozen historical cores through the
current harness, without changing their published historical outcomes.

The first measurement attempt completed the P001 scalar comparison, then
stopped because a dynamic scalar call took about 193 ns and its 100,000-call
batch could not reach the required 20 ms interval. The scalar harness now
allows up to one million calls. Both sides must still independently reach
20 ms with the same exact batch. Memory and SIMD bounds are unchanged.
The earlier full and partial datasets remain retained as historical attempts;
the changed execution inputs require fresh measurements.

The final source-current decision, uncertainty, cold/build/retained costs,
artifact identities, executed counts, and remaining profiles will be recorded
here after the canonical closure completes. P057 depends on that closure.
