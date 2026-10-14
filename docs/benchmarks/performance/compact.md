# Compact executor performance result

P046 completed with a **rejected** default-promotion decision. The Compact tier passed full conformance, the declared build budgets, and its dependency check, and it improved the selected scalar, memory, and SIMD portfolios. The complete gate still found protected warm and cold-control regressions, so the decoded executor remains the default. Compact remains an opt-in experimental execution policy.

## What was measured

The P046 portfolio used two independent runs with 30 complete pairs per comparison and no runtime instrumentation. The paired matrix contains 65,700 observations against the immediate structural predecessor and 65,700 against P001, for **131,400 paired matrix observations**. The two original-MVP cold-control reports add **2,520 pairs** (1,260 against each baseline), for 133,920 total timing pairs. P046 reused source-current, verified timing datasets after the analysis aggregation fix; it preserved the raw reports, data, failed analyses and prior attempts and did not remeasure unchanged workloads.

The P046 primary-family candidate/baseline median ratios were:

| Family | Run 1 | Run 2 | Workloads per run |
| --- | ---: | ---: | ---: |
| Scalar | 0.86678789 | 0.79659666 | 85 |
| Memory | 0.81608416 | 0.81643069 | 14 |
| SIMD | 0.68761976 | 0.69060426 | 114 |

Ratios below 1.0 mean the Compact candidate was faster for that selected family. These family results did not override the gate's protected-control and cold-control checks. The analysis rejected default adoption after scalar protected workloads crossed their intervals and cold controls crossed their limits, including `call_overhead` and `prepare_large`. The gate computed paired statistics once and retained its rejected decision.

Compact coverage also passed the full supported/fallback conformance corpus: **145,760 selected and executed cases** across the P121 corpus and interactions. Coverage recorded 163,877,104 optimized source events and 71,895,541 fallback source events. Fallback cases count as successful semantic coverage; they are not Compact optimization wins.

The default and Compact build budgets passed, and dependency analysis reported **zero violations**. Compact's clean-build duration was 1.31445573 times the default build. The retained-artifact P121 check reported a conservative **22,345,236-byte budget** based on capacity plus headers; this is not measured allocator usage. A separate allocator probe measured requested live heap for each complete compiled Engine+Module graph: 10,798,108 bytes for Compact and 10,568,460 bytes for decoded, a ratio of **1.02172956**. This whole-graph probe includes compiled code and module plans together. It cannot isolate physical code pages and does not report allocator bookkeeping or RSS.

## Selection and API

`Engine::new(Config)` continues to select `ExecutionPolicy::Decoded`. The `Portable` policy also maps to decoded execution. A consumer that enables the `compact-executor` feature can opt into Compact explicitly:

```rust
use wtime_runtime::{Config, Engine, ExecutionPolicy};

let engine = Engine::with_execution_policy(
    Config::default(),
    ExecutionPolicy::Compact,
)?;
```

`Engine::with_execution_policy` returns the crate's `Result`; this example belongs in a function that propagates that result. When the `compact-executor` feature is not enabled, requesting `ExecutionPolicy::Compact` returns `ErrorKind::DisabledFeature`.

## Evidence and limits

The [full source-bound contract](P046-verification.json) drove the executed sequence: verification and production/build/conformance proofs in `target/verification/runs/P046/verify/attempt-0005`, paired measurements in `target/verification/runs/P046/measure/attempt-0004/measurements`, one portfolio statistics computation in `target/verification/runs/P046/analyze/attempt-0003/statistics.json`, then publication at `target/performance/P046/shared-finalization/attempt-0007/completion.json`. Source-current timing datasets were admitted/reused after the import and aggregate fixes; unchanged workloads were not retimed. The final report records one portfolio decision computation.

The analyzed source-current datasets include two cold-control reports:

- `target/verification/runs/P046/measure/attempt-0004/measurements/cold-p001-original-mvp/report.json`
- `target/verification/runs/P046/measure/attempt-0004/measurements/cold-structural-original-mvp/report.json`

The build attestations and separate retained-heap probe are in:

- `target/verification/runs/P046/verify/attempt-0005/production/build-record.json`
- `target/verification/runs/P046/verify/attempt-0005/build.json`
- `target/verification/runs/P046/verify/attempt-0005/optional-build.json`
- `target/verification/runs/P046/verify/attempt-0005/retained/report.json`

The host was shared and physical-core isolation was unknown. These results support the P046 rejection under its declared gate; they make **no Wasmtime performance claim**. They also do not replace a separate allocator-wide or RSS study.

## P047: bounded local arithmetic fusions

P047 completed with an **inconclusive** outcome after one statistics computation over 1,500 unique paired samples: two runs with 30 pairs per declared group. The targeted warm-call portfolio median candidate/baseline ratios were 0.78600466 (95% interval 0.75059993–0.81349370) in run 1 and 0.78787446 (0.76205491–0.83081033) in run 2. Those results support an approximately 21% targeted warm-call gain. Fifteen protected, cold, and non-target regression bounds remained inconclusive, so the gate did not promote the Compact tier to default.

The two pure i32 patterns remain available only with `compact-executor` and explicit `ExecutionPolicy::Compact`. `Engine::new` and `Portable` still select Decoded. The default path does not prepare a fusion table, and four P036 default-guard checks passed. The retained code adds no dependency or unsafe code and supplies a bounded planner/source-prefix interface used by P048. Its retention is an opt-in experiment; it makes no default-adoption or Wasmtime performance claim.

