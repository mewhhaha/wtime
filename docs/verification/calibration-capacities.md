# Benchmark calibration capacities

These are current request and warm-calibration ceilings used by the benchmark
harness. A ceiling is not a target batch: each family keeps its workload and
resource bounds, exact request/response checks, and timing confirmation rules.
P056's diagnostic preflight confirms the chosen batch on both sides to at least
20 ms, with at most eight attempts, before paired work begins.

| Workload path | Maximum batch | Bound and rationale |
|---|---:|---|
| Scalar calls and fast numeric typed calls | 1,000,000 | Rust request validation, automatic numeric warm selection, and Python observation/calibration validators agree on the ceiling. The typed numeric path is exercised against the real Rust driver at 100,001 calls; 1,000,001 is rejected. |
| Typed SIMD memory and ordinary memory | 100,000 | Retains the resource-sensitive bound in the typed-memory endpoint and memory driver, with matching Python validators. |
| Representative workloads | 100,000 and 50,000,000 calls × input elements | The selected batch is the smaller of 100,000 and `floor(50,000,000 / input_elements)`. |
| GC warm/layout | 100,000 and 100,000,000 calls × input elements | The selected batch is the smaller of 100,000 and `floor(100,000,000 / max(input_elements, 1))`. |
| GC roots/lifecycle | 100,000 and 100,000,000 calls × `(slots + 8) × (input elements + 1)` | The selected batch is the smaller of 100,000 and `floor(100,000,000 / ((slots + 8) × (input_elements + 1)))`. |
| Threads | 128 groups | Keeps the multi-thread synchronization workload bounded; trial, latency-sample, and timeout limits remain in force. |
| Integration | 100,000 calls | Keeps the stateful compact-prefix workload bounded in both Rust and Python. |

Cold-stage batches remain exactly one where required. Every returned batch
must equal its requested batch, timings must be finite and positive, and
workload-specific oracles and memory proofs remain required. Diagnostic
confirmations do not add paired observations or statistics.

The P056 producer and its contract tests define the diagnostic behavior:
[p056_calibrate.py](../../tools/verification/producers/p056_calibrate.py) and
[test_p056_calibration_preflight.py](../../tools/verification/tests/test_p056_calibration_preflight.py).
