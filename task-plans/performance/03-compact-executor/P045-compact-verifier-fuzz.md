# P045 — Verify and fuzz prepared compact programs

Status: **done**

Outcome: **contract_only**

Kind: `gate` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)

[P019 — Define bounded optimizer effect and source expansion metadata](../01-execution-contracts/P019-effect-certificates.md)

[P037 — Map compact instructions to source events and control targets](P037-compact-source-maps.md)

[P040 — Lower structured control to resolved jump and result plans](P040-compact-control-plans.md)

[P041 — Resolve direct guest call bindings once per instance](P041-instance-call-plans.md)

[P043 — Resolve memory bindings and specialize checked scalar accesses](P043-compact-memory-plans.md)

[P044 — Materialize compact state for GC, exceptions and continuations](P044-compact-proposal-boundaries.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C5, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/execution/compact/lower_scalar.rs`
- `crates/wtime-runtime/src/execution/compact/verify.rs`
- `crates/wtime-runtime/src/execution/compact/prefix.rs`
- `crates/wtime-runtime/src/execution/compact/source_map.rs`
- `crates/wtime-runtime/src/execution/compact/lower_control.rs`
- `crates/wtime-runtime/src/execution/compact/memory.rs`
- `crates/wtime-runtime/tests/performance/p045.rs`
- `crates/wtime-runtime/tests/performance/p045_private.rs`
- `crates/wtime-runtime/tests/performance.rs`
- `tools/wtime-bench/p045_corpus.py`
- `tools/wtime-bench/tests/test_p045_compact_fuzz.py`
- `tools/wtime-bench/tests/test_p045_tier_mapping.py`
- `tools/wtime-bench/tier_check.py`
- `tools/wtime-conformance/Cargo.toml`
- `tools/wtime-conformance/src/cli.rs`
- `tools/wtime-conformance/src/bin/wtime-probe.rs`
- `tools/wtime-bench/profiling/Cargo.toml`
- `tools/wtime-bench/src/tier_state_driver.rs`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

The private compact verifier checks tags/side indices/targets/origins/effects/slot bounds before publication; generated valid Wasm and deliberately corrupted internal programs cannot reach unchecked execution.

## Implementation recipe

1. Implement a bounded verifier over fixed instructions and side tables, accepting only the sealed provenance and legal source event/branch expansion from lowering.
2. Add test-only corruption constructors for wrong tags, wide side indices, mid-expansion targets, missing materialization and overflow; keep these constructors out of production public APIs.
3. Generate deterministic valid scalar/control/memory cases and compare compact/reference outcomes/state/fuel; use external oracles for supported semantics.
4. Inject lowering/verifier allocation/work failures and prove decoded fallback remains usable with no partially published artifacts.

## Meaningful cases

- Each malformed internal table/target/effect is rejected before execution.
- At least 1000 recorded generated cases have replay seeds and nonzero compact coverage.
- Budget/allocation failures and feature-disabled inputs cannot waive validation or publish invalid code.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime --features compact-executor,execution-reference p045_ -- --nocapture
python3 -m unittest discover -s tools/wtime-bench/tests -p 'test_p045*.py' -v
python3 tools/wtime-bench/tier_check.py --tier compact --selection integration --output target/performance/P045/conformance
```

## Measurement and decision

Selection: `integration`. Explicit execution setting: `compact`. Decision rule: `gate`.

Validate current selected artifacts and all stated checks, retaining unselected/rejected optional work explicitly. A missing aspiration is reported with follow-ups; semantic, integrity and hard-budget failures remain blockers. Read [MEASUREMENT.md](../MEASUREMENT.md).

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. Executed evidence is recorded below.

## Completion details

The bounded runtime patch adds the private verifier and moves aggregate map, control-plan and memory-plan validation before compact publication. Its 100,000-unit work cap and 1 MiB retained/peak cap include declaration maps, already-retained functions and verification scratch. Test-only lowering/verifier fault injection returns safely without publishing partial compact state, and decoded fallback remains usable. The registered P045 runtime cases, Python corpus/tier mappings and conformance CLI/profiler feature paths are recorded in the result document. Compact remains opt-in; Decoded remains the default. This gate makes no Wasm performance claim and does not promote Compact.

The archived freeze and verify receipts bind the exact source, fixture, binary and helper identities. Verification completed with 340 runtime tests (including six P045-named cases), 10 Python tests, four conformance CLI tests, 1,000 unique seeded modules (834 returned, 166 traps and 166 full-memory checks under Decoded, Reference and Compact), and 89/89 tier checks. The tier receipt records 68 fuel probes and four compact/reference conformance runs across `reference_types` and `interactions`; all had zero failures, skips or unsupported cases. Source-family counts were nonzero for scalar, local, if, branch, memory and trap.

Attempt 0001 and the contract/preflight drafts remain retained, including its 39-check tier-check failure. The P001 frozen baseline remains verified. Full command, source and artifact identities are in [P045-result.json](../../../wtime/docs/benchmarks/performance/P045-result.json); authoritative proof: `/workspace/wtime/target/performance/P045/shared-finalization/attempt-0002/evidence.json` (SHA-256 `46c316a1b1298940a22a2e36e519b5fa806be8a26b4791071ed24ad843adf2ee`).

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/P045/shared-finalization/attempt-0002/evidence.json": "46c316a1b1298940a22a2e36e519b5fa806be8a26b4791071ed24ad843adf2ee"
  },
  "evidence": [
    {
      "bytes": 1460,
      "path": "/workspace/wtime/target/performance/P045/shared-finalization/attempt-0002/evidence.json",
      "sha256": "46c316a1b1298940a22a2e36e519b5fa806be8a26b4791071ed24ad843adf2ee"
    }
  ],
  "outcome": "contract_only",
  "status": "done",
  "task": "P045"
}
```
