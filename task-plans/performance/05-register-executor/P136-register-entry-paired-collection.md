# P136 — Bind Register entry settings to source-bound paired collection

Status: **done**

Outcome: **contract_only**

Kind: `foundation` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P135 — Add a fresh-entry Register batch protocol](P135-fresh-entry-batch-protocol.md)

P135 supplies a current-only Register batch command with exact fresh Engine/Module/Store lifetimes and raw total/phase nanoseconds. P136 adds a source-bound adapter for an immutable archived Register core and pairs it with a distinct uninstrumented current-core candidate executable built from the same minimal P135 harness. Here, “candidate production executable” means that current-core harness build produced by `build_current` with `source_origin: current-workspace`; it does not mean the usual `tools/wtime-bench` package, whose unconditional Wasmtime dependency is excluded from this Register-only comparison. Both sides use the same generated entry point, P135 protocol/driver/leaf overlay, manifest, and Register/runtime-only feature configuration; their verified core source maps and origins differ. P136 records raw paired observations only. P137 owns total-cost, cold-code and cache-churn analysis; P066 owns the declared candidate selection and the complete performance decision gate.

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), and [MEASUREMENT.md](../MEASUREMENT.md), plus contracts **C1, C2, C3, C4, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md). Inspect `tools/wtime-bench/src/entry_batch_driver.rs`, `protocol.rs`, `baseline.py`, `compare.py`, `checkpoint.py`, and the relevant current Register archive before implementation.

- `tools/wtime-bench/entry_batch_adapter.py` (new source-copy, build and adapter verification)
- `tools/wtime-bench/entry_batch_compare.py` (new raw paired collector)
- `tools/wtime-bench/tests/test_entry_batch_adapter.py` (adapter source/build identity checks)
- `tools/wtime-bench/tests/test_p136_entry_batch.py` (new focused collector and receipt checks)
- `tools/verification/producers/p136.py` (new source-bound freeze and smoke verification)
- `tools/verification/tests/test_p136_entry_batch.py` (new producer checks)
- `docs/benchmarks/performance/P136-contract.json` (short declarative acceptance contract)
- `docs/benchmarks/performance/P136-verification.json` (source-bound shared verification workflow and receipt contract)

Keep P136 limited to adapter construction, paired collection, source-bound verification and tests. Do not change runtime sources, P135 command semantics, workload registries, `baseline.py`'s decoded legacy-adapter identity, `compare.py`'s existing observation schema, P137 analysis, or P066's candidate policy. One integrator owns the archive adapter and collector. Use at most two focused implementation patches: source-copy/build/identity verification, then paired collection/receipt tests.

## Input/output contract

The adapter builder accepts a generic verified immutable Register-mode archive through `--core-archive`; it requires the archived core to expose the P134 eager and positive-threshold Register constructors. The concrete P136 workflow freezes its immediate predecessor at `target/performance/P136-before` from the P135-complete source tree. P135-before is historical context only and is not pinned into the builder. Verify the archive manifest and every source hash before copying. Snapshot and hash the complete archived source-role workspace closure and current workspace closure, including member manifests and configuration needed to preserve Cargo workspace membership (including CLI and conformance manifests). The minimal adapter compiles only the four core crates needed by its runtime dependency closure (`wtime-runtime`, `wtime-types`, `wtime-decode`, `wtime-validate`); it adds no third-party dependency. Refresh copied Rust source modification times before Cargo. Never write to the archive. Overlay the current P135 `protocol.rs`, `entry_batch_driver.rs`, and every required protocol leaf by exact bytes and record their paths and hashes. Refuse archived sources that do not compile with the required P134 APIs or do not have Register support.

