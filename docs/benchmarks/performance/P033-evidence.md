# P033 decision evidence

The P033 candidate was rejected by the predeclared strict build gate. The leaf-build ratio exceeded its 1.25 limit; the other strict checks and all raw comparison records are reported separately. No speedup claim is made. The six changed runtime files were restored and independently checked against the frozen predecessor archive.

The first focused-test wrapper record remains unchanged with `correctness_passed: false`; its output parser missed one completed test because poison-thread stderr interleaved with the test line. A separately recorded corrected observation substantiates ten unique test starts and two nonempty harness summaries (6 passed and 4 passed). The first retention guard also remains preserved: it rejected a source identity mismatch. The corrected 168-source retention record was then used for comparison.

The corrected P033 predecessor archive includes the validated shared benchmark-source correction. The comparison used that corrected archive, with no timing rerun after archive creation. Detailed counts and hashes appear in the adjacent decision, extraction, strict-build, and rollback JSON records.

Two preserved restore attempts stopped at guard checks before runtime restoration and before any timing retest. The first guard rejected a missing focused-test-name list in the original parser summary; the independent corrected observation remained available. The second guard hit a `NameError` because a retained-candidate value was not returned to the precondition scope. Both failure records and exact helper snapshots, plus the corrected helper snapshot, are hash-bound in the rollback evidence. These were orchestration-guard failures, not runtime-test failures.
