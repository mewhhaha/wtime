# P056 calibration preflight

Status: implemented; awaiting fresh full P056 execution. P056 remains pending.

## Why this preflight exists

P056 close attempt 0058 passed freeze and full verification, then completed
both scalar comparisons (25,500 pairs each) and both memory comparisons
(10,860 pairs each). Its P001 SIMD comparison stopped after 6,240 pairs:

```
simd_typed_f64_neg_1_75: warm calibration did not reach 20000000ns
within 100000 calls and 8 attempts
```

This followed the earlier scalar-driver capacity failure. The four completed
historical datasets total 72,720 pairs and remain in the immutable P056 dataset
cache. They are evidence from the previous harness and are not reusable after
the calibration harness change. Partial SIMD evidence and both failed
measurement attempts remain preserved. No P056 statistics or promotion
decision has been produced.

## Implemented behavior

`tools/verification/producers/p056_calibrate.py` is the first command in
P056's `measure` phase, after freeze and complete correctness/build
verification. It prepares the existing fixtures and source-bound inputs, then
checks each declared warm group against both required sides before paired
measurement can begin. It records the exact expected, attempted, and passed
group sets; driver capacities; source, binary, and fixture identities; each
request and response; confirmation intervals; and per-group failure details.
The producer builds nothing, emits no paired samples, and computes no
statistics. A failed or missing group blocks the measurement phase.

The producer covers 490 warm groups: scalar (85), memory (32), SIMD (114),
and representative (0), each across the P001 and structural routes; four
original-MVP groups on each route; and the independent P047 (8) and P050 (12)
refresh groups. Each group confirms a shared batch on both sides with at least
20 ms measured per side, using no more than eight attempts. Fast numeric typed
workloads and scalar calls admit up to 1,000,000 calls. Memory-sensitive typed
calls remain capped at 100,000; other driver-specific capacity rules are
listed in [calibration capacities](calibration-capacities.md).

The real Rust typed endpoint is checked at 100,001 accepted calls and
1,000,001 rejected calls, with both baseline and candidate binaries. The
memory-sensitive endpoint is checked to reject 100,001 calls. The diagnostic
producer and its contract tests are
[p056_calibrate.py](../../tools/verification/producers/p056_calibrate.py) and
[test_p056_calibration_preflight.py](../../tools/verification/tests/test_p056_calibration_preflight.py).

Normal measurement retains its per-run calibration, two runs of 30 pairs per
declared group, all semantic and protected controls, and the full workload
matrix. Diagnostic confirmations are not paired samples. Analysis computes
statistics once per dataset, separately from publication. No diagnostic
latency is claimed here; record the measured elapsed time and work counts from
the fresh preflight report when P056 is executed.

The optional removal of redundant memory-driver preflights remains separate
work. P057 remains behind the P056 milestone dependency. The default runtime
remains Decoded, safe Rust, with zero external core dependencies.
