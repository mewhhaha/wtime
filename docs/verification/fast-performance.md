# Fast performance iteration

The adopted schedule is in [the performance plan](../../../tasks/performance/VERIFICATION_FAST_PATH_PROPOSAL.md).
Use development checks while editing, a frozen selection for a candidate, and
the complete portfolio at an integration milestone. The default core stays
safe Rust with no external dependencies.

## Development

Activate Rust with `. /workspace/.setup/rust-env.sh`. Run the exact feature
group affected by the patch. Tooling changes have a quick shared group:

```sh
python3 -B tools/verify.py dev --contract docs/verification/tooling-development.json --group fast-verification --execute
```

`focused_compare.py --smoke` runs one pair per unique group, without calibration.
Its report is explicitly development evidence and cannot pass the candidate
gate. It uses an already built production binary and predecessor; it never
builds while timing. For example, the current P047 draft can be exercised with
the retained P046 binaries:

```sh
python3 -B tools/wtime-bench/focused_compare.py \
  --manifest docs/benchmarks/performance/P047-selection.json \
  --baseline target/verification/runs/P046/verify/attempt-0003/baselines/adapters/structural \
  --candidate target/verification/runs/P046/verify/attempt-0003/production/bin/wtime-bench \
  --candidate-mode compact --smoke --output NEW_EXCLUSIVE_DIRECTORY
```

## Candidate contracts

Declare targeted small/large rows, held-out rows, unaffected controls, and
semantic test names in a task-owned selection JSON. The P047 draft contains
nine rows and 29 logical groups. Identical preparation work reduces this to
25 actual groups, or 1,500 pairs across two runs. These are workload counts,
not a measured runtime speedup. The draft is not yet a frozen declaration or
evidence that P047 has been implemented.

Before editing runtime code, archive the immediate predecessor and freeze the
selection. The freezer checks actual archived and current core sources match:

```sh
python3 -B tools/wtime-bench/selection_manifest.py freeze \
  docs/benchmarks/performance/P047-selection.json \
  PREDECESSOR_ARCHIVE/manifest.json --output NEW_FREEZE_RECEIPT.json
```

After implementation, the task's shared `verify` phase must run every declared
semantic case against the current core and retain its ordinary full-lane
receipt. Include all transitive Rust sources, Cargo files, and fixtures in
that phase. Development or synthetic receipts cannot satisfy this requirement.
Register real task tests before using the P047 draft's named semantic cases.

The shared `measure` phase runs the focused producer:

```sh
python3 -B tools/wtime-bench/focused_compare.py \
  --manifest TASK_SELECTION.json --frozen FROZEN_SELECTION.json \
  --semantic-receipt VERIFY_ATTEMPT/receipt.json \
  --baseline PREDECESSOR_ADAPTER --candidate PRODUCTION_EXECUTABLE \
  --candidate-mode compact --output NEW_COMPARISON_DIRECTORY
```

The gate requires two complete runs of 30 pairs, exact declared groups,
production mode, independent oracles, shared warm batches that actually
reach 20 ms on both sides, and current source/binary identities. Use the
ordinary `tools/verify.py statistics` command with the selection JSON as
`--contract` and the task's default, optional-feature, and candidate build
records. It routes focused reports through `candidate_gate.py`, computes
10,000-resample statistics once, and preserves the existing build/dependency
ceilings. The initial implementation validates the existing Compact build
attestation; a new backend must supply its real feature/build attestation
before it can pass. `audit-statistics` recomputes without timing Wasm.

Prepare aliases bind actual Wasm bytes, feature profile, runtime settings,
timer sources, driver, and an explicit canonical preflight recipe. Each input
still receives its own correctness probe. Alias members reference the same
raw pairs and uncertainty; reported physical counts exclude duplicate
references. Instantiate, first-call, warm-call, and load-and-call never alias.

## Milestones and immutable reuse

P046 and P056 still require their complete scalar/memory/SIMD/representative
portfolios, P001 totals, full proposal/interaction correctness, cold controls,
and resource/build gates. Focused evidence cannot satisfy these milestones.
The legacy exhaustive comparator and historical report bytes are unchanged.

P046's `p046_measure.py` admits complete datasets to an immutable cache, then
reuses matches for the same execution inputs, modes, binaries, seeds, and
hardware. It revalidates every payload member and full workload matrix before
reuse. Partial checkpoints cannot enter the cache. Analysis-only edits cause
analysis to rerun; runtime, timer, oracle, fixture, or binary edits invalidate
the affected data.

The completed 25,500-pair P001/scalar report is admitted to
`target/verification/datasets/P046`. The explicit `p046_reuse.py` migration
rechecks the immutable successful P046 verification attempt, current core,
toolchain, fixtures, named outcomes, and archived payloads without rebuilding.
The updated P046 contract runs current gate tests during analysis. If core or
semantic inputs change, migration fails; use a fresh full verification
contract rather than changing the pinned historical receipt.

The previous P046 contract is retained as
`docs/benchmarks/performance/P046-verification-before-fast-path.json`.
No timing, status, or promotion decision is manufactured by migration.
