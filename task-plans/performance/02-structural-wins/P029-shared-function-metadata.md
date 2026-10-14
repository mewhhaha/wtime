# P029 — Stop cloning function signatures into every activation

Status: **done**

Outcome: **rejected**

Kind: `candidate` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P002 — Compare separately built baseline and candidate binaries](../00-measurement/P002-paired-driver.md)

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P008 — Add scaling, branch and call benchmark families](../00-measurement/P008-scalar-workloads.md)

[P016 — Add explicit executor policies and test integration hooks](../01-execution-contracts/P016-execution-policy.md)

[P017 — Make every Module clone share immutable metadata](../01-execution-contracts/P017-shared-module-payload.md)

[P021 — Compute bounded per-function execution metadata](../01-execution-contracts/P021-function-execution-plans.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/lib.rs`
- `crates/wtime-runtime/src/execution.rs`
- `crates/wtime-runtime/src/continuation.rs`
- `crates/wtime-runtime/src/linker.rs`
- `crates/wtime-runtime/tests/performance.rs`
- `crates/wtime-runtime/tests/performance/p029.rs`
- `crates/wtime-runtime/tests/fixtures/performance/calls/direct-recursive-multivalue.wat`
- `crates/wtime-runtime/tests/fixtures/performance/calls/direct-recursive-multivalue.wasm`
- `crates/wtime-runtime/tests/fixtures/performance/calls/host-type-import.wat`
- `crates/wtime-runtime/tests/fixtures/performance/calls/host-type-import.wasm`
- `crates/wtime-runtime/tests/fixtures/performance/calls/imported-direct.wat`
- `crates/wtime-runtime/tests/fixtures/performance/calls/imported-direct.wasm`
- `crates/wtime-runtime/tests/fixtures/performance/calls/indirect-bad-signature.wat`
- `crates/wtime-runtime/tests/fixtures/performance/calls/indirect-bad-signature.wasm`
- `crates/wtime-runtime/tests/fixtures/performance/calls/indirect-good.wat`
- `crates/wtime-runtime/tests/fixtures/performance/calls/indirect-good.wasm`
- `crates/wtime-runtime/tests/fixtures/performance/calls/reentry-gc-roots.wat`
- `crates/wtime-runtime/tests/fixtures/performance/calls/reentry-gc-roots.wasm`
- `crates/wtime-runtime/tests/fixtures/performance/calls/tail-recursive.wat`
- `crates/wtime-runtime/tests/fixtures/performance/calls/tail-recursive.wasm`
- `crates/wtime-runtime/tests/fixtures/performance/calls/LICENSE.txt`
- `crates/wtime-runtime/tests/fixtures/performance/calls/README.md`
- `crates/wtime-runtime/tests/fixtures/performance/calls/IDENTITIES.md`
- `crates/wtime-runtime/tests/fixtures/performance/calls/fixture-identities.json`
- `crates/wtime-runtime/tests/fixtures/performance/calls/source-hashes.sha256`
- `docs/benchmarks/performance/P029-contract.json`
- `docs/benchmarks/performance/P029-build-decision.md`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

A frame retains a checked function identity plus immutable shared signature/context metadata, not a deep Function clone. Public handles still carry Store/generation checks.

## Implementation recipe

1. Separate immutable function signature/canonical/host-type context from Store-owned binding and callback state; use a checked compact frame identity or shared metadata handle.
2. Cache frame parameter/result counts and instance/body IDs in a small header only when their lifetime is tied to the retained instance/function.
3. Update exception/root/continuation consumers and physical byte accounting for the new header; never let arena reuse invalidate a captured frame.
4. Compare call/deep-capture diagnostics and timing while preserving all public dynamic-handle checks.
5. Record the changed path with focused assertions/diagnostic counts and exact source/binary identities; compare the pre-edit archive, and revert a rejected structural optimization rather than adding a permanent toggle solely for testing.

## Meaningful cases

- Recursive guest calls do not copy signature Vecs.
- A captured frame remains valid through function-arena growth and host reentry.
- Stale/foreign Func handles still fail at public invocation and callback signatures remain checked.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime p029_ -- --nocapture
cargo test --locked -p wtime-runtime --features execution-reference,performance-counters p029_ -- --nocapture
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml -j 4
python3 tools/wtime-bench/compare.py --baseline target/performance/P029-before --candidate tools/wtime-bench/target/release/wtime-bench --candidate-mode decoded --selection scalar --pairs 30 --build-growth-explanation-file docs/benchmarks/performance/P029-build-decision.md --output target/performance/P029
python3 tools/wtime-bench/analyze.py --input target/performance/P029 --output target/performance/P029/analysis.json
python3 tools/checks/build_budget.py --output target/performance/P029/build.json
python3 tools/wtime-bench/gates.py --task P029 --comparison target/performance/P029 --build target/performance/P029/build.json --contract docs/benchmarks/performance/P029-contract.json --output target/performance/P029/decision.json
```

## Measurement and decision

Selection: `scalar`. Explicit execution setting: `decoded`. Decision rule: `measured_promotion`.

Use the paired protocol and family/control/cold/build rules in [MEASUREMENT.md](../MEASUREMENT.md). Predeclare the targeted family and held-out controls before changing the implementation. Keep instrumentation off in production timings. Adopt only with supported gains in both runs; otherwise record rejected/inconclusive and remove default-path cost. A necessary immutable-interface change may be contract_only with its separate justification.

## Before editing

Run these commands before changing the candidate sources. The archive is immutable and captures the immediate predecessor. Retain P001's original archive for milestone totals. For native work, use the named portable control and verify its actual policy/quantum; if this archive already exists with different sources, choose a new recorded run-specific directory and update the comparison path rather than overwriting it.

```sh
python3 tools/wtime-bench/baseline.py --output target/performance/P029-before
```

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, including candidate-path proof, no stale or zero-work pass; retain CC0 fixture license and provenance files; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. Executed evidence is recorded below.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/docs/benchmarks/performance/P029-build-decision.md": {
      "bytes": 1878,
      "sha256": "88083ae65212b0c4237e5119970adffd3c2457ceac2e1186cee8897ffa3f6173"
    },
    "/workspace/wtime/docs/benchmarks/performance/P029-candidate.patch": {
      "bytes": 12317,
      "sha256": "ea134513a2dd9607e65768ee3239c99070b1cd3a2d2b11a34a28e995efa46ad0"
    },
    "/workspace/wtime/docs/benchmarks/performance/P029-contract.json": {
      "bytes": 2757,
      "sha256": "c70803c8d8eb6188b4b2efa5978860503aed2a7fb1a0ddad90596e805dd3b1e4"
    },
    "/workspace/wtime/docs/benchmarks/performance/P029-evidence.json": {
      "bytes": 594306,
      "sha256": "420b7bcb73c2ddb39982bc85504d369aa99e751f2f212c53afd42d7d231b9f8e"
    },
    "/workspace/wtime/target/performance/P029-before-build.json": {
      "bytes": 99463,
      "sha256": "16d6d13f94e88335db70b206427bf128cedbf62db613bc0793cb8a91d1df6700"
    },
    "/workspace/wtime/target/performance/P029-before-diagnostic-tests.log": {
      "bytes": 1873,
      "sha256": "402b866248f2189ff813b268ad389fe1a7a48d51b69b0a7ff97eedcd8689720d"
    },
    "/workspace/wtime/target/performance/P029-before-tests-retry.log": {
      "bytes": 1813,
      "sha256": "52eca24c0fbd240b11d78ef40d935dbd711dc320453520b2eecf91ce6ff10b09"
    },
    "/workspace/wtime/target/performance/P029-before-tests.log": {
      "bytes": 1006,
      "sha256": "fb5ed77c6f90321ae8dcf35a8215bf2108be90d7ecdb474cbdf69eb751ae8cb2"
    },
    "/workspace/wtime/target/performance/P029-before/manifest.json": {
      "bytes": 37316,
      "sha256": "4d2d5ff10e6da1f24b243cba4d98679397ca7c14015be0034dee5d4c3653fd3e"
    },
    "/workspace/wtime/target/performance/P029-build.json": {
      "bytes": 99464,
      "sha256": "4aa183f885ed61ae522c37a9e916a64ff12a797672f47a654ac36c1f14116662"
    },
    "/workspace/wtime/target/performance/P029-candidate-bench-build.log": {
      "bytes": 206,
      "sha256": "d263086d7ec22165e1f328617707469379c2fb70e54596667fc0b8abe0ebec40"
    },
    "/workspace/wtime/target/performance/P029-candidate-retained/source-manifest.json": {
      "bytes": 15015,
      "sha256": "67805f43a388c003a9a1faed1daec5e05cd684f62737dc33c34766fb46761442"
    },
    "/workspace/wtime/target/performance/P029-candidate-tests.log": {
      "bytes": 2072,
      "sha256": "f0fd9e2d1f9f466bc1ac41a0b97d256a125911521700f98000fb591900a0971c"
    },
    "/workspace/wtime/target/performance/P029-candidate-validation-commands.json": {
      "bytes": 1204,
      "sha256": "bed5feaca1073a41e8841f75f6acb21ea6e861d913887961a834f50458ac6dd7"
    },
    "/workspace/wtime/target/performance/P029-candidate_default.log": {
      "bytes": 14844,
      "sha256": "a8e8fefa5b270536ff89213e52296f05731bffdba056bcd64f5aea3c061f2b54"
    },
    "/workspace/wtime/target/performance/P029-candidate_diagnostic_performance.log": {
      "bytes": 4655,
      "sha256": "ce7bbb8fd8a60dd1cba7fafff0a5a4cc8f800a2d6496e2a5f397431b02886d3c"
    },
    "/workspace/wtime/target/performance/P029-candidate_focused_diagnostic.log": {
      "bytes": 2132,
      "sha256": "2b78113670d899860a9562bf292025de05e613ce58b2ca43f1162ff06302661a"
    },
    "/workspace/wtime/target/performance/P029-incremental-build-decision.json": {
      "bytes": 1051,
      "sha256": "bcb3226342356ba385e2944f03531597e60a79c6a2f32282f5bacefaf30e7185"
    },
    "/workspace/wtime/target/performance/P029-public-match-array-failure.rs": {
      "bytes": 7922,
      "sha256": "1ea193964497abc46b24aeba67b8c360faa6c7237f878371bf7594abbf49338c"
    },
    "/workspace/wtime/target/performance/P029/analysis.json": {
      "bytes": 1260952,
      "sha256": "88d104de48e0d8775a5b3a5b0ad48a7a30025128d5c45134b042c08a77a6b418"
    },
    "/workspace/wtime/target/performance/P029/analysis.log": {
      "bytes": 0,
      "sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    },
    "/workspace/wtime/target/performance/P029/checkpoint.jsonl": {
      "bytes": 36214924,
      "sha256": "8da832a196aa25dd01c89c1d58926a7018b8cbcf3eff8f7741f3209c5ff26687"
    },
    "/workspace/wtime/target/performance/P029/commands.json": {
      "bytes": 5088,
      "sha256": "c97cca04a99459bb91bf8ea7fa268c3150ab4a0f94c5eb691a9dde461d016beb"
    },
    "/workspace/wtime/target/performance/P029/decision.json": {
      "bytes": 1671907,
      "sha256": "f1f4652e5d864a295f5ffe1a267c98483782b92f9fc9f736bc3bf09de55fa245"
    },
    "/workspace/wtime/target/performance/P029/extraction.json": {
      "bytes": 115147,
      "sha256": "774c1198c665b6152083b5aab40ae05d45089976981268e73faedd0196bdd601"
    },
    "/workspace/wtime/target/performance/P029/extraction.log": {
      "bytes": 76112,
      "sha256": "e8aab444a9ac23dc76463265a0a02b03fb4cc9f4a10aa2a18da4ba45ff3cb7b8"
    },
    "/workspace/wtime/target/performance/P029/final-decision.json": {
      "bytes": 9540,
      "sha256": "4956277b01e275869dd1ad0833a1758b3a954afbbfe04c7b72ea0e5496566484"
    },
    "/workspace/wtime/target/performance/P029/raw-exits.json": {
      "bytes": 39,
      "sha256": "8cb285167680019e8683e276900144b811a534abc0797ee27c6fde696476ba90"
    },
    "/workspace/wtime/target/performance/P029/raw-gate.log": {
      "bytes": 1671907,
      "sha256": "f1f4652e5d864a295f5ffe1a267c98483782b92f9fc9f736bc3bf09de55fa245"
    },
    "/workspace/wtime/target/performance/P029/report.json": {
      "bytes": 59830259,
      "sha256": "f675b214c272d2b0a1cd0db376927422ddb8b28535d9717db87e0365e6f4450e"
    },
    "/workspace/wtime/target/performance/P029/restore.log": {
      "bytes": 268,
      "sha256": "ee96d1980eab16d5f020676c66914f6d0ba0e2992088962581400071ee20ba6e"
    },
    "/workspace/wtime/target/performance/P029/restored-build.json": {
      "bytes": 99464,
      "sha256": "8630fe0f8110c854be03812380cc201fc9af76877f9f23476e804534c434011f"
    },
    "/workspace/wtime/target/performance/P029/restored-default-tests.log": {
      "bytes": 14586,
      "sha256": "f232d633bd12e74532155730e22266be3e490e64057d456492fb3e94452e8d50"
    },
    "/workspace/wtime/target/performance/P029/restored-diagnostic-tests.log": {
      "bytes": 4729,
      "sha256": "6d0b7b021de05e834403865ff89d5d6fc0ef5c428e6b747f5b739de68c811cb0"
    },
    "/workspace/wtime/target/performance/P029/restored-focused-default-tests.log": {
      "bytes": 1739,
      "sha256": "9dc43c6508002fb811499dff013e7f37669f7616423be6a152618e38102a1fc3"
    },
    "/workspace/wtime/target/performance/P029/restored-focused-diagnostic-tests.log": {
      "bytes": 1873,
      "sha256": "3118488165c3b099ef1c26914f7c07ab019843c1ef698f3154798b6bd37c2849"
    },
    "/workspace/wtime/target/performance/P029/rollback-identities.json": {
      "bytes": 1798,
      "sha256": "b9d9f7acbe5c6fe000c19d62c4bed4714da6e781725de739cfda8b278e5bccdf"
    },
    "/workspace/wtime/target/performance/P029/task-precompletion/P029-shared-function-metadata.md": {
      "bytes": 7376,
      "sha256": "5e655a63a84ae9be3f1a86c15535ad5e32f3d584a8ea3482379a811dabc7aeff"
    },
    "/workspace/wtime/target/performance/P029/task-precompletion/TASKS.json": {
      "bytes": 1061289,
      "sha256": "64646d768128ab2f6df8b20a900b634b33e5561d0e8173781fe6703c29f99f5c"
    }
  },
  "artifacts": [
    "/workspace/wtime/docs/benchmarks/performance/P029-build-decision.md",
    "/workspace/wtime/docs/benchmarks/performance/P029-candidate.patch",
    "/workspace/wtime/docs/benchmarks/performance/P029-contract.json",
    "/workspace/wtime/docs/benchmarks/performance/P029-evidence.json",
    "/workspace/wtime/target/performance/P029-before-build.json",
    "/workspace/wtime/target/performance/P029-before-diagnostic-tests.log",
    "/workspace/wtime/target/performance/P029-before-tests-retry.log",
    "/workspace/wtime/target/performance/P029-before-tests.log",
    "/workspace/wtime/target/performance/P029-before/manifest.json",
    "/workspace/wtime/target/performance/P029-build.json",
    "/workspace/wtime/target/performance/P029-candidate-bench-build.log",
    "/workspace/wtime/target/performance/P029-candidate-retained/source-manifest.json",
    "/workspace/wtime/target/performance/P029-candidate-tests.log",
    "/workspace/wtime/target/performance/P029-candidate-validation-commands.json",
    "/workspace/wtime/target/performance/P029-candidate_default.log",
    "/workspace/wtime/target/performance/P029-candidate_diagnostic_performance.log",
    "/workspace/wtime/target/performance/P029-candidate_focused_diagnostic.log",
    "/workspace/wtime/target/performance/P029-incremental-build-decision.json",
    "/workspace/wtime/target/performance/P029-public-match-array-failure.rs",
    "/workspace/wtime/target/performance/P029/analysis.json",
    "/workspace/wtime/target/performance/P029/analysis.log",
    "/workspace/wtime/target/performance/P029/checkpoint.jsonl",
    "/workspace/wtime/target/performance/P029/commands.json",
    "/workspace/wtime/target/performance/P029/decision.json",
    "/workspace/wtime/target/performance/P029/extraction.json",
    "/workspace/wtime/target/performance/P029/extraction.log",
    "/workspace/wtime/target/performance/P029/final-decision.json",
    "/workspace/wtime/target/performance/P029/raw-exits.json",
    "/workspace/wtime/target/performance/P029/raw-gate.log",
    "/workspace/wtime/target/performance/P029/report.json",
    "/workspace/wtime/target/performance/P029/restore.log",
    "/workspace/wtime/target/performance/P029/restored-build.json",
    "/workspace/wtime/target/performance/P029/restored-default-tests.log",
    "/workspace/wtime/target/performance/P029/restored-diagnostic-tests.log",
    "/workspace/wtime/target/performance/P029/restored-focused-default-tests.log",
    "/workspace/wtime/target/performance/P029/restored-focused-diagnostic-tests.log",
    "/workspace/wtime/target/performance/P029/rollback-identities.json",
    "/workspace/wtime/target/performance/P029/task-precompletion/P029-shared-function-metadata.md",
    "/workspace/wtime/target/performance/P029/task-precompletion/TASKS.json",
    "/workspace/wtime/target/performance/P029/complete_failure_text_failed.py",
    "/workspace/wtime/target/performance/P029/finalizer.log",
    "/workspace/wtime/target/performance/P029/finalizer-retry.log",
    "/workspace/wtime/target/performance/P029/finalizer-command-record.json",
    "/workspace/wtime/target/performance/P029/finalizer-retry-command-record.json",
    "/workspace/wtime/target/performance/P029/complete_executed.py",
    "/workspace/wtime/target/performance/P029/restore_executed.py",
    "/workspace/wtime/target/performance/P029/process_closed_executed.py",
    "/workspace/wtime/target/performance/P029/finalize_command_executed.py"
  ],
  "candidate_promoted": false,
  "commands": [
    {
      "command": ". /workspace/.setup/rust-env.sh\npython3 tools/wtime-bench/compare.py --baseline target/performance/P029-before --candidate tools/wtime-bench/target/release/wtime-bench --candidate-mode decoded --selection scalar --pairs 30 --build-growth-explanation-file docs/benchmarks/performance/P029-build-decision.md --output target/performance/P029 > target/performance/P029-run.log 2>&1",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P029-run.log",
      "log_sha256": "a6651069c51ef66aeac29ba445569051f42bc608a7da852b9c2a5b9abea3b7c5",
      "name": "comparison"
    },
    {
      "command": ". /workspace/.setup/rust-env.sh\npython3 /tmp/extract_p029.py --report /workspace/wtime/target/performance/P029/report.json --checkpoint /workspace/wtime/target/performance/P029/checkpoint.jsonl --archive /workspace/wtime/target/performance/P029-before --candidate-dir /workspace/wtime/target/performance/P029-candidate-retained --output /workspace/wtime/target/performance/P029/extraction.json",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P029/extraction.log",
      "log_sha256": "e8aab444a9ac23dc76463265a0a02b03fb4cc9f4a10aa2a18da4ba45ff3cb7b8",
      "name": "extraction",
      "stdout_path": "/workspace/wtime/target/performance/P029/extraction.log",
      "stdout_sha256": "e8aab444a9ac23dc76463265a0a02b03fb4cc9f4a10aa2a18da4ba45ff3cb7b8"
    },
    {
      "command": ". /workspace/.setup/rust-env.sh\npython3 tools/wtime-bench/analyze.py --input target/performance/P029 --output target/performance/P029/analysis.json",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P029/analysis.log",
      "log_sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
      "name": "analysis",
      "stdout_path": "/workspace/wtime/target/performance/P029/analysis.log",
      "stdout_sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    },
    {
      "command": ". /workspace/.setup/rust-env.sh\npython3 tools/wtime-bench/gates.py --task P029 --comparison target/performance/P029 --build target/performance/P029/comparison-build.json --contract docs/benchmarks/performance/P029-contract.json --output target/performance/P029/decision.json",
      "exit_code": 1,
      "log_path": "/workspace/wtime/target/performance/P029/raw-gate.log",
      "log_sha256": "f1f4652e5d864a295f5ffe1a267c98483782b92f9fc9f736bc3bf09de55fa245",
      "name": "raw_gate",
      "stdout_path": "/workspace/wtime/target/performance/P029/raw-gate.log",
      "stdout_sha256": "f1f4652e5d864a295f5ffe1a267c98483782b92f9fc9f736bc3bf09de55fa245"
    },
    {
      "command": ". /workspace/.setup/rust-env.sh\ncargo test --locked -p wtime-runtime",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P029/restored-default-tests.log",
      "log_sha256": "f232d633bd12e74532155730e22266be3e490e64057d456492fb3e94452e8d50",
      "name": "restored_default_tests"
    },
    {
      "command": ". /workspace/.setup/rust-env.sh\ncargo test --locked -p wtime-runtime --features execution-reference,performance-counters --test performance",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P029/restored-diagnostic-tests.log",
      "log_sha256": "6d0b7b021de05e834403865ff89d5d6fc0ef5c428e6b747f5b739de68c811cb0",
      "name": "restored_diagnostic_tests"
    },
    {
      "command": ". /workspace/.setup/rust-env.sh\ncargo test --locked -p wtime-runtime p029_ -- --nocapture",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P029/restored-focused-default-tests.log",
      "log_sha256": "9dc43c6508002fb811499dff013e7f37669f7616423be6a152618e38102a1fc3",
      "name": "restored_focused_default_tests"
    },
    {
      "command": ". /workspace/.setup/rust-env.sh\ncargo test --locked -p wtime-runtime --features execution-reference,performance-counters p029_ -- --nocapture",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P029/restored-focused-diagnostic-tests.log",
      "log_sha256": "3118488165c3b099ef1c26914f7c07ab019843c1ef698f3154798b6bd37c2849",
      "name": "restored_focused_diagnostic_tests"
    },
    {
      "command": ". /workspace/.setup/rust-env.sh\ncargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml -j 4",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P029/restored-benchmark-build.log",
      "log_sha256": "acb9f502228a45cbb5670fe08a83f4d3bdb4f5793ac52d6effdbd354568f9239",
      "name": "rollback_benchmark_build"
    },
    {
      "command": ". /workspace/.setup/rust-env.sh\npython3 tools/checks/build_budget.py --output target/performance/P029/restored-build.json",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P029/restored-build.log",
      "log_sha256": "5017bdce4c85ef184a4e717a977f7a680aff1b1d9d61eea0226d28b01b27c907",
      "name": "rollback_build"
    }
  ],
  "counts": {
    "analysis_group_count": 850,
    "correctness_probe_kinds": {},
    "correctness_probe_records": 0,
    "expected_pair_ids_sha256": "e2d25b4fb5e5913269e686e33e2b8957e5998391b8ae2e2103a30a433d37b7ad",
    "fixture_files_verified": 85,
    "group_count_total": 850,
    "observation_side_checks": 51000,
    "pair_event_count": 25500,
    "pairs_per_group": 30,
    "report_pair_ids_sha256": "e2d25b4fb5e5913269e686e33e2b8957e5998391b8ae2e2103a30a433d37b7ad",
    "run_count": 2,
    "run_ids": [
      0,
      1
    ],
    "run_seeds": {
      "0": 20261011,
      "1": 21261014
    },
    "stage_groups_per_run": 425,
    "stages_per_workload": 5,
    "warm_calibration_policy_limit": "legacy common MIN batch from one-shot estimates; no guarantee that both actual timers reached 20 ms",
    "warm_calibration_records": 0,
    "warm_estimated_under_20ms_observation_count": 4144,
    "warm_observed_duration_summary": {
      "0:branch_calls:baseline": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 23430526.0,
          "median": 15933843.0,
          "min": 12356189.0,
          "p95": 21785403.0
        },
        "estimated_under_20ms_count": 26,
        "observations": 30
      },
      "0:branch_calls:candidate": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 25447227.0,
          "median": 14709429.0,
          "min": 12476316.0,
          "p95": 21807869.0
        },
        "estimated_under_20ms_count": 28,
        "observations": 30
      },
      "0:call_overhead:baseline": {
        "batch": 48900,
        "estimated_total_ns": {
          "max": 32086859.7,
          "median": 21026804.4,
          "min": 17414805.9,
          "p95": 30898198.5
        },
        "estimated_under_20ms_count": 10,
        "observations": 30
      },
      "0:call_overhead:candidate": {
        "batch": 48900,
        "estimated_total_ns": {
          "max": 34310929.5,
          "median": 19346527.05,
          "min": 14944133.4,
          "p95": 30496436.1
        },
        "estimated_under_20ms_count": 17,
        "observations": 30
      },
      "0:integer_loop:baseline": {
        "batch": 3,
        "estimated_total_ns": {
          "max": 40095831.0,
          "median": 26245518.0,
          "min": 22202805.0,
          "p95": 34521609.999
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:integer_loop:candidate": {
        "batch": 3,
        "estimated_total_ns": {
          "max": 54060792.0,
          "median": 27007423.5,
          "min": 22549184.001000002,
          "p95": 39488439.999
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_accumulator_n0:baseline": {
        "batch": 9294,
        "estimated_total_ns": {
          "max": 11136042.918,
          "median": 8498484.717,
          "min": 6798384.414,
          "p95": 11027981.58
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "0:scalar_accumulator_n0:candidate": {
        "batch": 9294,
        "estimated_total_ns": {
          "max": 20101518.606000002,
          "median": 7084026.21,
          "min": 4583419.746,
          "p95": 11921804.148
        },
        "estimated_under_20ms_count": 29,
        "observations": 30
      },
      "0:scalar_accumulator_n10000:baseline": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 47980072.0,
          "median": 15848524.5,
          "min": 13618490.0,
          "p95": 39106652.0
        },
        "estimated_under_20ms_count": 25,
        "observations": 30
      },
      "0:scalar_accumulator_n10000:candidate": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 33330479.0,
          "median": 16538788.0,
          "min": 14153958.0,
          "p95": 27001127.0
        },
        "estimated_under_20ms_count": 26,
        "observations": 30
      },
      "0:scalar_accumulator_n1024:baseline": {
        "batch": 25,
        "estimated_total_ns": {
          "max": 37685609.0,
          "median": 22218772.0,
          "min": 18742838.0,
          "p95": 29121570.0
        },
        "estimated_under_20ms_count": 7,
        "observations": 30
      },
      "0:scalar_accumulator_n1024:candidate": {
        "batch": 25,
        "estimated_total_ns": {
          "max": 31826573.0,
          "median": 21998997.0,
          "min": 17980210.0,
          "p95": 30433350.0
        },
        "estimated_under_20ms_count": 11,
        "observations": 30
      },
      "0:scalar_accumulator_n17:baseline": {
        "batch": 2068,
        "estimated_total_ns": {
          "max": 149466358.53599998,
          "median": 27632179.652,
          "min": 22186885.424,
          "p95": 53470853.216000006
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_accumulator_n17:candidate": {
        "batch": 2068,
        "estimated_total_ns": {
          "max": 184911163.00800002,
          "median": 26240343.272,
          "min": 22614758.759999998,
          "p95": 64360699.26
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_accumulator_n1:baseline": {
        "batch": 7582,
        "estimated_total_ns": {
          "max": 21228220.076,
          "median": 13134086.304,
          "min": 10991731.548,
          "p95": 19394604.36
        },
        "estimated_under_20ms_count": 29,
        "observations": 30
      },
      "0:scalar_accumulator_n1:candidate": {
        "batch": 7582,
        "estimated_total_ns": {
          "max": 27369215.484,
          "median": 12022819.101,
          "min": 10862867.876,
          "p95": 19580151.064000003
        },
        "estimated_under_20ms_count": 29,
        "observations": 30
      },
      "0:scalar_accumulator_n32:baseline": {
        "batch": 1141,
        "estimated_total_ns": {
          "max": 48397322.344000004,
          "median": 30693359.823,
          "min": 27813948.197,
          "p95": 42927266.795
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_accumulator_n32:candidate": {
        "batch": 1141,
        "estimated_total_ns": {
          "max": 45266090.877,
          "median": 32054448.454,
          "min": 26940413.43,
          "p95": 43192730.714
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_br_table_n0:baseline": {
        "batch": 9824,
        "estimated_total_ns": {
          "max": 21107355.200000003,
          "median": 8381866.272,
          "min": 6858212.992,
          "p95": 18264063.648
        },
        "estimated_under_20ms_count": 29,
        "observations": 30
      },
      "0:scalar_br_table_n0:candidate": {
        "batch": 9824,
        "estimated_total_ns": {
          "max": 12213078.912,
          "median": 7539708.784,
          "min": 6459280.0,
          "p95": 11723205.152
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "0:scalar_br_table_n10000:baseline": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 43509326.0,
          "median": 30138376.0,
          "min": 25024718.0,
          "p95": 40187370.0
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_br_table_n10000:candidate": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 64418787.0,
          "median": 31318649.0,
          "min": 25090521.0,
          "p95": 58798972.0
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_br_table_n1024:baseline": {
        "batch": 16,
        "estimated_total_ns": {
          "max": 47420494.0,
          "median": 26688997.496,
          "min": 21159062.0,
          "p95": 38871760.992
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_br_table_n1024:candidate": {
        "batch": 16,
        "estimated_total_ns": {
          "max": 36234416.992,
          "median": 26978595.0,
          "min": 20923314.0,
          "p95": 34222976.992
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_br_table_n17:baseline": {
        "batch": 1103,
        "estimated_total_ns": {
          "max": 44292191.536,
          "median": 29130228.897,
          "min": 23690735.865000002,
          "p95": 37792964.782000005
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_br_table_n17:candidate": {
        "batch": 1103,
        "estimated_total_ns": {
          "max": 39332238.784,
          "median": 28061119.123499997,
          "min": 24180131.450000003,
          "p95": 35429414.467999995
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_br_table_n1:baseline": {
        "batch": 6979,
        "estimated_total_ns": {
          "max": 19918742.963000003,
          "median": 14751257.566499999,
          "min": 13080718.763,
          "p95": 18394710.816999998
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "0:scalar_br_table_n1:candidate": {
        "batch": 6979,
        "estimated_total_ns": {
          "max": 25093503.967,
          "median": 14446568.3845,
          "min": 11864027.819,
          "p95": 19473768.902000003
        },
        "estimated_under_20ms_count": 29,
        "observations": 30
      },
      "0:scalar_br_table_n32:baseline": {
        "batch": 725,
        "estimated_total_ns": {
          "max": 45478425.675,
          "median": 33728417.0125,
          "min": 28242063.975,
          "p95": 44530995.675000004
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_br_table_n32:candidate": {
        "batch": 725,
        "estimated_total_ns": {
          "max": 49336255.800000004,
          "median": 32587193.425,
          "min": 28259047.825000003,
          "p95": 44735278.2
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_branch_entropy_n0:baseline": {
        "batch": 23725,
        "estimated_total_ns": {
          "max": 27735521.45,
          "median": 20064446.025,
          "min": 17283757.400000002,
          "p95": 26433303.65
        },
        "estimated_under_20ms_count": 15,
        "observations": 30
      },
      "0:scalar_branch_entropy_n0:candidate": {
        "batch": 23725,
        "estimated_total_ns": {
          "max": 32035559.075,
          "median": 18477397.737499997,
          "min": 14833107.25,
          "p95": 23843696.175
        },
        "estimated_under_20ms_count": 22,
        "observations": 30
      },
      "0:scalar_branch_entropy_n10000:baseline": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 47224814.0,
          "median": 28380364.5,
          "min": 24174690.0,
          "p95": 43666384.0
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_branch_entropy_n10000:candidate": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 64523126.0,
          "median": 31503185.5,
          "min": 24793351.0,
          "p95": 50998704.0
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_branch_entropy_n1024:baseline": {
        "batch": 17,
        "estimated_total_ns": {
          "max": 39229995.005,
          "median": 26385356.005,
          "min": 21353561.008,
          "p95": 37623432.003
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_branch_entropy_n1024:candidate": {
        "batch": 17,
        "estimated_total_ns": {
          "max": 32412996.008,
          "median": 25825949.5,
          "min": 21387908.998,
          "p95": 32361350.994
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_branch_entropy_n17:baseline": {
        "batch": 90,
        "estimated_total_ns": {
          "max": 7492043.970000001,
          "median": 2120951.52,
          "min": 1402289.01,
          "p95": 3799783.98
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "0:scalar_branch_entropy_n17:candidate": {
        "batch": 90,
        "estimated_total_ns": {
          "max": 3707494.0199999996,
          "median": 2078661.0150000001,
          "min": 1443607.02,
          "p95": 3168551.97
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "0:scalar_branch_entropy_n1:baseline": {
        "batch": 12346,
        "estimated_total_ns": {
          "max": 51769506.466,
          "median": 35299423.934,
          "min": 26629247.898000002,
          "p95": 50850099.846
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_branch_entropy_n1:candidate": {
        "batch": 12346,
        "estimated_total_ns": {
          "max": 92796005.226,
          "median": 33725605.238000005,
          "min": 24725383.584,
          "p95": 46536456.83
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_branch_entropy_n32:baseline": {
        "batch": 763,
        "estimated_total_ns": {
          "max": 51006897.165,
          "median": 36432737.264,
          "min": 30936234.812,
          "p95": 49077125.958000004
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_branch_entropy_n32:candidate": {
        "batch": 763,
        "estimated_total_ns": {
          "max": 73163982.255,
          "median": 34913508.5075,
          "min": 31167287.998,
          "p95": 52798698.897
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_call_ref_n0:baseline": {
        "batch": 23175,
        "estimated_total_ns": {
          "max": 43703832.15,
          "median": 23663772.3375,
          "min": 19437336.0,
          "p95": 32305810.95
        },
        "estimated_under_20ms_count": 1,
        "observations": 30
      },
      "0:scalar_call_ref_n0:candidate": {
        "batch": 23175,
        "estimated_total_ns": {
          "max": 55423244.25000001,
          "median": 21465519.299999997,
          "min": 17539627.95,
          "p95": 48525390.9
        },
        "estimated_under_20ms_count": 8,
        "observations": 30
      },
      "0:scalar_call_ref_n10000:baseline": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 42813571.0,
          "median": 28629550.0,
          "min": 25137065.0,
          "p95": 37108075.0
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_call_ref_n10000:candidate": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 31274990.0,
          "median": 26909984.0,
          "min": 23691397.0,
          "p95": 31246983.0
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_call_ref_n1024:baseline": {
        "batch": 10,
        "estimated_total_ns": {
          "max": 20694482.0,
          "median": 14371686.0,
          "min": 12805128.0,
          "p95": 20377509.0
        },
        "estimated_under_20ms_count": 28,
        "observations": 30
      },
      "0:scalar_call_ref_n1024:candidate": {
        "batch": 10,
        "estimated_total_ns": {
          "max": 23527560.0,
          "median": 14170971.0,
          "min": 11710293.0,
          "p95": 20313246.0
        },
        "estimated_under_20ms_count": 28,
        "observations": 30
      },
      "0:scalar_call_ref_n17:baseline": {
        "batch": 1450,
        "estimated_total_ns": {
          "max": 54182489.3,
          "median": 36229619.525,
          "min": 31512835.500000004,
          "p95": 49648375.55
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_call_ref_n17:candidate": {
        "batch": 1450,
        "estimated_total_ns": {
          "max": 83938729.55,
          "median": 34902585.325,
          "min": 29362276.700000003,
          "p95": 68711442.89999999
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_call_ref_n1:baseline": {
        "batch": 6669,
        "estimated_total_ns": {
          "max": 25521982.902,
          "median": 17474443.9155,
          "min": 14214866.796,
          "p95": 25476286.914
        },
        "estimated_under_20ms_count": 20,
        "observations": 30
      },
      "0:scalar_call_ref_n1:candidate": {
        "batch": 6669,
        "estimated_total_ns": {
          "max": 26749705.788000003,
          "median": 16720076.646000002,
          "min": 14008594.626,
          "p95": 25129959.075000003
        },
        "estimated_under_20ms_count": 25,
        "observations": 30
      },
      "0:scalar_call_ref_n32:baseline": {
        "batch": 780,
        "estimated_total_ns": {
          "max": 47687374.02,
          "median": 36379401.63,
          "min": 31460158.859999996,
          "p95": 44683806.96
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_call_ref_n32:candidate": {
        "batch": 780,
        "estimated_total_ns": {
          "max": 61407276.06,
          "median": 34691923.89,
          "min": 29722948.319999997,
          "p95": 50962332.72
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_dependent_n0:baseline": {
        "batch": 19456,
        "estimated_total_ns": {
          "max": 107826922.496,
          "median": 18609265.152,
          "min": 15557523.456,
          "p95": 28770579.456
        },
        "estimated_under_20ms_count": 18,
        "observations": 30
      },
      "0:scalar_dependent_n0:candidate": {
        "batch": 19456,
        "estimated_total_ns": {
          "max": 34530859.008,
          "median": 17932585.472000003,
          "min": 14483863.552000001,
          "p95": 28902218.752
        },
        "estimated_under_20ms_count": 20,
        "observations": 30
      },
      "0:scalar_dependent_n10000:baseline": {
        "batch": 3,
        "estimated_total_ns": {
          "max": 40412174.001,
          "median": 24454524.0,
          "min": 19022108.001000002,
          "p95": 39215796.999
        },
        "estimated_under_20ms_count": 5,
        "observations": 30
      },
      "0:scalar_dependent_n10000:candidate": {
        "batch": 3,
        "estimated_total_ns": {
          "max": 49932657.999,
          "median": 22041616.9995,
          "min": 15915402.0,
          "p95": 35340861.0
        },
        "estimated_under_20ms_count": 5,
        "observations": 30
      },
      "0:scalar_dependent_n1024:baseline": {
        "batch": 27,
        "estimated_total_ns": {
          "max": 29006185.995,
          "median": 20066247.4905,
          "min": 17514125.991,
          "p95": 27324835.002
        },
        "estimated_under_20ms_count": 15,
        "observations": 30
      },
      "0:scalar_dependent_n1024:candidate": {
        "batch": 27,
        "estimated_total_ns": {
          "max": 44918527.995000005,
          "median": 19874296.512000002,
          "min": 17309245.995,
          "p95": 30675704.994000003
        },
        "estimated_under_20ms_count": 17,
        "observations": 30
      },
      "0:scalar_dependent_n17:baseline": {
        "batch": 2171,
        "estimated_total_ns": {
          "max": 699223162.0139999,
          "median": 50191239.3645,
          "min": 34715510.102,
          "p95": 203714082.936
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_dependent_n17:candidate": {
        "batch": 2171,
        "estimated_total_ns": {
          "max": 478658678.043,
          "median": 46573259.1805,
          "min": 31636576.192,
          "p95": 240700543.707
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_dependent_n1:baseline": {
        "batch": 342,
        "estimated_total_ns": {
          "max": 1772645.85,
          "median": 508938.066,
          "min": 325471.14,
          "p95": 1714178.898
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "0:scalar_dependent_n1:candidate": {
        "batch": 342,
        "estimated_total_ns": {
          "max": 2149444.008,
          "median": 507371.022,
          "min": 307448.082,
          "p95": 2042426.0520000001
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "0:scalar_dependent_n32:baseline": {
        "batch": 1315,
        "estimated_total_ns": {
          "max": 50947877.395,
          "median": 33809010.309999995,
          "min": 28017391.315,
          "p95": 46897754.980000004
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_dependent_n32:candidate": {
        "batch": 1315,
        "estimated_total_ns": {
          "max": 55226317.99999999,
          "median": 35519827.94,
          "min": 28921089.56,
          "p95": 50844084.445
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_direct_calls_n0:baseline": {
        "batch": 22936,
        "estimated_total_ns": {
          "max": 53790883.36000001,
          "median": 22553266.968000002,
          "min": 19979618.408,
          "p95": 37264486.176
        },
        "estimated_under_20ms_count": 1,
        "observations": 30
      },
      "0:scalar_direct_calls_n0:candidate": {
        "batch": 22936,
        "estimated_total_ns": {
          "max": 37458432.992,
          "median": 20217545.004,
          "min": 17341657.304,
          "p95": 26878767.208
        },
        "estimated_under_20ms_count": 13,
        "observations": 30
      },
      "0:scalar_direct_calls_n10000:baseline": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 27979776.0,
          "median": 22523540.5,
          "min": 19519589.0,
          "p95": 27614163.0
        },
        "estimated_under_20ms_count": 2,
        "observations": 30
      },
      "0:scalar_direct_calls_n10000:candidate": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 27774596.0,
          "median": 21364835.0,
          "min": 18612128.0,
          "p95": 25096952.0
        },
        "estimated_under_20ms_count": 9,
        "observations": 30
      },
      "0:scalar_direct_calls_n1024:baseline": {
        "batch": 17,
        "estimated_total_ns": {
          "max": 29076534.001,
          "median": 22732929.006,
          "min": 19467212.992,
          "p95": 28860110.996
        },
        "estimated_under_20ms_count": 1,
        "observations": 30
      },
      "0:scalar_direct_calls_n1024:candidate": {
        "batch": 17,
        "estimated_total_ns": {
          "max": 35929882.007,
          "median": 21737544.9975,
          "min": 18505688.997,
          "p95": 33025312.004
        },
        "estimated_under_20ms_count": 8,
        "observations": 30
      },
      "0:scalar_direct_calls_n17:baseline": {
        "batch": 1581,
        "estimated_total_ns": {
          "max": 194087849.253,
          "median": 37689100.9035,
          "min": 27585506.178,
          "p95": 82667528.787
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_direct_calls_n17:candidate": {
        "batch": 1581,
        "estimated_total_ns": {
          "max": 264085452.73200002,
          "median": 38179482.045,
          "min": 26167916.757,
          "p95": 121452050.046
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_direct_calls_n1:baseline": {
        "batch": 10390,
        "estimated_total_ns": {
          "max": 39642878.76,
          "median": 22941644.695,
          "min": 19941662.07,
          "p95": 32647541.12
        },
        "estimated_under_20ms_count": 1,
        "observations": 30
      },
      "0:scalar_direct_calls_n1:candidate": {
        "batch": 10390,
        "estimated_total_ns": {
          "max": 34565077.96,
          "median": 23237515.53,
          "min": 18835947.88,
          "p95": 32539931.89
        },
        "estimated_under_20ms_count": 6,
        "observations": 30
      },
      "0:scalar_direct_calls_n32:baseline": {
        "batch": 918,
        "estimated_total_ns": {
          "max": 55234709.622,
          "median": 40292573.715,
          "min": 34031576.412,
          "p95": 51928494.066
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_direct_calls_n32:candidate": {
        "batch": 918,
        "estimated_total_ns": {
          "max": 70781481.17999999,
          "median": 38532324.78,
          "min": 32571277.092000004,
          "p95": 53189546.993999995
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_dynamic_api_n0:baseline": {
        "batch": 44743,
        "estimated_total_ns": {
          "max": 32999439.019,
          "median": 18998392.3445,
          "min": 14713959.265,
          "p95": 27985448.953
        },
        "estimated_under_20ms_count": 16,
        "observations": 30
      },
      "0:scalar_dynamic_api_n0:candidate": {
        "batch": 44743,
        "estimated_total_ns": {
          "max": 55218052.188,
          "median": 15295752.493999999,
          "min": 13216411.055,
          "p95": 38269895.961
        },
        "estimated_under_20ms_count": 23,
        "observations": 30
      },
      "0:scalar_dynamic_api_n10000:baseline": {
        "batch": 16354,
        "estimated_total_ns": {
          "max": 17072938.194000002,
          "median": 7184173.191,
          "min": 5858199.0479999995,
          "p95": 13488664.722000001
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "0:scalar_dynamic_api_n10000:candidate": {
        "batch": 16354,
        "estimated_total_ns": {
          "max": 23202351.978,
          "median": 5961409.142,
          "min": 4928539.563999999,
          "p95": 9529066.95
        },
        "estimated_under_20ms_count": 29,
        "observations": 30
      },
      "0:scalar_dynamic_api_n1024:baseline": {
        "batch": 24661,
        "estimated_total_ns": {
          "max": 14606981.571,
          "median": 11083935.772,
          "min": 8102100.279,
          "p95": 13694302.622000001
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "0:scalar_dynamic_api_n1024:candidate": {
        "batch": 24661,
        "estimated_total_ns": {
          "max": 12353730.662,
          "median": 9553437.1205,
          "min": 6899161.359999999,
          "p95": 12249685.903
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "0:scalar_dynamic_api_n17:baseline": {
        "batch": 51814,
        "estimated_total_ns": {
          "max": 45203051.739999995,
          "median": 22653288.056,
          "min": 20056992.144,
          "p95": 36468040.364
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_dynamic_api_n17:candidate": {
        "batch": 51814,
        "estimated_total_ns": {
          "max": 36361096.268,
          "median": 18992395.792999998,
          "min": 16749445.454000002,
          "p95": 26977062.728000004
        },
        "estimated_under_20ms_count": 19,
        "observations": 30
      },
      "0:scalar_dynamic_api_n1:baseline": {
        "batch": 50000,
        "estimated_total_ns": {
          "max": 62525400.0,
          "median": 23966325.0,
          "min": 17610850.0,
          "p95": 46757700.0
        },
        "estimated_under_20ms_count": 4,
        "observations": 30
      },
      "0:scalar_dynamic_api_n1:candidate": {
        "batch": 50000,
        "estimated_total_ns": {
          "max": 44319950.0,
          "median": 22108900.0,
          "min": 15903400.0,
          "p95": 41109850.0
        },
        "estimated_under_20ms_count": 13,
        "observations": 30
      },
      "0:scalar_dynamic_api_n32:baseline": {
        "batch": 48662,
        "estimated_total_ns": {
          "max": 32791180.672,
          "median": 21134636.53,
          "min": 17895304.514000002,
          "p95": 31113363.573999997
        },
        "estimated_under_20ms_count": 8,
        "observations": 30
      },
      "0:scalar_dynamic_api_n32:candidate": {
        "batch": 48662,
        "estimated_total_ns": {
          "max": 44801497.554000005,
          "median": 18296255.063,
          "min": 15088431.691999998,
          "p95": 28444106.888
        },
        "estimated_under_20ms_count": 18,
        "observations": 30
      },
      "0:scalar_host_call_n0:baseline": {
        "batch": 25840,
        "estimated_total_ns": {
          "max": 46737919.12,
          "median": 24438916.439999998,
          "min": 20927273.36,
          "p95": 36453624.96
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_host_call_n0:candidate": {
        "batch": 25840,
        "estimated_total_ns": {
          "max": 74837963.04,
          "median": 20257733.12,
          "min": 18061927.44,
          "p95": 30172747.84
        },
        "estimated_under_20ms_count": 14,
        "observations": 30
      },
      "0:scalar_host_call_n10000:baseline": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 27528154.0,
          "median": 20266251.0,
          "min": 17154344.0,
          "p95": 27230021.0
        },
        "estimated_under_20ms_count": 12,
        "observations": 30
      },
      "0:scalar_host_call_n10000:candidate": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 27505219.0,
          "median": 19878837.5,
          "min": 16952989.0,
          "p95": 23858137.0
        },
        "estimated_under_20ms_count": 15,
        "observations": 30
      },
      "0:scalar_host_call_n1024:baseline": {
        "batch": 18,
        "estimated_total_ns": {
          "max": 23185596.996,
          "median": 17605881.998999998,
          "min": 14696697.006000001,
          "p95": 21785230.998
        },
        "estimated_under_20ms_count": 26,
        "observations": 30
      },
      "0:scalar_host_call_n1024:candidate": {
        "batch": 18,
        "estimated_total_ns": {
          "max": 35805814.992,
          "median": 16507903.5,
          "min": 15040434.996,
          "p95": 18694058.994
        },
        "estimated_under_20ms_count": 29,
        "observations": 30
      },
      "0:scalar_host_call_n17:baseline": {
        "batch": 92,
        "estimated_total_ns": {
          "max": 4484766.044,
          "median": 1872128.0099999998,
          "min": 999338.96,
          "p95": 4203775.964
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "0:scalar_host_call_n17:candidate": {
        "batch": 92,
        "estimated_total_ns": {
          "max": 4874755.976,
          "median": 1570094.494,
          "min": 1009448.012,
          "p95": 4358984.012
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "0:scalar_host_call_n1:baseline": {
        "batch": 10021,
        "estimated_total_ns": {
          "max": 61225694.518999994,
          "median": 24868288.9675,
          "min": 18582531.539,
          "p95": 59870885.38200001
        },
        "estimated_under_20ms_count": 5,
        "observations": 30
      },
      "0:scalar_host_call_n1:candidate": {
        "batch": 10021,
        "estimated_total_ns": {
          "max": 55847083.105000004,
          "median": 21252040.7605,
          "min": 16701519.692,
          "p95": 54013550.756
        },
        "estimated_under_20ms_count": 9,
        "observations": 30
      },
      "0:scalar_host_call_n32:baseline": {
        "batch": 462,
        "estimated_total_ns": {
          "max": 49249864.817999996,
          "median": 19337793.09,
          "min": 14068242.804,
          "p95": 48297364.037999995
        },
        "estimated_under_20ms_count": 17,
        "observations": 30
      },
      "0:scalar_host_call_n32:candidate": {
        "batch": 462,
        "estimated_total_ns": {
          "max": 68129731.824,
          "median": 20574762.053999998,
          "min": 14700337.806,
          "p95": 39231659.082
        },
        "estimated_under_20ms_count": 15,
        "observations": 30
      },
      "0:scalar_indirect_calls_n0:baseline": {
        "batch": 20834,
        "estimated_total_ns": {
          "max": 31926667.454,
          "median": 20177781.085,
          "min": 16948834.012000002,
          "p95": 28550788.596
        },
        "estimated_under_20ms_count": 14,
        "observations": 30
      },
      "0:scalar_indirect_calls_n0:candidate": {
        "batch": 20834,
        "estimated_total_ns": {
          "max": 25341790.077999998,
          "median": 18080078.544,
          "min": 15280468.126,
          "p95": 24724499.492000002
        },
        "estimated_under_20ms_count": 20,
        "observations": 30
      },
      "0:scalar_indirect_calls_n10000:baseline": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 31537256.0,
          "median": 25831500.5,
          "min": 22618717.0,
          "p95": 30406292.0
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_indirect_calls_n10000:candidate": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 49905243.0,
          "median": 24016246.0,
          "min": 20266292.0,
          "p95": 38337609.0
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_indirect_calls_n1024:baseline": {
        "batch": 12,
        "estimated_total_ns": {
          "max": 26206497.996,
          "median": 19242133.002,
          "min": 14719635.0,
          "p95": 24534480.996
        },
        "estimated_under_20ms_count": 17,
        "observations": 30
      },
      "0:scalar_indirect_calls_n1024:candidate": {
        "batch": 12,
        "estimated_total_ns": {
          "max": 34820694.0,
          "median": 19844979.0,
          "min": 14532833.003999999,
          "p95": 30043125.996
        },
        "estimated_under_20ms_count": 15,
        "observations": 30
      },
      "0:scalar_indirect_calls_n17:baseline": {
        "batch": 1555,
        "estimated_total_ns": {
          "max": 65516443.355,
          "median": 38068797.81,
          "min": 32925235.675,
          "p95": 62729531.925000004
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_indirect_calls_n17:candidate": {
        "batch": 1555,
        "estimated_total_ns": {
          "max": 59221708.49999999,
          "median": 38004005.625,
          "min": 31097739.029999997,
          "p95": 49591341.59
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_indirect_calls_n1:baseline": {
        "batch": 14065,
        "estimated_total_ns": {
          "max": 266951871.54999998,
          "median": 48052882.6225,
          "min": 28983154.77,
          "p95": 245187592.095
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_indirect_calls_n1:candidate": {
        "batch": 14065,
        "estimated_total_ns": {
          "max": 140810101.895,
          "median": 40082226.025000006,
          "min": 27122692.830000002,
          "p95": 85066695.28
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_indirect_calls_n32:baseline": {
        "batch": 857,
        "estimated_total_ns": {
          "max": 154403878.302,
          "median": 44548340.896,
          "min": 31916328.868,
          "p95": 99237217.421
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_indirect_calls_n32:candidate": {
        "batch": 857,
        "estimated_total_ns": {
          "max": 121277072.92799999,
          "median": 41491788.692,
          "min": 28567433.396,
          "p95": 76316665.86400001
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_multivalue_branch_n0:baseline": {
        "batch": 25543,
        "estimated_total_ns": {
          "max": 40427851.191,
          "median": 24981079.543,
          "min": 22272678.623999998,
          "p95": 39199207.348
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_multivalue_branch_n0:candidate": {
        "batch": 25543,
        "estimated_total_ns": {
          "max": 40017605.067999996,
          "median": 23010296.6065,
          "min": 18558138.935,
          "p95": 31685146.408999998
        },
        "estimated_under_20ms_count": 1,
        "observations": 30
      },
      "0:scalar_multivalue_branch_n10000:baseline": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 50391804.0,
          "median": 31676852.0,
          "min": 26091562.0,
          "p95": 48022407.0
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_multivalue_branch_n10000:candidate": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 44259073.0,
          "median": 31446900.5,
          "min": 26826209.0,
          "p95": 39539819.0
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_multivalue_branch_n1024:baseline": {
        "batch": 10,
        "estimated_total_ns": {
          "max": 40687585.0,
          "median": 16849801.0,
          "min": 14090450.0,
          "p95": 35835776.0
        },
        "estimated_under_20ms_count": 23,
        "observations": 30
      },
      "0:scalar_multivalue_branch_n1024:candidate": {
        "batch": 10,
        "estimated_total_ns": {
          "max": 40402240.0,
          "median": 16805855.0,
          "min": 12806413.0,
          "p95": 35502882.0
        },
        "estimated_under_20ms_count": 21,
        "observations": 30
      },
      "0:scalar_multivalue_branch_n17:baseline": {
        "batch": 957,
        "estimated_total_ns": {
          "max": 47162093.088,
          "median": 28905501.2235,
          "min": 26407785.294,
          "p95": 41108484.318
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_multivalue_branch_n17:candidate": {
        "batch": 957,
        "estimated_total_ns": {
          "max": 47607362.22,
          "median": 27189350.925,
          "min": 23394129.132000003,
          "p95": 43002890.699999996
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_multivalue_branch_n1:baseline": {
        "batch": 11326,
        "estimated_total_ns": {
          "max": 65985887.604,
          "median": 32027170.119000003,
          "min": 25895641.814000003,
          "p95": 59226009.808
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_multivalue_branch_n1:candidate": {
        "batch": 11326,
        "estimated_total_ns": {
          "max": 77814807.308,
          "median": 31460921.086,
          "min": 24325643.02,
          "p95": 55999164.452
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_multivalue_branch_n32:baseline": {
        "batch": 652,
        "estimated_total_ns": {
          "max": 54797669.260000005,
          "median": 33827353.488000005,
          "min": 30007207.9,
          "p95": 43879487.204
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_multivalue_branch_n32:candidate": {
        "batch": 652,
        "estimated_total_ns": {
          "max": 53668006.888,
          "median": 34193733.478,
          "min": 28740212.812,
          "p95": 52428020.9
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_ordinary_recursion_n0:baseline": {
        "batch": 8799,
        "estimated_total_ns": {
          "max": 50841449.106,
          "median": 10028264.295,
          "min": 7965690.705,
          "p95": 32782865.450999998
        },
        "estimated_under_20ms_count": 27,
        "observations": 30
      },
      "0:scalar_ordinary_recursion_n0:candidate": {
        "batch": 8799,
        "estimated_total_ns": {
          "max": 24904759.992,
          "median": 7907287.342499999,
          "min": 6246656.472,
          "p95": 10381887.306
        },
        "estimated_under_20ms_count": 29,
        "observations": 30
      },
      "0:scalar_ordinary_recursion_n17:baseline": {
        "batch": 1850,
        "estimated_total_ns": {
          "max": 79283226.30000001,
          "median": 51796099.275000006,
          "min": 42087442.65,
          "p95": 75119039.1
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_ordinary_recursion_n17:candidate": {
        "batch": 1850,
        "estimated_total_ns": {
          "max": 73469902.85000001,
          "median": 40778479.775,
          "min": 33213262.750000004,
          "p95": 55666622.1
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_ordinary_recursion_n1:baseline": {
        "batch": 15874,
        "estimated_total_ns": {
          "max": 163958196.52400002,
          "median": 35770471.599999994,
          "min": 27606600.392,
          "p95": 112310248.518
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_ordinary_recursion_n1:candidate": {
        "batch": 15874,
        "estimated_total_ns": {
          "max": 428543329.944,
          "median": 33323153.209,
          "min": 27268531.814,
          "p95": 59977845.379999995
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_ordinary_recursion_n32:baseline": {
        "batch": 875,
        "estimated_total_ns": {
          "max": 49835851.625,
          "median": 37036706.0,
          "min": 31093979.875,
          "p95": 49828043.125
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_ordinary_recursion_n32:candidate": {
        "batch": 875,
        "estimated_total_ns": {
          "max": 64669283.875,
          "median": 32114954.375,
          "min": 28574070.875,
          "p95": 42119400.75
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_table_mutation_n0:baseline": {
        "batch": 24450,
        "estimated_total_ns": {
          "max": 45725901.0,
          "median": 26273309.85,
          "min": 22276126.05,
          "p95": 31652578.8
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_table_mutation_n0:candidate": {
        "batch": 24450,
        "estimated_total_ns": {
          "max": 44523058.8,
          "median": 22761568.575000003,
          "min": 19381832.849999998,
          "p95": 32979382.499999996
        },
        "estimated_under_20ms_count": 2,
        "observations": 30
      },
      "0:scalar_table_mutation_n10000:baseline": {
        "batch": 1,
        "estimated_total_ns": {
          "max": 25070518.0,
          "median": 12764280.0,
          "min": 9930904.0,
          "p95": 21122986.0
        },
        "estimated_under_20ms_count": 27,
        "observations": 30
      },
      "0:scalar_table_mutation_n10000:candidate": {
        "batch": 1,
        "estimated_total_ns": {
          "max": 25128546.0,
          "median": 13609281.0,
          "min": 10549088.0,
          "p95": 19540138.0
        },
        "estimated_under_20ms_count": 29,
        "observations": 30
      },
      "0:scalar_table_mutation_n1024:baseline": {
        "batch": 16,
        "estimated_total_ns": {
          "max": 72468820.0,
          "median": 18748960.992,
          "min": 16405443.008,
          "p95": 25195227.008
        },
        "estimated_under_20ms_count": 21,
        "observations": 30
      },
      "0:scalar_table_mutation_n1024:candidate": {
        "batch": 16,
        "estimated_total_ns": {
          "max": 24575966.0,
          "median": 17700840.496,
          "min": 15371682.0,
          "p95": 23528304.992
        },
        "estimated_under_20ms_count": 23,
        "observations": 30
      },
      "0:scalar_table_mutation_n17:baseline": {
        "batch": 1614,
        "estimated_total_ns": {
          "max": 70619464.41,
          "median": 40549542.855000004,
          "min": 24739742.238,
          "p95": 69750832.206
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_table_mutation_n17:candidate": {
        "batch": 1614,
        "estimated_total_ns": {
          "max": 131971145.52600001,
          "median": 43263240.141,
          "min": 29358403.374,
          "p95": 112211480.988
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_table_mutation_n1:baseline": {
        "batch": 7154,
        "estimated_total_ns": {
          "max": 21807066.036,
          "median": 16205126.336,
          "min": 13764417.618,
          "p95": 20482953.638
        },
        "estimated_under_20ms_count": 28,
        "observations": 30
      },
      "0:scalar_table_mutation_n1:candidate": {
        "batch": 7154,
        "estimated_total_ns": {
          "max": 44030208.712,
          "median": 14755078.499000002,
          "min": 13293698.726,
          "p95": 19818597.428
        },
        "estimated_under_20ms_count": 29,
        "observations": 30
      },
      "0:scalar_table_mutation_n32:baseline": {
        "batch": 92,
        "estimated_total_ns": {
          "max": 7598212.012,
          "median": 3746154.4860000005,
          "min": 2680900.976,
          "p95": 6245947.988
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "0:scalar_table_mutation_n32:candidate": {
        "batch": 92,
        "estimated_total_ns": {
          "max": 8870873.036,
          "median": 3546405.512,
          "min": 2853890.968,
          "p95": 8314834.972
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "0:scalar_tail_calls_n0:baseline": {
        "batch": 9488,
        "estimated_total_ns": {
          "max": 12294939.408,
          "median": 10056549.424,
          "min": 7506175.024,
          "p95": 12283999.744
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "0:scalar_tail_calls_n0:candidate": {
        "batch": 9488,
        "estimated_total_ns": {
          "max": 14597250.048,
          "median": 8784028.352,
          "min": 7226231.584000001,
          "p95": 14238679.552
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "0:scalar_tail_calls_n10000:baseline": {
        "batch": 3,
        "estimated_total_ns": {
          "max": 49043202.0,
          "median": 25970979.0,
          "min": 20601390.0,
          "p95": 41168544.999
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_tail_calls_n10000:candidate": {
        "batch": 3,
        "estimated_total_ns": {
          "max": 42307931.001,
          "median": 24045112.5,
          "min": 18797034.0,
          "p95": 42125637.0
        },
        "estimated_under_20ms_count": 2,
        "observations": 30
      },
      "0:scalar_tail_calls_n1024:baseline": {
        "batch": 18,
        "estimated_total_ns": {
          "max": 32435602.992,
          "median": 18196057.998,
          "min": 14620737.006000001,
          "p95": 29775425.004
        },
        "estimated_under_20ms_count": 21,
        "observations": 30
      },
      "0:scalar_tail_calls_n1024:candidate": {
        "batch": 18,
        "estimated_total_ns": {
          "max": 24206059.998,
          "median": 16819524.999,
          "min": 13078081.998,
          "p95": 23306266.008
        },
        "estimated_under_20ms_count": 22,
        "observations": 30
      },
      "0:scalar_tail_calls_n17:baseline": {
        "batch": 1912,
        "estimated_total_ns": {
          "max": 52367003.199999996,
          "median": 34659772.352,
          "min": 28109541.416,
          "p95": 49697570.136
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_tail_calls_n17:candidate": {
        "batch": 1912,
        "estimated_total_ns": {
          "max": 38062048.248,
          "median": 29962908.024,
          "min": 23613079.544,
          "p95": 36622522.568
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_tail_calls_n1:baseline": {
        "batch": 7585,
        "estimated_total_ns": {
          "max": 49732205.42,
          "median": 15637766.95,
          "min": 11793886.16,
          "p95": 45989023.09
        },
        "estimated_under_20ms_count": 24,
        "observations": 30
      },
      "0:scalar_tail_calls_n1:candidate": {
        "batch": 7585,
        "estimated_total_ns": {
          "max": 162907929.74,
          "median": 15201071.9525,
          "min": 11330738.475,
          "p95": 36393846.39
        },
        "estimated_under_20ms_count": 24,
        "observations": 30
      },
      "0:scalar_tail_calls_n32:baseline": {
        "batch": 1018,
        "estimated_total_ns": {
          "max": 657872363.7739999,
          "median": 32182223.857,
          "min": 24629656.916,
          "p95": 487218520.79
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_tail_calls_n32:candidate": {
        "batch": 1018,
        "estimated_total_ns": {
          "max": 175036811.444,
          "median": 29441224.244999997,
          "min": 22625900.029999997,
          "p95": 135521586.958
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_try_table_normal_n0:baseline": {
        "batch": 6590,
        "estimated_total_ns": {
          "max": 13351603.6,
          "median": 6597239.115,
          "min": 5140694.25,
          "p95": 12369469.540000001
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "0:scalar_try_table_normal_n0:candidate": {
        "batch": 6590,
        "estimated_total_ns": {
          "max": 18659255.5,
          "median": 5338239.385,
          "min": 3559252.41,
          "p95": 15496931.97
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "0:scalar_try_table_normal_n10000:baseline": {
        "batch": 3,
        "estimated_total_ns": {
          "max": 52279919.001,
          "median": 32513476.5,
          "min": 27999923.001,
          "p95": 51446057.001
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_try_table_normal_n10000:candidate": {
        "batch": 3,
        "estimated_total_ns": {
          "max": 48958794.0,
          "median": 33926854.9995,
          "min": 28055204.001,
          "p95": 47801783.001
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_try_table_normal_n1024:baseline": {
        "batch": 15,
        "estimated_total_ns": {
          "max": 37395579.0,
          "median": 15852287.504999999,
          "min": 11558518.995,
          "p95": 35576268.0
        },
        "estimated_under_20ms_count": 18,
        "observations": 30
      },
      "0:scalar_try_table_normal_n1024:candidate": {
        "batch": 15,
        "estimated_total_ns": {
          "max": 484769008.005,
          "median": 18128333.505000003,
          "min": 12037813.995,
          "p95": 276426237.0
        },
        "estimated_under_20ms_count": 18,
        "observations": 30
      },
      "0:scalar_try_table_normal_n17:baseline": {
        "batch": 1941,
        "estimated_total_ns": {
          "max": 70365413.445,
          "median": 46526946.246,
          "min": 34347035.376,
          "p95": 62686130.331
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_try_table_normal_n17:candidate": {
        "batch": 1941,
        "estimated_total_ns": {
          "max": 109402772.448,
          "median": 45602523.645,
          "min": 33954064.398,
          "p95": 64141321.32299999
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_try_table_normal_n1:baseline": {
        "batch": 12415,
        "estimated_total_ns": {
          "max": 59068012.51,
          "median": 25074364.445,
          "min": 20869826.055,
          "p95": 54244487.050000004
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_try_table_normal_n1:candidate": {
        "batch": 12415,
        "estimated_total_ns": {
          "max": 53244893.325,
          "median": 22610533.205,
          "min": 19305002.21,
          "p95": 48740979.625
        },
        "estimated_under_20ms_count": 3,
        "observations": 30
      },
      "0:scalar_try_table_normal_n32:baseline": {
        "batch": 1139,
        "estimated_total_ns": {
          "max": 52629504.196,
          "median": 37675350.52150001,
          "min": 30847918.564999998,
          "p95": 52235445.504999995
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_try_table_normal_n32:candidate": {
        "batch": 1139,
        "estimated_total_ns": {
          "max": 67016134.605,
          "median": 36311087.074499995,
          "min": 29801421.311,
          "p95": 54828260.549
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:branch_calls:baseline": {
        "batch": 4,
        "estimated_total_ns": {
          "max": 94955241.0,
          "median": 32587827.5,
          "min": 26414847.0,
          "p95": 90193677.0
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:branch_calls:candidate": {
        "batch": 4,
        "estimated_total_ns": {
          "max": 196069596.0,
          "median": 36592673.5,
          "min": 26884242.0,
          "p95": 192557747.0
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:call_overhead:baseline": {
        "batch": 56981,
        "estimated_total_ns": {
          "max": 77705331.586,
          "median": 26487532.3785,
          "min": 21526851.990000002,
          "p95": 51602278.505
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:call_overhead:candidate": {
        "batch": 56981,
        "estimated_total_ns": {
          "max": 45986288.126,
          "median": 24928332.785000004,
          "min": 18435746.702,
          "p95": 45400352.503
        },
        "estimated_under_20ms_count": 3,
        "observations": 30
      },
      "1:integer_loop:baseline": {
        "batch": 3,
        "estimated_total_ns": {
          "max": 34213347.999,
          "median": 25574044.9995,
          "min": 21616125.998999998,
          "p95": 32090132.001
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:integer_loop:candidate": {
        "batch": 3,
        "estimated_total_ns": {
          "max": 44200194.0,
          "median": 25478034.4995,
          "min": 22838415.998999998,
          "p95": 30714233.001
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_accumulator_n0:baseline": {
        "batch": 8640,
        "estimated_total_ns": {
          "max": 14392814.4,
          "median": 7950178.08,
          "min": 6063050.88,
          "p95": 11806922.879999999
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "1:scalar_accumulator_n0:candidate": {
        "batch": 8640,
        "estimated_total_ns": {
          "max": 16036539.84,
          "median": 6901130.880000001,
          "min": 5427241.92,
          "p95": 11126073.6
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "1:scalar_accumulator_n10000:baseline": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 47258801.0,
          "median": 14209017.0,
          "min": 11418885.0,
          "p95": 25185480.0
        },
        "estimated_under_20ms_count": 28,
        "observations": 30
      },
      "1:scalar_accumulator_n10000:candidate": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 23271935.0,
          "median": 14095352.5,
          "min": 11631678.0,
          "p95": 21187254.0
        },
        "estimated_under_20ms_count": 28,
        "observations": 30
      },
      "1:scalar_accumulator_n1024:baseline": {
        "batch": 32,
        "estimated_total_ns": {
          "max": 34232335.008,
          "median": 23225434.0,
          "min": 18279040.992,
          "p95": 33876656.0
        },
        "estimated_under_20ms_count": 1,
        "observations": 30
      },
      "1:scalar_accumulator_n1024:candidate": {
        "batch": 32,
        "estimated_total_ns": {
          "max": 44648432.0,
          "median": 24169030.496,
          "min": 20433614.016,
          "p95": 31907768.0
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_accumulator_n17:baseline": {
        "batch": 2121,
        "estimated_total_ns": {
          "max": 41091736.476,
          "median": 28054464.879,
          "min": 24927737.583,
          "p95": 36639914.43000001
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_accumulator_n17:candidate": {
        "batch": 2121,
        "estimated_total_ns": {
          "max": 75523643.244,
          "median": 27406169.563500002,
          "min": 24513181.770000003,
          "p95": 39022121.943
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_accumulator_n1:baseline": {
        "batch": 5206,
        "estimated_total_ns": {
          "max": 27957235.169999998,
          "median": 7518382.859,
          "min": 6268945.461999999,
          "p95": 17153535.73
        },
        "estimated_under_20ms_count": 29,
        "observations": 30
      },
      "1:scalar_accumulator_n1:candidate": {
        "batch": 5206,
        "estimated_total_ns": {
          "max": 15702139.372,
          "median": 7035302.501,
          "min": 5775900.82,
          "p95": 12900410.734
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "1:scalar_accumulator_n32:baseline": {
        "batch": 590,
        "estimated_total_ns": {
          "max": 40164225.22,
          "median": 14659150.325,
          "min": 11553431.92,
          "p95": 27119840.29
        },
        "estimated_under_20ms_count": 25,
        "observations": 30
      },
      "1:scalar_accumulator_n32:candidate": {
        "batch": 590,
        "estimated_total_ns": {
          "max": 36609351.910000004,
          "median": 14824121.110000001,
          "min": 11640248.06,
          "p95": 28970977.09
        },
        "estimated_under_20ms_count": 21,
        "observations": 30
      },
      "1:scalar_br_table_n0:baseline": {
        "batch": 9129,
        "estimated_total_ns": {
          "max": 11141305.47,
          "median": 6606419.946,
          "min": 5483835.945,
          "p95": 11112202.218
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "1:scalar_br_table_n0:candidate": {
        "batch": 9129,
        "estimated_total_ns": {
          "max": 10545428.252999999,
          "median": 5950003.7655,
          "min": 4832353.989,
          "p95": 9656847.909
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "1:scalar_br_table_n10000:baseline": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 53414772.0,
          "median": 26381819.5,
          "min": 23373744.0,
          "p95": 39036725.0
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_br_table_n10000:candidate": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 35964062.0,
          "median": 25737103.0,
          "min": 22641208.0,
          "p95": 34974117.0
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_br_table_n1024:baseline": {
        "batch": 8,
        "estimated_total_ns": {
          "max": 17336300.0,
          "median": 11491028.5,
          "min": 9847185.0,
          "p95": 15610592.0
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "1:scalar_br_table_n1024:candidate": {
        "batch": 8,
        "estimated_total_ns": {
          "max": 15214146.0,
          "median": 11269066.5,
          "min": 9667631.0,
          "p95": 14456143.0
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "1:scalar_br_table_n17:baseline": {
        "batch": 1320,
        "estimated_total_ns": {
          "max": 118745758.55999999,
          "median": 31208681.46,
          "min": 27467745.360000003,
          "p95": 69348387.24
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_br_table_n17:candidate": {
        "batch": 1320,
        "estimated_total_ns": {
          "max": 46854941.76,
          "median": 30364806.78,
          "min": 26779629.360000003,
          "p95": 37795746.12
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_br_table_n1:baseline": {
        "batch": 7008,
        "estimated_total_ns": {
          "max": 37948439.136,
          "median": 16427999.424,
          "min": 14109682.944,
          "p95": 25883396.256
        },
        "estimated_under_20ms_count": 23,
        "observations": 30
      },
      "1:scalar_br_table_n1:candidate": {
        "batch": 7008,
        "estimated_total_ns": {
          "max": 36343095.552,
          "median": 15526497.311999999,
          "min": 13356154.752,
          "p95": 20986682.400000002
        },
        "estimated_under_20ms_count": 28,
        "observations": 30
      },
      "1:scalar_br_table_n32:baseline": {
        "batch": 313,
        "estimated_total_ns": {
          "max": 23802706.930999998,
          "median": 15242483.546500001,
          "min": 13619940.844,
          "p95": 22919850.993
        },
        "estimated_under_20ms_count": 26,
        "observations": 30
      },
      "1:scalar_br_table_n32:candidate": {
        "batch": 313,
        "estimated_total_ns": {
          "max": 44030727.876,
          "median": 15566790.515,
          "min": 12221203.001,
          "p95": 21454180.917
        },
        "estimated_under_20ms_count": 28,
        "observations": 30
      },
      "1:scalar_branch_entropy_n0:baseline": {
        "batch": 24331,
        "estimated_total_ns": {
          "max": 40950849.163,
          "median": 22951736.4375,
          "min": 19994339.884,
          "p95": 36899470.022
        },
        "estimated_under_20ms_count": 1,
        "observations": 30
      },
      "1:scalar_branch_entropy_n0:candidate": {
        "batch": 24331,
        "estimated_total_ns": {
          "max": 122508993.76900001,
          "median": 20530290.986500002,
          "min": 18035840.37,
          "p95": 39249990.607999995
        },
        "estimated_under_20ms_count": 12,
        "observations": 30
      },
      "1:scalar_branch_entropy_n10000:baseline": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 45623278.0,
          "median": 30296100.0,
          "min": 21832330.0,
          "p95": 38076255.0
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_branch_entropy_n10000:candidate": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 46811155.0,
          "median": 27670685.0,
          "min": 23324192.0,
          "p95": 42443263.0
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_branch_entropy_n1024:baseline": {
        "batch": 17,
        "estimated_total_ns": {
          "max": 35838761.004,
          "median": 25721664.5045,
          "min": 22917028.007,
          "p95": 35797565.992
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_branch_entropy_n1024:candidate": {
        "batch": 17,
        "estimated_total_ns": {
          "max": 41680381.006,
          "median": 25440490.9985,
          "min": 22086648.999,
          "p95": 32793683.995
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_branch_entropy_n17:baseline": {
        "batch": 491,
        "estimated_total_ns": {
          "max": 25944522.979000002,
          "median": 11682384.113,
          "min": 10321026.219999999,
          "p95": 20845751.155
        },
        "estimated_under_20ms_count": 28,
        "observations": 30
      },
      "1:scalar_branch_entropy_n17:candidate": {
        "batch": 491,
        "estimated_total_ns": {
          "max": 27665585.017,
          "median": 11587381.996,
          "min": 10181601.86,
          "p95": 17136260.885
        },
        "estimated_under_20ms_count": 29,
        "observations": 30
      },
      "1:scalar_branch_entropy_n1:baseline": {
        "batch": 11772,
        "estimated_total_ns": {
          "max": 41946225.839999996,
          "median": 30349558.008,
          "min": 24676372.224,
          "p95": 40469840.46
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_branch_entropy_n1:candidate": {
        "batch": 11772,
        "estimated_total_ns": {
          "max": 48044251.332,
          "median": 27268596.054,
          "min": 22652223.912,
          "p95": 35944683.660000004
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_branch_entropy_n32:baseline": {
        "batch": 121,
        "estimated_total_ns": {
          "max": 15781621.988,
          "median": 6114590.041999999,
          "min": 4538223.943,
          "p95": 10094108.948
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "1:scalar_branch_entropy_n32:candidate": {
        "batch": 121,
        "estimated_total_ns": {
          "max": 10894042.973,
          "median": 6058648.5355,
          "min": 5079946.025,
          "p95": 7689228.019
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "1:scalar_call_ref_n0:baseline": {
        "batch": 25189,
        "estimated_total_ns": {
          "max": 37922115.067,
          "median": 22072113.14,
          "min": 19501978.714,
          "p95": 32776480.958
        },
        "estimated_under_20ms_count": 2,
        "observations": 30
      },
      "1:scalar_call_ref_n0:candidate": {
        "batch": 25189,
        "estimated_total_ns": {
          "max": 39154537.27,
          "median": 20109361.071000002,
          "min": 17271316.441,
          "p95": 27308855.861999996
        },
        "estimated_under_20ms_count": 15,
        "observations": 30
      },
      "1:scalar_call_ref_n10000:baseline": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 34676025.0,
          "median": 23647886.0,
          "min": 21065297.0,
          "p95": 31107651.0
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_call_ref_n10000:candidate": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 34912541.0,
          "median": 22867452.0,
          "min": 19706056.0,
          "p95": 30644446.0
        },
        "estimated_under_20ms_count": 1,
        "observations": 30
      },
      "1:scalar_call_ref_n1024:baseline": {
        "batch": 20,
        "estimated_total_ns": {
          "max": 54452303.0,
          "median": 23718640.0,
          "min": 21085146.0,
          "p95": 36391528.0
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_call_ref_n1024:candidate": {
        "batch": 20,
        "estimated_total_ns": {
          "max": 46819972.0,
          "median": 23161845.5,
          "min": 19083027.0,
          "p95": 45576796.0
        },
        "estimated_under_20ms_count": 2,
        "observations": 30
      },
      "1:scalar_call_ref_n17:baseline": {
        "batch": 1453,
        "estimated_total_ns": {
          "max": 60506873.613,
          "median": 30120145.125,
          "min": 27542095.942999996,
          "p95": 47572135.39
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_call_ref_n17:candidate": {
        "batch": 1453,
        "estimated_total_ns": {
          "max": 41109717.375999995,
          "median": 28307317.666500002,
          "min": 25495202.535,
          "p95": 37434426.526
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_call_ref_n1:baseline": {
        "batch": 5584,
        "estimated_total_ns": {
          "max": 16009718.88,
          "median": 11007317.608,
          "min": 9735921.776,
          "p95": 14249005.503999999
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "1:scalar_call_ref_n1:candidate": {
        "batch": 5584,
        "estimated_total_ns": {
          "max": 14354582.192,
          "median": 10266996.472,
          "min": 9200991.328,
          "p95": 14293264.287999999
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "1:scalar_call_ref_n32:baseline": {
        "batch": 821,
        "estimated_total_ns": {
          "max": 180351368.79999998,
          "median": 32862914.931,
          "min": 27799998.402999997,
          "p95": 62060076.356
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_call_ref_n32:candidate": {
        "batch": 821,
        "estimated_total_ns": {
          "max": 109055009.604,
          "median": 30910342.125,
          "min": 25944765.82,
          "p95": 95351137.861
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_dependent_n0:baseline": {
        "batch": 23202,
        "estimated_total_ns": {
          "max": 58113098.118,
          "median": 21247556.328,
          "min": 17541060.03,
          "p95": 28899181.494
        },
        "estimated_under_20ms_count": 12,
        "observations": 30
      },
      "1:scalar_dependent_n0:candidate": {
        "batch": 23202,
        "estimated_total_ns": {
          "max": 22041250.344,
          "median": 17518240.862999998,
          "min": 15847638.858,
          "p95": 21637117.908
        },
        "estimated_under_20ms_count": 23,
        "observations": 30
      },
      "1:scalar_dependent_n10000:baseline": {
        "batch": 3,
        "estimated_total_ns": {
          "max": 26963225.001,
          "median": 21980570.001000002,
          "min": 18598793.001000002,
          "p95": 25881060.0
        },
        "estimated_under_20ms_count": 9,
        "observations": 30
      },
      "1:scalar_dependent_n10000:candidate": {
        "batch": 3,
        "estimated_total_ns": {
          "max": 52314006.0,
          "median": 21271338.0,
          "min": 18701609.001000002,
          "p95": 33589257.0
        },
        "estimated_under_20ms_count": 9,
        "observations": 30
      },
      "1:scalar_dependent_n1024:baseline": {
        "batch": 20,
        "estimated_total_ns": {
          "max": 35054010.0,
          "median": 16963216.5,
          "min": 13413478.0,
          "p95": 34906581.0
        },
        "estimated_under_20ms_count": 25,
        "observations": 30
      },
      "1:scalar_dependent_n1024:candidate": {
        "batch": 20,
        "estimated_total_ns": {
          "max": 34673379.0,
          "median": 16829322.0,
          "min": 13655688.0,
          "p95": 24567328.0
        },
        "estimated_under_20ms_count": 25,
        "observations": 30
      },
      "1:scalar_dependent_n17:baseline": {
        "batch": 1743,
        "estimated_total_ns": {
          "max": 39357797.555999994,
          "median": 28040073.264,
          "min": 22757685.174000002,
          "p95": 37211690.46
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_dependent_n17:candidate": {
        "batch": 1743,
        "estimated_total_ns": {
          "max": 52096077.305999994,
          "median": 27761148.8175,
          "min": 21829436.58,
          "p95": 39406566.696
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_dependent_n1:baseline": {
        "batch": 11351,
        "estimated_total_ns": {
          "max": 42631359.335999995,
          "median": 18964757.031499997,
          "min": 16945192.787,
          "p95": 27326215.784
        },
        "estimated_under_20ms_count": 21,
        "observations": 30
      },
      "1:scalar_dependent_n1:candidate": {
        "batch": 11351,
        "estimated_total_ns": {
          "max": 25873049.413,
          "median": 17992782.252499998,
          "min": 15688523.577,
          "p95": 23423878.196
        },
        "estimated_under_20ms_count": 20,
        "observations": 30
      },
      "1:scalar_dependent_n32:baseline": {
        "batch": 1051,
        "estimated_total_ns": {
          "max": 47853646.438,
          "median": 28253095.508,
          "min": 24923223.769,
          "p95": 44962517.802
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_dependent_n32:candidate": {
        "batch": 1051,
        "estimated_total_ns": {
          "max": 39156751.761999995,
          "median": 28470246.296499997,
          "min": 25017294.575,
          "p95": 32931117.528
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_direct_calls_n0:baseline": {
        "batch": 23365,
        "estimated_total_ns": {
          "max": 32026872.8,
          "median": 21731926.69,
          "min": 19254629.2,
          "p95": 28153656.75
        },
        "estimated_under_20ms_count": 2,
        "observations": 30
      },
      "1:scalar_direct_calls_n0:candidate": {
        "batch": 23365,
        "estimated_total_ns": {
          "max": 47684133.14,
          "median": 19801638.8975,
          "min": 18183414.044999998,
          "p95": 28767128.189999998
        },
        "estimated_under_20ms_count": 19,
        "observations": 30
      },
      "1:scalar_direct_calls_n10000:baseline": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 66774060.0,
          "median": 25490268.0,
          "min": 21478085.0,
          "p95": 35473619.0
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_direct_calls_n10000:candidate": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 36463753.0,
          "median": 23305253.5,
          "min": 18983179.0,
          "p95": 34850336.0
        },
        "estimated_under_20ms_count": 2,
        "observations": 30
      },
      "1:scalar_direct_calls_n1024:baseline": {
        "batch": 10,
        "estimated_total_ns": {
          "max": 14561488.0,
          "median": 11386015.5,
          "min": 10078351.0,
          "p95": 13573057.0
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "1:scalar_direct_calls_n1024:candidate": {
        "batch": 10,
        "estimated_total_ns": {
          "max": 50859940.0,
          "median": 10502854.5,
          "min": 9393398.0,
          "p95": 20137561.0
        },
        "estimated_under_20ms_count": 28,
        "observations": 30
      },
      "1:scalar_direct_calls_n17:baseline": {
        "batch": 198,
        "estimated_total_ns": {
          "max": 4954516.973999999,
          "median": 3619934.505,
          "min": 2926161.018,
          "p95": 4890950.064
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "1:scalar_direct_calls_n17:candidate": {
        "batch": 198,
        "estimated_total_ns": {
          "max": 10144286.064,
          "median": 3516091.524,
          "min": 3089798.91,
          "p95": 6987939.947999999
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "1:scalar_direct_calls_n1:baseline": {
        "batch": 11050,
        "estimated_total_ns": {
          "max": 43049462.949999996,
          "median": 20339536.1,
          "min": 17712243.9,
          "p95": 30715187.750000004
        },
        "estimated_under_20ms_count": 11,
        "observations": 30
      },
      "1:scalar_direct_calls_n1:candidate": {
        "batch": 11050,
        "estimated_total_ns": {
          "max": 36581212.85,
          "median": 19755919.299999997,
          "min": 15919414.55,
          "p95": 26860980.900000002
        },
        "estimated_under_20ms_count": 15,
        "observations": 30
      },
      "1:scalar_direct_calls_n32:baseline": {
        "batch": 839,
        "estimated_total_ns": {
          "max": 55011072.092,
          "median": 30634503.9045,
          "min": 26061440.856000002,
          "p95": 42989677.893
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_direct_calls_n32:candidate": {
        "batch": 839,
        "estimated_total_ns": {
          "max": 52937450.032,
          "median": 29640600.1735,
          "min": 24595557.675,
          "p95": 43172052.162
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_dynamic_api_n0:baseline": {
        "batch": 26144,
        "estimated_total_ns": {
          "max": 28581640.416,
          "median": 9147798.672,
          "min": 7738336.415999999,
          "p95": 16286666.24
        },
        "estimated_under_20ms_count": 29,
        "observations": 30
      },
      "1:scalar_dynamic_api_n0:candidate": {
        "batch": 26144,
        "estimated_total_ns": {
          "max": 10091113.408,
          "median": 7439445.136,
          "min": 6584680.128,
          "p95": 9786666.527999999
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "1:scalar_dynamic_api_n10000:baseline": {
        "batch": 46404,
        "estimated_total_ns": {
          "max": 17929299.096,
          "median": 15167449.026,
          "min": 13596232.788,
          "p95": 17821734.623999998
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "1:scalar_dynamic_api_n10000:candidate": {
        "batch": 46404,
        "estimated_total_ns": {
          "max": 16855556.94,
          "median": 13344143.058,
          "min": 11849586.228,
          "p95": 16811241.119999997
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "1:scalar_dynamic_api_n1024:baseline": {
        "batch": 49876,
        "estimated_total_ns": {
          "max": 36065335.6,
          "median": 19108243.740000002,
          "min": 16814147.244000003,
          "p95": 30261913.868000004
        },
        "estimated_under_20ms_count": 22,
        "observations": 30
      },
      "1:scalar_dynamic_api_n1024:candidate": {
        "batch": 49876,
        "estimated_total_ns": {
          "max": 33723807.028000005,
          "median": 16976618.314,
          "min": 14140195.132000001,
          "p95": 23078921.976
        },
        "estimated_under_20ms_count": 26,
        "observations": 30
      },
      "1:scalar_dynamic_api_n17:baseline": {
        "batch": 18019,
        "estimated_total_ns": {
          "max": 10506716.729,
          "median": 6544158.439,
          "min": 5367121.3209999995,
          "p95": 8501093.915000001
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "1:scalar_dynamic_api_n17:candidate": {
        "batch": 18019,
        "estimated_total_ns": {
          "max": 10128227.634,
          "median": 5633793.5115,
          "min": 4597926.249,
          "p95": 7464586.978
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "1:scalar_dynamic_api_n1:baseline": {
        "batch": 51021,
        "estimated_total_ns": {
          "max": 23115727.323,
          "median": 16771444.5465,
          "min": 14562056.673,
          "p95": 22559598.423
        },
        "estimated_under_20ms_count": 27,
        "observations": 30
      },
      "1:scalar_dynamic_api_n1:candidate": {
        "batch": 51021,
        "estimated_total_ns": {
          "max": 28331349.048,
          "median": 14326390.674,
          "min": 13012906.05,
          "p95": 23537875.077
        },
        "estimated_under_20ms_count": 28,
        "observations": 30
      },
      "1:scalar_dynamic_api_n32:baseline": {
        "batch": 49876,
        "estimated_total_ns": {
          "max": 46272758.256,
          "median": 17009037.714,
          "min": 15183501.3,
          "p95": 19832493.136
        },
        "estimated_under_20ms_count": 29,
        "observations": 30
      },
      "1:scalar_dynamic_api_n32:candidate": {
        "batch": 49876,
        "estimated_total_ns": {
          "max": 44621762.904,
          "median": 14309474.276,
          "min": 12750200.888,
          "p95": 19618674.724
        },
        "estimated_under_20ms_count": 29,
        "observations": 30
      },
      "1:scalar_host_call_n0:baseline": {
        "batch": 26110,
        "estimated_total_ns": {
          "max": 45344401.48,
          "median": 21198017.085,
          "min": 15585555.09,
          "p95": 43190744.24
        },
        "estimated_under_20ms_count": 12,
        "observations": 30
      },
      "1:scalar_host_call_n0:candidate": {
        "batch": 26110,
        "estimated_total_ns": {
          "max": 84717628.73,
          "median": 16761693.094999999,
          "min": 14746954.110000001,
          "p95": 44304596.84
        },
        "estimated_under_20ms_count": 17,
        "observations": 30
      },
      "1:scalar_host_call_n10000:baseline": {
        "batch": 3,
        "estimated_total_ns": {
          "max": 75923496.999,
          "median": 33062475.0,
          "min": 25330055.001,
          "p95": 71542289.001
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_host_call_n10000:candidate": {
        "batch": 3,
        "estimated_total_ns": {
          "max": 212455668.999,
          "median": 32481415.0005,
          "min": 26309766.0,
          "p95": 71986509.0
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_host_call_n1024:baseline": {
        "batch": 10,
        "estimated_total_ns": {
          "max": 25356853.0,
          "median": 19071765.5,
          "min": 10930810.0,
          "p95": 23100989.0
        },
        "estimated_under_20ms_count": 23,
        "observations": 30
      },
      "1:scalar_host_call_n1024:candidate": {
        "batch": 10,
        "estimated_total_ns": {
          "max": 57019331.0,
          "median": 17762523.0,
          "min": 11207924.0,
          "p95": 26192733.0
        },
        "estimated_under_20ms_count": 26,
        "observations": 30
      },
      "1:scalar_host_call_n17:baseline": {
        "batch": 250,
        "estimated_total_ns": {
          "max": 9016856.0,
          "median": 4283859.5,
          "min": 3311352.0,
          "p95": 7814819.0
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "1:scalar_host_call_n17:candidate": {
        "batch": 250,
        "estimated_total_ns": {
          "max": 8894718.0,
          "median": 4372720.0,
          "min": 3165257.0,
          "p95": 6721502.0
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "1:scalar_host_call_n1:baseline": {
        "batch": 13132,
        "estimated_total_ns": {
          "max": 44394288.708000004,
          "median": 31152019.303999998,
          "min": 21036150.8,
          "p95": 42324606.716
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_host_call_n1:candidate": {
        "batch": 13132,
        "estimated_total_ns": {
          "max": 49205485.812,
          "median": 26208103.642,
          "min": 20403542.964,
          "p95": 35545237.98
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_host_call_n32:baseline": {
        "batch": 1160,
        "estimated_total_ns": {
          "max": 114056438.56,
          "median": 56995839.04000001,
          "min": 32431212.72,
          "p95": 88560687.2
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_host_call_n32:candidate": {
        "batch": 1160,
        "estimated_total_ns": {
          "max": 111326891.28,
          "median": 58347318.5,
          "min": 31869901.48,
          "p95": 100017298.44
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_indirect_calls_n0:baseline": {
        "batch": 25032,
        "estimated_total_ns": {
          "max": 34861240.344,
          "median": 19682348.7,
          "min": 17864987.952,
          "p95": 34353866.736
        },
        "estimated_under_20ms_count": 17,
        "observations": 30
      },
      "1:scalar_indirect_calls_n0:candidate": {
        "batch": 25032,
        "estimated_total_ns": {
          "max": 32425226.232000005,
          "median": 18343524.696,
          "min": 15981179.759999998,
          "p95": 24729388.152
        },
        "estimated_under_20ms_count": 21,
        "observations": 30
      },
      "1:scalar_indirect_calls_n10000:baseline": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 51163417.0,
          "median": 24570936.5,
          "min": 17826851.0,
          "p95": 46551302.0
        },
        "estimated_under_20ms_count": 6,
        "observations": 30
      },
      "1:scalar_indirect_calls_n10000:candidate": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 90085831.0,
          "median": 23911085.0,
          "min": 16709185.0,
          "p95": 52355642.0
        },
        "estimated_under_20ms_count": 7,
        "observations": 30
      },
      "1:scalar_indirect_calls_n1024:baseline": {
        "batch": 15,
        "estimated_total_ns": {
          "max": 31687803.000000004,
          "median": 19251633.5025,
          "min": 16857769.005,
          "p95": 29437261.005
        },
        "estimated_under_20ms_count": 18,
        "observations": 30
      },
      "1:scalar_indirect_calls_n1024:candidate": {
        "batch": 15,
        "estimated_total_ns": {
          "max": 30094435.005,
          "median": 18274720.995,
          "min": 15614995.005,
          "p95": 27395908.005000003
        },
        "estimated_under_20ms_count": 23,
        "observations": 30
      },
      "1:scalar_indirect_calls_n17:baseline": {
        "batch": 1543,
        "estimated_total_ns": {
          "max": 47450203.302,
          "median": 31858209.904,
          "min": 27853405.866,
          "p95": 38376346.465
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_indirect_calls_n17:candidate": {
        "batch": 1543,
        "estimated_total_ns": {
          "max": 34912240.486999996,
          "median": 28716732.1105,
          "min": 25454826.253000002,
          "p95": 34017004.231
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_indirect_calls_n1:baseline": {
        "batch": 13587,
        "estimated_total_ns": {
          "max": 40114938.15,
          "median": 30515613.954,
          "min": 26686145.178,
          "p95": 36959167.182000004
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_indirect_calls_n1:candidate": {
        "batch": 13587,
        "estimated_total_ns": {
          "max": 48639693.69,
          "median": 27622608.7725,
          "min": 25234374.228,
          "p95": 35112327.033
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_indirect_calls_n32:baseline": {
        "batch": 859,
        "estimated_total_ns": {
          "max": 69378364.229,
          "median": 34725407.8625,
          "min": 30060473.929,
          "p95": 45159376.347
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_indirect_calls_n32:candidate": {
        "batch": 859,
        "estimated_total_ns": {
          "max": 40306802.024000004,
          "median": 31831460.0555,
          "min": 28621409.268,
          "p95": 38580207.728999995
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_multivalue_branch_n0:baseline": {
        "batch": 25032,
        "estimated_total_ns": {
          "max": 20445136.32,
          "median": 17683030.344,
          "min": 16354932.552,
          "p95": 20085701.832
        },
        "estimated_under_20ms_count": 28,
        "observations": 30
      },
      "1:scalar_multivalue_branch_n0:candidate": {
        "batch": 25032,
        "estimated_total_ns": {
          "max": 18519925.2,
          "median": 15903705.719999999,
          "min": 14565419.904,
          "p95": 18170853.96
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "1:scalar_multivalue_branch_n10000:baseline": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 36001127.0,
          "median": 25468587.0,
          "min": 23712239.0,
          "p95": 32149415.0
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_multivalue_branch_n10000:candidate": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 42176581.0,
          "median": 24448996.5,
          "min": 22147185.0,
          "p95": 30601680.0
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_multivalue_branch_n1024:baseline": {
        "batch": 12,
        "estimated_total_ns": {
          "max": 24657882.0,
          "median": 16855935.996,
          "min": 14983007.003999999,
          "p95": 19278783.0
        },
        "estimated_under_20ms_count": 29,
        "observations": 30
      },
      "1:scalar_multivalue_branch_n1024:candidate": {
        "batch": 12,
        "estimated_total_ns": {
          "max": 38248929.0,
          "median": 16910064.0,
          "min": 15024933.0,
          "p95": 26027361.996
        },
        "estimated_under_20ms_count": 28,
        "observations": 30
      },
      "1:scalar_multivalue_branch_n17:baseline": {
        "batch": 1214,
        "estimated_total_ns": {
          "max": 36318223.096,
          "median": 30064386.469,
          "min": 27516010.478,
          "p95": 34746178.076
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_multivalue_branch_n17:candidate": {
        "batch": 1214,
        "estimated_total_ns": {
          "max": 47720015.19,
          "median": 28575464.029,
          "min": 26187955.178,
          "p95": 36509560.422
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_multivalue_branch_n1:baseline": {
        "batch": 11628,
        "estimated_total_ns": {
          "max": 56218031.13600001,
          "median": 25488552.744000003,
          "min": 22845612.996000003,
          "p95": 39152638.8
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_multivalue_branch_n1:candidate": {
        "batch": 11628,
        "estimated_total_ns": {
          "max": 35253375.048,
          "median": 24111094.049999997,
          "min": 20370918.78,
          "p95": 30011588.928000003
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_multivalue_branch_n32:baseline": {
        "batch": 272,
        "estimated_total_ns": {
          "max": 22187059.040000003,
          "median": 11708931.968,
          "min": 10676647.088,
          "p95": 13882856.88
        },
        "estimated_under_20ms_count": 29,
        "observations": 30
      },
      "1:scalar_multivalue_branch_n32:candidate": {
        "batch": 272,
        "estimated_total_ns": {
          "max": 16185058.896,
          "median": 11466789.952,
          "min": 10107712.032,
          "p95": 13521145.023999998
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "1:scalar_ordinary_recursion_n0:baseline": {
        "batch": 9644,
        "estimated_total_ns": {
          "max": 12822990.296,
          "median": 8266426.93,
          "min": 7171847.396,
          "p95": 10773910.328
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "1:scalar_ordinary_recursion_n0:candidate": {
        "batch": 9644,
        "estimated_total_ns": {
          "max": 11397520.3,
          "median": 6796006.25,
          "min": 6161522.6680000005,
          "p95": 8021136.612
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "1:scalar_ordinary_recursion_n17:baseline": {
        "batch": 1836,
        "estimated_total_ns": {
          "max": 44947986.264,
          "median": 33180476.580000002,
          "min": 29313372.204,
          "p95": 38265888.132
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_ordinary_recursion_n17:candidate": {
        "batch": 1836,
        "estimated_total_ns": {
          "max": 32650559.244,
          "median": 27099781.362,
          "min": 24557888.016,
          "p95": 30882351.708000004
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_ordinary_recursion_n1:baseline": {
        "batch": 15397,
        "estimated_total_ns": {
          "max": 29366975.246,
          "median": 25134339.946000002,
          "min": 23092497.585,
          "p95": 28795330.827
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_ordinary_recursion_n1:candidate": {
        "batch": 15397,
        "estimated_total_ns": {
          "max": 39780690.005,
          "median": 24880997.708,
          "min": 22206692.777999997,
          "p95": 35391513.405999996
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_ordinary_recursion_n32:baseline": {
        "batch": 848,
        "estimated_total_ns": {
          "max": 33560800.768,
          "median": 27384623.424,
          "min": 23964422.336,
          "p95": 31685748.959999997
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_ordinary_recursion_n32:candidate": {
        "batch": 848,
        "estimated_total_ns": {
          "max": 30319837.200000003,
          "median": 24270914.552,
          "min": 21815877.808000002,
          "p95": 28754853.200000003
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_table_mutation_n0:baseline": {
        "batch": 22447,
        "estimated_total_ns": {
          "max": 31145796.122,
          "median": 18624848.2985,
          "min": 16545436.783000002,
          "p95": 29776798.486
        },
        "estimated_under_20ms_count": 22,
        "observations": 30
      },
      "1:scalar_table_mutation_n0:candidate": {
        "batch": 22447,
        "estimated_total_ns": {
          "max": 29847416.748,
          "median": 17062099.382,
          "min": 15064024.570999999,
          "p95": 25820806.547
        },
        "estimated_under_20ms_count": 24,
        "observations": 30
      },
      "1:scalar_table_mutation_n10000:baseline": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 33994766.0,
          "median": 23356612.0,
          "min": 19263769.0,
          "p95": 27924382.0
        },
        "estimated_under_20ms_count": 8,
        "observations": 30
      },
      "1:scalar_table_mutation_n10000:candidate": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 64312225.0,
          "median": 22211875.5,
          "min": 18899641.0,
          "p95": 51811784.0
        },
        "estimated_under_20ms_count": 5,
        "observations": 30
      },
      "1:scalar_table_mutation_n1024:baseline": {
        "batch": 20,
        "estimated_total_ns": {
          "max": 51357335.0,
          "median": 22804084.5,
          "min": 19512279.0,
          "p95": 33443981.0
        },
        "estimated_under_20ms_count": 1,
        "observations": 30
      },
      "1:scalar_table_mutation_n1024:candidate": {
        "batch": 20,
        "estimated_total_ns": {
          "max": 32978757.0,
          "median": 21179243.5,
          "min": 19091700.0,
          "p95": 29893460.0
        },
        "estimated_under_20ms_count": 7,
        "observations": 30
      },
      "1:scalar_table_mutation_n17:baseline": {
        "batch": 1610,
        "estimated_total_ns": {
          "max": 116942097.22999999,
          "median": 31549985.845,
          "min": 26712926.8,
          "p95": 61648477.800000004
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_table_mutation_n17:candidate": {
        "batch": 1610,
        "estimated_total_ns": {
          "max": 146931737.89,
          "median": 31203327.89,
          "min": 26021111.41,
          "p95": 72867484.27
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_table_mutation_n1:baseline": {
        "batch": 8443,
        "estimated_total_ns": {
          "max": 22383093.769,
          "median": 16555929.358,
          "min": 14228101.385,
          "p95": 20140269.92
        },
        "estimated_under_20ms_count": 28,
        "observations": 30
      },
      "1:scalar_table_mutation_n1:candidate": {
        "batch": 8443,
        "estimated_total_ns": {
          "max": 19520807.01,
          "median": 15593760.8565,
          "min": 13446330.243,
          "p95": 19483548.051000003
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "1:scalar_table_mutation_n32:baseline": {
        "batch": 880,
        "estimated_total_ns": {
          "max": 217323751.92000002,
          "median": 36607814.76,
          "min": 25983058.64,
          "p95": 148031171.2
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_table_mutation_n32:candidate": {
        "batch": 880,
        "estimated_total_ns": {
          "max": 360321887.2,
          "median": 41570433.96,
          "min": 25803535.12,
          "p95": 110949657.28
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_tail_calls_n0:baseline": {
        "batch": 8893,
        "estimated_total_ns": {
          "max": 13094035.413999999,
          "median": 7479017.4465,
          "min": 6581069.004,
          "p95": 10262886.613
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "1:scalar_tail_calls_n0:candidate": {
        "batch": 8893,
        "estimated_total_ns": {
          "max": 29211833.116,
          "median": 6798867.467,
          "min": 5747110.143,
          "p95": 8561166.598000001
        },
        "estimated_under_20ms_count": 29,
        "observations": 30
      },
      "1:scalar_tail_calls_n10000:baseline": {
        "batch": 4,
        "estimated_total_ns": {
          "max": 59309822.0,
          "median": 27356723.5,
          "min": 24626335.0,
          "p95": 35398621.0
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_tail_calls_n10000:candidate": {
        "batch": 4,
        "estimated_total_ns": {
          "max": 52335560.0,
          "median": 25149883.0,
          "min": 22388192.0,
          "p95": 31608353.0
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_tail_calls_n1024:baseline": {
        "batch": 26,
        "estimated_total_ns": {
          "max": 36184145.01,
          "median": 17474767.998999998,
          "min": 16038652.993999999,
          "p95": 26102407.006
        },
        "estimated_under_20ms_count": 26,
        "observations": 30
      },
      "1:scalar_tail_calls_n1024:candidate": {
        "batch": 26,
        "estimated_total_ns": {
          "max": 20266993.006,
          "median": 15813519.500999998,
          "min": 14663579.008,
          "p95": 19683765.01
        },
        "estimated_under_20ms_count": 29,
        "observations": 30
      },
      "1:scalar_tail_calls_n17:baseline": {
        "batch": 1964,
        "estimated_total_ns": {
          "max": 41945318.868,
          "median": 23346758.346,
          "min": 21188596.324,
          "p95": 27747521.624
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_tail_calls_n17:candidate": {
        "batch": 1964,
        "estimated_total_ns": {
          "max": 28004617.08,
          "median": 20785303.654,
          "min": 19059323.76,
          "p95": 26593636.272
        },
        "estimated_under_20ms_count": 8,
        "observations": 30
      },
      "1:scalar_tail_calls_n1:baseline": {
        "batch": 8167,
        "estimated_total_ns": {
          "max": 23318377.565,
          "median": 13203131.548,
          "min": 11347629.983000001,
          "p95": 21809875.496
        },
        "estimated_under_20ms_count": 28,
        "observations": 30
      },
      "1:scalar_tail_calls_n1:candidate": {
        "batch": 8167,
        "estimated_total_ns": {
          "max": 17363728.027999997,
          "median": 11623090.6425,
          "min": 10610076.38,
          "p95": 15655559.143000001
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "1:scalar_tail_calls_n32:baseline": {
        "batch": 136,
        "estimated_total_ns": {
          "max": 4595643.048,
          "median": 3225771.012,
          "min": 2688402.032,
          "p95": 4347457.056
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "1:scalar_tail_calls_n32:candidate": {
        "batch": 136,
        "estimated_total_ns": {
          "max": 4599710.944,
          "median": 2943980.0319999997,
          "min": 2519012.9439999997,
          "p95": 3771495.016
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "1:scalar_try_table_normal_n0:baseline": {
        "batch": 24450,
        "estimated_total_ns": {
          "max": 21999596.55,
          "median": 15524686.425,
          "min": 13909042.649999999,
          "p95": 20517926.55
        },
        "estimated_under_20ms_count": 28,
        "observations": 30
      },
      "1:scalar_try_table_normal_n0:candidate": {
        "batch": 24450,
        "estimated_total_ns": {
          "max": 16883165.1,
          "median": 13307487.075,
          "min": 12162408.0,
          "p95": 16183870.65
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "1:scalar_try_table_normal_n10000:baseline": {
        "batch": 3,
        "estimated_total_ns": {
          "max": 45013518.0,
          "median": 23182077.4995,
          "min": 20507855.001000002,
          "p95": 36178667.001
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_try_table_normal_n10000:candidate": {
        "batch": 3,
        "estimated_total_ns": {
          "max": 31545263.001,
          "median": 22435132.9995,
          "min": 20376636.998999998,
          "p95": 28598112.0
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_try_table_normal_n1024:baseline": {
        "batch": 19,
        "estimated_total_ns": {
          "max": 33624732.991000004,
          "median": 15638865.5055,
          "min": 13329383.006000001,
          "p95": 21896404.004
        },
        "estimated_under_20ms_count": 27,
        "observations": 30
      },
      "1:scalar_try_table_normal_n1024:candidate": {
        "batch": 19,
        "estimated_total_ns": {
          "max": 23500919.005999997,
          "median": 15612586.501,
          "min": 13440723.006000001,
          "p95": 22129228.997
        },
        "estimated_under_20ms_count": 26,
        "observations": 30
      },
      "1:scalar_try_table_normal_n17:baseline": {
        "batch": 359,
        "estimated_total_ns": {
          "max": 13179087.091,
          "median": 5696965.973999999,
          "min": 4492085.148,
          "p95": 8873791.079
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "1:scalar_try_table_normal_n17:candidate": {
        "batch": 359,
        "estimated_total_ns": {
          "max": 11647513.034,
          "median": 5097424.127,
          "min": 4554527.095,
          "p95": 6998502.165
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "1:scalar_try_table_normal_n1:baseline": {
        "batch": 15385,
        "estimated_total_ns": {
          "max": 51678830.4,
          "median": 27369284.215,
          "min": 22814170.34,
          "p95": 35418346.975
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_try_table_normal_n1:candidate": {
        "batch": 15385,
        "estimated_total_ns": {
          "max": 39737301.1,
          "median": 25532715.225,
          "min": 22342066.23,
          "p95": 34642696.815
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_try_table_normal_n32:baseline": {
        "batch": 1139,
        "estimated_total_ns": {
          "max": 62496939.112,
          "median": 30263093.32,
          "min": 27894739.867,
          "p95": 45423176.486
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_try_table_normal_n32:candidate": {
        "batch": 1139,
        "estimated_total_ns": {
          "max": 37005030.228,
          "median": 30608278.090499997,
          "min": 27511906.020999998,
          "p95": 36845954.071
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      }
    },
    "workload_count": 85
  },
  "outcome": "rejected",
  "raw_gate_decision": "rejected",
  "reason": "P029 comparison gate rejected the candidate",
  "schema": "wtime-p029-completion-v1",
  "status": "done",
  "strict_build_gate_passed": true,
  "task": "P029",
  "task_index_update_performed": false,
  "completion_development_failures": [
    {
      "kind": "completion checker compiler-text mismatch",
      "exit_code": 1,
      "summary": "Checker expected unquoted match and array64 type wording; retained Rust E0308 prints quoted match and found one with a size of64. Checker corrected, failed source/log preserved, actual retry0. Raw measurements and decisions unchanged.",
      "artifacts": [
        {
          "path": "/workspace/wtime/target/performance/P029/complete_failure_text_failed.py",
          "sha256": "06b5bcbfe236fe49e7e83509fe34d509b13cb45b8e40b29150c4453431ed9dcb"
        },
        {
          "path": "/workspace/wtime/target/performance/P029/finalizer.log",
          "sha256": "9c84e23b4c465d2ee93b26c475ac7363ba6f37e55369047c5ec4ed7f02ef73ca"
        },
        {
          "path": "/workspace/wtime/target/performance/P029/finalizer-retry.log",
          "sha256": "97998c991d07b04b6f6ce3b9232ffe7467796c3779befd2c739af37efe8348ff"
        },
        {
          "path": "/workspace/wtime/target/performance/P029/finalizer-command-record.json",
          "sha256": "615017feb88941c9bbac57e5570e752839ecc8c77ad8548aa34c201a8e995508"
        },
        {
          "path": "/workspace/wtime/target/performance/P029/finalizer-retry-command-record.json",
          "sha256": "40deff30501de1aea1777e5b536f8ae13f863a6fbee11bf8f02a06311d92384e"
        },
        {
          "path": "/workspace/wtime/target/performance/P029/complete_executed.py",
          "sha256": "863e83cc7a41549d235f9f459c508f36a80a4d968229763f50fab62d2681f17b"
        },
        {
          "path": "/workspace/wtime/target/performance/P029/restore_executed.py",
          "sha256": "182c672a0adf6026b17b4b51f55e3081af3dbe9dcf5acfdeefe5226c4de898f7"
        },
        {
          "path": "/workspace/wtime/target/performance/P029/process_closed_executed.py",
          "sha256": "c0e5746e1bcae913c31157ab199755a1541780b3d6f5405df0a6bdb9acb36f4b"
        },
        {
          "path": "/workspace/wtime/target/performance/P029/finalize_command_executed.py",
          "sha256": "c51061b2cee2ecf08c8fae32e78d53a8d9589791827d610439d535c9503f153b"
        }
      ]
    }
  ]
}
```
