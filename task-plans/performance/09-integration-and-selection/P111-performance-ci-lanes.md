# P111 — Add honest smoke, regression and reference-machine lanes

Status: **done**

Outcome: **contract_only**

Kind: `gate` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P003 — Define workload selections and engine capability preflight](../00-measurement/P003-workload-schema.md)

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)

[P108 — Choose defaults, opt-in tiers and retired experiments](P108-select-performance-portfolio.md)

[P109 — Refresh full conformance and hardening for selected tiers](P109-selected-tier-conformance.md)

[P110 — Verify the final portfolio stays small and quick to prepare](P110-final-cold-and-build-budget.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `tools/checks/performance_ci.py`
- `tools/checks/tests/test_performance_ci.py`
- `docs/benchmarks/performance/ci.md`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

Ordinary CI checks correctness/report integrity with a small smoke; paired performance promotion runs on a declared reference lane. Noisy shared workers emit trends/inconclusive outcomes rather than false speed guarantees.

## Implementation recipe

1. Define smoke (nonzero execution/oracle/feature settings), selected-tier conformance, default build/dependency and scheduled reference performance lanes with exact commands/artifact schemas.
2. Pin comparator/toolchain/fixtures, preserve baseline/candidate identities and raw paired observations; do not rebuild baseline from the candidate checkout.
3. Fail integrity/semantic/absolute budget errors everywhere, but require the declared stable reference protocol for relative speed regressions and promotion decisions.
4. Store trend/decision artifacts with retention limits and clear unsupported-target labels; keep Wasmtime/toolchain/native/AOT dependencies outside the default Rust build lane.

## Meaningful cases

- Zero-work, corrupted oracle or stale hash smoke fails.
- A noisy trend cannot claim a passed promotion gate.
- An unavailable comparator/architecture is explicitly unrun and default CI still checks the portable runtime.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
python3 -m unittest discover -s tools/checks/tests -p 'test_p111*.py' -v
python3 tools/checks/performance_ci.py --lane smoke --output target/performance/P111/smoke.json
```

## Measurement and decision

Selection: `integration`. Explicit execution setting: `decoded`. Decision rule: `gate`.

Validate current selected artifacts and all stated checks, retaining unselected/rejected optional work explicitly. A missing aspiration is reported with follow-ups; semantic, integrity and hard-budget failures remain blockers. Read [MEASUREMENT.md](../MEASUREMENT.md).

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. Executed evidence is recorded below.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/P111/shared-finalization/attempt-0001/evidence.json": "2a1a113b5c36d0881987dd7da22821562f9e1d6ca1d09d083bc3982e1f21c449"
  },
  "evidence": [
    {
      "bytes": 1779,
      "path": "/workspace/wtime/target/performance/P111/shared-finalization/attempt-0001/evidence.json",
      "sha256": "2a1a113b5c36d0881987dd7da22821562f9e1d6ca1d09d083bc3982e1f21c449"
    }
  ],
  "outcome": "contract_only",
  "status": "done",
  "task": "P111"
}
```
