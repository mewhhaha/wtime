# Performance task index

Progress: done=104, todo=40, in_progress=0, blocked=0, deferred=0 (144 total). Read [README.md](README.md), [EXECUTION.md](EXECUTION.md), and [TASKS.json](TASKS.json).

## Suggested first work

P001 freezes the unrebuildable historical baseline. Then P002/P003/P004/P005 provide paired measurement; P006/P007 gather diagnostic evidence. P016/P018/P019/P020/P021 establish executor/accounting contracts. P017 is the first direct structural optimization. Workload leaf tasks P008–P013/P015 can run in parallel after the shared schema, with manifest integration serialized.

## Lanes

| Lane | Tasks | Exit |
| --- | --- | --- |
| Evidence and contracts | P001–P022 | Archived baseline, named workloads, exact state/accounting |
| Structural wins | P023–P035 plus P017 | P035: 10x compute aspiration and current profiles |
| Compact execution | P036–P046 | P046: correctness/cold/build selection |
| Fusion and specialization | P047–P056 | P056: bounded accepted fusion portfolio |
| Register execution | P057–P067, P130–P133, P143–P144 | P067: adaptive crossover and portable aspiration |
| GC/SIMD/shared/continuations | P068–P080 | P080: family-specific measured decisions |
| Portable/offline research | P081–P093 | Optional independent experiment decisions |
| Native research | P094–P107 | Conditional go/no_go; P107 supported-target gate |
| Final selection and verification | P108–P112 | Selected portable/optional portfolio and honest report |

The IDs are labels, not a serial schedule. Optional reviews do not create dependency edges. P108/P112 can complete with unselected research deferred; they cannot ship an optional tier lacking its branch gate. Shared-file overlap still serializes edits.

## Tasks

### 00-measurement

| Task | Kind / branch | Prerequisites |
| --- | --- | --- |
| [P001 — Archive the current runtime and benchmark baseline](00-measurement/P001-freeze-baseline.md) | foundation / portable | — |
| [P002 — Compare separately built baseline and candidate binaries](00-measurement/P002-paired-driver.md) | foundation / portable | P001 |
| [P003 — Define workload selections and engine capability preflight](00-measurement/P003-workload-schema.md) | foundation / portable | P002 |
| [P004 — Compute uncertainty and preparation break-even](00-measurement/P004-paired-statistics.md) | foundation / portable | P002, P003 |
| [P005 — Bind speed decisions to build and dependency budgets](00-measurement/P005-performance-gates.md) | foundation / portable | P001, P004 |
| [P006 — Add bounded development-only hot-path counters](00-measurement/P006-diagnostic-counters.md) | foundation / portable | P001, P003 |
| [P007 — Capture CPU profiles and rank actionable costs](00-measurement/P007-profile-hotpaths.md) | foundation / portable | P003, P006 |
| [P008 — Add scaling, branch and call benchmark families](00-measurement/P008-scalar-workloads.md) | foundation / portable | P003 |
| [P009 — Add memory width, bulk and growth benchmark families](00-measurement/P009-memory-workloads.md) | foundation / portable | P003 |
| [P010 — Add SIMD and strict floating-point benchmarks](00-measurement/P010-simd-float-workloads.md) | foundation / portable | P003 |
| [P011 — Measure reference, GC and exception paths](00-measurement/P011-gc-exception-workloads.md) | foundation / portable | P003 |
| [P012 — Measure continuation capture, resume and accounting](00-measurement/P012-continuation-workloads.md) | foundation / portable | P003 |
| [P013 — Measure shared memory, contention and wait latency](00-measurement/P013-thread-workloads.md) | foundation / portable | P003 |
| [P014 — Run source-bound correctness checks across execution tiers](00-measurement/P014-tier-differential-runner.md) | foundation / portable | P003, P016, P018, P020 |
| [P015 — Add reproducible application-shaped held-out kernels](00-measurement/P015-representative-kernels.md) | foundation / portable | P003, P008, P009, P010 |