Build a tiny binary entry point that calls the shared protocol dispatcher. Generate a standalone workspace manifest with only the archived runtime crate paths and required features. Enable `register-executor` and `runtime-only`, keep execution instrumentation disabled, and omit Wasmtime entirely. The production `tools/wtime-bench/Cargo.toml` has an unconditional Wasmtime dependency; the adapter must not build that package. Generate and retain its isolated lockfile and build metadata. Do not overwrite an already configured `CARGO_TARGET_DIR`; use Cargo's normal workspace target unless the session already provides a target directory. Make the adapter workspace unique, refresh source mtimes, parse Cargo's `compiler-artifact` output to locate the executable, and record the actual target directory and executable path rather than guessing a fixed output path. The adapter manifest records archive path/manifest hash, complete copied core hash map, overlay protocol/driver/leaf map, generated workspace and lock hashes, Rust/Cargo versions, release profile, selected Cargo features, actual build command, and executable hash. Give this artifact a distinct `source-bound-register-entry-adapter` kind. Do not tag it as `legacy-api-adapter` or report its mode as decoded.

The paired collector accepts the immutable core archive, its verified archived-core adapter, the distinct current-core candidate executable and source/build record produced by `build_current`, an explicit fixture/case/input/expected row, one accepted P135 call count, explicit eager/adaptive strategy settings and threshold, a pair count, two run seeds, and a new output directory. Both executables are the same minimal P135 harness build shape: identical generated entry point, workspace manifest, protocol/driver/leaf overlay, Register/runtime-only features, uninstrumented timing, and Rust/Cargo toolchain identities. Their core source maps and source origins must differ and remain individually identified: one is the verified immutable archive and the other is the live current workspace. The current candidate is not the Wasmtime-linked `tools/wtime-bench` executable. Both executables receive exactly the same P135 argv for each paired row: same fixture bytes, input, expected result, call count, mode `register`, strategy, and threshold. Accept only call counts **1, 10, 100, 10000**; eager requires threshold zero and adaptive requires a caller-supplied positive threshold. Do not substitute the legacy decoded baseline or relabel the archived adapter as a current candidate.

Each side observation is a separate process and fresh P135 Engine/Module/Store lifetime. Randomize archived-adapter/candidate process order from a recorded deterministic seed for each pair. Do not calibrate, warm, preflight, batch, or divide measurements. Validate the P135 schema and argv against the source/executable/fixture hashes, settings, oracle result, requested and actual calls, fixture counter, uninstrumented flag, and nonnegative integer `total_ns`, `prepare_ns`, `instantiate_ns`, and `invoke_ns`. Preserve the untouched raw JSON and report the metric as `entry_total` using `total_ns`; keep phase values as separate raw observations. Do not report `ns_per_operation` or scale by N.

The collector normally requires a distinct archived-core adapter and current-core candidate executable/source record. A `--same-source-smoke` mode may run one exact adapter binary on both sides only with an explicit same-source alias in the receipt and `timing_evidence:false`; reject this mode for non-smoke collection and reject its report as input to P066. Smoke uses two complete pairs per explicitly requested row in each of two independent runs. It checks settings forwarding and receipt integrity only. It does not choose workloads or adaptive thresholds. P066 later supplies a frozen row selection, source-bound candidate, and its full two-run, 30-pair-per-row candidate gate; smoke receipts cannot satisfy that gate.

Create reports and JSONL checkpoints only under the requested output root, reject path escapes/collisions before guest launch, and bind every row to both executable/source identities, fixture hash, settings, pair order, run seed and exact argv. Reject stale or partial pair joins, duplicate pair IDs, changed source or binary identities, missing rows and malformed JSON. Run existing checkpoints through the crash-safe `checkpoint.Journal` contract; do not route `entry_total` through the existing `ns_per_operation` stage validator.

## Implementation recipe

