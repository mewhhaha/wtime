# Register execution

Decoded execution remains the default. Register execution is an explicit experimental
option, available eagerly or with a caller-selected entry threshold. The P067 timing
milestone is still in progress; no default promotion or universal threshold follows
from the diagnostic results below.

The [P145 proof](P145-proof.json) records an independent, untimed diagnostic build.
Its exact P067 selections cover 174 eager and 95 adaptive rows. Each includes one
compile-only `prepare_large` row; the other rows execute once eagerly or four times
with threshold 3, using fresh Stores that share their compiled Module.

| Strategy | Guest invocations | Optimized | Prefix exit | Mixed | Whole-function fallback |
| --- | ---: | ---: | ---: | ---: | ---: |
| Eager | 173 | 77 | 56 | 0 | 40 |
| Threshold 3 | 376 | 76 | 126 | 48 | 126 |

Mixed means a full invocation observed both Register dispatch and fallback frames
without a prefix exit. The integer control transitions from two cold invocations
to two optimized invocations. Strict scalar float and vector fallback controls are
checked separately. The producer verifies all fixture hashes, typed inputs,
feature profiles, host/table/memory setup, expected results or traps, activation
cleanup, executable identity, and the recorded Cargo build.

These counts measure execution paths and live activation capacities. They contain
no guest timings. [The retained records](P145-records.json.gz) preserve the source,
commands, observations, and verification receipts; P067 validates this proof before
publishing its separate paired timing results.
