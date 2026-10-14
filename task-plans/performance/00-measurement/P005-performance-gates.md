# P005 — Bind speed decisions to build and dependency budgets

Status: **done**

Outcome: **complete**

Kind: `foundation` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P001 — Archive the current runtime and benchmark baseline](P001-freeze-baseline.md)

[P004 — Compute uncertainty and preparation break-even](P004-paired-statistics.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `tools/wtime-bench/gates.py`
- `tools/wtime-bench/tests/test_p005_gates.py`
- `tools/checks/build_budget.py`
- `tools/checks/dependency_budget.py`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

gates.py --task PNNN --comparison DIR --build FILE --output FILE returns a source-bound adopted/rejected/inconclusive/contract_only decision; it checks the existing hard ceilings and new relative controls without modifying historical evidence. build_budget.py also accepts --features PACKAGE/FEATURE for a separately labeled optional build; its no-feature run remains the default production budget. gates.py accepts --optional-build FILE when the candidate requires nondefault compiled features, checking that report separately from --build.

## Implementation recipe

1. Read current build/dependency report schemas and record their source identities; keep clean<=60s, CLI leaf<=5s, stripped<=5MiB and zero external default dependencies.
2. Extend build measurement to retain mode/features, all compilation inputs, runtime-leaf timing and retained-code/RSS fields where measurable; use a temporary target and restore touched files on all exits.
3. Implement warm/control/preparation/first-use decisions from MEASUREMENT.md; compare matched source/config reports and flag relative build/size/RSS growth above 1.25 for a documented decision.
4. Inventory optional features separately while proving default dependency closure and forbid-unsafe boundaries; do not loosen the current unsafe scanner until the explicit native policy task lands.
5. Add --features PACKAGE/FEATURE with strict package/feature validation for optional build measurements, record its complete resolved graph/settings and keep default reports separately labeled. Never apply optional features to the default dependency gate.
6. Have gates.py emit its complete decision as JSON and validate both run statistics, predeclared family controls, current source identity and the correct default/optional build report. A missing field/report is an error, not an assumed pass.

## Meaningful cases

- A stale report or missing dependency closure fails even if timing is faster.
- A candidate that wins warm code but exceeds default cold/build limits is not adopted by default.
- An inconclusive interval stays inconclusive; source restoration is checked after an injected build failure.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
python3 -m unittest discover -s tools/wtime-bench/tests -p 'test_p005*.py' -v
python3 tools/checks/build_budget.py --output target/performance/P005-build.json
python3 tools/checks/dependency_budget.py
```

## Measurement and decision

Selection: `scalar`. Explicit execution setting: `decoded`. Decision rule: `integrity`.

This is a tooling/contract deliverable. Require correct nonzero checks and bounded/source-bound output; do not invent a runtime speedup requirement. If it changes production work, record cold/build/retention deltas under [MEASUREMENT.md](../MEASUREMENT.md).

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. Executed evidence is recorded below.

## Executed evidence

```json
{
  "artifacts": [
    "/workspace/wtime/target/performance/P005-build.json",
    "/workspace/wtime/target/performance/P005-build-with-runtime-leaf.json",
    "/workspace/wtime/target/performance/P005-optional-build.json"
  ],
  "commands": [
    {
      "command": "python3 -m unittest discover -s tools/wtime-bench/tests -p test_p005*.py -v",
      "exit_code": 0,
      "passed": 5,
      "failed": 0,
      "skipped": 0
    },
    {
      "command": "python3 tools/checks/build_budget.py --output target/performance/P005-build-with-runtime-leaf.json",
      "exit_code": 0,
      "budget_pass": true
    },
    {
      "command": "python3 tools/checks/build_budget.py --features wtime-runtime/opcode-coverage --output target/performance/P005-optional-build.json",
      "exit_code": 0,
      "budget_pass": true
    },
    {
      "command": "python3 tools/checks/dependency_budget.py",
      "exit_code": 0,
      "production_violations": 0
    }
  ],
  "hashes": {
    "P005-build.json": "4e7a30160dd3b9f5944704d076f0a7d85554a64e2301cb32312501ab6cb314cf",
    "P005-build-with-runtime-leaf.json": "72d7dfaf925cd6e954bf329351dc824c07dcd99a88bfec72ac365b8c77e96bf3",
    "P005-optional-build.json": "1233b881d739f47ba99511401099f766b83a463c766975c4d16af4531f8941d1"
  },
  "limitations": [
    "No optimization promotion is claimed. The gate rejects stale/missing comparison or build evidence.",
    "Repeated clean timings ranged from 6.54 to 8.78 seconds at the same source hash and stripped size; relative timing growth remains explicit shared-cloud uncertainty.",
    "Default and opcode-coverage build profiles are measured and labeled separately."
  ]
}
```