1. Add `entry_batch_adapter.py`: verify any accepted Register archive and its source manifest, copy the complete archived source-role closure into an isolated workspace, refresh copied Rust mtimes, overlay and hash the current P135 protocol/driver dependency set, build the runtime-only Register adapter with no Wasmtime dependency, and write/verify an immutable source and executable manifest. The `build_current` API makes a distinct current-workspace candidate with the same generated harness/manifest/overlay and its own verified core source map. Preserve configured Cargo target behavior and locate each executable from Cargo's actual build output.
2. Add `entry_batch_compare.py`: validate the distinct archived adapter and current-core candidate build records, common protocol/driver/settings/toolchain identities, explicit fixture row and P135 accepted settings before guest work; its CLI takes the archive with `--archive` and the adapter/candidate source outputs with `--adapter` and `--candidate`, never guessed Cargo output paths. Run each side as a fresh process with deterministic randomized order; validate every P135 response; and preserve exact raw pairs in a source/settings-bound checkpoint and report.
3. Add the producer and declarative contract for freezing P136-before, adapter attestation, candidate build record, smoke rows and receipts. Reject stale or foreign inputs before copying/building/launching and verify source stability again after collection. Make smoke output explicitly non-promoting and disallow same-binary/source aliases in candidate runs.
4. Add focused synthetic and CLI checks for archive/overlay identity, settings forwarding, exact N behavior, paired joins, and receipt recovery. Synthetic tests use a fresh temporary root and prove resolved output paths remain inside it before writing. Record helper sources before running them and disable bytecode writes when reading archived sources.
5. Run the two-run smoke on explicit P135 counter rows for 1, 10, 100, and 10000 calls, with caller-supplied eager/adaptive settings. Preserve actual argv, raw responses, pair order, fixtures, source maps, archived adapter/current candidate build records, lockfile, output journal and verifier receipts. Leave runtime semantics and all performance decisions to P066/P137.

## Meaningful cases

- `p136_source_bound_register_adapter_rejects_foreign_or_stale_inputs`: reject a changed archive manifest/source, protocol/driver/leaf overlay, workspace/lockfile, required P134 API or executable hash; prove the verified archive bytes and mtimes remain unchanged.
- `p136_candidate_and_adapter_share_protocol_driver_config_and_toolchain`: reject candidate records with different protocol, entry driver, compiled leaf, register feature, instrumentation, MVP/fuel settings or Rust/Cargo identity; retain the two distinct source maps and executable hashes.
- `p136_register_settings_forward_to_both_distinct_sides`: require both commands to use Register mode with exactly matching fixture/input/expected/calls/strategy/threshold; accept only eager-zero and caller-supplied positive adaptive settings; reject decoded or legacy adapter labeling.
- `p136_fixed_fresh_batch_rows_keep_exact_calls_without_calibration`: for each explicit accepted N, validate actual calls and fixture counter equal N, every result equals its oracle, raw timings are integer nanoseconds, and no warmup/calibration commands occur; reject divided timings and instrumented binaries.
- `p136_pairs_join_only_identical_fixture_and_settings`: reject side observations with different Wasm hash, input, expected result, N, strategy, threshold, mode, profile or run identity; retain randomized side order and unique pair IDs.
- `p136_receipts_reject_stale_duplicate_missing_or_smoke_only_pairs`: reject changed binary/source hashes, duplicate or missing sides, conflicting seeds, and same-source smoke aliases when the caller requests candidate evidence; preserve complete accepted raw records on resume.

## Verification

Use locked builds and exact feature configurations. Build the adapter from its copied Register sources with `register-executor,runtime-only`, no instrumentation and no Wasmtime dependency. Build and verify the separate current-core candidate with `build_current`; its `source_origin` must be `current-workspace`, and its executable must remain distinct from the archived adapter. Run adapter and collector tests, source-bound producer checks, then a non-promoting smoke with two pairs per row in each of two independent runs over all four accepted call counts. Preserve failed receipts and command logs. No timing ratio, speed decision, or promotion is asserted here.

```sh
python3 -B tools/wtime-bench/entry_batch_adapter.py build --core-archive target/performance/P136-before --harness tools/wtime-bench --output target/verification/P136-adapter
python3 -B tools/wtime-bench/entry_batch_adapter.py build-current --repo . --harness tools/wtime-bench --output target/verification/P136-candidate
python3 -B tools/wtime-bench/tests/test_entry_batch_adapter.py -v
python3 -B tools/wtime-bench/tests/test_p136_entry_batch.py -v
python3 -B tools/verification/tests/test_p136_entry_batch.py -v
python3 -B tools/verification/producers/p136.py freeze --archive target/performance/P136-before --output target/verification/P136-freeze
python3 -B tools/verification/producers/p136.py verify --freeze target/verification/P136-freeze/report.json --output target/verification/P136-smoke --smoke
```