### 01-execution-contracts

| Task | Kind / branch | Prerequisites |
| --- | --- | --- |
| [P016 — Add explicit executor policies and test integration hooks](01-execution-contracts/P016-execution-policy.md) | foundation / portable | P001, P003, P006 |
| [P017 — Make every Module clone share immutable metadata](01-execution-contracts/P017-shared-module-payload.md) | candidate / portable | P002, P003, P004, P005, P007, P016 |
| [P018 — Define source cursors and materialized guest state](01-execution-contracts/P018-source-cursor.md) | foundation / portable | P016 |
| [P019 — Define bounded optimizer effect and source expansion metadata](01-execution-contracts/P019-effect-certificates.md) | foundation / portable | P018 |
| [P020 — Centralize exact source instruction accounting](01-execution-contracts/P020-exact-execution-accounting.md) | foundation / portable | P018, P019 |
| [P021 — Compute bounded per-function execution metadata](01-execution-contracts/P021-function-execution-plans.md) | foundation / portable | P016, P018 |
| [P022 — Add adversarial state and interaction tier fixtures](01-execution-contracts/P022-state-differential-fixtures.md) | foundation / portable | P011, P012, P014, P021 |
| [P129 — Add bounded source-visit selection to execution diagnostics](01-execution-contracts/P129-source-visit-diagnostics.md) | foundation / portable | P018, P020, P057 |

### 02-structural-wins

| Task | Kind / branch | Prerequisites |
| --- | --- | --- |
| [P023 — Fetch ordinary instructions without cloning their payload](02-structural-wins/P023-borrow-instructions.md) | candidate / portable | P002, P004, P005, P008, P016, P017, P021 |
| [P024 — Move branch results and share label metadata without allocation](02-structural-wins/P024-in-place-branch-results.md) | candidate / portable | P002, P004, P005, P008, P016, P017, P021, P023 |
| [P025 — Remove per-load heap allocation for all scalar widths](02-structural-wins/P025-fixed-scalar-reads.md) | candidate / portable | P002, P004, P005, P009, P016, P017 |
| [P026 — Use fixed buffers for SIMD and lane memory access](02-structural-wins/P026-fixed-vector-reads.md) | candidate / portable | P002, P004, P005, P010, P016, P025 |
| [P027 — Use slice operations for unshared bulk memory and table copies](02-structural-wins/P027-unshared-bulk-slices.md) | candidate / portable | P002, P004, P005, P009, P016, P025 |
| [P028 — Execute common scalar operations in place on the operand stack](02-structural-wins/P028-in-place-numeric-stack.md) | candidate / portable | P002, P004, P005, P008, P016, P020 |
| [P029 — Stop cloning function signatures into every activation](02-structural-wins/P029-shared-function-metadata.md) | candidate / portable | P002, P004, P005, P008, P016, P017, P021 |
| [P030 — Split checked public entry from validated guest call entry](02-structural-wins/P030-validated-guest-frame-entry.md) | candidate / portable | P002, P004, P005, P008, P016, P021, P029 |
| [P031 — Reuse invocation scratch capacity safely under host reentry](02-structural-wins/P031-activation-scratch-pool.md) | candidate / portable | P002, P004, P005, P008, P016, P018, P029, P030 |
| [P032 — Move guest arguments and tail calls without split_off allocation](02-structural-wins/P032-call-argument-ranges.md) | candidate / portable | P002, P004, P005, P008, P016, P024, P030, P031 |
| [P033 — Cache immutable subtype answers with identity and work guards](02-structural-wins/P033-bounded-subtype-cache.md) | candidate / portable | P002, P004, P005, P011, P016, P029 |
| [P034 — Replace repeated flattened type scans with immutable layout lookups](02-structural-wins/P034-flat-gc-metadata.md) | candidate / portable | P002, P004, P005, P011, P016, P017, P021 |
| [P035 — Evaluate structural wins and choose the next bottleneck](02-structural-wins/P035-structural-milestone.md) | gate / portable | P004, P005, P007, P008, P009, P015, P017, P023, P024, P025, P026, P027, P028, P029, P030, P031, P032, P033, P034 |

