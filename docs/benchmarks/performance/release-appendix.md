# Performance release appendix

This appendix records a current-source proof of the selected portable performance portfolio.
It preserves the earlier release proof as historical evidence and makes no new timing claims.

## Current policy

- Default execution: `decoded`.
- Native and AOT: disabled.
- Compact and Register: experimental opt-ins only.

## Measured evidence

The exact final build budgets, cold reviews, and retained statistics are referenced in the P110 proof:

- P110 final proof: `/workspace/wtime/docs/benchmarks/performance/P110-proof.json` (`e3468d606dd93f8ac94ee033d01412152c0f6ef0341dbf66c7edd3885f1f3e71`).


## Build and deployment cost

These are retained source-current observations on the shared cloud host. Optional-tier ratios are unpaired cost reviews; no longitudinal P001 build comparison is available.

| Configuration | Clean (s) | CLI leaf (s) | Runtime leaf (s) | Stripped CLI (bytes) | Peak build RSS (KiB) |
| --- | ---: | ---: | ---: | ---: | ---: |
| compact | 3.385 | 0.148 | 0.460 | 1091816 | 249012 |
| default | 2.750 | 0.143 | 0.407 | 1000960 | 218468 |
| register | 3.160 | 0.150 | 0.496 | 1090512 | 246828 |

Compile-only module counters below are instrumented diagnostics with zero guest invocations. The columns overlap; they are not additive or a complete heap census. An unavailable adaptive cache is reported explicitly.

| Fixture | Mode | Wasm bytes | Lowered control bytes | Function-plan bytes | Compact retained bytes | Adaptive cache state |
| --- | --- | ---: | ---: | ---: | ---: | --- |
| call_overhead | decoded | 40 | 96 | 120 | not requested | not_requested |
| call_overhead | compact | 40 | 96 | 120 | 1320 | not_requested |
| call_overhead | register-eager | 40 | 96 | 120 | not requested | not_requested |
| call_overhead | register-adaptive-3 | 40 | 96 | 120 | not requested | empty |
| prepare_large | decoded | 215036 | 1584000 | 16000 | not requested | not_requested |
| prepare_large | compact | 215036 | 1584000 | 16000 | not requested | not_requested |
| prepare_large | register-eager | 215036 | 1584000 | 16000 | not requested | not_requested |
| prepare_large | register-adaptive-3 | 215036 | 1584000 | 16000 | not requested | unavailable |
| graph_bfs | decoded | 460 | 2880 | 600 | not requested | not_requested |
| graph_bfs | compact | 460 | 2880 | 600 | 17080 | not_requested |
| graph_bfs | register-eager | 460 | 2880 | 600 | not requested | not_requested |
| graph_bfs | register-adaptive-3 | 460 | 2880 | 600 | not requested | empty |

## Fixed warm examples

Candidate/baseline ratios below 1 favor the candidate. Each row preserves its own run and 95% interval. These examples do not summarize other workloads or establish a universal tier ranking. All original products remain in the linked statistics.

