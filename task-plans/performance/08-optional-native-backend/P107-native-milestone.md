# P107 — Audit selected native paths and their real crossover

Status: **todo**

Outcome: **pending**

Kind: `gate` · Branch: `optional` · Model: **Sol 6.1 Low**

## Prerequisites

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)

[P015 — Add reproducible application-shaped held-out kernels](../00-measurement/P015-representative-kernels.md)

[P094 — Decide the tiny native backend boundary and budget](P094-native-boundary-decision.md)

[P095 — Create the minimal optional numeric backend interface](P095-native-ir-and-abi.md)

[P096 — Implement one audited W^X mapping owner for x86 Linux](P096-executable-mapping-owner.md)

[P097 — Emit a tiny verified x86_64 integer block subset](P097-x86-integer-block-emitter.md)

[P098 — Add checked branches and bounded native numeric loops](P098-x86-control-emitter.md)

[P099 — Add exact fuel, traps and bounded cancellation to native execution](P099-native-accounting-exits.md)

[P100 — Compile and retain native code only for eligible hot functions](P100-bounded-native-tiering.md)

Optional results to review (do not block the portable path):

[P101 — Cross native memory boundaries through safe checked callouts](P101-native-memory-callouts.md)

[P102 — Try audited native checked access to a borrowed unshared view](P102-native-inline-unshared-memory.md)

[P103 — Extend native numeric code with checked traps and strict floats](P103-native-strict-numerics.md)

[P104 — Isolate a tiny feature-detected native SIMD kernel set](P104-optional-native-simd.md)

[P105 — Prototype reproducible checked copy-and-patch templates](P105-copy-and-patch-alternative.md)

[P106 — Prototype the same tiny native subset on AArch64](P106-aarch64-native-prototype.md)

Conditional branch: `P094.outcome == go`. If its decision is declined, record this task as deferred/not_selected; do not claim it implemented.

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C5, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `docs/benchmarks/performance/native.md`
- `docs/benchmarks/performance/native-safety.json`
- `tools/wtime-bench/tests/test_p107_native_gate.py`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

The native gate covers only selected implemented targets/subsets, binds W^X/ABI/code/lifetime/adversarial evidence and compares matched-policy portable/Wasmtime totals. Unsupported architectures/extensions remain explicitly unsupported.

## Implementation recipe

1. Audit all unsafe blocks/safe wrapper preconditions and generated instruction/relocation bounds; run native differential/fuel/trap/cancel/lifetime/drop/eviction cases and available sanitizer tooling.
2. Run pinned full corpus through native selection plus explicit portable fallback, proving nonzero eligible native execution and correct GC/exception/continuation exits.
3. Collect two paired runs with matched cancellation/fuel settings, native emission/protection spikes, retained mappings, cold/startup/build/dependency costs and measured break-even.
4. Evaluate selected extensions P101-P106 only if implemented, name actual supported targets and the 2-10x Wasmtime aspiration; leave native opt-in or reject it if evidence does not justify complexity.

## Meaningful cases

- A stale/unsupported-target or zero-native-coverage report fails.
- W^X, active mapping lifetime and source-accounting failures block selection despite timing wins.
- Missing AArch64 or unselected copy-and-patch/SIMD extensions are not claimed supported.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
python3 -m unittest discover -s tools/wtime-bench/tests -p 'test_p107*.py' -v
python3 tools/wtime-bench/tier_check.py --tier native --selection integration --output target/performance/P107/conformance
```

## Measurement and decision

Selection: `integration`. Explicit execution setting: `native`. Decision rule: `gate`.

Validate current selected artifacts and all stated checks, retaining unselected/rejected optional work explicitly. A missing aspiration is reported with follow-ups; semantic, integrity and hard-budget failures remain blockers. Read [MEASUREMENT.md](../MEASUREMENT.md).

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. No commands in this planning file have been executed as implementation evidence.