### 03-compact-executor

| Task | Kind / branch | Prerequisites |
| --- | --- | --- |
| [P036 — Define small immutable execution instructions and side tables](03-compact-executor/P036-compact-op-format.md) | foundation / portable | P019, P021, P035 |
| [P037 — Map compact instructions to source events and control targets](03-compact-executor/P037-compact-source-maps.md) | foundation / portable | P018, P019, P020, P036 |
| [P038 — Lower constants, locals and common integer operations](03-compact-executor/P038-compact-scalar-lowering.md) | candidate / portable | P002, P004, P005, P008, P014, P016, P023, P028, P036, P037 |
| [P039 — Lower float/vector constants and barrier adapters](03-compact-executor/P039-compact-float-vector-adapters.md) | candidate / portable | P002, P004, P005, P010, P014, P016, P026, P036, P037, P038 |
| [P040 — Lower structured control to resolved jump and result plans](03-compact-executor/P040-compact-control-plans.md) | candidate / portable | P002, P004, P005, P008, P014, P016, P024, P037, P038 |
| [P041 — Resolve direct guest call bindings once per instance](03-compact-executor/P041-instance-call-plans.md) | candidate / portable | P002, P004, P005, P008, P014, P016, P029, P030, P032, P038, P040 |
| [P042 — Add guarded bounded indirect and call_ref caches](03-compact-executor/P042-indirect-call-inline-cache.md) | candidate / portable | P002, P004, P005, P008, P014, P016, P033, P041 |
| [P043 — Resolve memory bindings and specialize checked scalar accesses](03-compact-executor/P043-compact-memory-plans.md) | candidate / portable | P002, P004, P005, P009, P014, P016, P025, P026, P027, P037, P038, P041 |
| [P044 — Materialize compact state for GC, exceptions and continuations](03-compact-executor/P044-compact-proposal-boundaries.md) | candidate / portable | P002, P004, P005, P014, P016, P022, P039, P040, P041, P043, P113–P120 |
| [P045 — Verify and fuzz prepared compact programs](03-compact-executor/P045-compact-verifier-fuzz.md) | gate / portable | P004, P005, P014, P019, P037, P040, P041, P043, P044 |
| [P046 — Decide whether compact execution earns default use](03-compact-executor/P046-compact-milestone.md) | gate / portable | P004, P005, P014, P015, P035, P038, P039, P040, P041, P042, P043, P044, P045 |
| [P113 — Add integration-prefix benchmark and paired baseline support](03-compact-executor/P113-integration-prefix-benchmark-foundation.md) | foundation / portable | P043, P014, P022 |
| [P114 — Verify bounded straight-line prefix and exit certificates](03-compact-executor/P114-prefix-exit-certificate.md) | foundation / portable | P113, P037, P043 |
| [P115 — Integrate canonical handoff and reference-local admission](03-compact-executor/P115-canonical-prefix-handoff.md) | foundation / portable | P114, P039, P041 |
| [P116 — Cover GC locals, host reentry and root lifetime](03-compact-executor/P116-prefix-gc-reentry-tests.md) | foundation / portable | P115, P022 |
| [P117 — Cover exception transfer from compact source prefixes](03-compact-executor/P117-prefix-eh-transfer-tests.md) | foundation / portable | P115, P022, P040 |
| [P118 — Cover suspend and bind transfer from compact prefixes](03-compact-executor/P118-prefix-suspend-bind-tests.md) | foundation / portable | P115, P022 |
| [P119 — Cover resume and resume_throw after compact exits](03-compact-executor/P119-prefix-resume-tests.md) | foundation / portable | P115, P118, P022 |
| [P120 — Cover switch transfer and compact resource boundaries](03-compact-executor/P120-prefix-switch-resource-tests.md) | foundation / portable | P115, P116, P117, P118, P119, P022 |