## Measurement and decision

Selection: `register`. Explicit execution setting: `register`. Decision rule: `integrity`.

P136 is a protocol foundation and makes no speed claim. P066 retains all prerequisite, semantic, build/dependency, selection and measured-promotion gates. P066's frozen candidate rows use the distinct P136 archived-core adapter and source-bound current-core P135 harness executable; same-source smoke is never accepted for that gate. P137 owns analysis of raw total, phase, cold-code and cache-churn distributions.

## Before editing

Freeze and verify an immutable Register archive from the P135-complete source tree before adapter/tool edits. The standard concrete path is `target/performance/P136-before`; never replace an archive with a different identity. The adapter builder accepts a generic verified Register archive and does not hard-code P135-before.

```sh
python3 -B tools/wtime-bench/baseline.py --output target/performance/P136-before --mode register
```

## Completion evidence

Record the P136-before archive identity, adapter's full copied-core and overlaid-harness source maps, generated workspace and lock hashes, Cargo target path and build output, toolchain/features, adapter executable hash, current-core `build_current` source/build record with `source_origin: current-workspace`, protocol/driver/config/toolchain equality, fixture identity, exact side argv, all raw paired JSON and exit statuses, seeds/order, checkpoint and smoke verifier receipt. Label smoke `timing_evidence:false` with no decision. Keep status pending until every named check and the source-bound two-run smoke pass. No candidate performance claim is made.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/P136/shared-finalization/attempt-0001/evidence.json": "138a7b9629ad4b29d805599deaccb3891405cfcf41a127867795e5db8d8fc4d1"
  },
  "evidence": [
    {
      "bytes": 1545,
      "path": "/workspace/wtime/target/performance/P136/shared-finalization/attempt-0001/evidence.json",
      "sha256": "138a7b9629ad4b29d805599deaccb3891405cfcf41a127867795e5db8d8fc4d1"
    }
  ],
  "outcome": "contract_only",
  "status": "done",
  "task": "P136"
}
```

The full shared close passed at
`target/performance/P136/shared-finalization/attempt-0001`, using freeze and verify
attempt `0001`. All **19** tests passed: eight adapter, seven collector and four
producer integrity checks; none failed or were ignored. The real smoke ran
**32 pairs in two runs**, with **64 fresh processes** and **161,776 checked Wasm
invocations** across eight explicit rows. Both sides used the same MVP fixture,
input 7, expected 8, call count and eager-zero/adaptive-three settings. Raw
nanoseconds, exact argv, outputs, source maps and the complete checkpoint are
retained in [P136 records](../../../wtime/docs/benchmarks/performance/P136-records.json.gz).

The immutable Register predecessor is commit
`d2bcb28107d56f7c321312f82b4bfee8a11e970e`; its manifest SHA-256 is
`9fe9882d9c7e23b833d769a8edddabcfe92bf04c0633d153c1e5e532e7537335`.
The separately built archived/current executable hashes are
`186855ed4c24c06291f4c673445a4e20babbe89ea900a3323332a45087c33849`
and `a90977d00dd42bf56720098819addaedc1f596629f2899ae4810dfa93304f95d`.
The fixture SHA-256 is
`6b365cbdbe8c189ad1b19a8cbdc6304012207dbf0f83c0de6824f8c8b8f07776`.
All **732** candidate workspace inputs remain unchanged. The existing P134
default/Register build-budget evidence was revalidated against its full current
compilation inputs and reused; no core build-time measurement was added.
Default stripped size remains 1,000,960 bytes, Register 1,090,512 bytes, with zero
external core dependencies.

Outcome: **contract_only**. Collection is explicitly non-promoting
(`timing_evidence:false`); no speedup or threshold selection is claimed.
P066/P137 retain the selection, analysis and promotion gates. See
[the architecture note](../../../wtime/docs/architecture/register-entry-pairs.md),
[completion](../../../wtime/docs/benchmarks/performance/P136-completion.json) and
[proof](../../../wtime/docs/benchmarks/performance/P136-proof.json).
