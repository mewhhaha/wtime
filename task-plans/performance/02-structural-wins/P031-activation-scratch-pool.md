# P031 — Reuse invocation scratch capacity safely under host reentry

Status: **done**

Outcome: **rejected**

Kind: `candidate` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P002 — Compare separately built baseline and candidate binaries](../00-measurement/P002-paired-driver.md)

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P008 — Add scaling, branch and call benchmark families](../00-measurement/P008-scalar-workloads.md)

[P016 — Add explicit executor policies and test integration hooks](../01-execution-contracts/P016-execution-policy.md)

[P018 — Define source cursors and materialized guest state](../01-execution-contracts/P018-source-cursor.md)

[P029 — Stop cloning function signatures into every activation](P029-shared-function-metadata.md)

[P030 — Split checked public entry from validated guest call entry](P030-validated-guest-frame-entry.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C2, C3, C4, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/activation.rs`
- `crates/wtime-runtime/src/execution.rs`
- `crates/wtime-runtime/src/lib.rs`
- `crates/wtime-runtime/tests/performance.rs`
- `crates/wtime-runtime/tests/performance/p031.rs`
- `tools/wtime-bench/compare.py`
- `tools/wtime-bench/tests/test_p031_calibration.py`
- `tools/wtime-bench/tests/test_p030_calibration.py`
- `docs/benchmarks/performance/P031-calibration-correctness.json`
- `docs/benchmarks/performance/P031-candidate.patch`
- `docs/benchmarks/performance/P031-contract.json`
- `docs/benchmarks/performance/P031-build-decision.md`
- `docs/benchmarks/performance/P031-evidence.json`

One integrator owns activation.rs, Store integration in lib.rs, dispatch in execution.rs, and public test registration in tests/performance.rs; the private unit tests stay nested in activation.rs.

## Input/output contract

Store owns a bounded idle activation pool; an active invocation exclusively owns its frames/operand/prompts buffers, and nested host reentry obtains another activation. Captured continuations own detached storage.

## Implementation recipe

1. Move activation vectors into a private container and hand them out with an explicit take/return protocol that cannot alias an outer active invocation.
2. Clear logical lengths and all roots on return, bound retained capacities/number of idle activations and trim oversized buffers according to a documented policy.
3. Return buffers on success, trap, resource error and host error; captured segments transfer ownership and cannot be returned to the pool until consumed/dropped.
4. Compare tiny invocation and recursive/reentrant call costs, tracking retained capacity as well as removed allocations.
5. Record the changed path with focused assertions/diagnostic counts and exact source/binary identities; compare the pre-edit archive, and revert a rejected structural optimization rather than adding a permanent toggle solely for testing.

## Meaningful cases

- Nested host invoke uses separate buffers and preserves outer locals/operands.
- Trap/error cleanup leaves no active roots or stale pooled references.
- Captured continuation storage is not recycled early and pool retention respects its byte budget.

## Verification

The public focused-test draft currently contains three default tests and a fourth under `execution-reference`; ten private activation/pool tests are staged but remain unbuilt and unrun. These are planned counts, not completion evidence.

The compare preflight will require each of 170 warm calibration records (85 selected workloads times two executables) to reach 20 ms. That target applies only to calibration observations; later timed pairs remain unfiltered and may be shorter. The existing compare CLI and benchmark protocol remain unchanged, and this calibration adds no Rust changes.

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime p031_ -- --nocapture
python3 tools/wtime-bench/tests/test_p030_calibration.py
python3 tools/wtime-bench/tests/test_p031_calibration.py
cargo test --locked -p wtime-runtime --features execution-reference,performance-counters p031_ -- --nocapture
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml -j 4
python3 tools/checks/build_budget.py --output target/performance/P031-build.json
python3 tools/wtime-bench/compare.py --baseline target/performance/P031-before --candidate tools/wtime-bench/target/release/wtime-bench --candidate-mode decoded --selection scalar --pairs 30 --build-growth-explanation-file docs/benchmarks/performance/P031-build-decision.md --output target/performance/P031
python3 tools/wtime-bench/analyze.py --input target/performance/P031 --output target/performance/P031/analysis.json
python3 tools/wtime-bench/gates.py --task P031 --comparison target/performance/P031 --build target/performance/P031-build.json --contract docs/benchmarks/performance/P031-contract.json --output target/performance/P031/decision.json
```

## Measurement and decision

Selection: `scalar`. Explicit execution setting: `decoded`. Decision rule: `measured_promotion`.

Use the paired protocol and family/control/cold/build rules in [MEASUREMENT.md](../MEASUREMENT.md). Predeclare the targeted family and held-out controls before changing the implementation. Keep instrumentation off in production timings. Adopt only with supported gains in both runs; otherwise record rejected/inconclusive and remove default-path cost. A necessary immutable-interface change may be contract_only with its separate justification.

## Before editing

Run these commands before changing the candidate sources. The archive is immutable and captures the immediate predecessor. Retain P001's original archive for milestone totals. For native work, use the named portable control and verify its actual policy/quantum; if this archive already exists with different sources, choose a new recorded run-specific directory and update the comparison path rather than overwriting it.

```sh
python3 tools/wtime-bench/baseline.py --output target/performance/P031-before
```

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. Executed evidence is recorded below.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/docs/benchmarks/performance/P031-build-decision.md": {
      "sha256": "5e8c0ab1fc253d470be94d910820eeb87d8ea1b451801f241772157c694f691a",
      "bytes": 3234
    },
    "/workspace/wtime/docs/benchmarks/performance/P031-calibration-correctness.json": {
      "sha256": "991ed7b4880ccc92ce2f5d81535a1f3af2ca58cc2af982bbc1d91c63837effbf",
      "bytes": 14837
    },
    "/workspace/wtime/docs/benchmarks/performance/P031-candidate.patch": {
      "sha256": "bd4e7677918702c1d164f24f92e86c69702c7f0f34b866c82c2fe5ecd837ddcb",
      "bytes": 82753
    },
    "/workspace/wtime/docs/benchmarks/performance/P031-contract.json": {
      "sha256": "5b2378a28cfbb85c2c9418197f930c5bc5f66e6ce501056d63040af379a03cf7",
      "bytes": 2757
    },
    "/workspace/wtime/docs/benchmarks/performance/P031-evidence.json": {
      "sha256": "bacd17f95e897f0e7765771f9815f61f095de9b9a9a3dbce8eac8c4658734bef",
      "bytes": 1388485
    },
    "/workspace/wtime/target/performance/P030/restored-default-tests.log": {
      "sha256": "143578b60a3c2727cb4e4ac01f1197ebc252bfb70b40c3a73823cb041d04d063",
      "bytes": 14952
    },
    "/workspace/wtime/target/performance/P030/restored-diagnostic-tests.log": {
      "sha256": "7cc30e87819b7a53be6ef961bc9b8e3fead1d2636bf80c19c355ba2433db0087",
      "bytes": 5164
    },
    "/workspace/wtime/target/performance/P031-before-build.json": {
      "sha256": "0b2bf156654d1d1c7db08bfa509315ae4128f5154844ee75f8edc494a5a183a5",
      "bytes": 106299
    },
    "/workspace/wtime/target/performance/P031-before/manifest.json": {
      "sha256": "82079d14c9c4b4a47c0448fed97e74b72dfcfea373994a2a859a04908cfcea83",
      "bytes": 37864
    },
    "/workspace/wtime/target/performance/P031-build.json": {
      "sha256": "4b9891b070b4f3ef4a58850349c91a8c0801e11d9bf5bc08722c372913bb9d76",
      "bytes": 106524
    },
    "/workspace/wtime/target/performance/P031-candidate-checks-retry/diagnostic_performance.log": {
      "sha256": "dd63f8f45022511f5d287e992c9f1f53435e3ae8f33c89fac5e7499d1fdb1a0c",
      "bytes": 5413
    },
    "/workspace/wtime/target/performance/P031-candidate-checks-retry/focused_default.log": {
      "sha256": "7e264f2d3620f5176f068b341b9c3ed950b00c7981c8eaa4c99525d273b4429e",
      "bytes": 2797
    },
    "/workspace/wtime/target/performance/P031-candidate-checks-retry/focused_diagnostic.log": {
      "sha256": "c8590616d0457fee4593eb42f41f4d4c0ae8712271d88014a6775fadee12941a",
      "bytes": 2871
    },
    "/workspace/wtime/target/performance/P031-candidate-checks-retry/full_default.log": {
      "sha256": "0e42605a59fa9510497d2eb630df717305e3115afa01196feabdad9143767e42",
      "bytes": 16212
    },
    "/workspace/wtime/target/performance/P031-candidate-checks/commands.json": {
      "sha256": "544dd7ff0e9c9157ff2c2bd87c6aa646463ae771df591758107398a395bd48a7",
      "bytes": 2380
    },
    "/workspace/wtime/target/performance/P031-candidate-retained/source-manifest.json": {
      "sha256": "8eea33eb242415d40a28d80bb53c93cad2a497e29ab894b10de9bd0cdc10b035",
      "bytes": 15367
    },
    "/workspace/wtime/target/performance/P031-incremental-build-decision.json": {
      "sha256": "a1926d33bb2b19d0c3bca702a01e243dd26e89636edc67235ef2b7241017d656",
      "bytes": 1394
    },
    "/workspace/wtime/target/performance/P031-preflight/focused_default.log": {
      "sha256": "363bbf97b18654f1228c3e6a77956df90ead97c0384beeb0c1bb56b1e1b52e23",
      "bytes": 1784
    },
    "/workspace/wtime/target/performance/P031-preflight/focused_diagnostic.log": {
      "sha256": "91fe26b5878ecb9ff8d3c69c7b5085beb70017d136e32201cbf7cd3ef5a339d0",
      "bytes": 1858
    },
    "/workspace/wtime/target/performance/P031-preflight/preflight-attempts.json": {
      "sha256": "c9e013de44e3f62f7e6071cded7d01971f84f8069607ec86b5864c77686b98cf",
      "bytes": 1332
    },
    "/workspace/wtime/target/performance/P031/analysis.json": {
      "sha256": "7ea1df3d15d1995fe10221c1b4e0d0284e2a767f45879da57098b9043d5c89c3",
      "bytes": 1264169
    },
    "/workspace/wtime/target/performance/P031/analysis.log": {
      "sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
      "bytes": 0
    },
    "/workspace/wtime/target/performance/P031/checkpoint.jsonl": {
      "sha256": "35d63dff9d4b7e2138c5e5b07ad7b33d36b5b981519bfc603ac71aafbe102338",
      "bytes": 36221985
    },
    "/workspace/wtime/target/performance/P031/commands.json": {
      "sha256": "d381b1b2e12bc26438c8f67fb693122a27f48256ad1c14da8521460cbcc23bc0",
      "bytes": 8076
    },
    "/workspace/wtime/target/performance/P031/decision.json": {
      "sha256": "48f75246d87736a7ce7a0cf4b8c2cdeac68c1633b77673a66240f3c865b42ac2",
      "bytes": 1688189
    },
    "/workspace/wtime/target/performance/P031/development-provenance.json": {
      "sha256": "6f86bbdc1eb0d0bcd4c7a52129199f801bb7112662ccd122ba8805addba7743e",
      "bytes": 7184
    },
    "/workspace/wtime/target/performance/P031/extraction.json": {
      "sha256": "199d387de8f3696e6c080287315a9d6bf6d6acdbbd19d677247bfc5d5735885c",
      "bytes": 477125
    },
    "/workspace/wtime/target/performance/P031/extraction.log": {
      "sha256": "d876a6553099c40825dafbe742848f09db0b637e6033632759d51db324387957",
      "bytes": 305179
    },
    "/workspace/wtime/target/performance/P031/final-decision.json": {
      "sha256": "75340d12d9bf865c1804e8e503254220ae0e8654a6f6064438421ede13429f54",
      "bytes": 12069
    },
    "/workspace/wtime/target/performance/P031/finalizer-wrapper-command.json": {
      "sha256": "bb7bda2b2e36e84a6fcf2cc65543035eee7af1ef3d79acede7c1d56ee1c000c8",
      "bytes": 8171
    },
    "/workspace/wtime/target/performance/P031/finalizer-wrapper.log": {
      "sha256": "75096960acfd83b0ebf9925b258c976eba4734bf6fa7607e41c06d3d8d4ce294",
      "bytes": 172
    },
    "/workspace/wtime/target/performance/P031/raw-exits.json": {
      "sha256": "8cb285167680019e8683e276900144b811a534abc0797ee27c6fde696476ba90",
      "bytes": 39
    },
    "/workspace/wtime/target/performance/P031/raw-gate.log": {
      "sha256": "48f75246d87736a7ce7a0cf4b8c2cdeac68c1633b77673a66240f3c865b42ac2",
      "bytes": 1688189
    },
    "/workspace/wtime/target/performance/P031/report.json": {
      "sha256": "52c27ab49fa8b7a16e346bf91eee19a48c1ea5bca4585875c836ea6dba918991",
      "bytes": 60166974
    },
    "/workspace/wtime/target/performance/P031/restored-build.json": {
      "sha256": "37fbf08e60f7fc83bb2f887a213eb414db74bc6fcf4dfcd027d90a109af74841",
      "bytes": 106297
    },
    "/workspace/wtime/target/performance/P031/restored-default-tests.log": {
      "sha256": "eeb778cb5449fe81a6b71f85dba2d438f72bb9e16ae91a7a74806a5873066879",
      "bytes": 15202
    },
    "/workspace/wtime/target/performance/P031/restored-diagnostic-tests.log": {
      "sha256": "6b4484529b6be3aeaff9e51ff311ab79de8fcc6b4ed44184cabc6dd6debb6508",
      "bytes": 5487
    },
    "/workspace/wtime/target/performance/P031/restored-focused-default-tests.log": {
      "sha256": "973688971c4b981b977eb93bf0b93a7b0fe9b66d73ddb427bc319a962571bc51",
      "bytes": 1710
    },
    "/workspace/wtime/target/performance/P031/restored-focused-diagnostic-tests.log": {
      "sha256": "533bff59b8af464596ce679edcb0113cbdae039d7f5dca611ad464d457f56bca",
      "bytes": 1858
    },
    "/workspace/wtime/target/performance/P031/rollback-identities.json": {
      "sha256": "3fba36f046f57309b5fa3a5a5a8dbff3af9036e4275fcece41e8f54475b98059",
      "bytes": 4487
    },
    "/workspace/wtime/target/performance/P031/task-precompletion/P031-activation-scratch-pool.md": {
      "sha256": "abc3595cd11e83be70eb672963b0768b36397f0b9700558ba18d52d8e6362f3e",
      "bytes": 7003
    },
    "/workspace/wtime/target/performance/P031/task-precompletion/TASKS.json": {
      "sha256": "6ae1a1852b3a2efb7b8603ee987bf3ef2faf3bc0ab54e911b4211084a3815c54",
      "bytes": 1801081
    },
    "/workspace/wtime/target/performance/P031/tooling-provenance/P031-complete-pre-settings-review.py": {
      "sha256": "0ad2ed50fbd88d35ae9c34721bb072ae40383af3aea57fedd0c2e8184289193d",
      "bytes": 54131
    },
    "/workspace/wtime/target/performance/P031/tooling-provenance/P031-complete.py": {
      "sha256": "6f2bb5077955cc49dd2cf87378c4438787581fe96b5809c3f015e84caba93724",
      "bytes": 56393
    },
    "/workspace/wtime/target/performance/P031/tooling-provenance/P031-finalize-command.py": {
      "sha256": "646fea83493f4adfe8b5f5cae29fe3d9055af6b3c9c5fe597f34e7671116664a",
      "bytes": 9748
    },
    "/workspace/wtime/target/performance/P031/tooling-provenance/P031-finalizer-review.md": {
      "sha256": "8d82c8dede6c99d71e475738f8247c91295c73d06d2a531d3f3bf98c6f6b435d",
      "bytes": 5038
    },
    "/workspace/wtime/target/performance/P031/tooling-provenance/P031-pool-static-review.md": {
      "sha256": "03ca063c92c2d6caa53b280d68611e14acf3b6e5d035c7133ae839cd83a146ae",
      "bytes": 3590
    },
    "/workspace/wtime/target/performance/P031/tooling-provenance/P031-process-closed.py": {
      "sha256": "51231ee1b2f5b98d003126849f3b3dff276b3a41092cd4943c2e489242cff511",
      "bytes": 3568
    },
    "/workspace/wtime/target/performance/P031/tooling-provenance/P031-restore-pre-review.py": {
      "sha256": "7c8084ce39efebc65cd327ec287a87643cc054b0f7d674681434754601b57f44",
      "bytes": 25632
    },
    "/workspace/wtime/target/performance/P031/tooling-provenance/P031-restore-pre-settings-review.py": {
      "sha256": "97bbca1ef0073ee90b43e1c21a5d1624a261809f6838378400a810421d1ec73a",
      "bytes": 25274
    },
    "/workspace/wtime/target/performance/P031/tooling-provenance/P031-restore.py": {
      "sha256": "d48449722b57d7069b239f48b9bf52d8a17ebee508972ca91be542c82b77650d",
      "bytes": 29390
    },
    "/workspace/wtime/target/performance/P031/tooling-provenance/P031-return-review.md": {
      "sha256": "fc5a0ba24de81c7d1fead9eed4cfea53a26ba3d79eef0a2118ac140c442a1257",
      "bytes": 2755
    },
    "/workspace/wtime/target/performance/P031/tooling-provenance/P031-runtime-tests-static-review.md": {
      "sha256": "cca029adbc1eb5f768950676cd57fe06682e81e9943a968a2f1b43cb0aa685d0",
      "bytes": 4628
    },
    "/workspace/wtime/target/performance/P031/tooling-provenance/P031-wrapper-review.md": {
      "sha256": "08f7b430e8ebee3d04f6b6674e2f623bfc351e3acaadab549637774ea88e1652",
      "bytes": 4148
    },
    "/workspace/wtime/target/performance/P031/tooling-provenance/extract_p031-original-reconstructed.py": {
      "sha256": "a1a9f89d6791135889ebefc0d44465c7bc79a376987c54f49180d04feba405de",
      "bytes": 41938
    },
    "/workspace/wtime/target/performance/P031/tooling-provenance/extract_p031-overwrite-recovery.py": {
      "sha256": "2ff46915b00641a2c417235380f0d5c3fdde83722567bb8f00436056d1b1b984",
      "bytes": 39514
    },
    "/workspace/wtime/target/performance/P031/tooling-provenance/extract_p031-recovery-review.md": {
      "sha256": "678620a26a69b7f67b4ee71ba537997bfc4a70b116a9e4aa4055a31d122e4bcf",
      "bytes": 1707
    },
    "/workspace/wtime/target/performance/P031/tooling-provenance/extract_p031.py": {
      "sha256": "5f9106f0e136c83a8dec6a492d556aed0f7857008bf7f6b1ed9aa60ddf625679",
      "bytes": 42579
    },
    "/workspace/wtime/target/performance/P031/tooling-provenance/test_extract_p031-after-repair.log": {
      "sha256": "03e2ce7efb78ca7222db4c9a68781f9953dd3bd674b491cc52a8a4986d74e6f4",
      "bytes": 102
    },
    "/workspace/wtime/target/performance/P031/tooling-provenance/test_extract_p031-before-repair.log": {
      "sha256": "a1f86328d0e7e3d369e5456540b639704c49478cc76236b2a2f39fc8e171e83f",
      "bytes": 4764
    },
    "/workspace/wtime/target/performance/P031/tooling-provenance/test_extract_p031.py": {
      "sha256": "45a248ae79fc34e33bea61c6c20a74f86a8157711f3ef1e69939b727fde1ca9a",
      "bytes": 6735
    },
    "/workspace/wtime/target/performance/P031/tooling-provenance/test_p031_finalizer_settings.py": {
      "sha256": "71bbd250b22862d3e9ebf92f8e6f5db32233e16d477ffa7adfca2937e029ae7d",
      "bytes": 5795
    }
  },
  "artifacts": [
    "/workspace/wtime/docs/benchmarks/performance/P031-build-decision.md",
    "/workspace/wtime/docs/benchmarks/performance/P031-calibration-correctness.json",
    "/workspace/wtime/docs/benchmarks/performance/P031-candidate.patch",
    "/workspace/wtime/docs/benchmarks/performance/P031-contract.json",
    "/workspace/wtime/docs/benchmarks/performance/P031-evidence.json",
    "/workspace/wtime/target/performance/P030/restored-default-tests.log",
    "/workspace/wtime/target/performance/P030/restored-diagnostic-tests.log",
    "/workspace/wtime/target/performance/P031-before-build.json",
    "/workspace/wtime/target/performance/P031-before/manifest.json",
    "/workspace/wtime/target/performance/P031-build.json",
    "/workspace/wtime/target/performance/P031-candidate-checks-retry/diagnostic_performance.log",
    "/workspace/wtime/target/performance/P031-candidate-checks-retry/focused_default.log",
    "/workspace/wtime/target/performance/P031-candidate-checks-retry/focused_diagnostic.log",
    "/workspace/wtime/target/performance/P031-candidate-checks-retry/full_default.log",
    "/workspace/wtime/target/performance/P031-candidate-checks/commands.json",
    "/workspace/wtime/target/performance/P031-candidate-retained/source-manifest.json",
    "/workspace/wtime/target/performance/P031-incremental-build-decision.json",
    "/workspace/wtime/target/performance/P031-preflight/focused_default.log",
    "/workspace/wtime/target/performance/P031-preflight/focused_diagnostic.log",
    "/workspace/wtime/target/performance/P031-preflight/preflight-attempts.json",
    "/workspace/wtime/target/performance/P031/analysis.json",
    "/workspace/wtime/target/performance/P031/analysis.log",
    "/workspace/wtime/target/performance/P031/checkpoint.jsonl",
    "/workspace/wtime/target/performance/P031/commands.json",
    "/workspace/wtime/target/performance/P031/decision.json",
    "/workspace/wtime/target/performance/P031/development-provenance.json",
    "/workspace/wtime/target/performance/P031/extraction.json",
    "/workspace/wtime/target/performance/P031/extraction.log",
    "/workspace/wtime/target/performance/P031/final-decision.json",
    "/workspace/wtime/target/performance/P031/finalizer-wrapper-command.json",
    "/workspace/wtime/target/performance/P031/finalizer-wrapper.log",
    "/workspace/wtime/target/performance/P031/raw-exits.json",
    "/workspace/wtime/target/performance/P031/raw-gate.log",
    "/workspace/wtime/target/performance/P031/report.json",
    "/workspace/wtime/target/performance/P031/restored-build.json",
    "/workspace/wtime/target/performance/P031/restored-default-tests.log",
    "/workspace/wtime/target/performance/P031/restored-diagnostic-tests.log",
    "/workspace/wtime/target/performance/P031/restored-focused-default-tests.log",
    "/workspace/wtime/target/performance/P031/restored-focused-diagnostic-tests.log",
    "/workspace/wtime/target/performance/P031/rollback-identities.json",
    "/workspace/wtime/target/performance/P031/task-precompletion/P031-activation-scratch-pool.md",
    "/workspace/wtime/target/performance/P031/task-precompletion/TASKS.json",
    "/workspace/wtime/target/performance/P031/tooling-provenance/P031-complete-pre-settings-review.py",
    "/workspace/wtime/target/performance/P031/tooling-provenance/P031-complete.py",
    "/workspace/wtime/target/performance/P031/tooling-provenance/P031-finalize-command.py",
    "/workspace/wtime/target/performance/P031/tooling-provenance/P031-finalizer-review.md",
    "/workspace/wtime/target/performance/P031/tooling-provenance/P031-pool-static-review.md",
    "/workspace/wtime/target/performance/P031/tooling-provenance/P031-process-closed.py",
    "/workspace/wtime/target/performance/P031/tooling-provenance/P031-restore-pre-review.py",
    "/workspace/wtime/target/performance/P031/tooling-provenance/P031-restore-pre-settings-review.py",
    "/workspace/wtime/target/performance/P031/tooling-provenance/P031-restore.py",
    "/workspace/wtime/target/performance/P031/tooling-provenance/P031-return-review.md",
    "/workspace/wtime/target/performance/P031/tooling-provenance/P031-runtime-tests-static-review.md",
    "/workspace/wtime/target/performance/P031/tooling-provenance/P031-wrapper-review.md",
    "/workspace/wtime/target/performance/P031/tooling-provenance/extract_p031-original-reconstructed.py",
    "/workspace/wtime/target/performance/P031/tooling-provenance/extract_p031-overwrite-recovery.py",
    "/workspace/wtime/target/performance/P031/tooling-provenance/extract_p031-recovery-review.md",
    "/workspace/wtime/target/performance/P031/tooling-provenance/extract_p031.py",
    "/workspace/wtime/target/performance/P031/tooling-provenance/test_extract_p031-after-repair.log",
    "/workspace/wtime/target/performance/P031/tooling-provenance/test_extract_p031-before-repair.log",
    "/workspace/wtime/target/performance/P031/tooling-provenance/test_extract_p031.py",
    "/workspace/wtime/target/performance/P031/tooling-provenance/test_p031_finalizer_settings.py"
  ],
  "candidate_promoted": false,
  "commands": [
    {
      "command": ". /workspace/.setup/rust-env.sh\npython3 tools/wtime-bench/compare.py --baseline target/performance/P031-before --candidate tools/wtime-bench/target/release/wtime-bench --candidate-mode decoded --selection scalar --pairs 30 --build-growth-explanation-file docs/benchmarks/performance/P031-build-decision.md --output target/performance/P031 > target/performance/P031-run.log 2>&1",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P031-run.log",
      "log_sha256": "a6651069c51ef66aeac29ba445569051f42bc608a7da852b9c2a5b9abea3b7c5",
      "name": "comparison"
    },
    {
      "command": ". /workspace/.setup/rust-env.sh\npython3 /tmp/extract_p031.py --report /workspace/wtime/target/performance/P031/report.json --checkpoint /workspace/wtime/target/performance/P031/checkpoint.jsonl --archive /workspace/wtime/target/performance/P031-before --candidate-dir /workspace/wtime/target/performance/P031-candidate-retained --output /workspace/wtime/target/performance/P031/extraction.json",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P031/extraction.log",
      "log_sha256": "d876a6553099c40825dafbe742848f09db0b637e6033632759d51db324387957",
      "name": "extraction",
      "stdout_path": "/workspace/wtime/target/performance/P031/extraction.log",
      "stdout_sha256": "d876a6553099c40825dafbe742848f09db0b637e6033632759d51db324387957"
    },
    {
      "command": ". /workspace/.setup/rust-env.sh\npython3 tools/wtime-bench/analyze.py --input target/performance/P031 --output target/performance/P031/analysis.json",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P031/analysis.log",
      "log_sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
      "name": "analysis",
      "stdout_path": "/workspace/wtime/target/performance/P031/analysis.log",
      "stdout_sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    },
    {
      "command": ". /workspace/.setup/rust-env.sh\npython3 tools/wtime-bench/gates.py --task P031 --comparison target/performance/P031 --build target/performance/P031/comparison-build.json --contract docs/benchmarks/performance/P031-contract.json --output target/performance/P031/decision.json",
      "exit_code": 1,
      "log_path": "/workspace/wtime/target/performance/P031/raw-gate.log",
      "log_sha256": "48f75246d87736a7ce7a0cf4b8c2cdeac68c1633b77673a66240f3c865b42ac2",
      "name": "raw_gate",
      "stdout_path": "/workspace/wtime/target/performance/P031/raw-gate.log",
      "stdout_sha256": "48f75246d87736a7ce7a0cf4b8c2cdeac68c1633b77673a66240f3c865b42ac2"
    },
    {
      "argv": [
        "cargo",
        "test",
        "--locked",
        "-p",
        "wtime-runtime"
      ],
      "command": ". /workspace/.setup/rust-env.sh\ncargo test --locked -p wtime-runtime",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P031/restored-default-tests.log",
      "log_sha256": "eeb778cb5449fe81a6b71f85dba2d438f72bb9e16ae91a7a74806a5873066879",
      "name": "restored_default_tests",
      "observed_tests": {
        "failed": 0,
        "ignored": 0,
        "passed": 211,
        "path": "/workspace/wtime/target/performance/P031/restored-default-tests.log",
        "sha256": "eeb778cb5449fe81a6b71f85dba2d438f72bb9e16ae91a7a74806a5873066879",
        "suite_summaries": 8
      }
    },
    {
      "argv": [
        "cargo",
        "test",
        "--locked",
        "-p",
        "wtime-runtime",
        "--features",
        "execution-reference,performance-counters",
        "--test",
        "performance"
      ],
      "command": ". /workspace/.setup/rust-env.sh\ncargo test --locked -p wtime-runtime --features execution-reference,performance-counters --test performance",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P031/restored-diagnostic-tests.log",
      "log_sha256": "6b4484529b6be3aeaff9e51ff311ab79de8fcc6b4ed44184cabc6dd6debb6508",
      "name": "restored_diagnostic_tests",
      "observed_tests": {
        "failed": 0,
        "ignored": 0,
        "passed": 80,
        "path": "/workspace/wtime/target/performance/P031/restored-diagnostic-tests.log",
        "sha256": "6b4484529b6be3aeaff9e51ff311ab79de8fcc6b4ed44184cabc6dd6debb6508",
        "suite_summaries": 1
      }
    },
    {
      "argv": [
        "cargo",
        "test",
        "--locked",
        "-p",
        "wtime-runtime",
        "p031_",
        "--",
        "--nocapture"
      ],
      "command": ". /workspace/.setup/rust-env.sh\ncargo test --locked -p wtime-runtime p031_ -- --nocapture",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P031/restored-focused-default-tests.log",
      "log_sha256": "973688971c4b981b977eb93bf0b93a7b0fe9b66d73ddb427bc319a962571bc51",
      "name": "restored_focused_default_tests",
      "observed_tests": {
        "failed": 0,
        "ignored": 0,
        "passed": 3,
        "path": "/workspace/wtime/target/performance/P031/restored-focused-default-tests.log",
        "sha256": "973688971c4b981b977eb93bf0b93a7b0fe9b66d73ddb427bc319a962571bc51",
        "suite_summaries": 7,
        "test_names": [
          "p031::p031_detached_continuation_survives_unrelated_call_and_is_one_shot",
          "p031::p031_errors_return_to_reuse_and_caught_gc_result_releases_exactly",
          "p031::p031_nested_host_reentry_preserves_outer_arguments_and_result"
        ]
      }
    },
    {
      "argv": [
        "cargo",
        "test",
        "--locked",
        "-p",
        "wtime-runtime",
        "--features",
        "execution-reference,performance-counters",
        "p031_",
        "--",
        "--nocapture"
      ],
      "command": ". /workspace/.setup/rust-env.sh\ncargo test --locked -p wtime-runtime --features execution-reference,performance-counters p031_ -- --nocapture",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P031/restored-focused-diagnostic-tests.log",
      "log_sha256": "533bff59b8af464596ce679edcb0113cbdae039d7f5dca611ad464d457f56bca",
      "name": "restored_focused_diagnostic_tests",
      "observed_tests": {
        "failed": 0,
        "ignored": 0,
        "passed": 4,
        "path": "/workspace/wtime/target/performance/P031/restored-focused-diagnostic-tests.log",
        "sha256": "533bff59b8af464596ce679edcb0113cbdae039d7f5dca611ad464d457f56bca",
        "suite_summaries": 7,
        "test_names": [
          "p031::p031_detached_continuation_survives_unrelated_call_and_is_one_shot",
          "p031::p031_nested_host_reentry_preserves_outer_arguments_and_result",
          "p031::p031_errors_return_to_reuse_and_caught_gc_result_releases_exactly",
          "p031::p031_reference_policy_runs_the_same_public_reentry_case"
        ]
      }
    },
    {
      "argv": [
        "cargo",
        "build",
        "--locked",
        "--release",
        "--manifest-path",
        "tools/wtime-bench/Cargo.toml",
        "-j",
        "4"
      ],
      "command": ". /workspace/.setup/rust-env.sh\ncargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml -j 4",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P031/restored-benchmark-build.log",
      "log_sha256": "2ed2f12bbdb3e053529c27d2a8fc4cfa51e244f2afe41c8f5003ce176f78f2f8",
      "name": "rollback_benchmark_build"
    },
    {
      "argv": [
        "python3",
        "tools/checks/build_budget.py",
        "--output",
        "target/performance/P031/restored-build.json"
      ],
      "command": ". /workspace/.setup/rust-env.sh\npython3 tools/checks/build_budget.py --output target/performance/P031/restored-build.json",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P031/restored-build.log",
      "log_sha256": "18a57999e8b2fd369cd75ded7d0558b90c0c4846b57af82ff8b9accea5374b43",
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
    "legacy_scalar_calibration_records": 6,
    "legacy_scalar_calibration_workloads": 3,
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
    "typed_scalar_calibration_records": 164,
    "typed_scalar_calibration_workloads": 82,
    "warm_calibration_attempt_checks": 198,
    "warm_calibration_policy_limit": "All 85 scalar workloads use bounded shared-batch calibration with both sides confirmed at or above 20 ms during calibration (164 typed-driver records and six legacy single-case records). Later timed observations can fall below 20 ms.",
    "warm_calibration_records": 170,
    "warm_calibration_records_by_case_run": {
      "0:branch_calls": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 4,
            "sides": [
              {
                "calibrated_total_ns": 23964410.0,
                "ns_per_operation": 5991102.5,
                "observed_batch": 4,
                "requested_batch": 4,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 21746488.0,
                "ns_per_operation": 5436622.0,
                "observed_batch": 4,
                "requested_batch": 4,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 23964410.0,
            "ns_per_operation": 5991102.5,
            "observed_batch": 4,
            "requested_batch": 4,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 21746488.0,
            "ns_per_operation": 5436622.0,
            "observed_batch": 4,
            "requested_batch": 4,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 37202318.0,
            "ns_per_operation": 9300579.5,
            "observed_batch": 4,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 23794569.0,
            "ns_per_operation": 5948642.25,
            "observed_batch": 4,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 4,
        "source_sha256": "d0af4fdf9ad5be38344267ae72fc1855edb5a8fa1b475fd0af4cb507730b2309",
        "wasm_sha256": "9b58d26f201900b5b6cce684133a98ab68b3cf55d979b64ce895e914e9d5327b",
        "workload_kind": "legacy_single_case_control"
      },
      "0:call_overhead": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 45455,
            "sides": [
              {
                "calibrated_total_ns": 16680530.440000001,
                "ns_per_operation": 366.968,
                "observed_batch": 45455,
                "requested_batch": 45455,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 20385022.03,
                "ns_per_operation": 448.466,
                "observed_batch": 45455,
                "requested_batch": 45455,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 59951,
            "sides": [
              {
                "calibrated_total_ns": 21208505.564,
                "ns_per_operation": 353.764,
                "observed_batch": 59951,
                "requested_batch": 59951,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 26421185.063,
                "ns_per_operation": 440.713,
                "observed_batch": 59951,
                "requested_batch": 59951,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 21208505.564,
            "ns_per_operation": 353.764,
            "observed_batch": 59951,
            "requested_batch": 59951,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 26421185.063,
            "ns_per_operation": 440.713,
            "observed_batch": 59951,
            "requested_batch": 59951,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 16165343.47,
            "ns_per_operation": 355.634,
            "observed_batch": 45455,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 17520114.664,
            "ns_per_operation": 504.568,
            "observed_batch": 34723,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 59951,
        "source_sha256": "9493b8f751690a2fd6e8b19dcee5e2c1d50b717253d292ce34d5692b306663a5",
        "wasm_sha256": "0815ca7d6026206d0773fe10c6b9643862bfffe8ba361c9789369ef9668ee746",
        "workload_kind": "legacy_single_case_control"
      },
      "0:integer_loop": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 3,
            "sides": [
              {
                "calibrated_total_ns": 20051877.0,
                "ns_per_operation": 6683959.0,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 20347628.001000002,
                "ns_per_operation": 6782542.667,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 20051877.0,
            "ns_per_operation": 6683959.0,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 20347628.001000002,
            "ns_per_operation": 6782542.667,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 20664720.998999998,
            "ns_per_operation": 6888240.333,
            "observed_batch": 3,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 21028895.001000002,
            "ns_per_operation": 7009631.667,
            "observed_batch": 3,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 3,
        "source_sha256": "791730112109f3d5751a7b3e2c1134e7e0e6f71a7f7ee01ea8481eb989ee3fa4",
        "wasm_sha256": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c",
        "workload_kind": "legacy_single_case_control"
      },
      "0:scalar_accumulator_n0": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 30770,
            "sides": [
              {
                "calibrated_total_ns": 20092994.619999997,
                "ns_per_operation": 653.006,
                "observed_batch": 30770,
                "requested_batch": 30770,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 20668301.31,
                "ns_per_operation": 671.703,
                "observed_batch": 30770,
                "requested_batch": 30770,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 20092994.619999997,
            "ns_per_operation": 653.006,
            "observed_batch": 30770,
            "requested_batch": 30770,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 20668301.31,
            "ns_per_operation": 671.703,
            "observed_batch": 30770,
            "requested_batch": 30770,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 6621165.984,
            "ns_per_operation": 690.568,
            "observed_batch": 9588,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 19615659.610000003,
            "ns_per_operation": 637.493,
            "observed_batch": 30770,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 30770,
        "source_sha256": "61aaec7a86d481378f31f3e27b0c1413684065841fa5909297181fcad9bef10d",
        "wasm_sha256": "ef61bf7afc44af7b71ef78eadcba48c0d85949d7d9096df8fbbbab53c05036b1",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_accumulator_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 17347,
            "sides": [
              {
                "calibrated_total_ns": 60114987.68,
                "ns_per_operation": 3465.44,
                "observed_batch": 17347,
                "requested_batch": 17347,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 36424172.433,
                "ns_per_operation": 2099.739,
                "observed_batch": 17347,
                "requested_batch": 17347,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 60114987.68,
            "ns_per_operation": 3465.44,
            "observed_batch": 17347,
            "requested_batch": 17347,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 36424172.433,
            "ns_per_operation": 2099.739,
            "observed_batch": 17347,
            "requested_batch": 17347,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 12050965.512,
            "ns_per_operation": 1557.576,
            "observed_batch": 7737,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 131139017.944,
            "ns_per_operation": 7559.752,
            "observed_batch": 17347,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 17347,
        "source_sha256": "61aaec7a86d481378f31f3e27b0c1413684065841fa5909297181fcad9bef10d",
        "wasm_sha256": "ef61bf7afc44af7b71ef78eadcba48c0d85949d7d9096df8fbbbab53c05036b1",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_accumulator_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 4,
            "sides": [
              {
                "calibrated_total_ns": 33569443.0,
                "ns_per_operation": 8392360.75,
                "observed_batch": 4,
                "requested_batch": 4,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 22059847.0,
                "ns_per_operation": 5514961.75,
                "observed_batch": 4,
                "requested_batch": 4,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 33569443.0,
            "ns_per_operation": 8392360.75,
            "observed_batch": 4,
            "requested_batch": 4,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22059847.0,
            "ns_per_operation": 5514961.75,
            "observed_batch": 4,
            "requested_batch": 4,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 21647617.0,
            "ns_per_operation": 5411904.25,
            "observed_batch": 4,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 35266665.0,
            "ns_per_operation": 8816666.25,
            "observed_batch": 4,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 4,
        "source_sha256": "61aaec7a86d481378f31f3e27b0c1413684065841fa5909297181fcad9bef10d",
        "wasm_sha256": "ef61bf7afc44af7b71ef78eadcba48c0d85949d7d9096df8fbbbab53c05036b1",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_accumulator_n1024": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 50,
            "sides": [
              {
                "calibrated_total_ns": 33073003.000000004,
                "ns_per_operation": 661460.06,
                "observed_batch": 50,
                "requested_batch": 50,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 31103462.0,
                "ns_per_operation": 622069.24,
                "observed_batch": 50,
                "requested_batch": 50,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 33073003.000000004,
            "ns_per_operation": 661460.06,
            "observed_batch": 50,
            "requested_batch": 50,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 31103462.0,
            "ns_per_operation": 622069.24,
            "observed_batch": 50,
            "requested_batch": 50,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 18640420.992,
            "ns_per_operation": 517789.472,
            "observed_batch": 36,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 33073453.999999996,
            "ns_per_operation": 661469.08,
            "observed_batch": 50,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 50,
        "source_sha256": "61aaec7a86d481378f31f3e27b0c1413684065841fa5909297181fcad9bef10d",
        "wasm_sha256": "ef61bf7afc44af7b71ef78eadcba48c0d85949d7d9096df8fbbbab53c05036b1",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_accumulator_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 2687,
            "sides": [
              {
                "calibrated_total_ns": 29687778.977,
                "ns_per_operation": 11048.671,
                "observed_batch": 2687,
                "requested_batch": 2687,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 31230659.751,
                "ns_per_operation": 11622.873,
                "observed_batch": 2687,
                "requested_batch": 2687,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 29687778.977,
            "ns_per_operation": 11048.671,
            "observed_batch": 2687,
            "requested_batch": 2687,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 31230659.751,
            "ns_per_operation": 11622.873,
            "observed_batch": 2687,
            "requested_batch": 2687,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 18881334.654000003,
            "ns_per_operation": 10080.798,
            "observed_batch": 1873,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 29058883.94,
            "ns_per_operation": 10814.62,
            "observed_batch": 2687,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 2687,
        "source_sha256": "61aaec7a86d481378f31f3e27b0c1413684065841fa5909297181fcad9bef10d",
        "wasm_sha256": "ef61bf7afc44af7b71ef78eadcba48c0d85949d7d9096df8fbbbab53c05036b1",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_accumulator_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1504,
            "sides": [
              {
                "calibrated_total_ns": 45083284.352,
                "ns_per_operation": 29975.588,
                "observed_batch": 1504,
                "requested_batch": 1504,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 32843625.568,
                "ns_per_operation": 21837.517,
                "observed_batch": 1504,
                "requested_batch": 1504,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 45083284.352,
            "ns_per_operation": 29975.588,
            "observed_batch": 1504,
            "requested_batch": 1504,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 32843625.568,
            "ns_per_operation": 21837.517,
            "observed_batch": 1504,
            "requested_batch": 1504,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 25650119.572,
            "ns_per_operation": 18805.073,
            "observed_batch": 1364,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 29362413.024,
            "ns_per_operation": 19522.881,
            "observed_batch": 1504,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1504,
        "source_sha256": "61aaec7a86d481378f31f3e27b0c1413684065841fa5909297181fcad9bef10d",
        "wasm_sha256": "ef61bf7afc44af7b71ef78eadcba48c0d85949d7d9096df8fbbbab53c05036b1",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_br_table_n0": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 33445,
            "sides": [
              {
                "calibrated_total_ns": 20331315.835,
                "ns_per_operation": 607.903,
                "observed_batch": 33445,
                "requested_batch": 33445,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 21696273.175,
                "ns_per_operation": 648.715,
                "observed_batch": 33445,
                "requested_batch": 33445,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 20331315.835,
            "ns_per_operation": 607.903,
            "observed_batch": 33445,
            "requested_batch": 33445,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 21696273.175,
            "ns_per_operation": 648.715,
            "observed_batch": 33445,
            "requested_batch": 33445,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 20932215.599999998,
            "ns_per_operation": 680.28,
            "observed_batch": 30770,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 21707243.135,
            "ns_per_operation": 649.043,
            "observed_batch": 33445,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 33445,
        "source_sha256": "637b96cd8926ffadf728af4949b43fa6b62fffb61968c14c716e0c203a1e5e6d",
        "wasm_sha256": "d43d5dd71d5ca1c3dae820aa85c13a2d69415e028b8d48097fed4558f743163a",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_br_table_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 12780,
            "sides": [
              {
                "calibrated_total_ns": 32004813.42,
                "ns_per_operation": 2504.289,
                "observed_batch": 12780,
                "requested_batch": 12780,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 24255494.279999997,
                "ns_per_operation": 1897.926,
                "observed_batch": 12780,
                "requested_batch": 12780,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 32004813.42,
            "ns_per_operation": 2504.289,
            "observed_batch": 12780,
            "requested_batch": 12780,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 24255494.279999997,
            "ns_per_operation": 1897.926,
            "observed_batch": 12780,
            "requested_batch": 12780,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 8364284.133,
            "ns_per_operation": 2040.567,
            "observed_batch": 4099,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 26434305.359999996,
            "ns_per_operation": 2068.412,
            "observed_batch": 12780,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 12780,
        "source_sha256": "637b96cd8926ffadf728af4949b43fa6b62fffb61968c14c716e0c203a1e5e6d",
        "wasm_sha256": "d43d5dd71d5ca1c3dae820aa85c13a2d69415e028b8d48097fed4558f743163a",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_br_table_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 2,
            "sides": [
              {
                "calibrated_total_ns": 22389893.0,
                "ns_per_operation": 11194946.5,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 22649875.0,
                "ns_per_operation": 11324937.5,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 22389893.0,
            "ns_per_operation": 11194946.5,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22649875.0,
            "ns_per_operation": 11324937.5,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 23876458.0,
            "ns_per_operation": 11938229.0,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22050502.0,
            "ns_per_operation": 11025251.0,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 2,
        "source_sha256": "637b96cd8926ffadf728af4949b43fa6b62fffb61968c14c716e0c203a1e5e6d",
        "wasm_sha256": "d43d5dd71d5ca1c3dae820aa85c13a2d69415e028b8d48097fed4558f743163a",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_br_table_n1024": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 15,
            "sides": [
              {
                "calibrated_total_ns": 21326206.005,
                "ns_per_operation": 1421747.067,
                "observed_batch": 15,
                "requested_batch": 15,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 21230208.0,
                "ns_per_operation": 1415347.2,
                "observed_batch": 15,
                "requested_batch": 15,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 21326206.005,
            "ns_per_operation": 1421747.067,
            "observed_batch": 15,
            "requested_batch": 15,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 21230208.0,
            "ns_per_operation": 1415347.2,
            "observed_batch": 15,
            "requested_batch": 15,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 15180589.996,
            "ns_per_operation": 1380053.636,
            "observed_batch": 11,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 17678487.0,
            "ns_per_operation": 1178565.8,
            "observed_batch": 15,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 15,
        "source_sha256": "637b96cd8926ffadf728af4949b43fa6b62fffb61968c14c716e0c203a1e5e6d",
        "wasm_sha256": "d43d5dd71d5ca1c3dae820aa85c13a2d69415e028b8d48097fed4558f743163a",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_br_table_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1453,
            "sides": [
              {
                "calibrated_total_ns": 33898484.188,
                "ns_per_operation": 23329.996,
                "observed_batch": 1453,
                "requested_batch": 1453,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 28651795.633,
                "ns_per_operation": 19719.061,
                "observed_batch": 1453,
                "requested_batch": 1453,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 33898484.188,
            "ns_per_operation": 23329.996,
            "observed_batch": 1453,
            "requested_batch": 1453,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 28651795.633,
            "ns_per_operation": 19719.061,
            "observed_batch": 1453,
            "requested_batch": 1453,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 78801767.436,
            "ns_per_operation": 62690.348,
            "observed_batch": 1257,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 37229536.089999996,
            "ns_per_operation": 25622.53,
            "observed_batch": 1453,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1453,
        "source_sha256": "637b96cd8926ffadf728af4949b43fa6b62fffb61968c14c716e0c203a1e5e6d",
        "wasm_sha256": "d43d5dd71d5ca1c3dae820aa85c13a2d69415e028b8d48097fed4558f743163a",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_br_table_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 773,
            "sides": [
              {
                "calibrated_total_ns": 34875826.727000006,
                "ns_per_operation": 45117.499,
                "observed_batch": 773,
                "requested_batch": 773,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 32925318.145999998,
                "ns_per_operation": 42594.202,
                "observed_batch": 773,
                "requested_batch": 773,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 34875826.727000006,
            "ns_per_operation": 45117.499,
            "observed_batch": 773,
            "requested_batch": 773,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 32925318.145999998,
            "ns_per_operation": 42594.202,
            "observed_batch": 773,
            "requested_batch": 773,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 44609485.025,
            "ns_per_operation": 59558.725,
            "observed_batch": 749,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 33662604.772999994,
            "ns_per_operation": 43548.001,
            "observed_batch": 773,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 773,
        "source_sha256": "637b96cd8926ffadf728af4949b43fa6b62fffb61968c14c716e0c203a1e5e6d",
        "wasm_sha256": "d43d5dd71d5ca1c3dae820aa85c13a2d69415e028b8d48097fed4558f743163a",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_branch_entropy_n0": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 23867,
            "sides": [
              {
                "calibrated_total_ns": 16422118.956,
                "ns_per_operation": 688.068,
                "observed_batch": 23867,
                "requested_batch": 23867,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 20779469.412,
                "ns_per_operation": 870.636,
                "observed_batch": 23867,
                "requested_batch": 23867,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 31974,
            "sides": [
              {
                "calibrated_total_ns": 24456592.86,
                "ns_per_operation": 764.89,
                "observed_batch": 31974,
                "requested_batch": 31974,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 20875313.016,
                "ns_per_operation": 652.884,
                "observed_batch": 31974,
                "requested_batch": 31974,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 24456592.86,
            "ns_per_operation": 764.89,
            "observed_batch": 31974,
            "requested_batch": 31974,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 20875313.016,
            "ns_per_operation": 652.884,
            "observed_batch": 31974,
            "requested_batch": 31974,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 15681502.079,
            "ns_per_operation": 657.037,
            "observed_batch": 23867,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 7329265.340000001,
            "ns_per_operation": 694.388,
            "observed_batch": 10555,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 31974,
        "source_sha256": "6b4861d8bfbbd7e2718e577ec2d479bfff32e94dc47a3b7dda9a1b363baa98b6",
        "wasm_sha256": "e4c87dbab50a2438cb1f749bc6fd6f9c834c81484acd42864735bd1ad1b6dec0",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_branch_entropy_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 11555,
            "sides": [
              {
                "calibrated_total_ns": 21726438.965,
                "ns_per_operation": 1880.263,
                "observed_batch": 11555,
                "requested_batch": 11555,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 21425373.44,
                "ns_per_operation": 1854.208,
                "observed_batch": 11555,
                "requested_batch": 11555,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 21726438.965,
            "ns_per_operation": 1880.263,
            "observed_batch": 11555,
            "requested_batch": 11555,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 21425373.44,
            "ns_per_operation": 1854.208,
            "observed_batch": 11555,
            "requested_batch": 11555,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 21012605.73,
            "ns_per_operation": 1818.486,
            "observed_batch": 11555,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 14141741.341,
            "ns_per_operation": 2104.739,
            "observed_batch": 6719,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 11555,
        "source_sha256": "6b4861d8bfbbd7e2718e577ec2d479bfff32e94dc47a3b7dda9a1b363baa98b6",
        "wasm_sha256": "e4c87dbab50a2438cb1f749bc6fd6f9c834c81484acd42864735bd1ad1b6dec0",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_branch_entropy_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 2,
            "sides": [
              {
                "calibrated_total_ns": 36424161.0,
                "ns_per_operation": 18212080.5,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 43255941.0,
                "ns_per_operation": 21627970.5,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 36424161.0,
            "ns_per_operation": 18212080.5,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 43255941.0,
            "ns_per_operation": 21627970.5,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 25938684.0,
            "ns_per_operation": 12969342.0,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 30952943.0,
            "ns_per_operation": 15476471.5,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 2,
        "source_sha256": "6b4861d8bfbbd7e2718e577ec2d479bfff32e94dc47a3b7dda9a1b363baa98b6",
        "wasm_sha256": "e4c87dbab50a2438cb1f749bc6fd6f9c834c81484acd42864735bd1ad1b6dec0",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_branch_entropy_n1024": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 25,
            "sides": [
              {
                "calibrated_total_ns": 27626352.0,
                "ns_per_operation": 1105054.08,
                "observed_batch": 25,
                "requested_batch": 25,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 28987603.000000004,
                "ns_per_operation": 1159504.12,
                "observed_batch": 25,
                "requested_batch": 25,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 27626352.0,
            "ns_per_operation": 1105054.08,
            "observed_batch": 25,
            "requested_batch": 25,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 28987603.000000004,
            "ns_per_operation": 1159504.12,
            "observed_batch": 25,
            "requested_batch": 25,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 30158078.999999996,
            "ns_per_operation": 1206323.16,
            "observed_batch": 25,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 20517158.0,
            "ns_per_operation": 1282322.375,
            "observed_batch": 16,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 25,
        "source_sha256": "6b4861d8bfbbd7e2718e577ec2d479bfff32e94dc47a3b7dda9a1b363baa98b6",
        "wasm_sha256": "e4c87dbab50a2438cb1f749bc6fd6f9c834c81484acd42864735bd1ad1b6dec0",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_branch_entropy_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1422,
            "sides": [
              {
                "calibrated_total_ns": 30910032.485999998,
                "ns_per_operation": 21737.013,
                "observed_batch": 1422,
                "requested_batch": 1422,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 34841906.568,
                "ns_per_operation": 24502.044,
                "observed_batch": 1422,
                "requested_batch": 1422,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 30910032.485999998,
            "ns_per_operation": 21737.013,
            "observed_batch": 1422,
            "requested_batch": 1422,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 34841906.568,
            "ns_per_operation": 24502.044,
            "observed_batch": 1422,
            "requested_batch": 1422,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 34699706.568,
            "ns_per_operation": 24402.044,
            "observed_batch": 1422,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 34632938.016,
            "ns_per_operation": 26722.946,
            "observed_batch": 1296,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1422,
        "source_sha256": "6b4861d8bfbbd7e2718e577ec2d479bfff32e94dc47a3b7dda9a1b363baa98b6",
        "wasm_sha256": "e4c87dbab50a2438cb1f749bc6fd6f9c834c81484acd42864735bd1ad1b6dec0",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_branch_entropy_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 756,
            "sides": [
              {
                "calibrated_total_ns": 28693431.143999998,
                "ns_per_operation": 37954.274,
                "observed_batch": 756,
                "requested_batch": 756,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 27883426.283999998,
                "ns_per_operation": 36882.839,
                "observed_batch": 756,
                "requested_batch": 756,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 28693431.143999998,
            "ns_per_operation": 37954.274,
            "observed_batch": 756,
            "requested_batch": 756,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27883426.283999998,
            "ns_per_operation": 36882.839,
            "observed_batch": 756,
            "requested_batch": 756,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 29973311.928,
            "ns_per_operation": 39647.238,
            "observed_batch": 756,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 44497454.808,
            "ns_per_operation": 59808.407,
            "observed_batch": 744,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 756,
        "source_sha256": "6b4861d8bfbbd7e2718e577ec2d479bfff32e94dc47a3b7dda9a1b363baa98b6",
        "wasm_sha256": "e4c87dbab50a2438cb1f749bc6fd6f9c834c81484acd42864735bd1ad1b6dec0",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_call_ref_n0": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 17168,
            "sides": [
              {
                "calibrated_total_ns": 13336668.944,
                "ns_per_operation": 776.833,
                "observed_batch": 17168,
                "requested_batch": 17168,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 15420709.632000001,
                "ns_per_operation": 898.224,
                "observed_batch": 17168,
                "requested_batch": 17168,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 28321,
            "sides": [
              {
                "calibrated_total_ns": 22070017.200999998,
                "ns_per_operation": 779.281,
                "observed_batch": 28321,
                "requested_batch": 28321,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 26781753.65,
                "ns_per_operation": 945.65,
                "observed_batch": 28321,
                "requested_batch": 28321,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 22070017.200999998,
            "ns_per_operation": 779.281,
            "observed_batch": 28321,
            "requested_batch": 28321,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 26781753.65,
            "ns_per_operation": 945.65,
            "observed_batch": 28321,
            "requested_batch": 28321,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 14776772.288,
            "ns_per_operation": 860.716,
            "observed_batch": 17168,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 5954748.24,
            "ns_per_operation": 942.207,
            "observed_batch": 6320,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 28321,
        "source_sha256": "0323154b79000eeeb0a63310541ec91b6c4d91e32a6a3406125a30f2c8e53a12",
        "wasm_sha256": "4f766dc9e29324ea942c892982363dec0ade98565288878a70967ce3b33f322b",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_call_ref_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 13765,
            "sides": [
              {
                "calibrated_total_ns": 23212181.035,
                "ns_per_operation": 1686.319,
                "observed_batch": 13765,
                "requested_batch": 13765,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 25023889.04,
                "ns_per_operation": 1817.936,
                "observed_batch": 13765,
                "requested_batch": 13765,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 23212181.035,
            "ns_per_operation": 1686.319,
            "observed_batch": 13765,
            "requested_batch": 13765,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 25023889.04,
            "ns_per_operation": 1817.936,
            "observed_batch": 13765,
            "requested_batch": 13765,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 22321358.954,
            "ns_per_operation": 1815.778,
            "observed_batch": 12293,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 24035204.15,
            "ns_per_operation": 1746.11,
            "observed_batch": 13765,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 13765,
        "source_sha256": "0323154b79000eeeb0a63310541ec91b6c4d91e32a6a3406125a30f2c8e53a12",
        "wasm_sha256": "4f766dc9e29324ea942c892982363dec0ade98565288878a70967ce3b33f322b",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_call_ref_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 2,
            "sides": [
              {
                "calibrated_total_ns": 20431751.0,
                "ns_per_operation": 10215875.5,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 22435974.0,
                "ns_per_operation": 11217987.0,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 20431751.0,
            "ns_per_operation": 10215875.5,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22435974.0,
            "ns_per_operation": 11217987.0,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 21316751.0,
            "ns_per_operation": 10658375.5,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 21523041.0,
            "ns_per_operation": 10761520.5,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 2,
        "source_sha256": "0323154b79000eeeb0a63310541ec91b6c4d91e32a6a3406125a30f2c8e53a12",
        "wasm_sha256": "4f766dc9e29324ea942c892982363dec0ade98565288878a70967ce3b33f322b",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_call_ref_n1024": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 17,
            "sides": [
              {
                "calibrated_total_ns": 19113942.996,
                "ns_per_operation": 1124349.588,
                "observed_batch": 17,
                "requested_batch": 17,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 25535483.998,
                "ns_per_operation": 1502087.294,
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
                "calibrated_total_ns": 24978255.0,
                "ns_per_operation": 1248912.75,
                "observed_batch": 20,
                "requested_batch": 20,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 43484963.0,
                "ns_per_operation": 2174248.15,
                "observed_batch": 20,
                "requested_batch": 20,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 24978255.0,
            "ns_per_operation": 1248912.75,
            "observed_batch": 20,
            "requested_batch": 20,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 43484963.0,
            "ns_per_operation": 2174248.15,
            "observed_batch": 20,
            "requested_batch": 20,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 15179284.995,
            "ns_per_operation": 1011952.333,
            "observed_batch": 15,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 21600407.995,
            "ns_per_operation": 1270612.235,
            "observed_batch": 17,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 20,
        "source_sha256": "0323154b79000eeeb0a63310541ec91b6c4d91e32a6a3406125a30f2c8e53a12",
        "wasm_sha256": "4f766dc9e29324ea942c892982363dec0ade98565288878a70967ce3b33f322b",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_call_ref_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1594,
            "sides": [
              {
                "calibrated_total_ns": 30977950.618,
                "ns_per_operation": 19434.097,
                "observed_batch": 1594,
                "requested_batch": 1594,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 34482378.746,
                "ns_per_operation": 21632.609,
                "observed_batch": 1594,
                "requested_batch": 1594,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 30977950.618,
            "ns_per_operation": 19434.097,
            "observed_batch": 1594,
            "requested_batch": 1594,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 34482378.746,
            "ns_per_operation": 21632.609,
            "observed_batch": 1594,
            "requested_batch": 1594,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 2896387.019,
            "ns_per_operation": 22806.197,
            "observed_batch": 127,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 31505787.778,
            "ns_per_operation": 19765.237,
            "observed_batch": 1594,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1594,
        "source_sha256": "0323154b79000eeeb0a63310541ec91b6c4d91e32a6a3406125a30f2c8e53a12",
        "wasm_sha256": "4f766dc9e29324ea942c892982363dec0ade98565288878a70967ce3b33f322b",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_call_ref_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 758,
            "sides": [
              {
                "calibrated_total_ns": 36549052.984,
                "ns_per_operation": 48217.748,
                "observed_batch": 758,
                "requested_batch": 758,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 30150167.068000004,
                "ns_per_operation": 39775.946,
                "observed_batch": 758,
                "requested_batch": 758,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 36549052.984,
            "ns_per_operation": 48217.748,
            "observed_batch": 758,
            "requested_batch": 758,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 30150167.068000004,
            "ns_per_operation": 39775.946,
            "observed_batch": 758,
            "requested_batch": 758,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 19456607.82,
            "ns_per_operation": 49888.738,
            "observed_batch": 390,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 39025097.884,
            "ns_per_operation": 51484.298,
            "observed_batch": 758,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 758,
        "source_sha256": "0323154b79000eeeb0a63310541ec91b6c4d91e32a6a3406125a30f2c8e53a12",
        "wasm_sha256": "4f766dc9e29324ea942c892982363dec0ade98565288878a70967ce3b33f322b",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_dependent_n0": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 29456,
            "sides": [
              {
                "calibrated_total_ns": 21249057.648000002,
                "ns_per_operation": 721.383,
                "observed_batch": 29456,
                "requested_batch": 29456,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 21851727.408,
                "ns_per_operation": 741.843,
                "observed_batch": 29456,
                "requested_batch": 29456,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 21249057.648000002,
            "ns_per_operation": 721.383,
            "observed_batch": 29456,
            "requested_batch": 29456,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 21851727.408,
            "ns_per_operation": 741.843,
            "observed_batch": 29456,
            "requested_batch": 29456,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 17782835.304,
            "ns_per_operation": 756.652,
            "observed_batch": 23502,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 21198334.415999997,
            "ns_per_operation": 719.661,
            "observed_batch": 29456,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 29456,
        "source_sha256": "0dd5e9778eb09bac9ee432e5fece3f432d034ea9878d2302972142501f0c60c6",
        "wasm_sha256": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_dependent_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 17422,
            "sides": [
              {
                "calibrated_total_ns": 41640931.970000006,
                "ns_per_operation": 2390.135,
                "observed_batch": 17422,
                "requested_batch": 17422,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 31446152.496000003,
                "ns_per_operation": 1804.968,
                "observed_batch": 17422,
                "requested_batch": 17422,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 41640931.970000006,
            "ns_per_operation": 2390.135,
            "observed_batch": 17422,
            "requested_batch": 17422,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 31446152.496000003,
            "ns_per_operation": 1804.968,
            "observed_batch": 17422,
            "requested_batch": 17422,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 26662538.327999998,
            "ns_per_operation": 1783.686,
            "observed_batch": 14948,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 26558985.322,
            "ns_per_operation": 1524.451,
            "observed_batch": 17422,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 17422,
        "source_sha256": "0dd5e9778eb09bac9ee432e5fece3f432d034ea9878d2302972142501f0c60c6",
        "wasm_sha256": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_dependent_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 4,
            "sides": [
              {
                "calibrated_total_ns": 25875282.0,
                "ns_per_operation": 6468820.5,
                "observed_batch": 4,
                "requested_batch": 4,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 28526736.0,
                "ns_per_operation": 7131684.0,
                "observed_batch": 4,
                "requested_batch": 4,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 25875282.0,
            "ns_per_operation": 6468820.5,
            "observed_batch": 4,
            "requested_batch": 4,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 28526736.0,
            "ns_per_operation": 7131684.0,
            "observed_batch": 4,
            "requested_batch": 4,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 24521919.0,
            "ns_per_operation": 6130479.75,
            "observed_batch": 4,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 28905921.0,
            "ns_per_operation": 9635307.0,
            "observed_batch": 3,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 4,
        "source_sha256": "0dd5e9778eb09bac9ee432e5fece3f432d034ea9878d2302972142501f0c60c6",
        "wasm_sha256": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_dependent_n1024": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 38,
            "sides": [
              {
                "calibrated_total_ns": 22588526.01,
                "ns_per_operation": 594434.895,
                "observed_batch": 38,
                "requested_batch": 38,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 23025064.994000003,
                "ns_per_operation": 605922.763,
                "observed_batch": 38,
                "requested_batch": 38,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 22588526.01,
            "ns_per_operation": 594434.895,
            "observed_batch": 38,
            "requested_batch": 38,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 23025064.994000003,
            "ns_per_operation": 605922.763,
            "observed_batch": 38,
            "requested_batch": 38,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 32133611.004,
            "ns_per_operation": 945106.206,
            "observed_batch": 34,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 37528262.984,
            "ns_per_operation": 987585.868,
            "observed_batch": 38,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 38,
        "source_sha256": "0dd5e9778eb09bac9ee432e5fece3f432d034ea9878d2302972142501f0c60c6",
        "wasm_sha256": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_dependent_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 2355,
            "sides": [
              {
                "calibrated_total_ns": 26444158.409999996,
                "ns_per_operation": 11228.942,
                "observed_batch": 2355,
                "requested_batch": 2355,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 25913998.455,
                "ns_per_operation": 11003.821,
                "observed_batch": 2355,
                "requested_batch": 2355,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 26444158.409999996,
            "ns_per_operation": 11228.942,
            "observed_batch": 2355,
            "requested_batch": 2355,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 25913998.455,
            "ns_per_operation": 11003.821,
            "observed_batch": 2355,
            "requested_batch": 2355,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 26462475.599999998,
            "ns_per_operation": 11236.72,
            "observed_batch": 2355,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 32845992.25,
            "ns_per_operation": 14042.75,
            "observed_batch": 2339,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 2355,
        "source_sha256": "0dd5e9778eb09bac9ee432e5fece3f432d034ea9878d2302972142501f0c60c6",
        "wasm_sha256": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_dependent_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1317,
            "sides": [
              {
                "calibrated_total_ns": 29930423.838,
                "ns_per_operation": 22726.214,
                "observed_batch": 1317,
                "requested_batch": 1317,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 32599254.537,
                "ns_per_operation": 24752.661,
                "observed_batch": 1317,
                "requested_batch": 1317,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 29930423.838,
            "ns_per_operation": 22726.214,
            "observed_batch": 1317,
            "requested_batch": 1317,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 32599254.537,
            "ns_per_operation": 24752.661,
            "observed_batch": 1317,
            "requested_batch": 1317,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 30693830.79,
            "ns_per_operation": 23305.87,
            "observed_batch": 1317,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 5092250.999,
            "ns_per_operation": 42084.719,
            "observed_batch": 121,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1317,
        "source_sha256": "0dd5e9778eb09bac9ee432e5fece3f432d034ea9878d2302972142501f0c60c6",
        "wasm_sha256": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_direct_calls_n0": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 28369,
            "sides": [
              {
                "calibrated_total_ns": 29329914.768000003,
                "ns_per_operation": 1033.872,
                "observed_batch": 28369,
                "requested_batch": 28369,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 21876896.195,
                "ns_per_operation": 771.155,
                "observed_batch": 28369,
                "requested_batch": 28369,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 29329914.768000003,
            "ns_per_operation": 1033.872,
            "observed_batch": 28369,
            "requested_batch": 28369,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 21876896.195,
            "ns_per_operation": 771.155,
            "observed_batch": 28369,
            "requested_batch": 28369,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 19612697.062000003,
            "ns_per_operation": 728.609,
            "observed_batch": 26918,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 21183926.632,
            "ns_per_operation": 746.728,
            "observed_batch": 28369,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 28369,
        "source_sha256": "0a075601a2a651f9cb95e8ee0404c64bbf076d17bf242b9c88fecbb8cb973826",
        "wasm_sha256": "f6adbecce99c05bf06f213f4f5c0d5d9f001074acbbb2e6bbe814696bb866e45",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_direct_calls_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 14610,
            "sides": [
              {
                "calibrated_total_ns": 26068783.709999997,
                "ns_per_operation": 1784.311,
                "observed_batch": 14610,
                "requested_batch": 14610,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 26990616.27,
                "ns_per_operation": 1847.407,
                "observed_batch": 14610,
                "requested_batch": 14610,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 26068783.709999997,
            "ns_per_operation": 1784.311,
            "observed_batch": 14610,
            "requested_batch": 14610,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 26990616.27,
            "ns_per_operation": 1847.407,
            "observed_batch": 14610,
            "requested_batch": 14610,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 24889458.924,
            "ns_per_operation": 1821.802,
            "observed_batch": 13662,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27417754.23,
            "ns_per_operation": 1876.643,
            "observed_batch": 14610,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 14610,
        "source_sha256": "0a075601a2a651f9cb95e8ee0404c64bbf076d17bf242b9c88fecbb8cb973826",
        "wasm_sha256": "f6adbecce99c05bf06f213f4f5c0d5d9f001074acbbb2e6bbe814696bb866e45",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_direct_calls_n10000": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 2,
            "sides": [
              {
                "calibrated_total_ns": 19888284.0,
                "ns_per_operation": 9944142.0,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 21892321.0,
                "ns_per_operation": 10946160.5,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 3,
            "sides": [
              {
                "calibrated_total_ns": 28986158.001,
                "ns_per_operation": 9662052.667,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 30392844.0,
                "ns_per_operation": 10130948.0,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 28986158.001,
            "ns_per_operation": 9662052.667,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 30392844.0,
            "ns_per_operation": 10130948.0,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 26481086.0,
            "ns_per_operation": 13240543.0,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 28121137.0,
            "ns_per_operation": 14060568.5,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 3,
        "source_sha256": "0a075601a2a651f9cb95e8ee0404c64bbf076d17bf242b9c88fecbb8cb973826",
        "wasm_sha256": "f6adbecce99c05bf06f213f4f5c0d5d9f001074acbbb2e6bbe814696bb866e45",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_direct_calls_n1024": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 28,
            "sides": [
              {
                "calibrated_total_ns": 70532460.992,
                "ns_per_operation": 2519016.464,
                "observed_batch": 28,
                "requested_batch": 28,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 47673473.988,
                "ns_per_operation": 1702624.071,
                "observed_batch": 28,
                "requested_batch": 28,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 70532460.992,
            "ns_per_operation": 2519016.464,
            "observed_batch": 28,
            "requested_batch": 28,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 47673473.988,
            "ns_per_operation": 1702624.071,
            "observed_batch": 28,
            "requested_batch": 28,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 22361163.008,
            "ns_per_operation": 1397572.688,
            "observed_batch": 16,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 53970218.988,
            "ns_per_operation": 1927507.821,
            "observed_batch": 28,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 28,
        "source_sha256": "0a075601a2a651f9cb95e8ee0404c64bbf076d17bf242b9c88fecbb8cb973826",
        "wasm_sha256": "f6adbecce99c05bf06f213f4f5c0d5d9f001074acbbb2e6bbe814696bb866e45",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_direct_calls_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1752,
            "sides": [
              {
                "calibrated_total_ns": 30824456.735999998,
                "ns_per_operation": 17593.868,
                "observed_batch": 1752,
                "requested_batch": 1752,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 31733212.128,
                "ns_per_operation": 18112.564,
                "observed_batch": 1752,
                "requested_batch": 1752,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 30824456.735999998,
            "ns_per_operation": 17593.868,
            "observed_batch": 1752,
            "requested_batch": 1752,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 31733212.128,
            "ns_per_operation": 18112.564,
            "observed_batch": 1752,
            "requested_batch": 1752,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 30489634.592000004,
            "ns_per_operation": 17809.366,
            "observed_batch": 1712,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 29544795.936,
            "ns_per_operation": 16863.468,
            "observed_batch": 1752,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1752,
        "source_sha256": "0a075601a2a651f9cb95e8ee0404c64bbf076d17bf242b9c88fecbb8cb973826",
        "wasm_sha256": "f6adbecce99c05bf06f213f4f5c0d5d9f001074acbbb2e6bbe814696bb866e45",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_direct_calls_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 955,
            "sides": [
              {
                "calibrated_total_ns": 40983576.699999996,
                "ns_per_operation": 42914.74,
                "observed_batch": 955,
                "requested_batch": 955,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 50493860.160000004,
                "ns_per_operation": 52873.152,
                "observed_batch": 955,
                "requested_batch": 955,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 40983576.699999996,
            "ns_per_operation": 42914.74,
            "observed_batch": 955,
            "requested_batch": 955,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 50493860.160000004,
            "ns_per_operation": 52873.152,
            "observed_batch": 955,
            "requested_batch": 955,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 31008747.455999997,
            "ns_per_operation": 33778.592,
            "observed_batch": 918,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 30972275.41,
            "ns_per_operation": 32431.702,
            "observed_batch": 955,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 955,
        "source_sha256": "0a075601a2a651f9cb95e8ee0404c64bbf076d17bf242b9c88fecbb8cb973826",
        "wasm_sha256": "f6adbecce99c05bf06f213f4f5c0d5d9f001074acbbb2e6bbe814696bb866e45",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_dynamic_api_n0": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 53051,
            "sides": [
              {
                "calibrated_total_ns": 27896762.247999996,
                "ns_per_operation": 525.848,
                "observed_batch": 53051,
                "requested_batch": 53051,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 24997896.455,
                "ns_per_operation": 471.205,
                "observed_batch": 53051,
                "requested_batch": 53051,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 27896762.247999996,
            "ns_per_operation": 525.848,
            "observed_batch": 53051,
            "requested_batch": 53051,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 24997896.455,
            "ns_per_operation": 471.205,
            "observed_batch": 53051,
            "requested_batch": 53051,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 27343228.114,
            "ns_per_operation": 515.414,
            "observed_batch": 53051,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 18349366.98,
            "ns_per_operation": 385.329,
            "observed_batch": 47620,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 53051,
        "source_sha256": "f02cc93a4a43691ce5be8782fa2df454fc2468a7935117ea18104a7dce22cb2d",
        "wasm_sha256": "c493d0aae68d0c5aa67fc0b562e616c45eb3d42b0302951158a763fa202ce589",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_dynamic_api_n1": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 49141,
            "sides": [
              {
                "calibrated_total_ns": 19461654.217,
                "ns_per_operation": 396.037,
                "observed_batch": 49141,
                "requested_batch": 49141,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 39798657.066999994,
                "ns_per_operation": 809.887,
                "observed_batch": 49141,
                "requested_batch": 49141,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 55551,
            "sides": [
              {
                "calibrated_total_ns": 22340223.507,
                "ns_per_operation": 402.157,
                "observed_batch": 55551,
                "requested_batch": 55551,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 37483259.454,
                "ns_per_operation": 674.754,
                "observed_batch": 55551,
                "requested_batch": 55551,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 22340223.507,
            "ns_per_operation": 402.157,
            "observed_batch": 55551,
            "requested_batch": 55551,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 37483259.454,
            "ns_per_operation": 674.754,
            "observed_batch": 55551,
            "requested_batch": 55551,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 23619474.226999998,
            "ns_per_operation": 480.647,
            "observed_batch": 49141,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 16888770.645999998,
            "ns_per_operation": 421.366,
            "observed_batch": 40081,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 55551,
        "source_sha256": "f02cc93a4a43691ce5be8782fa2df454fc2468a7935117ea18104a7dce22cb2d",
        "wasm_sha256": "c493d0aae68d0c5aa67fc0b562e616c45eb3d42b0302951158a763fa202ce589",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_dynamic_api_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 45559,
            "sides": [
              {
                "calibrated_total_ns": 34762564.857,
                "ns_per_operation": 763.023,
                "observed_batch": 45559,
                "requested_batch": 45559,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 55610773.288,
                "ns_per_operation": 1220.632,
                "observed_batch": 45559,
                "requested_batch": 45559,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 34762564.857,
            "ns_per_operation": 763.023,
            "observed_batch": 45559,
            "requested_batch": 45559,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 55610773.288,
            "ns_per_operation": 1220.632,
            "observed_batch": 45559,
            "requested_batch": 45559,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 34969772.496,
            "ns_per_operation": 1036.848,
            "observed_batch": 33727,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22683188.274,
            "ns_per_operation": 497.886,
            "observed_batch": 45559,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 45559,
        "source_sha256": "f02cc93a4a43691ce5be8782fa2df454fc2468a7935117ea18104a7dce22cb2d",
        "wasm_sha256": "c493d0aae68d0c5aa67fc0b562e616c45eb3d42b0302951158a763fa202ce589",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_dynamic_api_n1024": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 47620,
            "sides": [
              {
                "calibrated_total_ns": 25939756.880000003,
                "ns_per_operation": 544.724,
                "observed_batch": 47620,
                "requested_batch": 47620,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 32681653.62,
                "ns_per_operation": 686.301,
                "observed_batch": 47620,
                "requested_batch": 47620,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 25939756.880000003,
            "ns_per_operation": 544.724,
            "observed_batch": 47620,
            "requested_batch": 47620,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 32681653.62,
            "ns_per_operation": 686.301,
            "observed_batch": 47620,
            "requested_batch": 47620,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 25345561.560000002,
            "ns_per_operation": 548.724,
            "observed_batch": 46190,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 39353120.38,
            "ns_per_operation": 826.399,
            "observed_batch": 47620,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 47620,
        "source_sha256": "f02cc93a4a43691ce5be8782fa2df454fc2468a7935117ea18104a7dce22cb2d",
        "wasm_sha256": "c493d0aae68d0c5aa67fc0b562e616c45eb3d42b0302951158a763fa202ce589",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_dynamic_api_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 52771,
            "sides": [
              {
                "calibrated_total_ns": 24055924.205000002,
                "ns_per_operation": 455.855,
                "observed_batch": 52771,
                "requested_batch": 52771,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 22221815.329,
                "ns_per_operation": 421.099,
                "observed_batch": 52771,
                "requested_batch": 52771,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 24055924.205000002,
            "ns_per_operation": 455.855,
            "observed_batch": 52771,
            "requested_batch": 52771,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22221815.329,
            "ns_per_operation": 421.099,
            "observed_batch": 52771,
            "requested_batch": 52771,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 19089469.743,
            "ns_per_operation": 398.969,
            "observed_batch": 47847,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 32036535.306,
            "ns_per_operation": 607.086,
            "observed_batch": 52771,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 52771,
        "source_sha256": "f02cc93a4a43691ce5be8782fa2df454fc2468a7935117ea18104a7dce22cb2d",
        "wasm_sha256": "c493d0aae68d0c5aa67fc0b562e616c45eb3d42b0302951158a763fa202ce589",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_dynamic_api_n32": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 43479,
            "sides": [
              {
                "calibrated_total_ns": 15334738.946999999,
                "ns_per_operation": 352.693,
                "observed_batch": 43479,
                "requested_batch": 43479,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 22997390.949,
                "ns_per_operation": 528.931,
                "observed_batch": 43479,
                "requested_batch": 43479,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 62378,
            "sides": [
              {
                "calibrated_total_ns": 23505090.826,
                "ns_per_operation": 376.817,
                "observed_batch": 62378,
                "requested_batch": 62378,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 27779044.252,
                "ns_per_operation": 445.334,
                "observed_batch": 62378,
                "requested_batch": 62378,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 23505090.826,
            "ns_per_operation": 376.817,
            "observed_batch": 62378,
            "requested_batch": 62378,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27779044.252,
            "ns_per_operation": 445.334,
            "observed_batch": 62378,
            "requested_batch": 62378,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 18890495.046,
            "ns_per_operation": 434.474,
            "observed_batch": 43479,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 23266346.981000002,
            "ns_per_operation": 542.099,
            "observed_batch": 42919,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 62378,
        "source_sha256": "f02cc93a4a43691ce5be8782fa2df454fc2468a7935117ea18104a7dce22cb2d",
        "wasm_sha256": "c493d0aae68d0c5aa67fc0b562e616c45eb3d42b0302951158a763fa202ce589",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_host_call_n0": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 33334,
            "sides": [
              {
                "calibrated_total_ns": 22787322.404,
                "ns_per_operation": 683.606,
                "observed_batch": 33334,
                "requested_batch": 33334,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 26643466.192,
                "ns_per_operation": 799.288,
                "observed_batch": 33334,
                "requested_batch": 33334,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 22787322.404,
            "ns_per_operation": 683.606,
            "observed_batch": 33334,
            "requested_batch": 33334,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 26643466.192,
            "ns_per_operation": 799.288,
            "observed_batch": 33334,
            "requested_batch": 33334,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 18940539.331,
            "ns_per_operation": 650.587,
            "observed_batch": 29113,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22409681.518000003,
            "ns_per_operation": 672.277,
            "observed_batch": 33334,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 33334,
        "source_sha256": "4c4026de620eff0b1e9cb4ed4e0440e6045d921267e3962715f6430ac68f5120",
        "wasm_sha256": "c60311d57c929c3ff1d29adb70501e7bbf8419517acbd86c8d072438b610ebb7",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_host_call_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 15988,
            "sides": [
              {
                "calibrated_total_ns": 24384545.864,
                "ns_per_operation": 1525.178,
                "observed_batch": 15988,
                "requested_batch": 15988,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 28307665.316,
                "ns_per_operation": 1770.557,
                "observed_batch": 15988,
                "requested_batch": 15988,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 24384545.864,
            "ns_per_operation": 1525.178,
            "observed_batch": 15988,
            "requested_batch": 15988,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 28307665.316,
            "ns_per_operation": 1770.557,
            "observed_batch": 15988,
            "requested_batch": 15988,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 22812763.264000002,
            "ns_per_operation": 1587.748,
            "observed_batch": 14368,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27091602.048,
            "ns_per_operation": 1694.496,
            "observed_batch": 15988,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 15988,
        "source_sha256": "4c4026de620eff0b1e9cb4ed4e0440e6045d921267e3962715f6430ac68f5120",
        "wasm_sha256": "c60311d57c929c3ff1d29adb70501e7bbf8419517acbd86c8d072438b610ebb7",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_host_call_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 3,
            "sides": [
              {
                "calibrated_total_ns": 21617979.0,
                "ns_per_operation": 7205993.0,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 21439095.0,
                "ns_per_operation": 7146365.0,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 21617979.0,
            "ns_per_operation": 7205993.0,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 21439095.0,
            "ns_per_operation": 7146365.0,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 23406714.0,
            "ns_per_operation": 7802238.0,
            "observed_batch": 3,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 24986058.998999998,
            "ns_per_operation": 8328686.333,
            "observed_batch": 3,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 3,
        "source_sha256": "4c4026de620eff0b1e9cb4ed4e0440e6045d921267e3962715f6430ac68f5120",
        "wasm_sha256": "c60311d57c929c3ff1d29adb70501e7bbf8419517acbd86c8d072438b610ebb7",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_host_call_n1024": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 33,
            "sides": [
              {
                "calibrated_total_ns": 30745651.002,
                "ns_per_operation": 931686.394,
                "observed_batch": 33,
                "requested_batch": 33,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 31632578.009999998,
                "ns_per_operation": 958562.97,
                "observed_batch": 33,
                "requested_batch": 33,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 30745651.002,
            "ns_per_operation": 931686.394,
            "observed_batch": 33,
            "requested_batch": 33,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 31632578.009999998,
            "ns_per_operation": 958562.97,
            "observed_batch": 33,
            "requested_batch": 33,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 28923496.998,
            "ns_per_operation": 876469.606,
            "observed_batch": 33,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 23754759.009,
            "ns_per_operation": 819129.621,
            "observed_batch": 29,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 33,
        "source_sha256": "4c4026de620eff0b1e9cb4ed4e0440e6045d921267e3962715f6430ac68f5120",
        "wasm_sha256": "c60311d57c929c3ff1d29adb70501e7bbf8419517acbd86c8d072438b610ebb7",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_host_call_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 2051,
            "sides": [
              {
                "calibrated_total_ns": 34676392.366,
                "ns_per_operation": 16907.066,
                "observed_batch": 2051,
                "requested_batch": 2051,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 31043624.248,
                "ns_per_operation": 15135.848,
                "observed_batch": 2051,
                "requested_batch": 2051,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 34676392.366,
            "ns_per_operation": 16907.066,
            "observed_batch": 2051,
            "requested_batch": 2051,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 31043624.248,
            "ns_per_operation": 15135.848,
            "observed_batch": 2051,
            "requested_batch": 2051,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 25631021.735000003,
            "ns_per_operation": 13468.745,
            "observed_batch": 1903,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 29694812.812,
            "ns_per_operation": 14478.212,
            "observed_batch": 2051,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 2051,
        "source_sha256": "4c4026de620eff0b1e9cb4ed4e0440e6045d921267e3962715f6430ac68f5120",
        "wasm_sha256": "c60311d57c929c3ff1d29adb70501e7bbf8419517acbd86c8d072438b610ebb7",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_host_call_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1198,
            "sides": [
              {
                "calibrated_total_ns": 30353795.484,
                "ns_per_operation": 25337.058,
                "observed_batch": 1198,
                "requested_batch": 1198,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 32144806.682,
                "ns_per_operation": 26832.059,
                "observed_batch": 1198,
                "requested_batch": 1198,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 30353795.484,
            "ns_per_operation": 25337.058,
            "observed_batch": 1198,
            "requested_batch": 1198,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 32144806.682,
            "ns_per_operation": 26832.059,
            "observed_batch": 1198,
            "requested_batch": 1198,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 1164470.0019999999,
            "ns_per_operation": 20077.069,
            "observed_batch": 58,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 30005554.854,
            "ns_per_operation": 25046.373,
            "observed_batch": 1198,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1198,
        "source_sha256": "4c4026de620eff0b1e9cb4ed4e0440e6045d921267e3962715f6430ac68f5120",
        "wasm_sha256": "c60311d57c929c3ff1d29adb70501e7bbf8419517acbd86c8d072438b610ebb7",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_indirect_calls_n0": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 27701,
            "sides": [
              {
                "calibrated_total_ns": 19811644.396,
                "ns_per_operation": 715.196,
                "observed_batch": 27701,
                "requested_batch": 27701,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 23377649.528,
                "ns_per_operation": 843.928,
                "observed_batch": 27701,
                "requested_batch": 27701,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 30761,
            "sides": [
              {
                "calibrated_total_ns": 32675503.118,
                "ns_per_operation": 1062.238,
                "observed_batch": 30761,
                "requested_batch": 30761,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 23772746.781,
                "ns_per_operation": 772.821,
                "observed_batch": 30761,
                "requested_batch": 30761,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 32675503.118,
            "ns_per_operation": 1062.238,
            "observed_batch": 30761,
            "requested_batch": 30761,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 23772746.781,
            "ns_per_operation": 772.821,
            "observed_batch": 30761,
            "requested_batch": 30761,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 21529965.127,
            "ns_per_operation": 777.227,
            "observed_batch": 27701,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22275888.46,
            "ns_per_operation": 877.66,
            "observed_batch": 25381,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 30761,
        "source_sha256": "29024ae23844664a9fea4a809d1ce414de40f243f7e678ce5ea6d811843f7797",
        "wasm_sha256": "4b42e82f54ea6ab7d0f62c70766c904e41d6a408e57fa14b5abbf514c0da78a9",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_indirect_calls_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 14525,
            "sides": [
              {
                "calibrated_total_ns": 27097564.025,
                "ns_per_operation": 1865.581,
                "observed_batch": 14525,
                "requested_batch": 14525,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 26248447.05,
                "ns_per_operation": 1807.122,
                "observed_batch": 14525,
                "requested_batch": 14525,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 27097564.025,
            "ns_per_operation": 1865.581,
            "observed_batch": 14525,
            "requested_batch": 14525,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 26248447.05,
            "ns_per_operation": 1807.122,
            "observed_batch": 14525,
            "requested_batch": 14525,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 24571127.964,
            "ns_per_operation": 1964.433,
            "observed_batch": 12508,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 25157445.25,
            "ns_per_operation": 1732.01,
            "observed_batch": 14525,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 14525,
        "source_sha256": "29024ae23844664a9fea4a809d1ce414de40f243f7e678ce5ea6d811843f7797",
        "wasm_sha256": "4b42e82f54ea6ab7d0f62c70766c904e41d6a408e57fa14b5abbf514c0da78a9",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_indirect_calls_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 3,
            "sides": [
              {
                "calibrated_total_ns": 28133811.999,
                "ns_per_operation": 9377937.333,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 26812715.001,
                "ns_per_operation": 8937571.667,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 28133811.999,
            "ns_per_operation": 9377937.333,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 26812715.001,
            "ns_per_operation": 8937571.667,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 37629521.001,
            "ns_per_operation": 12543173.667,
            "observed_batch": 3,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 19826817.0,
            "ns_per_operation": 9913408.5,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 3,
        "source_sha256": "29024ae23844664a9fea4a809d1ce414de40f243f7e678ce5ea6d811843f7797",
        "wasm_sha256": "4b42e82f54ea6ab7d0f62c70766c904e41d6a408e57fa14b5abbf514c0da78a9",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_indirect_calls_n1024": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 22,
            "sides": [
              {
                "calibrated_total_ns": 23427443.006,
                "ns_per_operation": 1064883.773,
                "observed_batch": 22,
                "requested_batch": 22,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 23520532.002,
                "ns_per_operation": 1069115.091,
                "observed_batch": 22,
                "requested_batch": 22,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 23427443.006,
            "ns_per_operation": 1064883.773,
            "observed_batch": 22,
            "requested_batch": 22,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 23520532.002,
            "ns_per_operation": 1069115.091,
            "observed_batch": 22,
            "requested_batch": 22,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 21718426.004,
            "ns_per_operation": 987201.182,
            "observed_batch": 22,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 19923204.005999997,
            "ns_per_operation": 1106844.667,
            "observed_batch": 18,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 22,
        "source_sha256": "29024ae23844664a9fea4a809d1ce414de40f243f7e678ce5ea6d811843f7797",
        "wasm_sha256": "4b42e82f54ea6ab7d0f62c70766c904e41d6a408e57fa14b5abbf514c0da78a9",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_indirect_calls_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1615,
            "sides": [
              {
                "calibrated_total_ns": 24885496.240000002,
                "ns_per_operation": 15408.976,
                "observed_batch": 1615,
                "requested_batch": 1615,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 26476432.740000002,
                "ns_per_operation": 16394.076,
                "observed_batch": 1615,
                "requested_batch": 1615,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 24885496.240000002,
            "ns_per_operation": 15408.976,
            "observed_batch": 1615,
            "requested_batch": 1615,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 26476432.740000002,
            "ns_per_operation": 16394.076,
            "observed_batch": 1615,
            "requested_batch": 1615,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 38912576.697000004,
            "ns_per_operation": 24992.021,
            "observed_batch": 1557,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 25725448.05,
            "ns_per_operation": 15929.07,
            "observed_batch": 1615,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1615,
        "source_sha256": "29024ae23844664a9fea4a809d1ce414de40f243f7e678ce5ea6d811843f7797",
        "wasm_sha256": "4b42e82f54ea6ab7d0f62c70766c904e41d6a408e57fa14b5abbf514c0da78a9",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_indirect_calls_n32": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 205,
            "sides": [
              {
                "calibrated_total_ns": 7015128.085000001,
                "ns_per_operation": 34220.137,
                "observed_batch": 205,
                "requested_batch": 205,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 7936187.96,
                "ns_per_operation": 38713.112,
                "observed_batch": 205,
                "requested_batch": 205,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 643,
            "sides": [
              {
                "calibrated_total_ns": 20551369.242,
                "ns_per_operation": 31961.694,
                "observed_batch": 643,
                "requested_batch": 643,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 21559917.957,
                "ns_per_operation": 33530.199,
                "observed_batch": 643,
                "requested_batch": 643,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 20551369.242,
            "ns_per_operation": 31961.694,
            "observed_batch": 643,
            "requested_batch": 643,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 21559917.957,
            "ns_per_operation": 33530.199,
            "observed_batch": 643,
            "requested_batch": 643,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 7810891.959999999,
            "ns_per_operation": 38101.912,
            "observed_batch": 205,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 5415046.02,
            "ns_per_operation": 40111.452,
            "observed_batch": 135,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 643,
        "source_sha256": "29024ae23844664a9fea4a809d1ce414de40f243f7e678ce5ea6d811843f7797",
        "wasm_sha256": "4b42e82f54ea6ab7d0f62c70766c904e41d6a408e57fa14b5abbf514c0da78a9",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_multivalue_branch_n0": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 30817,
            "sides": [
              {
                "calibrated_total_ns": 63396670.765999995,
                "ns_per_operation": 2057.198,
                "observed_batch": 30817,
                "requested_batch": 30817,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 62106948.499,
                "ns_per_operation": 2015.347,
                "observed_batch": 30817,
                "requested_batch": 30817,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 63396670.765999995,
            "ns_per_operation": 2057.198,
            "observed_batch": 30817,
            "requested_batch": 30817,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 62106948.499,
            "ns_per_operation": 2015.347,
            "observed_batch": 30817,
            "requested_batch": 30817,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 37495303.78,
            "ns_per_operation": 1970.326,
            "observed_batch": 19030,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 64267620.82,
            "ns_per_operation": 2085.46,
            "observed_batch": 30817,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 30817,
        "source_sha256": "a1f4958100af82889208bb28090d6af7c3e56cbeeccabad4106601c9a9c17014",
        "wasm_sha256": "03ffdc2041ae8ba0c8d078422d36152cf1d35f1293c94a4074fe7e83791743ae",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_multivalue_branch_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 11690,
            "sides": [
              {
                "calibrated_total_ns": 30311737.470000003,
                "ns_per_operation": 2592.963,
                "observed_batch": 11690,
                "requested_batch": 11690,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 27775615.349999998,
                "ns_per_operation": 2376.015,
                "observed_batch": 11690,
                "requested_batch": 11690,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 30311737.470000003,
            "ns_per_operation": 2592.963,
            "observed_batch": 11690,
            "requested_batch": 11690,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27775615.349999998,
            "ns_per_operation": 2376.015,
            "observed_batch": 11690,
            "requested_batch": 11690,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 27598215.659999996,
            "ns_per_operation": 2376.084,
            "observed_batch": 11615,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27943927.97,
            "ns_per_operation": 2390.413,
            "observed_batch": 11690,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 11690,
        "source_sha256": "a1f4958100af82889208bb28090d6af7c3e56cbeeccabad4106601c9a9c17014",
        "wasm_sha256": "03ffdc2041ae8ba0c8d078422d36152cf1d35f1293c94a4074fe7e83791743ae",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_multivalue_branch_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 2,
            "sides": [
              {
                "calibrated_total_ns": 26168629.0,
                "ns_per_operation": 13084314.5,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 30610764.0,
                "ns_per_operation": 15305382.0,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 26168629.0,
            "ns_per_operation": 13084314.5,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 30610764.0,
            "ns_per_operation": 15305382.0,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 29001042.0,
            "ns_per_operation": 14500521.0,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 28286269.0,
            "ns_per_operation": 14143134.5,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 2,
        "source_sha256": "a1f4958100af82889208bb28090d6af7c3e56cbeeccabad4106601c9a9c17014",
        "wasm_sha256": "03ffdc2041ae8ba0c8d078422d36152cf1d35f1293c94a4074fe7e83791743ae",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_multivalue_branch_n1024": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 17,
            "sides": [
              {
                "calibrated_total_ns": 29466678.0,
                "ns_per_operation": 1733334.0,
                "observed_batch": 17,
                "requested_batch": 17,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 39082215.008,
                "ns_per_operation": 2298953.824,
                "observed_batch": 17,
                "requested_batch": 17,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 29466678.0,
            "ns_per_operation": 1733334.0,
            "observed_batch": 17,
            "requested_batch": 17,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 39082215.008,
            "ns_per_operation": 2298953.824,
            "observed_batch": 17,
            "requested_batch": 17,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 24284384.0,
            "ns_per_operation": 1517774.0,
            "observed_batch": 16,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 26080608.004,
            "ns_per_operation": 1534153.412,
            "observed_batch": 17,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 17,
        "source_sha256": "a1f4958100af82889208bb28090d6af7c3e56cbeeccabad4106601c9a9c17014",
        "wasm_sha256": "03ffdc2041ae8ba0c8d078422d36152cf1d35f1293c94a4074fe7e83791743ae",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_multivalue_branch_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1227,
            "sides": [
              {
                "calibrated_total_ns": 40421605.788,
                "ns_per_operation": 32943.444,
                "observed_batch": 1227,
                "requested_batch": 1227,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 36499654.89,
                "ns_per_operation": 29747.07,
                "observed_batch": 1227,
                "requested_batch": 1227,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 40421605.788,
            "ns_per_operation": 32943.444,
            "observed_batch": 1227,
            "requested_batch": 1227,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 36499654.89,
            "ns_per_operation": 29747.07,
            "observed_batch": 1227,
            "requested_batch": 1227,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 32844991.744000003,
            "ns_per_operation": 27010.684,
            "observed_batch": 1216,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 40799990.502000004,
            "ns_per_operation": 33251.826,
            "observed_batch": 1227,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1227,
        "source_sha256": "a1f4958100af82889208bb28090d6af7c3e56cbeeccabad4106601c9a9c17014",
        "wasm_sha256": "03ffdc2041ae8ba0c8d078422d36152cf1d35f1293c94a4074fe7e83791743ae",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_multivalue_branch_n32": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 67,
            "sides": [
              {
                "calibrated_total_ns": 3783759.005,
                "ns_per_operation": 56474.015,
                "observed_batch": 67,
                "requested_batch": 67,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 4160117.971,
                "ns_per_operation": 62091.313,
                "observed_batch": 67,
                "requested_batch": 67,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 390,
            "sides": [
              {
                "calibrated_total_ns": 23279568.0,
                "ns_per_operation": 59691.2,
                "observed_batch": 390,
                "requested_batch": 390,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 21067103.849999998,
                "ns_per_operation": 54018.215,
                "observed_batch": 390,
                "requested_batch": 390,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 23279568.0,
            "ns_per_operation": 59691.2,
            "observed_batch": 390,
            "requested_batch": 390,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 21067103.849999998,
            "ns_per_operation": 54018.215,
            "observed_batch": 390,
            "requested_batch": 390,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 3315313.0080000004,
            "ns_per_operation": 184184.056,
            "observed_batch": 18,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 3816474.971,
            "ns_per_operation": 56962.313,
            "observed_batch": 67,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 390,
        "source_sha256": "a1f4958100af82889208bb28090d6af7c3e56cbeeccabad4106601c9a9c17014",
        "wasm_sha256": "03ffdc2041ae8ba0c8d078422d36152cf1d35f1293c94a4074fe7e83791743ae",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_ordinary_recursion_n0": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 20943,
            "sides": [
              {
                "calibrated_total_ns": 20431969.857,
                "ns_per_operation": 975.599,
                "observed_batch": 20943,
                "requested_batch": 20943,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 18986546.826,
                "ns_per_operation": 906.582,
                "observed_batch": 20943,
                "requested_batch": 20943,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 24267,
            "sides": [
              {
                "calibrated_total_ns": 24657456.03,
                "ns_per_operation": 1016.09,
                "observed_batch": 24267,
                "requested_batch": 24267,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 24055925.634,
                "ns_per_operation": 991.302,
                "observed_batch": 24267,
                "requested_batch": 24267,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 24657456.03,
            "ns_per_operation": 1016.09,
            "observed_batch": 24267,
            "requested_batch": 24267,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 24055925.634,
            "ns_per_operation": 991.302,
            "observed_batch": 24267,
            "requested_batch": 24267,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 7996287.824,
            "ns_per_operation": 889.168,
            "observed_batch": 8993,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 26215484.022,
            "ns_per_operation": 1251.754,
            "observed_batch": 20943,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 24267,
        "source_sha256": "aca6a66637fa36b72f3c528dd5e3e44039b573f363b5b4fb77437e9278623903",
        "wasm_sha256": "e1bd7ea6ada8b15b2770ac387f8fc2256f20d7f87c4bc34e7559b239d8875ba1",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_ordinary_recursion_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 16434,
            "sides": [
              {
                "calibrated_total_ns": 28413925.848,
                "ns_per_operation": 1728.972,
                "observed_batch": 16434,
                "requested_batch": 16434,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 33543141.588000003,
                "ns_per_operation": 2041.082,
                "observed_batch": 16434,
                "requested_batch": 16434,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 28413925.848,
            "ns_per_operation": 1728.972,
            "observed_batch": 16434,
            "requested_batch": 16434,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 33543141.588000003,
            "ns_per_operation": 2041.082,
            "observed_batch": 16434,
            "requested_batch": 16434,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 28119412.134,
            "ns_per_operation": 1711.051,
            "observed_batch": 16434,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27944504.268000003,
            "ns_per_operation": 1806.601,
            "observed_batch": 15468,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 16434,
        "source_sha256": "aca6a66637fa36b72f3c528dd5e3e44039b573f363b5b4fb77437e9278623903",
        "wasm_sha256": "e1bd7ea6ada8b15b2770ac387f8fc2256f20d7f87c4bc34e7559b239d8875ba1",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_ordinary_recursion_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1942,
            "sides": [
              {
                "calibrated_total_ns": 37250616.708000004,
                "ns_per_operation": 19181.574,
                "observed_batch": 1942,
                "requested_batch": 1942,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 28419610.574,
                "ns_per_operation": 14634.197,
                "observed_batch": 1942,
                "requested_batch": 1942,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 37250616.708000004,
            "ns_per_operation": 19181.574,
            "observed_batch": 1942,
            "requested_batch": 1942,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 28419610.574,
            "ns_per_operation": 14634.197,
            "observed_batch": 1942,
            "requested_batch": 1942,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 32737336.844,
            "ns_per_operation": 17899.036,
            "observed_batch": 1829,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 34053309.85,
            "ns_per_operation": 17535.175,
            "observed_batch": 1942,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1942,
        "source_sha256": "aca6a66637fa36b72f3c528dd5e3e44039b573f363b5b4fb77437e9278623903",
        "wasm_sha256": "e1bd7ea6ada8b15b2770ac387f8fc2256f20d7f87c4bc34e7559b239d8875ba1",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_ordinary_recursion_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1070,
            "sides": [
              {
                "calibrated_total_ns": 42912959.9,
                "ns_per_operation": 40105.57,
                "observed_batch": 1070,
                "requested_batch": 1070,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 31779908.43,
                "ns_per_operation": 29700.849,
                "observed_batch": 1070,
                "requested_batch": 1070,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 42912959.9,
            "ns_per_operation": 40105.57,
            "observed_batch": 1070,
            "requested_batch": 1070,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 31779908.43,
            "ns_per_operation": 29700.849,
            "observed_batch": 1070,
            "requested_batch": 1070,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 33881875.203,
            "ns_per_operation": 40772.413,
            "observed_batch": 831,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 33375242.049999997,
            "ns_per_operation": 31191.815,
            "observed_batch": 1070,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1070,
        "source_sha256": "aca6a66637fa36b72f3c528dd5e3e44039b573f363b5b4fb77437e9278623903",
        "wasm_sha256": "e1bd7ea6ada8b15b2770ac387f8fc2256f20d7f87c4bc34e7559b239d8875ba1",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_table_mutation_n0": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 25807,
            "sides": [
              {
                "calibrated_total_ns": 43989863.797,
                "ns_per_operation": 1704.571,
                "observed_batch": 25807,
                "requested_batch": 25807,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 23408136.122,
                "ns_per_operation": 907.046,
                "observed_batch": 25807,
                "requested_batch": 25807,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 43989863.797,
            "ns_per_operation": 1704.571,
            "observed_batch": 25807,
            "requested_batch": 25807,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 23408136.122,
            "ns_per_operation": 907.046,
            "observed_batch": 25807,
            "requested_batch": 25807,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 16249643.8,
            "ns_per_operation": 735.278,
            "observed_batch": 22100,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 34820094.75,
            "ns_per_operation": 1349.25,
            "observed_batch": 25807,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 25807,
        "source_sha256": "9b61bd9cbca804bf639085769fd766a2cf8a69d516fc7efb9e4edfe4ff6d18c9",
        "wasm_sha256": "458f2b111e48d2efca98083b8c56860c43e82be8752813d1e2814d23c28d4f43",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_table_mutation_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 13727,
            "sides": [
              {
                "calibrated_total_ns": 32725785.715,
                "ns_per_operation": 2384.045,
                "observed_batch": 13727,
                "requested_batch": 13727,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 40278820.379,
                "ns_per_operation": 2934.277,
                "observed_batch": 13727,
                "requested_batch": 13727,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 32725785.715,
            "ns_per_operation": 2384.045,
            "observed_batch": 13727,
            "requested_batch": 13727,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 40278820.379,
            "ns_per_operation": 2934.277,
            "observed_batch": 13727,
            "requested_batch": 13727,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 17995124.07,
            "ns_per_operation": 2749.446,
            "observed_batch": 6545,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 34987116.787,
            "ns_per_operation": 2548.781,
            "observed_batch": 13727,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 13727,
        "source_sha256": "9b61bd9cbca804bf639085769fd766a2cf8a69d516fc7efb9e4edfe4ff6d18c9",
        "wasm_sha256": "458f2b111e48d2efca98083b8c56860c43e82be8752813d1e2814d23c28d4f43",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_table_mutation_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 2,
            "sides": [
              {
                "calibrated_total_ns": 417229573.0,
                "ns_per_operation": 208614786.5,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 36158076.0,
                "ns_per_operation": 18079038.0,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 417229573.0,
            "ns_per_operation": 208614786.5,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 36158076.0,
            "ns_per_operation": 18079038.0,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 10712594.0,
            "ns_per_operation": 10712594.0,
            "observed_batch": 1,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 48828018.0,
            "ns_per_operation": 24414009.0,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 2,
        "source_sha256": "9b61bd9cbca804bf639085769fd766a2cf8a69d516fc7efb9e4edfe4ff6d18c9",
        "wasm_sha256": "458f2b111e48d2efca98083b8c56860c43e82be8752813d1e2814d23c28d4f43",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_table_mutation_n1024": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 22,
            "sides": [
              {
                "calibrated_total_ns": 33437395.992,
                "ns_per_operation": 1519881.636,
                "observed_batch": 22,
                "requested_batch": 22,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 20064035.002,
                "ns_per_operation": 912001.591,
                "observed_batch": 22,
                "requested_batch": 22,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 33437395.992,
            "ns_per_operation": 1519881.636,
            "observed_batch": 22,
            "requested_batch": 22,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 20064035.002,
            "ns_per_operation": 912001.591,
            "observed_batch": 22,
            "requested_batch": 22,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 20585049.998,
            "ns_per_operation": 1210885.294,
            "observed_batch": 17,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 20447204.998,
            "ns_per_operation": 929418.409,
            "observed_batch": 22,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 22,
        "source_sha256": "9b61bd9cbca804bf639085769fd766a2cf8a69d516fc7efb9e4edfe4ff6d18c9",
        "wasm_sha256": "458f2b111e48d2efca98083b8c56860c43e82be8752813d1e2814d23c28d4f43",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_table_mutation_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1771,
            "sides": [
              {
                "calibrated_total_ns": 31327376.619,
                "ns_per_operation": 17689.089,
                "observed_batch": 1771,
                "requested_batch": 1771,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 35532531.111,
                "ns_per_operation": 20063.541,
                "observed_batch": 1771,
                "requested_batch": 1771,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 31327376.619,
            "ns_per_operation": 17689.089,
            "observed_batch": 1771,
            "requested_batch": 1771,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 35532531.111,
            "ns_per_operation": 20063.541,
            "observed_batch": 1771,
            "requested_batch": 1771,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 34990531.39,
            "ns_per_operation": 20765.894,
            "observed_batch": 1685,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 30431055.809,
            "ns_per_operation": 17182.979,
            "observed_batch": 1771,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1771,
        "source_sha256": "9b61bd9cbca804bf639085769fd766a2cf8a69d516fc7efb9e4edfe4ff6d18c9",
        "wasm_sha256": "458f2b111e48d2efca98083b8c56860c43e82be8752813d1e2814d23c28d4f43",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_table_mutation_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 964,
            "sides": [
              {
                "calibrated_total_ns": 53011505.232,
                "ns_per_operation": 54991.188,
                "observed_batch": 964,
                "requested_batch": 964,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 34597371.96,
                "ns_per_operation": 35889.39,
                "observed_batch": 964,
                "requested_batch": 964,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 53011505.232,
            "ns_per_operation": 54991.188,
            "observed_batch": 964,
            "requested_batch": 964,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 34597371.96,
            "ns_per_operation": 35889.39,
            "observed_batch": 964,
            "requested_batch": 964,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 31701925.246999998,
            "ns_per_operation": 33265.399,
            "observed_batch": 953,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 55639506.120000005,
            "ns_per_operation": 57717.33,
            "observed_batch": 964,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 964,
        "source_sha256": "9b61bd9cbca804bf639085769fd766a2cf8a69d516fc7efb9e4edfe4ff6d18c9",
        "wasm_sha256": "458f2b111e48d2efca98083b8c56860c43e82be8752813d1e2814d23c28d4f43",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_tail_calls_n0": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 23613,
            "sides": [
              {
                "calibrated_total_ns": 18883717.520999998,
                "ns_per_operation": 799.717,
                "observed_batch": 23613,
                "requested_batch": 23613,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 20581917.255,
                "ns_per_operation": 871.635,
                "observed_batch": 23613,
                "requested_batch": 23613,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 27510,
            "sides": [
              {
                "calibrated_total_ns": 22310857.59,
                "ns_per_operation": 811.009,
                "observed_batch": 27510,
                "requested_batch": 27510,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 24943372.02,
                "ns_per_operation": 906.702,
                "observed_batch": 27510,
                "requested_batch": 27510,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 22310857.59,
            "ns_per_operation": 811.009,
            "observed_batch": 27510,
            "requested_batch": 27510,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 24943372.02,
            "ns_per_operation": 906.702,
            "observed_batch": 27510,
            "requested_batch": 27510,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 8988313.230999999,
            "ns_per_operation": 899.641,
            "observed_batch": 9991,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 20458137.909,
            "ns_per_operation": 866.393,
            "observed_batch": 23613,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 27510,
        "source_sha256": "9953f19384c51b579921e6f789b13a8b57c86ba86d760b77846e3e33825ece02",
        "wasm_sha256": "3e23be294e7a9eda62c89dbc366b6a6cab90edfbda532da27a7e4fc6d0aa4a56",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_tail_calls_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 14546,
            "sides": [
              {
                "calibrated_total_ns": 43500918.496,
                "ns_per_operation": 2990.576,
                "observed_batch": 14546,
                "requested_batch": 14546,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 23867105.891999997,
                "ns_per_operation": 1640.802,
                "observed_batch": 14546,
                "requested_batch": 14546,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 43500918.496,
            "ns_per_operation": 2990.576,
            "observed_batch": 14546,
            "requested_batch": 14546,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 23867105.891999997,
            "ns_per_operation": 1640.802,
            "observed_batch": 14546,
            "requested_batch": 14546,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 9947269.368,
            "ns_per_operation": 1351.164,
            "observed_batch": 7362,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 109155118.618,
            "ns_per_operation": 7504.133,
            "observed_batch": 14546,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 14546,
        "source_sha256": "9953f19384c51b579921e6f789b13a8b57c86ba86d760b77846e3e33825ece02",
        "wasm_sha256": "3e23be294e7a9eda62c89dbc366b6a6cab90edfbda532da27a7e4fc6d0aa4a56",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_tail_calls_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 4,
            "sides": [
              {
                "calibrated_total_ns": 33529717.0,
                "ns_per_operation": 8382429.25,
                "observed_batch": 4,
                "requested_batch": 4,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 31884968.0,
                "ns_per_operation": 7971242.0,
                "observed_batch": 4,
                "requested_batch": 4,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 33529717.0,
            "ns_per_operation": 8382429.25,
            "observed_batch": 4,
            "requested_batch": 4,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 31884968.0,
            "ns_per_operation": 7971242.0,
            "observed_batch": 4,
            "requested_batch": 4,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 19990571.001000002,
            "ns_per_operation": 6663523.667,
            "observed_batch": 3,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 25520148.0,
            "ns_per_operation": 6380037.0,
            "observed_batch": 4,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 4,
        "source_sha256": "9953f19384c51b579921e6f789b13a8b57c86ba86d760b77846e3e33825ece02",
        "wasm_sha256": "3e23be294e7a9eda62c89dbc366b6a6cab90edfbda532da27a7e4fc6d0aa4a56",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_tail_calls_n1024": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 31,
            "sides": [
              {
                "calibrated_total_ns": 24270962.990000002,
                "ns_per_operation": 782934.29,
                "observed_batch": 31,
                "requested_batch": 31,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 21827224.992,
                "ns_per_operation": 704104.032,
                "observed_batch": 31,
                "requested_batch": 31,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 24270962.990000002,
            "ns_per_operation": 782934.29,
            "observed_batch": 31,
            "requested_batch": 31,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 21827224.992,
            "ns_per_operation": 704104.032,
            "observed_batch": 31,
            "requested_batch": 31,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 10732964.004,
            "ns_per_operation": 894413.667,
            "observed_batch": 12,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 25984746.003,
            "ns_per_operation": 838217.613,
            "observed_batch": 31,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 31,
        "source_sha256": "9953f19384c51b579921e6f789b13a8b57c86ba86d760b77846e3e33825ece02",
        "wasm_sha256": "3e23be294e7a9eda62c89dbc366b6a6cab90edfbda532da27a7e4fc6d0aa4a56",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_tail_calls_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 2365,
            "sides": [
              {
                "calibrated_total_ns": 28304350.745,
                "ns_per_operation": 11968.013,
                "observed_batch": 2365,
                "requested_batch": 2365,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 31222706.349999998,
                "ns_per_operation": 13201.99,
                "observed_batch": 2365,
                "requested_batch": 2365,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 28304350.745,
            "ns_per_operation": 11968.013,
            "observed_batch": 2365,
            "requested_batch": 2365,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 31222706.349999998,
            "ns_per_operation": 13201.99,
            "observed_batch": 2365,
            "requested_batch": 2365,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 26261516.580000002,
            "ns_per_operation": 13196.742,
            "observed_batch": 1990,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27218463.36,
            "ns_per_operation": 11508.864,
            "observed_batch": 2365,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 2365,
        "source_sha256": "9953f19384c51b579921e6f789b13a8b57c86ba86d760b77846e3e33825ece02",
        "wasm_sha256": "3e23be294e7a9eda62c89dbc366b6a6cab90edfbda532da27a7e4fc6d0aa4a56",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_tail_calls_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1314,
            "sides": [
              {
                "calibrated_total_ns": 35441828.208000004,
                "ns_per_operation": 26972.472,
                "observed_batch": 1314,
                "requested_batch": 1314,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 28809748.277999997,
                "ns_per_operation": 21925.227,
                "observed_batch": 1314,
                "requested_batch": 1314,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 35441828.208000004,
            "ns_per_operation": 26972.472,
            "observed_batch": 1314,
            "requested_batch": 1314,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 28809748.277999997,
            "ns_per_operation": 21925.227,
            "observed_batch": 1314,
            "requested_batch": 1314,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 35475650.4,
            "ns_per_operation": 29711.6,
            "observed_batch": 1194,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 37067432.796000004,
            "ns_per_operation": 28209.614,
            "observed_batch": 1314,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1314,
        "source_sha256": "9953f19384c51b579921e6f789b13a8b57c86ba86d760b77846e3e33825ece02",
        "wasm_sha256": "3e23be294e7a9eda62c89dbc366b6a6cab90edfbda532da27a7e4fc6d0aa4a56",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_try_table_normal_n0": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 16626,
            "sides": [
              {
                "calibrated_total_ns": 10228697.598,
                "ns_per_operation": 615.223,
                "observed_batch": 16626,
                "requested_batch": 16626,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 10941005.316,
                "ns_per_operation": 658.066,
                "observed_batch": 16626,
                "requested_batch": 16626,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 35760,
            "sides": [
              {
                "calibrated_total_ns": 21208719.6,
                "ns_per_operation": 593.085,
                "observed_batch": 35760,
                "requested_batch": 35760,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 25836421.2,
                "ns_per_operation": 722.495,
                "observed_batch": 35760,
                "requested_batch": 35760,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 21208719.6,
            "ns_per_operation": 593.085,
            "observed_batch": 35760,
            "requested_batch": 35760,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 25836421.2,
            "ns_per_operation": 722.495,
            "observed_batch": 35760,
            "requested_batch": 35760,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 7700832.279,
            "ns_per_operation": 564.453,
            "observed_batch": 13643,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 12110295.27,
            "ns_per_operation": 728.395,
            "observed_batch": 16626,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 35760,
        "source_sha256": "ab92800dd4e4d4053498c317effadedf52568270ab7b476eef74b09638de8964",
        "wasm_sha256": "97068a6310a3bc535369e2b6277b801875c14ca206a707fcb9bca9036f41d0df",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_try_table_normal_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 17212,
            "sides": [
              {
                "calibrated_total_ns": 29333051.852,
                "ns_per_operation": 1704.221,
                "observed_batch": 17212,
                "requested_batch": 17212,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 27929723.068,
                "ns_per_operation": 1622.689,
                "observed_batch": 17212,
                "requested_batch": 17212,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 29333051.852,
            "ns_per_operation": 1704.221,
            "observed_batch": 17212,
            "requested_batch": 17212,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27929723.068,
            "ns_per_operation": 1622.689,
            "observed_batch": 17212,
            "requested_batch": 17212,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 26317727.738,
            "ns_per_operation": 1935.554,
            "observed_batch": 13597,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 32440454.696000002,
            "ns_per_operation": 1884.758,
            "observed_batch": 17212,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 17212,
        "source_sha256": "ab92800dd4e4d4053498c317effadedf52568270ab7b476eef74b09638de8964",
        "wasm_sha256": "97068a6310a3bc535369e2b6277b801875c14ca206a707fcb9bca9036f41d0df",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_try_table_normal_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 3,
            "sides": [
              {
                "calibrated_total_ns": 24576735.0,
                "ns_per_operation": 8192245.0,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 24527897.001000002,
                "ns_per_operation": 8175965.667,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 24576735.0,
            "ns_per_operation": 8192245.0,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 24527897.001000002,
            "ns_per_operation": 8175965.667,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 22596560.001000002,
            "ns_per_operation": 7532186.667,
            "observed_batch": 3,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 29491253.001,
            "ns_per_operation": 9830417.667,
            "observed_batch": 3,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 3,
        "source_sha256": "ab92800dd4e4d4053498c317effadedf52568270ab7b476eef74b09638de8964",
        "wasm_sha256": "97068a6310a3bc535369e2b6277b801875c14ca206a707fcb9bca9036f41d0df",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_try_table_normal_n1024": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 32,
            "sides": [
              {
                "calibrated_total_ns": 35866600.992,
                "ns_per_operation": 1120831.281,
                "observed_batch": 32,
                "requested_batch": 32,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 26349278.016,
                "ns_per_operation": 823414.938,
                "observed_batch": 32,
                "requested_batch": 32,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 35866600.992,
            "ns_per_operation": 1120831.281,
            "observed_batch": 32,
            "requested_batch": 32,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 26349278.016,
            "ns_per_operation": 823414.938,
            "observed_batch": 32,
            "requested_batch": 32,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 32035575.008,
            "ns_per_operation": 1001111.719,
            "observed_batch": 32,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 21504211.0,
            "ns_per_operation": 860168.44,
            "observed_batch": 25,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 32,
        "source_sha256": "ab92800dd4e4d4053498c317effadedf52568270ab7b476eef74b09638de8964",
        "wasm_sha256": "97068a6310a3bc535369e2b6277b801875c14ca206a707fcb9bca9036f41d0df",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_try_table_normal_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 2168,
            "sides": [
              {
                "calibrated_total_ns": 132831226.68800001,
                "ns_per_operation": 61269.016,
                "observed_batch": 2168,
                "requested_batch": 2168,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 180537101.928,
                "ns_per_operation": 83273.571,
                "observed_batch": 2168,
                "requested_batch": 2168,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 132831226.68800001,
            "ns_per_operation": 61269.016,
            "observed_batch": 2168,
            "requested_batch": 2168,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 180537101.928,
            "ns_per_operation": 83273.571,
            "observed_batch": 2168,
            "requested_batch": 2168,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 34897486.232,
            "ns_per_operation": 22838.669,
            "observed_batch": 1528,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 98060602.03999999,
            "ns_per_operation": 45230.905,
            "observed_batch": 2168,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 2168,
        "source_sha256": "ab92800dd4e4d4053498c317effadedf52568270ab7b476eef74b09638de8964",
        "wasm_sha256": "97068a6310a3bc535369e2b6277b801875c14ca206a707fcb9bca9036f41d0df",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_try_table_normal_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1146,
            "sides": [
              {
                "calibrated_total_ns": 33843000.444,
                "ns_per_operation": 29531.414,
                "observed_batch": 1146,
                "requested_batch": 1146,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 27570502.38,
                "ns_per_operation": 24058.03,
                "observed_batch": 1146,
                "requested_batch": 1146,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 33843000.444,
            "ns_per_operation": 29531.414,
            "observed_batch": 1146,
            "requested_batch": 1146,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27570502.38,
            "ns_per_operation": 24058.03,
            "observed_batch": 1146,
            "requested_batch": 1146,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 33048594.39,
            "ns_per_operation": 28838.215,
            "observed_batch": 1146,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 40334814.248,
            "ns_per_operation": 40783.432,
            "observed_batch": 989,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1146,
        "source_sha256": "ab92800dd4e4d4053498c317effadedf52568270ab7b476eef74b09638de8964",
        "wasm_sha256": "97068a6310a3bc535369e2b6277b801875c14ca206a707fcb9bca9036f41d0df",
        "workload_kind": "typed_scalar_driver"
      },
      "1:branch_calls": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 3,
            "sides": [
              {
                "calibrated_total_ns": 19962255.998999998,
                "ns_per_operation": 6654085.333,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 20141897.001000002,
                "ns_per_operation": 6713965.667,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 4,
            "sides": [
              {
                "calibrated_total_ns": 25359937.0,
                "ns_per_operation": 6339984.25,
                "observed_batch": 4,
                "requested_batch": 4,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 26118501.0,
                "ns_per_operation": 6529625.25,
                "observed_batch": 4,
                "requested_batch": 4,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 25359937.0,
            "ns_per_operation": 6339984.25,
            "observed_batch": 4,
            "requested_batch": 4,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 26118501.0,
            "ns_per_operation": 6529625.25,
            "observed_batch": 4,
            "requested_batch": 4,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 20252301.0,
            "ns_per_operation": 6750767.0,
            "observed_batch": 3,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 19330691.001000002,
            "ns_per_operation": 6443563.667,
            "observed_batch": 3,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 4,
        "source_sha256": "d0af4fdf9ad5be38344267ae72fc1855edb5a8fa1b475fd0af4cb507730b2309",
        "wasm_sha256": "9b58d26f201900b5b6cce684133a98ab68b3cf55d979b64ce895e914e9d5327b",
        "workload_kind": "legacy_single_case_control"
      },
      "1:call_overhead": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 48662,
            "sides": [
              {
                "calibrated_total_ns": 24475720.788,
                "ns_per_operation": 502.974,
                "observed_batch": 48662,
                "requested_batch": 48662,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 22232013.292,
                "ns_per_operation": 456.866,
                "observed_batch": 48662,
                "requested_batch": 48662,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 24475720.788,
            "ns_per_operation": 502.974,
            "observed_batch": 48662,
            "requested_batch": 48662,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22232013.292,
            "ns_per_operation": 456.866,
            "observed_batch": 48662,
            "requested_batch": 48662,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 13459238.432,
            "ns_per_operation": 349.936,
            "observed_batch": 38462,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 29720267.838,
            "ns_per_operation": 610.749,
            "observed_batch": 48662,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 48662,
        "source_sha256": "9493b8f751690a2fd6e8b19dcee5e2c1d50b717253d292ce34d5692b306663a5",
        "wasm_sha256": "0815ca7d6026206d0773fe10c6b9643862bfffe8ba361c9789369ef9668ee746",
        "workload_kind": "legacy_single_case_control"
      },
      "1:integer_loop": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 3,
            "sides": [
              {
                "calibrated_total_ns": 20565315.998999998,
                "ns_per_operation": 6855105.333,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 23326545.998999998,
                "ns_per_operation": 7775515.333,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 20565315.998999998,
            "ns_per_operation": 6855105.333,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 23326545.998999998,
            "ns_per_operation": 7775515.333,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 17609318.001000002,
            "ns_per_operation": 5869772.667,
            "observed_batch": 3,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 18423272.001000002,
            "ns_per_operation": 6141090.667,
            "observed_batch": 3,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 3,
        "source_sha256": "791730112109f3d5751a7b3e2c1134e7e0e6f71a7f7ee01ea8481eb989ee3fa4",
        "wasm_sha256": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c",
        "workload_kind": "legacy_single_case_control"
      },
      "1:scalar_accumulator_n0": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 30304,
            "sides": [
              {
                "calibrated_total_ns": 38445472.64,
                "ns_per_operation": 1268.66,
                "observed_batch": 30304,
                "requested_batch": 30304,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 26588456.864,
                "ns_per_operation": 877.391,
                "observed_batch": 30304,
                "requested_batch": 30304,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 38445472.64,
            "ns_per_operation": 1268.66,
            "observed_batch": 30304,
            "requested_batch": 30304,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 26588456.864,
            "ns_per_operation": 877.391,
            "observed_batch": 30304,
            "requested_batch": 30304,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 10752171.495,
            "ns_per_operation": 1262.733,
            "observed_batch": 8515,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 45810587.104,
            "ns_per_operation": 1511.701,
            "observed_batch": 30304,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 30304,
        "source_sha256": "61aaec7a86d481378f31f3e27b0c1413684065841fa5909297181fcad9bef10d",
        "wasm_sha256": "ef61bf7afc44af7b71ef78eadcba48c0d85949d7d9096df8fbbbab53c05036b1",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_accumulator_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 19551,
            "sides": [
              {
                "calibrated_total_ns": 26164849.137000002,
                "ns_per_operation": 1338.287,
                "observed_batch": 19551,
                "requested_batch": 19551,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 28328538.755999997,
                "ns_per_operation": 1448.956,
                "observed_batch": 19551,
                "requested_batch": 19551,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 26164849.137000002,
            "ns_per_operation": 1338.287,
            "observed_batch": 19551,
            "requested_batch": 19551,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 28328538.755999997,
            "ns_per_operation": 1448.956,
            "observed_batch": 19551,
            "requested_batch": 19551,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 11370312.722,
            "ns_per_operation": 1404.089,
            "observed_batch": 8098,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 28322047.824,
            "ns_per_operation": 1448.624,
            "observed_batch": 19551,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 19551,
        "source_sha256": "61aaec7a86d481378f31f3e27b0c1413684065841fa5909297181fcad9bef10d",
        "wasm_sha256": "ef61bf7afc44af7b71ef78eadcba48c0d85949d7d9096df8fbbbab53c05036b1",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_accumulator_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 4,
            "sides": [
              {
                "calibrated_total_ns": 23795852.0,
                "ns_per_operation": 5948963.0,
                "observed_batch": 4,
                "requested_batch": 4,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 27495830.0,
                "ns_per_operation": 6873957.5,
                "observed_batch": 4,
                "requested_batch": 4,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 23795852.0,
            "ns_per_operation": 5948963.0,
            "observed_batch": 4,
            "requested_batch": 4,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27495830.0,
            "ns_per_operation": 6873957.5,
            "observed_batch": 4,
            "requested_batch": 4,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 30401059.0,
            "ns_per_operation": 7600264.75,
            "observed_batch": 4,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 21621093.0,
            "ns_per_operation": 7207031.0,
            "observed_batch": 3,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 4,
        "source_sha256": "61aaec7a86d481378f31f3e27b0c1413684065841fa5909297181fcad9bef10d",
        "wasm_sha256": "ef61bf7afc44af7b71ef78eadcba48c0d85949d7d9096df8fbbbab53c05036b1",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_accumulator_n1024": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 28,
            "sides": [
              {
                "calibrated_total_ns": 18525708.992000002,
                "ns_per_operation": 661632.464,
                "observed_batch": 28,
                "requested_batch": 28,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 19382634.992000002,
                "ns_per_operation": 692236.964,
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
                "calibrated_total_ns": 21093444.994,
                "ns_per_operation": 620395.441,
                "observed_batch": 34,
                "requested_batch": 34,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 20620798.992,
                "ns_per_operation": 606494.088,
                "observed_batch": 34,
                "requested_batch": 34,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 21093444.994,
            "ns_per_operation": 620395.441,
            "observed_batch": 34,
            "requested_batch": 34,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 20620798.992,
            "ns_per_operation": 606494.088,
            "observed_batch": 34,
            "requested_batch": 34,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 21350206.989,
            "ns_per_operation": 790748.407,
            "observed_batch": 27,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 20039811.987999998,
            "ns_per_operation": 715707.571,
            "observed_batch": 28,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 34,
        "source_sha256": "61aaec7a86d481378f31f3e27b0c1413684065841fa5909297181fcad9bef10d",
        "wasm_sha256": "ef61bf7afc44af7b71ef78eadcba48c0d85949d7d9096df8fbbbab53c05036b1",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_accumulator_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 2408,
            "sides": [
              {
                "calibrated_total_ns": 42218862.0,
                "ns_per_operation": 17532.75,
                "observed_batch": 2408,
                "requested_batch": 2408,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 41921604.032,
                "ns_per_operation": 17409.304,
                "observed_batch": 2408,
                "requested_batch": 2408,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 42218862.0,
            "ns_per_operation": 17532.75,
            "observed_batch": 2408,
            "requested_batch": 2408,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 41921604.032,
            "ns_per_operation": 17409.304,
            "observed_batch": 2408,
            "requested_batch": 2408,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 24251440.512000002,
            "ns_per_operation": 14574.183,
            "observed_batch": 1664,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 32112806.264,
            "ns_per_operation": 13335.883,
            "observed_batch": 2408,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 2408,
        "source_sha256": "61aaec7a86d481378f31f3e27b0c1413684065841fa5909297181fcad9bef10d",
        "wasm_sha256": "ef61bf7afc44af7b71ef78eadcba48c0d85949d7d9096df8fbbbab53c05036b1",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_accumulator_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1505,
            "sides": [
              {
                "calibrated_total_ns": 32979209.48,
                "ns_per_operation": 21913.096,
                "observed_batch": 1505,
                "requested_batch": 1505,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 30449238.259999998,
                "ns_per_operation": 20232.052,
                "observed_batch": 1505,
                "requested_batch": 1505,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 32979209.48,
            "ns_per_operation": 21913.096,
            "observed_batch": 1505,
            "requested_batch": 1505,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 30449238.259999998,
            "ns_per_operation": 20232.052,
            "observed_batch": 1505,
            "requested_batch": 1505,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 23632107.45,
            "ns_per_operation": 22506.769,
            "observed_batch": 1050,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 35957079.235,
            "ns_per_operation": 23891.747,
            "observed_batch": 1505,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1505,
        "source_sha256": "61aaec7a86d481378f31f3e27b0c1413684065841fa5909297181fcad9bef10d",
        "wasm_sha256": "ef61bf7afc44af7b71ef78eadcba48c0d85949d7d9096df8fbbbab53c05036b1",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_br_table_n0": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 31008,
            "sides": [
              {
                "calibrated_total_ns": 31518546.720000003,
                "ns_per_operation": 1016.465,
                "observed_batch": 31008,
                "requested_batch": 31008,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 29624547.072,
                "ns_per_operation": 955.384,
                "observed_batch": 31008,
                "requested_batch": 31008,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 31518546.720000003,
            "ns_per_operation": 1016.465,
            "observed_batch": 31008,
            "requested_batch": 31008,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 29624547.072,
            "ns_per_operation": 955.384,
            "observed_batch": 31008,
            "requested_batch": 31008,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 13084517.215,
            "ns_per_operation": 1048.019,
            "observed_batch": 12485,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 36592757.856,
            "ns_per_operation": 1180.107,
            "observed_batch": 31008,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 31008,
        "source_sha256": "637b96cd8926ffadf728af4949b43fa6b62fffb61968c14c716e0c203a1e5e6d",
        "wasm_sha256": "d43d5dd71d5ca1c3dae820aa85c13a2d69415e028b8d48097fed4558f743163a",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_br_table_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 13918,
            "sides": [
              {
                "calibrated_total_ns": 24873470.192,
                "ns_per_operation": 1787.144,
                "observed_batch": 13918,
                "requested_batch": 13918,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 27621189.588,
                "ns_per_operation": 1984.566,
                "observed_batch": 13918,
                "requested_batch": 13918,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 24873470.192,
            "ns_per_operation": 1787.144,
            "observed_batch": 13918,
            "requested_batch": 13918,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27621189.588,
            "ns_per_operation": 1984.566,
            "observed_batch": 13918,
            "requested_batch": 13918,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 32348241.486,
            "ns_per_operation": 2367.753,
            "observed_batch": 13662,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 31295402.408,
            "ns_per_operation": 2248.556,
            "observed_batch": 13918,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 13918,
        "source_sha256": "637b96cd8926ffadf728af4949b43fa6b62fffb61968c14c716e0c203a1e5e6d",
        "wasm_sha256": "d43d5dd71d5ca1c3dae820aa85c13a2d69415e028b8d48097fed4558f743163a",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_br_table_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 2,
            "sides": [
              {
                "calibrated_total_ns": 62016285.0,
                "ns_per_operation": 31008142.5,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 24033986.0,
                "ns_per_operation": 12016993.0,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 62016285.0,
            "ns_per_operation": 31008142.5,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 24033986.0,
            "ns_per_operation": 12016993.0,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 33527211.0,
            "ns_per_operation": 16763605.5,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 24572493.0,
            "ns_per_operation": 12286246.5,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 2,
        "source_sha256": "637b96cd8926ffadf728af4949b43fa6b62fffb61968c14c716e0c203a1e5e6d",
        "wasm_sha256": "d43d5dd71d5ca1c3dae820aa85c13a2d69415e028b8d48097fed4558f743163a",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_br_table_n1024": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 17,
            "sides": [
              {
                "calibrated_total_ns": 25976250.002,
                "ns_per_operation": 1528014.706,
                "observed_batch": 17,
                "requested_batch": 17,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 22777770.994,
                "ns_per_operation": 1339868.882,
                "observed_batch": 17,
                "requested_batch": 17,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 25976250.002,
            "ns_per_operation": 1528014.706,
            "observed_batch": 17,
            "requested_batch": 17,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22777770.994,
            "ns_per_operation": 1339868.882,
            "observed_batch": 17,
            "requested_batch": 17,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 14340557.997000001,
            "ns_per_operation": 1593395.333,
            "observed_batch": 9,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 25669500.999,
            "ns_per_operation": 1509970.647,
            "observed_batch": 17,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 17,
        "source_sha256": "637b96cd8926ffadf728af4949b43fa6b62fffb61968c14c716e0c203a1e5e6d",
        "wasm_sha256": "d43d5dd71d5ca1c3dae820aa85c13a2d69415e028b8d48097fed4558f743163a",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_br_table_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1457,
            "sides": [
              {
                "calibrated_total_ns": 36414902.99,
                "ns_per_operation": 24993.07,
                "observed_batch": 1457,
                "requested_batch": 1457,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 31529133.233999997,
                "ns_per_operation": 21639.762,
                "observed_batch": 1457,
                "requested_batch": 1457,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 36414902.99,
            "ns_per_operation": 24993.07,
            "observed_batch": 1457,
            "requested_batch": 1457,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 31529133.233999997,
            "ns_per_operation": 21639.762,
            "observed_batch": 1457,
            "requested_batch": 1457,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 38292987.935,
            "ns_per_operation": 28470.623,
            "observed_batch": 1345,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 33310254.540000003,
            "ns_per_operation": 22862.22,
            "observed_batch": 1457,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1457,
        "source_sha256": "637b96cd8926ffadf728af4949b43fa6b62fffb61968c14c716e0c203a1e5e6d",
        "wasm_sha256": "d43d5dd71d5ca1c3dae820aa85c13a2d69415e028b8d48097fed4558f743163a",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_br_table_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 739,
            "sides": [
              {
                "calibrated_total_ns": 30952125.983,
                "ns_per_operation": 41883.797,
                "observed_batch": 739,
                "requested_batch": 739,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 27642945.319999997,
                "ns_per_operation": 37405.88,
                "observed_batch": 739,
                "requested_batch": 739,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 30952125.983,
            "ns_per_operation": 41883.797,
            "observed_batch": 739,
            "requested_batch": 739,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27642945.319999997,
            "ns_per_operation": 37405.88,
            "observed_batch": 739,
            "requested_batch": 739,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 32504819.669,
            "ns_per_operation": 43984.871,
            "observed_batch": 739,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 1282928.0099999998,
            "ns_per_operation": 32895.59,
            "observed_batch": 39,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 739,
        "source_sha256": "637b96cd8926ffadf728af4949b43fa6b62fffb61968c14c716e0c203a1e5e6d",
        "wasm_sha256": "d43d5dd71d5ca1c3dae820aa85c13a2d69415e028b8d48097fed4558f743163a",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_branch_entropy_n0": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 23895,
            "sides": [
              {
                "calibrated_total_ns": 17711786.43,
                "ns_per_operation": 741.234,
                "observed_batch": 23895,
                "requested_batch": 23895,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 20831828.265,
                "ns_per_operation": 871.807,
                "observed_batch": 23895,
                "requested_batch": 23895,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 29681,
            "sides": [
              {
                "calibrated_total_ns": 24084766.174,
                "ns_per_operation": 811.454,
                "observed_batch": 29681,
                "requested_batch": 29681,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 25990612.865,
                "ns_per_operation": 875.665,
                "observed_batch": 29681,
                "requested_batch": 29681,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 24084766.174,
            "ns_per_operation": 811.454,
            "observed_batch": 29681,
            "requested_batch": 29681,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 25990612.865,
            "ns_per_operation": 875.665,
            "observed_batch": 29681,
            "requested_batch": 29681,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 20664419.895,
            "ns_per_operation": 864.801,
            "observed_batch": 23895,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 6535723.159,
            "ns_per_operation": 785.827,
            "observed_batch": 8317,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 29681,
        "source_sha256": "6b4861d8bfbbd7e2718e577ec2d479bfff32e94dc47a3b7dda9a1b363baa98b6",
        "wasm_sha256": "e4c87dbab50a2438cb1f749bc6fd6f9c834c81484acd42864735bd1ad1b6dec0",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_branch_entropy_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 12301,
            "sides": [
              {
                "calibrated_total_ns": 33475486.262999997,
                "ns_per_operation": 2721.363,
                "observed_batch": 12301,
                "requested_batch": 12301,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 24790783.447,
                "ns_per_operation": 2015.347,
                "observed_batch": 12301,
                "requested_batch": 12301,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 33475486.262999997,
            "ns_per_operation": 2721.363,
            "observed_batch": 12301,
            "requested_batch": 12301,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 24790783.447,
            "ns_per_operation": 2015.347,
            "observed_batch": 12301,
            "requested_batch": 12301,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 27424316.838,
            "ns_per_operation": 2229.438,
            "observed_batch": 12301,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 14170981.056,
            "ns_per_operation": 2123.948,
            "observed_batch": 6672,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 12301,
        "source_sha256": "6b4861d8bfbbd7e2718e577ec2d479bfff32e94dc47a3b7dda9a1b363baa98b6",
        "wasm_sha256": "e4c87dbab50a2438cb1f749bc6fd6f9c834c81484acd42864735bd1ad1b6dec0",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_branch_entropy_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 2,
            "sides": [
              {
                "calibrated_total_ns": 22000081.0,
                "ns_per_operation": 11000040.5,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 22097154.0,
                "ns_per_operation": 11048577.0,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 22000081.0,
            "ns_per_operation": 11000040.5,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22097154.0,
            "ns_per_operation": 11048577.0,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 33267390.0,
            "ns_per_operation": 16633695.0,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 26650357.0,
            "ns_per_operation": 13325178.5,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 2,
        "source_sha256": "6b4861d8bfbbd7e2718e577ec2d479bfff32e94dc47a3b7dda9a1b363baa98b6",
        "wasm_sha256": "e4c87dbab50a2438cb1f749bc6fd6f9c834c81484acd42864735bd1ad1b6dec0",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_branch_entropy_n1024": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 17,
            "sides": [
              {
                "calibrated_total_ns": 31122950.005,
                "ns_per_operation": 1830761.765,
                "observed_batch": 17,
                "requested_batch": 17,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 19728931.001,
                "ns_per_operation": 1160525.353,
                "observed_batch": 17,
                "requested_batch": 17,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 19,
            "sides": [
              {
                "calibrated_total_ns": 25446185.005,
                "ns_per_operation": 1339272.895,
                "observed_batch": 19,
                "requested_batch": 19,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 24073541.005999997,
                "ns_per_operation": 1267028.474,
                "observed_batch": 19,
                "requested_batch": 19,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 25446185.005,
            "ns_per_operation": 1339272.895,
            "observed_batch": 19,
            "requested_batch": 19,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 24073541.005999997,
            "ns_per_operation": 1267028.474,
            "observed_batch": 19,
            "requested_batch": 19,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 21939891.994,
            "ns_per_operation": 1290581.882,
            "observed_batch": 17,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 13632351.997,
            "ns_per_operation": 1239304.727,
            "observed_batch": 11,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 19,
        "source_sha256": "6b4861d8bfbbd7e2718e577ec2d479bfff32e94dc47a3b7dda9a1b363baa98b6",
        "wasm_sha256": "e4c87dbab50a2438cb1f749bc6fd6f9c834c81484acd42864735bd1ad1b6dec0",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_branch_entropy_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1392,
            "sides": [
              {
                "calibrated_total_ns": 27438616.8,
                "ns_per_operation": 19711.65,
                "observed_batch": 1392,
                "requested_batch": 1392,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 30450587.424,
                "ns_per_operation": 21875.422,
                "observed_batch": 1392,
                "requested_batch": 1392,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 27438616.8,
            "ns_per_operation": 19711.65,
            "observed_batch": 1392,
            "requested_batch": 1392,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 30450587.424,
            "ns_per_operation": 21875.422,
            "observed_batch": 1392,
            "requested_batch": 1392,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 36453100.224,
            "ns_per_operation": 26187.572,
            "observed_batch": 1392,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 52647573.888,
            "ns_per_operation": 40623.128,
            "observed_batch": 1296,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1392,
        "source_sha256": "6b4861d8bfbbd7e2718e577ec2d479bfff32e94dc47a3b7dda9a1b363baa98b6",
        "wasm_sha256": "e4c87dbab50a2438cb1f749bc6fd6f9c834c81484acd42864735bd1ad1b6dec0",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_branch_entropy_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 749,
            "sides": [
              {
                "calibrated_total_ns": 34176899.96,
                "ns_per_operation": 45630.04,
                "observed_batch": 749,
                "requested_batch": 749,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 44272249.273,
                "ns_per_operation": 59108.477,
                "observed_batch": 749,
                "requested_batch": 749,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 34176899.96,
            "ns_per_operation": 45630.04,
            "observed_batch": 749,
            "requested_batch": 749,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 44272249.273,
            "ns_per_operation": 59108.477,
            "observed_batch": 749,
            "requested_batch": 749,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 30947960.211,
            "ns_per_operation": 41319.039,
            "observed_batch": 749,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 31448898.752000004,
            "ns_per_operation": 42729.482,
            "observed_batch": 736,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 749,
        "source_sha256": "6b4861d8bfbbd7e2718e577ec2d479bfff32e94dc47a3b7dda9a1b363baa98b6",
        "wasm_sha256": "e4c87dbab50a2438cb1f749bc6fd6f9c834c81484acd42864735bd1ad1b6dec0",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_call_ref_n0": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 27473,
            "sides": [
              {
                "calibrated_total_ns": 21431192.786,
                "ns_per_operation": 780.082,
                "observed_batch": 27473,
                "requested_batch": 27473,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 23990714.831,
                "ns_per_operation": 873.247,
                "observed_batch": 27473,
                "requested_batch": 27473,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 21431192.786,
            "ns_per_operation": 780.082,
            "observed_batch": 27473,
            "requested_batch": 27473,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 23990714.831,
            "ns_per_operation": 873.247,
            "observed_batch": 27473,
            "requested_batch": 27473,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 19992196.528,
            "ns_per_operation": 781.678,
            "observed_batch": 25576,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 23331445.25,
            "ns_per_operation": 849.25,
            "observed_batch": 27473,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 27473,
        "source_sha256": "0323154b79000eeeb0a63310541ec91b6c4d91e32a6a3406125a30f2c8e53a12",
        "wasm_sha256": "4f766dc9e29324ea942c892982363dec0ade98565288878a70967ce3b33f322b",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_call_ref_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 13765,
            "sides": [
              {
                "calibrated_total_ns": 45753015.49,
                "ns_per_operation": 3323.866,
                "observed_batch": 13765,
                "requested_batch": 13765,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 29670030.785,
                "ns_per_operation": 2155.469,
                "observed_batch": 13765,
                "requested_batch": 13765,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 45753015.49,
            "ns_per_operation": 3323.866,
            "observed_batch": 13765,
            "requested_batch": 13765,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 29670030.785,
            "ns_per_operation": 2155.469,
            "observed_batch": 13765,
            "requested_batch": 13765,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 37363651.030999996,
            "ns_per_operation": 2776.109,
            "observed_batch": 13459,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 30631612.39,
            "ns_per_operation": 2225.326,
            "observed_batch": 13765,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 13765,
        "source_sha256": "0323154b79000eeeb0a63310541ec91b6c4d91e32a6a3406125a30f2c8e53a12",
        "wasm_sha256": "4f766dc9e29324ea942c892982363dec0ade98565288878a70967ce3b33f322b",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_call_ref_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 2,
            "sides": [
              {
                "calibrated_total_ns": 24076470.0,
                "ns_per_operation": 12038235.0,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 24342449.0,
                "ns_per_operation": 12171224.5,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 24076470.0,
            "ns_per_operation": 12038235.0,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 24342449.0,
            "ns_per_operation": 12171224.5,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 20923603.0,
            "ns_per_operation": 10461801.5,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 21171298.0,
            "ns_per_operation": 10585649.0,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 2,
        "source_sha256": "0323154b79000eeeb0a63310541ec91b6c4d91e32a6a3406125a30f2c8e53a12",
        "wasm_sha256": "4f766dc9e29324ea942c892982363dec0ade98565288878a70967ce3b33f322b",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_call_ref_n1024": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 19,
            "sides": [
              {
                "calibrated_total_ns": 27747158.991,
                "ns_per_operation": 1460376.789,
                "observed_batch": 19,
                "requested_batch": 19,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 18587269.992,
                "ns_per_operation": 978277.368,
                "observed_batch": 19,
                "requested_batch": 19,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 23,
            "sides": [
              {
                "calibrated_total_ns": 35594524.0,
                "ns_per_operation": 1547588.0,
                "observed_batch": 23,
                "requested_batch": 23,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 22557515.994999997,
                "ns_per_operation": 980761.565,
                "observed_batch": 23,
                "requested_batch": 23,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 35594524.0,
            "ns_per_operation": 1547588.0,
            "observed_batch": 23,
            "requested_batch": 23,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22557515.994999997,
            "ns_per_operation": 980761.565,
            "observed_batch": 23,
            "requested_batch": 23,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 23989911.005,
            "ns_per_operation": 1262626.895,
            "observed_batch": 19,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 17647422.0,
            "ns_per_operation": 1102963.875,
            "observed_batch": 16,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 23,
        "source_sha256": "0323154b79000eeeb0a63310541ec91b6c4d91e32a6a3406125a30f2c8e53a12",
        "wasm_sha256": "4f766dc9e29324ea942c892982363dec0ade98565288878a70967ce3b33f322b",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_call_ref_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1567,
            "sides": [
              {
                "calibrated_total_ns": 40210542.548,
                "ns_per_operation": 25660.844,
                "observed_batch": 1567,
                "requested_batch": 1567,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 33183835.622,
                "ns_per_operation": 21176.666,
                "observed_batch": 1567,
                "requested_batch": 1567,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 40210542.548,
            "ns_per_operation": 25660.844,
            "observed_batch": 1567,
            "requested_batch": 1567,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 33183835.622,
            "ns_per_operation": 21176.666,
            "observed_batch": 1567,
            "requested_batch": 1567,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 36874166.437,
            "ns_per_operation": 23743.829,
            "observed_batch": 1553,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 41497347.278,
            "ns_per_operation": 26482.034,
            "observed_batch": 1567,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1567,
        "source_sha256": "0323154b79000eeeb0a63310541ec91b6c4d91e32a6a3406125a30f2c8e53a12",
        "wasm_sha256": "4f766dc9e29324ea942c892982363dec0ade98565288878a70967ce3b33f322b",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_call_ref_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 841,
            "sides": [
              {
                "calibrated_total_ns": 31171180.584,
                "ns_per_operation": 37064.424,
                "observed_batch": 841,
                "requested_batch": 841,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 32315306.418999996,
                "ns_per_operation": 38424.859,
                "observed_batch": 841,
                "requested_batch": 841,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 31171180.584,
            "ns_per_operation": 37064.424,
            "observed_batch": 841,
            "requested_batch": 841,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 32315306.418999996,
            "ns_per_operation": 38424.859,
            "observed_batch": 841,
            "requested_batch": 841,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 37793709.092,
            "ns_per_operation": 44939.012,
            "observed_batch": 841,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 32708437.380000003,
            "ns_per_operation": 40133.052,
            "observed_batch": 815,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 841,
        "source_sha256": "0323154b79000eeeb0a63310541ec91b6c4d91e32a6a3406125a30f2c8e53a12",
        "wasm_sha256": "4f766dc9e29324ea942c892982363dec0ade98565288878a70967ce3b33f322b",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_dependent_n0": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 30304,
            "sides": [
              {
                "calibrated_total_ns": 22599692.864,
                "ns_per_operation": 745.766,
                "observed_batch": 30304,
                "requested_batch": 30304,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 26556183.104000002,
                "ns_per_operation": 876.326,
                "observed_batch": 30304,
                "requested_batch": 30304,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 22599692.864,
            "ns_per_operation": 745.766,
            "observed_batch": 30304,
            "requested_batch": 30304,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 26556183.104000002,
            "ns_per_operation": 876.326,
            "observed_batch": 30304,
            "requested_batch": 30304,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 19147438.426,
            "ns_per_operation": 790.759,
            "observed_batch": 24214,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 21520870.464,
            "ns_per_operation": 710.166,
            "observed_batch": 30304,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 30304,
        "source_sha256": "0dd5e9778eb09bac9ee432e5fece3f432d034ea9878d2302972142501f0c60c6",
        "wasm_sha256": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_dependent_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 17362,
            "sides": [
              {
                "calibrated_total_ns": 29636326.33,
                "ns_per_operation": 1706.965,
                "observed_batch": 17362,
                "requested_batch": 17362,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 32497445.034,
                "ns_per_operation": 1871.757,
                "observed_batch": 17362,
                "requested_batch": 17362,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 29636326.33,
            "ns_per_operation": 1706.965,
            "observed_batch": 17362,
            "requested_batch": 17362,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 32497445.034,
            "ns_per_operation": 1871.757,
            "observed_batch": 17362,
            "requested_batch": 17362,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 19062874.21,
            "ns_per_operation": 3658.901,
            "observed_batch": 5210,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 31761921.266,
            "ns_per_operation": 1829.393,
            "observed_batch": 17362,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 17362,
        "source_sha256": "0dd5e9778eb09bac9ee432e5fece3f432d034ea9878d2302972142501f0c60c6",
        "wasm_sha256": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_dependent_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 3,
            "sides": [
              {
                "calibrated_total_ns": 28850120.001,
                "ns_per_operation": 9616706.667,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 22864053.998999998,
                "ns_per_operation": 7621351.333,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 28850120.001,
            "ns_per_operation": 9616706.667,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22864053.998999998,
            "ns_per_operation": 7621351.333,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 18385104.0,
            "ns_per_operation": 6128368.0,
            "observed_batch": 3,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 19638357.998999998,
            "ns_per_operation": 6546119.333,
            "observed_batch": 3,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 3,
        "source_sha256": "0dd5e9778eb09bac9ee432e5fece3f432d034ea9878d2302972142501f0c60c6",
        "wasm_sha256": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_dependent_n1024": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 33,
            "sides": [
              {
                "calibrated_total_ns": 21407964.006,
                "ns_per_operation": 648726.182,
                "observed_batch": 33,
                "requested_batch": 33,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 32040168.006,
                "ns_per_operation": 970914.182,
                "observed_batch": 33,
                "requested_batch": 33,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 21407964.006,
            "ns_per_operation": 648726.182,
            "observed_batch": 33,
            "requested_batch": 33,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 32040168.006,
            "ns_per_operation": 970914.182,
            "observed_batch": 33,
            "requested_batch": 33,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 23405622.998999998,
            "ns_per_operation": 709261.303,
            "observed_batch": 33,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 24779372.01,
            "ns_per_operation": 825979.067,
            "observed_batch": 30,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 33,
        "source_sha256": "0dd5e9778eb09bac9ee432e5fece3f432d034ea9878d2302972142501f0c60c6",
        "wasm_sha256": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_dependent_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 2406,
            "sides": [
              {
                "calibrated_total_ns": 28346551.254,
                "ns_per_operation": 11781.609,
                "observed_batch": 2406,
                "requested_batch": 2406,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 31036885.116,
                "ns_per_operation": 12899.786,
                "observed_batch": 2406,
                "requested_batch": 2406,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 28346551.254,
            "ns_per_operation": 11781.609,
            "observed_batch": 2406,
            "requested_batch": 2406,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 31036885.116,
            "ns_per_operation": 12899.786,
            "observed_batch": 2406,
            "requested_batch": 2406,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 36236432.472,
            "ns_per_operation": 15296.088,
            "observed_batch": 2369,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 28059578.009999998,
            "ns_per_operation": 11662.335,
            "observed_batch": 2406,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 2406,
        "source_sha256": "0dd5e9778eb09bac9ee432e5fece3f432d034ea9878d2302972142501f0c60c6",
        "wasm_sha256": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_dependent_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1327,
            "sides": [
              {
                "calibrated_total_ns": 41372530.557000004,
                "ns_per_operation": 31177.491,
                "observed_batch": 1327,
                "requested_batch": 1327,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 43984033.382,
                "ns_per_operation": 33145.466,
                "observed_batch": 1327,
                "requested_batch": 1327,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 41372530.557000004,
            "ns_per_operation": 31177.491,
            "observed_batch": 1327,
            "requested_batch": 1327,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 43984033.382,
            "ns_per_operation": 33145.466,
            "observed_batch": 1327,
            "requested_batch": 1327,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 42744786.744,
            "ns_per_operation": 45184.764,
            "observed_batch": 946,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 46605979.697000004,
            "ns_per_operation": 35121.311,
            "observed_batch": 1327,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1327,
        "source_sha256": "0dd5e9778eb09bac9ee432e5fece3f432d034ea9878d2302972142501f0c60c6",
        "wasm_sha256": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_direct_calls_n0": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 29028,
            "sides": [
              {
                "calibrated_total_ns": 25833671.796,
                "ns_per_operation": 889.957,
                "observed_batch": 29028,
                "requested_batch": 29028,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 57996318.432,
                "ns_per_operation": 1997.944,
                "observed_batch": 29028,
                "requested_batch": 29028,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 25833671.796,
            "ns_per_operation": 889.957,
            "observed_batch": 29028,
            "requested_batch": 29028,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 57996318.432,
            "ns_per_operation": 1997.944,
            "observed_batch": 29028,
            "requested_batch": 29028,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 19513215.276,
            "ns_per_operation": 735.626,
            "observed_batch": 26526,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 31665977.555999998,
            "ns_per_operation": 1090.877,
            "observed_batch": 29028,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 29028,
        "source_sha256": "0a075601a2a651f9cb95e8ee0404c64bbf076d17bf242b9c88fecbb8cb973826",
        "wasm_sha256": "f6adbecce99c05bf06f213f4f5c0d5d9f001074acbbb2e6bbe814696bb866e45",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_direct_calls_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 14420,
            "sides": [
              {
                "calibrated_total_ns": 35668100.72,
                "ns_per_operation": 2473.516,
                "observed_batch": 14420,
                "requested_batch": 14420,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 33477587.360000003,
                "ns_per_operation": 2321.608,
                "observed_batch": 14420,
                "requested_batch": 14420,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 35668100.72,
            "ns_per_operation": 2473.516,
            "observed_batch": 14420,
            "requested_batch": 14420,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 33477587.360000003,
            "ns_per_operation": 2321.608,
            "observed_batch": 14420,
            "requested_batch": 14420,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 37046989.892000005,
            "ns_per_operation": 2656.079,
            "observed_batch": 13948,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 31477893.86,
            "ns_per_operation": 2182.933,
            "observed_batch": 14420,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 14420,
        "source_sha256": "0a075601a2a651f9cb95e8ee0404c64bbf076d17bf242b9c88fecbb8cb973826",
        "wasm_sha256": "f6adbecce99c05bf06f213f4f5c0d5d9f001074acbbb2e6bbe814696bb866e45",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_direct_calls_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 3,
            "sides": [
              {
                "calibrated_total_ns": 28718226.999,
                "ns_per_operation": 9572742.333,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 28801080.999,
                "ns_per_operation": 9600360.333,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 28718226.999,
            "ns_per_operation": 9572742.333,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 28801080.999,
            "ns_per_operation": 9600360.333,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 32748272.001,
            "ns_per_operation": 10916090.667,
            "observed_batch": 3,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 19878271.0,
            "ns_per_operation": 9939135.5,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 3,
        "source_sha256": "0a075601a2a651f9cb95e8ee0404c64bbf076d17bf242b9c88fecbb8cb973826",
        "wasm_sha256": "f6adbecce99c05bf06f213f4f5c0d5d9f001074acbbb2e6bbe814696bb866e45",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_direct_calls_n1024": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 20,
            "sides": [
              {
                "calibrated_total_ns": 18708379.0,
                "ns_per_operation": 935418.95,
                "observed_batch": 20,
                "requested_batch": 20,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 21194821.0,
                "ns_per_operation": 1059741.05,
                "observed_batch": 20,
                "requested_batch": 20,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 24,
            "sides": [
              {
                "calibrated_total_ns": 29684842.992000002,
                "ns_per_operation": 1236868.458,
                "observed_batch": 24,
                "requested_batch": 24,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 28894441.992000002,
                "ns_per_operation": 1203935.083,
                "observed_batch": 24,
                "requested_batch": 24,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 29684842.992000002,
            "ns_per_operation": 1236868.458,
            "observed_batch": 24,
            "requested_batch": 24,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 28894441.992000002,
            "ns_per_operation": 1203935.083,
            "observed_batch": 24,
            "requested_batch": 24,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 27644641.0,
            "ns_per_operation": 1382232.05,
            "observed_batch": 20,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27132917.994000003,
            "ns_per_operation": 1507384.333,
            "observed_batch": 18,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 24,
        "source_sha256": "0a075601a2a651f9cb95e8ee0404c64bbf076d17bf242b9c88fecbb8cb973826",
        "wasm_sha256": "f6adbecce99c05bf06f213f4f5c0d5d9f001074acbbb2e6bbe814696bb866e45",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_direct_calls_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1736,
            "sides": [
              {
                "calibrated_total_ns": 33125666.279999997,
                "ns_per_operation": 19081.605,
                "observed_batch": 1736,
                "requested_batch": 1736,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 37510739.784,
                "ns_per_operation": 21607.569,
                "observed_batch": 1736,
                "requested_batch": 1736,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 33125666.279999997,
            "ns_per_operation": 19081.605,
            "observed_batch": 1736,
            "requested_batch": 1736,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 37510739.784,
            "ns_per_operation": 21607.569,
            "observed_batch": 1736,
            "requested_batch": 1736,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 33861071.53,
            "ns_per_operation": 19859.866,
            "observed_batch": 1705,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 36238041.728,
            "ns_per_operation": 20874.448,
            "observed_batch": 1736,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1736,
        "source_sha256": "0a075601a2a651f9cb95e8ee0404c64bbf076d17bf242b9c88fecbb8cb973826",
        "wasm_sha256": "f6adbecce99c05bf06f213f4f5c0d5d9f001074acbbb2e6bbe814696bb866e45",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_direct_calls_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 959,
            "sides": [
              {
                "calibrated_total_ns": 36132544.126,
                "ns_per_operation": 37677.314,
                "observed_batch": 959,
                "requested_batch": 959,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 37330241.144,
                "ns_per_operation": 38926.216,
                "observed_batch": 959,
                "requested_batch": 959,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 36132544.126,
            "ns_per_operation": 37677.314,
            "observed_batch": 959,
            "requested_batch": 959,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 37330241.144,
            "ns_per_operation": 38926.216,
            "observed_batch": 959,
            "requested_batch": 959,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 36518052.326000005,
            "ns_per_operation": 39224.546,
            "observed_batch": 931,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 33925256.981,
            "ns_per_operation": 35375.659,
            "observed_batch": 959,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 959,
        "source_sha256": "0a075601a2a651f9cb95e8ee0404c64bbf076d17bf242b9c88fecbb8cb973826",
        "wasm_sha256": "f6adbecce99c05bf06f213f4f5c0d5d9f001074acbbb2e6bbe814696bb866e45",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_dynamic_api_n0": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 45978,
            "sides": [
              {
                "calibrated_total_ns": 28819010.4,
                "ns_per_operation": 626.8,
                "observed_batch": 45978,
                "requested_batch": 45978,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 37184293.698,
                "ns_per_operation": 808.741,
                "observed_batch": 45978,
                "requested_batch": 45978,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 28819010.4,
            "ns_per_operation": 626.8,
            "observed_batch": 45978,
            "requested_batch": 45978,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 37184293.698,
            "ns_per_operation": 808.741,
            "observed_batch": 45978,
            "requested_batch": 45978,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 21975573.132,
            "ns_per_operation": 486.756,
            "observed_batch": 45147,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22524576.222,
            "ns_per_operation": 489.899,
            "observed_batch": 45978,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 45978,
        "source_sha256": "f02cc93a4a43691ce5be8782fa2df454fc2468a7935117ea18104a7dce22cb2d",
        "wasm_sha256": "c493d0aae68d0c5aa67fc0b562e616c45eb3d42b0302951158a763fa202ce589",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_dynamic_api_n1": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 42373,
            "sides": [
              {
                "calibrated_total_ns": 17681829.17,
                "ns_per_operation": 417.29,
                "observed_batch": 42373,
                "requested_batch": 42373,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 16403223.895,
                "ns_per_operation": 387.115,
                "observed_batch": 42373,
                "requested_batch": 42373,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 56831,
            "sides": [
              {
                "calibrated_total_ns": 21129367.983,
                "ns_per_operation": 371.793,
                "observed_batch": 56831,
                "requested_batch": 56831,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 22950460.547,
                "ns_per_operation": 403.837,
                "observed_batch": 56831,
                "requested_batch": 56831,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 21129367.983,
            "ns_per_operation": 371.793,
            "observed_batch": 56831,
            "requested_batch": 56831,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22950460.547,
            "ns_per_operation": 403.837,
            "observed_batch": 56831,
            "requested_batch": 56831,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 14495706.302,
            "ns_per_operation": 344.267,
            "observed_batch": 42106,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 17253946.616,
            "ns_per_operation": 407.192,
            "observed_batch": 42373,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 56831,
        "source_sha256": "f02cc93a4a43691ce5be8782fa2df454fc2468a7935117ea18104a7dce22cb2d",
        "wasm_sha256": "c493d0aae68d0c5aa67fc0b562e616c45eb3d42b0302951158a763fa202ce589",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_dynamic_api_n10000": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 43764,
            "sides": [
              {
                "calibrated_total_ns": 14697395.412,
                "ns_per_operation": 335.833,
                "observed_batch": 43764,
                "requested_batch": 43764,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 21768913.824,
                "ns_per_operation": 497.416,
                "observed_batch": 43764,
                "requested_batch": 43764,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 65509,
            "sides": [
              {
                "calibrated_total_ns": 20953250.177,
                "ns_per_operation": 319.853,
                "observed_batch": 65509,
                "requested_batch": 65509,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 26408643.169999998,
                "ns_per_operation": 403.13,
                "observed_batch": 65509,
                "requested_batch": 65509,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 20953250.177,
            "ns_per_operation": 319.853,
            "observed_batch": 65509,
            "requested_batch": 65509,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 26408643.169999998,
            "ns_per_operation": 403.13,
            "observed_batch": 65509,
            "requested_batch": 65509,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 15508648.68,
            "ns_per_operation": 354.37,
            "observed_batch": 43764,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 7789832.19,
            "ns_per_operation": 702.99,
            "observed_batch": 11081,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 65509,
        "source_sha256": "f02cc93a4a43691ce5be8782fa2df454fc2468a7935117ea18104a7dce22cb2d",
        "wasm_sha256": "c493d0aae68d0c5aa67fc0b562e616c45eb3d42b0302951158a763fa202ce589",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_dynamic_api_n1024": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 48781,
            "sides": [
              {
                "calibrated_total_ns": 17095984.384,
                "ns_per_operation": 350.464,
                "observed_batch": 48781,
                "requested_batch": 48781,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 19704889.826,
                "ns_per_operation": 403.946,
                "observed_batch": 48781,
                "requested_batch": 48781,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 62774,
            "sides": [
              {
                "calibrated_total_ns": 29329017.184,
                "ns_per_operation": 467.216,
                "observed_batch": 62774,
                "requested_batch": 62774,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 27022386.554,
                "ns_per_operation": 430.471,
                "observed_batch": 62774,
                "requested_batch": 62774,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 29329017.184,
            "ns_per_operation": 467.216,
            "observed_batch": 62774,
            "requested_batch": 62774,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27022386.554,
            "ns_per_operation": 430.471,
            "observed_batch": 62774,
            "requested_batch": 62774,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 18647561.651,
            "ns_per_operation": 382.271,
            "observed_batch": 48781,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 16485593.769,
            "ns_per_operation": 360.207,
            "observed_batch": 45767,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 62774,
        "source_sha256": "f02cc93a4a43691ce5be8782fa2df454fc2468a7935117ea18104a7dce22cb2d",
        "wasm_sha256": "c493d0aae68d0c5aa67fc0b562e616c45eb3d42b0302951158a763fa202ce589",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_dynamic_api_n17": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 41667,
            "sides": [
              {
                "calibrated_total_ns": 17063428.173,
                "ns_per_operation": 409.519,
                "observed_batch": 41667,
                "requested_batch": 41667,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 18715108.053,
                "ns_per_operation": 449.159,
                "observed_batch": 41667,
                "requested_batch": 41667,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 53722,
            "sides": [
              {
                "calibrated_total_ns": 24016258.934,
                "ns_per_operation": 447.047,
                "observed_batch": 53722,
                "requested_batch": 53722,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 25806222.252,
                "ns_per_operation": 480.366,
                "observed_batch": 53722,
                "requested_batch": 53722,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 24016258.934,
            "ns_per_operation": 447.047,
            "observed_batch": 53722,
            "requested_batch": 53722,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 25806222.252,
            "ns_per_operation": 480.366,
            "observed_batch": 53722,
            "requested_batch": 53722,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 15502249.016999999,
            "ns_per_operation": 372.051,
            "observed_batch": 41667,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 18876029.466,
            "ns_per_operation": 503.979,
            "observed_batch": 37454,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 53722,
        "source_sha256": "f02cc93a4a43691ce5be8782fa2df454fc2468a7935117ea18104a7dce22cb2d",
        "wasm_sha256": "c493d0aae68d0c5aa67fc0b562e616c45eb3d42b0302951158a763fa202ce589",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_dynamic_api_n32": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 55249,
            "sides": [
              {
                "calibrated_total_ns": 19473836.026,
                "ns_per_operation": 352.474,
                "observed_batch": 55249,
                "requested_batch": 55249,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 20072016.949,
                "ns_per_operation": 363.301,
                "observed_batch": 55249,
                "requested_batch": 55249,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 62416,
            "sides": [
              {
                "calibrated_total_ns": 21124008.624,
                "ns_per_operation": 338.439,
                "observed_batch": 62416,
                "requested_batch": 62416,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 23106652.864,
                "ns_per_operation": 370.204,
                "observed_batch": 62416,
                "requested_batch": 62416,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 21124008.624,
            "ns_per_operation": 338.439,
            "observed_batch": 62416,
            "requested_batch": 62416,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 23106652.864,
            "ns_per_operation": 370.204,
            "observed_batch": 62416,
            "requested_batch": 62416,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 6243583.157,
            "ns_per_operation": 331.207,
            "observed_batch": 18851,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 19677483.84,
            "ns_per_operation": 356.16,
            "observed_batch": 55249,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 62416,
        "source_sha256": "f02cc93a4a43691ce5be8782fa2df454fc2468a7935117ea18104a7dce22cb2d",
        "wasm_sha256": "c493d0aae68d0c5aa67fc0b562e616c45eb3d42b0302951158a763fa202ce589",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_host_call_n0": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 28051,
            "sides": [
              {
                "calibrated_total_ns": 23831680.783999998,
                "ns_per_operation": 849.584,
                "observed_batch": 28051,
                "requested_batch": 28051,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 21507487.128,
                "ns_per_operation": 766.728,
                "observed_batch": 28051,
                "requested_batch": 28051,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 23831680.783999998,
            "ns_per_operation": 849.584,
            "observed_batch": 28051,
            "requested_batch": 28051,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 21507487.128,
            "ns_per_operation": 766.728,
            "observed_batch": 28051,
            "requested_batch": 28051,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 21255869.658,
            "ns_per_operation": 757.758,
            "observed_batch": 28051,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 18620254.224,
            "ns_per_operation": 707.564,
            "observed_batch": 26316,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 28051,
        "source_sha256": "4c4026de620eff0b1e9cb4ed4e0440e6045d921267e3962715f6430ac68f5120",
        "wasm_sha256": "c60311d57c929c3ff1d29adb70501e7bbf8419517acbd86c8d072438b610ebb7",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_host_call_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 13343,
            "sides": [
              {
                "calibrated_total_ns": 22563987.039,
                "ns_per_operation": 1691.073,
                "observed_batch": 13343,
                "requested_batch": 13343,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 26796440.011,
                "ns_per_operation": 2008.277,
                "observed_batch": 13343,
                "requested_batch": 13343,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 22563987.039,
            "ns_per_operation": 1691.073,
            "observed_batch": 13343,
            "requested_batch": 13343,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 26796440.011,
            "ns_per_operation": 2008.277,
            "observed_batch": 13343,
            "requested_batch": 13343,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 22944529.399,
            "ns_per_operation": 1719.593,
            "observed_batch": 13343,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 12625172.392,
            "ns_per_operation": 1663.834,
            "observed_batch": 7588,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 13343,
        "source_sha256": "4c4026de620eff0b1e9cb4ed4e0440e6045d921267e3962715f6430ac68f5120",
        "wasm_sha256": "c60311d57c929c3ff1d29adb70501e7bbf8419517acbd86c8d072438b610ebb7",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_host_call_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 3,
            "sides": [
              {
                "calibrated_total_ns": 34440063.0,
                "ns_per_operation": 11480021.0,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 27342119.001,
                "ns_per_operation": 9114039.667,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 34440063.0,
            "ns_per_operation": 11480021.0,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27342119.001,
            "ns_per_operation": 9114039.667,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 38085194.001,
            "ns_per_operation": 12695064.667,
            "observed_batch": 3,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 31999497.999,
            "ns_per_operation": 10666499.333,
            "observed_batch": 3,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 3,
        "source_sha256": "4c4026de620eff0b1e9cb4ed4e0440e6045d921267e3962715f6430ac68f5120",
        "wasm_sha256": "c60311d57c929c3ff1d29adb70501e7bbf8419517acbd86c8d072438b610ebb7",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_host_call_n1024": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 20,
            "sides": [
              {
                "calibrated_total_ns": 21030418.0,
                "ns_per_operation": 1051520.9,
                "observed_batch": 20,
                "requested_batch": 20,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 29669572.0,
                "ns_per_operation": 1483478.6,
                "observed_batch": 20,
                "requested_batch": 20,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 21030418.0,
            "ns_per_operation": 1051520.9,
            "observed_batch": 20,
            "requested_batch": 20,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 29669572.0,
            "ns_per_operation": 1483478.6,
            "observed_batch": 20,
            "requested_batch": 20,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 25304158.0,
            "ns_per_operation": 1265207.9,
            "observed_batch": 20,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 19420961.993,
            "ns_per_operation": 1142409.529,
            "observed_batch": 17,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 20,
        "source_sha256": "4c4026de620eff0b1e9cb4ed4e0440e6045d921267e3962715f6430ac68f5120",
        "wasm_sha256": "c60311d57c929c3ff1d29adb70501e7bbf8419517acbd86c8d072438b610ebb7",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_host_call_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 2163,
            "sides": [
              {
                "calibrated_total_ns": 37138294.704,
                "ns_per_operation": 17169.808,
                "observed_batch": 2163,
                "requested_batch": 2163,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 30596262.270000003,
                "ns_per_operation": 14145.29,
                "observed_batch": 2163,
                "requested_batch": 2163,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 37138294.704,
            "ns_per_operation": 17169.808,
            "observed_batch": 2163,
            "requested_batch": 2163,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 30596262.270000003,
            "ns_per_operation": 14145.29,
            "observed_batch": 2163,
            "requested_batch": 2163,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 31276841.349,
            "ns_per_operation": 14957.839,
            "observed_batch": 2091,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 29730919.512000002,
            "ns_per_operation": 13745.224,
            "observed_batch": 2163,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 2163,
        "source_sha256": "4c4026de620eff0b1e9cb4ed4e0440e6045d921267e3962715f6430ac68f5120",
        "wasm_sha256": "c60311d57c929c3ff1d29adb70501e7bbf8419517acbd86c8d072438b610ebb7",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_host_call_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1153,
            "sides": [
              {
                "calibrated_total_ns": 41112966.058,
                "ns_per_operation": 35657.386,
                "observed_batch": 1153,
                "requested_batch": 1153,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 31908314.551000003,
                "ns_per_operation": 27674.167,
                "observed_batch": 1153,
                "requested_batch": 1153,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 41112966.058,
            "ns_per_operation": 35657.386,
            "observed_batch": 1153,
            "requested_batch": 1153,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 31908314.551000003,
            "ns_per_operation": 27674.167,
            "observed_batch": 1153,
            "requested_batch": 1153,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 36147445.881,
            "ns_per_operation": 31350.777,
            "observed_batch": 1153,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 26876954.64,
            "ns_per_operation": 29470.345,
            "observed_batch": 912,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1153,
        "source_sha256": "4c4026de620eff0b1e9cb4ed4e0440e6045d921267e3962715f6430ac68f5120",
        "wasm_sha256": "c60311d57c929c3ff1d29adb70501e7bbf8419517acbd86c8d072438b610ebb7",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_indirect_calls_n0": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 28944,
            "sides": [
              {
                "calibrated_total_ns": 28603879.056,
                "ns_per_operation": 988.249,
                "observed_batch": 28944,
                "requested_batch": 28944,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 25662705.552,
                "ns_per_operation": 886.633,
                "observed_batch": 28944,
                "requested_batch": 28944,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 28603879.056,
            "ns_per_operation": 988.249,
            "observed_batch": 28944,
            "requested_batch": 28944,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 25662705.552,
            "ns_per_operation": 886.633,
            "observed_batch": 28944,
            "requested_batch": 28944,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 19170511.5,
            "ns_per_operation": 784.07,
            "observed_batch": 24450,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 29481663.744,
            "ns_per_operation": 1018.576,
            "observed_batch": 28944,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 28944,
        "source_sha256": "29024ae23844664a9fea4a809d1ce414de40f243f7e678ce5ea6d811843f7797",
        "wasm_sha256": "4b42e82f54ea6ab7d0f62c70766c904e41d6a408e57fa14b5abbf514c0da78a9",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_indirect_calls_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 14016,
            "sides": [
              {
                "calibrated_total_ns": 30434146.176,
                "ns_per_operation": 2171.386,
                "observed_batch": 14016,
                "requested_batch": 14016,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 29428203.839999996,
                "ns_per_operation": 2099.615,
                "observed_batch": 14016,
                "requested_batch": 14016,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 30434146.176,
            "ns_per_operation": 2171.386,
            "observed_batch": 14016,
            "requested_batch": 14016,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 29428203.839999996,
            "ns_per_operation": 2099.615,
            "observed_batch": 14016,
            "requested_batch": 14016,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 27460498.190999996,
            "ns_per_operation": 2063.613,
            "observed_batch": 13307,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 38766812.352,
            "ns_per_operation": 2765.897,
            "observed_batch": 14016,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 14016,
        "source_sha256": "29024ae23844664a9fea4a809d1ce414de40f243f7e678ce5ea6d811843f7797",
        "wasm_sha256": "4b42e82f54ea6ab7d0f62c70766c904e41d6a408e57fa14b5abbf514c0da78a9",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_indirect_calls_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 2,
            "sides": [
              {
                "calibrated_total_ns": 27760487.0,
                "ns_per_operation": 13880243.5,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 25551444.0,
                "ns_per_operation": 12775722.0,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 27760487.0,
            "ns_per_operation": 13880243.5,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 25551444.0,
            "ns_per_operation": 12775722.0,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 26362705.0,
            "ns_per_operation": 13181352.5,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 25586525.0,
            "ns_per_operation": 12793262.5,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 2,
        "source_sha256": "29024ae23844664a9fea4a809d1ce414de40f243f7e678ce5ea6d811843f7797",
        "wasm_sha256": "4b42e82f54ea6ab7d0f62c70766c904e41d6a408e57fa14b5abbf514c0da78a9",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_indirect_calls_n1024": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 19,
            "sides": [
              {
                "calibrated_total_ns": 20813969.007000003,
                "ns_per_operation": 1095472.053,
                "observed_batch": 19,
                "requested_batch": 19,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 19682397.008,
                "ns_per_operation": 1035915.632,
                "observed_batch": 19,
                "requested_batch": 19,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 22,
            "sides": [
              {
                "calibrated_total_ns": 24060194.994,
                "ns_per_operation": 1093645.227,
                "observed_batch": 22,
                "requested_batch": 22,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 28337335.004,
                "ns_per_operation": 1288060.682,
                "observed_batch": 22,
                "requested_batch": 22,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 24060194.994,
            "ns_per_operation": 1093645.227,
            "observed_batch": 22,
            "requested_batch": 22,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 28337335.004,
            "ns_per_operation": 1288060.682,
            "observed_batch": 22,
            "requested_batch": 22,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 21490171.996,
            "ns_per_operation": 1131061.684,
            "observed_batch": 19,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 15346902.0,
            "ns_per_operation": 1023126.8,
            "observed_batch": 15,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 22,
        "source_sha256": "29024ae23844664a9fea4a809d1ce414de40f243f7e678ce5ea6d811843f7797",
        "wasm_sha256": "4b42e82f54ea6ab7d0f62c70766c904e41d6a408e57fa14b5abbf514c0da78a9",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_indirect_calls_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1640,
            "sides": [
              {
                "calibrated_total_ns": 40051397.76,
                "ns_per_operation": 24421.584,
                "observed_batch": 1640,
                "requested_batch": 1640,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 33503922.44,
                "ns_per_operation": 20429.221,
                "observed_batch": 1640,
                "requested_batch": 1640,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 40051397.76,
            "ns_per_operation": 24421.584,
            "observed_batch": 1640,
            "requested_batch": 1640,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 33503922.44,
            "ns_per_operation": 20429.221,
            "observed_batch": 1640,
            "requested_batch": 1640,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 34976982.208,
            "ns_per_operation": 23071.888,
            "observed_batch": 1516,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 32500750.84,
            "ns_per_operation": 19817.531,
            "observed_batch": 1640,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1640,
        "source_sha256": "29024ae23844664a9fea4a809d1ce414de40f243f7e678ce5ea6d811843f7797",
        "wasm_sha256": "4b42e82f54ea6ab7d0f62c70766c904e41d6a408e57fa14b5abbf514c0da78a9",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_indirect_calls_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 852,
            "sides": [
              {
                "calibrated_total_ns": 38428836.335999995,
                "ns_per_operation": 45104.268,
                "observed_batch": 852,
                "requested_batch": 852,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 31847907.396,
                "ns_per_operation": 37380.173,
                "observed_batch": 852,
                "requested_batch": 852,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 38428836.335999995,
            "ns_per_operation": 45104.268,
            "observed_batch": 852,
            "requested_batch": 852,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 31847907.396,
            "ns_per_operation": 37380.173,
            "observed_batch": 852,
            "requested_batch": 852,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 31226281.380000003,
            "ns_per_operation": 36650.565,
            "observed_batch": 852,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 49952863.59,
            "ns_per_operation": 60184.173,
            "observed_batch": 830,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 852,
        "source_sha256": "29024ae23844664a9fea4a809d1ce414de40f243f7e678ce5ea6d811843f7797",
        "wasm_sha256": "4b42e82f54ea6ab7d0f62c70766c904e41d6a408e57fa14b5abbf514c0da78a9",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_multivalue_branch_n0": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 20555,
            "sides": [
              {
                "calibrated_total_ns": 15630803.09,
                "ns_per_operation": 760.438,
                "observed_batch": 20555,
                "requested_batch": 20555,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 19756726.02,
                "ns_per_operation": 961.164,
                "observed_batch": 20555,
                "requested_batch": 20555,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 28931,
            "sides": [
              {
                "calibrated_total_ns": 26965225.343,
                "ns_per_operation": 932.053,
                "observed_batch": 28931,
                "requested_batch": 28931,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 27235759.124,
                "ns_per_operation": 941.404,
                "observed_batch": 28931,
                "requested_batch": 28931,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 26965225.343,
            "ns_per_operation": 932.053,
            "observed_batch": 28931,
            "requested_batch": 28931,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27235759.124,
            "ns_per_operation": 941.404,
            "observed_batch": 28931,
            "requested_batch": 28931,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 15858388.049999999,
            "ns_per_operation": 771.51,
            "observed_batch": 20555,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 39043.968,
            "ns_per_operation": 610.062,
            "observed_batch": 64,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 28931,
        "source_sha256": "a1f4958100af82889208bb28090d6af7c3e56cbeeccabad4106601c9a9c17014",
        "wasm_sha256": "03ffdc2041ae8ba0c8d078422d36152cf1d35f1293c94a4074fe7e83791743ae",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_multivalue_branch_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 12508,
            "sides": [
              {
                "calibrated_total_ns": 28452047.664,
                "ns_per_operation": 2274.708,
                "observed_batch": 12508,
                "requested_batch": 12508,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 25918252.072,
                "ns_per_operation": 2072.134,
                "observed_batch": 12508,
                "requested_batch": 12508,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 28452047.664,
            "ns_per_operation": 2274.708,
            "observed_batch": 12508,
            "requested_batch": 12508,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 25918252.072,
            "ns_per_operation": 2072.134,
            "observed_batch": 12508,
            "requested_batch": 12508,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 23739585.696000002,
            "ns_per_operation": 2070.072,
            "observed_batch": 11468,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27746496.400000002,
            "ns_per_operation": 2218.3,
            "observed_batch": 12508,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 12508,
        "source_sha256": "a1f4958100af82889208bb28090d6af7c3e56cbeeccabad4106601c9a9c17014",
        "wasm_sha256": "03ffdc2041ae8ba0c8d078422d36152cf1d35f1293c94a4074fe7e83791743ae",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_multivalue_branch_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 2,
            "sides": [
              {
                "calibrated_total_ns": 33021952.0,
                "ns_per_operation": 16510976.0,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 34436279.0,
                "ns_per_operation": 17218139.5,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 33021952.0,
            "ns_per_operation": 16510976.0,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 34436279.0,
            "ns_per_operation": 17218139.5,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 38014569.0,
            "ns_per_operation": 19007284.5,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 28014407.0,
            "ns_per_operation": 14007203.5,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 2,
        "source_sha256": "a1f4958100af82889208bb28090d6af7c3e56cbeeccabad4106601c9a9c17014",
        "wasm_sha256": "03ffdc2041ae8ba0c8d078422d36152cf1d35f1293c94a4074fe7e83791743ae",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_multivalue_branch_n1024": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 17,
            "sides": [
              {
                "calibrated_total_ns": 43429810.002,
                "ns_per_operation": 2554694.706,
                "observed_batch": 17,
                "requested_batch": 17,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 29515589.006,
                "ns_per_operation": 1736211.118,
                "observed_batch": 17,
                "requested_batch": 17,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 43429810.002,
            "ns_per_operation": 2554694.706,
            "observed_batch": 17,
            "requested_batch": 17,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 29515589.006,
            "ns_per_operation": 1736211.118,
            "observed_batch": 17,
            "requested_batch": 17,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 28529674.992,
            "ns_per_operation": 1678216.176,
            "observed_batch": 17,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 21138229.995,
            "ns_per_operation": 1409215.333,
            "observed_batch": 15,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 17,
        "source_sha256": "a1f4958100af82889208bb28090d6af7c3e56cbeeccabad4106601c9a9c17014",
        "wasm_sha256": "03ffdc2041ae8ba0c8d078422d36152cf1d35f1293c94a4074fe7e83791743ae",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_multivalue_branch_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1233,
            "sides": [
              {
                "calibrated_total_ns": 31949893.026,
                "ns_per_operation": 25912.322,
                "observed_batch": 1233,
                "requested_batch": 1233,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 31968209.241,
                "ns_per_operation": 25927.177,
                "observed_batch": 1233,
                "requested_batch": 1233,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 31949893.026,
            "ns_per_operation": 25912.322,
            "observed_batch": 1233,
            "requested_batch": 1233,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 31968209.241,
            "ns_per_operation": 25927.177,
            "observed_batch": 1233,
            "requested_batch": 1233,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 31074117.984,
            "ns_per_operation": 25981.704,
            "observed_batch": 1196,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 32090263.911000002,
            "ns_per_operation": 26026.167,
            "observed_batch": 1233,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1233,
        "source_sha256": "a1f4958100af82889208bb28090d6af7c3e56cbeeccabad4106601c9a9c17014",
        "wasm_sha256": "03ffdc2041ae8ba0c8d078422d36152cf1d35f1293c94a4074fe7e83791743ae",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_multivalue_branch_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 670,
            "sides": [
              {
                "calibrated_total_ns": 37455615.01,
                "ns_per_operation": 55903.903,
                "observed_batch": 670,
                "requested_batch": 670,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 32410628.24,
                "ns_per_operation": 48374.072,
                "observed_batch": 670,
                "requested_batch": 670,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 37455615.01,
            "ns_per_operation": 55903.903,
            "observed_batch": 670,
            "requested_batch": 670,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 32410628.24,
            "ns_per_operation": 48374.072,
            "observed_batch": 670,
            "requested_batch": 670,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 35050355.21,
            "ns_per_operation": 52313.963,
            "observed_batch": 670,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 47862753.89,
            "ns_per_operation": 71974.066,
            "observed_batch": 665,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 670,
        "source_sha256": "a1f4958100af82889208bb28090d6af7c3e56cbeeccabad4106601c9a9c17014",
        "wasm_sha256": "03ffdc2041ae8ba0c8d078422d36152cf1d35f1293c94a4074fe7e83791743ae",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_ordinary_recursion_n0": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 23365,
            "sides": [
              {
                "calibrated_total_ns": 26527149.005,
                "ns_per_operation": 1135.337,
                "observed_batch": 23365,
                "requested_batch": 23365,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 27190761.735,
                "ns_per_operation": 1163.739,
                "observed_batch": 23365,
                "requested_batch": 23365,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 26527149.005,
            "ns_per_operation": 1135.337,
            "observed_batch": 23365,
            "requested_batch": 23365,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27190761.735,
            "ns_per_operation": 1163.739,
            "observed_batch": 23365,
            "requested_batch": 23365,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 5644779.5879999995,
            "ns_per_operation": 1003.338,
            "observed_batch": 5626,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22251587.655,
            "ns_per_operation": 952.347,
            "observed_batch": 23365,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 23365,
        "source_sha256": "aca6a66637fa36b72f3c528dd5e3e44039b573f363b5b4fb77437e9278623903",
        "wasm_sha256": "e1bd7ea6ada8b15b2770ac387f8fc2256f20d7f87c4bc34e7559b239d8875ba1",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_ordinary_recursion_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 15480,
            "sides": [
              {
                "calibrated_total_ns": 28951872.48,
                "ns_per_operation": 1870.276,
                "observed_batch": 15480,
                "requested_batch": 15480,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 25461411.12,
                "ns_per_operation": 1644.794,
                "observed_batch": 15480,
                "requested_batch": 15480,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 28951872.48,
            "ns_per_operation": 1870.276,
            "observed_batch": 15480,
            "requested_batch": 15480,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 25461411.12,
            "ns_per_operation": 1644.794,
            "observed_batch": 15480,
            "requested_batch": 15480,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 28256479.92,
            "ns_per_operation": 1825.354,
            "observed_batch": 15480,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 25913535.555,
            "ns_per_operation": 1724.465,
            "observed_batch": 15027,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 15480,
        "source_sha256": "aca6a66637fa36b72f3c528dd5e3e44039b573f363b5b4fb77437e9278623903",
        "wasm_sha256": "e1bd7ea6ada8b15b2770ac387f8fc2256f20d7f87c4bc34e7559b239d8875ba1",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_ordinary_recursion_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1971,
            "sides": [
              {
                "calibrated_total_ns": 33424424.955,
                "ns_per_operation": 16958.105,
                "observed_batch": 1971,
                "requested_batch": 1971,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 31947976.449,
                "ns_per_operation": 16209.019,
                "observed_batch": 1971,
                "requested_batch": 1971,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 33424424.955,
            "ns_per_operation": 16958.105,
            "observed_batch": 1971,
            "requested_batch": 1971,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 31947976.449,
            "ns_per_operation": 16209.019,
            "observed_batch": 1971,
            "requested_batch": 1971,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 32971047.334999997,
            "ns_per_operation": 18894.583,
            "observed_batch": 1745,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 30248028.369000003,
            "ns_per_operation": 15346.539,
            "observed_batch": 1971,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1971,
        "source_sha256": "aca6a66637fa36b72f3c528dd5e3e44039b573f363b5b4fb77437e9278623903",
        "wasm_sha256": "e1bd7ea6ada8b15b2770ac387f8fc2256f20d7f87c4bc34e7559b239d8875ba1",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_ordinary_recursion_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 895,
            "sides": [
              {
                "calibrated_total_ns": 35415186.695,
                "ns_per_operation": 39570.041,
                "observed_batch": 895,
                "requested_batch": 895,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 27516953.105,
                "ns_per_operation": 30745.199,
                "observed_batch": 895,
                "requested_batch": 895,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 35415186.695,
            "ns_per_operation": 39570.041,
            "observed_batch": 895,
            "requested_batch": 895,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27516953.105,
            "ns_per_operation": 30745.199,
            "observed_batch": 895,
            "requested_batch": 895,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 27763440.58,
            "ns_per_operation": 31020.604,
            "observed_batch": 895,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 1660743.007,
            "ns_per_operation": 21568.091,
            "observed_batch": 77,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 895,
        "source_sha256": "aca6a66637fa36b72f3c528dd5e3e44039b573f363b5b4fb77437e9278623903",
        "wasm_sha256": "e1bd7ea6ada8b15b2770ac387f8fc2256f20d7f87c4bc34e7559b239d8875ba1",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_table_mutation_n0": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 22051,
            "sides": [
              {
                "calibrated_total_ns": 19212176.311,
                "ns_per_operation": 871.261,
                "observed_batch": 22051,
                "requested_batch": 22051,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 17879171.31,
                "ns_per_operation": 810.81,
                "observed_batch": 22051,
                "requested_batch": 22051,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 27134,
            "sides": [
              {
                "calibrated_total_ns": 22386336.886,
                "ns_per_operation": 825.029,
                "observed_batch": 27134,
                "requested_batch": 27134,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 24370402.099999998,
                "ns_per_operation": 898.15,
                "observed_batch": 27134,
                "requested_batch": 27134,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 22386336.886,
            "ns_per_operation": 825.029,
            "observed_batch": 27134,
            "requested_batch": 27134,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 24370402.099999998,
            "ns_per_operation": 898.15,
            "observed_batch": 27134,
            "requested_batch": 27134,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 20756209.382,
            "ns_per_operation": 941.282,
            "observed_batch": 22051,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 12695688.192,
            "ns_per_operation": 884.224,
            "observed_batch": 14358,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 27134,
        "source_sha256": "9b61bd9cbca804bf639085769fd766a2cf8a69d516fc7efb9e4edfe4ff6d18c9",
        "wasm_sha256": "458f2b111e48d2efca98083b8c56860c43e82be8752813d1e2814d23c28d4f43",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_table_mutation_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 14870,
            "sides": [
              {
                "calibrated_total_ns": 30249014.97,
                "ns_per_operation": 2034.231,
                "observed_batch": 14870,
                "requested_batch": 14870,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 31717576.169999998,
                "ns_per_operation": 2132.991,
                "observed_batch": 14870,
                "requested_batch": 14870,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 30249014.97,
            "ns_per_operation": 2034.231,
            "observed_batch": 14870,
            "requested_batch": 14870,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 31717576.169999998,
            "ns_per_operation": 2132.991,
            "observed_batch": 14870,
            "requested_batch": 14870,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 34111028.864999995,
            "ns_per_operation": 2416.651,
            "observed_batch": 14115,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 33086701.679999996,
            "ns_per_operation": 2225.064,
            "observed_batch": 14870,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 14870,
        "source_sha256": "9b61bd9cbca804bf639085769fd766a2cf8a69d516fc7efb9e4edfe4ff6d18c9",
        "wasm_sha256": "458f2b111e48d2efca98083b8c56860c43e82be8752813d1e2814d23c28d4f43",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_table_mutation_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 2,
            "sides": [
              {
                "calibrated_total_ns": 36341164.0,
                "ns_per_operation": 18170582.0,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 26333522.0,
                "ns_per_operation": 13166761.0,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 36341164.0,
            "ns_per_operation": 18170582.0,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 26333522.0,
            "ns_per_operation": 13166761.0,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 28228393.0,
            "ns_per_operation": 14114196.5,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 25838837.0,
            "ns_per_operation": 12919418.5,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 2,
        "source_sha256": "9b61bd9cbca804bf639085769fd766a2cf8a69d516fc7efb9e4edfe4ff6d18c9",
        "wasm_sha256": "458f2b111e48d2efca98083b8c56860c43e82be8752813d1e2814d23c28d4f43",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_table_mutation_n1024": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 22,
            "sides": [
              {
                "calibrated_total_ns": 25607247.006,
                "ns_per_operation": 1163965.773,
                "observed_batch": 22,
                "requested_batch": 22,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 26199659.002,
                "ns_per_operation": 1190893.591,
                "observed_batch": 22,
                "requested_batch": 22,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 25607247.006,
            "ns_per_operation": 1163965.773,
            "observed_batch": 22,
            "requested_batch": 22,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 26199659.002,
            "ns_per_operation": 1190893.591,
            "observed_batch": 22,
            "requested_batch": 22,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 23253410.994,
            "ns_per_operation": 1056973.227,
            "observed_batch": 22,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 17619968.002,
            "ns_per_operation": 1258569.143,
            "observed_batch": 14,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 22,
        "source_sha256": "9b61bd9cbca804bf639085769fd766a2cf8a69d516fc7efb9e4edfe4ff6d18c9",
        "wasm_sha256": "458f2b111e48d2efca98083b8c56860c43e82be8752813d1e2814d23c28d4f43",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_table_mutation_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1711,
            "sides": [
              {
                "calibrated_total_ns": 36103831.532,
                "ns_per_operation": 21101.012,
                "observed_batch": 1711,
                "requested_batch": 1711,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 32984095.080999997,
                "ns_per_operation": 19277.671,
                "observed_batch": 1711,
                "requested_batch": 1711,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 36103831.532,
            "ns_per_operation": 21101.012,
            "observed_batch": 1711,
            "requested_batch": 1711,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 32984095.080999997,
            "ns_per_operation": 19277.671,
            "observed_batch": 1711,
            "requested_batch": 1711,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 33655368.289,
            "ns_per_operation": 19669.999,
            "observed_batch": 1711,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 5804440.007999999,
            "ns_per_operation": 18252.956,
            "observed_batch": 318,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1711,
        "source_sha256": "9b61bd9cbca804bf639085769fd766a2cf8a69d516fc7efb9e4edfe4ff6d18c9",
        "wasm_sha256": "458f2b111e48d2efca98083b8c56860c43e82be8752813d1e2814d23c28d4f43",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_table_mutation_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 961,
            "sides": [
              {
                "calibrated_total_ns": 32987827.043,
                "ns_per_operation": 34326.563,
                "observed_batch": 961,
                "requested_batch": 961,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 38722134.224,
                "ns_per_operation": 40293.584,
                "observed_batch": 961,
                "requested_batch": 961,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 32987827.043,
            "ns_per_operation": 34326.563,
            "observed_batch": 961,
            "requested_batch": 961,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 38722134.224,
            "ns_per_operation": 40293.584,
            "observed_batch": 961,
            "requested_batch": 961,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 10985122.115,
            "ns_per_operation": 41453.291,
            "observed_batch": 265,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 33734451.967999995,
            "ns_per_operation": 35103.488,
            "observed_batch": 961,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 961,
        "source_sha256": "9b61bd9cbca804bf639085769fd766a2cf8a69d516fc7efb9e4edfe4ff6d18c9",
        "wasm_sha256": "458f2b111e48d2efca98083b8c56860c43e82be8752813d1e2814d23c28d4f43",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_tail_calls_n0": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 23924,
            "sides": [
              {
                "calibrated_total_ns": 19729692.168,
                "ns_per_operation": 824.682,
                "observed_batch": 23924,
                "requested_batch": 23924,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 19296308.908,
                "ns_per_operation": 806.567,
                "observed_batch": 23924,
                "requested_batch": 23924,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 27277,
            "sides": [
              {
                "calibrated_total_ns": 25602655.908999998,
                "ns_per_operation": 938.617,
                "observed_batch": 27277,
                "requested_batch": 27277,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 32572747.718999997,
                "ns_per_operation": 1194.147,
                "observed_batch": 27277,
                "requested_batch": 27277,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 25602655.908999998,
            "ns_per_operation": 938.617,
            "observed_batch": 27277,
            "requested_batch": 27277,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 32572747.718999997,
            "ns_per_operation": 1194.147,
            "observed_batch": 27277,
            "requested_batch": 27277,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 7599442.032,
            "ns_per_operation": 998.088,
            "observed_batch": 7614,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 20126567.404,
            "ns_per_operation": 841.271,
            "observed_batch": 23924,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 27277,
        "source_sha256": "9953f19384c51b579921e6f789b13a8b57c86ba86d760b77846e3e33825ece02",
        "wasm_sha256": "3e23be294e7a9eda62c89dbc366b6a6cab90edfbda532da27a7e4fc6d0aa4a56",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_tail_calls_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 14971,
            "sides": [
              {
                "calibrated_total_ns": 26226587.046,
                "ns_per_operation": 1751.826,
                "observed_batch": 14971,
                "requested_batch": 14971,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 29900290.794,
                "ns_per_operation": 1997.214,
                "observed_batch": 14971,
                "requested_batch": 14971,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 26226587.046,
            "ns_per_operation": 1751.826,
            "observed_batch": 14971,
            "requested_batch": 14971,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 29900290.794,
            "ns_per_operation": 1997.214,
            "observed_batch": 14971,
            "requested_batch": 14971,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 13704669.6,
            "ns_per_operation": 1595.05,
            "observed_batch": 8592,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 26792730.382,
            "ns_per_operation": 1789.642,
            "observed_batch": 14971,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 14971,
        "source_sha256": "9953f19384c51b579921e6f789b13a8b57c86ba86d760b77846e3e33825ece02",
        "wasm_sha256": "3e23be294e7a9eda62c89dbc366b6a6cab90edfbda532da27a7e4fc6d0aa4a56",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_tail_calls_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 3,
            "sides": [
              {
                "calibrated_total_ns": 23000450.001000002,
                "ns_per_operation": 7666816.667,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 23815931.001000002,
                "ns_per_operation": 7938643.667,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 23000450.001000002,
            "ns_per_operation": 7666816.667,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 23815931.001000002,
            "ns_per_operation": 7938643.667,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 20982264.0,
            "ns_per_operation": 6994088.0,
            "observed_batch": 3,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 28831629.999,
            "ns_per_operation": 9610543.333,
            "observed_batch": 3,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 3,
        "source_sha256": "9953f19384c51b579921e6f789b13a8b57c86ba86d760b77846e3e33825ece02",
        "wasm_sha256": "3e23be294e7a9eda62c89dbc366b6a6cab90edfbda532da27a7e4fc6d0aa4a56",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_tail_calls_n1024": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 37,
            "sides": [
              {
                "calibrated_total_ns": 32070681.993,
                "ns_per_operation": 866775.189,
                "observed_batch": 37,
                "requested_batch": 37,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 28285855.016,
                "ns_per_operation": 764482.568,
                "observed_batch": 37,
                "requested_batch": 37,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 32070681.993,
            "ns_per_operation": 866775.189,
            "observed_batch": 37,
            "requested_batch": 37,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 28285855.016,
            "ns_per_operation": 764482.568,
            "observed_batch": 37,
            "requested_batch": 37,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 25497672.0,
            "ns_per_operation": 1019906.88,
            "observed_batch": 25,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 42738724.013,
            "ns_per_operation": 1155100.649,
            "observed_batch": 37,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 37,
        "source_sha256": "9953f19384c51b579921e6f789b13a8b57c86ba86d760b77846e3e33825ece02",
        "wasm_sha256": "3e23be294e7a9eda62c89dbc366b6a6cab90edfbda532da27a7e4fc6d0aa4a56",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_tail_calls_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 2369,
            "sides": [
              {
                "calibrated_total_ns": 30965632.527000003,
                "ns_per_operation": 13071.183,
                "observed_batch": 2369,
                "requested_batch": 2369,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 28127838.224,
                "ns_per_operation": 11873.296,
                "observed_batch": 2369,
                "requested_batch": 2369,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 30965632.527000003,
            "ns_per_operation": 13071.183,
            "observed_batch": 2369,
            "requested_batch": 2369,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 28127838.224,
            "ns_per_operation": 11873.296,
            "observed_batch": 2369,
            "requested_batch": 2369,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 25104135.138,
            "ns_per_operation": 12769.143,
            "observed_batch": 1966,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 30245887.685,
            "ns_per_operation": 12767.365,
            "observed_batch": 2369,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 2369,
        "source_sha256": "9953f19384c51b579921e6f789b13a8b57c86ba86d760b77846e3e33825ece02",
        "wasm_sha256": "3e23be294e7a9eda62c89dbc366b6a6cab90edfbda532da27a7e4fc6d0aa4a56",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_tail_calls_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1295,
            "sides": [
              {
                "calibrated_total_ns": 40368618.01,
                "ns_per_operation": 31172.678,
                "observed_batch": 1295,
                "requested_batch": 1295,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 48159205.92,
                "ns_per_operation": 37188.576,
                "observed_batch": 1295,
                "requested_batch": 1295,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 40368618.01,
            "ns_per_operation": 31172.678,
            "observed_batch": 1295,
            "requested_batch": 1295,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 48159205.92,
            "ns_per_operation": 37188.576,
            "observed_batch": 1295,
            "requested_batch": 1295,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 29382527.46,
            "ns_per_operation": 24900.447,
            "observed_batch": 1180,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 38931131.75,
            "ns_per_operation": 30062.65,
            "observed_batch": 1295,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1295,
        "source_sha256": "9953f19384c51b579921e6f789b13a8b57c86ba86d760b77846e3e33825ece02",
        "wasm_sha256": "3e23be294e7a9eda62c89dbc366b6a6cab90edfbda532da27a7e4fc6d0aa4a56",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_try_table_normal_n0": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 29851,
            "sides": [
              {
                "calibrated_total_ns": 55997938.665,
                "ns_per_operation": 1875.915,
                "observed_batch": 29851,
                "requested_batch": 29851,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 22094157.948,
                "ns_per_operation": 740.148,
                "observed_batch": 29851,
                "requested_batch": 29851,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 55997938.665,
            "ns_per_operation": 1875.915,
            "observed_batch": 29851,
            "requested_batch": 29851,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22094157.948,
            "ns_per_operation": 740.148,
            "observed_batch": 29851,
            "requested_batch": 29851,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 18156257.262,
            "ns_per_operation": 691.746,
            "observed_batch": 26247,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27447457.182,
            "ns_per_operation": 919.482,
            "observed_batch": 29851,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 29851,
        "source_sha256": "ab92800dd4e4d4053498c317effadedf52568270ab7b476eef74b09638de8964",
        "wasm_sha256": "97068a6310a3bc535369e2b6277b801875c14ca206a707fcb9bca9036f41d0df",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_try_table_normal_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 17362,
            "sides": [
              {
                "calibrated_total_ns": 32198471.394,
                "ns_per_operation": 1854.537,
                "observed_batch": 17362,
                "requested_batch": 17362,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 27020202.808000002,
                "ns_per_operation": 1556.284,
                "observed_batch": 17362,
                "requested_batch": 17362,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 32198471.394,
            "ns_per_operation": 1854.537,
            "observed_batch": 17362,
            "requested_batch": 17362,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27020202.808000002,
            "ns_per_operation": 1556.284,
            "observed_batch": 17362,
            "requested_batch": 17362,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 51344323.111999996,
            "ns_per_operation": 3347.524,
            "observed_batch": 15338,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27207365.168,
            "ns_per_operation": 1567.064,
            "observed_batch": 17362,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 17362,
        "source_sha256": "ab92800dd4e4d4053498c317effadedf52568270ab7b476eef74b09638de8964",
        "wasm_sha256": "97068a6310a3bc535369e2b6277b801875c14ca206a707fcb9bca9036f41d0df",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_try_table_normal_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 2,
            "sides": [
              {
                "calibrated_total_ns": 23299285.0,
                "ns_per_operation": 11649642.5,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 29880629.0,
                "ns_per_operation": 14940314.5,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 23299285.0,
            "ns_per_operation": 11649642.5,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 29880629.0,
            "ns_per_operation": 14940314.5,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 17878089.0,
            "ns_per_operation": 8939044.5,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 26403813.0,
            "ns_per_operation": 13201906.5,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 2,
        "source_sha256": "ab92800dd4e4d4053498c317effadedf52568270ab7b476eef74b09638de8964",
        "wasm_sha256": "97068a6310a3bc535369e2b6277b801875c14ca206a707fcb9bca9036f41d0df",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_try_table_normal_n1024": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 25,
            "sides": [
              {
                "calibrated_total_ns": 24083263.0,
                "ns_per_operation": 963330.52,
                "observed_batch": 25,
                "requested_batch": 25,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 23744724.0,
                "ns_per_operation": 949788.96,
                "observed_batch": 25,
                "requested_batch": 25,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 24083263.0,
            "ns_per_operation": 963330.52,
            "observed_batch": 25,
            "requested_batch": 25,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 23744724.0,
            "ns_per_operation": 949788.96,
            "observed_batch": 25,
            "requested_batch": 25,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 21129661.0,
            "ns_per_operation": 845186.44,
            "observed_batch": 25,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 16952202.002,
            "ns_per_operation": 892221.158,
            "observed_batch": 19,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 25,
        "source_sha256": "ab92800dd4e4d4053498c317effadedf52568270ab7b476eef74b09638de8964",
        "wasm_sha256": "97068a6310a3bc535369e2b6277b801875c14ca206a707fcb9bca9036f41d0df",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_try_table_normal_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 2051,
            "sides": [
              {
                "calibrated_total_ns": 40598779.977,
                "ns_per_operation": 19794.627,
                "observed_batch": 2051,
                "requested_batch": 2051,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 33696133.324,
                "ns_per_operation": 16429.124,
                "observed_batch": 2051,
                "requested_batch": 2051,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 40598779.977,
            "ns_per_operation": 19794.627,
            "observed_batch": 2051,
            "requested_batch": 2051,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 33696133.324,
            "ns_per_operation": 16429.124,
            "observed_batch": 2051,
            "requested_batch": 2051,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 39975419.547,
            "ns_per_operation": 19490.697,
            "observed_batch": 2051,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 33249335.665,
            "ns_per_operation": 16500.911,
            "observed_batch": 2015,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 2051,
        "source_sha256": "ab92800dd4e4d4053498c317effadedf52568270ab7b476eef74b09638de8964",
        "wasm_sha256": "97068a6310a3bc535369e2b6277b801875c14ca206a707fcb9bca9036f41d0df",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_try_table_normal_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1080,
            "sides": [
              {
                "calibrated_total_ns": 31682830.32,
                "ns_per_operation": 29335.954,
                "observed_batch": 1080,
                "requested_batch": 1080,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 32190502.68,
                "ns_per_operation": 29806.021,
                "observed_batch": 1080,
                "requested_batch": 1080,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 31682830.32,
            "ns_per_operation": 29335.954,
            "observed_batch": 1080,
            "requested_batch": 1080,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 32190502.68,
            "ns_per_operation": 29806.021,
            "observed_batch": 1080,
            "requested_batch": 1080,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 1196184.02,
            "ns_per_operation": 29175.22,
            "observed_batch": 41,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 37739286.72,
            "ns_per_operation": 34943.784,
            "observed_batch": 1080,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1080,
        "source_sha256": "ab92800dd4e4d4053498c317effadedf52568270ab7b476eef74b09638de8964",
        "wasm_sha256": "97068a6310a3bc535369e2b6277b801875c14ca206a707fcb9bca9036f41d0df",
        "workload_kind": "typed_scalar_driver"
      }
    },
    "warm_observed_under_20ms_legacy_count": 103,
    "warm_observed_under_20ms_total_count": 454,
    "warm_observed_under_20ms_typed_count": 351,
    "warm_timed_duration_summary": {
      "0:branch_calls:baseline": {
        "batch": 4,
        "calibration_class": "paired_shared_20ms_legacy",
        "estimated_total_ns": {
          "max": 371232102.0,
          "median": 24523054.0,
          "min": 21437406.0,
          "p95": 31115599.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:branch_calls:candidate": {
        "batch": 4,
        "calibration_class": "paired_shared_20ms_legacy",
        "estimated_total_ns": {
          "max": 43884688.0,
          "median": 24307593.5,
          "min": 21527961.0,
          "p95": 32075764.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:call_overhead:baseline": {
        "batch": 59951,
        "calibration_class": "paired_shared_20ms_legacy",
        "estimated_total_ns": {
          "max": 25450458.471,
          "median": 20590350.803000003,
          "min": 19007944.158,
          "p95": 25416466.254
        },
        "observations": 30,
        "observed_under_20ms_count": 10
      },
      "0:call_overhead:candidate": {
        "batch": 59951,
        "calibration_class": "paired_shared_20ms_legacy",
        "estimated_total_ns": {
          "max": 37497492.019,
          "median": 24317594.399499997,
          "min": 23165665.91,
          "p95": 35280683.892
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:integer_loop:baseline": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms_legacy",
        "estimated_total_ns": {
          "max": 28831602.999,
          "median": 18798389.5005,
          "min": 17090577.998999998,
          "p95": 25408806.999
        },
        "observations": 30,
        "observed_under_20ms_count": 22
      },
      "0:integer_loop:candidate": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms_legacy",
        "estimated_total_ns": {
          "max": 34631724.0,
          "median": 18657757.0005,
          "min": 17173890.0,
          "p95": 32465913.999
        },
        "observations": 30,
        "observed_under_20ms_count": 23
      },
      "0:scalar_accumulator_n0:baseline": {
        "batch": 30770,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 31906705.34,
          "median": 20300430.575000003,
          "min": 18523386.15,
          "p95": 30270233.66
        },
        "observations": 30,
        "observed_under_20ms_count": 13
      },
      "0:scalar_accumulator_n0:candidate": {
        "batch": 30770,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 52430818.43,
          "median": 21448566.97,
          "min": 18068482.47,
          "p95": 34597726.46
        },
        "observations": 30,
        "observed_under_20ms_count": 6
      },
      "0:scalar_accumulator_n10000:baseline": {
        "batch": 4,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 63234945.0,
          "median": 26616738.0,
          "min": 20535134.0,
          "p95": 47967208.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_accumulator_n10000:candidate": {
        "batch": 4,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 47141520.0,
          "median": 26370195.0,
          "min": 21074504.0,
          "p95": 38503689.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_accumulator_n1024:baseline": {
        "batch": 50,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 44899344.0,
          "median": 31085397.0,
          "min": 26256994.0,
          "p95": 37789618.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_accumulator_n1024:candidate": {
        "batch": 50,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 54616402.0,
          "median": 33188701.5,
          "min": 25429760.0,
          "p95": 38712557.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_accumulator_n17:baseline": {
        "batch": 2687,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 48181193.514000006,
          "median": 31313929.880999997,
          "min": 28301440.135999996,
          "p95": 44223277.445
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_accumulator_n17:candidate": {
        "batch": 2687,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 67713254.46599999,
          "median": 31941793.110000003,
          "min": 28234732.674,
          "p95": 48238859.221
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_accumulator_n1:baseline": {
        "batch": 17347,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 357509440.36499995,
          "median": 50690371.2535,
          "min": 30389324.603,
          "p95": 186357554.669
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_accumulator_n1:candidate": {
        "batch": 17347,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 150149699.326,
          "median": 53266999.225,
          "min": 31124421.075,
          "p95": 107260751.015
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_accumulator_n32:baseline": {
        "batch": 1504,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 60948025.312,
          "median": 32387951.92,
          "min": 27211920.128,
          "p95": 59493613.695999995
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_accumulator_n32:candidate": {
        "batch": 1504,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 55570143.936,
          "median": 30721092.031999998,
          "min": 28233534.687999997,
          "p95": 45579718.656
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_br_table_n0:baseline": {
        "batch": 33445,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 27109011.974999998,
          "median": 21382157.735,
          "min": 19343250.2,
          "p95": 24338561.955000002
        },
        "observations": 30,
        "observed_under_20ms_count": 5
      },
      "0:scalar_br_table_n0:candidate": {
        "batch": 33445,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 33721723.93,
          "median": 21738229.927500002,
          "min": 20009173.595,
          "p95": 28469253.57
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_br_table_n10000:baseline": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 35906257.0,
          "median": 23539790.0,
          "min": 21791695.0,
          "p95": 26413576.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_br_table_n10000:candidate": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 24077454.0,
          "median": 22165232.0,
          "min": 20831638.0,
          "p95": 24018944.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_br_table_n1024:baseline": {
        "batch": 15,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 36294067.995,
          "median": 21957045.502499998,
          "min": 19817460.0,
          "p95": 30402990.0
        },
        "observations": 30,
        "observed_under_20ms_count": 2
      },
      "0:scalar_br_table_n1024:candidate": {
        "batch": 15,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 27952584.0,
          "median": 20717494.005000003,
          "min": 18374388.0,
          "p95": 27062407.995
        },
        "observations": 30,
        "observed_under_20ms_count": 6
      },
      "0:scalar_br_table_n17:baseline": {
        "batch": 1453,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 43109770.423,
          "median": 32350479.783,
          "min": 29596655.379,
          "p95": 42917477.497
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_br_table_n17:candidate": {
        "batch": 1453,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 48778289.579,
          "median": 30352290.208499998,
          "min": 28155680.406999998,
          "p95": 39832399.606
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_br_table_n1:baseline": {
        "batch": 12780,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 49947166.62,
          "median": 27814181.130000003,
          "min": 24756278.580000002,
          "p95": 41126436.18
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_br_table_n1:candidate": {
        "batch": 12780,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 222282833.94000003,
          "median": 27536484.51,
          "min": 25321167.36,
          "p95": 46257376.14
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_br_table_n32:baseline": {
        "batch": 773,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 43302704.006,
          "median": 35793254.4335,
          "min": 31997899.801,
          "p95": 41616236.765
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_br_table_n32:candidate": {
        "batch": 773,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 136561020.77499998,
          "median": 33212114.6735,
          "min": 29558221.36,
          "p95": 42120344.077
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_branch_entropy_n0:baseline": {
        "batch": 31974,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 303429231.276,
          "median": 24351222.543,
          "min": 20273562.336,
          "p95": 73122299.82
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_branch_entropy_n0:candidate": {
        "batch": 31974,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 351420574.602,
          "median": 24663672.471,
          "min": 21140249.58,
          "p95": 54625660.56
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_branch_entropy_n10000:baseline": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 268981904.0,
          "median": 23804945.5,
          "min": 19686450.0,
          "p95": 215382424.0
        },
        "observations": 30,
        "observed_under_20ms_count": 2
      },
      "0:scalar_branch_entropy_n10000:candidate": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 564650219.0,
          "median": 22481201.5,
          "min": 20009062.0,
          "p95": 114995548.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_branch_entropy_n1024:baseline": {
        "batch": 25,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 41459745.0,
          "median": 29791562.0,
          "min": 25430023.0,
          "p95": 40414792.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_branch_entropy_n1024:candidate": {
        "batch": 25,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 55876440.0,
          "median": 29050893.5,
          "min": 25458888.0,
          "p95": 44498925.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_branch_entropy_n17:baseline": {
        "batch": 1422,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 49898039.724,
          "median": 34643934.306,
          "min": 29979057.618,
          "p95": 49837126.932
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_branch_entropy_n17:candidate": {
        "batch": 1422,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 61074257.256000005,
          "median": 32668592.868,
          "min": 29341535.202000003,
          "p95": 40716672.048
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_branch_entropy_n1:baseline": {
        "batch": 11555,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 29334100.75,
          "median": 21745764.7025,
          "min": 19775723.865,
          "p95": 28922523.205
        },
        "observations": 30,
        "observed_under_20ms_count": 1
      },
      "0:scalar_branch_entropy_n1:candidate": {
        "batch": 11555,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 39953469.29,
          "median": 21626933.0825,
          "min": 20095647.150000002,
          "p95": 26440185.665
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_branch_entropy_n32:baseline": {
        "batch": 756,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 39961982.339999996,
          "median": 29351146.230000004,
          "min": 26992955.051999997,
          "p95": 35827165.08
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_branch_entropy_n32:candidate": {
        "batch": 756,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 36547472.052,
          "median": 29026870.992000002,
          "min": 26004355.02,
          "p95": 33957945.252000004
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_call_ref_n0:baseline": {
        "batch": 28321,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 58911078.519999996,
          "median": 24688378.614,
          "min": 21216705.470999997,
          "p95": 30956467.297000002
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_call_ref_n0:candidate": {
        "batch": 28321,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 101676808.076,
          "median": 26247407.1825,
          "min": 22177693.643,
          "p95": 39949064.500999995
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_call_ref_n10000:baseline": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 76239858.0,
          "median": 23491761.5,
          "min": 20699371.0,
          "p95": 57260057.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_call_ref_n10000:candidate": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 73546677.0,
          "median": 25443158.0,
          "min": 20305357.0,
          "p95": 71387149.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_call_ref_n1024:baseline": {
        "batch": 20,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 63171600.0,
          "median": 31361090.5,
          "min": 20541203.0,
          "p95": 63071963.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_call_ref_n1024:candidate": {
        "batch": 20,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 90449978.0,
          "median": 28071543.0,
          "min": 19025965.0,
          "p95": 79764641.0
        },
        "observations": 30,
        "observed_under_20ms_count": 2
      },
      "0:scalar_call_ref_n17:baseline": {
        "batch": 1594,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 51072872.612,
          "median": 35927730.276,
          "min": 30167522.762,
          "p95": 50574607.34
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_call_ref_n17:candidate": {
        "batch": 1594,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 57541767.744,
          "median": 35826448.313,
          "min": 30355697.65,
          "p95": 55889137.236
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_call_ref_n1:baseline": {
        "batch": 13765,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 36124948.589999996,
          "median": 25990391.6325,
          "min": 22089383.75,
          "p95": 31140862.330000002
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_call_ref_n1:candidate": {
        "batch": 13765,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 50753454.57,
          "median": 26773386.1275,
          "min": 22693515.834999997,
          "p95": 45977522.64
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_call_ref_n32:baseline": {
        "batch": 758,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 40266770.104,
          "median": 28499756.992000002,
          "min": 25758967.706,
          "p95": 34453113.248
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_call_ref_n32:candidate": {
        "batch": 758,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 49282453.94,
          "median": 27412852.075000003,
          "min": 25329112.728,
          "p95": 36913801.826
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_dependent_n0:baseline": {
        "batch": 29456,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 37715845.328,
          "median": 21890491.504,
          "min": 18714486.672,
          "p95": 36165546.592
        },
        "observations": 30,
        "observed_under_20ms_count": 1
      },
      "0:scalar_dependent_n0:candidate": {
        "batch": 29456,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 39317662.607999995,
          "median": 23228913.232,
          "min": 20069197.568,
          "p95": 35923388.816
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_dependent_n10000:baseline": {
        "batch": 4,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 52723482.0,
          "median": 27366552.5,
          "min": 23856815.0,
          "p95": 42823800.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_dependent_n10000:candidate": {
        "batch": 4,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 131092301.0,
          "median": 27607907.5,
          "min": 24455186.0,
          "p95": 48168013.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_dependent_n1024:baseline": {
        "batch": 38,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 77156542.99599999,
          "median": 25324057.014,
          "min": 21790077.002,
          "p95": 33655877.012
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_dependent_n1024:candidate": {
        "batch": 38,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 33697688.982,
          "median": 25214914.991,
          "min": 22065063.014,
          "p95": 32773238.016
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_dependent_n17:baseline": {
        "batch": 2355,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 62931572.28,
          "median": 27120635.692500003,
          "min": 25380824.1,
          "p95": 41377903.425000004
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_dependent_n17:candidate": {
        "batch": 2355,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 48105813.435,
          "median": 28881734.129999995,
          "min": 23930551.515,
          "p95": 47862134.519999996
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_dependent_n1:baseline": {
        "batch": 17422,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 54591662.779999994,
          "median": 24774415.018,
          "min": 22535670.596,
          "p95": 41013774.814
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_dependent_n1:candidate": {
        "batch": 17422,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 40018804.394,
          "median": 25581219.127,
          "min": 22899929.772,
          "p95": 36831397.228
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_dependent_n32:baseline": {
        "batch": 1317,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 74509124.862,
          "median": 30571668.504,
          "min": 26753810.619,
          "p95": 73996211.31
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_dependent_n32:candidate": {
        "batch": 1317,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 66864896.004,
          "median": 31921038.388499998,
          "min": 27090002.526,
          "p95": 66649936.58099999
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_direct_calls_n0:baseline": {
        "batch": 28369,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 39283064.942,
          "median": 21545801.596,
          "min": 19061074.362,
          "p95": 30542973.208
        },
        "observations": 30,
        "observed_under_20ms_count": 2
      },
      "0:scalar_direct_calls_n0:candidate": {
        "batch": 28369,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 30003309.720999997,
          "median": 22919300.9155,
          "min": 20909314.712,
          "p95": 29724328.975
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_direct_calls_n10000:baseline": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 42850890.0,
          "median": 29990651.001,
          "min": 26274123.0,
          "p95": 40382670.999
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_direct_calls_n10000:candidate": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 35273652.0,
          "median": 29324206.5,
          "min": 25964430.999,
          "p95": 35204070.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_direct_calls_n1024:baseline": {
        "batch": 28,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 757600491.004,
          "median": 33052208.504,
          "min": 27797543.004,
          "p95": 199793332.004
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_direct_calls_n1024:candidate": {
        "batch": 28,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 209419219.996,
          "median": 32307394.994,
          "min": 28081083.996,
          "p95": 141971545.996
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_direct_calls_n17:baseline": {
        "batch": 1752,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 34781339.976,
          "median": 29479763.448,
          "min": 27758705.52,
          "p95": 34572690.792
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_direct_calls_n17:candidate": {
        "batch": 1752,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 44151941.760000005,
          "median": 29661225.972,
          "min": 27112832.472000003,
          "p95": 41462197.776
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_direct_calls_n1:baseline": {
        "batch": 14610,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 47518513.65,
          "median": 26346943.5,
          "min": 23997100.32,
          "p95": 30393153.779999997
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_direct_calls_n1:candidate": {
        "batch": 14610,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 31235274.18,
          "median": 27594871.259999998,
          "min": 23973066.87,
          "p95": 30764583.81
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_direct_calls_n32:baseline": {
        "batch": 955,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 49942371.535000004,
          "median": 33242666.625,
          "min": 30635051.54,
          "p95": 45748473.755
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_direct_calls_n32:candidate": {
        "batch": 955,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 49536019.99,
          "median": 32392177.049999997,
          "min": 29955415.285,
          "p95": 37672694.839999996
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_dynamic_api_n0:baseline": {
        "batch": 53051,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 39756896.859,
          "median": 25735093.151,
          "min": 16565705.26,
          "p95": 39263416.456999995
        },
        "observations": 30,
        "observed_under_20ms_count": 11
      },
      "0:scalar_dynamic_api_n0:candidate": {
        "batch": 53051,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 61989669.092,
          "median": 33820436.908,
          "min": 21939824.610999998,
          "p95": 49085066.393
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_dynamic_api_n10000:baseline": {
        "batch": 45559,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 65995446.188999996,
          "median": 20772466.5935,
          "min": 13673896.024,
          "p95": 42917215.826
        },
        "observations": 30,
        "observed_under_20ms_count": 14
      },
      "0:scalar_dynamic_api_n10000:candidate": {
        "batch": 45559,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 46890325.098,
          "median": 23704529.936,
          "min": 16280918.681,
          "p95": 40441403.089
        },
        "observations": 30,
        "observed_under_20ms_count": 9
      },
      "0:scalar_dynamic_api_n1024:baseline": {
        "batch": 47620,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 68427463.76,
          "median": 31080597.79,
          "min": 17539922.220000003,
          "p95": 54230322.68000001
        },
        "observations": 30,
        "observed_under_20ms_count": 7
      },
      "0:scalar_dynamic_api_n1024:candidate": {
        "batch": 47620,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 246074349.95999998,
          "median": 37012097.370000005,
          "min": 18526180.04,
          "p95": 165474261.79999998
        },
        "observations": 30,
        "observed_under_20ms_count": 2
      },
      "0:scalar_dynamic_api_n17:baseline": {
        "batch": 52771,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 58775548.23500001,
          "median": 20406915.097000003,
          "min": 18566737.556,
          "p95": 36664974.174
        },
        "observations": 30,
        "observed_under_20ms_count": 8
      },
      "0:scalar_dynamic_api_n17:candidate": {
        "batch": 52771,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 39828648.395,
          "median": 25950455.876,
          "min": 21659698.637000002,
          "p95": 38458660.464
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_dynamic_api_n1:baseline": {
        "batch": 55551,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 29741338.788000003,
          "median": 21309419.151,
          "min": 18989442.738,
          "p95": 28867021.599
        },
        "observations": 30,
        "observed_under_20ms_count": 5
      },
      "0:scalar_dynamic_api_n1:candidate": {
        "batch": 55551,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 44456076.525,
          "median": 25937734.0425,
          "min": 22756911.558000002,
          "p95": 42246646.602
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_dynamic_api_n32:baseline": {
        "batch": 62378,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 286415947.60400003,
          "median": 26904255.18,
          "min": 21299654.258,
          "p95": 249414440.808
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_dynamic_api_n32:candidate": {
        "batch": 62378,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 170496726.974,
          "median": 29940317.196000002,
          "min": 25794363.426,
          "p95": 56657500.754
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_host_call_n0:baseline": {
        "batch": 33334,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 46124322.468,
          "median": 22076574.856,
          "min": 19894664.551999997,
          "p95": 25815982.976000004
        },
        "observations": 30,
        "observed_under_20ms_count": 1
      },
      "0:scalar_host_call_n0:candidate": {
        "batch": 33334,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 43507836.806,
          "median": 23270682.071000002,
          "min": 20757415.14,
          "p95": 28605438.764
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_host_call_n10000:baseline": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 37635894.999,
          "median": 22448448.0,
          "min": 20471057.001000002,
          "p95": 35578418.001
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_host_call_n10000:candidate": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 50961570.0,
          "median": 22362209.5005,
          "min": 19939146.998999998,
          "p95": 33522138.0
        },
        "observations": 30,
        "observed_under_20ms_count": 1
      },
      "0:scalar_host_call_n1024:baseline": {
        "batch": 33,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 43105267.986,
          "median": 26187212.501999997,
          "min": 23183275.005,
          "p95": 33454340.997
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_host_call_n1024:candidate": {
        "batch": 33,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 35983091.991,
          "median": 25857011.994,
          "min": 22420021.008,
          "p95": 34503618.996
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_host_call_n17:baseline": {
        "batch": 2051,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 59618336.736,
          "median": 29323123.4135,
          "min": 26072039.217,
          "p95": 47536463.863
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_host_call_n17:candidate": {
        "batch": 2051,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 55913843.096999995,
          "median": 30858058.997500002,
          "min": 26752419.498,
          "p95": 51988202.434
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_host_call_n1:baseline": {
        "batch": 15988,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 41097409.808,
          "median": 24982680.926,
          "min": 22843126.796,
          "p95": 37239969.059999995
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_host_call_n1:candidate": {
        "batch": 15988,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 34466738.544,
          "median": 26086156.698,
          "min": 22281708.176,
          "p95": 31312050.336
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_host_call_n32:baseline": {
        "batch": 1198,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 44495527.782,
          "median": 31023471.494000003,
          "min": 28140946.922,
          "p95": 44031489.67
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_host_call_n32:candidate": {
        "batch": 1198,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 45450973.514,
          "median": 30563506.582,
          "min": 27400979.46,
          "p95": 43044777.335999995
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_indirect_calls_n0:baseline": {
        "batch": 30761,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 195704957.993,
          "median": 24921608.608999997,
          "min": 19503735.201,
          "p95": 167135402.394
        },
        "observations": 30,
        "observed_under_20ms_count": 2
      },
      "0:scalar_indirect_calls_n0:candidate": {
        "batch": 30761,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 97548268.087,
          "median": 26048291.756,
          "min": 20680466.494999997,
          "p95": 92632937.136
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_indirect_calls_n10000:baseline": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 43142583.0,
          "median": 30969127.0005,
          "min": 28406019.0,
          "p95": 40097630.001
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_indirect_calls_n10000:candidate": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 41660862.0,
          "median": 30222012.501,
          "min": 27445650.0,
          "p95": 34049996.001
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_indirect_calls_n1024:baseline": {
        "batch": 22,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 40270746.010000005,
          "median": 22471224.996,
          "min": 20525923.0,
          "p95": 27111438.002
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_indirect_calls_n1024:candidate": {
        "batch": 22,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 27500165.0,
          "median": 21903226.004,
          "min": 19924014.0,
          "p95": 26521506.99
        },
        "observations": 30,
        "observed_under_20ms_count": 1
      },
      "0:scalar_indirect_calls_n17:baseline": {
        "batch": 1615,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 40554008.215,
          "median": 27597719.165,
          "min": 24801708.425,
          "p95": 40113442.675
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_indirect_calls_n17:candidate": {
        "batch": 1615,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 110523345.42,
          "median": 26974576.26,
          "min": 24378938.57,
          "p95": 34344455.955000006
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_indirect_calls_n1:baseline": {
        "batch": 14525,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 36988130.375,
          "median": 26538424.15,
          "min": 24244679.725,
          "p95": 32541475.925
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_indirect_calls_n1:candidate": {
        "batch": 14525,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 35290535.525,
          "median": 25801694.3625,
          "min": 24204372.85,
          "p95": 31656758.175
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_indirect_calls_n32:baseline": {
        "batch": 643,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 28474903.279,
          "median": 21588013.842,
          "min": 20374512.092,
          "p95": 28404703.754
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_indirect_calls_n32:candidate": {
        "batch": 643,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 28520751.751,
          "median": 21266150.8685,
          "min": 19143900.112,
          "p95": 23656667.012000002
        },
        "observations": 30,
        "observed_under_20ms_count": 2
      },
      "0:scalar_multivalue_branch_n0:baseline": {
        "batch": 30817,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 77715173.476,
          "median": 28840104.8585,
          "min": 22896969.366,
          "p95": 62829607.149000004
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_multivalue_branch_n0:candidate": {
        "batch": 30817,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 61203239.974,
          "median": 29938761.725500003,
          "min": 22955614.117,
          "p95": 40426480.208000004
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_multivalue_branch_n10000:baseline": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 60551860.0,
          "median": 30000834.0,
          "min": 26912509.0,
          "p95": 49392094.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_multivalue_branch_n10000:candidate": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 67605054.0,
          "median": 29361214.0,
          "min": 25580182.0,
          "p95": 61531348.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_multivalue_branch_n1024:baseline": {
        "batch": 17,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 45212607.992,
          "median": 30138695.9985,
          "min": 25086243.001,
          "p95": 37784206.001
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_multivalue_branch_n1024:candidate": {
        "batch": 17,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 34094794.992,
          "median": 28515045.998999998,
          "min": 24070811.003,
          "p95": 32668289.003
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_multivalue_branch_n17:baseline": {
        "batch": 1227,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 64827273.828,
          "median": 38779683.467999995,
          "min": 31627073.472000003,
          "p95": 64522066.166999996
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_multivalue_branch_n17:candidate": {
        "batch": 1227,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 106287017.32200001,
          "median": 36131630.3505,
          "min": 30050590.743,
          "p95": 64221099.018
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_multivalue_branch_n1:baseline": {
        "batch": 11690,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 42587102.53,
          "median": 29345781.08,
          "min": 25451807.009999998,
          "p95": 39774745.71
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_multivalue_branch_n1:candidate": {
        "batch": 11690,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 40490840.04,
          "median": 29328666.92,
          "min": 25912498.22,
          "p95": 35687664.53
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_multivalue_branch_n32:baseline": {
        "batch": 390,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 39393407.04,
          "median": 20966179.455,
          "min": 18626942.1,
          "p95": 25616990.099999998
        },
        "observations": 30,
        "observed_under_20ms_count": 7
      },
      "0:scalar_multivalue_branch_n32:candidate": {
        "batch": 390,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 28525502.85,
          "median": 21573894.03,
          "min": 18013672.169999998,
          "p95": 24481205.97
        },
        "observations": 30,
        "observed_under_20ms_count": 7
      },
      "0:scalar_ordinary_recursion_n0:baseline": {
        "batch": 24267,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 40066637.025,
          "median": 25630696.1985,
          "min": 21314094.372,
          "p95": 38697371.550000004
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_ordinary_recursion_n0:candidate": {
        "batch": 24267,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 134360506.386,
          "median": 25405255.7685,
          "min": 21248088.132,
          "p95": 49890331.164
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_ordinary_recursion_n17:baseline": {
        "batch": 1942,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 64172873.427999996,
          "median": 37598246.36,
          "min": 33641491.272,
          "p95": 59267587.28
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_ordinary_recursion_n17:candidate": {
        "batch": 1942,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 43183710.24,
          "median": 32428238.424000002,
          "min": 28546508.621999998,
          "p95": 38815460.888000004
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_ordinary_recursion_n1:baseline": {
        "batch": 16434,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 61990181.946,
          "median": 30049437.527999997,
          "min": 26128564.506,
          "p95": 42033685.518
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_ordinary_recursion_n1:candidate": {
        "batch": 16434,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 59943655.92600001,
          "median": 29456671.365000002,
          "min": 26235615.582,
          "p95": 45658910.88
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_ordinary_recursion_n32:baseline": {
        "batch": 1070,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 68250765.34,
          "median": 35399377.1,
          "min": 31050924.92,
          "p95": 40998922.5
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_ordinary_recursion_n32:candidate": {
        "batch": 1070,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 37518345.18000001,
          "median": 31104159.025,
          "min": 27360761.35,
          "p95": 35554380.51
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_table_mutation_n0:baseline": {
        "batch": 25807,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 25385494.269,
          "median": 20089859.1585,
          "min": 17454719.292,
          "p95": 22638390.733000003
        },
        "observations": 30,
        "observed_under_20ms_count": 13
      },
      "0:scalar_table_mutation_n0:candidate": {
        "batch": 25807,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 30291714.653,
          "median": 20594786.017,
          "min": 19172794.509999998,
          "p95": 27417382.607
        },
        "observations": 30,
        "observed_under_20ms_count": 9
      },
      "0:scalar_table_mutation_n10000:baseline": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 169994948.0,
          "median": 23583905.0,
          "min": 16270285.0,
          "p95": 162054484.0
        },
        "observations": 30,
        "observed_under_20ms_count": 9
      },
      "0:scalar_table_mutation_n10000:candidate": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 737598606.0,
          "median": 27878656.5,
          "min": 17051702.0,
          "p95": 348929930.0
        },
        "observations": 30,
        "observed_under_20ms_count": 10
      },
      "0:scalar_table_mutation_n1024:baseline": {
        "batch": 22,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 34990731.996,
          "median": 22767408.004,
          "min": 19823056.990000002,
          "p95": 29056519.008
        },
        "observations": 30,
        "observed_under_20ms_count": 1
      },
      "0:scalar_table_mutation_n1024:candidate": {
        "batch": 22,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 31234920.992,
          "median": 22484061.996,
          "min": 19978138.994,
          "p95": 30238010.99
        },
        "observations": 30,
        "observed_under_20ms_count": 1
      },
      "0:scalar_table_mutation_n17:baseline": {
        "batch": 1771,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 51752356.81,
          "median": 33605128.788,
          "min": 29405134.99,
          "p95": 48821831.751
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_table_mutation_n17:candidate": {
        "batch": 1771,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 39265209.368,
          "median": 32933307.9075,
          "min": 29519585.865,
          "p95": 38360878.325
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_table_mutation_n1:baseline": {
        "batch": 13727,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 55597328.759,
          "median": 28065228.9925,
          "min": 24292740.535,
          "p95": 52492981.436000004
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_table_mutation_n1:candidate": {
        "batch": 13727,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 65341604.433,
          "median": 30300018.909999996,
          "min": 24281402.033,
          "p95": 63930468.833000004
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_table_mutation_n32:baseline": {
        "batch": 964,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 60110270.128000006,
          "median": 33975148.038,
          "min": 30004819.084,
          "p95": 48765236.58
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_table_mutation_n32:candidate": {
        "batch": 964,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 35693423.572,
          "median": 31251586.794,
          "min": 28451763.992,
          "p95": 35392627.616
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_tail_calls_n0:baseline": {
        "batch": 27510,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 44497287.449999996,
          "median": 24953124.314999998,
          "min": 21540770.16,
          "p95": 29979792.78
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_tail_calls_n0:candidate": {
        "batch": 27510,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 33097391.040000003,
          "median": 24096944.339999996,
          "min": 20955549.93,
          "p95": 28509273.240000002
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_tail_calls_n10000:baseline": {
        "batch": 4,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 38896949.0,
          "median": 27620158.0,
          "min": 25179304.0,
          "p95": 36075947.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_tail_calls_n10000:candidate": {
        "batch": 4,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 47936163.0,
          "median": 27662855.5,
          "min": 24201170.0,
          "p95": 35950724.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_tail_calls_n1024:baseline": {
        "batch": 31,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 257713101.009,
          "median": 23834163.504,
          "min": 19854917.014000002,
          "p95": 122000131.99900001
        },
        "observations": 30,
        "observed_under_20ms_count": 1
      },
      "0:scalar_tail_calls_n1024:candidate": {
        "batch": 31,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 63138472.985,
          "median": 22361618.0115,
          "min": 19538837.014999997,
          "p95": 42140846.991000004
        },
        "observations": 30,
        "observed_under_20ms_count": 2
      },
      "0:scalar_tail_calls_n17:baseline": {
        "batch": 2365,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 173038405.925,
          "median": 42874493.75,
          "min": 26730887.865,
          "p95": 114129406.105
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_tail_calls_n17:candidate": {
        "batch": 2365,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 672931343.59,
          "median": 39840035.565,
          "min": 25548184.474999998,
          "p95": 193448417.415
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_tail_calls_n1:baseline": {
        "batch": 14546,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 133404755.21800001,
          "median": 22686559.805,
          "min": 20676789.896,
          "p95": 36421380.296000004
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_tail_calls_n1:candidate": {
        "batch": 14546,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 73430404.446,
          "median": 22574141.044,
          "min": 20936814.192,
          "p95": 32118513.490000002
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_tail_calls_n32:baseline": {
        "batch": 1314,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 371337468.282,
          "median": 31016420.748000003,
          "min": 25371564.786000002,
          "p95": 321036206.68799996
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_tail_calls_n32:candidate": {
        "batch": 1314,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 212912283.114,
          "median": 29533065.858,
          "min": 24870120.048,
          "p95": 149187219.858
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_try_table_normal_n0:baseline": {
        "batch": 35760,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 156666884.39999998,
          "median": 25346258.880000003,
          "min": 20854481.04,
          "p95": 65604008.64
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_try_table_normal_n0:candidate": {
        "batch": 35760,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 182516965.92,
          "median": 27263692.200000003,
          "min": 21493655.28,
          "p95": 60456106.32
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_try_table_normal_n10000:baseline": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 41359629.999,
          "median": 24257553.998999998,
          "min": 21385151.001000002,
          "p95": 32692152.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_try_table_normal_n10000:candidate": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 30374649.999,
          "median": 22545987.0,
          "min": 20270145.0,
          "p95": 28425231.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_try_table_normal_n1024:baseline": {
        "batch": 32,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 146084124.0,
          "median": 27877577.008,
          "min": 23781492.0,
          "p95": 102764524.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_try_table_normal_n1024:candidate": {
        "batch": 32,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 58506555.008,
          "median": 26531886.0,
          "min": 23004816.0,
          "p95": 58207967.008
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_try_table_normal_n17:baseline": {
        "batch": 2168,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 131417376.328,
          "median": 50371293.572,
          "min": 35630180.28,
          "p95": 82837591.12799999
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_try_table_normal_n17:candidate": {
        "batch": 2168,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 115151537.904,
          "median": 55300549.428,
          "min": 25320835.136,
          "p95": 104798546.584
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_try_table_normal_n1:baseline": {
        "batch": 17212,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 29033563.051999997,
          "median": 26050757.876000002,
          "min": 23511213.336000003,
          "p95": 28774935.540000003
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_try_table_normal_n1:candidate": {
        "batch": 17212,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 40175183.256,
          "median": 26112454.29,
          "min": 23786416.004,
          "p95": 33907984.24
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_try_table_normal_n32:baseline": {
        "batch": 1146,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 40249424.651999995,
          "median": 30254608.572000004,
          "min": 27455825.598,
          "p95": 34290310.602
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_try_table_normal_n32:candidate": {
        "batch": 1146,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 52275972.672,
          "median": 29341407.585,
          "min": 27078390.498,
          "p95": 50225394.762
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:branch_calls:baseline": {
        "batch": 4,
        "calibration_class": "paired_shared_20ms_legacy",
        "estimated_total_ns": {
          "max": 35172273.0,
          "median": 27534163.5,
          "min": 23959462.0,
          "p95": 33564760.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:branch_calls:candidate": {
        "batch": 4,
        "calibration_class": "paired_shared_20ms_legacy",
        "estimated_total_ns": {
          "max": 40537445.0,
          "median": 26756977.5,
          "min": 24934006.0,
          "p95": 32382804.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:call_overhead:baseline": {
        "batch": 48662,
        "calibration_class": "paired_shared_20ms_legacy",
        "estimated_total_ns": {
          "max": 27629796.979999997,
          "median": 18508737.686,
          "min": 16327366.212000001,
          "p95": 24350124.165999997
        },
        "observations": 30,
        "observed_under_20ms_count": 19
      },
      "1:call_overhead:candidate": {
        "batch": 48662,
        "calibration_class": "paired_shared_20ms_legacy",
        "estimated_total_ns": {
          "max": 39698605.586,
          "median": 22992113.732,
          "min": 19713706.13,
          "p95": 31042998.322
        },
        "observations": 30,
        "observed_under_20ms_count": 3
      },
      "1:integer_loop:baseline": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms_legacy",
        "estimated_total_ns": {
          "max": 42158529.0,
          "median": 19831396.9995,
          "min": 17577998.001000002,
          "p95": 27549363.0
        },
        "observations": 30,
        "observed_under_20ms_count": 15
      },
      "1:integer_loop:candidate": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms_legacy",
        "estimated_total_ns": {
          "max": 27659676.999,
          "median": 20864205.0,
          "min": 18675299.001000002,
          "p95": 25616937.999
        },
        "observations": 30,
        "observed_under_20ms_count": 11
      },
      "1:scalar_accumulator_n0:baseline": {
        "batch": 30304,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 60958859.712,
          "median": 29957079.808,
          "min": 20664994.592,
          "p95": 55038518.752
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_accumulator_n0:candidate": {
        "batch": 30304,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 56722451.423999995,
          "median": 30245149.632,
          "min": 20733754.368,
          "p95": 45884922.816
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_accumulator_n10000:baseline": {
        "batch": 4,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 165171559.0,
          "median": 36101172.0,
          "min": 26026559.0,
          "p95": 159647911.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_accumulator_n10000:candidate": {
        "batch": 4,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 65070258.0,
          "median": 35042835.0,
          "min": 22909263.0,
          "p95": 63417147.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_accumulator_n1024:baseline": {
        "batch": 34,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 29274915.008,
          "median": 22406854.012000002,
          "min": 19929417.016000003,
          "p95": 26114680.016000003
        },
        "observations": 30,
        "observed_under_20ms_count": 1
      },
      "1:scalar_accumulator_n1024:candidate": {
        "batch": 34,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 32591504.983999997,
          "median": 23586041.009000003,
          "min": 20544378.994,
          "p95": 29859203.002
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_accumulator_n17:baseline": {
        "batch": 2408,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 102094805.4,
          "median": 55830008.555999994,
          "min": 30273496.4,
          "p95": 68588932.60800001
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_accumulator_n17:candidate": {
        "batch": 2408,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 106276174.59200001,
          "median": 56008951.852,
          "min": 30267806.296,
          "p95": 69158626.88
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_accumulator_n1:baseline": {
        "batch": 19551,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 44042479.047000006,
          "median": 28398247.8465,
          "min": 24931083.282,
          "p95": 38246389.587000005
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_accumulator_n1:candidate": {
        "batch": 19551,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 46989401.277,
          "median": 28540872.391499996,
          "min": 26068580.013,
          "p95": 46637463.725999996
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_accumulator_n32:baseline": {
        "batch": 1505,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 51494792.545,
          "median": 34022087.19,
          "min": 28869343.44,
          "p95": 50859470.339999996
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_accumulator_n32:candidate": {
        "batch": 1505,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 48374870.355000004,
          "median": 33676383.4225,
          "min": 30099765.220000003,
          "p95": 46391215.64
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_br_table_n0:baseline": {
        "batch": 31008,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 54998548.512,
          "median": 27887091.312,
          "min": 22734817.536,
          "p95": 52167983.232
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_br_table_n0:candidate": {
        "batch": 31008,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 51641498.4,
          "median": 27963805.104,
          "min": 22887407.904000003,
          "p95": 49431434.208000004
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_br_table_n10000:baseline": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 114834498.0,
          "median": 33817344.5,
          "min": 25140279.0,
          "p95": 60896728.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_br_table_n10000:candidate": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 123536877.0,
          "median": 29804094.0,
          "min": 24205373.0,
          "p95": 110381314.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_br_table_n1024:baseline": {
        "batch": 17,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 58125346.0,
          "median": 25211191.4965,
          "min": 21273219.994,
          "p95": 39805266.998
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_br_table_n1024:candidate": {
        "batch": 17,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 39672343.998,
          "median": 24641160.9945,
          "min": 20166429.996,
          "p95": 36529451.998
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_br_table_n17:baseline": {
        "batch": 1457,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 103909495.50999999,
          "median": 40931321.060499996,
          "min": 34706214.982,
          "p95": 95826916.226
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_br_table_n17:candidate": {
        "batch": 1457,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 233973219.20000002,
          "median": 36950891.037,
          "min": 33174440.285,
          "p95": 64465265.142
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_br_table_n1:baseline": {
        "batch": 13918,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 41231657.46,
          "median": 27292314.207000002,
          "min": 24926887.476,
          "p95": 34198919.896
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_br_table_n1:candidate": {
        "batch": 13918,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 39595179.019999996,
          "median": 27648698.515,
          "min": 24382944.200000003,
          "p95": 36838371.17
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_br_table_n32:baseline": {
        "batch": 739,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 55246951.252,
          "median": 33001657.498000003,
          "min": 28542728.811,
          "p95": 45084059.194
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_br_table_n32:candidate": {
        "batch": 739,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 133083905.764,
          "median": 31598994.853,
          "min": 26423754.944,
          "p95": 49349163.7
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_branch_entropy_n0:baseline": {
        "batch": 29681,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 56802102.793,
          "median": 23092248.3745,
          "min": 21421519.725,
          "p95": 41214650.747
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_branch_entropy_n0:candidate": {
        "batch": 29681,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 34791237.132,
          "median": 24660251.083,
          "min": 22146003.254,
          "p95": 30784955.114
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_branch_entropy_n10000:baseline": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 56429877.0,
          "median": 23773706.5,
          "min": 20814841.0,
          "p95": 32891045.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_branch_entropy_n10000:candidate": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 32724762.0,
          "median": 23567467.5,
          "min": 20771816.0,
          "p95": 32581917.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_branch_entropy_n1024:baseline": {
        "batch": 19,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 77840962.003,
          "median": 28401007.9955,
          "min": 21676032.998,
          "p95": 61261430.998
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_branch_entropy_n1024:candidate": {
        "batch": 19,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 52457809.004,
          "median": 26520505.501,
          "min": 21351626.998,
          "p95": 41986315.995
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_branch_entropy_n17:baseline": {
        "batch": 1392,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 69416846.208,
          "median": 30394689.575999998,
          "min": 27850988.448,
          "p95": 46626270.528
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_branch_entropy_n17:candidate": {
        "batch": 1392,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 48133832.975999996,
          "median": 30910543.2,
          "min": 27161180.064,
          "p95": 42807076.32
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_branch_entropy_n1:baseline": {
        "batch": 12301,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 37538432.757,
          "median": 26756317.1835,
          "min": 22377130.431,
          "p95": 36259571.593
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_branch_entropy_n1:candidate": {
        "batch": 12301,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 42864163.008,
          "median": 26664711.6365,
          "min": 22864828.178,
          "p95": 37111317.435
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_branch_entropy_n32:baseline": {
        "batch": 749,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 131625551.701,
          "median": 33898847.192,
          "min": 28900753.714,
          "p95": 65897962.242
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_branch_entropy_n32:candidate": {
        "batch": 749,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 94045195.741,
          "median": 36896557.5335,
          "min": 28561208.046,
          "p95": 69176378.684
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_call_ref_n0:baseline": {
        "batch": 27473,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 58547765.246,
          "median": 22773619.7215,
          "min": 20372630.623,
          "p95": 52879755.67
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_call_ref_n0:candidate": {
        "batch": 27473,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 87399277.967,
          "median": 23362833.1525,
          "min": 20781456.336,
          "p95": 46900998.991
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_call_ref_n10000:baseline": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 36491147.0,
          "median": 22976131.0,
          "min": 20510141.0,
          "p95": 32799631.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_call_ref_n10000:candidate": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 59510791.0,
          "median": 23223285.5,
          "min": 19775627.0,
          "p95": 59282352.0
        },
        "observations": 30,
        "observed_under_20ms_count": 1
      },
      "1:scalar_call_ref_n1024:baseline": {
        "batch": 23,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 43332965.011,
          "median": 27730118.9925,
          "min": 23310763.005000003,
          "p95": 32144515.007
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_call_ref_n1024:candidate": {
        "batch": 23,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 57361533.008,
          "median": 26464545.498999998,
          "min": 22655531.99,
          "p95": 34428484.996
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_call_ref_n17:baseline": {
        "batch": 1567,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 46169732.291,
          "median": 35536620.308,
          "min": 31579335.745,
          "p95": 45010526.804
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_call_ref_n17:candidate": {
        "batch": 1567,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 61925600.75700001,
          "median": 34789176.1945,
          "min": 30395166.381,
          "p95": 52784523.494
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_call_ref_n1:baseline": {
        "batch": 13765,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 130793956.33000001,
          "median": 40046720.975,
          "min": 24819974.330000002,
          "p95": 71572081.05
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_call_ref_n1:candidate": {
        "batch": 13765,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 367964259.375,
          "median": 37498144.99,
          "min": 24830256.785,
          "p95": 86594431.21
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_call_ref_n32:baseline": {
        "batch": 841,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 131973542.34500001,
          "median": 36126204.567999996,
          "min": 29396352.686,
          "p95": 83269911.134
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_call_ref_n32:candidate": {
        "batch": 841,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 62673096.191999994,
          "median": 33583470.9235,
          "min": 28524486.303999998,
          "p95": 43588462.324999996
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_dependent_n0:baseline": {
        "batch": 30304,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 35947059.36,
          "median": 25013467.072,
          "min": 22176800.544,
          "p95": 33517133.119999997
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_dependent_n0:candidate": {
        "batch": 30304,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 45337844.704,
          "median": 24947419.504,
          "min": 21926489.504,
          "p95": 37531200.96
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_dependent_n10000:baseline": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 39569394.0,
          "median": 22445763.0,
          "min": 19632849.0,
          "p95": 38522348.001
        },
        "observations": 30,
        "observed_under_20ms_count": 2
      },
      "1:scalar_dependent_n10000:candidate": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 35354912.001,
          "median": 22708138.5,
          "min": 20026949.001000002,
          "p95": 30662969.001
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_dependent_n1024:baseline": {
        "batch": 33,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 235591963.99199998,
          "median": 26295706.503,
          "min": 21817330.985999998,
          "p95": 96518195.994
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_dependent_n1024:candidate": {
        "batch": 33,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 38307350.004,
          "median": 25809258.502499998,
          "min": 22385464.992,
          "p95": 37091669.01
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_dependent_n17:baseline": {
        "batch": 2406,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 49106505.714,
          "median": 32886255.813,
          "min": 27437047.163999997,
          "p95": 45132123.335999995
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_dependent_n17:candidate": {
        "batch": 2406,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 56622358.88999999,
          "median": 32230077.057,
          "min": 28602058.830000002,
          "p95": 50355462.72
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_dependent_n1:baseline": {
        "batch": 17362,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 109987818.588,
          "median": 41075635.951000005,
          "min": 27735673.466,
          "p95": 106423261.09200001
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_dependent_n1:candidate": {
        "batch": 17362,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 255580515.60799998,
          "median": 44461486.381,
          "min": 26857642.402000003,
          "p95": 80063786.55600001
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_dependent_n32:baseline": {
        "batch": 1327,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 65964298.161,
          "median": 40994330.249,
          "min": 29800295.684,
          "p95": 60786281.792
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_dependent_n32:candidate": {
        "batch": 1327,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 85364782.05,
          "median": 38671704.04450001,
          "min": 23359268.582,
          "p95": 58160513.71700001
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_direct_calls_n0:baseline": {
        "batch": 29028,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 46765791.624,
          "median": 29084706.198,
          "min": 21548935.8,
          "p95": 46750174.56
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_direct_calls_n0:candidate": {
        "batch": 29028,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 54697373.316,
          "median": 28119467.141999997,
          "min": 24567383.351999998,
          "p95": 45236625.611999996
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_direct_calls_n10000:baseline": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 95486733.999,
          "median": 31925310.999,
          "min": 26913420.0,
          "p95": 63283371.999
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_direct_calls_n10000:candidate": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 404725485.0,
          "median": 32701215.0,
          "min": 26255460.0,
          "p95": 180798735.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_direct_calls_n1024:baseline": {
        "batch": 24,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 42437491.992,
          "median": 23239650.504,
          "min": 21749575.008,
          "p95": 41957010.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_direct_calls_n1024:candidate": {
        "batch": 24,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 69226156.992,
          "median": 23154226.008,
          "min": 20983575.0,
          "p95": 42215697.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_direct_calls_n17:baseline": {
        "batch": 1736,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 86403228.52,
          "median": 38618485.724,
          "min": 32193363.104,
          "p95": 65943498.096
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_direct_calls_n17:candidate": {
        "batch": 1736,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 76072971.296,
          "median": 38298834.308,
          "min": 31613460.984,
          "p95": 67124523.19999999
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_direct_calls_n1:baseline": {
        "batch": 14420,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 80648637.44,
          "median": 33134434.619999997,
          "min": 26460411.6,
          "p95": 78457085.84
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_direct_calls_n1:candidate": {
        "batch": 14420,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 94146290.97999999,
          "median": 32372229.47,
          "min": 26843796.14,
          "p95": 63037289.56
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_direct_calls_n32:baseline": {
        "batch": 959,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 97188904.63100001,
          "median": 38899699.307,
          "min": 29450208.151,
          "p95": 85030643.173
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_direct_calls_n32:candidate": {
        "batch": 959,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 120040438.049,
          "median": 39224908.1945,
          "min": 29242367.917,
          "p95": 84950308.702
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_dynamic_api_n0:baseline": {
        "batch": 45978,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 25933569.054,
          "median": 17957190.669,
          "min": 14932137.126,
          "p95": 25686345.348
        },
        "observations": 30,
        "observed_under_20ms_count": 23
      },
      "1:scalar_dynamic_api_n0:candidate": {
        "batch": 45978,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 51251998.446,
          "median": 20767113.15,
          "min": 17758818.588,
          "p95": 31770476.154000003
        },
        "observations": 30,
        "observed_under_20ms_count": 7
      },
      "1:scalar_dynamic_api_n10000:baseline": {
        "batch": 65509,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 69671507.369,
          "median": 23995750.173,
          "min": 20179195.832999997,
          "p95": 45040647.441
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_dynamic_api_n10000:candidate": {
        "batch": 65509,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 35721861.173,
          "median": 26863308.3845,
          "min": 23439251.218000002,
          "p95": 34757699.710999995
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_dynamic_api_n1024:baseline": {
        "batch": 62774,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 58675422.765999995,
          "median": 21704549.918,
          "min": 19830683.244,
          "p95": 31810222.308000002
        },
        "observations": 30,
        "observed_under_20ms_count": 1
      },
      "1:scalar_dynamic_api_n1024:candidate": {
        "batch": 62774,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 31442052.798,
          "median": 26006546.299000002,
          "min": 22187219.204,
          "p95": 31196732.006
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_dynamic_api_n17:baseline": {
        "batch": 53722,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 59905831.975999996,
          "median": 22234756.831,
          "min": 18552248.036000002,
          "p95": 42098170.86
        },
        "observations": 30,
        "observed_under_20ms_count": 5
      },
      "1:scalar_dynamic_api_n17:candidate": {
        "batch": 53722,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 44989757.510000005,
          "median": 26221278.424,
          "min": 20642141.28,
          "p95": 35566059.158
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_dynamic_api_n1:baseline": {
        "batch": 56831,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 26443407.469,
          "median": 19823562.096,
          "min": 17628351.059,
          "p95": 24377771.112
        },
        "observations": 30,
        "observed_under_20ms_count": 16
      },
      "1:scalar_dynamic_api_n1:candidate": {
        "batch": 56831,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 28501542.134,
          "median": 24084381.074500002,
          "min": 20751385.002,
          "p95": 27340200.649
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_dynamic_api_n32:baseline": {
        "batch": 62416,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 59371160.272,
          "median": 21096233.504,
          "min": 18696525.552,
          "p95": 30586461.472
        },
        "observations": 30,
        "observed_under_20ms_count": 7
      },
      "1:scalar_dynamic_api_n32:candidate": {
        "batch": 62416,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 36022645.408,
          "median": 24433991.52,
          "min": 22598773.872,
          "p95": 33879404.8
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_host_call_n0:baseline": {
        "batch": 28051,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 33218022.251000002,
          "median": 20901725.783,
          "min": 18031996.279,
          "p95": 28856316.159
        },
        "observations": 30,
        "observed_under_20ms_count": 11
      },
      "1:scalar_host_call_n0:candidate": {
        "batch": 28051,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 50399147.547,
          "median": 21123847.6265,
          "min": 18414611.919,
          "p95": 30897895.990000002
        },
        "observations": 30,
        "observed_under_20ms_count": 7
      },
      "1:scalar_host_call_n10000:baseline": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 50101889.001,
          "median": 29746380.501,
          "min": 26480262.999,
          "p95": 34383533.001
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_host_call_n10000:candidate": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 50370039.999,
          "median": 29949699.0,
          "min": 25988474.001,
          "p95": 43313655.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_host_call_n1024:baseline": {
        "batch": 20,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 56649455.0,
          "median": 20078751.0,
          "min": 16651678.0,
          "p95": 43124304.0
        },
        "observations": 30,
        "observed_under_20ms_count": 14
      },
      "1:scalar_host_call_n1024:candidate": {
        "batch": 20,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 41660907.0,
          "median": 20644950.5,
          "min": 16611085.0,
          "p95": 40841381.0
        },
        "observations": 30,
        "observed_under_20ms_count": 12
      },
      "1:scalar_host_call_n17:baseline": {
        "batch": 2163,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 72199750.35,
          "median": 35424720.681,
          "min": 31750384.995,
          "p95": 45252815.16
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_host_call_n17:candidate": {
        "batch": 2163,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 39739635.306,
          "median": 34062809.361,
          "min": 30713922.981,
          "p95": 39096655.437
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_host_call_n1:baseline": {
        "batch": 13343,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 53532342.831,
          "median": 23136762.0,
          "min": 20471057.431,
          "p95": 37528775.317
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_host_call_n1:candidate": {
        "batch": 13343,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 181941011.67000002,
          "median": 24132630.148000002,
          "min": 20902890.283,
          "p95": 36016279.552
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_host_call_n32:baseline": {
        "batch": 1153,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 101635122.49499999,
          "median": 39596680.5475,
          "min": 34224541.661,
          "p95": 50925004.12899999
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_host_call_n32:candidate": {
        "batch": 1153,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 43409708.62100001,
          "median": 37025074.3145,
          "min": 32758767.351,
          "p95": 42679732.791
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_indirect_calls_n0:baseline": {
        "batch": 28944,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 30706544.88,
          "median": 22850347.32,
          "min": 21308399.136,
          "p95": 27741174.191999998
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_indirect_calls_n0:candidate": {
        "batch": 28944,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 50973075.792,
          "median": 24988918.176,
          "min": 21497142.96,
          "p95": 32482201.391999997
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_indirect_calls_n10000:baseline": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 29785568.0,
          "median": 22629383.5,
          "min": 18119938.0,
          "p95": 29745062.0
        },
        "observations": 30,
        "observed_under_20ms_count": 5
      },
      "1:scalar_indirect_calls_n10000:candidate": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 25202445.0,
          "median": 21591443.5,
          "min": 18219205.0,
          "p95": 24520019.0
        },
        "observations": 30,
        "observed_under_20ms_count": 7
      },
      "1:scalar_indirect_calls_n1024:baseline": {
        "batch": 22,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 35621713.006,
          "median": 25721771.998999998,
          "min": 22513240.002,
          "p95": 32425124.006
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_indirect_calls_n1024:candidate": {
        "batch": 22,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 41685399.997999996,
          "median": 25062083.002,
          "min": 22198536.998,
          "p95": 36968023.004
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_indirect_calls_n17:baseline": {
        "batch": 1640,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 557387377.28,
          "median": 36222416.78,
          "min": 29405621.480000004,
          "p95": 60705428.199999996
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_indirect_calls_n17:candidate": {
        "batch": 1640,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 149266717.24,
          "median": 35368958.32,
          "min": 30022453.36,
          "p95": 101498988.28
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_indirect_calls_n1:baseline": {
        "batch": 14016,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 45087734.016,
          "median": 32095967.232,
          "min": 28115703.552,
          "p95": 43254230.976
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_indirect_calls_n1:candidate": {
        "batch": 14016,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 38280947.712,
          "median": 31906393.824,
          "min": 27265324.8,
          "p95": 37345940.352
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_indirect_calls_n32:baseline": {
        "batch": 852,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 49530673.236,
          "median": 34841731.026,
          "min": 29580600.78,
          "p95": 43456733.712
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_indirect_calls_n32:candidate": {
        "batch": 852,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 49463871.324,
          "median": 33112319.7,
          "min": 28185898.080000002,
          "p95": 45495918.18
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_multivalue_branch_n0:baseline": {
        "batch": 28931,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 44852452.575,
          "median": 23127788.571999997,
          "min": 20852133.974,
          "p95": 33978909.811
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_multivalue_branch_n0:candidate": {
        "batch": 28931,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 30440619.580000002,
          "median": 24278649.2865,
          "min": 21372400.147,
          "p95": 29468566.911000002
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_multivalue_branch_n10000:baseline": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 120649303.0,
          "median": 31443299.5,
          "min": 28437455.0,
          "p95": 67198756.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_multivalue_branch_n10000:candidate": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 64693468.0,
          "median": 31190273.5,
          "min": 26996910.0,
          "p95": 39491122.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_multivalue_branch_n1024:baseline": {
        "batch": 17,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 44989336.996,
          "median": 24203704.4995,
          "min": 21283164.994,
          "p95": 37985812.996
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_multivalue_branch_n1024:candidate": {
        "batch": 17,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 37487349.995,
          "median": 24088040.996,
          "min": 21319245.998,
          "p95": 33190241.006
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_multivalue_branch_n17:baseline": {
        "batch": 1233,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 41827040.505,
          "median": 31254445.269,
          "min": 29115371.52,
          "p95": 39457817.442
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_multivalue_branch_n17:candidate": {
        "batch": 1233,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 74657674.062,
          "median": 32445700.204500005,
          "min": 28996140.42,
          "p95": 52968253.419
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_multivalue_branch_n1:baseline": {
        "batch": 12508,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 32865157.748,
          "median": 27633417.825999998,
          "min": 26001017.508,
          "p95": 31975038.436
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_multivalue_branch_n1:candidate": {
        "batch": 12508,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 36503559.835999995,
          "median": 27968963.688,
          "min": 25106057.6,
          "p95": 31851059.14
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_multivalue_branch_n32:baseline": {
        "batch": 670,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 71741811.1,
          "median": 32245276.595,
          "min": 29876743.849999998,
          "p95": 40085966.67
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_multivalue_branch_n32:candidate": {
        "batch": 670,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 44519163.04,
          "median": 31884256.14,
          "min": 28313452.279999997,
          "p95": 37275149.84
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_ordinary_recursion_n0:baseline": {
        "batch": 23365,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 43452755.005,
          "median": 23415071.195,
          "min": 19756299.115000002,
          "p95": 32403142.76
        },
        "observations": 30,
        "observed_under_20ms_count": 2
      },
      "1:scalar_ordinary_recursion_n0:candidate": {
        "batch": 23365,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 32401764.225,
          "median": 21549130.612499997,
          "min": 19322083.955,
          "p95": 27670421.82
        },
        "observations": 30,
        "observed_under_20ms_count": 5
      },
      "1:scalar_ordinary_recursion_n17:baseline": {
        "batch": 1971,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 75017056.28400001,
          "median": 39603585.8385,
          "min": 32635486.871999998,
          "p95": 49682307.150000006
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_ordinary_recursion_n17:candidate": {
        "batch": 1971,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 39838941.963,
          "median": 33162965.891999997,
          "min": 27895880.331,
          "p95": 39454291.458000004
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_ordinary_recursion_n1:baseline": {
        "batch": 15480,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 160323140.16,
          "median": 31061486.880000003,
          "min": 23642526.6,
          "p95": 53136261.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_ordinary_recursion_n1:candidate": {
        "batch": 15480,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 381781599.12,
          "median": 26965169.28,
          "min": 23759354.16,
          "p95": 83235820.68
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_ordinary_recursion_n32:baseline": {
        "batch": 895,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 167133648.76500002,
          "median": 31966369.9775,
          "min": 25656276.745,
          "p95": 66934052.644999996
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_ordinary_recursion_n32:candidate": {
        "batch": 895,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 175346383.565,
          "median": 30618116.0225,
          "min": 23475847.314999998,
          "p95": 110683766.635
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_table_mutation_n0:baseline": {
        "batch": 27134,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 41938039.059999995,
          "median": 24376398.714,
          "min": 21474173.208,
          "p95": 36941367.228
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_table_mutation_n0:candidate": {
        "batch": 27134,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 77535567.80399999,
          "median": 24646124.240999997,
          "min": 21193173.504,
          "p95": 45549113.182000004
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_table_mutation_n10000:baseline": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 39175975.0,
          "median": 23096981.0,
          "min": 18758860.0,
          "p95": 33919954.0
        },
        "observations": 30,
        "observed_under_20ms_count": 2
      },
      "1:scalar_table_mutation_n10000:candidate": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 35843411.0,
          "median": 21795452.5,
          "min": 17804847.0,
          "p95": 28470841.0
        },
        "observations": 30,
        "observed_under_20ms_count": 4
      },
      "1:scalar_table_mutation_n1024:baseline": {
        "batch": 22,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 31085648.99,
          "median": 25234779.493,
          "min": 22214667.992000002,
          "p95": 30436586.994
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_table_mutation_n1024:candidate": {
        "batch": 22,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 38210651.006,
          "median": 24588025.0,
          "min": 21984252.994,
          "p95": 31341838.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_table_mutation_n17:baseline": {
        "batch": 1711,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 54192514.977000006,
          "median": 33841726.986999996,
          "min": 29320914.232,
          "p95": 50178897.374
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_table_mutation_n17:candidate": {
        "batch": 1711,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 67703197.203,
          "median": 34533438.9455,
          "min": 29479137.246000003,
          "p95": 64901819.677999996
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_table_mutation_n1:baseline": {
        "batch": 14870,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 74246281.75,
          "median": 37021050.89,
          "min": 28119393.05,
          "p95": 66838166.70999999
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_table_mutation_n1:candidate": {
        "batch": 14870,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 76503369.31,
          "median": 35117757.805,
          "min": 28012685.93,
          "p95": 54528052.08
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_table_mutation_n32:baseline": {
        "batch": 961,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 69114924.91700001,
          "median": 37835336.39749999,
          "min": 31808239.905000005,
          "p95": 50618622.304
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_table_mutation_n32:candidate": {
        "batch": 961,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 88193349.43599999,
          "median": 36750833.4825,
          "min": 31194583.744999997,
          "p95": 56757456.669
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_tail_calls_n0:baseline": {
        "batch": 27277,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 28642404.789,
          "median": 23583666.923,
          "min": 20175842.205,
          "p95": 27283300.987
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_tail_calls_n0:candidate": {
        "batch": 27277,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 38417226.847,
          "median": 23719601.8525,
          "min": 21368883.631,
          "p95": 29128099.051000003
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_tail_calls_n10000:baseline": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 65505203.001,
          "median": 22504549.9995,
          "min": 18271578.0,
          "p95": 42714401.001
        },
        "observations": 30,
        "observed_under_20ms_count": 7
      },
      "1:scalar_tail_calls_n10000:candidate": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 39486816.0,
          "median": 21591632.498999998,
          "min": 18880746.998999998,
          "p95": 39141449.001
        },
        "observations": 30,
        "observed_under_20ms_count": 1
      },
      "1:scalar_tail_calls_n1024:baseline": {
        "batch": 37,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 44439731.007,
          "median": 29152374.4995,
          "min": 24574430.008,
          "p95": 36537727.994
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_tail_calls_n1024:candidate": {
        "batch": 37,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 50855306.01,
          "median": 30546468.0105,
          "min": 25502498.011,
          "p95": 46309617.989
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_tail_calls_n17:baseline": {
        "batch": 2369,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 61338479.66,
          "median": 32587208.288999997,
          "min": 28212257.539,
          "p95": 50508250.286
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_tail_calls_n17:candidate": {
        "batch": 2369,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 41852732.114999995,
          "median": 31650553.069,
          "min": 26985551.435,
          "p95": 40339767.896000005
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_tail_calls_n1:baseline": {
        "batch": 14971,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 34314460.202,
          "median": 24413613.917000003,
          "min": 20712827.63,
          "p95": 32363948.496
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_tail_calls_n1:candidate": {
        "batch": 14971,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 76423856.00299999,
          "median": 24071242.118,
          "min": 20736900.998,
          "p95": 42294931.404
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_tail_calls_n32:baseline": {
        "batch": 1295,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 59915368.730000004,
          "median": 32935978.46,
          "min": 26471954.880000003,
          "p95": 52076593.870000005
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_tail_calls_n32:candidate": {
        "batch": 1295,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 62096871.34,
          "median": 31962763.2975,
          "min": 28035284.06,
          "p95": 48407563.61
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_try_table_normal_n0:baseline": {
        "batch": 29851,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 41213962.256,
          "median": 23283108.3525,
          "min": 19410373.941999998,
          "p95": 34811788.434999995
        },
        "observations": 30,
        "observed_under_20ms_count": 1
      },
      "1:scalar_try_table_normal_n0:candidate": {
        "batch": 29851,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 85717027.096,
          "median": 25932041.316,
          "min": 20548801.529,
          "p95": 44205659.327
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_try_table_normal_n10000:baseline": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 41902730.0,
          "median": 17453146.5,
          "min": 14852797.0,
          "p95": 39438726.0
        },
        "observations": 30,
        "observed_under_20ms_count": 20
      },
      "1:scalar_try_table_normal_n10000:candidate": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 39108734.0,
          "median": 16829132.0,
          "min": 14394299.0,
          "p95": 31591051.0
        },
        "observations": 30,
        "observed_under_20ms_count": 25
      },
      "1:scalar_try_table_normal_n1024:baseline": {
        "batch": 25,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 36157375.0,
          "median": 22750407.0,
          "min": 19403016.0,
          "p95": 33419336.0
        },
        "observations": 30,
        "observed_under_20ms_count": 1
      },
      "1:scalar_try_table_normal_n1024:candidate": {
        "batch": 25,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 73597670.0,
          "median": 22128854.0,
          "min": 19044677.0,
          "p95": 45409401.0
        },
        "observations": 30,
        "observed_under_20ms_count": 1
      },
      "1:scalar_try_table_normal_n17:baseline": {
        "batch": 2051,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 108104801.238,
          "median": 35798412.426,
          "min": 32487329.301,
          "p95": 72912861.30800001
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_try_table_normal_n17:candidate": {
        "batch": 2051,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 75549930.456,
          "median": 35890185.4465,
          "min": 30438530.024,
          "p95": 56023286.508
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_try_table_normal_n1:baseline": {
        "batch": 17362,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 48706990.198,
          "median": 29487125.983000003,
          "min": 26787708.266,
          "p95": 40547996.09
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_try_table_normal_n1:candidate": {
        "batch": 17362,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 65125348.136,
          "median": 30272843.292999998,
          "min": 26740136.386,
          "p95": 56566507.168
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_try_table_normal_n32:baseline": {
        "batch": 1080,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 82917947.16,
          "median": 34263396.900000006,
          "min": 29199961.080000002,
          "p95": 55964613.96
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_try_table_normal_n32:candidate": {
        "batch": 1080,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 51043481.64,
          "median": 32351718.060000002,
          "min": 28746391.32,
          "p95": 42185335.68
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      }
    },
    "workload_count": 85
  },
  "outcome": "rejected",
  "raw_gate_decision": "rejected",
  "reason": "P031 comparison gate rejected the candidate",
  "schema": "wtime-p031-completion-v1",
  "status": "done",
  "strict_build_gate_passed": true,
  "task": "P031",
  "task_index_update_performed": false,
  "development_provenance": {
    "path": "/workspace/wtime/target/performance/P031/development-provenance.json",
    "sha256": "6f86bbdc1eb0d0bcd4c7a52129199f801bb7112662ccd122ba8805addba7743e",
    "bytes": 7184
  },
  "development_failures": [
    {
      "kind": "preflight checker guard/parser failures",
      "actual_helper_exit_codes": [
        1,
        1,
        0
      ],
      "record": {
        "path": "/workspace/wtime/target/performance/P031-preflight/preflight-attempts.json",
        "sha256": "c9e013de44e3f62f7e6071cded7d01971f84f8069607ec86b5864c77686b98cf",
        "bytes": 1332
      },
      "correction": "Corrected the P030 verified_file schema, then revalidated existing successful Rust/Python logs with a robust unittest parser and executed only remaining suites."
    },
    {
      "kind": "candidate compilation failure",
      "actual_exit_code": 101,
      "executed_test_count": 0,
      "record": {
        "path": "/workspace/wtime/target/performance/P031-candidate-checks/commands.json",
        "sha256": "544dd7ff0e9c9157ff2c2bd87c6aa646463ae771df591758107398a395bd48a7",
        "bytes": 2380
      },
      "correction": "Added the missing execution::Frame import to the new private activation module, then completed 13/14 focused, 221 full-default and 80 diagnostic-performance checks before the single quiet candidate budget sample. No quiet build sample was retried."
    },
    {
      "kind": "temporary extractor staging overwrite",
      "synthetic_failure_log": {
        "path": "/workspace/wtime/target/performance/P031/tooling-provenance/test_extract_p031-before-repair.log",
        "sha256": "a1f86328d0e7e3d369e5456540b639704c49478cc76236b2a2f39fc8e171e83f",
        "bytes": 4764
      },
      "synthetic_recovery_log": {
        "path": "/workspace/wtime/target/performance/P031/tooling-provenance/test_extract_p031-after-repair.log",
        "sha256": "03e2ce7efb78ca7222db4c9a68781f9953dd3bd674b491cc52a8a4986d74e6f4",
        "bytes": 102
      },
      "correction": "An agent overwrote a reviewed /tmp draft. The overwritten draft was not used on real data. The recovery restored tested calibration and legacy observation APIs; root added metadata checks before the actual extraction passed. The original reviewed draft could not be recovered byte-for-byte; reconstructed material checks and the actual executed extractor are retained."
    }
  ]
}
```
