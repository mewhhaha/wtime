# P030 — Split checked public entry from validated guest call entry

Status: **done**

Outcome: **rejected**

Kind: `candidate` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P002 — Compare separately built baseline and candidate binaries](../00-measurement/P002-paired-driver.md)

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P008 — Add scaling, branch and call benchmark families](../00-measurement/P008-scalar-workloads.md)

[P016 — Add explicit executor policies and test integration hooks](../01-execution-contracts/P016-execution-policy.md)

[P021 — Compute bounded per-function execution metadata](../01-execution-contracts/P021-function-execution-plans.md)

[P029 — Stop cloning function signatures into every activation](P029-shared-function-metadata.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/execution.rs`
- `crates/wtime-runtime/tests/performance.rs`
- `crates/wtime-runtime/tests/performance/p030.rs`
- `crates/wtime-runtime/tests/fixtures/performance/frames/guest_frame.wat`
- `crates/wtime-runtime/tests/fixtures/performance/frames/guest_frame.wasm`
- `crates/wtime-runtime/tests/fixtures/performance/frames/tail_call.wat`
- `crates/wtime-runtime/tests/fixtures/performance/frames/tail_call.wasm`
- `crates/wtime-runtime/tests/fixtures/performance/frames/recursion.wat`
- `crates/wtime-runtime/tests/fixtures/performance/frames/recursion.wasm`
- `crates/wtime-runtime/tests/fixtures/performance/frames/import_control.wat`
- `crates/wtime-runtime/tests/fixtures/performance/frames/import_control.wasm`
- `crates/wtime-runtime/tests/fixtures/performance/frames/reference_control.wat`
- `crates/wtime-runtime/tests/fixtures/performance/frames/reference_control.wasm`
- `crates/wtime-runtime/tests/fixtures/performance/frames/LICENSE.txt`
- `crates/wtime-runtime/tests/fixtures/performance/frames/README.md`
- `crates/wtime-runtime/tests/fixtures/performance/frames/encoding-report.json`
- `crates/wtime-runtime/tests/fixtures/performance/frames/manifest.json`
- `crates/wtime-runtime/tests/fixtures/performance/frames/reference-control-notes.md`
- `crates/wtime-runtime/tests/fixtures/performance/frames/test-notes.md`
- `crates/wtime-runtime/tests/fixtures/performance/frames/source-hashes.sha256`
- `crates/wtime-runtime/tests/fixtures/performance/frames/failed-source-drafts/guest_frame.wat`
- `crates/wtime-runtime/tests/fixtures/performance/frames/failed-source-drafts/tail_call.wat`
- `crates/wtime-runtime/tests/fixtures/performance/frames/failed-source-drafts/recursion.wat`
- `crates/wtime-runtime/tests/fixtures/performance/frames/failed-source-drafts/source-hashes.sha256`
- `tools/wtime-bench/compare.py`
- `tools/wtime-bench/tests/test_p030_calibration.py`
- `tools/wtime-bench/tests/test_p008_compare.py`
- `tools/wtime-bench/tests/test_p025_compare.py`
- `docs/benchmarks/performance/P030-calibration-correctness.json`
- `docs/benchmarks/performance/P030-candidate.patch`
- `docs/benchmarks/performance/P030-contract.json`
- `docs/benchmarks/performance/P030-build-decision.md`
- `docs/benchmarks/performance/P030-evidence.json`

One integrator owns the dispatch change in execution.rs, public test registration in tests/performance.rs, and the private validated-entry proof in execution.rs; leaf owners submit bounded fixture/test hunks.

## Input/output contract

Public/host/dynamic invocation performs existing checks; an internal validated direct-guest call consumes known stack arguments and initializes locals without rechecking proven scalar parameter types. Reference/dynamic targets retain needed checks.

## Implementation recipe

1. Define distinct private constructors for checked external entry and validated direct guest entry; callers cannot select the latter through the public API.
2. Use FunctionPlan for parameter/local layout and zero initialization; omit only checks justified by the sealed module's validation and immutable bound target.
3. Retain logical call-depth/stack admission, fallible storage allocation and reference/canonical checks where a host/import/dynamic boundary can alter provenance.
4. Measure direct guest calls separately from indirect/host controls and prove optimized frame-entry coverage.
5. Record the changed path with focused assertions/diagnostic counts and exact source/binary identities; compare the pre-edit archive, and revert a rejected structural optimization rather than adding a permanent toggle solely for testing.

## Meaningful cases

- Direct scalar calls initialize all nonparameter locals correctly.
- Wrong host arguments and foreign/reference type contexts are still rejected.
- Low call depth, tail calls and exception cross-call cleanup preserve the same outcomes.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime p030_ -- --nocapture
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml -j 4
python3 tools/wtime-bench/compare.py --baseline target/performance/P030-before --candidate tools/wtime-bench/target/release/wtime-bench --candidate-mode decoded --selection scalar --output target/performance/P030
python3 tools/wtime-bench/analyze.py --input target/performance/P030 --output target/performance/P030/analysis.json
python3 tools/checks/build_budget.py --output target/performance/P030/build.json
python3 tools/wtime-bench/gates.py --task P030 --comparison target/performance/P030 --build target/performance/P030/build.json --output target/performance/P030/decision.json
```

## Measurement and decision

Selection: `scalar`. Explicit execution setting: `decoded`. Decision rule: `measured_promotion`.

Use the paired protocol and family/control/cold/build rules in [MEASUREMENT.md](../MEASUREMENT.md). Predeclare the targeted family and held-out controls before changing the implementation. Keep instrumentation off in production timings. Adopt only with supported gains in both runs; otherwise record rejected/inconclusive and remove default-path cost. A necessary immutable-interface change may be contract_only with its separate justification.

## Before editing

Run these commands before changing the candidate sources. The archive is immutable and captures the immediate predecessor. Retain P001's original archive for milestone totals. For native work, use the named portable control and verify its actual policy/quantum; if this archive already exists with different sources, choose a new recorded run-specific directory and update the comparison path rather than overwriting it.

```sh
python3 tools/wtime-bench/baseline.py --output target/performance/P030-before
```

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. Executed evidence is recorded below.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/docs/benchmarks/performance/P030-build-decision.md": {
      "bytes": 2979,
      "sha256": "460721654fc26f1762dd9887c47ecec74878f55374d9f7fe3cf4a88b49c77315"
    },
    "/workspace/wtime/docs/benchmarks/performance/P030-candidate.patch": {
      "bytes": 8154,
      "sha256": "bd52b49c27c507ed384c844caef5da18989d835d86038c0d8ecbb9909d852133"
    },
    "/workspace/wtime/docs/benchmarks/performance/P030-contract.json": {
      "bytes": 2757,
      "sha256": "c70803c8d8eb6188b4b2efa5978860503aed2a7fb1a0ddad90596e805dd3b1e4"
    },
    "/workspace/wtime/docs/benchmarks/performance/P030-evidence.json": {
      "bytes": 1352478,
      "sha256": "0de3e2ee6464989de185c5d5207cfe68d13615958dff41242f729beea7caa5f2"
    },
    "/workspace/wtime/target/performance/P029/restored-default-tests.log": {
      "bytes": 14586,
      "sha256": "f232d633bd12e74532155730e22266be3e490e64057d456492fb3e94452e8d50"
    },
    "/workspace/wtime/target/performance/P029/restored-diagnostic-tests.log": {
      "bytes": 4729,
      "sha256": "6d0b7b021de05e834403865ff89d5d6fc0ef5c428e6b747f5b739de68c811cb0"
    },
    "/workspace/wtime/target/performance/P030-before-build.json": {
      "bytes": 106050,
      "sha256": "2274e89f6ddefff9672d648dc569c560d38ac1f3367383543fcaae278e865aa2"
    },
    "/workspace/wtime/target/performance/P030-before/manifest.json": {
      "bytes": 37590,
      "sha256": "669dc4f122f81b645ee7ac7e5e2b19a43185a4cd2316b4436ecd3e4ae7498cc9"
    },
    "/workspace/wtime/target/performance/P030-build.json": {
      "bytes": 106053,
      "sha256": "56fb86a23bae2a6635eee4faec0ab162cec336959bd7753022e0d0aad9a4f5a3"
    },
    "/workspace/wtime/target/performance/P030-candidate-retained/source-manifest.json": {
      "bytes": 15135,
      "sha256": "7d69e579bb4b2319933460b88a144f342cc514e8f66f9d080e2395d617c5813e"
    },
    "/workspace/wtime/target/performance/P030-candidate-tests.log": {
      "bytes": 2260,
      "sha256": "2c094b5aa65908dedf721ba9cedfe6ab938a4da97192e5c99f39b6f27ee08a74"
    },
    "/workspace/wtime/target/performance/P030-candidate_default.log": {
      "bytes": 15311,
      "sha256": "4f70f638d09ef2ef660f9e05f4592c49d6af70166a90e574ae0ec510b2ebc9fd"
    },
    "/workspace/wtime/target/performance/P030-candidate_diagnostic_performance.log": {
      "bytes": 5090,
      "sha256": "e4c0412f004d869306f2c621d78ff29e77940f526e10b6929fb9caa51aa248d6"
    },
    "/workspace/wtime/target/performance/P030-candidate_focused_diagnostic.log": {
      "bytes": 2330,
      "sha256": "76046075f90045af5cc79722218cc485c61d2d7a304e40182b59ba0e4cb17b36"
    },
    "/workspace/wtime/target/performance/P030-incremental-build-decision.json": {
      "bytes": 1077,
      "sha256": "dd9d429af6eaae60c0a8bf3decb213cfc2713588fc67a580bf12cba0f115d682"
    },
    "/workspace/wtime/target/performance/P030-preflight/focused_default.log": {
      "bytes": 1901,
      "sha256": "0139fd973e70a5cec2c4e3f9a7e4a88e288d6f1b83fd154d5328d43cb5e1f255"
    },
    "/workspace/wtime/target/performance/P030-preflight/focused_diagnostic.log": {
      "bytes": 1970,
      "sha256": "968d2077ae730f064875f45d5b64c0f173b710b50454d79ad854d94f26311ffe"
    },
    "/workspace/wtime/target/performance/P030/analysis.json": {
      "bytes": 1264234,
      "sha256": "d911b8fef3f17801b170cff060b408903d7bb3ce0cfcfa4a9a6986106f8c35b1"
    },
    "/workspace/wtime/target/performance/P030/analysis.log": {
      "bytes": 0,
      "sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    },
    "/workspace/wtime/target/performance/P030/checkpoint.jsonl": {
      "bytes": 36222833,
      "sha256": "a29d60b6cc8cb002a39728d370e375d9a4095b97d301c28d47a3d6062194a612"
    },
    "/workspace/wtime/target/performance/P030/commands.json": {
      "bytes": 5088,
      "sha256": "ca1511ee1be70f3eb7a4718056bb5963516e56813d3c41ee9046d06e57c85e31"
    },
    "/workspace/wtime/target/performance/P030/complete_executed.py": {
      "bytes": 51189,
      "sha256": "90936891ad770981af7a922625a1ca1d00990c14213ab39145249af014f50edd"
    },
    "/workspace/wtime/target/performance/P030/decision.json": {
      "bytes": 1687363,
      "sha256": "214257ebbf3eaa5a7034c02e86485e52b5e765c8c26ffa054574d07207e637ec"
    },
    "/workspace/wtime/target/performance/P030/extraction.json": {
      "bytes": 466014,
      "sha256": "b3e9c4db9268276df98180890c9b4bdd8a0b04d207504f6cb631c0011fb930d8"
    },
    "/workspace/wtime/target/performance/P030/extraction.log": {
      "bytes": 294871,
      "sha256": "3025fb4ec69ae561738893980269acc09d3187819798109d272e229aa2055d1a"
    },
    "/workspace/wtime/target/performance/P030/final-decision.json": {
      "bytes": 9726,
      "sha256": "458945dfd0460dd50f9b32ce3af36a843df7dec9ea3d95faf3bf66b0649b71d3"
    },
    "/workspace/wtime/target/performance/P030/finalization-attempts.json": {
      "bytes": 9309,
      "sha256": "b61437d95bc35c0ac7c219010d3b29f4b012ace76d0c7150a26f3fa9cbbc322a"
    },
    "/workspace/wtime/target/performance/P030/finalize_command_failed_arguments.py": {
      "bytes": 2553,
      "sha256": "e456e290748706dd411074ed4829bbe02d42df72d160ecb946e6af620a98f180"
    },
    "/workspace/wtime/target/performance/P030/finalize_command_retry_executed.py": {
      "bytes": 2599,
      "sha256": "f4e3ffd7ee8d2aac830638743d10737e79466d31a2dd8c20e307c9aaf2385d76"
    },
    "/workspace/wtime/target/performance/P030/finalizer-command-record.json": {
      "bytes": 3398,
      "sha256": "8264e198f340be42b29e94e594ef005d1576c13b043b49d848f61c8837e98e45"
    },
    "/workspace/wtime/target/performance/P030/finalizer-retry-command-record.json": {
      "bytes": 3588,
      "sha256": "327f3754a1516effad0e4b7ba0c6adde7d41536d148db8a10290068265f74bbe"
    },
    "/workspace/wtime/target/performance/P030/finalizer-retry.log": {
      "bytes": 173,
      "sha256": "ce1f6bade8c5c49ff218cd50af44417fc4982d378314774c53206a264815cd0e"
    },
    "/workspace/wtime/target/performance/P030/finalizer.log": {
      "bytes": 92,
      "sha256": "8639e9712b365f6942abb0fde2721aab023f1dbec9e68eb79ae8afe014a13682"
    },
    "/workspace/wtime/target/performance/P030/process_closed_executed.py": {
      "bytes": 3540,
      "sha256": "162e21cff2f446f83fe2fcdbfc98f5bb735ef5ab97ac3afac78fc283e01a5630"
    },
    "/workspace/wtime/target/performance/P030/raw-exits.json": {
      "bytes": 39,
      "sha256": "8cb285167680019e8683e276900144b811a534abc0797ee27c6fde696476ba90"
    },
    "/workspace/wtime/target/performance/P030/raw-gate.log": {
      "bytes": 1687363,
      "sha256": "214257ebbf3eaa5a7034c02e86485e52b5e765c8c26ffa054574d07207e637ec"
    },
    "/workspace/wtime/target/performance/P030/report.json": {
      "bytes": 60159472,
      "sha256": "4ef14c1f0b18a8ea6a5c2ebcf0adc75994e225f0dd776aae2ff31b1b128c7976"
    },
    "/workspace/wtime/target/performance/P030/restore.log": {
      "bytes": 259,
      "sha256": "7a3965d360db48f8d29951a6dc61a3823154a004896ca0229a7aa0426783fc66"
    },
    "/workspace/wtime/target/performance/P030/restore_executed.py": {
      "bytes": 10464,
      "sha256": "52fe51a40ef556610cca2b88a4021df8526589e68e630ef085f11d1d50f38d16"
    },
    "/workspace/wtime/target/performance/P030/restored-build.json": {
      "bytes": 106053,
      "sha256": "ccb8c50287bb99919783134820d1f3c18da81db1c168e1d8dd7f0f6315300c8c"
    },
    "/workspace/wtime/target/performance/P030/restored-default-tests.log": {
      "bytes": 14952,
      "sha256": "143578b60a3c2727cb4e4ac01f1197ebc252bfb70b40c3a73823cb041d04d063"
    },
    "/workspace/wtime/target/performance/P030/restored-diagnostic-tests.log": {
      "bytes": 5164,
      "sha256": "7cc30e87819b7a53be6ef961bc9b8e3fead1d2636bf80c19c355ba2433db0087"
    },
    "/workspace/wtime/target/performance/P030/restored-focused-default-tests.log": {
      "bytes": 1827,
      "sha256": "1b78e5ff7cac1cc892104d2e9ff27007a1bf30cf3d3a9202cd18d7fbda9a4579"
    },
    "/workspace/wtime/target/performance/P030/restored-focused-diagnostic-tests.log": {
      "bytes": 1970,
      "sha256": "863183fdd27ac130816f0fee5a3aa95662979a86e05a8f9d7280e397c3daf467"
    },
    "/workspace/wtime/target/performance/P030/rollback-identities.json": {
      "bytes": 844,
      "sha256": "fd42b6d79403c6800dd9296a711da199bf5fc8445c7637dc2d70cb08fd69914b"
    },
    "/workspace/wtime/target/performance/P030/task-precompletion/P030-validated-guest-frame-entry.md": {
      "bytes": 7610,
      "sha256": "b06edfcb5d2d2479c7b52ad35d0289696d79c01279b6bed72075e11925a2fb23"
    },
    "/workspace/wtime/target/performance/P030/task-precompletion/TASKS.json": {
      "bytes": 1219137,
      "sha256": "b1d139bd285b03f2a54b2e5d9dcfe66e38213c67cbef29ed4498e0a2af6ab2e8"
    }
  },
  "artifacts": [
    "/workspace/wtime/docs/benchmarks/performance/P030-build-decision.md",
    "/workspace/wtime/docs/benchmarks/performance/P030-candidate.patch",
    "/workspace/wtime/docs/benchmarks/performance/P030-contract.json",
    "/workspace/wtime/docs/benchmarks/performance/P030-evidence.json",
    "/workspace/wtime/target/performance/P029/restored-default-tests.log",
    "/workspace/wtime/target/performance/P029/restored-diagnostic-tests.log",
    "/workspace/wtime/target/performance/P030-before-build.json",
    "/workspace/wtime/target/performance/P030-before/manifest.json",
    "/workspace/wtime/target/performance/P030-build.json",
    "/workspace/wtime/target/performance/P030-candidate-retained/source-manifest.json",
    "/workspace/wtime/target/performance/P030-candidate-tests.log",
    "/workspace/wtime/target/performance/P030-candidate_default.log",
    "/workspace/wtime/target/performance/P030-candidate_diagnostic_performance.log",
    "/workspace/wtime/target/performance/P030-candidate_focused_diagnostic.log",
    "/workspace/wtime/target/performance/P030-incremental-build-decision.json",
    "/workspace/wtime/target/performance/P030-preflight/focused_default.log",
    "/workspace/wtime/target/performance/P030-preflight/focused_diagnostic.log",
    "/workspace/wtime/target/performance/P030/analysis.json",
    "/workspace/wtime/target/performance/P030/analysis.log",
    "/workspace/wtime/target/performance/P030/checkpoint.jsonl",
    "/workspace/wtime/target/performance/P030/commands.json",
    "/workspace/wtime/target/performance/P030/complete_executed.py",
    "/workspace/wtime/target/performance/P030/decision.json",
    "/workspace/wtime/target/performance/P030/extraction.json",
    "/workspace/wtime/target/performance/P030/extraction.log",
    "/workspace/wtime/target/performance/P030/final-decision.json",
    "/workspace/wtime/target/performance/P030/finalization-attempts.json",
    "/workspace/wtime/target/performance/P030/finalize_command_failed_arguments.py",
    "/workspace/wtime/target/performance/P030/finalize_command_retry_executed.py",
    "/workspace/wtime/target/performance/P030/finalizer-command-record.json",
    "/workspace/wtime/target/performance/P030/finalizer-retry-command-record.json",
    "/workspace/wtime/target/performance/P030/finalizer-retry.log",
    "/workspace/wtime/target/performance/P030/finalizer.log",
    "/workspace/wtime/target/performance/P030/process_closed_executed.py",
    "/workspace/wtime/target/performance/P030/raw-exits.json",
    "/workspace/wtime/target/performance/P030/raw-gate.log",
    "/workspace/wtime/target/performance/P030/report.json",
    "/workspace/wtime/target/performance/P030/restore.log",
    "/workspace/wtime/target/performance/P030/restore_executed.py",
    "/workspace/wtime/target/performance/P030/restored-build.json",
    "/workspace/wtime/target/performance/P030/restored-default-tests.log",
    "/workspace/wtime/target/performance/P030/restored-diagnostic-tests.log",
    "/workspace/wtime/target/performance/P030/restored-focused-default-tests.log",
    "/workspace/wtime/target/performance/P030/restored-focused-diagnostic-tests.log",
    "/workspace/wtime/target/performance/P030/rollback-identities.json",
    "/workspace/wtime/target/performance/P030/task-precompletion/P030-validated-guest-frame-entry.md",
    "/workspace/wtime/target/performance/P030/task-precompletion/TASKS.json"
  ],
  "candidate_promoted": false,
  "commands": [
    {
      "command": ". /workspace/.setup/rust-env.sh\npython3 tools/wtime-bench/compare.py --baseline target/performance/P030-before --candidate tools/wtime-bench/target/release/wtime-bench --candidate-mode decoded --selection scalar --pairs 30 --build-growth-explanation-file docs/benchmarks/performance/P030-build-decision.md --output target/performance/P030 > target/performance/P030-run.log 2>&1",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P030-run.log",
      "log_sha256": "a6651069c51ef66aeac29ba445569051f42bc608a7da852b9c2a5b9abea3b7c5",
      "name": "comparison"
    },
    {
      "command": ". /workspace/.setup/rust-env.sh\npython3 /tmp/extract_p030.py --report /workspace/wtime/target/performance/P030/report.json --checkpoint /workspace/wtime/target/performance/P030/checkpoint.jsonl --archive /workspace/wtime/target/performance/P030-before --candidate-dir /workspace/wtime/target/performance/P030-candidate-retained --output /workspace/wtime/target/performance/P030/extraction.json",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P030/extraction.log",
      "log_sha256": "3025fb4ec69ae561738893980269acc09d3187819798109d272e229aa2055d1a",
      "name": "extraction",
      "stdout_path": "/workspace/wtime/target/performance/P030/extraction.log",
      "stdout_sha256": "3025fb4ec69ae561738893980269acc09d3187819798109d272e229aa2055d1a"
    },
    {
      "command": ". /workspace/.setup/rust-env.sh\npython3 tools/wtime-bench/analyze.py --input target/performance/P030 --output target/performance/P030/analysis.json",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P030/analysis.log",
      "log_sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
      "name": "analysis",
      "stdout_path": "/workspace/wtime/target/performance/P030/analysis.log",
      "stdout_sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    },
    {
      "command": ". /workspace/.setup/rust-env.sh\npython3 tools/wtime-bench/gates.py --task P030 --comparison target/performance/P030 --build target/performance/P030/comparison-build.json --contract docs/benchmarks/performance/P030-contract.json --output target/performance/P030/decision.json",
      "exit_code": 1,
      "log_path": "/workspace/wtime/target/performance/P030/raw-gate.log",
      "log_sha256": "214257ebbf3eaa5a7034c02e86485e52b5e765c8c26ffa054574d07207e637ec",
      "name": "raw_gate",
      "stdout_path": "/workspace/wtime/target/performance/P030/raw-gate.log",
      "stdout_sha256": "214257ebbf3eaa5a7034c02e86485e52b5e765c8c26ffa054574d07207e637ec"
    },
    {
      "command": ". /workspace/.setup/rust-env.sh\ncargo test --locked -p wtime-runtime",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P030/restored-default-tests.log",
      "log_sha256": "143578b60a3c2727cb4e4ac01f1197ebc252bfb70b40c3a73823cb041d04d063",
      "name": "restored_default_tests"
    },
    {
      "command": ". /workspace/.setup/rust-env.sh\ncargo test --locked -p wtime-runtime --features execution-reference,performance-counters --test performance",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P030/restored-diagnostic-tests.log",
      "log_sha256": "7cc30e87819b7a53be6ef961bc9b8e3fead1d2636bf80c19c355ba2433db0087",
      "name": "restored_diagnostic_tests"
    },
    {
      "command": ". /workspace/.setup/rust-env.sh\ncargo test --locked -p wtime-runtime p030_ -- --nocapture",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P030/restored-focused-default-tests.log",
      "log_sha256": "1b78e5ff7cac1cc892104d2e9ff27007a1bf30cf3d3a9202cd18d7fbda9a4579",
      "name": "restored_focused_default_tests"
    },
    {
      "command": ". /workspace/.setup/rust-env.sh\ncargo test --locked -p wtime-runtime --features execution-reference,performance-counters p030_ -- --nocapture",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P030/restored-focused-diagnostic-tests.log",
      "log_sha256": "863183fdd27ac130816f0fee5a3aa95662979a86e05a8f9d7280e397c3daf467",
      "name": "restored_focused_diagnostic_tests"
    },
    {
      "command": ". /workspace/.setup/rust-env.sh\ncargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml -j 4",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P030/restored-benchmark-build.log",
      "log_sha256": "a6b380b710c464006312ed02d079054679a9dcb3241492860dd3eaffc8398621",
      "name": "rollback_benchmark_build"
    },
    {
      "command": ". /workspace/.setup/rust-env.sh\npython3 tools/checks/build_budget.py --output target/performance/P030/restored-build.json",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P030/restored-build.log",
      "log_sha256": "8573a48a1e7e94ff3f22afaad27a12381d94a3e91ceae270e8edbd89b082816b",
      "name": "rollback_build"
    }
  ],
  "completion_verifier_attempts": {
    "artifacts": [
      {
        "path": "/workspace/wtime/target/performance/P030/finalizer.log",
        "sha256": "8639e9712b365f6942abb0fde2721aab023f1dbec9e68eb79ae8afe014a13682"
      },
      {
        "path": "/workspace/wtime/target/performance/P030/finalizer-command-record.json",
        "sha256": "8264e198f340be42b29e94e594ef005d1576c13b043b49d848f61c8837e98e45"
      },
      {
        "path": "/workspace/wtime/target/performance/P030/finalizer-retry.log",
        "sha256": "ce1f6bade8c5c49ff218cd50af44417fc4982d378314774c53206a264815cd0e"
      },
      {
        "path": "/workspace/wtime/target/performance/P030/finalizer-retry-command-record.json",
        "sha256": "327f3754a1516effad0e4b7ba0c6adde7d41536d148db8a10290068265f74bbe"
      },
      {
        "path": "/workspace/wtime/target/performance/P030/finalize_command_failed_arguments.py",
        "sha256": "e456e290748706dd411074ed4829bbe02d42df72d160ecb946e6af620a98f180"
      },
      {
        "path": "/workspace/wtime/target/performance/P030/finalize_command_retry_executed.py",
        "sha256": "f4e3ffd7ee8d2aac830638743d10737e79466d31a2dd8c20e307c9aaf2385d76"
      },
      {
        "path": "/workspace/wtime/target/performance/P030/complete_executed.py",
        "sha256": "90936891ad770981af7a922625a1ca1d00990c14213ab39145249af014f50edd"
      },
      {
        "path": "/workspace/wtime/target/performance/P030/restore_executed.py",
        "sha256": "52fe51a40ef556610cca2b88a4021df8526589e68e630ef085f11d1d50f38d16"
      },
      {
        "path": "/workspace/wtime/target/performance/P030/process_closed_executed.py",
        "sha256": "162e21cff2f446f83fe2fcdbfc98f5bb735ef5ab97ac3afac78fc283e01a5630"
      }
    ],
    "attempts": [
      {
        "argv": [
          "python3",
          "/tmp/P030-complete.py",
          "--comparison-dir",
          "/workspace/wtime/target/performance/P030",
          "--raw-compare-exit",
          "0",
          "--raw-gate-exit",
          "1",
          "--extraction",
          "/workspace/wtime/target/performance/P030/extraction.json",
          "--analysis",
          "/workspace/wtime/target/performance/P030/analysis.json",
          "--raw-gate",
          "/workspace/wtime/target/performance/P030/decision.json",
          "--command-record",
          "/workspace/wtime/target/performance/P030/commands.json",
          "--rollback-identities",
          "/workspace/wtime/target/performance/P030/rollback-identities.json",
          "--rollback-build",
          "/workspace/wtime/target/performance/P030/restored-build.json",
          "--restored-default-tests",
          "/workspace/wtime/target/performance/P030/restored-default-tests.log",
          "--restored-diagnostic-tests",
          "/workspace/wtime/target/performance/P030/restored-diagnostic-tests.log",
          "--restored-focused-default-tests",
          "/workspace/wtime/target/performance/P030/restored-focused-default-tests.log",
          "--restored-focused-diagnostic-tests",
          "/workspace/wtime/target/performance/P030/restored-focused-diagnostic-tests.log",
          "--candidate-patch",
          "/workspace/wtime/docs/benchmarks/performance/P030-candidate.patch",
          "--private-path-test-name",
          "p030_actual_scalar_vector_direct_calls_construct_validated_frames",
          "--private-path-test-name",
          "p030_actual_return_call_keeps_bounded_depth",
          "--private-path-test-name",
          "p030_reference_parameter_direct_call_keeps_checked_fallback",
          "--private-path-test-name",
          "p030_direct_import_keeps_host_signature_boundary"
        ],
        "command": ". /workspace/.setup/rust-env.sh\npython3 /tmp/P030-complete.py --comparison-dir /workspace/wtime/target/performance/P030 --raw-compare-exit 0 --raw-gate-exit 1 --extraction /workspace/wtime/target/performance/P030/extraction.json --analysis /workspace/wtime/target/performance/P030/analysis.json --raw-gate /workspace/wtime/target/performance/P030/decision.json --command-record /workspace/wtime/target/performance/P030/commands.json --rollback-identities /workspace/wtime/target/performance/P030/rollback-identities.json --rollback-build /workspace/wtime/target/performance/P030/restored-build.json --restored-default-tests /workspace/wtime/target/performance/P030/restored-default-tests.log --restored-diagnostic-tests /workspace/wtime/target/performance/P030/restored-diagnostic-tests.log --restored-focused-default-tests /workspace/wtime/target/performance/P030/restored-focused-default-tests.log --restored-focused-diagnostic-tests /workspace/wtime/target/performance/P030/restored-focused-diagnostic-tests.log --candidate-patch /workspace/wtime/docs/benchmarks/performance/P030-candidate.patch --private-path-test-name p030_actual_scalar_vector_direct_calls_construct_validated_frames --private-path-test-name p030_actual_return_call_keeps_bounded_depth --private-path-test-name p030_reference_parameter_direct_call_keeps_checked_fallback --private-path-test-name p030_direct_import_keeps_host_signature_boundary",
        "exit_code": 1,
        "helper_sha256_at_execution": "90936891ad770981af7a922625a1ca1d00990c14213ab39145249af014f50edd",
        "log_path": "/workspace/wtime/target/performance/P030/finalizer.log",
        "log_sha256": "8639e9712b365f6942abb0fde2721aab023f1dbec9e68eb79ae8afe014a13682",
        "task": "P030"
      },
      {
        "argv": [
          "python3",
          "/tmp/P030-complete.py",
          "--comparison-dir",
          "/workspace/wtime/target/performance/P030",
          "--raw-compare-exit",
          "0",
          "--raw-gate-exit",
          "1",
          "--extraction",
          "/workspace/wtime/target/performance/P030/extraction.json",
          "--analysis",
          "/workspace/wtime/target/performance/P030/analysis.json",
          "--raw-gate",
          "/workspace/wtime/target/performance/P030/decision.json",
          "--command-record",
          "/workspace/wtime/target/performance/P030/commands.json",
          "--rollback-identities",
          "/workspace/wtime/target/performance/P030/rollback-identities.json",
          "--rollback-build",
          "/workspace/wtime/target/performance/P030/restored-build.json",
          "--restored-default-tests",
          "/workspace/wtime/target/performance/P030/restored-default-tests.log",
          "--restored-diagnostic-tests",
          "/workspace/wtime/target/performance/P030/restored-diagnostic-tests.log",
          "--restored-focused-default-tests",
          "/workspace/wtime/target/performance/P030/restored-focused-default-tests.log",
          "--restored-focused-diagnostic-tests",
          "/workspace/wtime/target/performance/P030/restored-focused-diagnostic-tests.log",
          "--candidate-patch",
          "/workspace/wtime/docs/benchmarks/performance/P030-candidate.patch",
          "--private-path-test-name",
          "execution::p030_tests::p030_actual_scalar_vector_direct_calls_construct_validated_frames",
          "--private-path-test-name",
          "execution::p030_tests::p030_actual_return_call_keeps_bounded_depth",
          "--private-path-test-name",
          "execution::p030_tests::p030_reference_parameter_direct_call_keeps_checked_fallback",
          "--private-path-test-name",
          "execution::p030_tests::p030_direct_import_keeps_host_signature_boundary"
        ],
        "command": ". /workspace/.setup/rust-env.sh\npython3 /tmp/P030-complete.py --comparison-dir /workspace/wtime/target/performance/P030 --raw-compare-exit 0 --raw-gate-exit 1 --extraction /workspace/wtime/target/performance/P030/extraction.json --analysis /workspace/wtime/target/performance/P030/analysis.json --raw-gate /workspace/wtime/target/performance/P030/decision.json --command-record /workspace/wtime/target/performance/P030/commands.json --rollback-identities /workspace/wtime/target/performance/P030/rollback-identities.json --rollback-build /workspace/wtime/target/performance/P030/restored-build.json --restored-default-tests /workspace/wtime/target/performance/P030/restored-default-tests.log --restored-diagnostic-tests /workspace/wtime/target/performance/P030/restored-diagnostic-tests.log --restored-focused-default-tests /workspace/wtime/target/performance/P030/restored-focused-default-tests.log --restored-focused-diagnostic-tests /workspace/wtime/target/performance/P030/restored-focused-diagnostic-tests.log --candidate-patch /workspace/wtime/docs/benchmarks/performance/P030-candidate.patch --private-path-test-name execution::p030_tests::p030_actual_scalar_vector_direct_calls_construct_validated_frames --private-path-test-name execution::p030_tests::p030_actual_return_call_keeps_bounded_depth --private-path-test-name execution::p030_tests::p030_reference_parameter_direct_call_keeps_checked_fallback --private-path-test-name execution::p030_tests::p030_direct_import_keeps_host_signature_boundary",
        "exit_code": 0,
        "helper_sha256_at_execution": "90936891ad770981af7a922625a1ca1d00990c14213ab39145249af014f50edd",
        "log_path": "/workspace/wtime/target/performance/P030/finalizer-retry.log",
        "log_sha256": "ce1f6bade8c5c49ff218cd50af44417fc4982d378314774c53206a264815cd0e",
        "task": "P030"
      }
    ],
    "correction": "Verifier invocation used leaf test names initially; retry passes the full execution::p030_tests:: names already present in the actual candidate log. No tests, builds, timing samples, ratios or thresholds were repeated or changed.",
    "task": "P030"
  },
  "counts": {
    "analysis_group_count": 850,
    "correctness_probe_kinds": {},
    "correctness_probe_records": 0,
    "expected_pair_ids_sha256": "e2d25b4fb5e5913269e686e33e2b8957e5998391b8ae2e2103a30a433d37b7ad",
    "fixture_files_verified": 85,
    "group_count_total": 850,
    "legacy_scalar_uncalibrated_workloads": 3,
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
    "typed_scalar_calibration_workloads": 82,
    "warm_calibration_attempt_checks": 210,
    "warm_calibration_policy_limit": "Typed scalar workloads use bounded shared-batch confirmation with both sides at or above 20 ms during calibration. Later timed observations can fall below 20 ms. The three legacy scalar controls retain the uncalibrated common MIN one-shot estimate and have no 20 ms guarantee.",
    "warm_calibration_records": 164,
    "warm_calibration_records_by_case_run": {
      "0:scalar_accumulator_n0": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 9644,
            "sides": [
              {
                "calibrated_total_ns": 6516981.22,
                "ns_per_operation": 675.755,
                "observed_batch": 9644,
                "requested_batch": 9644,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 10249421.388,
                "ns_per_operation": 1062.777,
                "observed_batch": 9644,
                "requested_batch": 9644,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 32557,
            "sides": [
              {
                "calibrated_total_ns": 24406355.05,
                "ns_per_operation": 749.65,
                "observed_batch": 32557,
                "requested_batch": 32557,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 22279764.367000002,
                "ns_per_operation": 684.331,
                "observed_batch": 32557,
                "requested_batch": 32557,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 24406355.05,
            "ns_per_operation": 749.65,
            "observed_batch": 32557,
            "requested_batch": 32557,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22279764.367000002,
            "ns_per_operation": 684.331,
            "observed_batch": 32557,
            "requested_batch": 32557,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 7402560.808,
            "ns_per_operation": 767.582,
            "observed_batch": 9644,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 4562013.82,
            "ns_per_operation": 686.74,
            "observed_batch": 6643,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 32557,
        "source_sha256": "61aaec7a86d481378f31f3e27b0c1413684065841fa5909297181fcad9bef10d",
        "wasm_sha256": "ef61bf7afc44af7b71ef78eadcba48c0d85949d7d9096df8fbbbab53c05036b1"
      },
      "0:scalar_accumulator_n1": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 8010,
            "sides": [
              {
                "calibrated_total_ns": 10793939.58,
                "ns_per_operation": 1347.558,
                "observed_batch": 8010,
                "requested_batch": 8010,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 13698862.200000001,
                "ns_per_operation": 1710.22,
                "observed_batch": 8010,
                "requested_batch": 8010,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 16326,
            "sides": [
              {
                "calibrated_total_ns": 30060525.042,
                "ns_per_operation": 1841.267,
                "observed_batch": 16326,
                "requested_batch": 16326,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 21636194.76,
                "ns_per_operation": 1325.26,
                "observed_batch": 16326,
                "requested_batch": 16326,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 30060525.042,
            "ns_per_operation": 1841.267,
            "observed_batch": 16326,
            "requested_batch": 16326,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 21636194.76,
            "ns_per_operation": 1325.26,
            "observed_batch": 16326,
            "requested_batch": 16326,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 14461137.006000001,
            "ns_per_operation": 1996.843,
            "observed_batch": 7242,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 16319269.620000001,
            "ns_per_operation": 2037.362,
            "observed_batch": 8010,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 16326,
        "source_sha256": "61aaec7a86d481378f31f3e27b0c1413684065841fa5909297181fcad9bef10d",
        "wasm_sha256": "ef61bf7afc44af7b71ef78eadcba48c0d85949d7d9096df8fbbbab53c05036b1"
      },
      "0:scalar_accumulator_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 4,
            "sides": [
              {
                "calibrated_total_ns": 21801506.0,
                "ns_per_operation": 5450376.5,
                "observed_batch": 4,
                "requested_batch": 4,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 21919933.0,
                "ns_per_operation": 5479983.25,
                "observed_batch": 4,
                "requested_batch": 4,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 21801506.0,
            "ns_per_operation": 5450376.5,
            "observed_batch": 4,
            "requested_batch": 4,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 21919933.0,
            "ns_per_operation": 5479983.25,
            "observed_batch": 4,
            "requested_batch": 4,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 24683944.0,
            "ns_per_operation": 6170986.0,
            "observed_batch": 4,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 18238455.0,
            "ns_per_operation": 6079485.0,
            "observed_batch": 3,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 4,
        "source_sha256": "61aaec7a86d481378f31f3e27b0c1413684065841fa5909297181fcad9bef10d",
        "wasm_sha256": "ef61bf7afc44af7b71ef78eadcba48c0d85949d7d9096df8fbbbab53c05036b1"
      },
      "0:scalar_accumulator_n1024": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 42,
            "sides": [
              {
                "calibrated_total_ns": 26575427.004,
                "ns_per_operation": 632748.262,
                "observed_batch": 42,
                "requested_batch": 42,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 24823190.994,
                "ns_per_operation": 591028.357,
                "observed_batch": 42,
                "requested_batch": 42,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 26575427.004,
            "ns_per_operation": 632748.262,
            "observed_batch": 42,
            "requested_batch": 42,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 24823190.994,
            "ns_per_operation": 591028.357,
            "observed_batch": 42,
            "requested_batch": 42,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 25076211.006,
            "ns_per_operation": 597052.643,
            "observed_batch": 42,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 16926400.014,
            "ns_per_operation": 583668.966,
            "observed_batch": 29,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 42,
        "source_sha256": "61aaec7a86d481378f31f3e27b0c1413684065841fa5909297181fcad9bef10d",
        "wasm_sha256": "ef61bf7afc44af7b71ef78eadcba48c0d85949d7d9096df8fbbbab53c05036b1"
      },
      "0:scalar_accumulator_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 2248,
            "sides": [
              {
                "calibrated_total_ns": 26331693.976,
                "ns_per_operation": 11713.387,
                "observed_batch": 2248,
                "requested_batch": 2248,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 32096573.08,
                "ns_per_operation": 14277.835,
                "observed_batch": 2248,
                "requested_batch": 2248,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 26331693.976,
            "ns_per_operation": 11713.387,
            "observed_batch": 2248,
            "requested_batch": 2248,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 32096573.08,
            "ns_per_operation": 14277.835,
            "observed_batch": 2248,
            "requested_batch": 2248,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 25380887.04,
            "ns_per_operation": 11432.832,
            "observed_batch": 2220,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 26162970.336,
            "ns_per_operation": 11638.332,
            "observed_batch": 2248,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 2248,
        "source_sha256": "61aaec7a86d481378f31f3e27b0c1413684065841fa5909297181fcad9bef10d",
        "wasm_sha256": "ef61bf7afc44af7b71ef78eadcba48c0d85949d7d9096df8fbbbab53c05036b1"
      },
      "0:scalar_accumulator_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1347,
            "sides": [
              {
                "calibrated_total_ns": 27604573.430999998,
                "ns_per_operation": 20493.373,
                "observed_batch": 1347,
                "requested_batch": 1347,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 26430895.961999997,
                "ns_per_operation": 19622.046,
                "observed_batch": 1347,
                "requested_batch": 1347,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 27604573.430999998,
            "ns_per_operation": 20493.373,
            "observed_batch": 1347,
            "requested_batch": 1347,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 26430895.961999997,
            "ns_per_operation": 19622.046,
            "observed_batch": 1347,
            "requested_batch": 1347,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 35898911.217,
            "ns_per_operation": 31027.581,
            "observed_batch": 1157,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27535240.647000004,
            "ns_per_operation": 20441.901,
            "observed_batch": 1347,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1347,
        "source_sha256": "61aaec7a86d481378f31f3e27b0c1413684065841fa5909297181fcad9bef10d",
        "wasm_sha256": "ef61bf7afc44af7b71ef78eadcba48c0d85949d7d9096df8fbbbab53c05036b1"
      },
      "0:scalar_br_table_n0": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 31202,
            "sides": [
              {
                "calibrated_total_ns": 31767130.623999998,
                "ns_per_operation": 1018.112,
                "observed_batch": 31202,
                "requested_batch": 31202,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 31347838.148,
                "ns_per_operation": 1004.674,
                "observed_batch": 31202,
                "requested_batch": 31202,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 31767130.623999998,
            "ns_per_operation": 1018.112,
            "observed_batch": 31202,
            "requested_batch": 31202,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 31347838.148,
            "ns_per_operation": 1004.674,
            "observed_batch": 31202,
            "requested_batch": 31202,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 27609267.74,
            "ns_per_operation": 982.886,
            "observed_batch": 28090,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 30550751.856000002,
            "ns_per_operation": 979.128,
            "observed_batch": 31202,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 31202,
        "source_sha256": "637b96cd8926ffadf728af4949b43fa6b62fffb61968c14c716e0c203a1e5e6d",
        "wasm_sha256": "d43d5dd71d5ca1c3dae820aa85c13a2d69415e028b8d48097fed4558f743163a"
      },
      "0:scalar_br_table_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 13325,
            "sides": [
              {
                "calibrated_total_ns": 46712066.7,
                "ns_per_operation": 3505.596,
                "observed_batch": 13325,
                "requested_batch": 13325,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 32560050.575,
                "ns_per_operation": 2443.531,
                "observed_batch": 13325,
                "requested_batch": 13325,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 46712066.7,
            "ns_per_operation": 3505.596,
            "observed_batch": 13325,
            "requested_batch": 13325,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 32560050.575,
            "ns_per_operation": 2443.531,
            "observed_batch": 13325,
            "requested_batch": 13325,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 43774290.625,
            "ns_per_operation": 3285.125,
            "observed_batch": 13325,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 38584855.647,
            "ns_per_operation": 2949.687,
            "observed_batch": 13081,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 13325,
        "source_sha256": "637b96cd8926ffadf728af4949b43fa6b62fffb61968c14c716e0c203a1e5e6d",
        "wasm_sha256": "d43d5dd71d5ca1c3dae820aa85c13a2d69415e028b8d48097fed4558f743163a"
      },
      "0:scalar_br_table_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 2,
            "sides": [
              {
                "calibrated_total_ns": 26685397.0,
                "ns_per_operation": 13342698.5,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 25069695.0,
                "ns_per_operation": 12534847.5,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 26685397.0,
            "ns_per_operation": 13342698.5,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 25069695.0,
            "ns_per_operation": 12534847.5,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 35647192.0,
            "ns_per_operation": 17823596.0,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 13085887.0,
            "ns_per_operation": 13085887.0,
            "observed_batch": 1,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 2,
        "source_sha256": "637b96cd8926ffadf728af4949b43fa6b62fffb61968c14c716e0c203a1e5e6d",
        "wasm_sha256": "d43d5dd71d5ca1c3dae820aa85c13a2d69415e028b8d48097fed4558f743163a"
      },
      "0:scalar_br_table_n1024": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 19,
            "sides": [
              {
                "calibrated_total_ns": 23826098.002,
                "ns_per_operation": 1254005.158,
                "observed_batch": 19,
                "requested_batch": 19,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 25764356.991,
                "ns_per_operation": 1356018.789,
                "observed_batch": 19,
                "requested_batch": 19,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 23826098.002,
            "ns_per_operation": 1254005.158,
            "observed_batch": 19,
            "requested_batch": 19,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 25764356.991,
            "ns_per_operation": 1356018.789,
            "observed_batch": 19,
            "requested_batch": 19,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 33324441.998,
            "ns_per_operation": 1960261.294,
            "observed_batch": 17,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 25580664.003,
            "ns_per_operation": 1346350.737,
            "observed_batch": 19,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 19,
        "source_sha256": "637b96cd8926ffadf728af4949b43fa6b62fffb61968c14c716e0c203a1e5e6d",
        "wasm_sha256": "d43d5dd71d5ca1c3dae820aa85c13a2d69415e028b8d48097fed4558f743163a"
      },
      "0:scalar_br_table_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1398,
            "sides": [
              {
                "calibrated_total_ns": 48358743.647999994,
                "ns_per_operation": 34591.376,
                "observed_batch": 1398,
                "requested_batch": 1398,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 51930538.584,
                "ns_per_operation": 37146.308,
                "observed_batch": 1398,
                "requested_batch": 1398,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 48358743.647999994,
            "ns_per_operation": 34591.376,
            "observed_batch": 1398,
            "requested_batch": 1398,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 51930538.584,
            "ns_per_operation": 37146.308,
            "observed_batch": 1398,
            "requested_batch": 1398,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 57919053.324,
            "ns_per_operation": 41429.938,
            "observed_batch": 1398,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 50250694.536,
            "ns_per_operation": 36047.844,
            "observed_batch": 1394,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1398,
        "source_sha256": "637b96cd8926ffadf728af4949b43fa6b62fffb61968c14c716e0c203a1e5e6d",
        "wasm_sha256": "d43d5dd71d5ca1c3dae820aa85c13a2d69415e028b8d48097fed4558f743163a"
      },
      "0:scalar_br_table_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 750,
            "sides": [
              {
                "calibrated_total_ns": 31708377.000000004,
                "ns_per_operation": 42277.836,
                "observed_batch": 750,
                "requested_batch": 750,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 35230050.75,
                "ns_per_operation": 46973.401,
                "observed_batch": 750,
                "requested_batch": 750,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 31708377.000000004,
            "ns_per_operation": 42277.836,
            "observed_batch": 750,
            "requested_batch": 750,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 35230050.75,
            "ns_per_operation": 46973.401,
            "observed_batch": 750,
            "requested_batch": 750,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 35593389.974,
            "ns_per_operation": 47712.319,
            "observed_batch": 746,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 33474519.75,
            "ns_per_operation": 44632.693,
            "observed_batch": 750,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 750,
        "source_sha256": "637b96cd8926ffadf728af4949b43fa6b62fffb61968c14c716e0c203a1e5e6d",
        "wasm_sha256": "d43d5dd71d5ca1c3dae820aa85c13a2d69415e028b8d48097fed4558f743163a"
      },
      "0:scalar_branch_entropy_n0": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 23981,
            "sides": [
              {
                "calibrated_total_ns": 20930472.914,
                "ns_per_operation": 872.794,
                "observed_batch": 23981,
                "requested_batch": 23981,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 19698832.735,
                "ns_per_operation": 821.435,
                "observed_batch": 23981,
                "requested_batch": 23981,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 26783,
            "sides": [
              {
                "calibrated_total_ns": 20456560.787,
                "ns_per_operation": 763.789,
                "observed_batch": 26783,
                "requested_batch": 26783,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 21883960.772,
                "ns_per_operation": 817.084,
                "observed_batch": 26783,
                "requested_batch": 26783,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 20456560.787,
            "ns_per_operation": 763.789,
            "observed_batch": 26783,
            "requested_batch": 26783,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 21883960.772,
            "ns_per_operation": 817.084,
            "observed_batch": 26783,
            "requested_batch": 26783,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 26725241.754,
            "ns_per_operation": 1114.434,
            "observed_batch": 23981,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 17395402.32,
            "ns_per_operation": 1123.734,
            "observed_batch": 15480,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 26783,
        "source_sha256": "6b4861d8bfbbd7e2718e577ec2d479bfff32e94dc47a3b7dda9a1b363baa98b6",
        "wasm_sha256": "e4c87dbab50a2438cb1f749bc6fd6f9c834c81484acd42864735bd1ad1b6dec0"
      },
      "0:scalar_branch_entropy_n1": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 8207,
            "sides": [
              {
                "calibrated_total_ns": 18679591.592,
                "ns_per_operation": 2276.056,
                "observed_batch": 8207,
                "requested_batch": 8207,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 18985466.482,
                "ns_per_operation": 2313.326,
                "observed_batch": 8207,
                "requested_batch": 8207,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 9666,
            "sides": [
              {
                "calibrated_total_ns": 28868533.596,
                "ns_per_operation": 2986.606,
                "observed_batch": 9666,
                "requested_batch": 9666,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 27395174.214,
                "ns_per_operation": 2834.179,
                "observed_batch": 9666,
                "requested_batch": 9666,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 28868533.596,
            "ns_per_operation": 2986.606,
            "observed_batch": 9666,
            "requested_batch": 9666,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27395174.214,
            "ns_per_operation": 2834.179,
            "observed_batch": 9666,
            "requested_batch": 9666,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 26001220.432,
            "ns_per_operation": 3168.176,
            "observed_batch": 8207,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 1855450.924,
            "ns_per_operation": 8752.127,
            "observed_batch": 212,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 9666,
        "source_sha256": "6b4861d8bfbbd7e2718e577ec2d479bfff32e94dc47a3b7dda9a1b363baa98b6",
        "wasm_sha256": "e4c87dbab50a2438cb1f749bc6fd6f9c834c81484acd42864735bd1ad1b6dec0"
      },
      "0:scalar_branch_entropy_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 2,
            "sides": [
              {
                "calibrated_total_ns": 26625738.0,
                "ns_per_operation": 13312869.0,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 33575094.0,
                "ns_per_operation": 16787547.0,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 26625738.0,
            "ns_per_operation": 13312869.0,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 33575094.0,
            "ns_per_operation": 16787547.0,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 22575052.0,
            "ns_per_operation": 11287526.0,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 31010590.0,
            "ns_per_operation": 15505295.0,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 2,
        "source_sha256": "6b4861d8bfbbd7e2718e577ec2d479bfff32e94dc47a3b7dda9a1b363baa98b6",
        "wasm_sha256": "e4c87dbab50a2438cb1f749bc6fd6f9c834c81484acd42864735bd1ad1b6dec0"
      },
      "0:scalar_branch_entropy_n1024": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 17,
            "sides": [
              {
                "calibrated_total_ns": 29297394.006,
                "ns_per_operation": 1723376.118,
                "observed_batch": 17,
                "requested_batch": 17,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 23516307.995,
                "ns_per_operation": 1383312.235,
                "observed_batch": 17,
                "requested_batch": 17,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 29297394.006,
            "ns_per_operation": 1723376.118,
            "observed_batch": 17,
            "requested_batch": 17,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 23516307.995,
            "ns_per_operation": 1383312.235,
            "observed_batch": 17,
            "requested_batch": 17,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 28142308.997,
            "ns_per_operation": 1655429.941,
            "observed_batch": 17,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 19463024.0,
            "ns_per_operation": 1390216.0,
            "observed_batch": 14,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 17,
        "source_sha256": "6b4861d8bfbbd7e2718e577ec2d479bfff32e94dc47a3b7dda9a1b363baa98b6",
        "wasm_sha256": "e4c87dbab50a2438cb1f749bc6fd6f9c834c81484acd42864735bd1ad1b6dec0"
      },
      "0:scalar_branch_entropy_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1428,
            "sides": [
              {
                "calibrated_total_ns": 53711081.256000005,
                "ns_per_operation": 37612.802,
                "observed_batch": 1428,
                "requested_batch": 1428,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 37028755.428,
                "ns_per_operation": 25930.501,
                "observed_batch": 1428,
                "requested_batch": 1428,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 53711081.256000005,
            "ns_per_operation": 37612.802,
            "observed_batch": 1428,
            "requested_batch": 1428,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 37028755.428,
            "ns_per_operation": 25930.501,
            "observed_batch": 1428,
            "requested_batch": 1428,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 41356962.291999996,
            "ns_per_operation": 30121.604,
            "observed_batch": 1373,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 54359921.616,
            "ns_per_operation": 38067.172,
            "observed_batch": 1428,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1428,
        "source_sha256": "6b4861d8bfbbd7e2718e577ec2d479bfff32e94dc47a3b7dda9a1b363baa98b6",
        "wasm_sha256": "e4c87dbab50a2438cb1f749bc6fd6f9c834c81484acd42864735bd1ad1b6dec0"
      },
      "0:scalar_branch_entropy_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 766,
            "sides": [
              {
                "calibrated_total_ns": 33098162.939999998,
                "ns_per_operation": 43209.09,
                "observed_batch": 766,
                "requested_batch": 766,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 32527227.138,
                "ns_per_operation": 42463.743,
                "observed_batch": 766,
                "requested_batch": 766,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 33098162.939999998,
            "ns_per_operation": 43209.09,
            "observed_batch": 766,
            "requested_batch": 766,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 32527227.138,
            "ns_per_operation": 42463.743,
            "observed_batch": 766,
            "requested_batch": 766,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 36375352.996,
            "ns_per_operation": 47487.406,
            "observed_batch": 766,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 33696621.236999996,
            "ns_per_operation": 44749.829,
            "observed_batch": 753,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 766,
        "source_sha256": "6b4861d8bfbbd7e2718e577ec2d479bfff32e94dc47a3b7dda9a1b363baa98b6",
        "wasm_sha256": "e4c87dbab50a2438cb1f749bc6fd6f9c834c81484acd42864735bd1ad1b6dec0"
      },
      "0:scalar_call_ref_n0": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 25221,
            "sides": [
              {
                "calibrated_total_ns": 24824828.532,
                "ns_per_operation": 984.292,
                "observed_batch": 25221,
                "requested_batch": 25221,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 29296108.296,
                "ns_per_operation": 1161.576,
                "observed_batch": 25221,
                "requested_batch": 25221,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 24824828.532,
            "ns_per_operation": 984.292,
            "observed_batch": 25221,
            "requested_batch": 25221,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 29296108.296,
            "ns_per_operation": 1161.576,
            "observed_batch": 25221,
            "requested_batch": 25221,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 24231151.413000003,
            "ns_per_operation": 960.753,
            "observed_batch": 25221,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 23379329.4,
            "ns_per_operation": 929.3,
            "observed_batch": 25158,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 25221,
        "source_sha256": "0323154b79000eeeb0a63310541ec91b6c4d91e32a6a3406125a30f2c8e53a12",
        "wasm_sha256": "4f766dc9e29324ea942c892982363dec0ade98565288878a70967ce3b33f322b"
      },
      "0:scalar_call_ref_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 12683,
            "sides": [
              {
                "calibrated_total_ns": 31810612.790000003,
                "ns_per_operation": 2508.13,
                "observed_batch": 12683,
                "requested_batch": 12683,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 34570116.515,
                "ns_per_operation": 2725.705,
                "observed_batch": 12683,
                "requested_batch": 12683,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 31810612.790000003,
            "ns_per_operation": 2508.13,
            "observed_batch": 12683,
            "requested_batch": 12683,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 34570116.515,
            "ns_per_operation": 2725.705,
            "observed_batch": 12683,
            "requested_batch": 12683,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 28526895.309,
            "ns_per_operation": 2249.223,
            "observed_batch": 12683,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 24215969.856,
            "ns_per_operation": 1926.336,
            "observed_batch": 12571,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 12683,
        "source_sha256": "0323154b79000eeeb0a63310541ec91b6c4d91e32a6a3406125a30f2c8e53a12",
        "wasm_sha256": "4f766dc9e29324ea942c892982363dec0ade98565288878a70967ce3b33f322b"
      },
      "0:scalar_call_ref_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 2,
            "sides": [
              {
                "calibrated_total_ns": 26879269.0,
                "ns_per_operation": 13439634.5,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 21574288.0,
                "ns_per_operation": 10787144.0,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 26879269.0,
            "ns_per_operation": 13439634.5,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 21574288.0,
            "ns_per_operation": 10787144.0,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 19890635.0,
            "ns_per_operation": 9945317.5,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 23740638.0,
            "ns_per_operation": 11870319.0,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 2,
        "source_sha256": "0323154b79000eeeb0a63310541ec91b6c4d91e32a6a3406125a30f2c8e53a12",
        "wasm_sha256": "4f766dc9e29324ea942c892982363dec0ade98565288878a70967ce3b33f322b"
      },
      "0:scalar_call_ref_n1024": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 16,
            "sides": [
              {
                "calibrated_total_ns": 22554559.008,
                "ns_per_operation": 1409659.938,
                "observed_batch": 16,
                "requested_batch": 16,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 20412754.0,
                "ns_per_operation": 1275797.125,
                "observed_batch": 16,
                "requested_batch": 16,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 22554559.008,
            "ns_per_operation": 1409659.938,
            "observed_batch": 16,
            "requested_batch": 16,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 20412754.0,
            "ns_per_operation": 1275797.125,
            "observed_batch": 16,
            "requested_batch": 16,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 18876484.0,
            "ns_per_operation": 1179780.25,
            "observed_batch": 16,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 18638343.0,
            "ns_per_operation": 1242556.2,
            "observed_batch": 15,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 16,
        "source_sha256": "0323154b79000eeeb0a63310541ec91b6c4d91e32a6a3406125a30f2c8e53a12",
        "wasm_sha256": "4f766dc9e29324ea942c892982363dec0ade98565288878a70967ce3b33f322b"
      },
      "0:scalar_call_ref_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1506,
            "sides": [
              {
                "calibrated_total_ns": 31514277.389999997,
                "ns_per_operation": 20925.815,
                "observed_batch": 1506,
                "requested_batch": 1506,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 38159351.79,
                "ns_per_operation": 25338.215,
                "observed_batch": 1506,
                "requested_batch": 1506,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 31514277.389999997,
            "ns_per_operation": 20925.815,
            "observed_batch": 1506,
            "requested_batch": 1506,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 38159351.79,
            "ns_per_operation": 25338.215,
            "observed_batch": 1506,
            "requested_batch": 1506,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 33946111.760000005,
            "ns_per_operation": 22936.562,
            "observed_batch": 1480,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 41354570.244,
            "ns_per_operation": 27459.874,
            "observed_batch": 1506,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1506,
        "source_sha256": "0323154b79000eeeb0a63310541ec91b6c4d91e32a6a3406125a30f2c8e53a12",
        "wasm_sha256": "4f766dc9e29324ea942c892982363dec0ade98565288878a70967ce3b33f322b"
      },
      "0:scalar_call_ref_n32": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 220,
            "sides": [
              {
                "calibrated_total_ns": 14743497.999999998,
                "ns_per_operation": 67015.9,
                "observed_batch": 220,
                "requested_batch": 220,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 10792937.98,
                "ns_per_operation": 49058.809,
                "observed_batch": 220,
                "requested_batch": 220,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 449,
            "sides": [
              {
                "calibrated_total_ns": 21524879.951,
                "ns_per_operation": 47939.599,
                "observed_batch": 449,
                "requested_batch": 449,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 22722041.915999997,
                "ns_per_operation": 50605.884,
                "observed_batch": 449,
                "requested_batch": 449,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 21524879.951,
            "ns_per_operation": 47939.599,
            "observed_batch": 449,
            "requested_batch": 449,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22722041.915999997,
            "ns_per_operation": 50605.884,
            "observed_batch": 449,
            "requested_batch": 449,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 6146610.007999999,
            "ns_per_operation": 109760.893,
            "observed_batch": 56,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 12538812.0,
            "ns_per_operation": 56994.6,
            "observed_batch": 220,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 449,
        "source_sha256": "0323154b79000eeeb0a63310541ec91b6c4d91e32a6a3406125a30f2c8e53a12",
        "wasm_sha256": "4f766dc9e29324ea942c892982363dec0ade98565288878a70967ce3b33f322b"
      },
      "0:scalar_dependent_n0": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 24661,
            "sides": [
              {
                "calibrated_total_ns": 21294255.619000003,
                "ns_per_operation": 863.479,
                "observed_batch": 24661,
                "requested_batch": 24661,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 26596543.246000003,
                "ns_per_operation": 1078.486,
                "observed_batch": 24661,
                "requested_batch": 24661,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 21294255.619000003,
            "ns_per_operation": 863.479,
            "observed_batch": 24661,
            "requested_batch": 24661,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 26596543.246000003,
            "ns_per_operation": 1078.486,
            "observed_batch": 24661,
            "requested_batch": 24661,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 20196693.152999997,
            "ns_per_operation": 818.973,
            "observed_batch": 24661,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 17455343.099999998,
            "ns_per_operation": 762.775,
            "observed_batch": 22884,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 24661,
        "source_sha256": "0dd5e9778eb09bac9ee432e5fece3f432d034ea9878d2302972142501f0c60c6",
        "wasm_sha256": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c"
      },
      "0:scalar_dependent_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 16300,
            "sides": [
              {
                "calibrated_total_ns": 29659838.6,
                "ns_per_operation": 1819.622,
                "observed_batch": 16300,
                "requested_batch": 16300,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 27567538.0,
                "ns_per_operation": 1691.26,
                "observed_batch": 16300,
                "requested_batch": 16300,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 29659838.6,
            "ns_per_operation": 1819.622,
            "observed_batch": 16300,
            "requested_batch": 16300,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27567538.0,
            "ns_per_operation": 1691.26,
            "observed_batch": 16300,
            "requested_batch": 16300,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 22165617.484,
            "ns_per_operation": 1485.034,
            "observed_batch": 14926,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 24829985.599999998,
            "ns_per_operation": 1523.312,
            "observed_batch": 16300,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 16300,
        "source_sha256": "0dd5e9778eb09bac9ee432e5fece3f432d034ea9878d2302972142501f0c60c6",
        "wasm_sha256": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c"
      },
      "0:scalar_dependent_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 3,
            "sides": [
              {
                "calibrated_total_ns": 29903216.001,
                "ns_per_operation": 9967738.667,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 21894033.0,
                "ns_per_operation": 7298011.0,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 29903216.001,
            "ns_per_operation": 9967738.667,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 21894033.0,
            "ns_per_operation": 7298011.0,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 28285491.0,
            "ns_per_operation": 9428497.0,
            "observed_batch": 3,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 21532323.0,
            "ns_per_operation": 7177441.0,
            "observed_batch": 3,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 3,
        "source_sha256": "0dd5e9778eb09bac9ee432e5fece3f432d034ea9878d2302972142501f0c60c6",
        "wasm_sha256": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c"
      },
      "0:scalar_dependent_n1024": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 22,
            "sides": [
              {
                "calibrated_total_ns": 26936527.992,
                "ns_per_operation": 1224387.636,
                "observed_batch": 22,
                "requested_batch": 22,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 16606119.002,
                "ns_per_operation": 754823.591,
                "observed_batch": 22,
                "requested_batch": 22,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 30,
            "sides": [
              {
                "calibrated_total_ns": 24511104.99,
                "ns_per_operation": 817036.833,
                "observed_batch": 30,
                "requested_batch": 30,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 23307881.009999998,
                "ns_per_operation": 776929.367,
                "observed_batch": 30,
                "requested_batch": 30,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 24511104.99,
            "ns_per_operation": 817036.833,
            "observed_batch": 30,
            "requested_batch": 30,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 23307881.009999998,
            "ns_per_operation": 776929.367,
            "observed_batch": 30,
            "requested_batch": 30,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 11388660.998,
            "ns_per_operation": 876050.846,
            "observed_batch": 13,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 25627401.008,
            "ns_per_operation": 1164881.864,
            "observed_batch": 22,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 30,
        "source_sha256": "0dd5e9778eb09bac9ee432e5fece3f432d034ea9878d2302972142501f0c60c6",
        "wasm_sha256": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c"
      },
      "0:scalar_dependent_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 2380,
            "sides": [
              {
                "calibrated_total_ns": 53372285.400000006,
                "ns_per_operation": 22425.33,
                "observed_batch": 2380,
                "requested_batch": 2380,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 53211207.0,
                "ns_per_operation": 22357.65,
                "observed_batch": 2380,
                "requested_batch": 2380,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 53372285.400000006,
            "ns_per_operation": 22425.33,
            "observed_batch": 2380,
            "requested_batch": 2380,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 53211207.0,
            "ns_per_operation": 22357.65,
            "observed_batch": 2380,
            "requested_batch": 2380,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 33254417.944,
            "ns_per_operation": 14260.042,
            "observed_batch": 2332,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 40200660.92,
            "ns_per_operation": 16891.034,
            "observed_batch": 2380,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 2380,
        "source_sha256": "0dd5e9778eb09bac9ee432e5fece3f432d034ea9878d2302972142501f0c60c6",
        "wasm_sha256": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c"
      },
      "0:scalar_dependent_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1306,
            "sides": [
              {
                "calibrated_total_ns": 33336411.398,
                "ns_per_operation": 25525.583,
                "observed_batch": 1306,
                "requested_batch": 1306,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 32952054.292,
                "ns_per_operation": 25231.282,
                "observed_batch": 1306,
                "requested_batch": 1306,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 33336411.398,
            "ns_per_operation": 25525.583,
            "observed_batch": 1306,
            "requested_batch": 1306,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 32952054.292,
            "ns_per_operation": 25231.282,
            "observed_batch": 1306,
            "requested_batch": 1306,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 24628178.491,
            "ns_per_operation": 18930.191,
            "observed_batch": 1301,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 30981249.358,
            "ns_per_operation": 23722.243,
            "observed_batch": 1306,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1306,
        "source_sha256": "0dd5e9778eb09bac9ee432e5fece3f432d034ea9878d2302972142501f0c60c6",
        "wasm_sha256": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c"
      },
      "0:scalar_direct_calls_n0": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 24510,
            "sides": [
              {
                "calibrated_total_ns": 24871939.17,
                "ns_per_operation": 1014.767,
                "observed_batch": 24510,
                "requested_batch": 24510,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 20259353.25,
                "ns_per_operation": 826.575,
                "observed_batch": 24510,
                "requested_batch": 24510,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 24871939.17,
            "ns_per_operation": 1014.767,
            "observed_batch": 24510,
            "requested_batch": 24510,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 20259353.25,
            "ns_per_operation": 826.575,
            "observed_batch": 24510,
            "requested_batch": 24510,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 9184832.504999999,
            "ns_per_operation": 878.511,
            "observed_batch": 10455,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27495318.0,
            "ns_per_operation": 1121.8,
            "observed_batch": 24510,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 24510,
        "source_sha256": "0a075601a2a651f9cb95e8ee0404c64bbf076d17bf242b9c88fecbb8cb973826",
        "wasm_sha256": "f6adbecce99c05bf06f213f4f5c0d5d9f001074acbbb2e6bbe814696bb866e45"
      },
      "0:scalar_direct_calls_n1": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 8393,
            "sides": [
              {
                "calibrated_total_ns": 24190245.849000003,
                "ns_per_operation": 2882.193,
                "observed_batch": 8393,
                "requested_batch": 8393,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 18455325.735,
                "ns_per_operation": 2198.895,
                "observed_batch": 8393,
                "requested_batch": 8393,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 10006,
            "sides": [
              {
                "calibrated_total_ns": 31149838.696,
                "ns_per_operation": 3113.116,
                "observed_batch": 10006,
                "requested_batch": 10006,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 28253261.792,
                "ns_per_operation": 2823.632,
                "observed_batch": 10006,
                "requested_batch": 10006,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 31149838.696,
            "ns_per_operation": 3113.116,
            "observed_batch": 10006,
            "requested_batch": 10006,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 28253261.792,
            "ns_per_operation": 2823.632,
            "observed_batch": 10006,
            "requested_batch": 10006,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 22361302.040000003,
            "ns_per_operation": 2664.28,
            "observed_batch": 8393,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 14481099.618,
            "ns_per_operation": 2045.931,
            "observed_batch": 7078,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 10006,
        "source_sha256": "0a075601a2a651f9cb95e8ee0404c64bbf076d17bf242b9c88fecbb8cb973826",
        "wasm_sha256": "f6adbecce99c05bf06f213f4f5c0d5d9f001074acbbb2e6bbe814696bb866e45"
      },
      "0:scalar_direct_calls_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 3,
            "sides": [
              {
                "calibrated_total_ns": 36346302.999,
                "ns_per_operation": 12115434.333,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 33413079.999,
                "ns_per_operation": 11137693.333,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 36346302.999,
            "ns_per_operation": 12115434.333,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 33413079.999,
            "ns_per_operation": 11137693.333,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 33747930.0,
            "ns_per_operation": 16873965.0,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 29909766.999,
            "ns_per_operation": 9969922.333,
            "observed_batch": 3,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 3,
        "source_sha256": "0a075601a2a651f9cb95e8ee0404c64bbf076d17bf242b9c88fecbb8cb973826",
        "wasm_sha256": "f6adbecce99c05bf06f213f4f5c0d5d9f001074acbbb2e6bbe814696bb866e45"
      },
      "0:scalar_direct_calls_n1024": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 17,
            "sides": [
              {
                "calibrated_total_ns": 21725861.994,
                "ns_per_operation": 1277991.882,
                "observed_batch": 17,
                "requested_batch": 17,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 23924642.997,
                "ns_per_operation": 1407331.941,
                "observed_batch": 17,
                "requested_batch": 17,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 21725861.994,
            "ns_per_operation": 1277991.882,
            "observed_batch": 17,
            "requested_batch": 17,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 23924642.997,
            "ns_per_operation": 1407331.941,
            "observed_batch": 17,
            "requested_batch": 17,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 23547576.996,
            "ns_per_operation": 1385151.588,
            "observed_batch": 17,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 12679369.002,
            "ns_per_operation": 1408818.778,
            "observed_batch": 9,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 17,
        "source_sha256": "0a075601a2a651f9cb95e8ee0404c64bbf076d17bf242b9c88fecbb8cb973826",
        "wasm_sha256": "f6adbecce99c05bf06f213f4f5c0d5d9f001074acbbb2e6bbe814696bb866e45"
      },
      "0:scalar_direct_calls_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1588,
            "sides": [
              {
                "calibrated_total_ns": 28411368.52,
                "ns_per_operation": 17891.29,
                "observed_batch": 1588,
                "requested_batch": 1588,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 37232860.967999995,
                "ns_per_operation": 23446.386,
                "observed_batch": 1588,
                "requested_batch": 1588,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 28411368.52,
            "ns_per_operation": 17891.29,
            "observed_batch": 1588,
            "requested_batch": 1588,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 37232860.967999995,
            "ns_per_operation": 23446.386,
            "observed_batch": 1588,
            "requested_batch": 1588,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 23681777.522,
            "ns_per_operation": 21646.963,
            "observed_batch": 1094,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 33445522.256,
            "ns_per_operation": 21061.412,
            "observed_batch": 1588,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1588,
        "source_sha256": "0a075601a2a651f9cb95e8ee0404c64bbf076d17bf242b9c88fecbb8cb973826",
        "wasm_sha256": "f6adbecce99c05bf06f213f4f5c0d5d9f001074acbbb2e6bbe814696bb866e45"
      },
      "0:scalar_direct_calls_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 908,
            "sides": [
              {
                "calibrated_total_ns": 33642755.644,
                "ns_per_operation": 37051.493,
                "observed_batch": 908,
                "requested_batch": 908,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 27946376.784,
                "ns_per_operation": 30777.948,
                "observed_batch": 908,
                "requested_batch": 908,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 33642755.644,
            "ns_per_operation": 37051.493,
            "observed_batch": 908,
            "requested_batch": 908,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27946376.784,
            "ns_per_operation": 30777.948,
            "observed_batch": 908,
            "requested_batch": 908,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 34667006.884,
            "ns_per_operation": 38179.523,
            "observed_batch": 908,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 32692938.217000004,
            "ns_per_operation": 42960.497,
            "observed_batch": 761,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 908,
        "source_sha256": "0a075601a2a651f9cb95e8ee0404c64bbf076d17bf242b9c88fecbb8cb973826",
        "wasm_sha256": "f6adbecce99c05bf06f213f4f5c0d5d9f001074acbbb2e6bbe814696bb866e45"
      },
      "0:scalar_dynamic_api_n0": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 49505,
            "sides": [
              {
                "calibrated_total_ns": 20135317.165,
                "ns_per_operation": 406.733,
                "observed_batch": 49505,
                "requested_batch": 49505,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 27997899.285,
                "ns_per_operation": 565.557,
                "observed_batch": 49505,
                "requested_batch": 49505,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 20135317.165,
            "ns_per_operation": 406.733,
            "observed_batch": 49505,
            "requested_batch": 49505,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27997899.285,
            "ns_per_operation": 565.557,
            "observed_batch": 49505,
            "requested_batch": 49505,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 21174971.669999998,
            "ns_per_operation": 427.734,
            "observed_batch": 49505,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 17013576.04,
            "ns_per_operation": 375.145,
            "observed_batch": 45352,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 49505,
        "source_sha256": "f02cc93a4a43691ce5be8782fa2df454fc2468a7935117ea18104a7dce22cb2d",
        "wasm_sha256": "c493d0aae68d0c5aa67fc0b562e616c45eb3d42b0302951158a763fa202ce589"
      },
      "0:scalar_dynamic_api_n1": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 47733,
            "sides": [
              {
                "calibrated_total_ns": 15310121.085,
                "ns_per_operation": 320.745,
                "observed_batch": 47733,
                "requested_batch": 47733,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 16056856.137,
                "ns_per_operation": 336.389,
                "observed_batch": 47733,
                "requested_batch": 47733,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 68591,
            "sides": [
              {
                "calibrated_total_ns": 31996123.906999998,
                "ns_per_operation": 466.477,
                "observed_batch": 68591,
                "requested_batch": 68591,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 36324421.78,
                "ns_per_operation": 529.58,
                "observed_batch": 68591,
                "requested_batch": 68591,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 31996123.906999998,
            "ns_per_operation": 466.477,
            "observed_batch": 68591,
            "requested_batch": 68591,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 36324421.78,
            "ns_per_operation": 529.58,
            "observed_batch": 68591,
            "requested_batch": 68591,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 23963729.265,
            "ns_per_operation": 670.971,
            "observed_batch": 35715,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 16460247.719999999,
            "ns_per_operation": 344.84,
            "observed_batch": 47733,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 68591,
        "source_sha256": "f02cc93a4a43691ce5be8782fa2df454fc2468a7935117ea18104a7dce22cb2d",
        "wasm_sha256": "c493d0aae68d0c5aa67fc0b562e616c45eb3d42b0302951158a763fa202ce589"
      },
      "0:scalar_dynamic_api_n10000": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 53764,
            "sides": [
              {
                "calibrated_total_ns": 21755064.96,
                "ns_per_operation": 404.64,
                "observed_batch": 53764,
                "requested_batch": 53764,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 18704764.419999998,
                "ns_per_operation": 347.905,
                "observed_batch": 53764,
                "requested_batch": 53764,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 63236,
            "sides": [
              {
                "calibrated_total_ns": 22710324.096,
                "ns_per_operation": 359.136,
                "observed_batch": 63236,
                "requested_batch": 63236,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 25693039.744,
                "ns_per_operation": 406.304,
                "observed_batch": 63236,
                "requested_batch": 63236,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 22710324.096,
            "ns_per_operation": 359.136,
            "observed_batch": 63236,
            "requested_batch": 63236,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 25693039.744,
            "ns_per_operation": 406.304,
            "observed_batch": 63236,
            "requested_batch": 63236,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 18317986.204,
            "ns_per_operation": 340.711,
            "observed_batch": 53764,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 16180944.329,
            "ns_per_operation": 335.753,
            "observed_batch": 48193,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 63236,
        "source_sha256": "f02cc93a4a43691ce5be8782fa2df454fc2468a7935117ea18104a7dce22cb2d",
        "wasm_sha256": "c493d0aae68d0c5aa67fc0b562e616c45eb3d42b0302951158a763fa202ce589"
      },
      "0:scalar_dynamic_api_n1024": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 55556,
            "sides": [
              {
                "calibrated_total_ns": 18916762.444000002,
                "ns_per_operation": 340.499,
                "observed_batch": 55556,
                "requested_batch": 55556,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 20572775.692,
                "ns_per_operation": 370.307,
                "observed_batch": 55556,
                "requested_batch": 55556,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 64612,
            "sides": [
              {
                "calibrated_total_ns": 21866186.876,
                "ns_per_operation": 338.423,
                "observed_batch": 64612,
                "requested_batch": 64612,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 25277506.64,
                "ns_per_operation": 391.22,
                "observed_batch": 64612,
                "requested_batch": 64612,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 21866186.876,
            "ns_per_operation": 338.423,
            "observed_batch": 64612,
            "requested_batch": 64612,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 25277506.64,
            "ns_per_operation": 391.22,
            "observed_batch": 64612,
            "requested_batch": 64612,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 19190320.188,
            "ns_per_operation": 345.423,
            "observed_batch": 55556,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 13060669.752,
            "ns_per_operation": 342.189,
            "observed_batch": 38168,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 64612,
        "source_sha256": "f02cc93a4a43691ce5be8782fa2df454fc2468a7935117ea18104a7dce22cb2d",
        "wasm_sha256": "c493d0aae68d0c5aa67fc0b562e616c45eb3d42b0302951158a763fa202ce589"
      },
      "0:scalar_dynamic_api_n17": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 50126,
            "sides": [
              {
                "calibrated_total_ns": 18264911.88,
                "ns_per_operation": 364.38,
                "observed_batch": 50126,
                "requested_batch": 50126,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 18241452.912,
                "ns_per_operation": 363.912,
                "observed_batch": 50126,
                "requested_batch": 50126,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 60455,
            "sides": [
              {
                "calibrated_total_ns": 24338336.63,
                "ns_per_operation": 402.586,
                "observed_batch": 60455,
                "requested_batch": 60455,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 21528085.955,
                "ns_per_operation": 356.101,
                "observed_batch": 60455,
                "requested_batch": 60455,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 24338336.63,
            "ns_per_operation": 402.586,
            "observed_batch": 60455,
            "requested_batch": 60455,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 21528085.955,
            "ns_per_operation": 356.101,
            "observed_batch": 60455,
            "requested_batch": 60455,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 20408436.25,
            "ns_per_operation": 412.25,
            "observed_batch": 49505,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 25068413.608,
            "ns_per_operation": 500.108,
            "observed_batch": 50126,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 60455,
        "source_sha256": "f02cc93a4a43691ce5be8782fa2df454fc2468a7935117ea18104a7dce22cb2d",
        "wasm_sha256": "c493d0aae68d0c5aa67fc0b562e616c45eb3d42b0302951158a763fa202ce589"
      },
      "0:scalar_dynamic_api_n32": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 46083,
            "sides": [
              {
                "calibrated_total_ns": 15885409.179000001,
                "ns_per_operation": 344.713,
                "observed_batch": 46083,
                "requested_batch": 46083,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 15301629.735000001,
                "ns_per_operation": 332.045,
                "observed_batch": 46083,
                "requested_batch": 46083,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 66257,
            "sides": [
              {
                "calibrated_total_ns": 35918516.013000004,
                "ns_per_operation": 542.109,
                "observed_batch": 66257,
                "requested_batch": 66257,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 26261028.207,
                "ns_per_operation": 396.351,
                "observed_batch": 66257,
                "requested_batch": 66257,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 35918516.013000004,
            "ns_per_operation": 542.109,
            "observed_batch": 66257,
            "requested_batch": 66257,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 26261028.207,
            "ns_per_operation": 396.351,
            "observed_batch": 66257,
            "requested_batch": 66257,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 18008176.491,
            "ns_per_operation": 390.777,
            "observed_batch": 46083,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 17099604.062999997,
            "ns_per_operation": 371.061,
            "observed_batch": 46083,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 66257,
        "source_sha256": "f02cc93a4a43691ce5be8782fa2df454fc2468a7935117ea18104a7dce22cb2d",
        "wasm_sha256": "c493d0aae68d0c5aa67fc0b562e616c45eb3d42b0302951158a763fa202ce589"
      },
      "0:scalar_host_call_n0": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 28090,
            "sides": [
              {
                "calibrated_total_ns": 33075637.92,
                "ns_per_operation": 1177.488,
                "observed_batch": 28090,
                "requested_batch": 28090,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 32656619.389999997,
                "ns_per_operation": 1162.571,
                "observed_batch": 28090,
                "requested_batch": 28090,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 33075637.92,
            "ns_per_operation": 1177.488,
            "observed_batch": 28090,
            "requested_batch": 28090,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 32656619.389999997,
            "ns_per_operation": 1162.571,
            "observed_batch": 28090,
            "requested_batch": 28090,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 31921644.54,
            "ns_per_operation": 1136.406,
            "observed_batch": 28090,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 29018962.375,
            "ns_per_operation": 1147.675,
            "observed_batch": 25285,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 28090,
        "source_sha256": "4c4026de620eff0b1e9cb4ed4e0440e6045d921267e3962715f6430ac68f5120",
        "wasm_sha256": "c60311d57c929c3ff1d29adb70501e7bbf8419517acbd86c8d072438b610ebb7"
      },
      "0:scalar_host_call_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 14145,
            "sides": [
              {
                "calibrated_total_ns": 27545930.564999998,
                "ns_per_operation": 1947.397,
                "observed_batch": 14145,
                "requested_batch": 14145,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 25124490.45,
                "ns_per_operation": 1776.21,
                "observed_batch": 14145,
                "requested_batch": 14145,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 27545930.564999998,
            "ns_per_operation": 1947.397,
            "observed_batch": 14145,
            "requested_batch": 14145,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 25124490.45,
            "ns_per_operation": 1776.21,
            "observed_batch": 14145,
            "requested_batch": 14145,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 18745857.626,
            "ns_per_operation": 1970.966,
            "observed_batch": 9511,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 30010131.015,
            "ns_per_operation": 2121.607,
            "observed_batch": 14145,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 14145,
        "source_sha256": "4c4026de620eff0b1e9cb4ed4e0440e6045d921267e3962715f6430ac68f5120",
        "wasm_sha256": "c60311d57c929c3ff1d29adb70501e7bbf8419517acbd86c8d072438b610ebb7"
      },
      "0:scalar_host_call_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 3,
            "sides": [
              {
                "calibrated_total_ns": 37807149.999,
                "ns_per_operation": 12602383.333,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 41886608.001,
                "ns_per_operation": 13962202.667,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 37807149.999,
            "ns_per_operation": 12602383.333,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 41886608.001,
            "ns_per_operation": 13962202.667,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 49947861.0,
            "ns_per_operation": 16649287.0,
            "observed_batch": 3,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 17482918.0,
            "ns_per_operation": 17482918.0,
            "observed_batch": 1,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 3,
        "source_sha256": "4c4026de620eff0b1e9cb4ed4e0440e6045d921267e3962715f6430ac68f5120",
        "wasm_sha256": "c60311d57c929c3ff1d29adb70501e7bbf8419517acbd86c8d072438b610ebb7"
      },
      "0:scalar_host_call_n1024": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 33,
            "sides": [
              {
                "calibrated_total_ns": 27288069.996000003,
                "ns_per_operation": 826911.212,
                "observed_batch": 33,
                "requested_batch": 33,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 33166468.005,
                "ns_per_operation": 1005044.485,
                "observed_batch": 33,
                "requested_batch": 33,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 27288069.996000003,
            "ns_per_operation": 826911.212,
            "observed_batch": 33,
            "requested_batch": 33,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 33166468.005,
            "ns_per_operation": 1005044.485,
            "observed_batch": 33,
            "requested_batch": 33,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 21835351.992,
            "ns_per_operation": 909806.333,
            "observed_batch": 24,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 30476746.014000002,
            "ns_per_operation": 923537.758,
            "observed_batch": 33,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 33,
        "source_sha256": "4c4026de620eff0b1e9cb4ed4e0440e6045d921267e3962715f6430ac68f5120",
        "wasm_sha256": "c60311d57c929c3ff1d29adb70501e7bbf8419517acbd86c8d072438b610ebb7"
      },
      "0:scalar_host_call_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1868,
            "sides": [
              {
                "calibrated_total_ns": 41742012.308,
                "ns_per_operation": 22345.831,
                "observed_batch": 1868,
                "requested_batch": 1868,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 32313758.648,
                "ns_per_operation": 17298.586,
                "observed_batch": 1868,
                "requested_batch": 1868,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 41742012.308,
            "ns_per_operation": 22345.831,
            "observed_batch": 1868,
            "requested_batch": 1868,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 32313758.648,
            "ns_per_operation": 17298.586,
            "observed_batch": 1868,
            "requested_batch": 1868,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 37764472.436,
            "ns_per_operation": 20216.527,
            "observed_batch": 1868,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 1596058.016,
            "ns_per_operation": 28501.036,
            "observed_batch": 56,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1868,
        "source_sha256": "4c4026de620eff0b1e9cb4ed4e0440e6045d921267e3962715f6430ac68f5120",
        "wasm_sha256": "c60311d57c929c3ff1d29adb70501e7bbf8419517acbd86c8d072438b610ebb7"
      },
      "0:scalar_host_call_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1169,
            "sides": [
              {
                "calibrated_total_ns": 43481559.373,
                "ns_per_operation": 37195.517,
                "observed_batch": 1169,
                "requested_batch": 1169,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 39865916.02,
                "ns_per_operation": 34102.58,
                "observed_batch": 1169,
                "requested_batch": 1169,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 43481559.373,
            "ns_per_operation": 37195.517,
            "observed_batch": 1169,
            "requested_batch": 1169,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 39865916.02,
            "ns_per_operation": 34102.58,
            "observed_batch": 1169,
            "requested_batch": 1169,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 40650142.008,
            "ns_per_operation": 34773.432,
            "observed_batch": 1169,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27221357.283999998,
            "ns_per_operation": 33941.842,
            "observed_batch": 802,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1169,
        "source_sha256": "4c4026de620eff0b1e9cb4ed4e0440e6045d921267e3962715f6430ac68f5120",
        "wasm_sha256": "c60311d57c929c3ff1d29adb70501e7bbf8419517acbd86c8d072438b610ebb7"
      },
      "0:scalar_indirect_calls_n0": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 25126,
            "sides": [
              {
                "calibrated_total_ns": 19573179.126,
                "ns_per_operation": 779.001,
                "observed_batch": 25126,
                "requested_batch": 25126,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 20225123.448,
                "ns_per_operation": 804.948,
                "observed_batch": 25126,
                "requested_batch": 25126,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 28242,
            "sides": [
              {
                "calibrated_total_ns": 23066455.806,
                "ns_per_operation": 816.743,
                "observed_batch": 28242,
                "requested_batch": 28242,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 29215275.804,
                "ns_per_operation": 1034.462,
                "observed_batch": 28242,
                "requested_batch": 28242,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 23066455.806,
            "ns_per_operation": 816.743,
            "observed_batch": 28242,
            "requested_batch": 28242,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 29215275.804,
            "ns_per_operation": 1034.462,
            "observed_batch": 28242,
            "requested_batch": 28242,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 18556228.528,
            "ns_per_operation": 812.729,
            "observed_batch": 22832,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 17804459.482,
            "ns_per_operation": 708.607,
            "observed_batch": 25126,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 28242,
        "source_sha256": "29024ae23844664a9fea4a809d1ce414de40f243f7e678ce5ea6d811843f7797",
        "wasm_sha256": "4b42e82f54ea6ab7d0f62c70766c904e41d6a408e57fa14b5abbf514c0da78a9"
      },
      "0:scalar_indirect_calls_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 13387,
            "sides": [
              {
                "calibrated_total_ns": 49173355.979,
                "ns_per_operation": 3673.217,
                "observed_batch": 13387,
                "requested_batch": 13387,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 45163882.544,
                "ns_per_operation": 3373.712,
                "observed_batch": 13387,
                "requested_batch": 13387,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 49173355.979,
            "ns_per_operation": 3673.217,
            "observed_batch": 13387,
            "requested_batch": 13387,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 45163882.544,
            "ns_per_operation": 3373.712,
            "observed_batch": 13387,
            "requested_batch": 13387,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 40407906.987,
            "ns_per_operation": 3776.087,
            "observed_batch": 10701,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 34964554.823,
            "ns_per_operation": 2611.829,
            "observed_batch": 13387,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 13387,
        "source_sha256": "29024ae23844664a9fea4a809d1ce414de40f243f7e678ce5ea6d811843f7797",
        "wasm_sha256": "4b42e82f54ea6ab7d0f62c70766c904e41d6a408e57fa14b5abbf514c0da78a9"
      },
      "0:scalar_indirect_calls_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1,
            "sides": [
              {
                "calibrated_total_ns": 25944529.0,
                "ns_per_operation": 25944529.0,
                "observed_batch": 1,
                "requested_batch": 1,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 22872871.0,
                "ns_per_operation": 22872871.0,
                "observed_batch": 1,
                "requested_batch": 1,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 25944529.0,
            "ns_per_operation": 25944529.0,
            "observed_batch": 1,
            "requested_batch": 1,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22872871.0,
            "ns_per_operation": 22872871.0,
            "observed_batch": 1,
            "requested_batch": 1,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 53583661.0,
            "ns_per_operation": 53583661.0,
            "observed_batch": 1,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 21594179.0,
            "ns_per_operation": 21594179.0,
            "observed_batch": 1,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1,
        "source_sha256": "29024ae23844664a9fea4a809d1ce414de40f243f7e678ce5ea6d811843f7797",
        "wasm_sha256": "4b42e82f54ea6ab7d0f62c70766c904e41d6a408e57fa14b5abbf514c0da78a9"
      },
      "0:scalar_indirect_calls_n1024": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 22,
            "sides": [
              {
                "calibrated_total_ns": 30882823.994,
                "ns_per_operation": 1403764.727,
                "observed_batch": 22,
                "requested_batch": 22,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 30371204.996,
                "ns_per_operation": 1380509.318,
                "observed_batch": 22,
                "requested_batch": 22,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 30882823.994,
            "ns_per_operation": 1403764.727,
            "observed_batch": 22,
            "requested_batch": 22,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 30371204.996,
            "ns_per_operation": 1380509.318,
            "observed_batch": 22,
            "requested_batch": 22,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 20651733.998999998,
            "ns_per_operation": 1588594.923,
            "observed_batch": 13,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 31192752.998,
            "ns_per_operation": 1417852.409,
            "observed_batch": 22,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 22,
        "source_sha256": "29024ae23844664a9fea4a809d1ce414de40f243f7e678ce5ea6d811843f7797",
        "wasm_sha256": "4b42e82f54ea6ab7d0f62c70766c904e41d6a408e57fa14b5abbf514c0da78a9"
      },
      "0:scalar_indirect_calls_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1574,
            "sides": [
              {
                "calibrated_total_ns": 34335728.662,
                "ns_per_operation": 21814.313,
                "observed_batch": 1574,
                "requested_batch": 1574,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 29705985.524,
                "ns_per_operation": 18872.926,
                "observed_batch": 1574,
                "requested_batch": 1574,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 34335728.662,
            "ns_per_operation": 21814.313,
            "observed_batch": 1574,
            "requested_batch": 1574,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 29705985.524,
            "ns_per_operation": 18872.926,
            "observed_batch": 1574,
            "requested_batch": 1574,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 3244241.0179999997,
            "ns_per_operation": 20533.171,
            "observed_batch": 158,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 36912618.502000004,
            "ns_per_operation": 23451.473,
            "observed_batch": 1574,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1574,
        "source_sha256": "29024ae23844664a9fea4a809d1ce414de40f243f7e678ce5ea6d811843f7797",
        "wasm_sha256": "4b42e82f54ea6ab7d0f62c70766c904e41d6a408e57fa14b5abbf514c0da78a9"
      },
      "0:scalar_indirect_calls_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 847,
            "sides": [
              {
                "calibrated_total_ns": 33828875.08,
                "ns_per_operation": 39939.64,
                "observed_batch": 847,
                "requested_batch": 847,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 33140203.096,
                "ns_per_operation": 39126.568,
                "observed_batch": 847,
                "requested_batch": 847,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 33828875.08,
            "ns_per_operation": 39939.64,
            "observed_batch": 847,
            "requested_batch": 847,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 33140203.096,
            "ns_per_operation": 39126.568,
            "observed_batch": 847,
            "requested_batch": 847,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 37624996.101,
            "ns_per_operation": 44421.483,
            "observed_batch": 847,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 4157782.0,
            "ns_per_operation": 41577.82,
            "observed_batch": 100,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 847,
        "source_sha256": "29024ae23844664a9fea4a809d1ce414de40f243f7e678ce5ea6d811843f7797",
        "wasm_sha256": "4b42e82f54ea6ab7d0f62c70766c904e41d6a408e57fa14b5abbf514c0da78a9"
      },
      "0:scalar_multivalue_branch_n0": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 25126,
            "sides": [
              {
                "calibrated_total_ns": 23307706.758,
                "ns_per_operation": 927.633,
                "observed_batch": 25126,
                "requested_batch": 25126,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 21661375.86,
                "ns_per_operation": 862.11,
                "observed_batch": 25126,
                "requested_batch": 25126,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 23307706.758,
            "ns_per_operation": 927.633,
            "observed_batch": 25126,
            "requested_batch": 25126,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 21661375.86,
            "ns_per_operation": 862.11,
            "observed_batch": 25126,
            "requested_batch": 25126,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 190822292.56,
            "ns_per_operation": 8014.376,
            "observed_batch": 23810,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 50919849.08,
            "ns_per_operation": 2026.58,
            "observed_batch": 25126,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 25126,
        "source_sha256": "a1f4958100af82889208bb28090d6af7c3e56cbeeccabad4106601c9a9c17014",
        "wasm_sha256": "03ffdc2041ae8ba0c8d078422d36152cf1d35f1293c94a4074fe7e83791743ae"
      },
      "0:scalar_multivalue_branch_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 11642,
            "sides": [
              {
                "calibrated_total_ns": 38805580.08,
                "ns_per_operation": 3333.24,
                "observed_batch": 11642,
                "requested_batch": 11642,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 37931929.474,
                "ns_per_operation": 3258.197,
                "observed_batch": 11642,
                "requested_batch": 11642,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 38805580.08,
            "ns_per_operation": 3333.24,
            "observed_batch": 11642,
            "requested_batch": 11642,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 37931929.474,
            "ns_per_operation": 3258.197,
            "observed_batch": 11642,
            "requested_batch": 11642,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 27974887.775999997,
            "ns_per_operation": 2402.928,
            "observed_batch": 11642,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 31793545.305000003,
            "ns_per_operation": 2907.503,
            "observed_batch": 10935,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 11642,
        "source_sha256": "a1f4958100af82889208bb28090d6af7c3e56cbeeccabad4106601c9a9c17014",
        "wasm_sha256": "03ffdc2041ae8ba0c8d078422d36152cf1d35f1293c94a4074fe7e83791743ae"
      },
      "0:scalar_multivalue_branch_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 2,
            "sides": [
              {
                "calibrated_total_ns": 31688887.0,
                "ns_per_operation": 15844443.5,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 39427241.0,
                "ns_per_operation": 19713620.5,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 31688887.0,
            "ns_per_operation": 15844443.5,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 39427241.0,
            "ns_per_operation": 19713620.5,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 42267385.0,
            "ns_per_operation": 21133692.5,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 36304015.0,
            "ns_per_operation": 18152007.5,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 2,
        "source_sha256": "a1f4958100af82889208bb28090d6af7c3e56cbeeccabad4106601c9a9c17014",
        "wasm_sha256": "03ffdc2041ae8ba0c8d078422d36152cf1d35f1293c94a4074fe7e83791743ae"
      },
      "0:scalar_multivalue_branch_n1024": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 14,
            "sides": [
              {
                "calibrated_total_ns": 26327832.006,
                "ns_per_operation": 1880559.429,
                "observed_batch": 14,
                "requested_batch": 14,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 23501811.004,
                "ns_per_operation": 1678700.786,
                "observed_batch": 14,
                "requested_batch": 14,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 26327832.006,
            "ns_per_operation": 1880559.429,
            "observed_batch": 14,
            "requested_batch": 14,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 23501811.004,
            "ns_per_operation": 1678700.786,
            "observed_batch": 14,
            "requested_batch": 14,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 19129222.0,
            "ns_per_operation": 1912922.2,
            "observed_batch": 10,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 21199518.004,
            "ns_per_operation": 1514251.286,
            "observed_batch": 14,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 14,
        "source_sha256": "a1f4958100af82889208bb28090d6af7c3e56cbeeccabad4106601c9a9c17014",
        "wasm_sha256": "03ffdc2041ae8ba0c8d078422d36152cf1d35f1293c94a4074fe7e83791743ae"
      },
      "0:scalar_multivalue_branch_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1219,
            "sides": [
              {
                "calibrated_total_ns": 52081371.511,
                "ns_per_operation": 42724.669,
                "observed_batch": 1219,
                "requested_batch": 1219,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 53535096.056,
                "ns_per_operation": 43917.224,
                "observed_batch": 1219,
                "requested_batch": 1219,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 52081371.511,
            "ns_per_operation": 42724.669,
            "observed_batch": 1219,
            "requested_batch": 1219,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 53535096.056,
            "ns_per_operation": 43917.224,
            "observed_batch": 1219,
            "requested_batch": 1219,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 50270661.648,
            "ns_per_operation": 44252.343,
            "observed_batch": 1136,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 59570035.926,
            "ns_per_operation": 48867.954,
            "observed_batch": 1219,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1219,
        "source_sha256": "a1f4958100af82889208bb28090d6af7c3e56cbeeccabad4106601c9a9c17014",
        "wasm_sha256": "03ffdc2041ae8ba0c8d078422d36152cf1d35f1293c94a4074fe7e83791743ae"
      },
      "0:scalar_multivalue_branch_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 662,
            "sides": [
              {
                "calibrated_total_ns": 34179486.989999995,
                "ns_per_operation": 51630.645,
                "observed_batch": 662,
                "requested_batch": 662,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 33176069.758,
                "ns_per_operation": 50114.909,
                "observed_batch": 662,
                "requested_batch": 662,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 34179486.989999995,
            "ns_per_operation": 51630.645,
            "observed_batch": 662,
            "requested_batch": 662,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 33176069.758,
            "ns_per_operation": 50114.909,
            "observed_batch": 662,
            "requested_batch": 662,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 30425257.696,
            "ns_per_operation": 46379.966,
            "observed_batch": 656,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 34491596.158,
            "ns_per_operation": 52102.109,
            "observed_batch": 662,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 662,
        "source_sha256": "a1f4958100af82889208bb28090d6af7c3e56cbeeccabad4106601c9a9c17014",
        "wasm_sha256": "03ffdc2041ae8ba0c8d078422d36152cf1d35f1293c94a4074fe7e83791743ae"
      },
      "0:scalar_ordinary_recursion_n0": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 8929,
            "sides": [
              {
                "calibrated_total_ns": 7889967.9860000005,
                "ns_per_operation": 883.634,
                "observed_batch": 8929,
                "requested_batch": 8929,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 7961114.257999999,
                "ns_per_operation": 891.602,
                "observed_batch": 8929,
                "requested_batch": 8929,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 24898,
            "sides": [
              {
                "calibrated_total_ns": 32888515.14,
                "ns_per_operation": 1320.93,
                "observed_batch": 24898,
                "requested_batch": 24898,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 32496122.66,
                "ns_per_operation": 1305.17,
                "observed_batch": 24898,
                "requested_batch": 24898,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 32888515.14,
            "ns_per_operation": 1320.93,
            "observed_batch": 24898,
            "requested_batch": 24898,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 32496122.66,
            "ns_per_operation": 1305.17,
            "observed_batch": 24898,
            "requested_batch": 24898,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 4270303.737,
            "ns_per_operation": 999.369,
            "observed_batch": 4273,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 8254021.174,
            "ns_per_operation": 924.406,
            "observed_batch": 8929,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 24898,
        "source_sha256": "aca6a66637fa36b72f3c528dd5e3e44039b573f363b5b4fb77437e9278623903",
        "wasm_sha256": "e1bd7ea6ada8b15b2770ac387f8fc2256f20d7f87c4bc34e7559b239d8875ba1"
      },
      "0:scalar_ordinary_recursion_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 15152,
            "sides": [
              {
                "calibrated_total_ns": 29112461.872,
                "ns_per_operation": 1921.361,
                "observed_batch": 15152,
                "requested_batch": 15152,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 29248026.816,
                "ns_per_operation": 1930.308,
                "observed_batch": 15152,
                "requested_batch": 15152,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 29112461.872,
            "ns_per_operation": 1921.361,
            "observed_batch": 15152,
            "requested_batch": 15152,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 29248026.816,
            "ns_per_operation": 1930.308,
            "observed_batch": 15152,
            "requested_batch": 15152,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 14871031.231,
            "ns_per_operation": 1904.833,
            "observed_batch": 7807,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27135959.232,
            "ns_per_operation": 1790.916,
            "observed_batch": 15152,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 15152,
        "source_sha256": "aca6a66637fa36b72f3c528dd5e3e44039b573f363b5b4fb77437e9278623903",
        "wasm_sha256": "e1bd7ea6ada8b15b2770ac387f8fc2256f20d7f87c4bc34e7559b239d8875ba1"
      },
      "0:scalar_ordinary_recursion_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1836,
            "sides": [
              {
                "calibrated_total_ns": 40795644.599999994,
                "ns_per_operation": 22219.85,
                "observed_batch": 1836,
                "requested_batch": 1836,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 37894757.256000005,
                "ns_per_operation": 20639.846,
                "observed_batch": 1836,
                "requested_batch": 1836,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 40795644.599999994,
            "ns_per_operation": 22219.85,
            "observed_batch": 1836,
            "requested_batch": 1836,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 37894757.256000005,
            "ns_per_operation": 20639.846,
            "observed_batch": 1836,
            "requested_batch": 1836,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 36454101.3,
            "ns_per_operation": 19855.175,
            "observed_batch": 1836,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 38473199.16,
            "ns_per_operation": 21069.66,
            "observed_batch": 1826,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1836,
        "source_sha256": "aca6a66637fa36b72f3c528dd5e3e44039b573f363b5b4fb77437e9278623903",
        "wasm_sha256": "e1bd7ea6ada8b15b2770ac387f8fc2256f20d7f87c4bc34e7559b239d8875ba1"
      },
      "0:scalar_ordinary_recursion_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 906,
            "sides": [
              {
                "calibrated_total_ns": 33350705.297999997,
                "ns_per_operation": 36810.933,
                "observed_batch": 906,
                "requested_batch": 906,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 35816554.626,
                "ns_per_operation": 39532.621,
                "observed_batch": 906,
                "requested_batch": 906,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 33350705.297999997,
            "ns_per_operation": 36810.933,
            "observed_batch": 906,
            "requested_batch": 906,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 35816554.626,
            "ns_per_operation": 39532.621,
            "observed_batch": 906,
            "requested_batch": 906,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 31267582.985999998,
            "ns_per_operation": 34511.681,
            "observed_batch": 906,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 51474779.095000006,
            "ns_per_operation": 65572.967,
            "observed_batch": 785,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 906,
        "source_sha256": "aca6a66637fa36b72f3c528dd5e3e44039b573f363b5b4fb77437e9278623903",
        "wasm_sha256": "e1bd7ea6ada8b15b2770ac387f8fc2256f20d7f87c4bc34e7559b239d8875ba1"
      },
      "0:scalar_table_mutation_n0": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 23641,
            "sides": [
              {
                "calibrated_total_ns": 28940532.047,
                "ns_per_operation": 1224.167,
                "observed_batch": 23641,
                "requested_batch": 23641,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 19162425.319,
                "ns_per_operation": 810.559,
                "observed_batch": 23641,
                "requested_batch": 23641,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 27142,
            "sides": [
              {
                "calibrated_total_ns": 23622388.292,
                "ns_per_operation": 870.326,
                "observed_batch": 27142,
                "requested_batch": 27142,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 23984598.282,
                "ns_per_operation": 883.671,
                "observed_batch": 27142,
                "requested_batch": 27142,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 23622388.292,
            "ns_per_operation": 870.326,
            "observed_batch": 27142,
            "requested_batch": 27142,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 23984598.282,
            "ns_per_operation": 883.671,
            "observed_batch": 27142,
            "requested_batch": 27142,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 20254758.144,
            "ns_per_operation": 909.427,
            "observed_batch": 22272,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 19707492.215,
            "ns_per_operation": 833.615,
            "observed_batch": 23641,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 27142,
        "source_sha256": "9b61bd9cbca804bf639085769fd766a2cf8a69d516fc7efb9e4edfe4ff6d18c9",
        "wasm_sha256": "458f2b111e48d2efca98083b8c56860c43e82be8752813d1e2814d23c28d4f43"
      },
      "0:scalar_table_mutation_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 13343,
            "sides": [
              {
                "calibrated_total_ns": 43363709.246,
                "ns_per_operation": 3249.922,
                "observed_batch": 13343,
                "requested_batch": 13343,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 31289201.569999997,
                "ns_per_operation": 2344.99,
                "observed_batch": 13343,
                "requested_batch": 13343,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 43363709.246,
            "ns_per_operation": 3249.922,
            "observed_batch": 13343,
            "requested_batch": 13343,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 31289201.569999997,
            "ns_per_operation": 2344.99,
            "observed_batch": 13343,
            "requested_batch": 13343,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 31372293.168,
            "ns_per_operation": 2381.018,
            "observed_batch": 13176,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 43600427.409,
            "ns_per_operation": 3267.663,
            "observed_batch": 13343,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 13343,
        "source_sha256": "9b61bd9cbca804bf639085769fd766a2cf8a69d516fc7efb9e4edfe4ff6d18c9",
        "wasm_sha256": "458f2b111e48d2efca98083b8c56860c43e82be8752813d1e2814d23c28d4f43"
      },
      "0:scalar_table_mutation_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 2,
            "sides": [
              {
                "calibrated_total_ns": 23924206.0,
                "ns_per_operation": 11962103.0,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 31452708.0,
                "ns_per_operation": 15726354.0,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 23924206.0,
            "ns_per_operation": 11962103.0,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 31452708.0,
            "ns_per_operation": 15726354.0,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 27172745.0,
            "ns_per_operation": 13586372.5,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 25107591.0,
            "ns_per_operation": 12553795.5,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 2,
        "source_sha256": "9b61bd9cbca804bf639085769fd766a2cf8a69d516fc7efb9e4edfe4ff6d18c9",
        "wasm_sha256": "458f2b111e48d2efca98083b8c56860c43e82be8752813d1e2814d23c28d4f43"
      },
      "0:scalar_table_mutation_n1024": {
        "attempt_count": 3,
        "attempts": [
          {
            "batch": 15,
            "sides": [
              {
                "calibrated_total_ns": 16885753.995,
                "ns_per_operation": 1125716.933,
                "observed_batch": 15,
                "requested_batch": 15,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 26513820.0,
                "ns_per_operation": 1767588.0,
                "observed_batch": 15,
                "requested_batch": 15,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 20,
            "sides": [
              {
                "calibrated_total_ns": 22459119.0,
                "ns_per_operation": 1122955.95,
                "observed_batch": 20,
                "requested_batch": 20,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 19573687.0,
                "ns_per_operation": 978684.35,
                "observed_batch": 20,
                "requested_batch": 20,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 23,
            "sides": [
              {
                "calibrated_total_ns": 25318571.005000003,
                "ns_per_operation": 1100807.435,
                "observed_batch": 23,
                "requested_batch": 23,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 26570164.006,
                "ns_per_operation": 1155224.522,
                "observed_batch": 23,
                "requested_batch": 23,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 25318571.005000003,
            "ns_per_operation": 1100807.435,
            "observed_batch": 23,
            "requested_batch": 23,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 26570164.006,
            "ns_per_operation": 1155224.522,
            "observed_batch": 23,
            "requested_batch": 23,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 17490871.994999997,
            "ns_per_operation": 1166058.133,
            "observed_batch": 15,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 112554242.0,
            "ns_per_operation": 11255424.2,
            "observed_batch": 10,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 23,
        "source_sha256": "9b61bd9cbca804bf639085769fd766a2cf8a69d516fc7efb9e4edfe4ff6d18c9",
        "wasm_sha256": "458f2b111e48d2efca98083b8c56860c43e82be8752813d1e2814d23c28d4f43"
      },
      "0:scalar_table_mutation_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1751,
            "sides": [
              {
                "calibrated_total_ns": 35372448.284,
                "ns_per_operation": 20201.284,
                "observed_batch": 1751,
                "requested_batch": 1751,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 38563388.386999995,
                "ns_per_operation": 22023.637,
                "observed_batch": 1751,
                "requested_batch": 1751,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 35372448.284,
            "ns_per_operation": 20201.284,
            "observed_batch": 1751,
            "requested_batch": 1751,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 38563388.386999995,
            "ns_per_operation": 22023.637,
            "observed_batch": 1751,
            "requested_batch": 1751,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 32996836.71,
            "ns_per_operation": 20393.595,
            "observed_batch": 1618,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 35968390.628,
            "ns_per_operation": 20541.628,
            "observed_batch": 1751,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1751,
        "source_sha256": "9b61bd9cbca804bf639085769fd766a2cf8a69d516fc7efb9e4edfe4ff6d18c9",
        "wasm_sha256": "458f2b111e48d2efca98083b8c56860c43e82be8752813d1e2814d23c28d4f43"
      },
      "0:scalar_table_mutation_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 949,
            "sides": [
              {
                "calibrated_total_ns": 60487676.118999995,
                "ns_per_operation": 63738.331,
                "observed_batch": 949,
                "requested_batch": 949,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 99850152.922,
                "ns_per_operation": 105216.178,
                "observed_batch": 949,
                "requested_batch": 949,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 60487676.118999995,
            "ns_per_operation": 63738.331,
            "observed_batch": 949,
            "requested_batch": 949,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 99850152.922,
            "ns_per_operation": 105216.178,
            "observed_batch": 949,
            "requested_batch": 949,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 87935347.692,
            "ns_per_operation": 93349.626,
            "observed_batch": 942,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 82603993.953,
            "ns_per_operation": 87043.197,
            "observed_batch": 949,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 949,
        "source_sha256": "9b61bd9cbca804bf639085769fd766a2cf8a69d516fc7efb9e4edfe4ff6d18c9",
        "wasm_sha256": "458f2b111e48d2efca98083b8c56860c43e82be8752813d1e2814d23c28d4f43"
      },
      "0:scalar_tail_calls_n0": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 9260,
            "sides": [
              {
                "calibrated_total_ns": 7507757.98,
                "ns_per_operation": 810.773,
                "observed_batch": 9260,
                "requested_batch": 9260,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 10857757.440000001,
                "ns_per_operation": 1172.544,
                "observed_batch": 9260,
                "requested_batch": 9260,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 27135,
            "sides": [
              {
                "calibrated_total_ns": 24019250.76,
                "ns_per_operation": 885.176,
                "observed_batch": 27135,
                "requested_batch": 27135,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 20013690.599999998,
                "ns_per_operation": 737.56,
                "observed_batch": 27135,
                "requested_batch": 27135,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 24019250.76,
            "ns_per_operation": 885.176,
            "observed_batch": 27135,
            "requested_batch": 27135,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 20013690.599999998,
            "ns_per_operation": 737.56,
            "observed_batch": 27135,
            "requested_batch": 27135,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 7931736.34,
            "ns_per_operation": 856.559,
            "observed_batch": 9260,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 6672793.050000001,
            "ns_per_operation": 729.267,
            "observed_batch": 9150,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 27135,
        "source_sha256": "9953f19384c51b579921e6f789b13a8b57c86ba86d760b77846e3e33825ece02",
        "wasm_sha256": "3e23be294e7a9eda62c89dbc366b6a6cab90edfbda532da27a7e4fc6d0aa4a56"
      },
      "0:scalar_tail_calls_n1": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 7016,
            "sides": [
              {
                "calibrated_total_ns": 31675212.376000002,
                "ns_per_operation": 4514.711,
                "observed_batch": 7016,
                "requested_batch": 7016,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 14804145.879999999,
                "ns_per_operation": 2110.055,
                "observed_batch": 7016,
                "requested_batch": 7016,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 10427,
            "sides": [
              {
                "calibrated_total_ns": 24651940.907,
                "ns_per_operation": 2364.241,
                "observed_batch": 10427,
                "requested_batch": 10427,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 22115781.697,
                "ns_per_operation": 2121.011,
                "observed_batch": 10427,
                "requested_batch": 10427,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 24651940.907,
            "ns_per_operation": 2364.241,
            "observed_batch": 10427,
            "requested_batch": 10427,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22115781.697,
            "ns_per_operation": 2121.011,
            "observed_batch": 10427,
            "requested_batch": 10427,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 12613343.752,
            "ns_per_operation": 1797.797,
            "observed_batch": 7016,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 15947325.475000001,
            "ns_per_operation": 2281.775,
            "observed_batch": 6989,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 10427,
        "source_sha256": "9953f19384c51b579921e6f789b13a8b57c86ba86d760b77846e3e33825ece02",
        "wasm_sha256": "3e23be294e7a9eda62c89dbc366b6a6cab90edfbda532da27a7e4fc6d0aa4a56"
      },
      "0:scalar_tail_calls_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 4,
            "sides": [
              {
                "calibrated_total_ns": 47405817.0,
                "ns_per_operation": 11851454.25,
                "observed_batch": 4,
                "requested_batch": 4,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 37693725.0,
                "ns_per_operation": 9423431.25,
                "observed_batch": 4,
                "requested_batch": 4,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 47405817.0,
            "ns_per_operation": 11851454.25,
            "observed_batch": 4,
            "requested_batch": 4,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 37693725.0,
            "ns_per_operation": 9423431.25,
            "observed_batch": 4,
            "requested_batch": 4,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 24449217.0,
            "ns_per_operation": 6112304.25,
            "observed_batch": 4,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 8223996.0,
            "ns_per_operation": 8223996.0,
            "observed_batch": 1,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 4,
        "source_sha256": "9953f19384c51b579921e6f789b13a8b57c86ba86d760b77846e3e33825ece02",
        "wasm_sha256": "3e23be294e7a9eda62c89dbc366b6a6cab90edfbda532da27a7e4fc6d0aa4a56"
      },
      "0:scalar_tail_calls_n1024": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 32,
            "sides": [
              {
                "calibrated_total_ns": 28648124.992,
                "ns_per_operation": 895253.906,
                "observed_batch": 32,
                "requested_batch": 32,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 24208676.0,
                "ns_per_operation": 756521.125,
                "observed_batch": 32,
                "requested_batch": 32,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 28648124.992,
            "ns_per_operation": 895253.906,
            "observed_batch": 32,
            "requested_batch": 32,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 24208676.0,
            "ns_per_operation": 756521.125,
            "observed_batch": 32,
            "requested_batch": 32,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 26225452.992,
            "ns_per_operation": 819545.406,
            "observed_batch": 32,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22242163.991,
            "ns_per_operation": 717489.161,
            "observed_batch": 31,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 32,
        "source_sha256": "9953f19384c51b579921e6f789b13a8b57c86ba86d760b77846e3e33825ece02",
        "wasm_sha256": "3e23be294e7a9eda62c89dbc366b6a6cab90edfbda532da27a7e4fc6d0aa4a56"
      },
      "0:scalar_tail_calls_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1926,
            "sides": [
              {
                "calibrated_total_ns": 27470705.562,
                "ns_per_operation": 14263.087,
                "observed_batch": 1926,
                "requested_batch": 1926,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 26797545.450000003,
                "ns_per_operation": 13913.575,
                "observed_batch": 1926,
                "requested_batch": 1926,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 27470705.562,
            "ns_per_operation": 14263.087,
            "observed_batch": 1926,
            "requested_batch": 1926,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 26797545.450000003,
            "ns_per_operation": 13913.575,
            "observed_batch": 1926,
            "requested_batch": 1926,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 27086524.416,
            "ns_per_operation": 14063.616,
            "observed_batch": 1926,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 33033381.746,
            "ns_per_operation": 17267.842,
            "observed_batch": 1913,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1926,
        "source_sha256": "9953f19384c51b579921e6f789b13a8b57c86ba86d760b77846e3e33825ece02",
        "wasm_sha256": "3e23be294e7a9eda62c89dbc366b6a6cab90edfbda532da27a7e4fc6d0aa4a56"
      },
      "0:scalar_tail_calls_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1036,
            "sides": [
              {
                "calibrated_total_ns": 27987192.94,
                "ns_per_operation": 27014.665,
                "observed_batch": 1036,
                "requested_batch": 1036,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 27399133.44,
                "ns_per_operation": 26447.04,
                "observed_batch": 1036,
                "requested_batch": 1036,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 27987192.94,
            "ns_per_operation": 27014.665,
            "observed_batch": 1036,
            "requested_batch": 1036,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27399133.44,
            "ns_per_operation": 26447.04,
            "observed_batch": 1036,
            "requested_batch": 1036,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 11664667.17,
            "ns_per_operation": 29016.585,
            "observed_batch": 402,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 26566083.768,
            "ns_per_operation": 25642.938,
            "observed_batch": 1036,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1036,
        "source_sha256": "9953f19384c51b579921e6f789b13a8b57c86ba86d760b77846e3e33825ece02",
        "wasm_sha256": "3e23be294e7a9eda62c89dbc366b6a6cab90edfbda532da27a7e4fc6d0aa4a56"
      },
      "0:scalar_try_table_normal_n0": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 27740,
            "sides": [
              {
                "calibrated_total_ns": 24088112.22,
                "ns_per_operation": 868.353,
                "observed_batch": 27740,
                "requested_batch": 27740,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 27347728.66,
                "ns_per_operation": 985.859,
                "observed_batch": 27740,
                "requested_batch": 27740,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 24088112.22,
            "ns_per_operation": 868.353,
            "observed_batch": 27740,
            "requested_batch": 27740,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27347728.66,
            "ns_per_operation": 985.859,
            "observed_batch": 27740,
            "requested_batch": 27740,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 17156263.535,
            "ns_per_operation": 703.385,
            "observed_batch": 24391,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 17420720.0,
            "ns_per_operation": 628.0,
            "observed_batch": 27740,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 27740,
        "source_sha256": "ab92800dd4e4d4053498c317effadedf52568270ab7b476eef74b09638de8964",
        "wasm_sha256": "97068a6310a3bc535369e2b6277b801875c14ca206a707fcb9bca9036f41d0df"
      },
      "0:scalar_try_table_normal_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 15338,
            "sides": [
              {
                "calibrated_total_ns": 93875676.832,
                "ns_per_operation": 6120.464,
                "observed_batch": 15338,
                "requested_batch": 15338,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 88709547.01,
                "ns_per_operation": 5783.645,
                "observed_batch": 15338,
                "requested_batch": 15338,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 93875676.832,
            "ns_per_operation": 6120.464,
            "observed_batch": 15338,
            "requested_batch": 15338,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 88709547.01,
            "ns_per_operation": 5783.645,
            "observed_batch": 15338,
            "requested_batch": 15338,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 94643175.014,
            "ns_per_operation": 6170.503,
            "observed_batch": 15338,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 54753362.010000005,
            "ns_per_operation": 5962.47,
            "observed_batch": 9183,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 15338,
        "source_sha256": "ab92800dd4e4d4053498c317effadedf52568270ab7b476eef74b09638de8964",
        "wasm_sha256": "97068a6310a3bc535369e2b6277b801875c14ca206a707fcb9bca9036f41d0df"
      },
      "0:scalar_try_table_normal_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 3,
            "sides": [
              {
                "calibrated_total_ns": 28952949.0,
                "ns_per_operation": 9650983.0,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 30452955.999,
                "ns_per_operation": 10150985.333,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 28952949.0,
            "ns_per_operation": 9650983.0,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 30452955.999,
            "ns_per_operation": 10150985.333,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 23486721.0,
            "ns_per_operation": 7828907.0,
            "observed_batch": 3,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 24016488.998999998,
            "ns_per_operation": 8005496.333,
            "observed_batch": 3,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 3,
        "source_sha256": "ab92800dd4e4d4053498c317effadedf52568270ab7b476eef74b09638de8964",
        "wasm_sha256": "97068a6310a3bc535369e2b6277b801875c14ca206a707fcb9bca9036f41d0df"
      },
      "0:scalar_try_table_normal_n1024": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 23,
            "sides": [
              {
                "calibrated_total_ns": 21817937.011,
                "ns_per_operation": 948605.957,
                "observed_batch": 23,
                "requested_batch": 23,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 19692656.994,
                "ns_per_operation": 856202.478,
                "observed_batch": 23,
                "requested_batch": 23,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 26,
            "sides": [
              {
                "calibrated_total_ns": 25727199.004,
                "ns_per_operation": 989507.654,
                "observed_batch": 26,
                "requested_batch": 26,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 24096306.0,
                "ns_per_operation": 926781.0,
                "observed_batch": 26,
                "requested_batch": 26,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 25727199.004,
            "ns_per_operation": 989507.654,
            "observed_batch": 26,
            "requested_batch": 26,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 24096306.0,
            "ns_per_operation": 926781.0,
            "observed_batch": 26,
            "requested_batch": 26,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 14704950.0,
            "ns_per_operation": 919059.375,
            "observed_batch": 16,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 19858498.011,
            "ns_per_operation": 863412.957,
            "observed_batch": 23,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 26,
        "source_sha256": "ab92800dd4e4d4053498c317effadedf52568270ab7b476eef74b09638de8964",
        "wasm_sha256": "97068a6310a3bc535369e2b6277b801875c14ca206a707fcb9bca9036f41d0df"
      },
      "0:scalar_try_table_normal_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 2065,
            "sides": [
              {
                "calibrated_total_ns": 28300310.815,
                "ns_per_operation": 13704.751,
                "observed_batch": 2065,
                "requested_batch": 2065,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 31200146.950000003,
                "ns_per_operation": 15109.03,
                "observed_batch": 2065,
                "requested_batch": 2065,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 28300310.815,
            "ns_per_operation": 13704.751,
            "observed_batch": 2065,
            "requested_batch": 2065,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 31200146.950000003,
            "ns_per_operation": 15109.03,
            "observed_batch": 2065,
            "requested_batch": 2065,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 30104604.565,
            "ns_per_operation": 14578.501,
            "observed_batch": 2065,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 28661470.953,
            "ns_per_operation": 14453.591,
            "observed_batch": 1983,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 2065,
        "source_sha256": "ab92800dd4e4d4053498c317effadedf52568270ab7b476eef74b09638de8964",
        "wasm_sha256": "97068a6310a3bc535369e2b6277b801875c14ca206a707fcb9bca9036f41d0df"
      },
      "0:scalar_try_table_normal_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1138,
            "sides": [
              {
                "calibrated_total_ns": 35095912.034,
                "ns_per_operation": 30839.993,
                "observed_batch": 1138,
                "requested_batch": 1138,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 36861489.446,
                "ns_per_operation": 32391.467,
                "observed_batch": 1138,
                "requested_batch": 1138,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 35095912.034,
            "ns_per_operation": 30839.993,
            "observed_batch": 1138,
            "requested_batch": 1138,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 36861489.446,
            "ns_per_operation": 32391.467,
            "observed_batch": 1138,
            "requested_batch": 1138,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 39193014.742,
            "ns_per_operation": 34440.259,
            "observed_batch": 1138,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 33172566.303999998,
            "ns_per_operation": 33440.087,
            "observed_batch": 992,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1138,
        "source_sha256": "ab92800dd4e4d4053498c317effadedf52568270ab7b476eef74b09638de8964",
        "wasm_sha256": "97068a6310a3bc535369e2b6277b801875c14ca206a707fcb9bca9036f41d0df"
      },
      "1:scalar_accumulator_n0": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 8334,
            "sides": [
              {
                "calibrated_total_ns": 5898813.534,
                "ns_per_operation": 707.801,
                "observed_batch": 8334,
                "requested_batch": 8334,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 8411289.516,
                "ns_per_operation": 1009.274,
                "observed_batch": 8334,
                "requested_batch": 8334,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 31083,
            "sides": [
              {
                "calibrated_total_ns": 21639425.106,
                "ns_per_operation": 696.182,
                "observed_batch": 31083,
                "requested_batch": 31083,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 28386549.75,
                "ns_per_operation": 913.25,
                "observed_batch": 31083,
                "requested_batch": 31083,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 21639425.106,
            "ns_per_operation": 696.182,
            "observed_batch": 31083,
            "requested_batch": 31083,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 28386549.75,
            "ns_per_operation": 913.25,
            "observed_batch": 31083,
            "requested_batch": 31083,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 5329092.960000001,
            "ns_per_operation": 639.44,
            "observed_batch": 8334,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 167752.08800000002,
            "ns_per_operation": 855.878,
            "observed_batch": 196,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 31083,
        "source_sha256": "61aaec7a86d481378f31f3e27b0c1413684065841fa5909297181fcad9bef10d",
        "wasm_sha256": "ef61bf7afc44af7b71ef78eadcba48c0d85949d7d9096df8fbbbab53c05036b1"
      },
      "1:scalar_accumulator_n1": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 7634,
            "sides": [
              {
                "calibrated_total_ns": 11945660.298,
                "ns_per_operation": 1564.797,
                "observed_batch": 7634,
                "requested_batch": 7634,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 11341459.734000001,
                "ns_per_operation": 1485.651,
                "observed_batch": 7634,
                "requested_batch": 7634,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 14809,
            "sides": [
              {
                "calibrated_total_ns": 31113175.876,
                "ns_per_operation": 2100.964,
                "observed_batch": 14809,
                "requested_batch": 14809,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 23889952.845,
                "ns_per_operation": 1613.205,
                "observed_batch": 14809,
                "requested_batch": 14809,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 31113175.876,
            "ns_per_operation": 2100.964,
            "observed_batch": 14809,
            "requested_batch": 14809,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 23889952.845,
            "ns_per_operation": 1613.205,
            "observed_batch": 14809,
            "requested_batch": 14809,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 13681662.434,
            "ns_per_operation": 1792.201,
            "observed_batch": 7634,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 9321625.698,
            "ns_per_operation": 1770.153,
            "observed_batch": 5266,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 14809,
        "source_sha256": "61aaec7a86d481378f31f3e27b0c1413684065841fa5909297181fcad9bef10d",
        "wasm_sha256": "ef61bf7afc44af7b71ef78eadcba48c0d85949d7d9096df8fbbbab53c05036b1"
      },
      "1:scalar_accumulator_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 4,
            "sides": [
              {
                "calibrated_total_ns": 30406234.0,
                "ns_per_operation": 7601558.5,
                "observed_batch": 4,
                "requested_batch": 4,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 25370962.0,
                "ns_per_operation": 6342740.5,
                "observed_batch": 4,
                "requested_batch": 4,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 30406234.0,
            "ns_per_operation": 7601558.5,
            "observed_batch": 4,
            "requested_batch": 4,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 25370962.0,
            "ns_per_operation": 6342740.5,
            "observed_batch": 4,
            "requested_batch": 4,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 18710181.998999998,
            "ns_per_operation": 6236727.333,
            "observed_batch": 3,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27530835.0,
            "ns_per_operation": 6882708.75,
            "observed_batch": 4,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 4,
        "source_sha256": "61aaec7a86d481378f31f3e27b0c1413684065841fa5909297181fcad9bef10d",
        "wasm_sha256": "ef61bf7afc44af7b71ef78eadcba48c0d85949d7d9096df8fbbbab53c05036b1"
      },
      "1:scalar_accumulator_n1024": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 29,
            "sides": [
              {
                "calibrated_total_ns": 26666181.993,
                "ns_per_operation": 919523.517,
                "observed_batch": 29,
                "requested_batch": 29,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 18213070.998999998,
                "ns_per_operation": 628036.931,
                "observed_batch": 29,
                "requested_batch": 29,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 36,
            "sides": [
              {
                "calibrated_total_ns": 24651632.987999998,
                "ns_per_operation": 684767.583,
                "observed_batch": 36,
                "requested_batch": 36,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 24182614.008,
                "ns_per_operation": 671739.278,
                "observed_batch": 36,
                "requested_batch": 36,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 24651632.987999998,
            "ns_per_operation": 684767.583,
            "observed_batch": 36,
            "requested_batch": 36,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 24182614.008,
            "ns_per_operation": 671739.278,
            "observed_batch": 36,
            "requested_batch": 36,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 22555669.996000003,
            "ns_per_operation": 777781.724,
            "observed_batch": 29,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 15136429.0,
            "ns_per_operation": 756821.45,
            "observed_batch": 20,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 36,
        "source_sha256": "61aaec7a86d481378f31f3e27b0c1413684065841fa5909297181fcad9bef10d",
        "wasm_sha256": "ef61bf7afc44af7b71ef78eadcba48c0d85949d7d9096df8fbbbab53c05036b1"
      },
      "1:scalar_accumulator_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 2250,
            "sides": [
              {
                "calibrated_total_ns": 27284058.0,
                "ns_per_operation": 12126.248,
                "observed_batch": 2250,
                "requested_batch": 2250,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 32817224.250000004,
                "ns_per_operation": 14585.433,
                "observed_batch": 2250,
                "requested_batch": 2250,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 27284058.0,
            "ns_per_operation": 12126.248,
            "observed_batch": 2250,
            "requested_batch": 2250,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 32817224.250000004,
            "ns_per_operation": 14585.433,
            "observed_batch": 2250,
            "requested_batch": 2250,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 27763004.25,
            "ns_per_operation": 12339.113,
            "observed_batch": 2250,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 20448814.408,
            "ns_per_operation": 11765.716,
            "observed_batch": 1738,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 2250,
        "source_sha256": "61aaec7a86d481378f31f3e27b0c1413684065841fa5909297181fcad9bef10d",
        "wasm_sha256": "ef61bf7afc44af7b71ef78eadcba48c0d85949d7d9096df8fbbbab53c05036b1"
      },
      "1:scalar_accumulator_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1332,
            "sides": [
              {
                "calibrated_total_ns": 30201453.648,
                "ns_per_operation": 22673.764,
                "observed_batch": 1332,
                "requested_batch": 1332,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 29225033.712,
                "ns_per_operation": 21940.716,
                "observed_batch": 1332,
                "requested_batch": 1332,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 30201453.648,
            "ns_per_operation": 22673.764,
            "observed_batch": 1332,
            "requested_batch": 1332,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 29225033.712,
            "ns_per_operation": 21940.716,
            "observed_batch": 1332,
            "requested_batch": 1332,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 28357448.832,
            "ns_per_operation": 21289.376,
            "observed_batch": 1332,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 10706794.024999999,
            "ns_per_operation": 22540.619,
            "observed_batch": 475,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1332,
        "source_sha256": "61aaec7a86d481378f31f3e27b0c1413684065841fa5909297181fcad9bef10d",
        "wasm_sha256": "ef61bf7afc44af7b71ef78eadcba48c0d85949d7d9096df8fbbbab53c05036b1"
      },
      "1:scalar_br_table_n0": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 31646,
            "sides": [
              {
                "calibrated_total_ns": 26028866.645999998,
                "ns_per_operation": 822.501,
                "observed_batch": 31646,
                "requested_batch": 31646,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 28051394.152000003,
                "ns_per_operation": 886.412,
                "observed_batch": 31646,
                "requested_batch": 31646,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 26028866.645999998,
            "ns_per_operation": 822.501,
            "observed_batch": 31646,
            "requested_batch": 31646,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 28051394.152000003,
            "ns_per_operation": 886.412,
            "observed_batch": 31646,
            "requested_batch": 31646,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 5064531.721,
            "ns_per_operation": 846.487,
            "observed_batch": 5983,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 25581328.914,
            "ns_per_operation": 808.359,
            "observed_batch": 31646,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 31646,
        "source_sha256": "637b96cd8926ffadf728af4949b43fa6b62fffb61968c14c716e0c203a1e5e6d",
        "wasm_sha256": "d43d5dd71d5ca1c3dae820aa85c13a2d69415e028b8d48097fed4558f743163a"
      },
      "1:scalar_br_table_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 14155,
            "sides": [
              {
                "calibrated_total_ns": 28531681.455000002,
                "ns_per_operation": 2015.661,
                "observed_batch": 14155,
                "requested_batch": 14155,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 27540123.705000002,
                "ns_per_operation": 1945.611,
                "observed_batch": 14155,
                "requested_batch": 14155,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 28531681.455000002,
            "ns_per_operation": 2015.661,
            "observed_batch": 14155,
            "requested_batch": 14155,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27540123.705000002,
            "ns_per_operation": 1945.611,
            "observed_batch": 14155,
            "requested_batch": 14155,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 30027553.544999998,
            "ns_per_operation": 2121.339,
            "observed_batch": 14155,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 26041540.823,
            "ns_per_operation": 2031.163,
            "observed_batch": 12821,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 14155,
        "source_sha256": "637b96cd8926ffadf728af4949b43fa6b62fffb61968c14c716e0c203a1e5e6d",
        "wasm_sha256": "d43d5dd71d5ca1c3dae820aa85c13a2d69415e028b8d48097fed4558f743163a"
      },
      "1:scalar_br_table_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 2,
            "sides": [
              {
                "calibrated_total_ns": 27732784.0,
                "ns_per_operation": 13866392.0,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 32535619.0,
                "ns_per_operation": 16267809.5,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 27732784.0,
            "ns_per_operation": 13866392.0,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 32535619.0,
            "ns_per_operation": 16267809.5,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 27347243.0,
            "ns_per_operation": 13673621.5,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 29707144.0,
            "ns_per_operation": 14853572.0,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 2,
        "source_sha256": "637b96cd8926ffadf728af4949b43fa6b62fffb61968c14c716e0c203a1e5e6d",
        "wasm_sha256": "d43d5dd71d5ca1c3dae820aa85c13a2d69415e028b8d48097fed4558f743163a"
      },
      "1:scalar_br_table_n1024": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 15,
            "sides": [
              {
                "calibrated_total_ns": 26487613.005000003,
                "ns_per_operation": 1765840.867,
                "observed_batch": 15,
                "requested_batch": 15,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 24060640.005,
                "ns_per_operation": 1604042.667,
                "observed_batch": 15,
                "requested_batch": 15,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 26487613.005000003,
            "ns_per_operation": 1765840.867,
            "observed_batch": 15,
            "requested_batch": 15,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 24060640.005,
            "ns_per_operation": 1604042.667,
            "observed_batch": 15,
            "requested_batch": 15,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 13313759.0,
            "ns_per_operation": 1664219.875,
            "observed_batch": 8,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 24635668.005000003,
            "ns_per_operation": 1642377.867,
            "observed_batch": 15,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 15,
        "source_sha256": "637b96cd8926ffadf728af4949b43fa6b62fffb61968c14c716e0c203a1e5e6d",
        "wasm_sha256": "d43d5dd71d5ca1c3dae820aa85c13a2d69415e028b8d48097fed4558f743163a"
      },
      "1:scalar_br_table_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1339,
            "sides": [
              {
                "calibrated_total_ns": 41998115.276,
                "ns_per_operation": 31365.284,
                "observed_batch": 1339,
                "requested_batch": 1339,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 36501925.993,
                "ns_per_operation": 27260.587,
                "observed_batch": 1339,
                "requested_batch": 1339,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 41998115.276,
            "ns_per_operation": 31365.284,
            "observed_batch": 1339,
            "requested_batch": 1339,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 36501925.993,
            "ns_per_operation": 27260.587,
            "observed_batch": 1339,
            "requested_batch": 1339,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 41121305.94,
            "ns_per_operation": 30710.46,
            "observed_batch": 1339,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 9457982.905,
            "ns_per_operation": 32060.959,
            "observed_batch": 295,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1339,
        "source_sha256": "637b96cd8926ffadf728af4949b43fa6b62fffb61968c14c716e0c203a1e5e6d",
        "wasm_sha256": "d43d5dd71d5ca1c3dae820aa85c13a2d69415e028b8d48097fed4558f743163a"
      },
      "1:scalar_br_table_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 761,
            "sides": [
              {
                "calibrated_total_ns": 57052701.938999996,
                "ns_per_operation": 74970.699,
                "observed_batch": 761,
                "requested_batch": 761,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 32033673.262999997,
                "ns_per_operation": 42094.183,
                "observed_batch": 761,
                "requested_batch": 761,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 57052701.938999996,
            "ns_per_operation": 74970.699,
            "observed_batch": 761,
            "requested_batch": 761,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 32033673.262999997,
            "ns_per_operation": 42094.183,
            "observed_batch": 761,
            "requested_batch": 761,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 33851658.886,
            "ns_per_operation": 44483.126,
            "observed_batch": 761,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 50819768.046,
            "ns_per_operation": 68031.818,
            "observed_batch": 747,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 761,
        "source_sha256": "637b96cd8926ffadf728af4949b43fa6b62fffb61968c14c716e0c203a1e5e6d",
        "wasm_sha256": "d43d5dd71d5ca1c3dae820aa85c13a2d69415e028b8d48097fed4558f743163a"
      },
      "1:scalar_branch_entropy_n0": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 24969,
            "sides": [
              {
                "calibrated_total_ns": 19800242.217,
                "ns_per_operation": 792.993,
                "observed_batch": 24969,
                "requested_batch": 24969,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 19367629.323,
                "ns_per_operation": 775.667,
                "observed_batch": 24969,
                "requested_batch": 24969,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 28363,
            "sides": [
              {
                "calibrated_total_ns": 23045504.759999998,
                "ns_per_operation": 812.52,
                "observed_batch": 28363,
                "requested_batch": 28363,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 21000447.371,
                "ns_per_operation": 740.417,
                "observed_batch": 28363,
                "requested_batch": 28363,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 23045504.759999998,
            "ns_per_operation": 812.52,
            "observed_batch": 28363,
            "requested_batch": 28363,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 21000447.371,
            "ns_per_operation": 740.417,
            "observed_batch": 28363,
            "requested_batch": 28363,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 18155983.68,
            "ns_per_operation": 741.666,
            "observed_batch": 24480,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 20590935.54,
            "ns_per_operation": 824.66,
            "observed_batch": 24969,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 28363,
        "source_sha256": "6b4861d8bfbbd7e2718e577ec2d479bfff32e94dc47a3b7dda9a1b363baa98b6",
        "wasm_sha256": "e4c87dbab50a2438cb1f749bc6fd6f9c834c81484acd42864735bd1ad1b6dec0"
      },
      "1:scalar_branch_entropy_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 12611,
            "sides": [
              {
                "calibrated_total_ns": 31686764.319,
                "ns_per_operation": 2512.629,
                "observed_batch": 12611,
                "requested_batch": 12611,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 31050425.87,
                "ns_per_operation": 2462.17,
                "observed_batch": 12611,
                "requested_batch": 12611,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 31686764.319,
            "ns_per_operation": 2512.629,
            "observed_batch": 12611,
            "requested_batch": 12611,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 31050425.87,
            "ns_per_operation": 2462.17,
            "observed_batch": 12611,
            "requested_batch": 12611,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 32627484.428000003,
            "ns_per_operation": 2681.858,
            "observed_batch": 12166,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 34843247.175000004,
            "ns_per_operation": 2762.925,
            "observed_batch": 12611,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 12611,
        "source_sha256": "6b4861d8bfbbd7e2718e577ec2d479bfff32e94dc47a3b7dda9a1b363baa98b6",
        "wasm_sha256": "e4c87dbab50a2438cb1f749bc6fd6f9c834c81484acd42864735bd1ad1b6dec0"
      },
      "1:scalar_branch_entropy_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 2,
            "sides": [
              {
                "calibrated_total_ns": 46030663.0,
                "ns_per_operation": 23015331.5,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 25407167.0,
                "ns_per_operation": 12703583.5,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 46030663.0,
            "ns_per_operation": 23015331.5,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 25407167.0,
            "ns_per_operation": 12703583.5,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 26144612.0,
            "ns_per_operation": 13072306.0,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 25286742.0,
            "ns_per_operation": 12643371.0,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 2,
        "source_sha256": "6b4861d8bfbbd7e2718e577ec2d479bfff32e94dc47a3b7dda9a1b363baa98b6",
        "wasm_sha256": "e4c87dbab50a2438cb1f749bc6fd6f9c834c81484acd42864735bd1ad1b6dec0"
      },
      "1:scalar_branch_entropy_n1024": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 10,
            "sides": [
              {
                "calibrated_total_ns": 14137986.0,
                "ns_per_operation": 1413798.6,
                "observed_batch": 10,
                "requested_batch": 10,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 15614073.0,
                "ns_per_operation": 1561407.3,
                "observed_batch": 10,
                "requested_batch": 10,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 16,
            "sides": [
              {
                "calibrated_total_ns": 22251800.0,
                "ns_per_operation": 1390737.5,
                "observed_batch": 16,
                "requested_batch": 16,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 31379943.008,
                "ns_per_operation": 1961246.438,
                "observed_batch": 16,
                "requested_batch": 16,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 22251800.0,
            "ns_per_operation": 1390737.5,
            "observed_batch": 16,
            "requested_batch": 16,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 31379943.008,
            "ns_per_operation": 1961246.438,
            "observed_batch": 16,
            "requested_batch": 16,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 17496680.0,
            "ns_per_operation": 1749668.0,
            "observed_batch": 10,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 15344883.0,
            "ns_per_operation": 1534488.3,
            "observed_batch": 10,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 16,
        "source_sha256": "6b4861d8bfbbd7e2718e577ec2d479bfff32e94dc47a3b7dda9a1b363baa98b6",
        "wasm_sha256": "e4c87dbab50a2438cb1f749bc6fd6f9c834c81484acd42864735bd1ad1b6dec0"
      },
      "1:scalar_branch_entropy_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1425,
            "sides": [
              {
                "calibrated_total_ns": 36553009.875,
                "ns_per_operation": 25651.235,
                "observed_batch": 1425,
                "requested_batch": 1425,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 31902915.674999997,
                "ns_per_operation": 22388.011,
                "observed_batch": 1425,
                "requested_batch": 1425,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 36553009.875,
            "ns_per_operation": 25651.235,
            "observed_batch": 1425,
            "requested_batch": 1425,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 31902915.674999997,
            "ns_per_operation": 22388.011,
            "observed_batch": 1425,
            "requested_batch": 1425,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 31565683.725,
            "ns_per_operation": 22151.357,
            "observed_batch": 1425,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 30589980.794999998,
            "ns_per_operation": 21803.265,
            "observed_batch": 1403,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1425,
        "source_sha256": "6b4861d8bfbbd7e2718e577ec2d479bfff32e94dc47a3b7dda9a1b363baa98b6",
        "wasm_sha256": "e4c87dbab50a2438cb1f749bc6fd6f9c834c81484acd42864735bd1ad1b6dec0"
      },
      "1:scalar_branch_entropy_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 776,
            "sides": [
              {
                "calibrated_total_ns": 40180331.728,
                "ns_per_operation": 51778.778,
                "observed_batch": 776,
                "requested_batch": 776,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 40895748.632,
                "ns_per_operation": 52700.707,
                "observed_batch": 776,
                "requested_batch": 776,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 40180331.728,
            "ns_per_operation": 51778.778,
            "observed_batch": 776,
            "requested_batch": 776,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 40895748.632,
            "ns_per_operation": 52700.707,
            "observed_batch": 776,
            "requested_batch": 776,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 40251458.336,
            "ns_per_operation": 51870.436,
            "observed_batch": 776,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 30411242.316,
            "ns_per_operation": 43320.858,
            "observed_batch": 702,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 776,
        "source_sha256": "6b4861d8bfbbd7e2718e577ec2d479bfff32e94dc47a3b7dda9a1b363baa98b6",
        "wasm_sha256": "e4c87dbab50a2438cb1f749bc6fd6f9c834c81484acd42864735bd1ad1b6dec0"
      },
      "1:scalar_call_ref_n0": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 24039,
            "sides": [
              {
                "calibrated_total_ns": 20649501.0,
                "ns_per_operation": 859.0,
                "observed_batch": 24039,
                "requested_batch": 24039,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 21431321.397,
                "ns_per_operation": 891.523,
                "observed_batch": 24039,
                "requested_batch": 24039,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 20649501.0,
            "ns_per_operation": 859.0,
            "observed_batch": 24039,
            "requested_batch": 24039,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 21431321.397,
            "ns_per_operation": 891.523,
            "observed_batch": 24039,
            "requested_batch": 24039,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 21838133.394,
            "ns_per_operation": 908.446,
            "observed_batch": 24039,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 18449453.913,
            "ns_per_operation": 776.721,
            "observed_batch": 23753,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 24039,
        "source_sha256": "0323154b79000eeeb0a63310541ec91b6c4d91e32a6a3406125a30f2c8e53a12",
        "wasm_sha256": "4f766dc9e29324ea942c892982363dec0ade98565288878a70967ce3b33f322b"
      },
      "1:scalar_call_ref_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 13514,
            "sides": [
              {
                "calibrated_total_ns": 32347245.540000003,
                "ns_per_operation": 2393.61,
                "observed_batch": 13514,
                "requested_batch": 13514,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 27037081.408,
                "ns_per_operation": 2000.672,
                "observed_batch": 13514,
                "requested_batch": 13514,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 32347245.540000003,
            "ns_per_operation": 2393.61,
            "observed_batch": 13514,
            "requested_batch": 13514,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27037081.408,
            "ns_per_operation": 2000.672,
            "observed_batch": 13514,
            "requested_batch": 13514,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 27022472.774,
            "ns_per_operation": 1999.591,
            "observed_batch": 13514,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 28799821.983,
            "ns_per_operation": 2171.441,
            "observed_batch": 13263,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 13514,
        "source_sha256": "0323154b79000eeeb0a63310541ec91b6c4d91e32a6a3406125a30f2c8e53a12",
        "wasm_sha256": "4f766dc9e29324ea942c892982363dec0ade98565288878a70967ce3b33f322b"
      },
      "1:scalar_call_ref_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 2,
            "sides": [
              {
                "calibrated_total_ns": 27145996.0,
                "ns_per_operation": 13572998.0,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 28081974.0,
                "ns_per_operation": 14040987.0,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 27145996.0,
            "ns_per_operation": 13572998.0,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 28081974.0,
            "ns_per_operation": 14040987.0,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 24508084.0,
            "ns_per_operation": 12254042.0,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 26260090.0,
            "ns_per_operation": 13130045.0,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 2,
        "source_sha256": "0323154b79000eeeb0a63310541ec91b6c4d91e32a6a3406125a30f2c8e53a12",
        "wasm_sha256": "4f766dc9e29324ea942c892982363dec0ade98565288878a70967ce3b33f322b"
      },
      "1:scalar_call_ref_n1024": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 14,
            "sides": [
              {
                "calibrated_total_ns": 15273488.006000001,
                "ns_per_operation": 1090963.429,
                "observed_batch": 14,
                "requested_batch": 14,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 16504000.996,
                "ns_per_operation": 1178857.214,
                "observed_batch": 14,
                "requested_batch": 14,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 21,
            "sides": [
              {
                "calibrated_total_ns": 24874795.007999998,
                "ns_per_operation": 1184514.048,
                "observed_batch": 21,
                "requested_batch": 21,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 24474815.001000002,
                "ns_per_operation": 1165467.381,
                "observed_batch": 21,
                "requested_batch": 21,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 24874795.007999998,
            "ns_per_operation": 1184514.048,
            "observed_batch": 21,
            "requested_batch": 21,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 24474815.001000002,
            "ns_per_operation": 1165467.381,
            "observed_batch": 21,
            "requested_batch": 21,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 15137355.001999998,
            "ns_per_operation": 1081239.643,
            "observed_batch": 14,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 15264939.004,
            "ns_per_operation": 1090352.786,
            "observed_batch": 14,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 21,
        "source_sha256": "0323154b79000eeeb0a63310541ec91b6c4d91e32a6a3406125a30f2c8e53a12",
        "wasm_sha256": "4f766dc9e29324ea942c892982363dec0ade98565288878a70967ce3b33f322b"
      },
      "1:scalar_call_ref_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1555,
            "sides": [
              {
                "calibrated_total_ns": 38410062.775,
                "ns_per_operation": 24701.005,
                "observed_batch": 1555,
                "requested_batch": 1555,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 30719863.145,
                "ns_per_operation": 19755.539,
                "observed_batch": 1555,
                "requested_batch": 1555,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 38410062.775,
            "ns_per_operation": 24701.005,
            "observed_batch": 1555,
            "requested_batch": 1555,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 30719863.145,
            "ns_per_operation": 19755.539,
            "observed_batch": 1555,
            "requested_batch": 1555,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 33331700.556,
            "ns_per_operation": 21629.916,
            "observed_batch": 1541,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 30666781.665,
            "ns_per_operation": 19721.403,
            "observed_batch": 1555,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1555,
        "source_sha256": "0323154b79000eeeb0a63310541ec91b6c4d91e32a6a3406125a30f2c8e53a12",
        "wasm_sha256": "4f766dc9e29324ea942c892982363dec0ade98565288878a70967ce3b33f322b"
      },
      "1:scalar_call_ref_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 784,
            "sides": [
              {
                "calibrated_total_ns": 34331360.0,
                "ns_per_operation": 43790.0,
                "observed_batch": 784,
                "requested_batch": 784,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 27498412.704,
                "ns_per_operation": 35074.506,
                "observed_batch": 784,
                "requested_batch": 784,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 34331360.0,
            "ns_per_operation": 43790.0,
            "observed_batch": 784,
            "requested_batch": 784,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27498412.704,
            "ns_per_operation": 35074.506,
            "observed_batch": 784,
            "requested_batch": 784,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 33679749.376,
            "ns_per_operation": 42958.864,
            "observed_batch": 784,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 21787929.072,
            "ns_per_operation": 35085.232,
            "observed_batch": 621,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 784,
        "source_sha256": "0323154b79000eeeb0a63310541ec91b6c4d91e32a6a3406125a30f2c8e53a12",
        "wasm_sha256": "4f766dc9e29324ea942c892982363dec0ade98565288878a70967ce3b33f322b"
      },
      "1:scalar_dependent_n0": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 24361,
            "sides": [
              {
                "calibrated_total_ns": 20901299.502,
                "ns_per_operation": 857.982,
                "observed_batch": 24361,
                "requested_batch": 24361,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 19747830.513,
                "ns_per_operation": 810.633,
                "observed_batch": 24361,
                "requested_batch": 24361,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 27140,
            "sides": [
              {
                "calibrated_total_ns": 24269945.0,
                "ns_per_operation": 894.25,
                "observed_batch": 27140,
                "requested_batch": 27140,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 21550815.54,
                "ns_per_operation": 794.061,
                "observed_batch": 27140,
                "requested_batch": 27140,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 24269945.0,
            "ns_per_operation": 894.25,
            "observed_batch": 27140,
            "requested_batch": 27140,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 21550815.54,
            "ns_per_operation": 794.061,
            "observed_batch": 27140,
            "requested_batch": 27140,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 19768488.641,
            "ns_per_operation": 811.481,
            "observed_batch": 24361,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 23695767.184,
            "ns_per_operation": 1005.848,
            "observed_batch": 23558,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 27140,
        "source_sha256": "0dd5e9778eb09bac9ee432e5fece3f432d034ea9878d2302972142501f0c60c6",
        "wasm_sha256": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c"
      },
      "1:scalar_dependent_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 15445,
            "sides": [
              {
                "calibrated_total_ns": 25240666.905,
                "ns_per_operation": 1634.229,
                "observed_batch": 15445,
                "requested_batch": 15445,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 25690780.540000003,
                "ns_per_operation": 1663.372,
                "observed_batch": 15445,
                "requested_batch": 15445,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 25240666.905,
            "ns_per_operation": 1634.229,
            "observed_batch": 15445,
            "requested_batch": 15445,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 25690780.540000003,
            "ns_per_operation": 1663.372,
            "observed_batch": 15445,
            "requested_batch": 15445,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 34430071.225,
            "ns_per_operation": 2229.205,
            "observed_batch": 15445,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 17427976.823000003,
            "ns_per_operation": 1571.929,
            "observed_batch": 11087,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 15445,
        "source_sha256": "0dd5e9778eb09bac9ee432e5fece3f432d034ea9878d2302972142501f0c60c6",
        "wasm_sha256": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c"
      },
      "1:scalar_dependent_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 3,
            "sides": [
              {
                "calibrated_total_ns": 33661338.0,
                "ns_per_operation": 11220446.0,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 22551674.001000002,
                "ns_per_operation": 7517224.667,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 33661338.0,
            "ns_per_operation": 11220446.0,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22551674.001000002,
            "ns_per_operation": 7517224.667,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 18013666.0,
            "ns_per_operation": 9006833.0,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22883607.998999998,
            "ns_per_operation": 7627869.333,
            "observed_batch": 3,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 3,
        "source_sha256": "0dd5e9778eb09bac9ee432e5fece3f432d034ea9878d2302972142501f0c60c6",
        "wasm_sha256": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c"
      },
      "1:scalar_dependent_n1024": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 28,
            "sides": [
              {
                "calibrated_total_ns": 20413864.996,
                "ns_per_operation": 729066.607,
                "observed_batch": 28,
                "requested_batch": 28,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 18347043.987999998,
                "ns_per_operation": 655251.571,
                "observed_batch": 28,
                "requested_batch": 28,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 34,
            "sides": [
              {
                "calibrated_total_ns": 27537231.006,
                "ns_per_operation": 809918.559,
                "observed_batch": 34,
                "requested_batch": 34,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 37036268.986,
                "ns_per_operation": 1089302.029,
                "observed_batch": 34,
                "requested_batch": 34,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 27537231.006,
            "ns_per_operation": 809918.559,
            "observed_batch": 34,
            "requested_batch": 34,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 37036268.986,
            "ns_per_operation": 1089302.029,
            "observed_batch": 34,
            "requested_batch": 34,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 19615643.992000002,
            "ns_per_operation": 700558.714,
            "observed_batch": 28,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 25572921.998,
            "ns_per_operation": 983573.923,
            "observed_batch": 26,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 34,
        "source_sha256": "0dd5e9778eb09bac9ee432e5fece3f432d034ea9878d2302972142501f0c60c6",
        "wasm_sha256": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c"
      },
      "1:scalar_dependent_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 2350,
            "sides": [
              {
                "calibrated_total_ns": 40361792.85,
                "ns_per_operation": 17175.231,
                "observed_batch": 2350,
                "requested_batch": 2350,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 34017575.4,
                "ns_per_operation": 14475.564,
                "observed_batch": 2350,
                "requested_batch": 2350,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 40361792.85,
            "ns_per_operation": 17175.231,
            "observed_batch": 2350,
            "requested_batch": 2350,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 34017575.4,
            "ns_per_operation": 14475.564,
            "observed_batch": 2350,
            "requested_batch": 2350,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 1452961.045,
            "ns_per_operation": 10020.421,
            "observed_batch": 145,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 39294173.75,
            "ns_per_operation": 16720.925,
            "observed_batch": 2350,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 2350,
        "source_sha256": "0dd5e9778eb09bac9ee432e5fece3f432d034ea9878d2302972142501f0c60c6",
        "wasm_sha256": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c"
      },
      "1:scalar_dependent_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1318,
            "sides": [
              {
                "calibrated_total_ns": 32092633.092,
                "ns_per_operation": 24349.494,
                "observed_batch": 1318,
                "requested_batch": 1318,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 30365793.446,
                "ns_per_operation": 23039.297,
                "observed_batch": 1318,
                "requested_batch": 1318,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 32092633.092,
            "ns_per_operation": 24349.494,
            "observed_batch": 1318,
            "requested_batch": 1318,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 30365793.446,
            "ns_per_operation": 23039.297,
            "observed_batch": 1318,
            "requested_batch": 1318,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 28210320.523,
            "ns_per_operation": 21551.047,
            "observed_batch": 1309,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 30446046.466000002,
            "ns_per_operation": 23100.187,
            "observed_batch": 1318,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1318,
        "source_sha256": "0dd5e9778eb09bac9ee432e5fece3f432d034ea9878d2302972142501f0c60c6",
        "wasm_sha256": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c"
      },
      "1:scalar_direct_calls_n0": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 23015,
            "sides": [
              {
                "calibrated_total_ns": 23695668.625,
                "ns_per_operation": 1029.575,
                "observed_batch": 23015,
                "requested_batch": 23015,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 25293945.3,
                "ns_per_operation": 1099.02,
                "observed_batch": 23015,
                "requested_batch": 23015,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 23695668.625,
            "ns_per_operation": 1029.575,
            "observed_batch": 23015,
            "requested_batch": 23015,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 25293945.3,
            "ns_per_operation": 1099.02,
            "observed_batch": 23015,
            "requested_batch": 23015,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 28251303.755000003,
            "ns_per_operation": 1227.517,
            "observed_batch": 23015,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 18962956.088999998,
            "ns_per_operation": 1005.939,
            "observed_batch": 18851,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 23015,
        "source_sha256": "0a075601a2a651f9cb95e8ee0404c64bbf076d17bf242b9c88fecbb8cb973826",
        "wasm_sha256": "f6adbecce99c05bf06f213f4f5c0d5d9f001074acbbb2e6bbe814696bb866e45"
      },
      "1:scalar_direct_calls_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 13957,
            "sides": [
              {
                "calibrated_total_ns": 37648393.392000005,
                "ns_per_operation": 2697.456,
                "observed_batch": 13957,
                "requested_batch": 13957,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 30782973.006,
                "ns_per_operation": 2205.558,
                "observed_batch": 13957,
                "requested_batch": 13957,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 37648393.392000005,
            "ns_per_operation": 2697.456,
            "observed_batch": 13957,
            "requested_batch": 13957,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 30782973.006,
            "ns_per_operation": 2205.558,
            "observed_batch": 13957,
            "requested_batch": 13957,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 37416148.912,
            "ns_per_operation": 2680.816,
            "observed_batch": 13957,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 31887302.759999998,
            "ns_per_operation": 2351.571,
            "observed_batch": 13560,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 13957,
        "source_sha256": "0a075601a2a651f9cb95e8ee0404c64bbf076d17bf242b9c88fecbb8cb973826",
        "wasm_sha256": "f6adbecce99c05bf06f213f4f5c0d5d9f001074acbbb2e6bbe814696bb866e45"
      },
      "1:scalar_direct_calls_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 2,
            "sides": [
              {
                "calibrated_total_ns": 33621303.0,
                "ns_per_operation": 16810651.5,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 23970084.0,
                "ns_per_operation": 11985042.0,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 33621303.0,
            "ns_per_operation": 16810651.5,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 23970084.0,
            "ns_per_operation": 11985042.0,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 26663819.0,
            "ns_per_operation": 13331909.5,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 31406760.0,
            "ns_per_operation": 15703380.0,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 2,
        "source_sha256": "0a075601a2a651f9cb95e8ee0404c64bbf076d17bf242b9c88fecbb8cb973826",
        "wasm_sha256": "f6adbecce99c05bf06f213f4f5c0d5d9f001074acbbb2e6bbe814696bb866e45"
      },
      "1:scalar_direct_calls_n1024": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 17,
            "sides": [
              {
                "calibrated_total_ns": 19162364.997,
                "ns_per_operation": 1127197.941,
                "observed_batch": 17,
                "requested_batch": 17,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 23309956.997,
                "ns_per_operation": 1371173.941,
                "observed_batch": 17,
                "requested_batch": 17,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 20,
            "sides": [
              {
                "calibrated_total_ns": 27013800.0,
                "ns_per_operation": 1350690.0,
                "observed_batch": 20,
                "requested_batch": 20,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 25460523.0,
                "ns_per_operation": 1273026.15,
                "observed_batch": 20,
                "requested_batch": 20,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 27013800.0,
            "ns_per_operation": 1350690.0,
            "observed_batch": 20,
            "requested_batch": 20,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 25460523.0,
            "ns_per_operation": 1273026.15,
            "observed_batch": 20,
            "requested_batch": 20,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 20477957.002,
            "ns_per_operation": 1204585.706,
            "observed_batch": 17,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 30612528.992,
            "ns_per_operation": 1913283.062,
            "observed_batch": 16,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 20,
        "source_sha256": "0a075601a2a651f9cb95e8ee0404c64bbf076d17bf242b9c88fecbb8cb973826",
        "wasm_sha256": "f6adbecce99c05bf06f213f4f5c0d5d9f001074acbbb2e6bbe814696bb866e45"
      },
      "1:scalar_direct_calls_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1690,
            "sides": [
              {
                "calibrated_total_ns": 47936473.129999995,
                "ns_per_operation": 28364.777,
                "observed_batch": 1690,
                "requested_batch": 1690,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 41759754.660000004,
                "ns_per_operation": 24709.914,
                "observed_batch": 1690,
                "requested_batch": 1690,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 47936473.129999995,
            "ns_per_operation": 28364.777,
            "observed_batch": 1690,
            "requested_batch": 1690,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 41759754.660000004,
            "ns_per_operation": 24709.914,
            "observed_batch": 1690,
            "requested_batch": 1690,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 40473627.48,
            "ns_per_operation": 23948.892,
            "observed_batch": 1690,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 37699943.92,
            "ns_per_operation": 22334.09,
            "observed_batch": 1688,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1690,
        "source_sha256": "0a075601a2a651f9cb95e8ee0404c64bbf076d17bf242b9c88fecbb8cb973826",
        "wasm_sha256": "f6adbecce99c05bf06f213f4f5c0d5d9f001074acbbb2e6bbe814696bb866e45"
      },
      "1:scalar_direct_calls_n32": {
        "attempt_count": 3,
        "attempts": [
          {
            "batch": 219,
            "sides": [
              {
                "calibrated_total_ns": 9600596.022,
                "ns_per_operation": 43838.338,
                "observed_batch": 219,
                "requested_batch": 219,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 9783328.965,
                "ns_per_operation": 44672.735,
                "observed_batch": 219,
                "requested_batch": 219,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 502,
            "sides": [
              {
                "calibrated_total_ns": 18933860.206,
                "ns_per_operation": 37716.853,
                "observed_batch": 502,
                "requested_batch": 502,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 22097997.832,
                "ns_per_operation": 44019.916,
                "observed_batch": 502,
                "requested_batch": 502,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 584,
            "sides": [
              {
                "calibrated_total_ns": 25513999.904,
                "ns_per_operation": 43688.356,
                "observed_batch": 584,
                "requested_batch": 584,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 34565338.816,
                "ns_per_operation": 59187.224,
                "observed_batch": 584,
                "requested_batch": 584,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 25513999.904,
            "ns_per_operation": 43688.356,
            "observed_batch": 584,
            "requested_batch": 584,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 34565338.816,
            "ns_per_operation": 59187.224,
            "observed_batch": 584,
            "requested_batch": 584,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 9180045.066000002,
            "ns_per_operation": 41918.014,
            "observed_batch": 219,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 8081896.01,
            "ns_per_operation": 37590.214,
            "observed_batch": 215,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 584,
        "source_sha256": "0a075601a2a651f9cb95e8ee0404c64bbf076d17bf242b9c88fecbb8cb973826",
        "wasm_sha256": "f6adbecce99c05bf06f213f4f5c0d5d9f001074acbbb2e6bbe814696bb866e45"
      },
      "1:scalar_dynamic_api_n0": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 51021,
            "sides": [
              {
                "calibrated_total_ns": 17080555.275,
                "ns_per_operation": 334.775,
                "observed_batch": 51021,
                "requested_batch": 51021,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 18761340.078,
                "ns_per_operation": 367.718,
                "observed_batch": 51021,
                "requested_batch": 51021,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 65716,
            "sides": [
              {
                "calibrated_total_ns": 22420393.436,
                "ns_per_operation": 341.171,
                "observed_batch": 65716,
                "requested_batch": 65716,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 22932715.371999998,
                "ns_per_operation": 348.967,
                "observed_batch": 65716,
                "requested_batch": 65716,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 22420393.436,
            "ns_per_operation": 341.171,
            "observed_batch": 65716,
            "requested_batch": 65716,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22932715.371999998,
            "ns_per_operation": 348.967,
            "observed_batch": 65716,
            "requested_batch": 65716,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 19095033.699,
            "ns_per_operation": 418.173,
            "observed_batch": 45663,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 17627704.479000002,
            "ns_per_operation": 345.499,
            "observed_batch": 51021,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 65716,
        "source_sha256": "f02cc93a4a43691ce5be8782fa2df454fc2468a7935117ea18104a7dce22cb2d",
        "wasm_sha256": "c493d0aae68d0c5aa67fc0b562e616c45eb3d42b0302951158a763fa202ce589"
      },
      "1:scalar_dynamic_api_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 56498,
            "sides": [
              {
                "calibrated_total_ns": 21224264.672,
                "ns_per_operation": 375.664,
                "observed_batch": 56498,
                "requested_batch": 56498,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 22210380.764,
                "ns_per_operation": 393.118,
                "observed_batch": 56498,
                "requested_batch": 56498,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 21224264.672,
            "ns_per_operation": 375.664,
            "observed_batch": 56498,
            "requested_batch": 56498,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22210380.764,
            "ns_per_operation": 393.118,
            "observed_batch": 56498,
            "requested_batch": 56498,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 22065745.884,
            "ns_per_operation": 390.558,
            "observed_batch": 56498,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 17695920.301999997,
            "ns_per_operation": 347.722,
            "observed_batch": 50891,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 56498,
        "source_sha256": "f02cc93a4a43691ce5be8782fa2df454fc2468a7935117ea18104a7dce22cb2d",
        "wasm_sha256": "c493d0aae68d0c5aa67fc0b562e616c45eb3d42b0302951158a763fa202ce589"
      },
      "1:scalar_dynamic_api_n10000": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 54201,
            "sides": [
              {
                "calibrated_total_ns": 18067686.546,
                "ns_per_operation": 333.346,
                "observed_batch": 54201,
                "requested_batch": 54201,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 17414998.104,
                "ns_per_operation": 321.304,
                "observed_batch": 54201,
                "requested_batch": 54201,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 68471,
            "sides": [
              {
                "calibrated_total_ns": 24353696.808999997,
                "ns_per_operation": 355.679,
                "observed_batch": 68471,
                "requested_batch": 68471,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 27680908.112,
                "ns_per_operation": 404.272,
                "observed_batch": 68471,
                "requested_batch": 68471,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 24353696.808999997,
            "ns_per_operation": 355.679,
            "observed_batch": 68471,
            "requested_batch": 68471,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27680908.112,
            "ns_per_operation": 404.272,
            "observed_batch": 68471,
            "requested_batch": 68471,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 20585898.322,
            "ns_per_operation": 428.186,
            "observed_batch": 48077,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 20413614.228,
            "ns_per_operation": 376.628,
            "observed_batch": 54201,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 68471,
        "source_sha256": "f02cc93a4a43691ce5be8782fa2df454fc2468a7935117ea18104a7dce22cb2d",
        "wasm_sha256": "c493d0aae68d0c5aa67fc0b562e616c45eb3d42b0302951158a763fa202ce589"
      },
      "1:scalar_dynamic_api_n1024": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 54348,
            "sides": [
              {
                "calibrated_total_ns": 22471484.952,
                "ns_per_operation": 413.474,
                "observed_batch": 54348,
                "requested_batch": 54348,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 19587997.464,
                "ns_per_operation": 360.418,
                "observed_batch": 54348,
                "requested_batch": 54348,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 61041,
            "sides": [
              {
                "calibrated_total_ns": 22152328.269,
                "ns_per_operation": 362.909,
                "observed_batch": 61041,
                "requested_batch": 61041,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 27209147.832000002,
                "ns_per_operation": 445.752,
                "observed_batch": 61041,
                "requested_batch": 61041,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 22152328.269,
            "ns_per_operation": 362.909,
            "observed_batch": 61041,
            "requested_batch": 61041,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27209147.832000002,
            "ns_per_operation": 445.752,
            "observed_batch": 61041,
            "requested_batch": 61041,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 21404144.58,
            "ns_per_operation": 393.835,
            "observed_batch": 54348,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 17605710.675,
            "ns_per_operation": 355.635,
            "observed_batch": 49505,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 61041,
        "source_sha256": "f02cc93a4a43691ce5be8782fa2df454fc2468a7935117ea18104a7dce22cb2d",
        "wasm_sha256": "c493d0aae68d0c5aa67fc0b562e616c45eb3d42b0302951158a763fa202ce589"
      },
      "1:scalar_dynamic_api_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 56180,
            "sides": [
              {
                "calibrated_total_ns": 25223977.3,
                "ns_per_operation": 448.985,
                "observed_batch": 56180,
                "requested_batch": 56180,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 22799866.48,
                "ns_per_operation": 405.836,
                "observed_batch": 56180,
                "requested_batch": 56180,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 25223977.3,
            "ns_per_operation": 448.985,
            "observed_batch": 56180,
            "requested_batch": 56180,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22799866.48,
            "ns_per_operation": 405.836,
            "observed_batch": 56180,
            "requested_batch": 56180,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 21638007.9,
            "ns_per_operation": 385.155,
            "observed_batch": 56180,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 15577254.309,
            "ns_per_operation": 366.843,
            "observed_batch": 42463,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 56180,
        "source_sha256": "f02cc93a4a43691ce5be8782fa2df454fc2468a7935117ea18104a7dce22cb2d",
        "wasm_sha256": "c493d0aae68d0c5aa67fc0b562e616c45eb3d42b0302951158a763fa202ce589"
      },
      "1:scalar_dynamic_api_n32": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 51680,
            "sides": [
              {
                "calibrated_total_ns": 18382679.36,
                "ns_per_operation": 355.702,
                "observed_batch": 51680,
                "requested_batch": 51680,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 20911950.24,
                "ns_per_operation": 404.643,
                "observed_batch": 51680,
                "requested_batch": 51680,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 61850,
            "sides": [
              {
                "calibrated_total_ns": 22286039.400000002,
                "ns_per_operation": 360.324,
                "observed_batch": 61850,
                "requested_batch": 61850,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 23959576.7,
                "ns_per_operation": 387.382,
                "observed_batch": 61850,
                "requested_batch": 61850,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 22286039.400000002,
            "ns_per_operation": 360.324,
            "observed_batch": 61850,
            "requested_batch": 61850,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 23959576.7,
            "ns_per_operation": 387.382,
            "observed_batch": 61850,
            "requested_batch": 61850,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 19162788.96,
            "ns_per_operation": 370.797,
            "observed_batch": 51680,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 18377252.959999997,
            "ns_per_operation": 355.597,
            "observed_batch": 51680,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 61850,
        "source_sha256": "f02cc93a4a43691ce5be8782fa2df454fc2468a7935117ea18104a7dce22cb2d",
        "wasm_sha256": "c493d0aae68d0c5aa67fc0b562e616c45eb3d42b0302951158a763fa202ce589"
      },
      "1:scalar_host_call_n0": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 27064,
            "sides": [
              {
                "calibrated_total_ns": 146858709.336,
                "ns_per_operation": 5426.349,
                "observed_batch": 27064,
                "requested_batch": 27064,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 29230743.84,
                "ns_per_operation": 1080.06,
                "observed_batch": 27064,
                "requested_batch": 27064,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 146858709.336,
            "ns_per_operation": 5426.349,
            "observed_batch": 27064,
            "requested_batch": 27064,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 29230743.84,
            "ns_per_operation": 1080.06,
            "observed_batch": 27064,
            "requested_batch": 27064,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 22326894.786,
            "ns_per_operation": 887.467,
            "observed_batch": 25158,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 23703706.696000002,
            "ns_per_operation": 875.839,
            "observed_batch": 27064,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 27064,
        "source_sha256": "4c4026de620eff0b1e9cb4ed4e0440e6045d921267e3962715f6430ac68f5120",
        "wasm_sha256": "c60311d57c929c3ff1d29adb70501e7bbf8419517acbd86c8d072438b610ebb7"
      },
      "1:scalar_host_call_n1": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 14462,
            "sides": [
              {
                "calibrated_total_ns": 19400917.62,
                "ns_per_operation": 1341.51,
                "observed_batch": 14462,
                "requested_batch": 14462,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 23861938.45,
                "ns_per_operation": 1649.975,
                "observed_batch": 14462,
                "requested_batch": 14462,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 16400,
            "sides": [
              {
                "calibrated_total_ns": 25845547.200000003,
                "ns_per_operation": 1575.948,
                "observed_batch": 16400,
                "requested_batch": 16400,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 22316874.0,
                "ns_per_operation": 1360.785,
                "observed_batch": 16400,
                "requested_batch": 16400,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 25845547.200000003,
            "ns_per_operation": 1575.948,
            "observed_batch": 16400,
            "requested_batch": 16400,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22316874.0,
            "ns_per_operation": 1360.785,
            "observed_batch": 16400,
            "requested_batch": 16400,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 20512853.795,
            "ns_per_operation": 1530.239,
            "observed_batch": 13405,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 20863662.148,
            "ns_per_operation": 1442.654,
            "observed_batch": 14462,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 16400,
        "source_sha256": "4c4026de620eff0b1e9cb4ed4e0440e6045d921267e3962715f6430ac68f5120",
        "wasm_sha256": "c60311d57c929c3ff1d29adb70501e7bbf8419517acbd86c8d072438b610ebb7"
      },
      "1:scalar_host_call_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 3,
            "sides": [
              {
                "calibrated_total_ns": 21789852.998999998,
                "ns_per_operation": 7263284.333,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 25229378.001,
                "ns_per_operation": 8409792.667,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 21789852.998999998,
            "ns_per_operation": 7263284.333,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 25229378.001,
            "ns_per_operation": 8409792.667,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 23313819.998999998,
            "ns_per_operation": 7771273.333,
            "observed_batch": 3,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22970916.0,
            "ns_per_operation": 7656972.0,
            "observed_batch": 3,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 3,
        "source_sha256": "4c4026de620eff0b1e9cb4ed4e0440e6045d921267e3962715f6430ac68f5120",
        "wasm_sha256": "c60311d57c929c3ff1d29adb70501e7bbf8419517acbd86c8d072438b610ebb7"
      },
      "1:scalar_host_call_n1024": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 39,
            "sides": [
              {
                "calibrated_total_ns": 31388831.019,
                "ns_per_operation": 804841.821,
                "observed_batch": 39,
                "requested_batch": 39,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 29387975.994,
                "ns_per_operation": 753537.846,
                "observed_batch": 39,
                "requested_batch": 39,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 31388831.019,
            "ns_per_operation": 804841.821,
            "observed_batch": 39,
            "requested_batch": 39,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 29387975.994,
            "ns_per_operation": 753537.846,
            "observed_batch": 39,
            "requested_batch": 39,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 32784922.989,
            "ns_per_operation": 840639.051,
            "observed_batch": 39,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 26669440.987999998,
            "ns_per_operation": 860304.548,
            "observed_batch": 31,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 39,
        "source_sha256": "4c4026de620eff0b1e9cb4ed4e0440e6045d921267e3962715f6430ac68f5120",
        "wasm_sha256": "c60311d57c929c3ff1d29adb70501e7bbf8419517acbd86c8d072438b610ebb7"
      },
      "1:scalar_host_call_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 2075,
            "sides": [
              {
                "calibrated_total_ns": 28958604.55,
                "ns_per_operation": 13955.954,
                "observed_batch": 2075,
                "requested_batch": 2075,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 29629771.599999998,
                "ns_per_operation": 14279.408,
                "observed_batch": 2075,
                "requested_batch": 2075,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 28958604.55,
            "ns_per_operation": 13955.954,
            "observed_batch": 2075,
            "requested_batch": 2075,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 29629771.599999998,
            "ns_per_operation": 14279.408,
            "observed_batch": 2075,
            "requested_batch": 2075,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 29525114.905,
            "ns_per_operation": 14437.709,
            "observed_batch": 2045,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 34214907.4,
            "ns_per_operation": 16489.112,
            "observed_batch": 2075,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 2075,
        "source_sha256": "4c4026de620eff0b1e9cb4ed4e0440e6045d921267e3962715f6430ac68f5120",
        "wasm_sha256": "c60311d57c929c3ff1d29adb70501e7bbf8419517acbd86c8d072438b610ebb7"
      },
      "1:scalar_host_call_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1136,
            "sides": [
              {
                "calibrated_total_ns": 33387647.76,
                "ns_per_operation": 29390.535,
                "observed_batch": 1136,
                "requested_batch": 1136,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 46635084.496,
                "ns_per_operation": 41052.011,
                "observed_batch": 1136,
                "requested_batch": 1136,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 33387647.76,
            "ns_per_operation": 29390.535,
            "observed_batch": 1136,
            "requested_batch": 1136,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 46635084.496,
            "ns_per_operation": 41052.011,
            "observed_batch": 1136,
            "requested_batch": 1136,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 31238326.672,
            "ns_per_operation": 27498.527,
            "observed_batch": 1136,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 66300876.357999995,
            "ns_per_operation": 59516.047,
            "observed_batch": 1114,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1136,
        "source_sha256": "4c4026de620eff0b1e9cb4ed4e0440e6045d921267e3962715f6430ac68f5120",
        "wasm_sha256": "c60311d57c929c3ff1d29adb70501e7bbf8419517acbd86c8d072438b610ebb7"
      },
      "1:scalar_indirect_calls_n0": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 22858,
            "sides": [
              {
                "calibrated_total_ns": 18266673.546,
                "ns_per_operation": 799.137,
                "observed_batch": 22858,
                "requested_batch": 22858,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 17765671.902000003,
                "ns_per_operation": 777.219,
                "observed_batch": 22858,
                "requested_batch": 22858,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 28307,
            "sides": [
              {
                "calibrated_total_ns": 26439870.279999997,
                "ns_per_operation": 934.04,
                "observed_batch": 28307,
                "requested_batch": 28307,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 23458803.496,
                "ns_per_operation": 828.728,
                "observed_batch": 28307,
                "requested_batch": 28307,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 26439870.279999997,
            "ns_per_operation": 934.04,
            "observed_batch": 28307,
            "requested_batch": 28307,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 23458803.496,
            "ns_per_operation": 828.728,
            "observed_batch": 28307,
            "requested_batch": 28307,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 24794095.458,
            "ns_per_operation": 1084.701,
            "observed_batch": 22858,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 15329126.694,
            "ns_per_operation": 807.051,
            "observed_batch": 18994,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 28307,
        "source_sha256": "29024ae23844664a9fea4a809d1ce414de40f243f7e678ce5ea6d811843f7797",
        "wasm_sha256": "4b42e82f54ea6ab7d0f62c70766c904e41d6a408e57fa14b5abbf514c0da78a9"
      },
      "1:scalar_indirect_calls_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 12954,
            "sides": [
              {
                "calibrated_total_ns": 47388180.306,
                "ns_per_operation": 3658.189,
                "observed_batch": 12954,
                "requested_batch": 12954,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 29278721.478,
                "ns_per_operation": 2260.207,
                "observed_batch": 12954,
                "requested_batch": 12954,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 47388180.306,
            "ns_per_operation": 3658.189,
            "observed_batch": 12954,
            "requested_batch": 12954,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 29278721.478,
            "ns_per_operation": 2260.207,
            "observed_batch": 12954,
            "requested_batch": 12954,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 29453950.235999998,
            "ns_per_operation": 2273.734,
            "observed_batch": 12954,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 31403290.604,
            "ns_per_operation": 2454.149,
            "observed_batch": 12796,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 12954,
        "source_sha256": "29024ae23844664a9fea4a809d1ce414de40f243f7e678ce5ea6d811843f7797",
        "wasm_sha256": "4b42e82f54ea6ab7d0f62c70766c904e41d6a408e57fa14b5abbf514c0da78a9"
      },
      "1:scalar_indirect_calls_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 2,
            "sides": [
              {
                "calibrated_total_ns": 26758943.0,
                "ns_per_operation": 13379471.5,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 33954994.0,
                "ns_per_operation": 16977497.0,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 26758943.0,
            "ns_per_operation": 13379471.5,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 33954994.0,
            "ns_per_operation": 16977497.0,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 24285690.0,
            "ns_per_operation": 12142845.0,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27913760.0,
            "ns_per_operation": 13956880.0,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 2,
        "source_sha256": "29024ae23844664a9fea4a809d1ce414de40f243f7e678ce5ea6d811843f7797",
        "wasm_sha256": "4b42e82f54ea6ab7d0f62c70766c904e41d6a408e57fa14b5abbf514c0da78a9"
      },
      "1:scalar_indirect_calls_n1024": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 20,
            "sides": [
              {
                "calibrated_total_ns": 24544266.0,
                "ns_per_operation": 1227213.3,
                "observed_batch": 20,
                "requested_batch": 20,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 26747424.0,
                "ns_per_operation": 1337371.2,
                "observed_batch": 20,
                "requested_batch": 20,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 24544266.0,
            "ns_per_operation": 1227213.3,
            "observed_batch": 20,
            "requested_batch": 20,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 26747424.0,
            "ns_per_operation": 1337371.2,
            "observed_batch": 20,
            "requested_batch": 20,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 23360519.995,
            "ns_per_operation": 1374148.235,
            "observed_batch": 17,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 25191659.0,
            "ns_per_operation": 1259582.95,
            "observed_batch": 20,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 20,
        "source_sha256": "29024ae23844664a9fea4a809d1ce414de40f243f7e678ce5ea6d811843f7797",
        "wasm_sha256": "4b42e82f54ea6ab7d0f62c70766c904e41d6a408e57fa14b5abbf514c0da78a9"
      },
      "1:scalar_indirect_calls_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1569,
            "sides": [
              {
                "calibrated_total_ns": 37038063.471,
                "ns_per_operation": 23606.159,
                "observed_batch": 1569,
                "requested_batch": 1569,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 37087946.688,
                "ns_per_operation": 23637.952,
                "observed_batch": 1569,
                "requested_batch": 1569,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 37038063.471,
            "ns_per_operation": 23606.159,
            "observed_batch": 1569,
            "requested_batch": 1569,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 37087946.688,
            "ns_per_operation": 23637.952,
            "observed_batch": 1569,
            "requested_batch": 1569,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 40050116.703,
            "ns_per_operation": 25525.887,
            "observed_batch": 1569,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 7671492.862,
            "ns_per_operation": 26544.958,
            "observed_batch": 289,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1569,
        "source_sha256": "29024ae23844664a9fea4a809d1ce414de40f243f7e678ce5ea6d811843f7797",
        "wasm_sha256": "4b42e82f54ea6ab7d0f62c70766c904e41d6a408e57fa14b5abbf514c0da78a9"
      },
      "1:scalar_indirect_calls_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 861,
            "sides": [
              {
                "calibrated_total_ns": 33655772.787,
                "ns_per_operation": 39089.167,
                "observed_batch": 861,
                "requested_batch": 861,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 35497785.855,
                "ns_per_operation": 41228.555,
                "observed_batch": 861,
                "requested_batch": 861,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 33655772.787,
            "ns_per_operation": 39089.167,
            "observed_batch": 861,
            "requested_batch": 861,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 35497785.855,
            "ns_per_operation": 41228.555,
            "observed_batch": 861,
            "requested_batch": 861,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 35954530.857,
            "ns_per_operation": 41759.037,
            "observed_batch": 861,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 34868228.178,
            "ns_per_operation": 40829.307,
            "observed_batch": 854,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 861,
        "source_sha256": "29024ae23844664a9fea4a809d1ce414de40f243f7e678ce5ea6d811843f7797",
        "wasm_sha256": "4b42e82f54ea6ab7d0f62c70766c904e41d6a408e57fa14b5abbf514c0da78a9"
      },
      "1:scalar_multivalue_branch_n0": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 23981,
            "sides": [
              {
                "calibrated_total_ns": 23305814.945,
                "ns_per_operation": 971.845,
                "observed_batch": 23981,
                "requested_batch": 23981,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 22079786.32,
                "ns_per_operation": 920.72,
                "observed_batch": 23981,
                "requested_batch": 23981,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 23305814.945,
            "ns_per_operation": 971.845,
            "observed_batch": 23981,
            "requested_batch": 23981,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22079786.32,
            "ns_per_operation": 920.72,
            "observed_batch": 23981,
            "requested_batch": 23981,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 20561462.55,
            "ns_per_operation": 887.226,
            "observed_batch": 23175,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22999121.936,
            "ns_per_operation": 959.056,
            "observed_batch": 23981,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 23981,
        "source_sha256": "a1f4958100af82889208bb28090d6af7c3e56cbeeccabad4106601c9a9c17014",
        "wasm_sha256": "03ffdc2041ae8ba0c8d078422d36152cf1d35f1293c94a4074fe7e83791743ae"
      },
      "1:scalar_multivalue_branch_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 11662,
            "sides": [
              {
                "calibrated_total_ns": 33439735.419999998,
                "ns_per_operation": 2867.41,
                "observed_batch": 11662,
                "requested_batch": 11662,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 28526639.778,
                "ns_per_operation": 2446.119,
                "observed_batch": 11662,
                "requested_batch": 11662,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 33439735.419999998,
            "ns_per_operation": 2867.41,
            "observed_batch": 11662,
            "requested_batch": 11662,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 28526639.778,
            "ns_per_operation": 2446.119,
            "observed_batch": 11662,
            "requested_batch": 11662,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 24952901.512,
            "ns_per_operation": 2139.676,
            "observed_batch": 11662,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 23756970.508,
            "ns_per_operation": 2189.179,
            "observed_batch": 10852,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 11662,
        "source_sha256": "a1f4958100af82889208bb28090d6af7c3e56cbeeccabad4106601c9a9c17014",
        "wasm_sha256": "03ffdc2041ae8ba0c8d078422d36152cf1d35f1293c94a4074fe7e83791743ae"
      },
      "1:scalar_multivalue_branch_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 2,
            "sides": [
              {
                "calibrated_total_ns": 30217216.0,
                "ns_per_operation": 15108608.0,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 29914876.0,
                "ns_per_operation": 14957438.0,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 30217216.0,
            "ns_per_operation": 15108608.0,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 29914876.0,
            "ns_per_operation": 14957438.0,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 32620479.0,
            "ns_per_operation": 16310239.5,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 29550601.0,
            "ns_per_operation": 14775300.5,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 2,
        "source_sha256": "a1f4958100af82889208bb28090d6af7c3e56cbeeccabad4106601c9a9c17014",
        "wasm_sha256": "03ffdc2041ae8ba0c8d078422d36152cf1d35f1293c94a4074fe7e83791743ae"
      },
      "1:scalar_multivalue_branch_n1024": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 14,
            "sides": [
              {
                "calibrated_total_ns": 19865253.002,
                "ns_per_operation": 1418946.643,
                "observed_batch": 14,
                "requested_batch": 14,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 23747992.996,
                "ns_per_operation": 1696285.214,
                "observed_batch": 14,
                "requested_batch": 14,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 16,
            "sides": [
              {
                "calibrated_total_ns": 21529551.008,
                "ns_per_operation": 1345596.938,
                "observed_batch": 16,
                "requested_batch": 16,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 22348103.008,
                "ns_per_operation": 1396756.438,
                "observed_batch": 16,
                "requested_batch": 16,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 21529551.008,
            "ns_per_operation": 1345596.938,
            "observed_batch": 16,
            "requested_batch": 16,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22348103.008,
            "ns_per_operation": 1396756.438,
            "observed_batch": 16,
            "requested_batch": 16,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 21651583.996,
            "ns_per_operation": 1546541.714,
            "observed_batch": 14,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 19662141.002,
            "ns_per_operation": 1404438.643,
            "observed_batch": 14,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 16,
        "source_sha256": "a1f4958100af82889208bb28090d6af7c3e56cbeeccabad4106601c9a9c17014",
        "wasm_sha256": "03ffdc2041ae8ba0c8d078422d36152cf1d35f1293c94a4074fe7e83791743ae"
      },
      "1:scalar_multivalue_branch_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1184,
            "sides": [
              {
                "calibrated_total_ns": 33037854.112,
                "ns_per_operation": 27903.593,
                "observed_batch": 1184,
                "requested_batch": 1184,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 30703317.504,
                "ns_per_operation": 25931.856,
                "observed_batch": 1184,
                "requested_batch": 1184,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 33037854.112,
            "ns_per_operation": 27903.593,
            "observed_batch": 1184,
            "requested_batch": 1184,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 30703317.504,
            "ns_per_operation": 25931.856,
            "observed_batch": 1184,
            "requested_batch": 1184,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 45953006.519999996,
            "ns_per_operation": 40239.06,
            "observed_batch": 1142,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 37284984.064,
            "ns_per_operation": 31490.696,
            "observed_batch": 1184,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1184,
        "source_sha256": "a1f4958100af82889208bb28090d6af7c3e56cbeeccabad4106601c9a9c17014",
        "wasm_sha256": "03ffdc2041ae8ba0c8d078422d36152cf1d35f1293c94a4074fe7e83791743ae"
      },
      "1:scalar_multivalue_branch_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 675,
            "sides": [
              {
                "calibrated_total_ns": 30231858.825,
                "ns_per_operation": 44787.939,
                "observed_batch": 675,
                "requested_batch": 675,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 38336901.75,
                "ns_per_operation": 56795.41,
                "observed_batch": 675,
                "requested_batch": 675,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 30231858.825,
            "ns_per_operation": 44787.939,
            "observed_batch": 675,
            "requested_batch": 675,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 38336901.75,
            "ns_per_operation": 56795.41,
            "observed_batch": 675,
            "requested_batch": 675,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 33087885.749999996,
            "ns_per_operation": 49019.09,
            "observed_batch": 675,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 31563890.16,
            "ns_per_operation": 47824.076,
            "observed_batch": 660,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 675,
        "source_sha256": "a1f4958100af82889208bb28090d6af7c3e56cbeeccabad4106601c9a9c17014",
        "wasm_sha256": "03ffdc2041ae8ba0c8d078422d36152cf1d35f1293c94a4074fe7e83791743ae"
      },
      "1:scalar_ordinary_recursion_n0": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 9611,
            "sides": [
              {
                "calibrated_total_ns": 10864111.013,
                "ns_per_operation": 1130.383,
                "observed_batch": 9611,
                "requested_batch": 9611,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 10323232.766,
                "ns_per_operation": 1074.106,
                "observed_batch": 9611,
                "requested_batch": 9611,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 20483,
            "sides": [
              {
                "calibrated_total_ns": 26120024.014999997,
                "ns_per_operation": 1275.205,
                "observed_batch": 20483,
                "requested_batch": 20483,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 22833301.351999998,
                "ns_per_operation": 1114.744,
                "observed_batch": 20483,
                "requested_batch": 20483,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 26120024.014999997,
            "ns_per_operation": 1275.205,
            "observed_batch": 20483,
            "requested_batch": 20483,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22833301.351999998,
            "ns_per_operation": 1114.744,
            "observed_batch": 20483,
            "requested_batch": 20483,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 10669036.546,
            "ns_per_operation": 1110.086,
            "observed_batch": 9611,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 8457519.98,
            "ns_per_operation": 1010.215,
            "observed_batch": 8372,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 20483,
        "source_sha256": "aca6a66637fa36b72f3c528dd5e3e44039b573f363b5b4fb77437e9278623903",
        "wasm_sha256": "e1bd7ea6ada8b15b2770ac387f8fc2256f20d7f87c4bc34e7559b239d8875ba1"
      },
      "1:scalar_ordinary_recursion_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 15409,
            "sides": [
              {
                "calibrated_total_ns": 32676263.764,
                "ns_per_operation": 2120.596,
                "observed_batch": 15409,
                "requested_batch": 15409,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 33522248.682,
                "ns_per_operation": 2175.498,
                "observed_batch": 15409,
                "requested_batch": 15409,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 32676263.764,
            "ns_per_operation": 2120.596,
            "observed_batch": 15409,
            "requested_batch": 15409,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 33522248.682,
            "ns_per_operation": 2175.498,
            "observed_batch": 15409,
            "requested_batch": 15409,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 33153433.163999997,
            "ns_per_operation": 2287.548,
            "observed_batch": 14493,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 33782383.42,
            "ns_per_operation": 2192.38,
            "observed_batch": 15409,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 15409,
        "source_sha256": "aca6a66637fa36b72f3c528dd5e3e44039b573f363b5b4fb77437e9278623903",
        "wasm_sha256": "e1bd7ea6ada8b15b2770ac387f8fc2256f20d7f87c4bc34e7559b239d8875ba1"
      },
      "1:scalar_ordinary_recursion_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1867,
            "sides": [
              {
                "calibrated_total_ns": 42462310.535,
                "ns_per_operation": 22743.605,
                "observed_batch": 1867,
                "requested_batch": 1867,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 42616005.709,
                "ns_per_operation": 22825.927,
                "observed_batch": 1867,
                "requested_batch": 1867,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 42462310.535,
            "ns_per_operation": 22743.605,
            "observed_batch": 1867,
            "requested_batch": 1867,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 42616005.709,
            "ns_per_operation": 22825.927,
            "observed_batch": 1867,
            "requested_batch": 1867,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 42130529.996,
            "ns_per_operation": 22847.359,
            "observed_batch": 1844,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 40979503.662,
            "ns_per_operation": 21949.386,
            "observed_batch": 1867,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1867,
        "source_sha256": "aca6a66637fa36b72f3c528dd5e3e44039b573f363b5b4fb77437e9278623903",
        "wasm_sha256": "e1bd7ea6ada8b15b2770ac387f8fc2256f20d7f87c4bc34e7559b239d8875ba1"
      },
      "1:scalar_ordinary_recursion_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 876,
            "sides": [
              {
                "calibrated_total_ns": 27564830.076,
                "ns_per_operation": 31466.701,
                "observed_batch": 876,
                "requested_batch": 876,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 30922588.884,
                "ns_per_operation": 35299.759,
                "observed_batch": 876,
                "requested_batch": 876,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 27564830.076,
            "ns_per_operation": 31466.701,
            "observed_batch": 876,
            "requested_batch": 876,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 30922588.884,
            "ns_per_operation": 35299.759,
            "observed_batch": 876,
            "requested_batch": 876,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 4239609.972,
            "ns_per_operation": 34190.403,
            "observed_batch": 124,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 28789138.188,
            "ns_per_operation": 32864.313,
            "observed_batch": 876,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 876,
        "source_sha256": "aca6a66637fa36b72f3c528dd5e3e44039b573f363b5b4fb77437e9278623903",
        "wasm_sha256": "e1bd7ea6ada8b15b2770ac387f8fc2256f20d7f87c4bc34e7559b239d8875ba1"
      },
      "1:scalar_table_mutation_n0": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 24302,
            "sides": [
              {
                "calibrated_total_ns": 23150133.803999998,
                "ns_per_operation": 952.602,
                "observed_batch": 24302,
                "requested_batch": 24302,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 19690330.97,
                "ns_per_operation": 810.235,
                "observed_batch": 24302,
                "requested_batch": 24302,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 27153,
            "sides": [
              {
                "calibrated_total_ns": 26388941.733,
                "ns_per_operation": 971.861,
                "observed_batch": 27153,
                "requested_batch": 27153,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 24858978.794999998,
                "ns_per_operation": 915.515,
                "observed_batch": 27153,
                "requested_batch": 27153,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 26388941.733,
            "ns_per_operation": 971.861,
            "observed_batch": 27153,
            "requested_batch": 27153,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 24858978.794999998,
            "ns_per_operation": 915.515,
            "observed_batch": 27153,
            "requested_batch": 27153,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 19634882.022,
            "ns_per_operation": 872.739,
            "observed_batch": 22498,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 23286832.553999998,
            "ns_per_operation": 958.227,
            "observed_batch": 24302,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 27153,
        "source_sha256": "9b61bd9cbca804bf639085769fd766a2cf8a69d516fc7efb9e4edfe4ff6d18c9",
        "wasm_sha256": "458f2b111e48d2efca98083b8c56860c43e82be8752813d1e2814d23c28d4f43"
      },
      "1:scalar_table_mutation_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 14348,
            "sides": [
              {
                "calibrated_total_ns": 28619826.468000002,
                "ns_per_operation": 1994.691,
                "observed_batch": 14348,
                "requested_batch": 14348,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 38186298.076,
                "ns_per_operation": 2661.437,
                "observed_batch": 14348,
                "requested_batch": 14348,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 28619826.468000002,
            "ns_per_operation": 1994.691,
            "observed_batch": 14348,
            "requested_batch": 14348,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 38186298.076,
            "ns_per_operation": 2661.437,
            "observed_batch": 14348,
            "requested_batch": 14348,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 26867232.616,
            "ns_per_operation": 1872.542,
            "observed_batch": 14348,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 25026670.27,
            "ns_per_operation": 1779.358,
            "observed_batch": 14065,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 14348,
        "source_sha256": "9b61bd9cbca804bf639085769fd766a2cf8a69d516fc7efb9e4edfe4ff6d18c9",
        "wasm_sha256": "458f2b111e48d2efca98083b8c56860c43e82be8752813d1e2814d23c28d4f43"
      },
      "1:scalar_table_mutation_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 3,
            "sides": [
              {
                "calibrated_total_ns": 32152667.001,
                "ns_per_operation": 10717555.667,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 28808637.0,
                "ns_per_operation": 9602879.0,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 32152667.001,
            "ns_per_operation": 10717555.667,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 28808637.0,
            "ns_per_operation": 9602879.0,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 30509274.0,
            "ns_per_operation": 10169758.0,
            "observed_batch": 3,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 37837122.0,
            "ns_per_operation": 12612374.0,
            "observed_batch": 3,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 3,
        "source_sha256": "9b61bd9cbca804bf639085769fd766a2cf8a69d516fc7efb9e4edfe4ff6d18c9",
        "wasm_sha256": "458f2b111e48d2efca98083b8c56860c43e82be8752813d1e2814d23c28d4f43"
      },
      "1:scalar_table_mutation_n1024": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 20,
            "sides": [
              {
                "calibrated_total_ns": 28009347.0,
                "ns_per_operation": 1400467.35,
                "observed_batch": 20,
                "requested_batch": 20,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 19261583.0,
                "ns_per_operation": 963079.15,
                "observed_batch": 20,
                "requested_batch": 20,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 23,
            "sides": [
              {
                "calibrated_total_ns": 21927280.989,
                "ns_per_operation": 953360.043,
                "observed_batch": 23,
                "requested_batch": 23,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 23429077.006,
                "ns_per_operation": 1018655.522,
                "observed_batch": 23,
                "requested_batch": 23,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 21927280.989,
            "ns_per_operation": 953360.043,
            "observed_batch": 23,
            "requested_batch": 23,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 23429077.006,
            "ns_per_operation": 1018655.522,
            "observed_batch": 23,
            "requested_batch": 23,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 19597369.0,
            "ns_per_operation": 979868.45,
            "observed_batch": 20,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 18487213.997,
            "ns_per_operation": 973011.263,
            "observed_batch": 19,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 23,
        "source_sha256": "9b61bd9cbca804bf639085769fd766a2cf8a69d516fc7efb9e4edfe4ff6d18c9",
        "wasm_sha256": "458f2b111e48d2efca98083b8c56860c43e82be8752813d1e2814d23c28d4f43"
      },
      "1:scalar_table_mutation_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1735,
            "sides": [
              {
                "calibrated_total_ns": 39661529.184999995,
                "ns_per_operation": 22859.671,
                "observed_batch": 1735,
                "requested_batch": 1735,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 33204067.385,
                "ns_per_operation": 19137.791,
                "observed_batch": 1735,
                "requested_batch": 1735,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 39661529.184999995,
            "ns_per_operation": 22859.671,
            "observed_batch": 1735,
            "requested_batch": 1735,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 33204067.385,
            "ns_per_operation": 19137.791,
            "observed_batch": 1735,
            "requested_batch": 1735,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 31983095.04,
            "ns_per_operation": 19220.61,
            "observed_batch": 1664,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 35331250.254999995,
            "ns_per_operation": 20363.833,
            "observed_batch": 1735,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1735,
        "source_sha256": "9b61bd9cbca804bf639085769fd766a2cf8a69d516fc7efb9e4edfe4ff6d18c9",
        "wasm_sha256": "458f2b111e48d2efca98083b8c56860c43e82be8752813d1e2814d23c28d4f43"
      },
      "1:scalar_table_mutation_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 958,
            "sides": [
              {
                "calibrated_total_ns": 34880759.882,
                "ns_per_operation": 36409.979,
                "observed_batch": 958,
                "requested_batch": 958,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 34058523.81,
                "ns_per_operation": 35551.695,
                "observed_batch": 958,
                "requested_batch": 958,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 34880759.882,
            "ns_per_operation": 36409.979,
            "observed_batch": 958,
            "requested_batch": 958,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 34058523.81,
            "ns_per_operation": 35551.695,
            "observed_batch": 958,
            "requested_batch": 958,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 31169859.704,
            "ns_per_operation": 32536.388,
            "observed_batch": 958,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 26046200.163000003,
            "ns_per_operation": 36326.639,
            "observed_batch": 717,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 958,
        "source_sha256": "9b61bd9cbca804bf639085769fd766a2cf8a69d516fc7efb9e4edfe4ff6d18c9",
        "wasm_sha256": "458f2b111e48d2efca98083b8c56860c43e82be8752813d1e2814d23c28d4f43"
      },
      "1:scalar_tail_calls_n0": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 10852,
            "sides": [
              {
                "calibrated_total_ns": 9204232.32,
                "ns_per_operation": 848.16,
                "observed_batch": 10852,
                "requested_batch": 10852,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 8726082.348000001,
                "ns_per_operation": 804.099,
                "observed_batch": 10852,
                "requested_batch": 10852,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 27360,
            "sides": [
              {
                "calibrated_total_ns": 22498839.36,
                "ns_per_operation": 822.326,
                "observed_batch": 27360,
                "requested_batch": 27360,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 22861906.56,
                "ns_per_operation": 835.596,
                "observed_batch": 27360,
                "requested_batch": 27360,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 22498839.36,
            "ns_per_operation": 822.326,
            "observed_batch": 27360,
            "requested_batch": 27360,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22861906.56,
            "ns_per_operation": 835.596,
            "observed_batch": 27360,
            "requested_batch": 27360,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 8638596.0,
            "ns_per_operation": 799.87,
            "observed_batch": 10800,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 9225198.384,
            "ns_per_operation": 850.092,
            "observed_batch": 10852,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 27360,
        "source_sha256": "9953f19384c51b579921e6f789b13a8b57c86ba86d760b77846e3e33825ece02",
        "wasm_sha256": "3e23be294e7a9eda62c89dbc366b6a6cab90edfbda532da27a7e4fc6d0aa4a56"
      },
      "1:scalar_tail_calls_n1": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 7972,
            "sides": [
              {
                "calibrated_total_ns": 13233121.4,
                "ns_per_operation": 1659.95,
                "observed_batch": 7972,
                "requested_batch": 7972,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 14730605.795999998,
                "ns_per_operation": 1847.793,
                "observed_batch": 7972,
                "requested_batch": 7972,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 13254,
            "sides": [
              {
                "calibrated_total_ns": 23180344.728,
                "ns_per_operation": 1748.932,
                "observed_batch": 13254,
                "requested_batch": 13254,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 23363886.12,
                "ns_per_operation": 1762.78,
                "observed_batch": 13254,
                "requested_batch": 13254,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 23180344.728,
            "ns_per_operation": 1748.932,
            "observed_batch": 13254,
            "requested_batch": 13254,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 23363886.12,
            "ns_per_operation": 1762.78,
            "observed_batch": 13254,
            "requested_batch": 13254,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 13307938.725000001,
            "ns_per_operation": 1751.275,
            "observed_batch": 7599,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 13767835.328,
            "ns_per_operation": 1727.024,
            "observed_batch": 7972,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 13254,
        "source_sha256": "9953f19384c51b579921e6f789b13a8b57c86ba86d760b77846e3e33825ece02",
        "wasm_sha256": "3e23be294e7a9eda62c89dbc366b6a6cab90edfbda532da27a7e4fc6d0aa4a56"
      },
      "1:scalar_tail_calls_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 4,
            "sides": [
              {
                "calibrated_total_ns": 33339402.0,
                "ns_per_operation": 8334850.5,
                "observed_batch": 4,
                "requested_batch": 4,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 33276234.0,
                "ns_per_operation": 8319058.5,
                "observed_batch": 4,
                "requested_batch": 4,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 33339402.0,
            "ns_per_operation": 8334850.5,
            "observed_batch": 4,
            "requested_batch": 4,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 33276234.0,
            "ns_per_operation": 8319058.5,
            "observed_batch": 4,
            "requested_batch": 4,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 20705474.001000002,
            "ns_per_operation": 6901824.667,
            "observed_batch": 3,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 29894607.0,
            "ns_per_operation": 7473651.75,
            "observed_batch": 4,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 4,
        "source_sha256": "9953f19384c51b579921e6f789b13a8b57c86ba86d760b77846e3e33825ece02",
        "wasm_sha256": "3e23be294e7a9eda62c89dbc366b6a6cab90edfbda532da27a7e4fc6d0aa4a56"
      },
      "1:scalar_tail_calls_n1024": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 39,
            "sides": [
              {
                "calibrated_total_ns": 50721716.019,
                "ns_per_operation": 1300556.821,
                "observed_batch": 39,
                "requested_batch": 39,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 32050487.001000002,
                "ns_per_operation": 821807.359,
                "observed_batch": 39,
                "requested_batch": 39,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 50721716.019,
            "ns_per_operation": 1300556.821,
            "observed_batch": 39,
            "requested_batch": 39,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 32050487.001000002,
            "ns_per_operation": 821807.359,
            "observed_batch": 39,
            "requested_batch": 39,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 26055559.985000003,
            "ns_per_operation": 840501.935,
            "observed_batch": 31,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 29359785.0,
            "ns_per_operation": 752815.0,
            "observed_batch": 39,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 39,
        "source_sha256": "9953f19384c51b579921e6f789b13a8b57c86ba86d760b77846e3e33825ece02",
        "wasm_sha256": "3e23be294e7a9eda62c89dbc366b6a6cab90edfbda532da27a7e4fc6d0aa4a56"
      },
      "1:scalar_tail_calls_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 2028,
            "sides": [
              {
                "calibrated_total_ns": 24499241.832000002,
                "ns_per_operation": 12080.494,
                "observed_batch": 2028,
                "requested_batch": 2028,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 24787530.143999998,
                "ns_per_operation": 12222.648,
                "observed_batch": 2028,
                "requested_batch": 2028,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 24499241.832000002,
            "ns_per_operation": 12080.494,
            "observed_batch": 2028,
            "requested_batch": 2028,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 24787530.143999998,
            "ns_per_operation": 12222.648,
            "observed_batch": 2028,
            "requested_batch": 2028,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 27705933.684,
            "ns_per_operation": 13661.703,
            "observed_batch": 2028,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 26756565.240000002,
            "ns_per_operation": 13311.724,
            "observed_batch": 2010,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 2028,
        "source_sha256": "9953f19384c51b579921e6f789b13a8b57c86ba86d760b77846e3e33825ece02",
        "wasm_sha256": "3e23be294e7a9eda62c89dbc366b6a6cab90edfbda532da27a7e4fc6d0aa4a56"
      },
      "1:scalar_tail_calls_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1191,
            "sides": [
              {
                "calibrated_total_ns": 27565225.746,
                "ns_per_operation": 23144.606,
                "observed_batch": 1191,
                "requested_batch": 1191,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 32688456.356999997,
                "ns_per_operation": 27446.227,
                "observed_batch": 1191,
                "requested_batch": 1191,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 27565225.746,
            "ns_per_operation": 23144.606,
            "observed_batch": 1191,
            "requested_batch": 1191,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 32688456.356999997,
            "ns_per_operation": 27446.227,
            "observed_batch": 1191,
            "requested_batch": 1191,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 31841438.595,
            "ns_per_operation": 26735.045,
            "observed_batch": 1191,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 25607957.325000003,
            "ns_per_operation": 26814.615,
            "observed_batch": 955,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1191,
        "source_sha256": "9953f19384c51b579921e6f789b13a8b57c86ba86d760b77846e3e33825ece02",
        "wasm_sha256": "3e23be294e7a9eda62c89dbc366b6a6cab90edfbda532da27a7e4fc6d0aa4a56"
      },
      "1:scalar_try_table_normal_n0": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 25840,
            "sides": [
              {
                "calibrated_total_ns": 18797256.32,
                "ns_per_operation": 727.448,
                "observed_batch": 25840,
                "requested_batch": 25840,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 17742260.8,
                "ns_per_operation": 686.62,
                "observed_batch": 25840,
                "requested_batch": 25840,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 32042,
            "sides": [
              {
                "calibrated_total_ns": 21234009.106,
                "ns_per_operation": 662.693,
                "observed_batch": 32042,
                "requested_batch": 32042,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 21028107.214,
                "ns_per_operation": 656.267,
                "observed_batch": 32042,
                "requested_batch": 32042,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 21234009.106,
            "ns_per_operation": 662.693,
            "observed_batch": 32042,
            "requested_batch": 32042,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 21028107.214,
            "ns_per_operation": 656.267,
            "observed_batch": 32042,
            "requested_batch": 32042,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 17479312.959999997,
            "ns_per_operation": 676.444,
            "observed_batch": 25840,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 15278622.5,
            "ns_per_operation": 652.375,
            "observed_batch": 23420,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 32042,
        "source_sha256": "ab92800dd4e4d4053498c317effadedf52568270ab7b476eef74b09638de8964",
        "wasm_sha256": "97068a6310a3bc535369e2b6277b801875c14ca206a707fcb9bca9036f41d0df"
      },
      "1:scalar_try_table_normal_n1": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 13064,
            "sides": [
              {
                "calibrated_total_ns": 20966792.456,
                "ns_per_operation": 1604.929,
                "observed_batch": 13064,
                "requested_batch": 13064,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 19131901.4,
                "ns_per_operation": 1464.475,
                "observed_batch": 13064,
                "requested_batch": 13064,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 15023,
            "sides": [
              {
                "calibrated_total_ns": 26781246.709,
                "ns_per_operation": 1782.683,
                "observed_batch": 15023,
                "requested_batch": 15023,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 37542597.184,
                "ns_per_operation": 2499.008,
                "observed_batch": 15023,
                "requested_batch": 15023,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 26781246.709,
            "ns_per_operation": 1782.683,
            "observed_batch": 15023,
            "requested_batch": 15023,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 37542597.184,
            "ns_per_operation": 2499.008,
            "observed_batch": 15023,
            "requested_batch": 15023,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 22125425.551999997,
            "ns_per_operation": 1693.618,
            "observed_batch": 13064,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 20684321.685000002,
            "ns_per_operation": 1628.815,
            "observed_batch": 12699,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 15023,
        "source_sha256": "ab92800dd4e4d4053498c317effadedf52568270ab7b476eef74b09638de8964",
        "wasm_sha256": "97068a6310a3bc535369e2b6277b801875c14ca206a707fcb9bca9036f41d0df"
      },
      "1:scalar_try_table_normal_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 3,
            "sides": [
              {
                "calibrated_total_ns": 25316874.999,
                "ns_per_operation": 8438958.333,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 31297932.999,
                "ns_per_operation": 10432644.333,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 25316874.999,
            "ns_per_operation": 8438958.333,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 31297932.999,
            "ns_per_operation": 10432644.333,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 24567521.001000002,
            "ns_per_operation": 8189173.667,
            "observed_batch": 3,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22505727.998999998,
            "ns_per_operation": 7501909.333,
            "observed_batch": 3,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 3,
        "source_sha256": "ab92800dd4e4d4053498c317effadedf52568270ab7b476eef74b09638de8964",
        "wasm_sha256": "97068a6310a3bc535369e2b6277b801875c14ca206a707fcb9bca9036f41d0df"
      },
      "1:scalar_try_table_normal_n1024": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 25,
            "sides": [
              {
                "calibrated_total_ns": 17808369.0,
                "ns_per_operation": 712334.76,
                "observed_batch": 25,
                "requested_batch": 25,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 21457633.0,
                "ns_per_operation": 858305.32,
                "observed_batch": 25,
                "requested_batch": 25,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 31,
            "sides": [
              {
                "calibrated_total_ns": 24925099.012000002,
                "ns_per_operation": 804035.452,
                "observed_batch": 31,
                "requested_batch": 31,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 26682889.997,
                "ns_per_operation": 860738.387,
                "observed_batch": 31,
                "requested_batch": 31,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 24925099.012000002,
            "ns_per_operation": 804035.452,
            "observed_batch": 31,
            "requested_batch": 31,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 26682889.997,
            "ns_per_operation": 860738.387,
            "observed_batch": 31,
            "requested_batch": 31,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 19790134.0,
            "ns_per_operation": 791605.36,
            "observed_batch": 25,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 20954762.994,
            "ns_per_operation": 952489.227,
            "observed_batch": 22,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 31,
        "source_sha256": "ab92800dd4e4d4053498c317effadedf52568270ab7b476eef74b09638de8964",
        "wasm_sha256": "97068a6310a3bc535369e2b6277b801875c14ca206a707fcb9bca9036f41d0df"
      },
      "1:scalar_try_table_normal_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 2095,
            "sides": [
              {
                "calibrated_total_ns": 30070165.595,
                "ns_per_operation": 14353.301,
                "observed_batch": 2095,
                "requested_batch": 2095,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 33290620.545,
                "ns_per_operation": 15890.511,
                "observed_batch": 2095,
                "requested_batch": 2095,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 30070165.595,
            "ns_per_operation": 14353.301,
            "observed_batch": 2095,
            "requested_batch": 2095,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 33290620.545,
            "ns_per_operation": 15890.511,
            "observed_batch": 2095,
            "requested_batch": 2095,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 29287456.834999997,
            "ns_per_operation": 13979.693,
            "observed_batch": 2095,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 34706356.625,
            "ns_per_operation": 16725.955,
            "observed_batch": 2075,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 2095,
        "source_sha256": "ab92800dd4e4d4053498c317effadedf52568270ab7b476eef74b09638de8964",
        "wasm_sha256": "97068a6310a3bc535369e2b6277b801875c14ca206a707fcb9bca9036f41d0df"
      },
      "1:scalar_try_table_normal_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1151,
            "sides": [
              {
                "calibrated_total_ns": 28775987.558000002,
                "ns_per_operation": 25000.858,
                "observed_batch": 1151,
                "requested_batch": 1151,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 29409064.031000003,
                "ns_per_operation": 25550.881,
                "observed_batch": 1151,
                "requested_batch": 1151,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 28775987.558000002,
            "ns_per_operation": 25000.858,
            "observed_batch": 1151,
            "requested_batch": 1151,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 29409064.031000003,
            "ns_per_operation": 25550.881,
            "observed_batch": 1151,
            "requested_batch": 1151,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 28638389.072,
            "ns_per_operation": 25165.544,
            "observed_batch": 1138,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 30429306.977999996,
            "ns_per_operation": 26437.278,
            "observed_batch": 1151,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1151,
        "source_sha256": "ab92800dd4e4d4053498c317effadedf52568270ab7b476eef74b09638de8964",
        "wasm_sha256": "97068a6310a3bc535369e2b6277b801875c14ca206a707fcb9bca9036f41d0df"
      }
    },
    "warm_observed_under_20ms_legacy_count": 141,
    "warm_observed_under_20ms_total_count": 562,
    "warm_observed_under_20ms_typed_count": 421,
    "warm_timed_duration_summary": {
      "0:branch_calls:baseline": {
        "batch": 4,
        "calibration_class": "legacy_common_min",
        "estimated_total_ns": {
          "max": 151307917.0,
          "median": 27920961.5,
          "min": 24894396.0,
          "p95": 64006404.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:branch_calls:candidate": {
        "batch": 4,
        "calibration_class": "legacy_common_min",
        "estimated_total_ns": {
          "max": 77935272.0,
          "median": 27479054.0,
          "min": 25199198.0,
          "p95": 66548903.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:call_overhead:baseline": {
        "batch": 44248,
        "calibration_class": "legacy_common_min",
        "estimated_total_ns": {
          "max": 25538795.152,
          "median": 16442977.156,
          "min": 14208652.271999998,
          "p95": 21963424.007999998
        },
        "observations": 30,
        "observed_under_20ms_count": 27
      },
      "0:call_overhead:candidate": {
        "batch": 44248,
        "calibration_class": "legacy_common_min",
        "estimated_total_ns": {
          "max": 21202889.384,
          "median": 16175652.864,
          "min": 14205068.184,
          "p95": 20378947.376000002
        },
        "observations": 30,
        "observed_under_20ms_count": 28
      },
      "0:integer_loop:baseline": {
        "batch": 3,
        "calibration_class": "legacy_common_min",
        "estimated_total_ns": {
          "max": 45608025.0,
          "median": 22081371.0,
          "min": 19365219.0,
          "p95": 30336182.001
        },
        "observations": 30,
        "observed_under_20ms_count": 1
      },
      "0:integer_loop:candidate": {
        "batch": 3,
        "calibration_class": "legacy_common_min",
        "estimated_total_ns": {
          "max": 26169323.001,
          "median": 22002300.0,
          "min": 20449458.998999998,
          "p95": 25698662.001
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_accumulator_n0:baseline": {
        "batch": 32557,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 49026577.033,
          "median": 24738338.779,
          "min": 19255577.194000002,
          "p95": 48109316.114999995
        },
        "observations": 30,
        "observed_under_20ms_count": 1
      },
      "0:scalar_accumulator_n0:candidate": {
        "batch": 32557,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 50157346.75700001,
          "median": 23007250.531999998,
          "min": 18981317.026,
          "p95": 35026220.551
        },
        "observations": 30,
        "observed_under_20ms_count": 2
      },
      "0:scalar_accumulator_n10000:baseline": {
        "batch": 4,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 156534452.0,
          "median": 28449703.0,
          "min": 20621366.0,
          "p95": 56324377.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_accumulator_n10000:candidate": {
        "batch": 4,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 291682789.0,
          "median": 26870214.5,
          "min": 20166669.0,
          "p95": 46123224.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_accumulator_n1024:baseline": {
        "batch": 42,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 47577371.982,
          "median": 25733679.489,
          "min": 24121931.988,
          "p95": 44694582.99599999
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_accumulator_n1024:candidate": {
        "batch": 42,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 32690356.985999998,
          "median": 25466780.01,
          "min": 22503249.006,
          "p95": 32293219.979999997
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_accumulator_n17:baseline": {
        "batch": 2248,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 67467724.58399999,
          "median": 26716595.412,
          "min": 23019083.888,
          "p95": 36168182.152
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_accumulator_n17:candidate": {
        "batch": 2248,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 44347838.128000006,
          "median": 26034782.632,
          "min": 23678723.52,
          "p95": 44149243.064
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_accumulator_n1:baseline": {
        "batch": 16326,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 143813823.858,
          "median": 29520575.244000003,
          "min": 19278067.32,
          "p95": 52523403.138
        },
        "observations": 30,
        "observed_under_20ms_count": 1
      },
      "0:scalar_accumulator_n1:candidate": {
        "batch": 16326,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 48798446.652,
          "median": 28536533.757,
          "min": 19230885.18,
          "p95": 38970015.066
        },
        "observations": 30,
        "observed_under_20ms_count": 1
      },
      "0:scalar_accumulator_n32:baseline": {
        "batch": 1347,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 42348078.417,
          "median": 27642601.935,
          "min": 24024550.506,
          "p95": 41614150.65
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_accumulator_n32:candidate": {
        "batch": 1347,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 81469431.80399999,
          "median": 28366543.044,
          "min": 23637581.652,
          "p95": 66277976.58
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_br_table_n0:baseline": {
        "batch": 31202,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 59929150.966,
          "median": 32478380.213999998,
          "min": 24384706.222,
          "p95": 55852172.838
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_br_table_n0:candidate": {
        "batch": 31202,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 71737391.856,
          "median": 28476115.275,
          "min": 23104737.778,
          "p95": 56532501.246
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_br_table_n10000:baseline": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 799136569.0,
          "median": 32323179.5,
          "min": 24519439.0,
          "p95": 329774904.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_br_table_n10000:candidate": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 106893644.0,
          "median": 30998934.0,
          "min": 22790537.0,
          "p95": 77225007.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_br_table_n1024:baseline": {
        "batch": 19,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 43005535.996999994,
          "median": 27924468.995500002,
          "min": 24369203.008,
          "p95": 39258306.008
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_br_table_n1024:candidate": {
        "batch": 19,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 45118456.001,
          "median": 29487889.9995,
          "min": 24851327.001,
          "p95": 40641636.994
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_br_table_n17:baseline": {
        "batch": 1398,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 158384051.574,
          "median": 43497304.896,
          "min": 31306352.058,
          "p95": 134160287.65799999
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_br_table_n17:candidate": {
        "batch": 1398,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 101166717.36000001,
          "median": 41109768.342,
          "min": 29941493.046,
          "p95": 77280771.756
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_br_table_n1:baseline": {
        "batch": 13325,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 45360312.074999996,
          "median": 29196674.0,
          "min": 26395865.6,
          "p95": 42565766.425000004
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_br_table_n1:candidate": {
        "batch": 13325,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 67120196.975,
          "median": 29849472.4125,
          "min": 25401634.05,
          "p95": 53253482.425
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_br_table_n32:baseline": {
        "batch": 750,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 49242680.25000001,
          "median": 34815637.5,
          "min": 31113356.25,
          "p95": 46927131.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_br_table_n32:candidate": {
        "batch": 750,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 49728546.75000001,
          "median": 35193133.125,
          "min": 31378839.75,
          "p95": 48394838.25
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_branch_entropy_n0:baseline": {
        "batch": 26783,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 34925299.83,
          "median": 22488305.7755,
          "min": 19062987.731,
          "p95": 31031453.375
        },
        "observations": 30,
        "observed_under_20ms_count": 5
      },
      "0:scalar_branch_entropy_n0:candidate": {
        "batch": 26783,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 97609893.576,
          "median": 20958273.3345,
          "min": 18822663.872,
          "p95": 29859322.163000003
        },
        "observations": 30,
        "observed_under_20ms_count": 8
      },
      "0:scalar_branch_entropy_n10000:baseline": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 33926123.0,
          "median": 26220021.0,
          "min": 22745130.0,
          "p95": 31287621.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_branch_entropy_n10000:candidate": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 31667078.0,
          "median": 25678383.0,
          "min": 22423772.0,
          "p95": 29948284.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_branch_entropy_n1024:baseline": {
        "batch": 17,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 50315458.008,
          "median": 27437029.4955,
          "min": 23432171.0,
          "p95": 41023501.006
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_branch_entropy_n1024:candidate": {
        "batch": 17,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 33187358.996,
          "median": 26079045.5,
          "min": 22491852.992,
          "p95": 32421622.998
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_branch_entropy_n17:baseline": {
        "batch": 1428,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 45927128.94,
          "median": 34872563.964,
          "min": 30265022.003999997,
          "p95": 42548582.328
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_branch_entropy_n17:candidate": {
        "batch": 1428,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 77551921.104,
          "median": 35530551.378,
          "min": 29615366.256,
          "p95": 56203433.91599999
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_branch_entropy_n1:baseline": {
        "batch": 9666,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 61551812.08800001,
          "median": 26971073.631,
          "min": 20834212.391999997,
          "p95": 43397092.224
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_branch_entropy_n1:candidate": {
        "batch": 9666,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 51121618.128,
          "median": 24427736.379,
          "min": 20293660.674,
          "p95": 42200721.738
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_branch_entropy_n32:baseline": {
        "batch": 766,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 51085420.134,
          "median": 35948588.352,
          "min": 31471379.632,
          "p95": 42913612.638000004
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_branch_entropy_n32:candidate": {
        "batch": 766,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 66349417.108,
          "median": 38331052.874,
          "min": 33451453.63,
          "p95": 64629321.97799999
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_call_ref_n0:baseline": {
        "batch": 25221,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 65882901.504,
          "median": 26821133.7345,
          "min": 22816051.544999998,
          "p95": 49543754.085
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_call_ref_n0:candidate": {
        "batch": 25221,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 42685836.312,
          "median": 24732242.241,
          "min": 22590701.91,
          "p95": 42048501.642
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_call_ref_n10000:baseline": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 31195792.0,
          "median": 25069493.5,
          "min": 20713218.0,
          "p95": 31033059.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_call_ref_n10000:candidate": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 33621958.0,
          "median": 24114417.5,
          "min": 21262475.0,
          "p95": 32675162.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_call_ref_n1024:baseline": {
        "batch": 16,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 32159119.008,
          "median": 21529033.496,
          "min": 18056318.0,
          "p95": 30885130.0
        },
        "observations": 30,
        "observed_under_20ms_count": 10
      },
      "0:scalar_call_ref_n1024:candidate": {
        "batch": 16,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 71446492.992,
          "median": 20881437.0,
          "min": 18023618.0,
          "p95": 31434680.992
        },
        "observations": 30,
        "observed_under_20ms_count": 10
      },
      "0:scalar_call_ref_n17:baseline": {
        "batch": 1506,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 41955920.562,
          "median": 33016497.33,
          "min": 29197004.226,
          "p95": 41841854.616000004
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_call_ref_n17:candidate": {
        "batch": 1506,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 46224987.798,
          "median": 34015811.16,
          "min": 29630361.75,
          "p95": 43363432.68
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_call_ref_n1:baseline": {
        "batch": 12683,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 48630731.292,
          "median": 27711511.5805,
          "min": 23426566.372,
          "p95": 43128249.791
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_call_ref_n1:candidate": {
        "batch": 12683,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 58852062.456,
          "median": 27162546.950000003,
          "min": 23586321.44,
          "p95": 45613027.053
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_call_ref_n32:baseline": {
        "batch": 449,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 38030845.086,
          "median": 19970084.322499998,
          "min": 17201585.119999997,
          "p95": 35790990.177
        },
        "observations": 30,
        "observed_under_20ms_count": 15
      },
      "0:scalar_call_ref_n32:candidate": {
        "batch": 449,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 126288345.996,
          "median": 20538389.4595,
          "min": 16991792.115000002,
          "p95": 36930857.946
        },
        "observations": 30,
        "observed_under_20ms_count": 12
      },
      "0:scalar_dependent_n0:baseline": {
        "batch": 24661,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 295415154.76199996,
          "median": 34906474.10250001,
          "min": 19312916.895999998,
          "p95": 51511502.224
        },
        "observations": 30,
        "observed_under_20ms_count": 1
      },
      "0:scalar_dependent_n0:candidate": {
        "batch": 24661,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 164665122.167,
          "median": 31755957.369499996,
          "min": 18340089.768,
          "p95": 107504278.06300001
        },
        "observations": 30,
        "observed_under_20ms_count": 1
      },
      "0:scalar_dependent_n10000:baseline": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 75047298.0,
          "median": 21151201.5,
          "min": 18090842.001000002,
          "p95": 28664076.0
        },
        "observations": 30,
        "observed_under_20ms_count": 13
      },
      "0:scalar_dependent_n10000:candidate": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 94471772.001,
          "median": 20522901.0,
          "min": 17955882.0,
          "p95": 68834238.999
        },
        "observations": 30,
        "observed_under_20ms_count": 13
      },
      "0:scalar_dependent_n1024:baseline": {
        "batch": 30,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 51084534.99,
          "median": 29689346.505000003,
          "min": 20476539.0,
          "p95": 45532449.99
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_dependent_n1024:candidate": {
        "batch": 30,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 53617305.99,
          "median": 27910161.0,
          "min": 21067319.009999998,
          "p95": 40795838.01
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_dependent_n17:baseline": {
        "batch": 2380,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 92654064.02000001,
          "median": 37926559.019999996,
          "min": 30219121.8,
          "p95": 55911112.31999999
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_dependent_n17:candidate": {
        "batch": 2380,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 67946603.34,
          "median": 38047964.01,
          "min": 30550474.92,
          "p95": 66393194.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_dependent_n1:baseline": {
        "batch": 16300,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 69419206.1,
          "median": 29967851.549999997,
          "min": 22519558.4,
          "p95": 63802910.7
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_dependent_n1:candidate": {
        "batch": 16300,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 55302183.6,
          "median": 27712991.049999997,
          "min": 21741869.099999998,
          "p95": 49577656.2
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_dependent_n32:baseline": {
        "batch": 1306,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 167102176.294,
          "median": 43996011.477,
          "min": 26358039.395999998,
          "p95": 132141897.55600001
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_dependent_n32:candidate": {
        "batch": 1306,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 562059748.652,
          "median": 39637969.796000004,
          "min": 26984702.599999998,
          "p95": 87983152.602
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_direct_calls_n0:baseline": {
        "batch": 24510,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 40584785.46,
          "median": 21624204.855,
          "min": 17350212.330000002,
          "p95": 27657574.200000003
        },
        "observations": 30,
        "observed_under_20ms_count": 11
      },
      "0:scalar_direct_calls_n0:candidate": {
        "batch": 24510,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 31706454.630000003,
          "median": 21414534.060000002,
          "min": 14286437.819999998,
          "p95": 27425611.56
        },
        "observations": 30,
        "observed_under_20ms_count": 11
      },
      "0:scalar_direct_calls_n10000:baseline": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 47052290.001,
          "median": 32295576.4995,
          "min": 22016846.001000002,
          "p95": 46969587.999
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_direct_calls_n10000:candidate": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 99146328.0,
          "median": 30379689.0,
          "min": 24154284.0,
          "p95": 48557699.001
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_direct_calls_n1024:baseline": {
        "batch": 17,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 35993445.007,
          "median": 22059599.993,
          "min": 17310456.992,
          "p95": 28927572.997
        },
        "observations": 30,
        "observed_under_20ms_count": 7
      },
      "0:scalar_direct_calls_n1024:candidate": {
        "batch": 17,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 39399306.998,
          "median": 23174070.4975,
          "min": 18167413.001,
          "p95": 34375520.004
        },
        "observations": 30,
        "observed_under_20ms_count": 3
      },
      "0:scalar_direct_calls_n17:baseline": {
        "batch": 1588,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 57002211.212000005,
          "median": 32112067.54,
          "min": 24734824.568,
          "p95": 56890293.736
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_direct_calls_n17:candidate": {
        "batch": 1588,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 60107878.692,
          "median": 34197582.382,
          "min": 22138401.691999998,
          "p95": 49919486.66
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_direct_calls_n1:baseline": {
        "batch": 10006,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 70629862.506,
          "median": 24725296.281999998,
          "min": 18438156.26,
          "p95": 50510257.982
        },
        "observations": 30,
        "observed_under_20ms_count": 1
      },
      "0:scalar_direct_calls_n1:candidate": {
        "batch": 10006,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 34447916.356,
          "median": 24026942.521,
          "min": 16679391.634000001,
          "p95": 33428004.775999997
        },
        "observations": 30,
        "observed_under_20ms_count": 5
      },
      "0:scalar_direct_calls_n32:baseline": {
        "batch": 908,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 50812669.72,
          "median": 33595486.072,
          "min": 25828156.248,
          "p95": 47303523.936
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_direct_calls_n32:candidate": {
        "batch": 908,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 50017319.832,
          "median": 34138476.882,
          "min": 27537056.74,
          "p95": 43681636.332
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_dynamic_api_n0:baseline": {
        "batch": 49505,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 33026419.165000003,
          "median": 21371110.48,
          "min": 17674869.16,
          "p95": 29287554.04
        },
        "observations": 30,
        "observed_under_20ms_count": 9
      },
      "0:scalar_dynamic_api_n0:candidate": {
        "batch": 49505,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 71160170.17,
          "median": 21352966.8975,
          "min": 18538533.39,
          "p95": 30761862.445
        },
        "observations": 30,
        "observed_under_20ms_count": 7
      },
      "0:scalar_dynamic_api_n10000:baseline": {
        "batch": 63236,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 59322956.32,
          "median": 23825332.866,
          "min": 19586528.932,
          "p95": 34898873.388000004
        },
        "observations": 30,
        "observed_under_20ms_count": 1
      },
      "0:scalar_dynamic_api_n10000:candidate": {
        "batch": 63236,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 30917598.064,
          "median": 23093565.874,
          "min": 20717441.555999998,
          "p95": 27768634.972
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_dynamic_api_n1024:baseline": {
        "batch": 64612,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 53419069.40400001,
          "median": 23878139.944,
          "min": 20940619.976,
          "p95": 47157778.932
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_dynamic_api_n1024:candidate": {
        "batch": 64612,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 51581891.796,
          "median": 26215479.044,
          "min": 20514245.388,
          "p95": 47761448.848
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_dynamic_api_n17:baseline": {
        "batch": 60455,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 165987908.01999998,
          "median": 29323244.3375,
          "min": 20642359.75,
          "p95": 162102827.9
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_dynamic_api_n17:candidate": {
        "batch": 60455,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 540649790.46,
          "median": 30018144.335,
          "min": 19982856.155,
          "p95": 75347061.515
        },
        "observations": 30,
        "observed_under_20ms_count": 1
      },
      "0:scalar_dynamic_api_n1:baseline": {
        "batch": 68591,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 56256143.288,
          "median": 23566221.416,
          "min": 20639169.082,
          "p95": 35783307.381000005
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_dynamic_api_n1:candidate": {
        "batch": 68591,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 39642305.632,
          "median": 23766952.9775,
          "min": 21339963.329000004,
          "p95": 27017720.536000002
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_dynamic_api_n32:baseline": {
        "batch": 66257,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 44317783.389,
          "median": 23535977.1825,
          "min": 20841073.093,
          "p95": 35811179.673
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_dynamic_api_n32:candidate": {
        "batch": 66257,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 31095271.441,
          "median": 23680086.1575,
          "min": 21219334.305999998,
          "p95": 30722575.816
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_host_call_n0:baseline": {
        "batch": 28090,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 43936383.61,
          "median": 28108272.545,
          "min": 20350334.21,
          "p95": 41005388.74
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_host_call_n0:candidate": {
        "batch": 28090,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 88519286.66,
          "median": 27711459.16,
          "min": 20929915.18,
          "p95": 59287793.330000006
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_host_call_n10000:baseline": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 101031753.0,
          "median": 36926082.4995,
          "min": 27332307.0,
          "p95": 59766372.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_host_call_n10000:candidate": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 68595426.0,
          "median": 38478781.0005,
          "min": 27799281.0,
          "p95": 65150441.001
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_host_call_n1024:baseline": {
        "batch": 33,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 60553751.016,
          "median": 30256954.001999997,
          "min": 26216645.003999997,
          "p95": 47403530.988
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_host_call_n1024:candidate": {
        "batch": 33,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 74474605.986,
          "median": 29484153.996,
          "min": 27168468.987,
          "p95": 42504109.989
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_host_call_n17:baseline": {
        "batch": 1868,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 81243463.224,
          "median": 42013093.66599999,
          "min": 36213804.54,
          "p95": 54388609.544
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_host_call_n17:candidate": {
        "batch": 1868,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 61344172.924,
          "median": 42866383.923999995,
          "min": 35125877.604,
          "p95": 54450989.536
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_host_call_n1:baseline": {
        "batch": 14145,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 54240289.695,
          "median": 27923793.0225,
          "min": 23963044.5,
          "p95": 46278833.025
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_host_call_n1:candidate": {
        "batch": 14145,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 39037767.06,
          "median": 26917751.115,
          "min": 23997586.59,
          "p95": 34670908.515
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_host_call_n32:baseline": {
        "batch": 1169,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 95877252.26099999,
          "median": 39679210.9385,
          "min": 32118680.643000003,
          "p95": 93560655.48200001
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_host_call_n32:candidate": {
        "batch": 1169,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 82203660.32900001,
          "median": 37952547.087,
          "min": 32835406.233,
          "p95": 81896717.168
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_indirect_calls_n0:baseline": {
        "batch": 28242,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 41584904.658,
          "median": 24273801.306,
          "min": 20954801.466,
          "p95": 32255470.619999997
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_indirect_calls_n0:candidate": {
        "batch": 28242,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 68694598.15200001,
          "median": 23435847.045,
          "min": 21165430.302,
          "p95": 31054677.264000002
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_indirect_calls_n10000:baseline": {
        "batch": 1,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 24373897.0,
          "median": 10708473.0,
          "min": 9588586.0,
          "p95": 21496236.0
        },
        "observations": 30,
        "observed_under_20ms_count": 28
      },
      "0:scalar_indirect_calls_n10000:candidate": {
        "batch": 1,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 23135066.0,
          "median": 10774034.0,
          "min": 9553895.0,
          "p95": 21900766.0
        },
        "observations": 30,
        "observed_under_20ms_count": 28
      },
      "0:scalar_indirect_calls_n1024:baseline": {
        "batch": 22,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 69133041.01,
          "median": 28892565.504,
          "min": 23560864.008,
          "p95": 48209484.004
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_indirect_calls_n1024:candidate": {
        "batch": 22,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 39524135.002000004,
          "median": 28112887.506,
          "min": 23831744.992,
          "p95": 36077993.996
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_indirect_calls_n17:baseline": {
        "batch": 1574,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 49034933.754,
          "median": 34368710.258,
          "min": 29988586.206,
          "p95": 47286419.652
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_indirect_calls_n17:candidate": {
        "batch": 1574,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 55490745.122,
          "median": 35781102.956,
          "min": 29488844.354000002,
          "p95": 46520032.756000005
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_indirect_calls_n1:baseline": {
        "batch": 13387,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 86136804.094,
          "median": 37600635.8475,
          "min": 22651982.055999998,
          "p95": 64859131.458000004
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_indirect_calls_n1:candidate": {
        "batch": 13387,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 62615470.258,
          "median": 37077833.336500004,
          "min": 23768658.661,
          "p95": 61123944.265999995
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_indirect_calls_n32:baseline": {
        "batch": 847,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 68422149.02600001,
          "median": 38448471.523,
          "min": 32134754.805999998,
          "p95": 67167713.228
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_indirect_calls_n32:candidate": {
        "batch": 847,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 60764050.193,
          "median": 36852759.097,
          "min": 31351887.104999997,
          "p95": 46574032.197
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_multivalue_branch_n0:baseline": {
        "batch": 25126,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 229260478.432,
          "median": 23997277.265,
          "min": 18004889.584,
          "p95": 93605706.952
        },
        "observations": 30,
        "observed_under_20ms_count": 4
      },
      "0:scalar_multivalue_branch_n0:candidate": {
        "batch": 25126,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 83446360.616,
          "median": 28843605.270999998,
          "min": 17894284.932,
          "p95": 57649245.155999996
        },
        "observations": 30,
        "observed_under_20ms_count": 6
      },
      "0:scalar_multivalue_branch_n10000:baseline": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 69318649.0,
          "median": 37454541.5,
          "min": 30664045.0,
          "p95": 59237861.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_multivalue_branch_n10000:candidate": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 66670955.0,
          "median": 35600568.5,
          "min": 30411173.0,
          "p95": 48767665.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_multivalue_branch_n1024:baseline": {
        "batch": 14,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 50069502.994,
          "median": 21909973.001000002,
          "min": 19441317.0,
          "p95": 39687097.002000004
        },
        "observations": 30,
        "observed_under_20ms_count": 2
      },
      "0:scalar_multivalue_branch_n1024:candidate": {
        "batch": 14,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 160736566.004,
          "median": 22760538.5,
          "min": 18873834.994,
          "p95": 122279595.99800001
        },
        "observations": 30,
        "observed_under_20ms_count": 3
      },
      "0:scalar_multivalue_branch_n17:baseline": {
        "batch": 1219,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 167705219.578,
          "median": 55869592.5945,
          "min": 42728932.893,
          "p95": 133588093.816
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_multivalue_branch_n17:candidate": {
        "batch": 1219,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 108885001.523,
          "median": 57410685.916999996,
          "min": 37144324.536,
          "p95": 81641932.566
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_multivalue_branch_n1:baseline": {
        "batch": 11642,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 40658183.182000004,
          "median": 31497605.630000003,
          "min": 27253863.79,
          "p95": 36681206.129999995
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_multivalue_branch_n1:candidate": {
        "batch": 11642,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 68623035.554,
          "median": 32573291.504,
          "min": 27632205.506,
          "p95": 47604789.952
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_multivalue_branch_n32:baseline": {
        "batch": 662,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 130164741.112,
          "median": 36204750.21,
          "min": 32967842.292,
          "p95": 51001463.732
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_multivalue_branch_n32:candidate": {
        "batch": 662,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 53915164.713999994,
          "median": 36807767.996,
          "min": 32094557.048,
          "p95": 52171567.268
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_ordinary_recursion_n0:baseline": {
        "batch": 24898,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 127871173.29800001,
          "median": 28317304.585,
          "min": 22659620.004,
          "p95": 59583403.8
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_ordinary_recursion_n0:candidate": {
        "batch": 24898,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 61766784.114,
          "median": 26450577.034999996,
          "min": 22514688.746,
          "p95": 57000460.382
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_ordinary_recursion_n17:baseline": {
        "batch": 1836,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 71752014.648,
          "median": 41065711.938,
          "min": 33201460.224,
          "p95": 66156885.432
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_ordinary_recursion_n17:candidate": {
        "batch": 1836,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 67438969.74,
          "median": 40079503.620000005,
          "min": 31783489.884,
          "p95": 55007378.856
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_ordinary_recursion_n1:baseline": {
        "batch": 15152,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 53242264.304,
          "median": 32144914.968000002,
          "min": 24997542.32,
          "p95": 52334220.096
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_ordinary_recursion_n1:candidate": {
        "batch": 15152,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 49894248.08,
          "median": 31249537.832000002,
          "min": 25979891.936,
          "p95": 47111583.28
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_ordinary_recursion_n32:baseline": {
        "batch": 906,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 185128423.248,
          "median": 33097562.103,
          "min": 27925011.048,
          "p95": 125110963.32599999
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_ordinary_recursion_n32:candidate": {
        "batch": 906,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 62741357.076,
          "median": 32384148.711000003,
          "min": 26250842.64,
          "p95": 46118834.646
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_table_mutation_n0:baseline": {
        "batch": 27142,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 70115494.32800001,
          "median": 27535735.423,
          "min": 22967234.696,
          "p95": 46691459.772
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_table_mutation_n0:candidate": {
        "batch": 27142,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 55185114.4,
          "median": 27350898.403,
          "min": 22267459.652,
          "p95": 54848092.186
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_table_mutation_n10000:baseline": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 41673393.0,
          "median": 25680368.0,
          "min": 20436170.0,
          "p95": 39612877.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_table_mutation_n10000:candidate": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 56179257.0,
          "median": 25923961.5,
          "min": 22142560.0,
          "p95": 41177860.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_table_mutation_n1024:baseline": {
        "batch": 23,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 61359374.010000005,
          "median": 26861451.508,
          "min": 23066007.01,
          "p95": 45007320.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_table_mutation_n1024:candidate": {
        "batch": 23,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 59480069.011,
          "median": 26803665.503,
          "min": 23313684.005000003,
          "p95": 54484394.997
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_table_mutation_n17:baseline": {
        "batch": 1751,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 102308112.641,
          "median": 38810507.017000005,
          "min": 35184317.342,
          "p95": 83523111.485
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_table_mutation_n17:candidate": {
        "batch": 1751,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 79440476.383,
          "median": 41102017.953999996,
          "min": 35233084.442999996,
          "p95": 77359838.37599999
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_table_mutation_n1:baseline": {
        "batch": 13343,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 272580357.588,
          "median": 34525039.186000004,
          "min": 25024823.186,
          "p95": 247846118.25599998
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_table_mutation_n1:candidate": {
        "batch": 13343,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 69444110.505,
          "median": 30888924.913000003,
          "min": 24845159.691,
          "p95": 52089390.782
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_table_mutation_n32:baseline": {
        "batch": 949,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 97861770.162,
          "median": 37531792.22,
          "min": 25978888.286,
          "p95": 62363728.596
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_table_mutation_n32:candidate": {
        "batch": 949,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 101022879.672,
          "median": 38906062.845,
          "min": 31723767.023000002,
          "p95": 95359021.81
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_tail_calls_n0:baseline": {
        "batch": 27135,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 59102770.635000005,
          "median": 24858088.5825,
          "min": 19632633.795,
          "p95": 51920923.050000004
        },
        "observations": 30,
        "observed_under_20ms_count": 1
      },
      "0:scalar_tail_calls_n0:candidate": {
        "batch": 27135,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 63369613.71,
          "median": 25552188.314999998,
          "min": 20140058.294999998,
          "p95": 41852779.785000004
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_tail_calls_n10000:baseline": {
        "batch": 4,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 51125651.0,
          "median": 29895807.5,
          "min": 22959426.0,
          "p95": 49613448.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_tail_calls_n10000:candidate": {
        "batch": 4,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 56583723.0,
          "median": 29656915.5,
          "min": 23619170.0,
          "p95": 45736735.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_tail_calls_n1024:baseline": {
        "batch": 32,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 41861808.992,
          "median": 25179040.496,
          "min": 20732720.0,
          "p95": 34761718.016
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_tail_calls_n1024:candidate": {
        "batch": 32,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 33025156.0,
          "median": 24984453.008,
          "min": 20783487.008,
          "p95": 29144560.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_tail_calls_n17:baseline": {
        "batch": 1926,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 54953772.192,
          "median": 26977068.873,
          "min": 22925328.228,
          "p95": 32539240.349999998
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_tail_calls_n17:candidate": {
        "batch": 1926,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 51732554.526,
          "median": 26652952.926,
          "min": 23163164.19,
          "p95": 37622200.878
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_tail_calls_n1:baseline": {
        "batch": 10427,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 41035917.287999995,
          "median": 21081173.049000002,
          "min": 17520498.527,
          "p95": 25827522.595000003
        },
        "observations": 30,
        "observed_under_20ms_count": 8
      },
      "0:scalar_tail_calls_n1:candidate": {
        "batch": 10427,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 33976160.533,
          "median": 21798394.244000003,
          "min": 18744357.224999998,
          "p95": 30053961.066999998
        },
        "observations": 30,
        "observed_under_20ms_count": 6
      },
      "0:scalar_tail_calls_n32:baseline": {
        "batch": 1036,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 45700497.164,
          "median": 26007288.678,
          "min": 22575795.088,
          "p95": 35430328.724
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_tail_calls_n32:candidate": {
        "batch": 1036,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 39402934.956,
          "median": 27135857.868,
          "min": 24360662.507999998,
          "p95": 37351510.952
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_try_table_normal_n0:baseline": {
        "batch": 27740,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 36199146.56,
          "median": 20074841.589999996,
          "min": 17203793.2,
          "p95": 30423595.34
        },
        "observations": 30,
        "observed_under_20ms_count": 15
      },
      "0:scalar_try_table_normal_n0:candidate": {
        "batch": 27740,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 32700744.2,
          "median": 19373935.009999998,
          "min": 16603943.440000001,
          "p95": 32355714.080000002
        },
        "observations": 30,
        "observed_under_20ms_count": 17
      },
      "0:scalar_try_table_normal_n10000:baseline": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 109803549.0,
          "median": 28364261.001,
          "min": 18624108.998999998,
          "p95": 51150558.999
        },
        "observations": 30,
        "observed_under_20ms_count": 1
      },
      "0:scalar_try_table_normal_n10000:candidate": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 59658296.001,
          "median": 27216328.5,
          "min": 23922504.998999998,
          "p95": 37258203.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_try_table_normal_n1024:baseline": {
        "batch": 26,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 28769416.0,
          "median": 23381675.993,
          "min": 20950592.987999998,
          "p95": 28375776.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_try_table_normal_n1024:candidate": {
        "batch": 26,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 40101656.997999996,
          "median": 23288215.496,
          "min": 21260932.004,
          "p95": 37104840.006
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_try_table_normal_n17:baseline": {
        "batch": 2065,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 49112586.775,
          "median": 32747962.537499998,
          "min": 28884470.405,
          "p95": 43220086.56
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_try_table_normal_n17:candidate": {
        "batch": 2065,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 64201859.785000004,
          "median": 33230415.715000004,
          "min": 28506424.66,
          "p95": 41638837.589999996
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_try_table_normal_n1:baseline": {
        "batch": 15338,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 84799630.064,
          "median": 31086360.521000005,
          "min": 21404991.914,
          "p95": 76856478.65200001
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_try_table_normal_n1:candidate": {
        "batch": 15338,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 84547120.57000001,
          "median": 32113546.380999997,
          "min": 21852370.698,
          "p95": 81872081.34200001
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_try_table_normal_n32:baseline": {
        "batch": 1138,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 58487498.62,
          "median": 33990366.087,
          "min": 29060192.186,
          "p95": 55134492.006
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_try_table_normal_n32:candidate": {
        "batch": 1138,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 56200407.672,
          "median": 33386335.601999998,
          "min": 29595269.544,
          "p95": 48734953.558
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:branch_calls:baseline": {
        "batch": 3,
        "calibration_class": "legacy_common_min",
        "estimated_total_ns": {
          "max": 38495289.999,
          "median": 21034453.9995,
          "min": 18219398.001000002,
          "p95": 36126822.0
        },
        "observations": 30,
        "observed_under_20ms_count": 7
      },
      "1:branch_calls:candidate": {
        "batch": 3,
        "calibration_class": "legacy_common_min",
        "estimated_total_ns": {
          "max": 37360827.0,
          "median": 21171986.5005,
          "min": 17825262.0,
          "p95": 33747842.001
        },
        "observations": 30,
        "observed_under_20ms_count": 8
      },
      "1:call_overhead:baseline": {
        "batch": 50633,
        "calibration_class": "legacy_common_min",
        "estimated_total_ns": {
          "max": 42458504.682,
          "median": 18395044.8495,
          "min": 16604535.387000002,
          "p95": 28062833.92
        },
        "observations": 30,
        "observed_under_20ms_count": 22
      },
      "1:call_overhead:candidate": {
        "batch": 50633,
        "calibration_class": "legacy_common_min",
        "estimated_total_ns": {
          "max": 27811896.772,
          "median": 18698893.4825,
          "min": 16486965.561,
          "p95": 27465313.887
        },
        "observations": 30,
        "observed_under_20ms_count": 19
      },
      "1:integer_loop:baseline": {
        "batch": 3,
        "calibration_class": "legacy_common_min",
        "estimated_total_ns": {
          "max": 48331445.001,
          "median": 20490370.5,
          "min": 18070650.0,
          "p95": 35284110.999
        },
        "observations": 30,
        "observed_under_20ms_count": 14
      },
      "1:integer_loop:candidate": {
        "batch": 3,
        "calibration_class": "legacy_common_min",
        "estimated_total_ns": {
          "max": 25645224.999,
          "median": 20042133.0,
          "min": 17869427.001000002,
          "p95": 24643656.0
        },
        "observations": 30,
        "observed_under_20ms_count": 15
      },
      "1:scalar_accumulator_n0:baseline": {
        "batch": 31083,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 45846989.838,
          "median": 22630226.814000003,
          "min": 18983724.669,
          "p95": 36074245.974
        },
        "observations": 30,
        "observed_under_20ms_count": 1
      },
      "1:scalar_accumulator_n0:candidate": {
        "batch": 31083,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 35328378.306,
          "median": 22092863.91,
          "min": 19445742.380999997,
          "p95": 29092289.265
        },
        "observations": 30,
        "observed_under_20ms_count": 4
      },
      "1:scalar_accumulator_n10000:baseline": {
        "batch": 4,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 50358281.0,
          "median": 27317249.0,
          "min": 21346319.0,
          "p95": 39931917.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_accumulator_n10000:candidate": {
        "batch": 4,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 40818349.0,
          "median": 25595899.5,
          "min": 22499999.0,
          "p95": 39282657.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_accumulator_n1024:baseline": {
        "batch": 36,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 41521395.996,
          "median": 26349425.009999998,
          "min": 21798108.0,
          "p95": 34423263.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_accumulator_n1024:candidate": {
        "batch": 36,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 113365575.0,
          "median": 25550730.486,
          "min": 21379336.992,
          "p95": 33992057.988
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_accumulator_n17:baseline": {
        "batch": 2250,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 36411799.5,
          "median": 26788011.75,
          "min": 23171865.75,
          "p95": 35123519.25
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_accumulator_n17:candidate": {
        "batch": 2250,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 42841320.75,
          "median": 27379107.0,
          "min": 23057124.75,
          "p95": 40198547.25
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_accumulator_n1:baseline": {
        "batch": 14809,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 46427088.731000006,
          "median": 21310913.663500004,
          "min": 19103965.415999997,
          "p95": 29968469.794
        },
        "observations": 30,
        "observed_under_20ms_count": 6
      },
      "1:scalar_accumulator_n1:candidate": {
        "batch": 14809,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 28719834.15,
          "median": 20351357.104000002,
          "min": 18853560.035,
          "p95": 25367328.303000003
        },
        "observations": 30,
        "observed_under_20ms_count": 10
      },
      "1:scalar_accumulator_n32:baseline": {
        "batch": 1332,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 50611568.436000004,
          "median": 30849507.611999996,
          "min": 26494858.62,
          "p95": 39077879.004
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_accumulator_n32:candidate": {
        "batch": 1332,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 47379462.444,
          "median": 31330433.538,
          "min": 26147868.623999998,
          "p95": 40753883.988
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_br_table_n0:baseline": {
        "batch": 31646,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 46808674.564,
          "median": 26190751.759000003,
          "min": 23667252.25,
          "p95": 32854624.032
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_br_table_n0:candidate": {
        "batch": 31646,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 36959996.32,
          "median": 26570313.883,
          "min": 23193131.878000002,
          "p95": 34073058.324
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_br_table_n10000:baseline": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 86425043.0,
          "median": 30342935.5,
          "min": 26853414.0,
          "p95": 41958572.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_br_table_n10000:candidate": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 36213830.0,
          "median": 29962910.5,
          "min": 26229922.0,
          "p95": 34396565.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_br_table_n1024:baseline": {
        "batch": 15,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 40828216.995,
          "median": 26220978.0,
          "min": 22655865.0,
          "p95": 37665852.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_br_table_n1024:candidate": {
        "batch": 15,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 50398246.004999995,
          "median": 26039266.995,
          "min": 22266003.0,
          "p95": 31958170.995
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_br_table_n17:baseline": {
        "batch": 1339,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 53230663.577,
          "median": 43444343.735,
          "min": 37537105.554000005,
          "p95": 51059396.076
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_br_table_n17:candidate": {
        "batch": 1339,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 89926633.43300001,
          "median": 43698665.361,
          "min": 38913888.117,
          "p95": 76282800.54200001
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_br_table_n1:baseline": {
        "batch": 14155,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 64031671.24,
          "median": 30711162.192500003,
          "min": 26767586.270000003,
          "p95": 43193699.4
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_br_table_n1:candidate": {
        "batch": 14155,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 41114329.9,
          "median": 30033307.5525,
          "min": 24947946.865,
          "p95": 40894332.89
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_br_table_n32:baseline": {
        "batch": 761,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 45953631.85,
          "median": 35794075.9995,
          "min": 32073216.345,
          "p95": 44981080.699
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_br_table_n32:candidate": {
        "batch": 761,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 61922834.828,
          "median": 35392981.817499995,
          "min": 31129557.32,
          "p95": 42637244.076
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_branch_entropy_n0:baseline": {
        "batch": 28363,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 50020277.725,
          "median": 22614075.167,
          "min": 19811186.781,
          "p95": 29261398.025
        },
        "observations": 30,
        "observed_under_20ms_count": 1
      },
      "1:scalar_branch_entropy_n0:candidate": {
        "batch": 28363,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 32495347.284999996,
          "median": 22372720.2185,
          "min": 19454351.878,
          "p95": 27152977.694
        },
        "observations": 30,
        "observed_under_20ms_count": 1
      },
      "1:scalar_branch_entropy_n10000:baseline": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 50505227.0,
          "median": 28538822.0,
          "min": 22683461.0,
          "p95": 43365062.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_branch_entropy_n10000:candidate": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 297017816.0,
          "median": 28744489.0,
          "min": 23061351.0,
          "p95": 78997625.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_branch_entropy_n1024:baseline": {
        "batch": 16,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 31112111.008,
          "median": 24842722.0,
          "min": 21843100.992,
          "p95": 30218480.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_branch_entropy_n1024:candidate": {
        "batch": 16,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 49130659.008,
          "median": 24357877.504,
          "min": 21722152.992,
          "p95": 31657299.008
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_branch_entropy_n17:baseline": {
        "batch": 1425,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 43534776.0,
          "median": 30681585.225,
          "min": 28908041.625,
          "p95": 37311877.95
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_branch_entropy_n17:candidate": {
        "batch": 1425,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 61420991.24999999,
          "median": 31511464.612500004,
          "min": 27485174.85,
          "p95": 46324055.325
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_branch_entropy_n1:baseline": {
        "batch": 12611,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 86660143.69,
          "median": 33684592.633499995,
          "min": 29684742.847,
          "p95": 54717161.684
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_branch_entropy_n1:candidate": {
        "batch": 12611,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 66143421.289000005,
          "median": 35013798.339,
          "min": 29961163.356,
          "p95": 61123045.244
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_branch_entropy_n32:baseline": {
        "batch": 776,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 58631369.864,
          "median": 38018306.1,
          "min": 34137282.168,
          "p95": 46352534.072
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_branch_entropy_n32:candidate": {
        "batch": 776,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 57367660.072,
          "median": 38220963.544,
          "min": 34091735.624,
          "p95": 44572661.672000006
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_call_ref_n0:baseline": {
        "batch": 24039,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 30258898.937999997,
          "median": 20695211.1585,
          "min": 18253774.26,
          "p95": 28295585.729999997
        },
        "observations": 30,
        "observed_under_20ms_count": 10
      },
      "1:scalar_call_ref_n0:candidate": {
        "batch": 24039,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 32986003.292999998,
          "median": 19426721.2065,
          "min": 17291012.31,
          "p95": 28632876.939000003
        },
        "observations": 30,
        "observed_under_20ms_count": 18
      },
      "1:scalar_call_ref_n10000:baseline": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 475378977.0,
          "median": 26004494.0,
          "min": 20684762.0,
          "p95": 56343974.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_call_ref_n10000:candidate": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 219019998.0,
          "median": 25844889.5,
          "min": 20441818.0,
          "p95": 36222763.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_call_ref_n1024:baseline": {
        "batch": 21,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 91658112.0,
          "median": 24430736.9985,
          "min": 21906862.992000002,
          "p95": 34078447.997999996
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_call_ref_n1024:candidate": {
        "batch": 21,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 55395662.007,
          "median": 24713626.496999998,
          "min": 20940185.007,
          "p95": 28145463.003
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_call_ref_n17:baseline": {
        "batch": 1555,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 49296426.510000005,
          "median": 33911840.4125,
          "min": 29400498.515,
          "p95": 45929800.195
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_call_ref_n17:candidate": {
        "batch": 1555,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 48942984.339999996,
          "median": 32789394.7625,
          "min": 30203520.73,
          "p95": 47766731.025
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_call_ref_n1:baseline": {
        "batch": 13514,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 35429072.698,
          "median": 29432397.366,
          "min": 25955853.296,
          "p95": 35296608.47
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_call_ref_n1:candidate": {
        "batch": 13514,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 38262485.508,
          "median": 29568929.308,
          "min": 25306262.344,
          "p95": 37189730.673999995
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_call_ref_n32:baseline": {
        "batch": 784,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 57980665.12,
          "median": 29770254.976,
          "min": 27222182.847999997,
          "p95": 49761249.104
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_call_ref_n32:candidate": {
        "batch": 784,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 82991749.23200001,
          "median": 28777279.384000003,
          "min": 26057826.816,
          "p95": 31379880.672
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_dependent_n0:baseline": {
        "batch": 27140,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 32982563.500000004,
          "median": 24185064.65,
          "min": 21171235.5,
          "p95": 31734367.76
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_dependent_n0:candidate": {
        "batch": 27140,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 57103591.32,
          "median": 23594091.150000002,
          "min": 20686922.2,
          "p95": 31758766.62
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_dependent_n10000:baseline": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 32368763.001,
          "median": 24816328.0005,
          "min": 21533487.0,
          "p95": 32024459.001
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_dependent_n10000:candidate": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 30956673.999,
          "median": 24211000.0005,
          "min": 21991092.0,
          "p95": 29706909.999
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_dependent_n1024:baseline": {
        "batch": 34,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 42739586.984,
          "median": 28758681.992,
          "min": 24414433.992,
          "p95": 33813645.014
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_dependent_n1024:candidate": {
        "batch": 34,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 33626857.99,
          "median": 27908925.5,
          "min": 23571174.016000003,
          "p95": 32917032.985999998
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_dependent_n17:baseline": {
        "batch": 2350,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 46131331.9,
          "median": 36108659.45,
          "min": 31209645.0,
          "p95": 45705504.85
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_dependent_n17:candidate": {
        "batch": 2350,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 55253907.35,
          "median": 34960436.525,
          "min": 31082545.25,
          "p95": 44200508.449999996
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_dependent_n1:baseline": {
        "batch": 15445,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 36179526.375,
          "median": 25917397.302500002,
          "min": 22615912.715,
          "p95": 35916096.455
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_dependent_n1:candidate": {
        "batch": 15445,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 46235039.95999999,
          "median": 24588880.182499997,
          "min": 22511257.395,
          "p95": 40440740.095
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_dependent_n32:baseline": {
        "batch": 1318,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 54166055.562,
          "median": 34193860.523,
          "min": 31153341.94,
          "p95": 41679243.164000005
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_dependent_n32:candidate": {
        "batch": 1318,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 60284903.512,
          "median": 32660326.665,
          "min": 29668106.191999998,
          "p95": 39311653.864
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_direct_calls_n0:baseline": {
        "batch": 23015,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 28398254.53,
          "median": 22822698.1675,
          "min": 19908665.45,
          "p95": 28109485.325
        },
        "observations": 30,
        "observed_under_20ms_count": 1
      },
      "1:scalar_direct_calls_n0:candidate": {
        "batch": 23015,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 31916879.790000003,
          "median": 22598163.8275,
          "min": 19951703.5,
          "p95": 28054134.25
        },
        "observations": 30,
        "observed_under_20ms_count": 2
      },
      "1:scalar_direct_calls_n10000:baseline": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 47714765.0,
          "median": 27547195.0,
          "min": 23518481.0,
          "p95": 40488557.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_direct_calls_n10000:candidate": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 34331069.0,
          "median": 26596231.5,
          "min": 23897576.0,
          "p95": 33113593.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_direct_calls_n1024:baseline": {
        "batch": 20,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 48119115.0,
          "median": 27746500.5,
          "min": 22226766.0,
          "p95": 34659985.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_direct_calls_n1024:candidate": {
        "batch": 20,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 36690751.0,
          "median": 24295902.0,
          "min": 22289853.0,
          "p95": 35079005.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_direct_calls_n17:baseline": {
        "batch": 1690,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 72882165.98,
          "median": 39365535.03999999,
          "min": 32012564.740000002,
          "p95": 63198864.82
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_direct_calls_n17:candidate": {
        "batch": 1690,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 62405999.63,
          "median": 38103226.72,
          "min": 32868505.8,
          "p95": 62211705.400000006
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_direct_calls_n1:baseline": {
        "batch": 13957,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 49051429.875999995,
          "median": 32589936.9465,
          "min": 27685314.555,
          "p95": 38847830.058
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_direct_calls_n1:candidate": {
        "batch": 13957,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 56218377.29,
          "median": 31886266.8775,
          "min": 28613566.711,
          "p95": 43548226.647
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_direct_calls_n32:baseline": {
        "batch": 584,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 31725648.16,
          "median": 24911084.728,
          "min": 19945959.944,
          "p95": 31564753.823999997
        },
        "observations": 30,
        "observed_under_20ms_count": 1
      },
      "1:scalar_direct_calls_n32:candidate": {
        "batch": 584,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 74036292.232,
          "median": 24043101.004,
          "min": 20270275.0,
          "p95": 33009721.080000002
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_dynamic_api_n0:baseline": {
        "batch": 65716,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 30162198.248,
          "median": 22916319.23,
          "min": 20597957.324,
          "p95": 28021630.979999997
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_dynamic_api_n0:candidate": {
        "batch": 65716,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 41430323.620000005,
          "median": 23059678.684,
          "min": 21167715.044,
          "p95": 29210959.148
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_dynamic_api_n10000:baseline": {
        "batch": 68471,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 37610846.416,
          "median": 24385330.411,
          "min": 21327347.080000002,
          "p95": 30513484.911000002
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_dynamic_api_n10000:candidate": {
        "batch": 68471,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 28030931.864,
          "median": 23347789.347999997,
          "min": 21443131.541,
          "p95": 25854581.129
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_dynamic_api_n1024:baseline": {
        "batch": 61041,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 30327122.112,
          "median": 22726418.874,
          "min": 19794802.766999997,
          "p95": 28345426.047000002
        },
        "observations": 30,
        "observed_under_20ms_count": 2
      },
      "1:scalar_dynamic_api_n1024:candidate": {
        "batch": 61041,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 39120444.408,
          "median": 23505851.403,
          "min": 19307268.3,
          "p95": 32789821.257000003
        },
        "observations": 30,
        "observed_under_20ms_count": 1
      },
      "1:scalar_dynamic_api_n17:baseline": {
        "batch": 56180,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 36924922.98,
          "median": 22879220.729999997,
          "min": 19789573.54,
          "p95": 35176769.92
        },
        "observations": 30,
        "observed_under_20ms_count": 2
      },
      "1:scalar_dynamic_api_n17:candidate": {
        "batch": 56180,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 43429499.56,
          "median": 22698152.59,
          "min": 19019064.84,
          "p95": 29290285.7
        },
        "observations": 30,
        "observed_under_20ms_count": 2
      },
      "1:scalar_dynamic_api_n1:baseline": {
        "batch": 56498,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 58337631.378,
          "median": 20925107.762000002,
          "min": 17724439.564,
          "p95": 29387321.704
        },
        "observations": 30,
        "observed_under_20ms_count": 10
      },
      "1:scalar_dynamic_api_n1:candidate": {
        "batch": 56498,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 41434446.742,
          "median": 22510074.405,
          "min": 18338233.836,
          "p95": 41319077.826
        },
        "observations": 30,
        "observed_under_20ms_count": 10
      },
      "1:scalar_dynamic_api_n32:baseline": {
        "batch": 61850,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 40179429.949999996,
          "median": 23107685.725,
          "min": 20234907.85,
          "p95": 35666173.599999994
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_dynamic_api_n32:candidate": {
        "batch": 61850,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 43735681.25,
          "median": 23053752.525,
          "min": 21075511.2,
          "p95": 42658687.2
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_host_call_n0:baseline": {
        "batch": 27064,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 48421068.448,
          "median": 20884666.328,
          "min": 17030319.704,
          "p95": 43549927.344
        },
        "observations": 30,
        "observed_under_20ms_count": 12
      },
      "1:scalar_host_call_n0:candidate": {
        "batch": 27064,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 142730014.944,
          "median": 20861729.588,
          "min": 16604765.368,
          "p95": 80004079.848
        },
        "observations": 30,
        "observed_under_20ms_count": 11
      },
      "1:scalar_host_call_n10000:baseline": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 311554296.0,
          "median": 25838996.5005,
          "min": 20758709.001000002,
          "p95": 173578197.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_host_call_n10000:candidate": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 64878129.999,
          "median": 24464255.5005,
          "min": 21602232.0,
          "p95": 62195331.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_host_call_n1024:baseline": {
        "batch": 39,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 39369426.993,
          "median": 31611054.501000002,
          "min": 28085207.007000003,
          "p95": 36100468.989
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_host_call_n1024:candidate": {
        "batch": 39,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 45032649.012,
          "median": 30834142.501500003,
          "min": 27430366.002,
          "p95": 41566502.991000004
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_host_call_n17:baseline": {
        "batch": 2075,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 41484256.975,
          "median": 32615972.575000003,
          "min": 27097024.525000002,
          "p95": 40679142.449999996
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_host_call_n17:candidate": {
        "batch": 2075,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 57814002.75,
          "median": 31462458.2875,
          "min": 27878527.625,
          "p95": 38935084.2
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_host_call_n1:baseline": {
        "batch": 16400,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 148703326.4,
          "median": 24907951.0,
          "min": 22210470.8,
          "p95": 34986382.4
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_host_call_n1:candidate": {
        "batch": 16400,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 132225065.6,
          "median": 25042398.2,
          "min": 21143257.2,
          "p95": 80050925.60000001
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_host_call_n32:baseline": {
        "batch": 1136,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 144524555.52,
          "median": 31196279.904,
          "min": 26897928.544,
          "p95": 67812919.056
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_host_call_n32:candidate": {
        "batch": 1136,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 147468030.352,
          "median": 30172938.16,
          "min": 26117397.712,
          "p95": 88432884.464
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_indirect_calls_n0:baseline": {
        "batch": 28307,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 54921298.014,
          "median": 27927247.4415,
          "min": 23908262.042,
          "p95": 49105738.092
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_indirect_calls_n0:candidate": {
        "batch": 28307,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 38263194.654,
          "median": 25871253.417499997,
          "min": 23146124.374,
          "p95": 35588862.522
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_indirect_calls_n10000:baseline": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 106358923.0,
          "median": 26975158.5,
          "min": 20454151.0,
          "p95": 48432952.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_indirect_calls_n10000:candidate": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 53651512.0,
          "median": 24282341.0,
          "min": 20823099.0,
          "p95": 48017009.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_indirect_calls_n1024:baseline": {
        "batch": 20,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 50796553.0,
          "median": 26451931.5,
          "min": 23457034.0,
          "p95": 47900360.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_indirect_calls_n1024:candidate": {
        "batch": 20,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 33890326.0,
          "median": 26838121.0,
          "min": 23156310.0,
          "p95": 33392101.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_indirect_calls_n17:baseline": {
        "batch": 1569,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 131470426.22399999,
          "median": 33676513.1355,
          "min": 30186789.620999996,
          "p95": 41223212.502000004
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_indirect_calls_n17:candidate": {
        "batch": 1569,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 61396214.217,
          "median": 33878664.6645,
          "min": 29290504.647,
          "p95": 45324203.511
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_indirect_calls_n1:baseline": {
        "batch": 12954,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 86239946.646,
          "median": 30173195.178,
          "min": 26723726.334,
          "p95": 75324206.73
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_indirect_calls_n1:candidate": {
        "batch": 12954,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 77394424.332,
          "median": 31363564.145999998,
          "min": 27603911.772,
          "p95": 74515190.568
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_indirect_calls_n32:baseline": {
        "batch": 861,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 51704071.146,
          "median": 37706112.6645,
          "min": 32419889.943,
          "p95": 47910826.299
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_indirect_calls_n32:candidate": {
        "batch": 861,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 75255158.265,
          "median": 37812801.32699999,
          "min": 30707423.796000004,
          "p95": 63593559.875999995
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_multivalue_branch_n0:baseline": {
        "batch": 23981,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 35142812.563999996,
          "median": 21157261.231,
          "min": 17538264.54,
          "p95": 32878286.733999997
        },
        "observations": 30,
        "observed_under_20ms_count": 9
      },
      "1:scalar_multivalue_branch_n0:candidate": {
        "batch": 23981,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 27559636.668,
          "median": 20438178.9555,
          "min": 17661215.127,
          "p95": 27336565.406
        },
        "observations": 30,
        "observed_under_20ms_count": 11
      },
      "1:scalar_multivalue_branch_n10000:baseline": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 41352359.0,
          "median": 29969208.5,
          "min": 25859711.0,
          "p95": 38266350.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_multivalue_branch_n10000:candidate": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 42388793.0,
          "median": 30152148.5,
          "min": 27018755.0,
          "p95": 39381593.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_multivalue_branch_n1024:baseline": {
        "batch": 16,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 135176266.0,
          "median": 25205058.0,
          "min": 21034571.008,
          "p95": 59315860.992
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_multivalue_branch_n1024:candidate": {
        "batch": 16,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 61344067.008,
          "median": 27104529.008,
          "min": 22424838.0,
          "p95": 56516894.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_multivalue_branch_n17:baseline": {
        "batch": 1184,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 39315025.472,
          "median": 33862789.536,
          "min": 30980239.711999997,
          "p95": 38758412.864
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_multivalue_branch_n17:candidate": {
        "batch": 1184,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 53023430.752,
          "median": 33347770.256,
          "min": 30920258.272,
          "p95": 44083263.423999995
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_multivalue_branch_n1:baseline": {
        "batch": 11662,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 32920379.912000004,
          "median": 25730436.207000002,
          "min": 24022168.954000004,
          "p95": 31811801.854
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_multivalue_branch_n1:candidate": {
        "batch": 11662,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 32683594.664,
          "median": 26169557.155,
          "min": 23861863.102,
          "p95": 31750821.256
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_multivalue_branch_n32:baseline": {
        "batch": 675,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 50275202.175000004,
          "median": 30840762.825000003,
          "min": 28677694.950000003,
          "p95": 38675236.05
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_multivalue_branch_n32:candidate": {
        "batch": 675,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 43136155.125,
          "median": 31236145.0875,
          "min": 28897842.150000002,
          "p95": 38260032.75
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_ordinary_recursion_n0:baseline": {
        "batch": 20483,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 27128955.146,
          "median": 21970854.3955,
          "min": 19870558.3,
          "p95": 26111503.087
        },
        "observations": 30,
        "observed_under_20ms_count": 1
      },
      "1:scalar_ordinary_recursion_n0:candidate": {
        "batch": 20483,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 34037871.046,
          "median": 23129987.365499996,
          "min": 20453197.235,
          "p95": 32744820.222
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_ordinary_recursion_n17:baseline": {
        "batch": 1867,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 81289398.439,
          "median": 41925798.213,
          "min": 35167536.396,
          "p95": 64500324.392000005
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_ordinary_recursion_n17:candidate": {
        "batch": 1867,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 61150605.268,
          "median": 39715737.8965,
          "min": 34412999.548,
          "p95": 58320636.54
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_ordinary_recursion_n1:baseline": {
        "batch": 15409,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 57300554.577999994,
          "median": 34348748.9195,
          "min": 31473298.543,
          "p95": 46791339.216
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_ordinary_recursion_n1:candidate": {
        "batch": 15409,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 68047068.54,
          "median": 34195059.5535,
          "min": 31165334.268999998,
          "p95": 44805750.885
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_ordinary_recursion_n32:baseline": {
        "batch": 876,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 36065623.428,
          "median": 30955186.158,
          "min": 26955975.912,
          "p95": 35563117.416
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_ordinary_recursion_n32:candidate": {
        "batch": 876,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 53217988.128,
          "median": 31560940.158,
          "min": 28121339.736,
          "p95": 39979604.567999996
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_table_mutation_n0:baseline": {
        "batch": 27153,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 35400778.056,
          "median": 23006492.523000002,
          "min": 19629120.924,
          "p95": 31598570.619
        },
        "observations": 30,
        "observed_under_20ms_count": 1
      },
      "1:scalar_table_mutation_n0:candidate": {
        "batch": 27153,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 53597822.607,
          "median": 22200985.2015,
          "min": 18706543.443,
          "p95": 30881840.031
        },
        "observations": 30,
        "observed_under_20ms_count": 4
      },
      "1:scalar_table_mutation_n10000:baseline": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 73939274.001,
          "median": 29448508.5,
          "min": 27279113.001,
          "p95": 38418041.001
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_table_mutation_n10000:candidate": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 36930849.999,
          "median": 29384712.0,
          "min": 26760956.001,
          "p95": 34729155.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_table_mutation_n1024:baseline": {
        "batch": 23,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 40496589.003,
          "median": 22566868.4965,
          "min": 20748967.989,
          "p95": 37305684.992
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_table_mutation_n1024:candidate": {
        "batch": 23,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 28988027.989,
          "median": 22592197.994,
          "min": 20765882.994,
          "p95": 28589838.994
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_table_mutation_n17:baseline": {
        "batch": 1735,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 208604697.425,
          "median": 33450862.46,
          "min": 26436670.39,
          "p95": 81407084.85000001
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_table_mutation_n17:candidate": {
        "batch": 1735,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 140628884.32,
          "median": 33866943.22,
          "min": 26762243.14,
          "p95": 114036780.485
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_table_mutation_n1:baseline": {
        "batch": 14348,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 40908328.896,
          "median": 25945136.873999998,
          "min": 24142375.240000002,
          "p95": 32492107.752
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_table_mutation_n1:candidate": {
        "batch": 14348,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 37390529.3,
          "median": 26155607.686,
          "min": 22963227.904000003,
          "p95": 35022707.556
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_table_mutation_n32:baseline": {
        "batch": 958,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 48979094.074,
          "median": 34608123.141,
          "min": 30718111.93,
          "p95": 48876658.008
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_table_mutation_n32:candidate": {
        "batch": 958,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 61730792.574,
          "median": 34617085.70999999,
          "min": 30260951.456,
          "p95": 52725234.282000005
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_tail_calls_n0:baseline": {
        "batch": 27360,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 52540092.0,
          "median": 27054922.32,
          "min": 22016592.0,
          "p95": 51516745.92
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_tail_calls_n0:candidate": {
        "batch": 27360,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 74530582.56,
          "median": 26970174.72,
          "min": 22627458.720000003,
          "p95": 63246607.2
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_tail_calls_n10000:baseline": {
        "batch": 4,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 46625662.0,
          "median": 29456411.5,
          "min": 26193305.0,
          "p95": 38319835.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_tail_calls_n10000:candidate": {
        "batch": 4,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 38855577.0,
          "median": 28748556.5,
          "min": 26344649.0,
          "p95": 37779998.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_tail_calls_n1024:baseline": {
        "batch": 39,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 48029946.003,
          "median": 29851566.498,
          "min": 25744854.992999997,
          "p95": 46920850.002000004
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_tail_calls_n1024:candidate": {
        "batch": 39,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 45533684.000999995,
          "median": 30540354.0,
          "min": 26362998.012,
          "p95": 41028979.992
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_tail_calls_n17:baseline": {
        "batch": 2028,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 72137101.764,
          "median": 28319325.606,
          "min": 23749210.368,
          "p95": 53127221.94
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_tail_calls_n17:candidate": {
        "batch": 2028,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 51898616.952,
          "median": 29138063.682,
          "min": 25366132.74,
          "p95": 49054172.544
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_tail_calls_n1:baseline": {
        "batch": 13254,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 53894660.676,
          "median": 22361393.322,
          "min": 19932995.442,
          "p95": 48282280.884
        },
        "observations": 30,
        "observed_under_20ms_count": 1
      },
      "1:scalar_tail_calls_n1:candidate": {
        "batch": 13254,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 34276315.194,
          "median": 22658673.915,
          "min": 19445897.688,
          "p95": 32630725.062
        },
        "observations": 30,
        "observed_under_20ms_count": 2
      },
      "1:scalar_tail_calls_n32:baseline": {
        "batch": 1191,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 56658348.471,
          "median": 29072521.998,
          "min": 25503116.436,
          "p95": 33577611.699
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_tail_calls_n32:candidate": {
        "batch": 1191,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 40386562.272,
          "median": 29203549.862999998,
          "min": 26787302.657999996,
          "p95": 36929578.773
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_try_table_normal_n0:baseline": {
        "batch": 32042,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 35046193.836,
          "median": 21223947.917999998,
          "min": 19018785.436,
          "p95": 34259210.274
        },
        "observations": 30,
        "observed_under_20ms_count": 5
      },
      "1:scalar_try_table_normal_n0:candidate": {
        "batch": 32042,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 40136898.628,
          "median": 21812255.059,
          "min": 18313733.268,
          "p95": 39156285.26
        },
        "observations": 30,
        "observed_under_20ms_count": 9
      },
      "1:scalar_try_table_normal_n10000:baseline": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 44853423.0,
          "median": 23500018.0005,
          "min": 21360396.998999998,
          "p95": 41572701.999
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_try_table_normal_n10000:candidate": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 38978117.001,
          "median": 23306286.501000002,
          "min": 21128069.001000002,
          "p95": 32844938.001
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_try_table_normal_n1024:baseline": {
        "batch": 31,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 32859036.985000003,
          "median": 23783791.4955,
          "min": 21436466.985000003,
          "p95": 29964286.993
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_try_table_normal_n1024:candidate": {
        "batch": 31,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 56596084.991000004,
          "median": 24490720.502,
          "min": 22121180.012000002,
          "p95": 46811977.986
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_try_table_normal_n17:baseline": {
        "batch": 2095,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 53262152.89,
          "median": 31101817.9675,
          "min": 27940055.49,
          "p95": 41652861.23
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_try_table_normal_n17:candidate": {
        "batch": 2095,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 40271678.010000005,
          "median": 31470556.8225,
          "min": 27479913.880000003,
          "p95": 38408079.14
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_try_table_normal_n1:baseline": {
        "batch": 15023,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 61047718.191,
          "median": 23799436.6,
          "min": 20072996.473,
          "p95": 46992199.390999995
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_try_table_normal_n1:candidate": {
        "batch": 15023,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 35915741.491000004,
          "median": 21407910.207000002,
          "min": 18813723.544,
          "p95": 33928003.292
        },
        "observations": 30,
        "observed_under_20ms_count": 5
      },
      "1:scalar_try_table_normal_n32:baseline": {
        "batch": 1151,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 32799997.507,
          "median": 29768190.4185,
          "min": 26744137.617,
          "p95": 32378595.689
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_try_table_normal_n32:candidate": {
        "batch": 1151,
        "calibration_class": "paired_shared_20ms",
        "estimated_total_ns": {
          "max": 43064783.553,
          "median": 29249173.4415,
          "min": 26770545.009999998,
          "p95": 35011806.298
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      }
    },
    "workload_count": 85
  },
  "outcome": "rejected",
  "raw_gate_decision": "rejected",
  "reason": "P030 candidate failed the predeclared strict incremental build gate.",
  "schema": "wtime-p030-completion-v1",
  "status": "done",
  "strict_build_gate_passed": false,
  "task": "P030",
  "task_index_update_performed": false
}
```
