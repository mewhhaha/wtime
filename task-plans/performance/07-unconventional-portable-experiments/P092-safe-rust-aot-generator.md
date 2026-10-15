# P092 — Generate safe numeric Rust kernels from verified programs

Status: **deferred**

Outcome: **not_selected**

Kind: `experiment` · Branch: `optional` · Model: **Sol 6.1 Low**

## Prerequisites

[P002 — Compare separately built baseline and candidate binaries](../00-measurement/P002-paired-driver.md)

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P008 — Add scaling, branch and call benchmark families](../00-measurement/P008-scalar-workloads.md)

[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)

[P016 — Add explicit executor policies and test integration hooks](../01-execution-contracts/P016-execution-policy.md)

[P020 — Centralize exact source instruction accounting](../01-execution-contracts/P020-exact-execution-accounting.md)

[P061 — Execute checked integer register operations](../05-register-executor/P061-register-integer-execution.md)

[P062 — Add strict floating-point register operations](../05-register-executor/P062-register-float-execution.md)

[P091 — Export a verified numeric program for an offline Rust AOT tool](P091-safe-aot-export-contract.md)

Conditional branch: `P091.outcome == go`. If its decision is declined, record this task as deferred/not_selected; do not claim it implemented.

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `tools/wtime-aot/src/generate.rs`
- `tools/wtime-aot/tests/generation.rs`
- `tools/wtime-aot/templates/*`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

Generated Rust uses initialized typed slots, wrapping/exact helpers and an iterative control state machine, with original source accounting; it requires normal rustc build time explicitly reported as offline AOT cost.

## Implementation recipe

1. Emit numeric constants/operations and structured jumps from the verified view using numeric internal identifiers, bounded output size and deterministic formatting.
2. Generate exact source accounting/trap/limit/cancellation exits and materialization state; no recursion for guest loops or unsafe indexing/fast-math.
3. Compile generated small kernels in a temporary standalone crate, run independent/original-runtime differential inputs and retain source/binary/toolchain hashes.
4. Measure generator+rustc bootstrap/code size and execution separately, including what an application must ship/load to use the artifact.
5. Forward the explicit `aot` setting through the harness/policy integrator and record actual optimized/fallback counts. Persistent research selectors are nondefault; reversible changes to an existing tier use before/after artifacts and are selected only by their gate.

## Meaningful cases

- Regeneration is deterministic and generated source contains no unsafe or guest text injection.
- Loop/division/float boundary/fuel prefixes match source semantics.
- Large output/work limits decline safely and generated compilation failure preserves portable execution.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked --manifest-path tools/wtime-aot/Cargo.toml -- --nocapture
python3 tools/wtime-bench/tier_check.py --tier aot --selection scalar --output target/performance/P092/conformance
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml --features experimental-executors -j 4
python3 tools/wtime-bench/compare.py --baseline target/performance/P092-before --candidate tools/wtime-bench/target/release/wtime-bench --candidate-mode aot --selection scalar --output target/performance/P092
python3 tools/wtime-bench/analyze.py --input target/performance/P092 --output target/performance/P092/analysis.json
python3 tools/checks/build_budget.py --output target/performance/P092/build.json
python3 tools/wtime-bench/gates.py --task P092 --comparison target/performance/P092 --build target/performance/P092/build.json --output target/performance/P092/decision.json
```

## Measurement and decision

Selection: `scalar`. Explicit execution setting: `aot`. Decision rule: `measured_promotion`.

Use the paired protocol and family/control/cold/build rules in [MEASUREMENT.md](../MEASUREMENT.md). Predeclare the targeted family and held-out controls before changing the implementation. Keep instrumentation off in production timings. Adopt only with supported gains in both runs; otherwise record rejected/inconclusive and remove default-path cost. A necessary immutable-interface change may be contract_only with its separate justification.

## Before editing

Run these commands before changing the candidate sources. The archive is immutable and captures the immediate predecessor. Retain P001's original archive for milestone totals. For native work, use the named portable control and verify its actual policy/quantum; if this archive already exists with different sources, choose a new recorded run-specific directory and update the comparison path rather than overwriting it.

```sh
python3 tools/wtime-bench/baseline.py --output target/performance/P092-before
```

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. Executed evidence is recorded below.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/final-optional-dispositions-0002/proof.json": "5a2943b1b7f0e4fedf44a2d4e96153937683fc97d45e079e55c0ef43a5e3b97f"
  },
  "evidence": [
    {
      "bytes": 38990,
      "path": "/workspace/wtime/target/performance/final-optional-dispositions-0002/proof.json",
      "sha256": "5a2943b1b7f0e4fedf44a2d4e96153937683fc97d45e079e55c0ef43a5e3b97f"
    }
  ],
  "implementation_claimed": false,
  "outcome": "not_selected",
  "reason": "Conditional research is not selected for the current portfolio; this record claims no implementation, test execution, or speed result.",
  "schema": "wtime-task-disposition-record-v1",
  "status": "deferred",
  "task": "P092",
  "tests_claimed": 0,
  "timing_evidence": false
}
```