| Comparison | Workload | Run | Baseline (us) | Candidate (us) | Ratio | 95% interval |
| --- | --- | ---: | ---: | ---: | ---: | --- |
| adaptive-vs-decoded | branch_calls | 1 | 3730.001 | 3827.526 | 1.024 | 0.986–1.062 |
| adaptive-vs-decoded | call_overhead | 1 | 0.206 | 0.264 | 1.256 | 1.240–1.305 |
| adaptive-vs-decoded | integer_loop | 1 | 3330.285 | 2449.047 | 0.732 | 0.721–0.747 |
| adaptive-vs-decoded | branch_calls | 2 | 3761.073 | 3907.671 | 1.051 | 1.033–1.071 |
| adaptive-vs-decoded | call_overhead | 2 | 0.224 | 0.281 | 1.279 | 1.247–1.326 |
| adaptive-vs-decoded | integer_loop | 2 | 3552.178 | 2637.510 | 0.747 | 0.732–0.781 |
| compact-vs-decoded | branch_calls | 1 | 4072.148 | 3842.577 | 1.024 | 0.960–1.076 |
| compact-vs-decoded | call_overhead | 1 | 0.210 | 0.211 | 1.020 | 1.000–1.045 |
| compact-vs-decoded | integer_loop | 1 | 3508.747 | 3424.761 | 0.904 | 0.850–0.962 |
| compact-vs-decoded | branch_calls | 2 | 3593.510 | 3683.245 | 1.039 | 1.020–1.083 |
| compact-vs-decoded | call_overhead | 2 | 0.223 | 0.254 | 1.021 | 0.995–1.038 |
| compact-vs-decoded | integer_loop | 2 | 3392.815 | 2862.798 | 0.850 | 0.824–0.879 |
| default-vs-p001 | branch_calls | 1 | 11540.008 | 3727.106 | 0.308 | 0.299–0.333 |
| default-vs-p001 | call_overhead | 1 | 0.358 | 0.208 | 0.583 | 0.565–0.592 |
| default-vs-p001 | integer_loop | 1 | 9906.026 | 3524.110 | 0.357 | 0.338–0.369 |
| default-vs-p001 | branch_calls | 2 | 11504.716 | 3583.843 | 0.311 | 0.303–0.319 |
| default-vs-p001 | call_overhead | 2 | 0.362 | 0.208 | 0.576 | 0.565–0.587 |
| default-vs-p001 | integer_loop | 2 | 10061.401 | 3350.338 | 0.345 | 0.327–0.358 |
| eager-vs-decoded | branch_calls | 1 | 3822.724 | 4072.374 | 1.054 | 1.029–1.094 |
| eager-vs-decoded | call_overhead | 1 | 0.212 | 0.262 | 1.229 | 1.196–1.268 |
| eager-vs-decoded | integer_loop | 1 | 3277.045 | 3240.852 | 0.970 | 0.949–1.004 |
| eager-vs-decoded | branch_calls | 2 | 4143.120 | 4303.961 | 1.056 | 0.968–1.131 |
| eager-vs-decoded | call_overhead | 2 | 0.201 | 0.250 | 1.255 | 1.243–1.279 |
| eager-vs-decoded | integer_loop | 2 | 3429.152 | 3453.193 | 1.004 | 0.935–1.086 |

## Cold controls and amortization

| Comparison | Workload / stage | Run | Ratio | 95% interval | Budget review |
| --- | --- | ---: | ---: | --- | --- |
| adaptive-vs-decoded | call_overhead / first_call | 1 | 1.244 | 1.220–1.292 | over_budget |
| adaptive-vs-decoded | call_overhead / load_and_call | 1 | 1.108 | 1.080–1.147 | over_budget |
| adaptive-vs-decoded | prepare_large / prepare | 1 | 0.990 | 0.980–1.010 | within_budget |
| adaptive-vs-decoded | call_overhead / first_call | 2 | 1.214 | 1.200–1.262 | over_budget |
| adaptive-vs-decoded | call_overhead / load_and_call | 2 | 1.089 | 1.070–1.120 | over_budget |
| adaptive-vs-decoded | prepare_large / prepare | 2 | 0.994 | 0.959–1.032 | within_budget |
| compact-vs-decoded | call_overhead / first_call | 1 | 1.052 | 1.007–1.087 | inconclusive |
| compact-vs-decoded | call_overhead / load_and_call | 1 | 1.649 | 1.575–1.703 | over_budget |
| compact-vs-decoded | prepare_large / prepare | 1 | 1.035 | 1.014–1.099 | within_budget |
| compact-vs-decoded | call_overhead / first_call | 2 | 1.047 | 0.983–1.108 | inconclusive |
| compact-vs-decoded | call_overhead / load_and_call | 2 | 1.664 | 1.622–1.708 | over_budget |
| compact-vs-decoded | prepare_large / prepare | 2 | 1.022 | 0.999–1.102 | within_budget |
| default-vs-p001 | call_overhead / first_call | 1 | 0.722 | 0.704–0.743 | within_budget |
| default-vs-p001 | call_overhead / load_and_call | 1 | 0.864 | 0.835–0.878 | within_budget |
| default-vs-p001 | prepare_large / prepare | 1 | 1.005 | 0.989–1.040 | within_budget |
| default-vs-p001 | call_overhead / first_call | 2 | 0.732 | 0.709–0.754 | within_budget |
| default-vs-p001 | call_overhead / load_and_call | 2 | 0.844 | 0.835–0.868 | within_budget |
| default-vs-p001 | prepare_large / prepare | 2 | 1.021 | 0.984–1.058 | within_budget |
| eager-vs-decoded | call_overhead / first_call | 1 | 1.193 | 1.147–1.238 | over_budget |
| eager-vs-decoded | call_overhead / load_and_call | 1 | 1.310 | 1.250–1.380 | over_budget |
| eager-vs-decoded | prepare_large / prepare | 1 | 1.011 | 0.970–1.037 | within_budget |
| eager-vs-decoded | call_overhead / first_call | 2 | 1.199 | 1.151–1.218 | over_budget |
| eager-vs-decoded | call_overhead / load_and_call | 2 | 1.330 | 1.282–1.365 | over_budget |
| eager-vs-decoded | prepare_large / prepare | 2 | 0.985 | 0.945–1.007 | within_budget |

