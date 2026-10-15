# P094 — Decide the tiny native backend boundary and budget

Status: **done**

Outcome: **no_go**

Kind: `experiment` · Branch: `optional` · Model: **Sol 6.1 Low**

## Prerequisites

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P018 — Define source cursors and materialized guest state](../01-execution-contracts/P018-source-cursor.md)

[P019 — Define bounded optimizer effect and source expansion metadata](../01-execution-contracts/P019-effect-certificates.md)

[P020 — Centralize exact source instruction accounting](../01-execution-contracts/P020-exact-execution-accounting.md)

[P067 — Evaluate register execution and adaptive break-even](../05-register-executor/P067-register-milestone.md)

[P080 — Evaluate GC, SIMD, continuation and shared-memory progress](../06-memory-gc-simd-and-continuations/P080-specialist-milestone.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `docs/architecture/performance-native-adr.md`
- `tools/wtime-bench/tests/test_p094_native_policy.py`
- `tools/checks/dependency_budget.py`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

A go/no_go ADR defines an optional numeric native tier, x86_64 Linux initial support, strict unsafe/W^X/ABI/code budgets and a cycle-free dependency graph. A no_go defers native children instead of marking them implemented.

## Implementation recipe

1. Compare measured remaining portable costs with native bootstrap/safety/maintenance cost; declare the initial no-ref/no-host/no-native-guest-stack subset and portable fallback.
2. Permit at most two additional internal crates: zero-dependency wtime-exec-ir and opt-in wtime-native -> exec-ir; runtime native feature may depend on both, neither depends on runtime, and default-members/build closure stay unchanged.
3. Keep existing core forbid-unsafe and zero external deps; native prefers zero external deps with at most one explicitly justified platform dependency. Update scanner rules narrowly to inventory only the named optional boundary.
4. Specify separate native safety review/codegen evidence, W^X mappings, ABI/lifetime/stack alignment, bounded code/work and supported-target probes; record go/no_go without pretending these implementation checks already passed.

## Meaningful cases

- Default metadata/tree still excludes native and any external dependency.
- Unsafe code outside the declared native paths is still rejected.
- A no_go clearly leaves every native child deferred and produces no unsupported speed claim.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
python3 -m unittest discover -s tools/wtime-bench/tests -p 'test_p094*.py' -v
```

## Measurement and decision

Selection: `integration`. Explicit execution setting: `decoded`. Decision rule: `design_go_no_go`.

Complete the design/integrity checks and a concrete cost/scope decision artifact. Record go or no_go; this task establishes a conditional interface, not a measured speedup. Descendants remain unimplemented if no_go.

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. Executed evidence is recorded below.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/P094/shared-finalization/attempt-0002/evidence.json": "9e0941ab76ac73bd228d38024707efd199fa3b723289f841dc49433b2e5680d0"
  },
  "evidence": [
    {
      "bytes": 2089,
      "path": "/workspace/wtime/target/performance/P094/shared-finalization/attempt-0002/evidence.json",
      "sha256": "9e0941ab76ac73bd228d38024707efd199fa3b723289f841dc49433b2e5680d0"
    }
  ],
  "outcome": "no_go",
  "status": "done",
  "task": "P094"
}
```