### 04-specialization-and-fusion

| Task | Kind / branch | Prerequisites |
| --- | --- | --- |
| [P047 — Fuse a bounded set of local and constant arithmetic patterns](04-specialization-and-fusion/P047-local-arithmetic-fusions.md) | candidate / portable | P002, P004, P005, P008, P014, P016, P019, P020, P037, P038, P046 |
| [P048 — Fuse integer compare and conditional control transfers](04-specialization-and-fusion/P048-compare-branch-fusions.md) | candidate / portable | P002, P004, P005, P008, P014, P016, P040, P047 |
| [P126 — Add scalable memory-width and read-scan workload aliases](04-specialization-and-fusion/P126-scalable-memory-workload-aliases.md) | foundation / portable | P009, P016, P025, P043 |
| [P049 — Fuse pure address formation with a checked memory operation](04-specialization-and-fusion/P049-address-memory-fusions.md) | candidate / portable | P002, P004, P005, P009, P014, P016, P043, P047, P126 |
| [P050 — Inline tiny direct numeric leaves with logical call accounting](04-specialization-and-fusion/P050-bounded-leaf-inlining.md) | candidate / portable | P002, P004, P005, P008, P014, P016, P020, P041, P047 |
| [P051 — Try a two-value numeric operand cache](04-specialization-and-fusion/P051-operand-top-cache.md) | experiment / optional | P002, P004, P005, P008, P014, P016, P018, P020, P038, P044, P047 |
| [P052 — Try safe unboxed storage for proven numeric activations](04-specialization-and-fusion/P052-numeric-slot-storage.md) | experiment / optional | P002, P004, P005, P008, P014, P016, P021, P038, P051 |
| [P053 — Separate metered and unmetered dispatch without weaker defaults](04-specialization-and-fusion/P053-accounting-mode-specialization.md) | candidate / portable | P002, P004, P005, P008, P014, P016, P020, P038, P047 |
| [P054 — Fold pure integer work while preserving source events](04-specialization-and-fusion/P054-effect-preserving-peepholes.md) | candidate / portable | P002, P004, P005, P008, P014, P016, P019, P020, P037, P047 |
| [P055 — Compare split match and handler-function scalar dispatch](04-specialization-and-fusion/P055-dispatch-shape-experiment.md) | experiment / optional | P002, P004, P005, P007, P008, P014, P016, P038, P040, P053 |
| [P056 — Select fusions and keep the portable hot path small](04-specialization-and-fusion/P056-specialization-milestone.md) | gate / portable | P004, P005, P014, P015, P046, P047, P048, P049, P050, P053, P054 |
| [P127 — Bind metered and unmetered settings to staged benchmark runs](04-specialization-and-fusion/P127-metered-unmetered-benchmark-setting.md) | foundation / portable | P002, P003, P008, P014, P016, P020, P126 |
| [P128 — Add minimal scalar identity and literal-folding workloads](04-specialization-and-fusion/P128-minimal-scalar-workloads.md) | foundation / portable | P003, P008, P014, P016, P019, P020, P037, P047, P126 |

### 05-register-executor