Full Compact runtime verification passed 347 cases with zero failures or skips, including the seven named P047 focused checks. Default and optional build budgets and dependency checks passed. The default build took 15.0455 s clean, 0.5912 s leaf and produced 1,000,712 stripped bytes; optional Compact took 11.0066 s clean, 0.5593 s leaf and produced 1,080,560 stripped bytes.

The source-bound analysis is `target/verification/runs/P047/analyze/attempt-0001/statistics.json`, full verification is `target/verification/runs/P047/verify/attempt-0001/receipt.json`, and the opt-in retention justification is `target/performance/P047/experimental-retention/attempt-0001/receipt.json`. The published completion record is `target/performance/P047/shared-finalization/attempt-0002/completion.json`; publication is `target/verification/invocations/publish/attempt-0018` and primary task snapshot is `target/performance/TASK-status/P047-publication-0001`.

## P048: compare and conditional-branch fusion

P048 completed with a **rejected** decision after one statistics computation. The selection had 1,500 logical pair references, with aliases reusing samples for 1,380 unique complete pairs. Targeted warm-call candidate/baseline ratios were 1.19343217 (95% interval 1.13393741–1.24607920) in run 1 and 1.20111768 (1.14651776–1.25308836) in run 2, showing a supported 19–20% regression. Additional primary and protected controls also showed supported regressions.

Full Compact runtime verification passed 352 cases with zero failures or skips. Default and optional build budgets and dependency checks passed. The eight candidate paths were retained in the rollback archive, while the exact 111-source predecessor was restored with fresh timestamps and P047 preserved. Seven restored P047 checks passed. P048 contributes no retained runtime change, opt-in experiment, or speed claim.

The source-bound analysis is `target/verification/runs/P048/analyze/attempt-0001/statistics.json`; full verification is `target/verification/runs/P048/verify/attempt-0001/receipt.json`; rollback is `target/performance/P048/rollback/attempt-0001/receipt.json`. The published completion record is `target/performance/P048/shared-finalization/attempt-0001/completion.json`; publication is `target/verification/invocations/publish/attempt-0019` and primary task snapshot is `target/performance/TASK-status/P048-publication-0001`.


## P126: scalable memory-workload aliases

P126 completed as a **contract-only foundation** for P049. It preserved the original 441 workload rows and appended 18 fixed aliases to the existing memory selection, leaving 459 rows total and all three admitted fixtures unchanged. The proof exercised 180 real stage/mode observations across decoded and compact runtime-only binaries; a warm batch of three represents 252 logical guest calls. All 14 named checks passed, including independent result, checksum, full-memory-state, repeat, and reset validation. The runtime source-entry count remained 111. These results validate workload coverage and the proof protocol; they provide no candidate comparison or performance claim.

The published completion is `target/performance/P126/shared-finalization/attempt-0002/completion.json` (evidence SHA-256 `498e0ba5cb2104f3d638e72d18fdbe72d527ce64bfeec93da84f760a972b9f15`). The full receipt is `target/verification/runs/P126/verify/attempt-0001/receipt.json`, and independent memory proof is `target/verification/runs/P126/verify/attempt-0001/memory-proof/report.json`. Two failed preflight attempts were retained and corrected before any driver or guest commands; canonical close attempt `0038` and publication attempt `0020` contain the successful record.


## P049: checked memory-address fusion

P049 completed as **rejected** after one statistics computation. Across two runs, the selected warm-call median ratios were 0.73615654 (95% interval 0.66134717–0.81614486) and 0.65142163 (0.52063894–0.79280063), indicating a targeted improvement. The complete gate rejected the candidate because it found supported regressions in `call_overhead`, `load_and_call`, and `read_scan_1024` at `warm_call`, alongside 37 other uncertainty reasons. This does not support a portfolio-wide or Wasmtime speed claim.

All eight focused checks and 355 full runtime cases passed; both build budgets passed. The rollback removed all 11 candidate core paths and restored the exact 111-source predecessor with fresh timestamps. Seventeen restored P047/P043 checks passed. P049 contributes no retained code; the decoded executor remains the default and P047 remains opt-in. The completion record is `target/performance/P049/shared-finalization/attempt-0001/completion.json`, statistics are `target/verification/runs/P049/analyze/attempt-0001/statistics.json`, and rollback evidence is `target/performance/P049/rollback/attempt-0001/receipt.json`.


## P050: bounded numeric leaf inlining

P050 completed **inconclusive** after two paired runs with 3,360 unique physical pairs and 3,660 logical references. The targeted direct-call warm-call ratios were 0.51972298 (95% interval 0.42718021–0.59793480) and 0.48282901 (0.39673729–0.64129041), or 48.03% and 51.72% lower time. No protected/cold regression was supported, but 56 bounds remained uncertain. The result retains the inliner only as an opt-in experiment behind `compact-executor` and explicit `ExecutionPolicy::Compact`; `Engine::new(Config)` remains Decoded. No default-adoption, full-portfolio, P001-total, or Wasmtime claim follows.

All 354 full Compact runtime cases and seven focused P050 checks passed. Both build budgets passed without threshold changes after two failed attempts. The first analysis attempt computed statistics but failed the final task-index reference check; the accepted analysis-only retry reused the same 3,360 physical pairs and prior passing build/correctness evidence. The accepted statistics artifact records one computation; the history is not a single-computation-across-attempts claim. Completion is `target/performance/P050/shared-finalization/attempt-0003/completion.json` (evidence SHA-256 `8cb480f26bcf239f4929c13d7e9ee414377f59a94349506e3ffa53bfe810d4fe`); accepted statistics, verification, and retention receipts are under `target/verification/runs/P050/analyze/attempt-0002`, `target/verification/runs/P050/verify/attempt-0002`, and `target/performance/P050/experimental-retention/attempt-0001`.
