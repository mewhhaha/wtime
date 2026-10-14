# P108 — Choose defaults, opt-in tiers and retired experiments

Status: **todo**

Outcome: **pending**

Kind: `gate` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)

[P035 — Evaluate structural wins and choose the next bottleneck](../02-structural-wins/P035-structural-milestone.md)

[P046 — Decide whether compact execution earns default use](../03-compact-executor/P046-compact-milestone.md)

[P056 — Select fusions and keep the portable hot path small](../04-specialization-and-fusion/P056-specialization-milestone.md)

[P067 — Evaluate register execution and adaptive break-even](../05-register-executor/P067-register-milestone.md)

[P080 — Evaluate GC, SIMD, continuation and shared-memory progress](../06-memory-gc-simd-and-continuations/P080-specialist-milestone.md)

Optional results to review (do not block the portable path):

[P051 — Try a two-value numeric operand cache](../04-specialization-and-fusion/P051-operand-top-cache.md)

[P052 — Try safe unboxed storage for proven numeric activations](../04-specialization-and-fusion/P052-numeric-slot-storage.md)

[P055 — Compare safe match, split dispatch and handler-table shapes](../04-specialization-and-fusion/P055-dispatch-shape-experiment.md)

[P059 — Try conservative temporary-slot reuse without global SSA](../05-register-executor/P059-bounded-slot-reuse.md)

[P070 — Try compact safe storage for packed numeric GC arrays](../06-memory-gc-simd-and-continuations/P070-packed-gc-array-storage.md)

[P075 — Try typed safe lane kernels that autovectorize](../06-memory-gc-simd-and-continuations/P075-portable-lane-specialization.md)

[P081 — Generate a small profile-selected superinstruction set](../07-unconventional-portable-experiments/P081-generated-superinstructions.md)

[P082 — Try one dispatch per safe numeric basic-block tape](../07-unconventional-portable-experiments/P082-basic-block-tapes.md)

[P083 — Hoist successful unshared bounds checks with conservative guards](../07-unconventional-portable-experiments/P083-region-bounds-certificates.md)

[P084 — Record short scalar loop traces at verified backedges](../07-unconventional-portable-experiments/P084-bounded-trace-recording.md)

[P085 — Execute guarded scalar traces with exact side exits](../07-unconventional-portable-experiments/P085-trace-execution-and-osr.md)

[P086 — Try exact closed-form acceleration of one pure integer loop](../07-unconventional-portable-experiments/P086-proved-loop-summaries.md)

[P087 — Try lockstep execution of independent numeric invocations](../07-unconventional-portable-experiments/P087-independent-instance-batching.md)

[P088 — Evaluate an explicit bounded cancellation polling policy](../07-unconventional-portable-experiments/P088-explicit-bounded-polling.md)

[P089 — Cache identical prepared modules within one Engine](../07-unconventional-portable-experiments/P089-bounded-module-cache.md)

[P090 — Try bounded parallel lowering for large modules only](../07-unconventional-portable-experiments/P090-parallel-cold-lowering.md)

[P091 — Export a verified numeric program for an offline Rust AOT tool](../07-unconventional-portable-experiments/P091-safe-aot-export-contract.md)

[P092 — Generate safe numeric Rust kernels from verified programs](../07-unconventional-portable-experiments/P092-safe-rust-aot-generator.md)

[P093 — Evaluate safe Rust AOT distribution and break-even](../07-unconventional-portable-experiments/P093-aot-break-even-gate.md)

[P094 — Decide the tiny native backend boundary and budget](../08-optional-native-backend/P094-native-boundary-decision.md)

[P095 — Create the minimal optional numeric backend interface](../08-optional-native-backend/P095-native-ir-and-abi.md)

[P096 — Implement one audited W^X mapping owner for x86 Linux](../08-optional-native-backend/P096-executable-mapping-owner.md)

[P097 — Emit a tiny verified x86_64 integer block subset](../08-optional-native-backend/P097-x86-integer-block-emitter.md)

[P098 — Add checked branches and bounded native numeric loops](../08-optional-native-backend/P098-x86-control-emitter.md)

[P099 — Add exact fuel, traps and bounded cancellation to native execution](../08-optional-native-backend/P099-native-accounting-exits.md)

[P100 — Compile and retain native code only for eligible hot functions](../08-optional-native-backend/P100-bounded-native-tiering.md)

[P101 — Cross native memory boundaries through safe checked callouts](../08-optional-native-backend/P101-native-memory-callouts.md)

[P102 — Try audited native checked access to a borrowed unshared view](../08-optional-native-backend/P102-native-inline-unshared-memory.md)

[P103 — Extend native numeric code with checked traps and strict floats](../08-optional-native-backend/P103-native-strict-numerics.md)

[P104 — Isolate a tiny feature-detected native SIMD kernel set](../08-optional-native-backend/P104-optional-native-simd.md)

[P105 — Prototype reproducible checked copy-and-patch templates](../08-optional-native-backend/P105-copy-and-patch-alternative.md)

[P106 — Prototype the same tiny native subset on AArch64](../08-optional-native-backend/P106-aarch64-native-prototype.md)

[P107 — Audit selected native paths and their real crossover](../08-optional-native-backend/P107-native-milestone.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C5, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `docs/architecture/performance-policy.md`
- `docs/benchmarks/performance/decisions.json`
- `tools/wtime-bench/tests/test_p108_selection.py`
- `tools/wtime-bench/tier_check.py`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

A portfolio manifest names each selected tier/optimization, source-bound acceptance artifacts and default policy; pending/rejected/deferred experiments remain visible and cannot become default through Cargo feature drift.

## Implementation recipe

1. Review all required milestone evidence and any completed optional experiments, recording adopted/rejected/inconclusive/not_selected outcomes without waiting for every optional branch.
2. Select the smallest compatible default path that passes warm/cold/build/support gates; optional AOT/native/traces need their own passing branch gate before being offered as supported.
3. Remove retired default-path tables/features/code and bound remaining variants; refresh dependency/feature policy and preserve unimplemented architecture limitations.
4. Produce a machine-readable selection manifest used by final checks, listing exact eligible subsets, fallback reasons, quantum policy and outstanding work.
5. Teach tier_check --tier selected to expand decisions.json into the exact selected supported modes/features; unavailable pending branch gates are hard errors, not silent omissions.

## Meaningful cases

- Pending or inconclusive variants cannot silently become defaults.
- Native/AOT selection requires their branch gate; unsupported targets remain excluded.
- Combined feature/dependency closure matches the documented selection and old releases/evidence remain distinguishable.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
python3 -m unittest discover -s tools/wtime-bench/tests -p 'test_p108*.py' -v
```

## Measurement and decision

Selection: `integration`. Explicit execution setting: `decoded`. Decision rule: `gate`.

Validate current selected artifacts and all stated checks, retaining unselected/rejected optional work explicitly. A missing aspiration is reported with follow-ups; semantic, integrity and hard-budget failures remain blockers. Read [MEASUREMENT.md](../MEASUREMENT.md).

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. No commands in this planning file have been executed as implementation evidence.