| Task | Kind / branch | Prerequisites |
| --- | --- | --- |
| [P057 — Define a small numeric register bytecode and eligibility rules](05-register-executor/P057-register-ir-contract.md) | foundation / portable | P018, P019, P020, P021, P056 |
| [P058 — Translate numeric stack code into direct slot operands](05-register-executor/P058-stack-to-register-lowering.md) | foundation / portable | P037, P040, P057, P129 |
| [P059 — Try conservative temporary-slot reuse without global SSA](05-register-executor/P059-bounded-slot-reuse.md) | experiment / optional | P002, P004, P005, P008, P014, P016, P058 |
| [P060 — Implement safe parallel moves and register control transfers](05-register-executor/P060-register-edge-moves.md) | candidate / portable | P002, P004, P005, P008, P014, P016, P040, P058, P061 |
| [P061 — Execute checked integer register operations](05-register-executor/P061-register-integer-execution.md) | candidate / portable | P002, P004, P005, P008, P014, P016, P028, P057, P058, P130 |
| [P062 — Add strict floating-point register operations](05-register-executor/P062-register-float-execution.md) | candidate / portable | P002, P004, P005, P010, P014, P016, P039, P061 |
| [P063 — Add direct checked unshared memory register operations](05-register-executor/P063-register-unshared-memory.md) | candidate / portable | P002, P004, P005, P009, P014, P016, P043, P061 |
| [P064 — Exit register execution before calls and resume decoded state](05-register-executor/P064-register-call-exits.md) | candidate / portable | P002, P004, P005, P008, P014, P016, P018, P022, P041, P061, P063, P131 |
| [P065 — Verify roots and captured state around register exits](05-register-executor/P065-register-root-and-capture-barriers.md) | gate / portable | P004, P005, P014, P022, P044, P062, P063, P064 |
| [P066 — Compile register code lazily at bounded function thresholds](05-register-executor/P066-function-boundary-tiering.md) | candidate / portable | P002, P004, P005, P014, P015, P016, P057, P061, P062, P063, P064, P065, P132, P133, P134, P135, P136, P137, P138, P139, P140, P142 |
| [P067 — Evaluate register execution and adaptive break-even](05-register-executor/P067-register-milestone.md) | gate / portable | P004, P005, P014, P015, P056, P057, P058, P060, P061, P062, P063, P064, P065, P066 |
| [P130 — Prepare bounded parallel register edge moves](05-register-executor/P130-bounded-parallel-edge-moves.md) | foundation / portable | P057, P058 |
| [P131 — Prepare bounded Register prefixes for decoded handoff](05-register-executor/P131-bounded-register-prefix.md) | foundation / portable | P057, P058, P063, P129 |
| [P132 — Extract transactional per-function Register preparation](05-register-executor/P132-extract-transactional-register-preparation.md) | foundation / portable | P065, P130, P131 |
| [P133 — Add a bounded lazy Register preparation cache](05-register-executor/P133-bounded-lazy-register-cache.md) | foundation / portable | P132 |
| [P134 — Integrate opt-in lazy Register entry](05-register-executor/P134-opt-in-lazy-register-entry.md) | foundation / portable | P133 |
| [P135 — Add a fresh-entry Register batch protocol](05-register-executor/P135-fresh-entry-batch-protocol.md) | foundation / portable | P134 |
| [P136 — Bind Register entry settings to source-bound paired collection](05-register-executor/P136-register-entry-paired-collection.md) | foundation / portable | P135 |
| [P137 — Analyze paired Register entry totals and phases](05-register-executor/P137-entry-total-paired-analysis.md) | foundation / portable | P136 |
| [P138 — Expose source-bound Register cache diagnostics](05-register-executor/P138-register-cache-diagnostics.md) | foundation / portable | P134, P137 |
| [P139 — Collect side-specific Register entry settings](05-register-executor/P139-side-specific-register-entry-settings.md) | foundation / portable | P136, P137, P138 |
| [P140 — Freeze and attest the adaptive Register entry threshold](05-register-executor/P140-register-entry-threshold.md) | foundation / portable | P127, P139 |
| [P141 — Add individual-entry Register timing and diagnostics](05-register-executor/P141-register-entry-sequence-driver.md) | foundation / portable | P138, P140 |
| [P142 — Collect and analyze source-bound Register cold and churn sequences](05-register-executor/P142-register-cold-churn-collection.md) | foundation / portable | P139, P141 |
| [P143 — Forward explicit Register thresholds through conformance](05-register-executor/P143-register-threshold-conformance.md) | foundation / portable | P065, P066, P138 |
| [P144 — Report Register activation capacity high-water](05-register-executor/P144-register-activation-capacity-diagnostics.md) | foundation / portable | P065, P066, P138, P141 |

