# P046 — Decide whether compact execution earns default use

Status: **done**

Outcome: **rejected**

Kind: `gate` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)

[P015 — Add reproducible application-shaped held-out kernels](../00-measurement/P015-representative-kernels.md)

[P035 — Evaluate structural wins and choose the next bottleneck](../02-structural-wins/P035-structural-milestone.md)

[P038 — Lower constants, locals and common integer operations](P038-compact-scalar-lowering.md)

[P039 — Lower float/vector constants and barrier adapters](P039-compact-float-vector-adapters.md)

[P040 — Lower structured control to resolved jump and result plans](P040-compact-control-plans.md)

[P041 — Resolve direct guest call bindings once per instance](P041-instance-call-plans.md)

[P042 — Add guarded bounded indirect and call_ref caches](P042-indirect-call-inline-cache.md)

[P043 — Resolve memory bindings and specialize checked scalar accesses](P043-compact-memory-plans.md)

[P044 — Materialize compact state for GC, exceptions and continuations](P044-compact-proposal-boundaries.md)

[P045 — Verify and fuzz prepared compact programs](P045-compact-verifier-fuzz.md)

[P121 — Record complete Compact conformance and preparation coverage](P121-compact-full-conformance-counts.md)

[P122 — Prove Compact representative timing keeps the source-bound guest boundary](P122-compact-representative-protocol.md)

[P123 — Adapt supported typed workload routes to immutable P001 sources](P123-p001-typed-harness-adapter.md)

[P124 — Validate the complete Compact portfolio gate producer](P124-compact-four-family-portfolio-gate.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C5, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `docs/benchmarks/performance/compact.md`
- `tools/wtime-bench/tests/test_p046_compact_gate.py`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

A compact-tier decision contains complete supported/fallback coverage, two paired runs, full pinned corpus/interaction evidence and build/cold/retained-code gates; default selection changes only after this decision.

## Implementation recipe

0. Complete P121–P124 tooling foundations first: full Compact corpus/preparation counts, actual-mode representative timer, frozen-P001 adapter capability proof, and one portfolio producer. Reuse source-current foundation evidence; the parent retains every original acceptance requirement.

   The adopted [fast verification flow](../../../wtime/docs/verification/fast-performance.md)
   revalidates the successful P046 semantic/build attempt and caches complete
   timing datasets. The live `P046-verification.json` moves gate tests into
   analysis so statistics-only changes do not repeat builds or Wasm timings.
   The complete four-family/two-baseline matrix remains mandatory; the old
   exhaustive contract and every partial/complete report are preserved.

1. Run full pinned MVP/Core3/Phase3/interactions through compact selection and preserve executed optimized/fallback source counts; full semantic support can use explicit decoded fallback.
2. Compare predeclared scalar/memory/SIMD/representative families against archived and structural baselines without diagnostic features.
3. Measure preparation/first-use, all build budgets, retained module/code bytes and dependency closure; inspect any 1.25 relative build/cold growth.
4. Promote compact only if MEASUREMENT rules pass; otherwise keep it opt-in, state the limiting costs and file small follow-ups without modifying historical reports.

## Meaningful cases

- No failure/skip/unsupported directive is hidden by fallback.
- A tier with zero optimized coverage cannot pass as a compact win.
- Cold/default/build regression or inconclusive controls prevent unconditional promotion.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
python3 -m unittest discover -s tools/wtime-bench/tests -p 'test_p046*.py' -v
python3 tools/wtime-bench/tier_check.py --tier compact --selection integration --output target/performance/P046/conformance
```

## Measurement and decision

Selection: `integration`. Explicit execution setting: `compact`. Decision rule: `gate`.

Validate current selected artifacts and all stated checks, retaining unselected/rejected optional work explicitly. A missing aspiration is reported with follow-ups; semantic, integrity and hard-budget failures remain blockers. Read [MEASUREMENT.md](../MEASUREMENT.md).

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. Executed evidence is recorded below.

## Published result

P046 completed with a rejected default-promotion decision. The complete paired matrix contains 131,400 observations (65,700 per baseline) plus 2,520 cold-control pairs across two runs. Compact passed the 145,760-case supported/fallback conformance corpus, default and optional build budgets, and zero-dependency-violation check. The primary scalar/memory/SIMD family ratios were 0.86678789/0.79659666, 0.81608416/0.81643069, and 0.68761976/0.69060426 across the two runs, but protected warm and cold-control regressions rejected default adoption. `Engine::new(Config)` remains decoded; Compact remains opt-in behind `compact-executor` and `Engine::with_execution_policy`. The full result, API example, artifact paths, and limitations are documented in [compact.md](../../../wtime/docs/benchmarks/performance/compact.md). The verified timing datasets were reused after analysis fixes; historical raw data, unsuccessful attempts, and aggregates remain intact.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/P046/shared-finalization/attempt-0007/evidence.json": "e42f27d67abbe495c6a83c5b67e6620f7e60eb0437fc34507922aa6b31eddc3f"
  },
  "evidence": [
    {
      "bytes": 2354,
      "path": "/workspace/wtime/target/performance/P046/shared-finalization/attempt-0007/evidence.json",
      "sha256": "e42f27d67abbe495c6a83c5b67e6620f7e60eb0437fc34507922aa6b31eddc3f"
    }
  ],
  "outcome": "rejected",
  "status": "done",
  "task": "P046"
}
```
