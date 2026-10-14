# P024 — Move branch results and share label metadata without allocation

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

[P023 — Fetch ordinary instructions without cloning their payload](P023-borrow-instructions.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C2, C3, C4, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/execution.rs`
- `crates/wtime-runtime/src/continuation.rs`
- `crates/wtime-runtime/tests/performance/p024.rs`

- `crates/wtime-runtime/src/reference.rs`
- `crates/wtime-runtime/tests/fixtures/performance/branch`
- `docs/benchmarks/performance/P024-build-decision.md`
- `docs/benchmarks/performance/P024-contract.json`
- `tools/wtime-bench/fixtures/scalar/try_table_normal.wasm`
- `tools/wtime-bench/fixtures/scalar/try_table_normal.wat`
- `tools/wtime-bench/scalar_workloads.py`
- `tools/wtime-bench/tests/test_p024_scalar_model.py`
- `tools/wtime-bench/workloads.json`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

keep uses validated checked ranges plus copy_within/truncate; labels retain immutable catch ranges or shared slices. A branch does not allocate a temporary tail Vec or clone catches.

## Implementation recipe

1. Preserve keep's underflow/height checks, move the last n Copy values to height in place and truncate to height+n with checked arithmetic.
2. Reserve label capacity from FunctionPlan and replace owned cloned catch vectors with ranges/shared metadata that survive exceptions and captured frames.
3. Update unwind/capture/accounting consumers for shared catches; preserve loop parameter versus block result behavior and multi-value overlap.
4. Benchmark branch-heavy and exception-normal-path controls, retaining diagnostic proof that branch tail/catch allocations disappeared.
5. Record the changed path with focused assertions/diagnostic counts and exact source/binary identities; compare the pre-edit archive, and revert a rejected structural optimization rather than adding a permanent toggle solely for testing.

## Meaningful cases

- Zero/multiple results and overlapping tail moves preserve exact operand order.
- Nested loop parameters and br_table choose the same label/result height.
- Captured/cross-call try_table catches still match and unwind correctly.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime p024_ -- --nocapture
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml -j 4
python3 tools/wtime-bench/compare.py --baseline target/performance/P024-before --candidate tools/wtime-bench/target/release/wtime-bench --candidate-mode decoded --selection scalar --pairs 30 --build-growth-explanation-file docs/benchmarks/performance/P024-build-decision.md --output target/performance/P024-final
python3 tools/wtime-bench/analyze.py --input target/performance/P024-final --output target/performance/P024-final/analysis.json
python3 tools/checks/build_budget.py --output target/performance/P024-final/build.json
python3 tools/wtime-bench/gates.py --task P024 --comparison target/performance/P024-final --build target/performance/P024-final/build.json --contract docs/benchmarks/performance/P024-contract.json --output target/performance/P024-final/decision.json
```

## Measurement and decision

Selection: `scalar`. Explicit execution setting: `decoded`. Decision rule: `measured_promotion`.

Use the paired protocol and family/control/cold/build rules in [MEASUREMENT.md](../MEASUREMENT.md). Predeclare the targeted family and held-out controls before changing the implementation. Keep instrumentation off in production timings. Adopt only with supported gains in both runs; otherwise record rejected/inconclusive and remove default-path cost. A necessary immutable-interface change may be contract_only with its separate justification.

## Before editing

Run these commands before changing the candidate sources. The archive is immutable and captures the immediate predecessor. Retain P001's original archive for milestone totals. For native work, use the named portable control and verify its actual policy/quantum; if this archive already exists with different sources, choose a new recorded run-specific directory and update the comparison path rather than overwriting it.

```sh
python3 tools/wtime-bench/baseline.py --output target/performance/P024-before
```

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. Executed evidence is recorded below.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/crates/wtime-runtime/tests/fixtures/performance/branch/captured_try_table_four.wasm": {
      "sha256": "fddf2a13804c5ee205923f45171972456c10bdb2fd5ec7e211e9b14fbc2cec1e",
      "bytes": 186
    },
    "/workspace/wtime/crates/wtime-runtime/tests/fixtures/performance/branch/captured_try_table_four.wat": {
      "sha256": "f6e6e2d1cd6728b458c2d6df81dc3253545921232811810c59ba8e705c46f7b3",
      "bytes": 965
    },
    "/workspace/wtime/crates/wtime-runtime/tests/fixtures/performance/branch/captured_try_table_one.wasm": {
      "sha256": "4a4cd0068ca6344f5fcea882496a09e0eccb7cbf8561eb29717bb50eb00a9ea0",
      "bytes": 147
    },
    "/workspace/wtime/crates/wtime-runtime/tests/fixtures/performance/branch/captured_try_table_one.wat": {
      "sha256": "b7beaebcf74f3a214b03bb68720d3e46bd1923da88d47564fa9b1b1f8f606b56",
      "bytes": 717
    },
    "/workspace/wtime/crates/wtime-runtime/tests/fixtures/performance/branch/loop_params_br_table.wasm": {
      "sha256": "ab5d339be99a2a64aa3ce3b309a4728d1ba258b367b5c144b892b5c34db16b01",
      "bytes": 239
    },
    "/workspace/wtime/crates/wtime-runtime/tests/fixtures/performance/branch/loop_params_br_table.wat": {
      "sha256": "cefb2097c391e921e71e9e4fecf184fb4082eefa75e4186ab77c31566a83f014",
      "bytes": 1507
    },
    "/workspace/wtime/crates/wtime-runtime/tests/fixtures/performance/branch/nested_catch_order.wasm": {
      "sha256": "09538c065ecb3a7ac2fb63b531c496e8d5b59cec01fa0b563d51d5e2372f062f",
      "bytes": 195
    },
    "/workspace/wtime/crates/wtime-runtime/tests/fixtures/performance/branch/nested_catch_order.wat": {
      "sha256": "a663ac2132d84d21f479dba1a2216c15bb0bb8fb7491fb8a0f775c47fc0361da",
      "bytes": 894
    },
    "/workspace/wtime/crates/wtime-runtime/tests/fixtures/performance/branch/overlap_multivalue.wasm": {
      "sha256": "2de7943e8836c27aa89c37bdf04b7ad53367dd18486b994fc00c8b36f0e76634",
      "bytes": 73
    },
    "/workspace/wtime/crates/wtime-runtime/tests/fixtures/performance/branch/overlap_multivalue.wat": {
      "sha256": "0b8c3bc6b00123e5613566cacb1028db1b7bbb1f953370ea914fff95adf22db0",
      "bytes": 469
    },
    "/workspace/wtime/crates/wtime-runtime/tests/fixtures/performance/branch/try_table_depth_1.wasm": {
      "sha256": "4a4cd0068ca6344f5fcea882496a09e0eccb7cbf8561eb29717bb50eb00a9ea0",
      "bytes": 147
    },
    "/workspace/wtime/crates/wtime-runtime/tests/fixtures/performance/branch/try_table_depth_1.wat": {
      "sha256": "f55be641c2939425635b4e2ba26fe4443fc15758d78a01e4fd3cedeeba49b655",
      "bytes": 704
    },
    "/workspace/wtime/crates/wtime-runtime/tests/fixtures/performance/branch/try_table_depth_4.wasm": {
      "sha256": "977de88fd7a5e5fc04bb88aef8819583174551ab51f33aafd83faa5771407ced",
      "bytes": 168
    },
    "/workspace/wtime/crates/wtime-runtime/tests/fixtures/performance/branch/try_table_depth_4.wat": {
      "sha256": "6dc6def6b0683e6747bcee21908bc68db9889b78cc147fb4f527a93bdb0664c4",
      "bytes": 890
    },
    "/workspace/wtime/crates/wtime-runtime/tests/fixtures/performance/branch/try_table_depth_5.wasm": {
      "sha256": "98e2f2e1b78018ac86823531d00386a90db23ee561e0b7fb581307564679f6fe",
      "bytes": 175
    },
    "/workspace/wtime/crates/wtime-runtime/tests/fixtures/performance/branch/try_table_depth_5.wat": {
      "sha256": "04ac0b39513656df48c45736f444936fd52b5098be30eb85985e70c644d2aad9",
      "bytes": 960
    },
    "/workspace/wtime/crates/wtime-runtime/tests/fixtures/performance/branch/try_table_normal.wasm": {
      "sha256": "97068a6310a3bc535369e2b6277b801875c14ca206a707fcb9bca9036f41d0df",
      "bytes": 146
    },
    "/workspace/wtime/crates/wtime-runtime/tests/fixtures/performance/branch/try_table_normal.wat": {
      "sha256": "ab92800dd4e4d4053498c317effadedf52568270ab7b476eef74b09638de8964",
      "bytes": 791
    },
    "/workspace/wtime/docs/benchmarks/performance/P024-build-decision.md": {
      "sha256": "105afb3b5c6ecabf0d465ea3bf8fa9fc9be5a39f6ef5eda28658fd952247a174",
      "bytes": 2168
    },
    "/workspace/wtime/docs/benchmarks/performance/P024-candidate.patch": {
      "sha256": "f9aba931ca7e1093f240b3eebdf79ab5c907dd13123407eb91f68b5b086aa503",
      "bytes": 29945
    },
    "/workspace/wtime/docs/benchmarks/performance/P024-contract.json": {
      "sha256": "7b6a180c9728f2bd030b13b418174b56f9044b66c0e431cfcaccba865e507572",
      "bytes": 2757
    },
    "/workspace/wtime/target/performance/P024-analysis.log": {
      "sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
      "bytes": 0
    },
    "/workspace/wtime/target/performance/P024-baseline.log": {
      "sha256": "57b3df3eba4faf05105b3505b4bd67fd72a4c768c8ebb050ff312d6b0b5b6de5",
      "bytes": 243
    },
    "/workspace/wtime/target/performance/P024-before-build.json": {
      "sha256": "b288148825490865c331a383f53d253729e08a6e201d487ef60d1b63ef3432e3",
      "bytes": 80379
    },
    "/workspace/wtime/target/performance/P024-before-build.log": {
      "sha256": "263071eafbbe034f551ad830d5b96001d7fe8225fd70566b464276e97eb0ce86",
      "bytes": 1703
    },
    "/workspace/wtime/target/performance/P024-before-diagnostic-tests.log": {
      "sha256": "24a844f63ce75ed7c7c203bbbd42e30128efeab12b1036df5a8dae6935c2f661",
      "bytes": 1925
    },
    "/workspace/wtime/target/performance/P024-before-tests.log": {
      "sha256": "ffefd4db4ebe9aac7d50ede48d7b94e9c876b14eb834c1496091ac036030183f",
      "bytes": 1855
    },
    "/workspace/wtime/target/performance/P024-before/binary/wtime-bench": {
      "sha256": "77132ce9531de66a1c8a14e2c43946eeee9f80951de11b18c55c7d4fcd920398",
      "bytes": 17264360
    },
    "/workspace/wtime/target/performance/P024-before/manifest.json": {
      "sha256": "d84d516d2e894245599bb7b0a2be478cc597ccd9c3d1e48c526153362fc1f8ec",
      "bytes": 35669
    },
    "/workspace/wtime/target/performance/P024-candidate-bench-build.log": {
      "sha256": "bebed7b08a4ba0c2e9370d83c80266ad5abb989cb2e102d928d51e4b42195631",
      "bytes": 206
    },
    "/workspace/wtime/target/performance/P024-candidate-build-budget.log": {
      "sha256": "5c48b184525db37ce46e8f1f4e3c1cdbd83a852275c290591ba7a624ed997026",
      "bytes": 1703
    },
    "/workspace/wtime/target/performance/P024-candidate-diagnostic-tests.log": {
      "sha256": "77512c2a8ddca1160bfd09bd904573f338345a520bb671d4028a00a80dd8e3ed",
      "bytes": 3094
    },
    "/workspace/wtime/target/performance/P024-candidate-tests.log": {
      "sha256": "feb8e18b0b06a90d84a9a870eca87024ac3595f49d28c4771ecc70333d90ec17",
      "bytes": 2492
    },
    "/workspace/wtime/target/performance/P024-final/analysis.json": {
      "sha256": "cf47972c5f9bba8affe928a8b61a536d5a6a516ec357d1fad6a36c23bb23b6f7",
      "bytes": 1262034
    },
    "/workspace/wtime/target/performance/P024-final/build.json": {
      "sha256": "15ff1d3b152b0282fed39630f3f4507c1a37ef1d7345783591b281284e80fa19",
      "bytes": 80379
    },
    "/workspace/wtime/target/performance/P024-final/checkpoint.jsonl": {
      "sha256": "05575811134ec6e371b959cb9a3c0ed3dcd1966642fc0e95a486cc21e4994090",
      "bytes": 36214176
    },
    "/workspace/wtime/target/performance/P024-final/decision.json": {
      "sha256": "f5c85fe24c66d6c3373e0811fe6363c2919e77094cc8fba30b0044026b1bb27d",
      "bytes": 1680470
    },
    "/workspace/wtime/target/performance/P024-final/extract_completed.py": {
      "sha256": "8863702d2e42527c7ff8cc879d1c5a0c0fbba6ab37cafd13ad8d633375dbb6cf",
      "bytes": 8465
    },
    "/workspace/wtime/target/performance/P024-final/extraction.json": {
      "sha256": "fe549f69a966821f9239566fc02237b3fde794df57543f444046ec24fdcfd43c",
      "bytes": 1986
    },
    "/workspace/wtime/target/performance/P024-final/gate-environment-failure.json": {
      "sha256": "a186fdd17189aec704512f89e308d836dd4f7a6f4d89182c4671625bac4c418d",
      "bytes": 129
    },
    "/workspace/wtime/target/performance/P024-final/promotion-decision.json": {
      "sha256": "9edbe2ceb01f95423bcac61fa0ef97a88cb22c7f7a8f6fd993b6b055de8e6950",
      "bytes": 563
    },
    "/workspace/wtime/target/performance/P024-final/report.json": {
      "sha256": "3bc6c7f58bd56ec74437fcb78b2eaa83e034a6cf3a6f5c4e297013a0fbe9f132",
      "bytes": 59829463
    },
    "/workspace/wtime/target/performance/P024-gate-environment-failure.exit": {
      "sha256": "53c234e5e8472b6ac51c1ae1cab3fe06fad053beb8ebfd8977b010655bfdd3c3",
      "bytes": 2
    },
    "/workspace/wtime/target/performance/P024-gate-environment-failure.log": {
      "sha256": "f3ed7c4c7990d7036220894c91d786af366aba1119ec5637010729ca7c2bb00c",
      "bytes": 55
    },
    "/workspace/wtime/target/performance/P024-gate.exit": {
      "sha256": "4355a46b19d348dc2f57c046f8ef63d4538ebb936000f3c9ee954a27460dd865",
      "bytes": 2
    },
    "/workspace/wtime/target/performance/P024-gate.log": {
      "sha256": "f5c85fe24c66d6c3373e0811fe6363c2919e77094cc8fba30b0044026b1bb27d",
      "bytes": 1680470
    },
    "/workspace/wtime/target/performance/P024-incremental-build-decision.json": {
      "sha256": "0f5c028bead015f4d97769b37eea6adfb4a2e485cd5c71cc30ce0f5e9dee0bf5",
      "bytes": 1215
    },
    "/workspace/wtime/target/performance/P024-model-tests-final.log": {
      "sha256": "f74c2b6ff61e8205e74ae248baa6923e2600a61c0892baec4d41f9d407163404",
      "bytes": 754
    },
    "/workspace/wtime/target/performance/P024-original-baseline-verify.log": {
      "sha256": "4dadaaf12b589ccc3a3d5dda0650d9667b86fb3293f64d113d2ed0fd7166dab7",
      "bytes": 108
    },
    "/workspace/wtime/target/performance/P024-performance-tests.log": {
      "sha256": "4d62bb35204cf30c813585732e0bebf8156e8788f787d0efc1026a871dd49f24",
      "bytes": 3709
    },
    "/workspace/wtime/target/performance/P024-plan-check-fixed.log": {
      "sha256": "beda6715764d45c41bfd80e6f4df78465bd43929e07e75d8aa5e2623bd6930ce",
      "bytes": 78
    },
    "/workspace/wtime/target/performance/P024-rollback-bench-build.log": {
      "sha256": "caa288a9da68362724f10e3d4267594124846e942ca8341bf48f39bbef359d19",
      "bytes": 206
    },
    "/workspace/wtime/target/performance/P024-rollback-build.json": {
      "sha256": "3e36d33491af6d102f049eb56336cd7423c1e4dba649355e5d3f772eed736ac3",
      "bytes": 80379
    },
    "/workspace/wtime/target/performance/P024-rollback-build.log": {
      "sha256": "a06a124aefb6f93b023fcd044d640fd093fb5a5ddd54ef1ddb782c84450ac412",
      "bytes": 1703
    },
    "/workspace/wtime/target/performance/P024-rollback-identities.json": {
      "sha256": "eb1718539d063587305551cd3fa362de8619e97a929653f616bde012275de24c",
      "bytes": 453
    },
    "/workspace/wtime/target/performance/P024-rollback-performance-tests.log": {
      "sha256": "81a29a565d5d132eb20366ca8376e0cbf7439dcd9a6e0745145ba816896c0a75",
      "bytes": 3302
    },
    "/workspace/wtime/target/performance/P024-rollback-runtime-tests.log": {
      "sha256": "fba96662fadb9640aa0e8e27a9bca3dbfda08721010a60dab65b635dd3fd2f5f",
      "bytes": 13178
    },
    "/workspace/wtime/target/performance/P024-run.log": {
      "sha256": "701f5ff410b70ce425560435ce8f08c0fea61a6d3df725caecba4353b50a56fb",
      "bytes": 85
    },
    "/workspace/wtime/target/performance/P024-runtime-tests.log": {
      "sha256": "a76d2fcd9d4d89d8c770c0a6b63961c680859a3063358a6090927677e89533ed",
      "bytes": 13579
    },
    "/workspace/wtime/target/performance/P024/build.json": {
      "sha256": "15ff1d3b152b0282fed39630f3f4507c1a37ef1d7345783591b281284e80fa19",
      "bytes": 80379
    },
    "/workspace/wtime/target/performance/P024/candidate-retained/crates/wtime-runtime/src/continuation.rs": {
      "sha256": "f69c75c3ef274fe771911738129a18eee6251963b88c6c4bcfaddf1614d8272b",
      "bytes": 58755
    },
    "/workspace/wtime/target/performance/P024/candidate-retained/crates/wtime-runtime/src/execution.rs": {
      "sha256": "173770414722b60dfe1e8cc2a0441c14b8662aeb453b91790128e6fbb63c0ddd",
      "bytes": 58022
    },
    "/workspace/wtime/target/performance/P024/candidate-retained/crates/wtime-runtime/src/reference.rs": {
      "sha256": "b640d438d1f0616e1e264db7fc8a7a664b0b4ebba056d93b6ee440f8d0ec8a11",
      "bytes": 29772
    },
    "/workspace/wtime/target/performance/P024/candidate-retained/crates/wtime-runtime/tests/performance/p024.rs": {
      "sha256": "ffc3f81e2884148c08b5fd299ee7194757eb73b574e3b4a320e7d0126b5b44c8",
      "bytes": 8547
    },
    "/workspace/wtime/target/performance/P024/candidate-retained/source-manifest.json": {
      "sha256": "ebb91feee000ffdd9d59a9cd4894e72d5a42a225b492e180279959080e050124",
      "bytes": 14666
    },
    "/workspace/wtime/target/performance/P024/candidate-retained/source/tools/wtime-bench/fixtures/scalar/br_table.wat": {
      "sha256": "637b96cd8926ffadf728af4949b43fa6b62fffb61968c14c716e0c203a1e5e6d",
      "bytes": 1088
    },
    "/workspace/wtime/target/performance/P024/candidate-retained/source/tools/wtime-bench/fixtures/scalar/multivalue_branch.wat": {
      "sha256": "a1f4958100af82889208bb28090d6af7c3e56cbeeccabad4106601c9a9c17014",
      "bytes": 881
    },
    "/workspace/wtime/target/performance/P024/candidate-retained/source/tools/wtime-bench/fixtures/scalar/try_table_normal.wat": {
      "sha256": "ab92800dd4e4d4053498c317effadedf52568270ab7b476eef74b09638de8964",
      "bytes": 791
    },
    "/workspace/wtime/target/performance/P024/candidate-retained/source/tools/wtime-bench/scalar_workloads.py": {
      "sha256": "b564e862913ed3fbd9a54bec0c722962fe999c25024375de69c1ceac03d2682e",
      "bytes": 11200
    },
    "/workspace/wtime/target/performance/P024/candidate-retained/source/tools/wtime-bench/workloads.json": {
      "sha256": "e7941449302a744de2ad5a1a96c42ac1be9631c7809d673f93b44027b5cc0cf9",
      "bytes": 454808
    },
    "/workspace/wtime/target/performance/P024/candidate-retained/tools/wtime-bench/target/release/wtime-bench": {
      "sha256": "5a38d8ca422bd62b29aed9a557f5f9eccf0d1882625b4d915698f17e2ffdca34",
      "bytes": 17261136
    },
    "/workspace/wtime/tools/wtime-bench/fixtures/scalar/try_table_normal.wasm": {
      "sha256": "97068a6310a3bc535369e2b6277b801875c14ca206a707fcb9bca9036f41d0df",
      "bytes": 146
    },
    "/workspace/wtime/docs/benchmarks/performance/P024-evidence.json": {
      "sha256": "740e1bb7e498f31b3aa8f6b1ec674df753d92f86d9d6310319fb77f402b952d8",
      "bytes": 37581
    }
  },
  "artifacts": [
    "/workspace/wtime/crates/wtime-runtime/tests/fixtures/performance/branch/captured_try_table_four.wasm",
    "/workspace/wtime/crates/wtime-runtime/tests/fixtures/performance/branch/captured_try_table_four.wat",
    "/workspace/wtime/crates/wtime-runtime/tests/fixtures/performance/branch/captured_try_table_one.wasm",
    "/workspace/wtime/crates/wtime-runtime/tests/fixtures/performance/branch/captured_try_table_one.wat",
    "/workspace/wtime/crates/wtime-runtime/tests/fixtures/performance/branch/loop_params_br_table.wasm",
    "/workspace/wtime/crates/wtime-runtime/tests/fixtures/performance/branch/loop_params_br_table.wat",
    "/workspace/wtime/crates/wtime-runtime/tests/fixtures/performance/branch/nested_catch_order.wasm",
    "/workspace/wtime/crates/wtime-runtime/tests/fixtures/performance/branch/nested_catch_order.wat",
    "/workspace/wtime/crates/wtime-runtime/tests/fixtures/performance/branch/overlap_multivalue.wasm",
    "/workspace/wtime/crates/wtime-runtime/tests/fixtures/performance/branch/overlap_multivalue.wat",
    "/workspace/wtime/crates/wtime-runtime/tests/fixtures/performance/branch/try_table_depth_1.wasm",
    "/workspace/wtime/crates/wtime-runtime/tests/fixtures/performance/branch/try_table_depth_1.wat",
    "/workspace/wtime/crates/wtime-runtime/tests/fixtures/performance/branch/try_table_depth_4.wasm",
    "/workspace/wtime/crates/wtime-runtime/tests/fixtures/performance/branch/try_table_depth_4.wat",
    "/workspace/wtime/crates/wtime-runtime/tests/fixtures/performance/branch/try_table_depth_5.wasm",
    "/workspace/wtime/crates/wtime-runtime/tests/fixtures/performance/branch/try_table_depth_5.wat",
    "/workspace/wtime/crates/wtime-runtime/tests/fixtures/performance/branch/try_table_normal.wasm",
    "/workspace/wtime/crates/wtime-runtime/tests/fixtures/performance/branch/try_table_normal.wat",
    "/workspace/wtime/docs/benchmarks/performance/P024-build-decision.md",
    "/workspace/wtime/docs/benchmarks/performance/P024-candidate.patch",
    "/workspace/wtime/docs/benchmarks/performance/P024-contract.json",
    "/workspace/wtime/target/performance/P024-analysis.log",
    "/workspace/wtime/target/performance/P024-baseline.log",
    "/workspace/wtime/target/performance/P024-before-build.json",
    "/workspace/wtime/target/performance/P024-before-build.log",
    "/workspace/wtime/target/performance/P024-before-diagnostic-tests.log",
    "/workspace/wtime/target/performance/P024-before-tests.log",
    "/workspace/wtime/target/performance/P024-before/binary/wtime-bench",
    "/workspace/wtime/target/performance/P024-before/manifest.json",
    "/workspace/wtime/target/performance/P024-candidate-bench-build.log",
    "/workspace/wtime/target/performance/P024-candidate-build-budget.log",
    "/workspace/wtime/target/performance/P024-candidate-diagnostic-tests.log",
    "/workspace/wtime/target/performance/P024-candidate-tests.log",
    "/workspace/wtime/target/performance/P024-final/analysis.json",
    "/workspace/wtime/target/performance/P024-final/build.json",
    "/workspace/wtime/target/performance/P024-final/checkpoint.jsonl",
    "/workspace/wtime/target/performance/P024-final/decision.json",
    "/workspace/wtime/target/performance/P024-final/extract_completed.py",
    "/workspace/wtime/target/performance/P024-final/extraction.json",
    "/workspace/wtime/target/performance/P024-final/gate-environment-failure.json",
    "/workspace/wtime/target/performance/P024-final/promotion-decision.json",
    "/workspace/wtime/target/performance/P024-final/report.json",
    "/workspace/wtime/target/performance/P024-gate-environment-failure.exit",
    "/workspace/wtime/target/performance/P024-gate-environment-failure.log",
    "/workspace/wtime/target/performance/P024-gate.exit",
    "/workspace/wtime/target/performance/P024-gate.log",
    "/workspace/wtime/target/performance/P024-incremental-build-decision.json",
    "/workspace/wtime/target/performance/P024-model-tests-final.log",
    "/workspace/wtime/target/performance/P024-original-baseline-verify.log",
    "/workspace/wtime/target/performance/P024-performance-tests.log",
    "/workspace/wtime/target/performance/P024-plan-check-fixed.log",
    "/workspace/wtime/target/performance/P024-rollback-bench-build.log",
    "/workspace/wtime/target/performance/P024-rollback-build.json",
    "/workspace/wtime/target/performance/P024-rollback-build.log",
    "/workspace/wtime/target/performance/P024-rollback-identities.json",
    "/workspace/wtime/target/performance/P024-rollback-performance-tests.log",
    "/workspace/wtime/target/performance/P024-rollback-runtime-tests.log",
    "/workspace/wtime/target/performance/P024-run.log",
    "/workspace/wtime/target/performance/P024-runtime-tests.log",
    "/workspace/wtime/target/performance/P024/build.json",
    "/workspace/wtime/target/performance/P024/candidate-retained/crates/wtime-runtime/src/continuation.rs",
    "/workspace/wtime/target/performance/P024/candidate-retained/crates/wtime-runtime/src/execution.rs",
    "/workspace/wtime/target/performance/P024/candidate-retained/crates/wtime-runtime/src/reference.rs",
    "/workspace/wtime/target/performance/P024/candidate-retained/crates/wtime-runtime/tests/performance/p024.rs",
    "/workspace/wtime/target/performance/P024/candidate-retained/source-manifest.json",
    "/workspace/wtime/target/performance/P024/candidate-retained/source/tools/wtime-bench/fixtures/scalar/br_table.wat",
    "/workspace/wtime/target/performance/P024/candidate-retained/source/tools/wtime-bench/fixtures/scalar/multivalue_branch.wat",
    "/workspace/wtime/target/performance/P024/candidate-retained/source/tools/wtime-bench/fixtures/scalar/try_table_normal.wat",
    "/workspace/wtime/target/performance/P024/candidate-retained/source/tools/wtime-bench/scalar_workloads.py",
    "/workspace/wtime/target/performance/P024/candidate-retained/source/tools/wtime-bench/workloads.json",
    "/workspace/wtime/target/performance/P024/candidate-retained/tools/wtime-bench/target/release/wtime-bench",
    "/workspace/wtime/tools/wtime-bench/fixtures/scalar/try_table_normal.wasm",
    "/workspace/wtime/docs/benchmarks/performance/P024-evidence.json"
  ],
  "build_gate": {
    "before": 0.9914023730016197,
    "candidate": 1.6301838689978467,
    "candidate_must_reject_regardless_of_paired_timing": true,
    "decision_sha256": "0f5c028bead015f4d97769b37eea6adfb4a2e485cd5c71cc30ce0f5e9dee0bf5",
    "failed_metric": "runtime_leaf_seconds",
    "limit": 1.25,
    "other_five_metrics_passed": true,
    "passed": false,
    "ratio": 1.6443211287282076
  },
  "build_metrics": {
    "before": {
      "budget_pass": true,
      "clean_peak_rss_kib": 281144,
      "clean_seconds": 6.234115412997198,
      "inner_cargo_build_command": [
        "cargo",
        "build",
        "--locked",
        "--offline",
        "-j",
        "5"
      ],
      "leaf_seconds": 0.2983397710049758,
      "release_seconds": 13.810009526001522,
      "runtime_leaf_seconds": 0.9914023730016197,
      "source_stable": true,
      "stripped_bytes": 1000264
    },
    "candidate": {
      "budget_pass": true,
      "clean_peak_rss_kib": 279900,
      "clean_seconds": 6.209930525998061,
      "inner_cargo_build_command": [
        "cargo",
        "build",
        "--locked",
        "--offline",
        "-j",
        "5"
      ],
      "leaf_seconds": 0.34364878899941687,
      "release_seconds": 13.205649543997424,
      "runtime_leaf_seconds": 1.6301838689978467,
      "source_stable": true,
      "stripped_bytes": 998184
    },
    "default_dependency_closure_pass": true,
    "ratios": {
      "clean_peak_rss_kib": {
        "before": 281144,
        "candidate": 279900,
        "limit": 1.25,
        "pass": true,
        "ratio": 0.995575221238938
      },
      "clean_seconds": {
        "before": 6.234115412997198,
        "candidate": 6.209930525998061,
        "limit": 1.25,
        "pass": true,
        "ratio": 0.9961205583475861
      },
      "leaf_seconds": {
        "before": 0.2983397710049758,
        "candidate": 0.34364878899941687,
        "limit": 1.25,
        "pass": true,
        "ratio": 1.1518705261514912
      },
      "release_seconds": {
        "before": 13.810009526001522,
        "candidate": 13.205649543997424,
        "limit": 1.25,
        "pass": true,
        "ratio": 0.9562375405414306
      },
      "runtime_leaf_seconds": {
        "before": 0.9914023730016197,
        "candidate": 1.6301838689978467,
        "limit": 1.25,
        "pass": false,
        "ratio": 1.6443211287282076
      },
      "stripped_bytes": {
        "before": 1000264,
        "candidate": 998184,
        "limit": 1.25,
        "pass": true,
        "ratio": 0.9979205489750705
      }
    }
  },
  "commands": [
    {
      "command": "python3 tools/wtime-bench/baseline.py --output target/performance/P024-before",
      "evidence": "/workspace/wtime/target/performance/P024-baseline.log",
      "exit_code": 0,
      "result": "133 files; five workloads; correctness passed"
    },
    {
      "command": "python3 tools/checks/build_budget.py --output target/performance/P024-before-build.json",
      "evidence": "/workspace/wtime/target/performance/P024-before-build.json",
      "exit_code": 0,
      "note": "Top-level command that generated the report; the report records its measured inner cargo build command separately."
    },
    {
      "command": "python3 tools/checks/build_budget.py --output target/performance/P024/build.json",
      "evidence": "/workspace/wtime/target/performance/P024/build.json",
      "exit_code": 0,
      "note": "Top-level command that generated the report; the report records its measured inner cargo build command separately."
    },
    {
      "command": "cargo test --locked -p wtime-runtime p024_ -- --nocapture",
      "evidence": "/workspace/wtime/target/performance/P024-candidate-tests.log",
      "exit_code": 0,
      "failed": 0,
      "passed": 11,
      "skipped": 0
    },
    {
      "command": "cargo test --locked -p wtime-runtime --features execution-reference,performance-counters p024_ -- --nocapture",
      "evidence": "/workspace/wtime/target/performance/P024-candidate-diagnostic-tests.log",
      "exit_code": 0,
      "failed": 0,
      "passed": 12,
      "skipped": 0
    },
    {
      "command": "cargo test --locked -p wtime-runtime --features execution-reference,performance-counters --test performance",
      "evidence": "/workspace/wtime/target/performance/P024-performance-tests.log",
      "exit_code": 0,
      "failed": 0,
      "passed": 43,
      "skipped": 0
    },
    {
      "command": "cargo test --locked -p wtime-runtime",
      "evidence": "/workspace/wtime/target/performance/P024-runtime-tests.log",
      "exit_code": 0,
      "failed": 0,
      "passed": 183,
      "skipped": 0
    },
    {
      "command": "python3 tools/wtime-bench/tests/test_p024_scalar_model.py -v",
      "evidence": "/workspace/wtime/target/performance/P024-model-tests-final.log",
      "exit_code": 0,
      "failed": 0,
      "passed": 4,
      "skipped": 0
    },
    {
      "command": "python3 tools/wtime-bench/compare.py --baseline target/performance/P024-before --candidate tools/wtime-bench/target/release/wtime-bench --candidate-mode decoded --selection scalar --pairs 30 --build-growth-explanation-file docs/benchmarks/performance/P024-build-decision.md --output target/performance/P024-final",
      "exit_code": 0,
      "note": "25500 valid pairs; stable source and binary identities.",
      "status": "completed"
    },
    {
      "command": "python3 tools/wtime-bench/gates.py (same declared arguments, Rust environment missing)",
      "exit_code": 2,
      "error": "cargo unavailable in shell",
      "note": "Retained setup failure; rerun with activated Rust environment and unchanged measured sources."
    },
    {
      "command": "python3 tools/wtime-bench/analyze.py --input target/performance/P024-final --output target/performance/P024-final/analysis.json",
      "exit_code": 0
    },
    {
      "command": "python3 tools/wtime-bench/gates.py --task P024 --comparison target/performance/P024-final --build target/performance/P024-final/build.json --contract docs/benchmarks/performance/P024-contract.json --output target/performance/P024-final/decision.json",
      "exit_code": 1
    },
    {
      "command": "cargo test --locked -p wtime-runtime",
      "exit_code": 0,
      "passed": 179,
      "failed": 0,
      "skipped": 0
    },
    {
      "command": "cargo test --locked -p wtime-runtime --features execution-reference,performance-counters --test performance",
      "exit_code": 0,
      "passed": 43,
      "failed": 0,
      "skipped": 0
    },
    {
      "command": "cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml -j 4",
      "exit_code": 0,
      "note": "Restored production runtime benchmark binary."
    },
    {
      "command": "python3 tools/checks/build_budget.py --output target/performance/P024-rollback-build.json",
      "exit_code": 0
    }
  ],
  "fixture_hashes": {
    "crates/wtime-runtime/tests/fixtures/performance/branch/captured_try_table_four.wasm": "fddf2a13804c5ee205923f45171972456c10bdb2fd5ec7e211e9b14fbc2cec1e",
    "crates/wtime-runtime/tests/fixtures/performance/branch/captured_try_table_four.wat": "f6e6e2d1cd6728b458c2d6df81dc3253545921232811810c59ba8e705c46f7b3",
    "crates/wtime-runtime/tests/fixtures/performance/branch/captured_try_table_one.wasm": "4a4cd0068ca6344f5fcea882496a09e0eccb7cbf8561eb29717bb50eb00a9ea0",
    "crates/wtime-runtime/tests/fixtures/performance/branch/captured_try_table_one.wat": "b7beaebcf74f3a214b03bb68720d3e46bd1923da88d47564fa9b1b1f8f606b56",
    "crates/wtime-runtime/tests/fixtures/performance/branch/loop_params_br_table.wasm": "ab5d339be99a2a64aa3ce3b309a4728d1ba258b367b5c144b892b5c34db16b01",
    "crates/wtime-runtime/tests/fixtures/performance/branch/loop_params_br_table.wat": "cefb2097c391e921e71e9e4fecf184fb4082eefa75e4186ab77c31566a83f014",
    "crates/wtime-runtime/tests/fixtures/performance/branch/nested_catch_order.wasm": "09538c065ecb3a7ac2fb63b531c496e8d5b59cec01fa0b563d51d5e2372f062f",
    "crates/wtime-runtime/tests/fixtures/performance/branch/nested_catch_order.wat": "a663ac2132d84d21f479dba1a2216c15bb0bb8fb7491fb8a0f775c47fc0361da",
    "crates/wtime-runtime/tests/fixtures/performance/branch/overlap_multivalue.wasm": "2de7943e8836c27aa89c37bdf04b7ad53367dd18486b994fc00c8b36f0e76634",
    "crates/wtime-runtime/tests/fixtures/performance/branch/overlap_multivalue.wat": "0b8c3bc6b00123e5613566cacb1028db1b7bbb1f953370ea914fff95adf22db0",
    "crates/wtime-runtime/tests/fixtures/performance/branch/try_table_depth_1.wasm": "4a4cd0068ca6344f5fcea882496a09e0eccb7cbf8561eb29717bb50eb00a9ea0",
    "crates/wtime-runtime/tests/fixtures/performance/branch/try_table_depth_1.wat": "f55be641c2939425635b4e2ba26fe4443fc15758d78a01e4fd3cedeeba49b655",
    "crates/wtime-runtime/tests/fixtures/performance/branch/try_table_depth_4.wasm": "977de88fd7a5e5fc04bb88aef8819583174551ab51f33aafd83faa5771407ced",
    "crates/wtime-runtime/tests/fixtures/performance/branch/try_table_depth_4.wat": "6dc6def6b0683e6747bcee21908bc68db9889b78cc147fb4f527a93bdb0664c4",
    "crates/wtime-runtime/tests/fixtures/performance/branch/try_table_depth_5.wasm": "98e2f2e1b78018ac86823531d00386a90db23ee561e0b7fb581307564679f6fe",
    "crates/wtime-runtime/tests/fixtures/performance/branch/try_table_depth_5.wat": "04ac0b39513656df48c45736f444936fd52b5098be30eb85985e70c644d2aad9",
    "crates/wtime-runtime/tests/fixtures/performance/branch/try_table_normal.wasm": "97068a6310a3bc535369e2b6277b801875c14ca206a707fcb9bca9036f41d0df",
    "crates/wtime-runtime/tests/fixtures/performance/branch/try_table_normal.wat": "ab92800dd4e4d4053498c317effadedf52568270ab7b476eef74b09638de8964",
    "tools/wtime-bench/fixtures/scalar/br_table.wat": "637b96cd8926ffadf728af4949b43fa6b62fffb61968c14c716e0c203a1e5e6d",
    "tools/wtime-bench/fixtures/scalar/multivalue_branch.wat": "a1f4958100af82889208bb28090d6af7c3e56cbeeccabad4106601c9a9c17014",
    "tools/wtime-bench/fixtures/scalar/try_table_normal.wasm": "97068a6310a3bc535369e2b6277b801875c14ca206a707fcb9bca9036f41d0df",
    "tools/wtime-bench/fixtures/scalar/try_table_normal.wat": "ab92800dd4e4d4053498c317effadedf52568270ab7b476eef74b09638de8964"
  },
  "initial_setup_failure": {
    "measurement_data_counted": false,
    "message": "comparison failed: output already exists; refusing to overwrite report or checkpoint",
    "path": "/workspace/wtime/target/performance/P024-run.log",
    "sha256": "701f5ff410b70ce425560435ce8f08c0fea61a6d3df725caecba4353b50a56fb"
  },
  "limitations": [
    "No Wasmtime timing ratio is claimed.",
    "The predeclared runtime-leaf rebuild ratio1.6443211287282076 exceeds1.25; this rejects promotion regardless of timing.",
    "The first invocation failed at output-directory setup and produced no measurements; its log is retained.",
    "Timing and candidate-test results describe the retained candidate. Final default execution uses the restored predecessor."
  ],
  "outcome_note": "Candidate implemented and verified, then rejected by the predeclared incremental-build limit. Production runtime restored byte for byte; semantic fixtures and historical candidate evidence retained.",
  "paired_run": {
    "checkpoint_path": "/workspace/wtime/target/performance/P024-final/checkpoint.jsonl",
    "final_observation_count": 25500,
    "final_report_available": true,
    "gate_result": "rejected",
    "progress_log_path": "/workspace/wtime/target/performance/P024-run-final.log",
    "report_path": "/workspace/wtime/target/performance/P024-final/report.json",
    "rollback_status": "complete",
    "selection": "scalar",
    "status": "complete",
    "runs": 2,
    "groups": 850,
    "pairs_per_group": 30,
    "workloads": 85,
    "stages": 5,
    "correctness_probes": 0,
    "oracle_note": "Every observation validates the independent result; there is no separate correctness_probes list.",
    "source_stable": true,
    "instrumented": false,
    "gate_exit_code": 1,
    "primary_family": [
      {
        "bootstrap_resamples": 10000,
        "bootstrap_seed": 6032341737971213015,
        "ci95": {
          "lower": 0.9135379896417285,
          "upper": 0.9499025930973741
        },
        "input_fixture_groups": 12,
        "median_ratio": 0.9334248836620018,
        "required_median_max": 0.95,
        "required_upper_max_exclusive": 1.0,
        "run": 1,
        "workloads": 12
      },
      {
        "bootstrap_resamples": 10000,
        "bootstrap_seed": 3954944471358900215,
        "ci95": {
          "lower": 0.8992115347779975,
          "upper": 0.9452103484870663
        },
        "input_fixture_groups": 12,
        "median_ratio": 0.9229955965345837,
        "required_median_max": 0.95,
        "required_upper_max_exclusive": 1.0,
        "run": 2,
        "workloads": 12
      }
    ],
    "raw_gate_reasons": 423
  },
  "policy": {
    "build_policy_sha256": "105afb3b5c6ecabf0d465ea3bf8fa9fc9be5a39f6ef5eda28658fd952247a174",
    "contract_sha256": "7b6a180c9728f2bd030b13b418174b56f9044b66c0e431cfcaccba865e507572",
    "primary_workloads": [
      "scalar_br_table_n1",
      "scalar_br_table_n32",
      "scalar_br_table_n1024",
      "scalar_br_table_n10000",
      "scalar_multivalue_branch_n1",
      "scalar_multivalue_branch_n32",
      "scalar_multivalue_branch_n1024",
      "scalar_multivalue_branch_n10000",
      "scalar_try_table_normal_n1",
      "scalar_try_table_normal_n32",
      "scalar_try_table_normal_n1024",
      "scalar_try_table_normal_n10000"
    ],
    "protected_count": 73,
    "selection": "scalar"
  },
  "schema": "wtime-p024-executed-evidence-v1",
  "source_identity": {
    "baseline_binary_sha256": "77132ce9531de66a1c8a14e2c43946eeee9f80951de11b18c55c7d4fcd920398",
    "baseline_files": 133,
    "baseline_manifest_sha256": "d84d516d2e894245599bb7b0a2be478cc597ccd9c3d1e48c526153362fc1f8ec",
    "before_build_source_sha256": "3660387cd20759f4d90e5f5eb25ff528ba325279fa2eff993dbcd5e5825eeec5",
    "candidate_binary_matches_retained": true,
    "candidate_binary_sha256": "5a38d8ca422bd62b29aed9a557f5f9eccf0d1882625b4d915698f17e2ffdca34",
    "candidate_build_source_sha256": "611797b6ad49ce118ab5f71b6df7c5829a401e3090e6c72315b54e191a11b3a7",
    "candidate_key_sources": {
      "crates/wtime-runtime/src/continuation.rs": "f69c75c3ef274fe771911738129a18eee6251963b88c6c4bcfaddf1614d8272b",
      "crates/wtime-runtime/src/execution.rs": "173770414722b60dfe1e8cc2a0441c14b8662aeb453b91790128e6fbb63c0ddd",
      "crates/wtime-runtime/src/reference.rs": "b640d438d1f0616e1e264db7fc8a7a664b0b4ebba056d93b6ee440f8d0ec8a11",
      "crates/wtime-runtime/tests/performance/p024.rs": "ffc3f81e2884148c08b5fd299ee7194757eb73b574e3b4a320e7d0126b5b44c8",
      "tools/wtime-bench/fixtures/scalar/try_table_normal.wat": "ab92800dd4e4d4053498c317effadedf52568270ab7b476eef74b09638de8964",
      "tools/wtime-bench/scalar_workloads.py": "b564e862913ed3fbd9a54bec0c722962fe999c25024375de69c1ceac03d2682e",
      "tools/wtime-bench/workloads.json": "e7941449302a744de2ad5a1a96c42ac1be9631c7809d673f93b44027b5cc0cf9"
    },
    "candidate_source_count": 128,
    "candidate_source_manifest_sha256": "ebb91feee000ffdd9d59a9cd4894e72d5a42a225b492e180279959080e050124",
    "identity_note": "Candidate identities are historical measurement identities, with immutable retained source and binary copies. Current runtime sources are restored to the immediate predecessor.",
    "production_restored": {
      "restored_runtime_sources": true,
      "archive": "target/performance/P024-before/source",
      "sha256": {
        "crates/wtime-runtime/src/execution.rs": "1baf160513d2daf53706f2bc03a54f7d01777b75da65656c5a6bedb4e995f4a5",
        "crates/wtime-runtime/src/reference.rs": "5afabf5f234dd6339a84be2440794ef0e181891f801d78a65a5703ccb304d7f9",
        "crates/wtime-runtime/src/continuation.rs": "c30b6fb4b89999b2e6f494b9258ce9ea3ea1e39a5971643b43147026e0fc8f46"
      }
    }
  },
  "status": "done",
  "task": "P024",
  "tests": {
    "candidate_default_focused": {
      "failed": 0,
      "log": "/workspace/wtime/target/performance/P024-candidate-tests.log",
      "passed": 11,
      "skipped": 0
    },
    "candidate_default_full_runtime": {
      "failed": 0,
      "log": "/workspace/wtime/target/performance/P024-runtime-tests.log",
      "passed": 183,
      "skipped": 0
    },
    "candidate_reference_counters_focused": {
      "failed": 0,
      "log": "/workspace/wtime/target/performance/P024-candidate-diagnostic-tests.log",
      "passed": 12,
      "skipped": 0
    },
    "reference_counters_performance_target": {
      "failed": 0,
      "log": "/workspace/wtime/target/performance/P024-performance-tests.log",
      "passed": 43,
      "skipped": 0
    },
    "scalar_model": {
      "failed": 0,
      "log": "/workspace/wtime/target/performance/P024-model-tests-final.log",
      "passed": 4,
      "skipped": 0
    },
    "restored_default_full_runtime": {
      "passed": 179,
      "failed": 0,
      "skipped": 0
    },
    "restored_reference_counters_performance": {
      "passed": 43,
      "failed": 0,
      "skipped": 0
    }
  },
  "outcome": "rejected"
}
```