### 06-memory-gc-simd-and-continuations

| Task | Kind / branch | Prerequisites |
| --- | --- | --- |
| [P068 — Copy only references into host and collection root snapshots](06-memory-gc-simd-and-continuations/P068-reference-only-root-snapshots.md) | candidate / portable | P002, P004, P005, P011, P014, P016, P022, P031, P125 |
| [P069 — Use conservative typed root maps for prepared frames](06-memory-gc-simd-and-continuations/P069-typed-root-slot-maps.md) | candidate / portable | P002, P004, P005, P011, P014, P016, P021, P044, P065, P068 |
| [P070 — Try compact safe storage for packed numeric GC arrays](06-memory-gc-simd-and-continuations/P070-packed-gc-array-storage.md) | experiment / optional | P002, P004, P005, P011, P014, P016, P034, P068 |
| [P071 — Reuse bounded GC mark and root work buffers](06-memory-gc-simd-and-continuations/P071-collector-scratch-reuse.md) | candidate / portable | P002, P004, P005, P011, P014, P016, P068, P069 |
| [P072 — Maintain continuation bytes incrementally and transactionally](06-memory-gc-simd-and-continuations/P072-continuation-byte-ledger.md) | candidate / portable | P002, P004, P005, P012, P014, P016, P029, P031 |
| [P073 — Share immutable continuation metadata and bound snapshot copies](06-memory-gc-simd-and-continuations/P073-continuation-frame-sharing.md) | candidate / portable | P002, P004, P005, P012, P014, P016, P024, P029, P031, P069, P072 |
| [P074 — Specialize portable SIMD bitwise and select operations](06-memory-gc-simd-and-continuations/P074-portable-vector-bitops.md) | candidate / portable | P002, P004, P005, P010, P014, P016, P026, P039 |
| [P075 — Try typed safe lane kernels that autovectorize](06-memory-gc-simd-and-continuations/P075-portable-lane-specialization.md) | experiment / optional | P002, P004, P005, P010, P014, P016, P074 |
| [P076 — Remove shared read allocation without weakening coordination](06-memory-gc-simd-and-continuations/P076-shared-fixed-width-access.md) | candidate / portable | P002, P004, P005, P013, P014, P016, P025, P026 |
| [P077 — Improve wait registration lookup while retaining exact notifications](06-memory-gc-simd-and-continuations/P077-wait-queue-indexing.md) | candidate / portable | P002, P004, P005, P013, P014, P016, P076 |
| [P078 — Reduce shared bulk-copy overhead with coordinated fixed chunks](06-memory-gc-simd-and-continuations/P078-shared-bulk-chunks.md) | candidate / portable | P002, P004, P005, P009, P013, P014, P016, P027, P076 |
| [P079 — Check specialist retention, allocation and compilation costs](06-memory-gc-simd-and-continuations/P079-specialist-budget-gate.md) | gate / portable | P004, P005, P014, P068, P069, P071, P072, P073, P074, P076, P077, P078 |
| [P080 — Evaluate GC, SIMD, continuation and shared-memory progress](06-memory-gc-simd-and-continuations/P080-specialist-milestone.md) | gate / portable | P004, P005, P014, P067, P068, P069, P071, P072, P073, P074, P076, P077, P078, P079 |
| [P125 — Add source-bound host root-snapshot workloads](06-memory-gc-simd-and-continuations/P125-host-root-workloads.md) | foundation / portable | P011, P033, P034, P123 |

### 07-unconventional-portable-experiments

