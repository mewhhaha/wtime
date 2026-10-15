# P109 — Refresh full conformance and hardening for selected tiers

Status: **done**

Outcome: **contract_only**

Kind: `gate` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)

[P022 — Add adversarial state and interaction tier fixtures](../01-execution-contracts/P022-state-differential-fixtures.md)

[P108 — Choose defaults, opt-in tiers and retired experiments](P108-select-performance-portfolio.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C5, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `docs/verification/performance/*`
- `tools/wtime-bench/tests/test_p109_selected_conformance.py`
- `tools/checks/opcode_coverage.py`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

Each supported selected tier/profile has fresh current-source normative and interaction evidence, executed optimized/fallback counts, source-op coverage and hardening results. Original missing AArch64 evidence remains separately visible.

## Implementation recipe

1. Run pinned MVP/Core3/Phase3/interactions and existing feature-disabled/CLI/API suites for default and each supported optional tier; refresh evidence in new selected-tier paths.
2. Run exact source-fuel/cancel/trap/root/cross-module/suspension/shared-SC/atomic-wait and allocation-failure regressions plus deterministic fuzz/differential seeds appropriate to changed lowering.
3. Bind coverage to original source opcodes even when compact/native code fuses/elides operations; do not accept fallback-only counts as optimized-family coverage.
4. Validate current source/binary/spec identities and strict ledger ownership, retaining failures/skips/unsupported and not rewriting old reports as new evidence.

## Meaningful cases

- Zero cases or stale current-source hashes fail the gate.
- Each selected optimized family has nonzero optimized coverage and feature-disabled rejection.
- Any semantic/hardening failure remains a blocker independently of performance.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
python3 -m unittest discover -s tools/wtime-bench/tests -p 'test_p109*.py' -v
python3 tools/wtime-bench/tier_check.py --tier decoded --selection integration --output target/performance/P109/conformance
python3 tools/wtime-bench/tier_check.py --tier selected --selection integration --output docs/verification/performance
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
    "/workspace/wtime/target/performance/P109/shared-finalization/attempt-0001/evidence.json": "a004e37124c6807c3bb3427e722923cd83032476ac3af7991328a9b2abaf90ed"
  },
  "evidence": [
    {
      "bytes": 1925,
      "path": "/workspace/wtime/target/performance/P109/shared-finalization/attempt-0001/evidence.json",
      "sha256": "a004e37124c6807c3bb3427e722923cd83032476ac3af7991328a9b2abaf90ed"
    }
  ],
  "outcome": "contract_only",
  "status": "done",
  "task": "P109"
}
```