All remaining cold rows, including arithmetic and mixed branch/call modules, stay in the P110 cold review. A budget overrun or uncertain interval remains visible and does not promote an optional tier.

Break-even counts below are descriptive estimates already computed by the paired analyzer from preparation, instantiation and warm medians. They are not measured deployment totals or confidence bounds.

- adaptive-vs-decoded, call_overhead, run 1: warm savings are not statistically supported.
- adaptive-vs-decoded, call_overhead, run 2: warm savings are not statistically supported.
- compact-vs-decoded, call_overhead, run 1: warm savings are not statistically supported.
- compact-vs-decoded, call_overhead, run 2: warm savings are not statistically supported.
- default-vs-p001, call_overhead, run 1: 0 calls.
- default-vs-p001, call_overhead, run 2: 0 calls.
- eager-vs-decoded, call_overhead, run 1: warm savings are not statistically supported.
- eager-vs-decoded, call_overhead, run 2: warm savings are not statistically supported.

## Wasmtime comparison scope

The historical Wasmtime 49.0.2 comparisons in `docs/benchmarks/wasmtime-comparison*.json` retain their original source identities and settings. This current portfolio did not collect a new matched Wasmtime comparison. Its wtime/predecessor ratios cannot refresh those historical ratios. GC, thread and continuation comparator limitations describe the pinned harness interface; no unsupported or unrun comparator receives a ratio.

## Verified milestones

- P108 (contract_only): completion `df907197f06d5378b52b7642877dca51d98fdb744780e9aa2df0f30f3d8aa26f`, proof `eb21c40fa2b14a7c0487ba9f2c79415709c7f44f51903184f2a1656e56b2fe56`.
- P109 (contract_only): completion `8ae78499d9b1491748d533945e19fac68778fa2091eabf69dc8fca387d8f853c`, proof `0df3ddc30b4cebcaa61db2bc603a327c8555c4a17e26812dc37e692ca2f40ab5`.
- P110 (inconclusive): completion `65814aeda7d983ada610bfd2921afecd386ceca417730baa364921eab892d71e`, proof `e3468d606dd93f8ac94ee033d01412152c0f6ef0341dbf66c7edd3885f1f3e71`.
- P111 (contract_only): completion `54ec45cec5058ec3ecfbda1e637691ac039fdd8c274d054823a3090e7ca3ed36`, proof `a8b2c453f3a2b80958471b4682b19414c4887b3c6bf9c3916612ae9e6d9e976d`.

## Current core sources

The accompanying `report.json` retains the complete current core source map and exact prerequisite proof hashes.

## Limitations

- Native AArch64 release execution for T166 and T169 remains unrun; this report does not verify the original release on that target.
- Only the decoded tier is the default. Compact and Register remain experimental opt-ins and carry no default speed claim.
- Historical release proof remains history; this report is a separate current-source appendix.
- The specialist thread portfolio used a two-CPU mask. Four requested workers were quota-clamped to two effective workers; no four-worker scaling claim is made.
