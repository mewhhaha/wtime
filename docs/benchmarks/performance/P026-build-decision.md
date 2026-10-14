# P026 predeclared build and promotion policy

Target five nonzero vector memory workloads; protect the zero-work case and
all other 109 SIMD/scalar/float/relaxed workloads. Run the entire 114-row
selection, all declared stages, two seeded runs and 30 pairs per group.
Use the existing uncertainty, primary-family, control, cold-stage and hard
build/dependency gates without changing their thresholds.

Earlier accepted Module sharing and bounded function plans changed the build
relative to the original P001 baseline. This explanation permits reporting
that cumulative historical growth separately. It does not authorize growth
from this SIMD candidate: compare the immediate predecessor and candidate
clean build, CLI-leaf rebuild, runtime-leaf rebuild, release build, stripped
binary size and clean peak RSS; each candidate/predecessor ratio must be at
most 1.25. Reject and restore the predecessor if any ratio fails. Preserve
hard budgets, zero default external dependencies, safe Rust, source/binary
identity, fuel/cancellation and shared-memory synchronization contracts.

Record one quiet build-budget observation on each side before timing; do not
retry an unfavorable budget sample until it passes or relax this policy after
seeing candidate results. Retain failed setup or semantic drafts separately.
No Wasmtime speed comparison is inferred from capability/correctness probes.
