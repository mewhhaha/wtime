# P081 — Generate a small profile-selected superinstruction set

Status: **todo**

Outcome: **pending**

Kind: `experiment` · Branch: `optional` · Model: **Sol 6.1 Low**

## Prerequisites

[P002 — Compare separately built baseline and candidate binaries](../00-measurement/P002-paired-driver.md)

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P007 — Capture CPU profiles and rank actionable costs](../00-measurement/P007-profile-hotpaths.md)

[P008 — Add scaling, branch and call benchmark families](../00-measurement/P008-scalar-workloads.md)

[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)

[P016 — Add explicit executor policies and test integration hooks](../01-execution-contracts/P016-execution-policy.md)

[P019 — Define bounded optimizer effect and source expansion metadata](../01-execution-contracts/P019-effect-certificates.md)

[P020 — Centralize exact source instruction accounting](../01-execution-contracts/P020-exact-execution-accounting.md)

[P047 — Fuse a bounded set of local and constant arithmetic patterns](../04-specialization-and-fusion/P047-local-arithmetic-fusions.md)

[P048 — Fuse integer compare and conditional control transfers](../04-specialization-and-fusion/P048-compare-branch-fusions.md)

[P049 — Fuse pure address formation with a checked memory operation](../04-specialization-and-fusion/P049-address-memory-fusions.md)

[P056 — Select fusions and keep the portable hot path small](../04-specialization-and-fusion/P056-specialization-milestone.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `tools/wtime-superops/generate.py`
- `crates/wtime-runtime/src/execution/compact/generated_superops.rs`
- `crates/wtime-runtime/tests/performance/p081.rs`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

An offline stdlib generator emits at most 16 audited common nontrapping scalar patterns with bounded source recipes; production builds do not run a profiler or generate code dynamically.

## Implementation recipe

1. Choose patterns from recorded opcode n-grams across all declared families, freeze selection before held-out evaluation and exclude fixture names/hashes from runtime matching.
2. Generate safe Rust handlers plus source-expansion/accounting/materialization metadata from a small declarative template schema; reject unsupported effects and interior targets.
3. Cap handler count/source size and retain deterministic checked-in output hashes; no build.rs compiler dependency or combinatorial const-generic matrix.
4. Compare emitted versus hand-written fusions and held-out workloads, retaining build/preparation/code-size costs and at most one winning representation.
5. Forward the explicit `generated-superops` setting through the harness/policy integrator and record actual optimized/fallback counts. Persistent research selectors are nondefault; reversible changes to an existing tier use before/after artifacts and are selected only by their gate.

## Meaningful cases

- Regeneration is deterministic and invalid effect/source recipes fail.
- Each source prefix/fuel stop matches unfused execution.
- Held-out constants/programs work and the runtime has no fixture-identity recognizer.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime --features experimental-executors,execution-reference p081_ -- --nocapture
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml --features experimental-executors -j 4
python3 tools/wtime-bench/compare.py --baseline target/performance/P081-before --candidate tools/wtime-bench/target/release/wtime-bench --candidate-mode generated-superops --selection scalar --output target/performance/P081
python3 tools/wtime-bench/analyze.py --input target/performance/P081 --output target/performance/P081/analysis.json
python3 tools/checks/build_budget.py --output target/performance/P081/build.json
python3 tools/checks/build_budget.py --features wtime-runtime/experimental-executors --output target/performance/P081/optional-build.json
python3 tools/wtime-bench/gates.py --task P081 --comparison target/performance/P081 --build target/performance/P081/build.json --optional-build target/performance/P081/optional-build.json --output target/performance/P081/decision.json
```

## Measurement and decision

Selection: `scalar`. Explicit execution setting: `generated-superops`. Decision rule: `measured_promotion`.

Use the paired protocol and family/control/cold/build rules in [MEASUREMENT.md](../MEASUREMENT.md). Predeclare the targeted family and held-out controls before changing the implementation. Keep instrumentation off in production timings. Adopt only with supported gains in both runs; otherwise record rejected/inconclusive and remove default-path cost. A necessary immutable-interface change may be contract_only with its separate justification.

## Before editing

Run these commands before changing the candidate sources. The archive is immutable and captures the immediate predecessor. Retain P001's original archive for milestone totals. For native work, use the named portable control and verify its actual policy/quantum; if this archive already exists with different sources, choose a new recorded run-specific directory and update the comparison path rather than overwriting it.

```sh
python3 tools/wtime-bench/baseline.py --output target/performance/P081-before
```

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. No commands in this planning file have been executed as implementation evidence.