| Task | Kind / branch | Prerequisites |
| --- | --- | --- |
| [P081 — Generate a small profile-selected superinstruction set](07-unconventional-portable-experiments/P081-generated-superinstructions.md) | experiment / optional | P002, P004, P005, P007, P008, P014, P016, P019, P020, P047, P048, P049, P056 |
| [P082 — Try one dispatch per safe numeric basic-block tape](07-unconventional-portable-experiments/P082-basic-block-tapes.md) | experiment / optional | P002, P004, P005, P008, P014, P016, P018, P019, P020, P055, P057, P060 |
| [P083 — Hoist successful unshared bounds checks with conservative guards](07-unconventional-portable-experiments/P083-region-bounds-certificates.md) | experiment / optional | P002, P004, P005, P009, P014, P016, P019, P020, P043, P063 |
| [P084 — Record short scalar loop traces at verified backedges](07-unconventional-portable-experiments/P084-bounded-trace-recording.md) | experiment / optional | P002, P004, P005, P008, P014, P016, P018, P019, P020, P057, P060, P061, P067 |
| [P085 — Execute guarded scalar traces with exact side exits](07-unconventional-portable-experiments/P085-trace-execution-and-osr.md) | experiment / optional | P002, P004, P005, P008, P014, P016, P020, P058, P060, P061, P084 |
| [P086 — Try exact closed-form acceleration of one pure integer loop](07-unconventional-portable-experiments/P086-proved-loop-summaries.md) | experiment / optional | P002, P004, P005, P008, P014, P016, P019, P020, P058, P061, P088 |
| [P087 — Try lockstep execution of independent numeric invocations](07-unconventional-portable-experiments/P087-independent-instance-batching.md) | experiment / optional | P002, P004, P005, P008, P014, P016, P057, P058, P061, P067 |
| [P088 — Evaluate an explicit bounded cancellation polling policy](07-unconventional-portable-experiments/P088-explicit-bounded-polling.md) | experiment / optional | P002, P004, P005, P014, P016, P020, P022, P053, P067 |
| [P089 — Cache identical prepared modules within one Engine](07-unconventional-portable-experiments/P089-bounded-module-cache.md) | experiment / optional | P001, P002, P004, P005, P014, P015, P016, P017, P021, P066 |
| [P090 — Try bounded parallel lowering for large modules only](07-unconventional-portable-experiments/P090-parallel-cold-lowering.md) | experiment / optional | P002, P004, P005, P014, P015, P016, P021, P036, P057, P066 |
| [P091 — Export a verified numeric program for an offline Rust AOT tool](07-unconventional-portable-experiments/P091-safe-aot-export-contract.md) | experiment / optional | P014, P018, P019, P057, P058, P060, P061, P067 |
| [P092 — Generate safe numeric Rust kernels from verified programs](07-unconventional-portable-experiments/P092-safe-rust-aot-generator.md) | experiment / optional | P002, P004, P005, P008, P014, P016, P020, P061, P062, P091 |
| [P093 — Evaluate safe Rust AOT distribution and break-even](07-unconventional-portable-experiments/P093-aot-break-even-gate.md) | gate / optional | P004, P005, P014, P015, P091, P092 |

### 08-optional-native-backend

| Task | Kind / branch | Prerequisites |
| --- | --- | --- |
| [P094 — Decide the tiny native backend boundary and budget](08-optional-native-backend/P094-native-boundary-decision.md) | experiment / optional | P005, P018, P019, P020, P067, P080 |
| [P095 — Create the minimal optional numeric backend interface](08-optional-native-backend/P095-native-ir-and-abi.md) | foundation / optional | P057, P058, P060, P094 |
| [P096 — Implement one audited W^X mapping owner for x86 Linux](08-optional-native-backend/P096-executable-mapping-owner.md) | foundation / optional | P094, P095 |
| [P097 — Emit a tiny verified x86_64 integer block subset](08-optional-native-backend/P097-x86-integer-block-emitter.md) | foundation / optional | P061, P095, P096 |
| [P098 — Add checked branches and bounded native numeric loops](08-optional-native-backend/P098-x86-control-emitter.md) | foundation / optional | P060, P097 |
| [P099 — Add exact fuel, traps and bounded cancellation to native execution](08-optional-native-backend/P099-native-accounting-exits.md) | candidate / optional | P002, P004, P005, P014, P016, P018, P020, P022, P095, P097, P098 |
| [P100 — Compile and retain native code only for eligible hot functions](08-optional-native-backend/P100-bounded-native-tiering.md) | candidate / optional | P002, P004, P005, P014, P015, P016, P066, P094, P095, P096, P097, P098, P099 |
| [P101 — Cross native memory boundaries through safe checked callouts](08-optional-native-backend/P101-native-memory-callouts.md) | experiment / optional | P002, P004, P005, P009, P014, P016, P043, P063, P099, P100 |
| [P102 — Try audited native checked access to a borrowed unshared view](08-optional-native-backend/P102-native-inline-unshared-memory.md) | experiment / optional | P002, P004, P005, P009, P014, P016, P083, P095, P097, P098, P099, P101 |
| [P103 — Extend native numeric code with checked traps and strict floats](08-optional-native-backend/P103-native-strict-numerics.md) | experiment / optional | P002, P004, P005, P010, P014, P016, P062, P097, P099, P100 |
| [P104 — Isolate a tiny feature-detected native SIMD kernel set](08-optional-native-backend/P104-optional-native-simd.md) | experiment / optional | P002, P004, P005, P010, P014, P016, P074, P075, P094, P095, P099 |
| [P105 — Prototype reproducible checked copy-and-patch templates](08-optional-native-backend/P105-copy-and-patch-alternative.md) | experiment / optional | P002, P004, P005, P008, P014, P016, P019, P095, P096, P097, P098, P099 |
| [P106 — Prototype the same tiny native subset on AArch64](08-optional-native-backend/P106-aarch64-native-prototype.md) | experiment / optional | P002, P004, P005, P008, P014, P016, P095, P096, P097, P098, P099 |
| [P107 — Audit selected native paths and their real crossover](08-optional-native-backend/P107-native-milestone.md) | gate / optional | P004, P005, P014, P015, P094, P095, P096, P097, P098, P099, P100 |

### 09-integration-and-selection

| Task | Kind / branch | Prerequisites |
| --- | --- | --- |
| [P108 — Choose defaults, opt-in tiers and retired experiments](09-integration-and-selection/P108-select-performance-portfolio.md) | gate / portable | P004, P005, P014, P035, P046, P056, P067, P080 |
| [P109 — Refresh full conformance and hardening for selected tiers](09-integration-and-selection/P109-selected-tier-conformance.md) | gate / portable | P004, P005, P014, P022, P108 |
| [P110 — Verify the final portfolio stays small and quick to prepare](09-integration-and-selection/P110-final-cold-and-build-budget.md) | gate / portable | P004, P005, P014, P108, P109 |
| [P111 — Add honest smoke, regression and reference-machine lanes](09-integration-and-selection/P111-performance-ci-lanes.md) | gate / portable | P003, P004, P005, P014, P108, P109, P110 |
| [P112 — Publish the selected performance results and next climb](09-integration-and-selection/P112-performance-release-report.md) | gate / portable | P004, P005, P014, P108, P109, P110, P111 |

### P046 tooling foundations

| Task | Kind / branch | Prerequisites |
| --- | --- | --- |
| [P121 — Record complete Compact conformance and preparation coverage](03-compact-executor/P121-compact-full-conformance-counts.md) | foundation / portable | P014, P045 |
| [P122 — Prove Compact representative timing keeps the source-bound guest boundary](03-compact-executor/P122-compact-representative-protocol.md) | foundation / portable | P015, P035, P045 |
| [P123 — Adapt supported typed workload routes to immutable P001 sources](03-compact-executor/P123-p001-typed-harness-adapter.md) | foundation / portable | P001, P002, P003, P015, P045, P122 |
| [P124 — Validate the complete Compact portfolio gate producer](03-compact-executor/P124-compact-four-family-portfolio-gate.md) | foundation / portable | P004, P005, P015, P035, P045, P121, P122, P123 |
