# P025 — Remove per-load heap allocation for all scalar widths

Status: **done**

Outcome: **rejected**

Kind: `candidate` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P002 — Compare separately built baseline and candidate binaries](../00-measurement/P002-paired-driver.md)

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P009 — Add memory width, bulk and growth benchmark families](../00-measurement/P009-memory-workloads.md)

[P016 — Add explicit executor policies and test integration hooks](../01-execution-contracts/P016-execution-policy.md)

[P017 — Make every Module clone share immutable metadata](../01-execution-contracts/P017-shared-module-payload.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C5, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/lib.rs`
- `crates/wtime-runtime/src/shared_memory.rs`
- `crates/wtime-runtime/src/execution.rs`
- `crates/wtime-runtime/tests/performance/p025.rs`

- `crates/wtime-runtime/tests/performance.rs`

- `crates/wtime-runtime/tests/fixtures/performance/p025/*`

- `tools/wtime-bench/src/memory_driver.rs`

- `tools/wtime-bench/compare.py`

- `tools/wtime-bench/memory_workloads.py`

- `tools/wtime-bench/memory_scaling.py`

- `tools/wtime-bench/workloads.json`

- `tools/wtime-bench/tests/test_p009_memory.py`

- `tools/wtime-bench/tests/test_p025_driver.py`

- `tools/wtime-bench/tests/test_p025_compare.py`

- `docs/benchmarks/performance/P025-contract.json`

- `docs/benchmarks/performance/P025-build-decision.md`

- `docs/benchmarks/performance/P025-evidence.json`
- `tools/wtime-bench/gates.py`
- `docs/benchmarks/performance/P025-candidate.patch`
- `tools/wtime-bench/tests/test_p025_regressions.py`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

Mem::read_into(off, &mut [u8]) copies checked bytes into caller storage; scalar loads use [u8;8]. Public read_memory still returns an owned Vec and shared reads retain their lock semantics.

## Implementation recipe

1. Add a private checked read_into path for unshared bytes and shared backing; hold the shared lock through bounds validation and copy.
2. Route scalar load_store through a fixed stack buffer, avoid its duplicate bounds check where the callee already performs the whole check, and retain checked effective-address arithmetic.
3. Leave public host reads and bulk snapshots unchanged in this patch; preserve little-endian conversion, narrow signed extension and precise OOB trap category.
4. Verify zero read-Vec events for guest scalar loads, then compare read-only/mixed/original memory kernels without counters.
5. Record the changed path with focused assertions/diagnostic counts and exact source/binary identities; compare the pre-edit archive, and revert a rejected structural optimization rather than adding a permanent toggle solely for testing.

## Meaningful cases

- All 1/2/4/8-byte signed/unsigned and unaligned loads match reference bits.
- Near-end/effective-address-overflow loads trap without reading/mutating beyond bounds.
- Shared ordinary loads remain synchronized and public read_memory ownership is unchanged.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime p025_ -- --nocapture
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml -j 4
python3 tools/wtime-bench/compare.py --baseline target/performance/P025-before --candidate tools/wtime-bench/target/release/wtime-bench --candidate-mode decoded --selection memory --pairs 60 --build-growth-explanation-file docs/benchmarks/performance/P025-build-decision.md --output target/performance/P025-final
python3 tools/wtime-bench/analyze.py --input target/performance/P025-final --output target/performance/P025-final/analysis.json
python3 tools/checks/build_budget.py --output target/performance/P025-final/build.json
python3 tools/wtime-bench/gates.py --task P025 --comparison target/performance/P025-final --build target/performance/P025-final/build.json --contract docs/benchmarks/performance/P025-contract.json --output target/performance/P025-final/decision.json
```

## Measurement and decision

Selection: `memory`. Explicit execution setting: `decoded`. Decision rule: `measured_promotion`.

Use the paired protocol and family/control/cold/build rules in [MEASUREMENT.md](../MEASUREMENT.md). Predeclare the targeted family and held-out controls before changing the implementation. Keep instrumentation off in production timings. Adopt only with supported gains in both runs; otherwise record rejected/inconclusive and remove default-path cost. A necessary immutable-interface change may be contract_only with its separate justification.

## Before editing

Run these commands before changing the candidate sources. The archive is immutable and captures the immediate predecessor. Retain P001's original archive for milestone totals. For native work, use the named portable control and verify its actual policy/quantum; if this archive already exists with different sources, choose a new recorded run-specific directory and update the comparison path rather than overwriting it.

```sh
python3 tools/wtime-bench/baseline.py --output target/performance/P025-before
```

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. Executed evidence is recorded below.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/docs/benchmarks/performance/P025-build-decision.md": {
      "sha256": "43971b285865f711ff4477ff85664450ece92535642d5dbfa7221770e88d9373",
      "bytes": 1843
    },
    "/workspace/wtime/docs/benchmarks/performance/P025-candidate.patch": {
      "sha256": "1b205f43725940d0854f3aadeede0c40992072e15f4c77c110675d04a6097156",
      "bytes": 2644
    },
    "/workspace/wtime/docs/benchmarks/performance/P025-contract.json": {
      "sha256": "a5bf9557cfcd9c07ea8363afc198891803238686440e2199cde999b8937ddf2b",
      "bytes": 900
    },
    "/workspace/wtime/target/performance/P025-analysis.log": {
      "sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
      "bytes": 0
    },
    "/workspace/wtime/target/performance/P025-baseline.log": {
      "sha256": "295ad79cd4fc2586e786e07ec527f4fb5436aa44cf770edd1efbb5c11ccfee42",
      "bytes": 243
    },
    "/workspace/wtime/target/performance/P025-before-build-final.json": {
      "sha256": "c39eaad74424808c6e331add0c375fe668ad9e330ee0cf308a1339545554a47b",
      "bytes": 74584
    },
    "/workspace/wtime/target/performance/P025-before-build-final.log": {
      "sha256": "79bcc27898c2c1c48890dcf78ffff70dcfe758557376b8172a76a22333da35d0",
      "bytes": 1702
    },
    "/workspace/wtime/target/performance/P025-before-tests-corrected.log": {
      "sha256": "644e3ae11d4ac897c3b9c45c4afa02b1fa56297739cb1127fa227c1d97fc32f4",
      "bytes": 1666
    },
    "/workspace/wtime/target/performance/P025-before-tests-final.log": {
      "sha256": "fe53197aee29845086b7578231729d71b0922dfec77c8618ada43ee1d82e25b5",
      "bytes": 2378
    },
    "/workspace/wtime/target/performance/P025-before-tests.log": {
      "sha256": "e98ca06a9483f6c63a3514444537edd99f60e877f485c79f658cab08375c7764",
      "bytes": 820
    },
    "/workspace/wtime/target/performance/P025-before-tests2.log": {
      "sha256": "ee58de2c590b9d6299ec93dc8abe3da69f379f0c1bada37d897e5cc4ac4f03aa",
      "bytes": 2291
    },
    "/workspace/wtime/target/performance/P025-before/binary/wtime-bench": {
      "sha256": "77132ce9531de66a1c8a14e2c43946eeee9f80951de11b18c55c7d4fcd920398",
      "bytes": 17264360
    },
    "/workspace/wtime/target/performance/P025-before/manifest.json": {
      "sha256": "35432e359d5af1557848c000f820597af7bdc4e569d4a2cc4a138ad6132ea0bc",
      "bytes": 29875
    },
    "/workspace/wtime/target/performance/P025-candidate-build-budget.json": {
      "sha256": "83039315e19cc4d69cb2895ecde915245349ce4b36b5e6e1ee8f874205c61376",
      "bytes": 74586
    },
    "/workspace/wtime/target/performance/P025-candidate-build.log": {
      "sha256": "a3b75706096a9035a2c340edb6912415cb2b8832fa58c82e985e1d34df7944d6",
      "bytes": 206
    },
    "/workspace/wtime/target/performance/P025-candidate-counter-tests.log": {
      "sha256": "3b48a31afb0146aab427a9ade720e604b5735557754f270e4b5d684c7a0bea74",
      "bytes": 1666
    },
    "/workspace/wtime/target/performance/P025-candidate-tests.log": {
      "sha256": "8b6fa3598cfa53611af63689b19ac2ba45fea2ead1a4c94df8e2ad373f445c1b",
      "bytes": 1666
    },
    "/workspace/wtime/target/performance/P025-final-tool-tests-with-driver.log": {
      "sha256": "ab8c5b4a0faffd7c801147ce402411e213946ceb6279af91dfb4caf283b379ca",
      "bytes": 3302
    },
    "/workspace/wtime/target/performance/P025-final-tool-tests.log": {
      "sha256": "31065681cf788435c4ed84f3513a606a95ad96e7525587394e7446c06eb23a18",
      "bytes": 3764
    },
    "/workspace/wtime/target/performance/P025-final/analysis.json": {
      "sha256": "374adf12ef6234dcb96432a1468b0a685ddea232fd86093ef1549e5494d1ba4c",
      "bytes": 155831
    },
    "/workspace/wtime/target/performance/P025-final/build.json": {
      "sha256": "83039315e19cc4d69cb2895ecde915245349ce4b36b5e6e1ee8f874205c61376",
      "bytes": 74586
    },
    "/workspace/wtime/target/performance/P025-final/candidate-retained/crates/wtime-runtime/src/execution.rs": {
      "sha256": "01fe8140bead06e070f79555839b6cb63d5f3d00f8095505b55f557b23c7d723",
      "bytes": 53212
    },
    "/workspace/wtime/target/performance/P025-final/candidate-retained/crates/wtime-runtime/src/lib.rs": {
      "sha256": "13357c4735da348c4471dc2360153b186ff30679029d36662eb290f3860866cb",
      "bytes": 72053
    },
    "/workspace/wtime/target/performance/P025-final/candidate-retained/crates/wtime-runtime/src/shared_memory.rs": {
      "sha256": "f64f46b42620a79f83617038e949bfee387291411b2fe3e01e7c0572b0c3a24a",
      "bytes": 17221
    },
    "/workspace/wtime/target/performance/P025-final/candidate-retained/identities.json": {
      "sha256": "605cac242e8a3137ce1488a5a64bd9f8c807f2ed647e3bb6d4f34ab4bd1f0c1e",
      "bytes": 452
    },
    "/workspace/wtime/target/performance/P025-final/candidate-retained/raw-gate-source.py": {
      "sha256": "28f14026c1981e7564420dd2f1f604adb571db73e96e93a736ce9e9c2cccc9a6",
      "bytes": 28503
    },
    "/workspace/wtime/target/performance/P025-final/candidate-retained/source-manifest.json": {
      "sha256": "c91753aa1c9b325c3874bb9f4460cfe527b0fe845fa60517794eb654494f66bb",
      "bytes": 13894
    },
    "/workspace/wtime/target/performance/P025-final/candidate-retained/tools/wtime-bench/target/release/wtime-bench": {
      "sha256": "087863cb4794d681286832679505324201d16d45a4a5d5aea0828fd8d25ee068",
      "bytes": 17265936
    },
    "/workspace/wtime/target/performance/P025-final/checkpoint.jsonl": {
      "sha256": "34cb7a3dc65752da79d2409eba1a9fede956f011a86d4a5e6f4356bce33ca827",
      "bytes": 10137598
    },
    "/workspace/wtime/target/performance/P025-final/decision.json": {
      "sha256": "7be59f7c9b493d46b60357b0a0e8bed2464969d19cabcd10a0785fa44ec123a8",
      "bytes": 169
    },
    "/workspace/wtime/target/performance/P025-final/extract_completed.py": {
      "sha256": "dc863a428a8bf87192836beb06451385930be497e3509d295ba879172bac37e2",
      "bytes": 24447
    },
    "/workspace/wtime/target/performance/P025-final/extraction.json": {
      "sha256": "c68b310d24588d584f566ba695864679b8601f42f3c9561e8d6cb3b1771fe28b",
      "bytes": 8339
    },
    "/workspace/wtime/target/performance/P025-final/fixtures/memory/bulk_copy_aliased.wasm": {
      "sha256": "62090599a827936312ab6e8aef30c8dd58950e5541925dc1f104cbcae631ea9f",
      "bytes": 230
    },
    "/workspace/wtime/target/performance/P025-final/fixtures/memory/bulk_copy_backward.wasm": {
      "sha256": "c471424ab8f70544c8910c159fb722762f50a69d8275672baadfdd4be610f24f",
      "bytes": 199
    },
    "/workspace/wtime/target/performance/P025-final/fixtures/memory/bulk_copy_forward.wasm": {
      "sha256": "fa79408dcd1e5a6de0b30efec318fef7af1b32f38fa91aa85e096bc89e8242dd",
      "bytes": 199
    },
    "/workspace/wtime/target/performance/P025-final/fixtures/memory/bulk_fill.wasm": {
      "sha256": "fce9add69d0ce7c47d47bf4df17d3af2a189bd0553f45dc49d26aec4d368bb91",
      "bytes": 142
    },
    "/workspace/wtime/target/performance/P025-final/fixtures/memory/custom_page_one_byte.wasm": {
      "sha256": "29dd399af1a1ba4bf59db09733986c819b5dfc072dce44b3a398adda316ba3b9",
      "bytes": 87
    },
    "/workspace/wtime/target/performance/P025-final/fixtures/memory/effective_address_overflow.wasm": {
      "sha256": "670a1ec729e5580947ddc083bef8455df6d256918740102938ce4b79cc083ec9",
      "bytes": 78
    },
    "/workspace/wtime/target/performance/P025-final/fixtures/memory/growth.wasm": {
      "sha256": "077018a548b5a666bd3b909d844c7f620162312fa8032f927384ed5946766152",
      "bytes": 112
    },
    "/workspace/wtime/target/performance/P025-final/fixtures/memory/memory64.wasm": {
      "sha256": "760627846a98bffa0faf30cc8cb953b9eb72580a24eda9954d8da9d24ee8a3ff",
      "bytes": 98
    },
    "/workspace/wtime/target/performance/P025-final/fixtures/memory/multi_memory_second.wasm": {
      "sha256": "577f2cbd04a7c384a896c859ef04cd02573120f375c4d6df5b15790be610a310",
      "bytes": 131
    },
    "/workspace/wtime/target/performance/P025-final/fixtures/memory/oob_order.wasm": {
      "sha256": "44cfc8f92c7be0e77a8625cfbe2f1306007ff3e10e8e26aa551a4ee44037dabf",
      "bytes": 92
    },
    "/workspace/wtime/target/performance/P025-final/fixtures/memory/read_scan.wasm": {
      "sha256": "fa603f253df536b584a90272ab4bb9beb46cb4a0c8e51ca0a2ba3da3eda0ae18",
      "bytes": 191
    },
    "/workspace/wtime/target/performance/P025-final/fixtures/memory/widths.wasm": {
      "sha256": "febd4bc1275c2db30f6d67c722a577c3e5fba64ee072c14175c5fb2f5769df74",
      "bytes": 383
    },
    "/workspace/wtime/target/performance/P025-final/fixtures/memory/widths_shuffled.wasm": {
      "sha256": "e948f8be21a6e70d46be7dd3a1a3d78d700ba7535a6220bf8d213b2392eab511",
      "bytes": 391
    },
    "/workspace/wtime/target/performance/P025-final/promotion-decision.json": {
      "sha256": "204691e97fa3eb2bea32de3c12bfbec1afa63b30bf3b70adc3cafdc4d3c1cb14",
      "bytes": 1602
    },
    "/workspace/wtime/target/performance/P025-final/report.json": {
      "sha256": "5247529ef8984ba0833e0273629853df28b11700c894dadc44a9742dd2e1873a",
      "bytes": 17033787
    },
    "/workspace/wtime/target/performance/P025-final/supplement_completed.py": {
      "sha256": "8c6bad69356177148bd0c9ca8ce90359babd342f04964dc2b32ec8987dfd9850",
      "bytes": 1904
    },
    "/workspace/wtime/target/performance/P025-final/timing-supplement.json": {
      "sha256": "892e69fee8883c3a6b36427af4df067e63fe011bcfa187a67c63b64bd98c133e",
      "bytes": 35651
    },
    "/workspace/wtime/target/performance/P025-gate.exit": {
      "sha256": "53c234e5e8472b6ac51c1ae1cab3fe06fad053beb8ebfd8977b010655bfdd3c3",
      "bytes": 2
    },
    "/workspace/wtime/target/performance/P025-gate.log": {
      "sha256": "3995c703634a90edfc6ca45d2a89058ecc4d99cb3953cd2c4839c4241f2a3684",
      "bytes": 95
    },
    "/workspace/wtime/target/performance/P025-helper-identity-companion.json": {
      "sha256": "9f00b4a2731f90b3eb39407be1acc3e70b3b0f3c2796c1911504bd2ae3bc3b97",
      "bytes": 1074
    },
    "/workspace/wtime/target/performance/P025-incremental-build-decision.json": {
      "sha256": "9609483a67baa9f466ff41352bcb3400ba1fe7fc9ab727fe8dc581dfa474f4ce",
      "bytes": 1452
    },
    "/workspace/wtime/target/performance/P025-performance-tests.log": {
      "sha256": "56225246898133bd69961288f64e3979f47baf1e1ef68a0f6545782e2948f7e4",
      "bytes": 2849
    },
    "/workspace/wtime/target/performance/P025-rollback-bench-build.log": {
      "sha256": "6f18cbeb53d04afaa4f8f46684d85ce2993c9d6fb75e8732dc50eb209482b4cb",
      "bytes": 206
    },
    "/workspace/wtime/target/performance/P025-rollback-build.json": {
      "sha256": "9a8f2cd6f8523a0b2d5dc7d3d80df8fbd7461b16c41ab69cff53da10cfc5fc9f",
      "bytes": 74583
    },
    "/workspace/wtime/target/performance/P025-rollback-build.log": {
      "sha256": "8c0fa017de4b02715c8ae27de0ff11b38812169800d18637a988263f737fc01d",
      "bytes": 1701
    },
    "/workspace/wtime/target/performance/P025-rollback-identities.json": {
      "sha256": "f8df214eafc0b71c0c29c75de6c714879c420ca509350fd882389dfb22010c03",
      "bytes": 847
    },
    "/workspace/wtime/target/performance/P025-rollback-performance-tests.log": {
      "sha256": "f9fd5e230e26d99bf553bd6c03e120f83a3dd9b517a80e073f38f89cb5adcc9c",
      "bytes": 2849
    },
    "/workspace/wtime/target/performance/P025-rollback-runtime-tests.log": {
      "sha256": "408be4977aab6820bd514986e34e9dec8926cd5cb1153d38e15b19590f117bf3",
      "bytes": 12796
    },
    "/workspace/wtime/target/performance/P025-run-final.log": {
      "sha256": "9f47346b35f315805b160d238d9594caafc22bcc1a2ec364ae3a7d146f09f58d",
      "bytes": 4780
    },
    "/workspace/wtime/target/performance/P025-run.log": {
      "sha256": "ba77649de83fbd532bd687dc39cf0e94df75be1e9ab24c4dca60f5c1964e1a80",
      "bytes": 341
    },
    "/workspace/wtime/target/performance/P025-run2.log": {
      "sha256": "4156c5b4f2c936e2f9d09bc82ada0b7223f035124c62a179b18c178999fff7c9",
      "bytes": 87
    },
    "/workspace/wtime/target/performance/P025-run3.log": {
      "sha256": "f5af7237fe37541f2153e97d1002d414c32e1f7449fd61ded27e6429a9a43f43",
      "bytes": 1252
    },
    "/workspace/wtime/target/performance/P025-runtime-tests.log": {
      "sha256": "c6bb37c8c7369d2195f02fd6d9bde89bd254c52a5eb8acddda771b132aa69d70",
      "bytes": 12795
    },
    "/workspace/wtime/target/performance/P025-timing-supplement.log": {
      "sha256": "7b062f9a13040a4faf3d00229867c570d98d6d16d1f78a0867f3cf3a24039770",
      "bytes": 627
    },
    "/workspace/wtime/target/performance/P025-tool-tests.log": {
      "sha256": "8273dd68e0e37bd2909d2b6ae1a3f021cfe8b9b206752d08be8406492ecc5876",
      "bytes": 2201
    },
    "/workspace/wtime/target/performance/source-binding-tests.log": {
      "sha256": "88a13290a69c50aff73e3043da4cfe8b1c51e9d4d44653a4bbebb1280d78089a",
      "bytes": 966
    },
    "/workspace/wtime/target/performance/stage-coverage-tests.log": {
      "sha256": "168ecc2e6b53683dae85a2b3d8e203f8413e713d24454b9de86ff71609acfc5f",
      "bytes": 696
    },
    "/workspace/wtime/docs/benchmarks/performance/P025-evidence.json": {
      "sha256": "3d718890956a898106547fd79709ce1178a1837944353d5a7c7a7e5411fe7770",
      "bytes": 35077
    }
  },
  "artifacts": [
    "/workspace/wtime/docs/benchmarks/performance/P025-build-decision.md",
    "/workspace/wtime/docs/benchmarks/performance/P025-candidate.patch",
    "/workspace/wtime/docs/benchmarks/performance/P025-contract.json",
    "/workspace/wtime/target/performance/P025-analysis.log",
    "/workspace/wtime/target/performance/P025-baseline.log",
    "/workspace/wtime/target/performance/P025-before-build-final.json",
    "/workspace/wtime/target/performance/P025-before-build-final.log",
    "/workspace/wtime/target/performance/P025-before-tests-corrected.log",
    "/workspace/wtime/target/performance/P025-before-tests-final.log",
    "/workspace/wtime/target/performance/P025-before-tests.log",
    "/workspace/wtime/target/performance/P025-before-tests2.log",
    "/workspace/wtime/target/performance/P025-before/binary/wtime-bench",
    "/workspace/wtime/target/performance/P025-before/manifest.json",
    "/workspace/wtime/target/performance/P025-candidate-build-budget.json",
    "/workspace/wtime/target/performance/P025-candidate-build.log",
    "/workspace/wtime/target/performance/P025-candidate-counter-tests.log",
    "/workspace/wtime/target/performance/P025-candidate-tests.log",
    "/workspace/wtime/target/performance/P025-final-tool-tests-with-driver.log",
    "/workspace/wtime/target/performance/P025-final-tool-tests.log",
    "/workspace/wtime/target/performance/P025-final/analysis.json",
    "/workspace/wtime/target/performance/P025-final/build.json",
    "/workspace/wtime/target/performance/P025-final/candidate-retained/crates/wtime-runtime/src/execution.rs",
    "/workspace/wtime/target/performance/P025-final/candidate-retained/crates/wtime-runtime/src/lib.rs",
    "/workspace/wtime/target/performance/P025-final/candidate-retained/crates/wtime-runtime/src/shared_memory.rs",
    "/workspace/wtime/target/performance/P025-final/candidate-retained/identities.json",
    "/workspace/wtime/target/performance/P025-final/candidate-retained/raw-gate-source.py",
    "/workspace/wtime/target/performance/P025-final/candidate-retained/source-manifest.json",
    "/workspace/wtime/target/performance/P025-final/candidate-retained/tools/wtime-bench/target/release/wtime-bench",
    "/workspace/wtime/target/performance/P025-final/checkpoint.jsonl",
    "/workspace/wtime/target/performance/P025-final/decision.json",
    "/workspace/wtime/target/performance/P025-final/extract_completed.py",
    "/workspace/wtime/target/performance/P025-final/extraction.json",
    "/workspace/wtime/target/performance/P025-final/fixtures/memory/bulk_copy_aliased.wasm",
    "/workspace/wtime/target/performance/P025-final/fixtures/memory/bulk_copy_backward.wasm",
    "/workspace/wtime/target/performance/P025-final/fixtures/memory/bulk_copy_forward.wasm",
    "/workspace/wtime/target/performance/P025-final/fixtures/memory/bulk_fill.wasm",
    "/workspace/wtime/target/performance/P025-final/fixtures/memory/custom_page_one_byte.wasm",
    "/workspace/wtime/target/performance/P025-final/fixtures/memory/effective_address_overflow.wasm",
    "/workspace/wtime/target/performance/P025-final/fixtures/memory/growth.wasm",
    "/workspace/wtime/target/performance/P025-final/fixtures/memory/memory64.wasm",
    "/workspace/wtime/target/performance/P025-final/fixtures/memory/multi_memory_second.wasm",
    "/workspace/wtime/target/performance/P025-final/fixtures/memory/oob_order.wasm",
    "/workspace/wtime/target/performance/P025-final/fixtures/memory/read_scan.wasm",
    "/workspace/wtime/target/performance/P025-final/fixtures/memory/widths.wasm",
    "/workspace/wtime/target/performance/P025-final/fixtures/memory/widths_shuffled.wasm",
    "/workspace/wtime/target/performance/P025-final/promotion-decision.json",
    "/workspace/wtime/target/performance/P025-final/report.json",
    "/workspace/wtime/target/performance/P025-final/supplement_completed.py",
    "/workspace/wtime/target/performance/P025-final/timing-supplement.json",
    "/workspace/wtime/target/performance/P025-gate.exit",
    "/workspace/wtime/target/performance/P025-gate.log",
    "/workspace/wtime/target/performance/P025-helper-identity-companion.json",
    "/workspace/wtime/target/performance/P025-incremental-build-decision.json",
    "/workspace/wtime/target/performance/P025-performance-tests.log",
    "/workspace/wtime/target/performance/P025-rollback-bench-build.log",
    "/workspace/wtime/target/performance/P025-rollback-build.json",
    "/workspace/wtime/target/performance/P025-rollback-build.log",
    "/workspace/wtime/target/performance/P025-rollback-identities.json",
    "/workspace/wtime/target/performance/P025-rollback-performance-tests.log",
    "/workspace/wtime/target/performance/P025-rollback-runtime-tests.log",
    "/workspace/wtime/target/performance/P025-run-final.log",
    "/workspace/wtime/target/performance/P025-run.log",
    "/workspace/wtime/target/performance/P025-run2.log",
    "/workspace/wtime/target/performance/P025-run3.log",
    "/workspace/wtime/target/performance/P025-runtime-tests.log",
    "/workspace/wtime/target/performance/P025-timing-supplement.log",
    "/workspace/wtime/target/performance/P025-tool-tests.log",
    "/workspace/wtime/target/performance/source-binding-tests.log",
    "/workspace/wtime/target/performance/stage-coverage-tests.log",
    "/workspace/wtime/docs/benchmarks/performance/P025-evidence.json"
  ],
  "build_metrics": {
    "before": {
      "budget_pass": true,
      "clean_peak_rss_kib": 278740,
      "clean_seconds": 8.72141878399998,
      "leaf_seconds": 0.3627999680029461,
      "release_seconds": 17.194877675006865,
      "runtime_leaf_seconds": 1.2530675469970447,
      "source_stable": true,
      "stripped_bytes": 1000264
    },
    "candidate": {
      "budget_pass": true,
      "clean_peak_rss_kib": 286800,
      "clean_seconds": 8.672789557000215,
      "leaf_seconds": 0.41428620900114765,
      "release_seconds": 23.562572540002293,
      "runtime_leaf_seconds": 1.4865648550039623,
      "source_stable": true,
      "stripped_bytes": 1001712
    },
    "default_dependency_closure_pass": true,
    "incremental_ratios": {
      "clean_peak_rss_kib": {
        "before": 278740,
        "candidate": 286800,
        "limit": 1.25,
        "pass": true,
        "ratio": 1.0289158355456698
      },
      "clean_seconds": {
        "before": 8.72141878399998,
        "candidate": 8.672789557000215,
        "limit": 1.25,
        "pass": true,
        "ratio": 0.9944241609990133
      },
      "leaf_seconds": {
        "before": 0.3627999680029461,
        "candidate": 0.41428620900114765,
        "limit": 1.25,
        "pass": true,
        "ratio": 1.141913576458153
      },
      "release_seconds": {
        "before": 17.194877675006865,
        "candidate": 23.562572540002293,
        "limit": 1.25,
        "pass": false,
        "ratio": 1.370325104100683
      },
      "runtime_leaf_seconds": {
        "before": 1.2530675469970447,
        "candidate": 1.4865648550039623,
        "limit": 1.25,
        "pass": true,
        "ratio": 1.186340559666149
      },
      "stripped_bytes": {
        "before": 1000264,
        "candidate": 1001712,
        "limit": 1.25,
        "pass": true,
        "ratio": 1.0014476178288931
      }
    }
  },
  "commands": [
    {
      "command": "python3 tools/wtime-bench/baseline.py --output target/performance/P025-before",
      "evidence": "target/performance/P025-baseline.log",
      "exit_code": 0,
      "result": "110 files archived; correctness passed"
    },
    {
      "command": "cargo build --locked --offline -j 5",
      "evidence": "target/performance/P025-before-build-final.json",
      "exit_code": 0,
      "note": "Predecessor clean build measurement; JSON captures source identity and build metrics."
    },
    {
      "command": "cargo build --locked --offline -j 5",
      "evidence": "target/performance/P025-candidate-build-budget.json",
      "exit_code": 0,
      "note": "Candidate clean build measurement; JSON captures source identity and build metrics."
    },
    {
      "command": "cargo test --locked -p wtime-runtime",
      "evidence": "target/performance/P025-runtime-tests.log",
      "exit_code": 0,
      "failed": 0,
      "passed": 172,
      "skipped": 0
    },
    {
      "command": "cargo test --locked -p wtime-runtime --features execution-reference,performance-counters --test performance",
      "evidence": "target/performance/P025-performance-tests.log",
      "exit_code": 0,
      "failed": 0,
      "passed": 35,
      "skipped": 0,
      "note": "Diagnostic/reference-enabled suite; log binary fingerprint confirms these features. Production timings used a separate noninstrumented default release binary."
    },
    {
      "command": "cargo test --locked -p wtime-runtime p025_ -- --nocapture",
      "evidence": "target/performance/P025-candidate-tests.log",
      "exit_code": 0,
      "failed": 0,
      "passed": 4,
      "skipped": 0,
      "note": "Focused name filter across runtime test targets; four executed P025 integration cases, other targets filtered out."
    },
    {
      "command": "cargo test --locked -p wtime-runtime --features performance-counters p025_ -- --nocapture",
      "evidence": "target/performance/P025-candidate-counter-tests.log",
      "exit_code": 0,
      "failed": 0,
      "note": "Focused name filter across runtime test targets; four executed P025 integration cases, other targets filtered out.",
      "passed": 4,
      "skipped": 0
    },
    {
      "command": "python3 -m unittest discover -s tools/wtime-bench/tests -p 'test_p025*.py' -v",
      "evidence": "target/performance/P025-tool-tests.log",
      "exit_code": 0,
      "failed": 0,
      "passed": 12,
      "skipped": 0
    },
    {
      "command": "python3 tools/wtime-bench/compare.py --baseline target/performance/P025-before --candidate tools/wtime-bench/target/release/wtime-bench --candidate-mode decoded --selection memory --pairs 60 --build-growth-explanation-file docs/benchmarks/performance/P025-build-decision.md --output target/performance/P025-final",
      "exit_code": 0,
      "status": "completed",
      "evidence": "target/performance/P025-final/report.json"
    },
    {
      "command": "python3 tools/wtime-bench/analyze.py --input target/performance/P025-final --output target/performance/P025-final/analysis.json",
      "exit_code": 0
    },
    {
      "command": "python3 tools/wtime-bench/gates.py --task P025 --comparison target/performance/P025-final --build target/performance/P025-final/build.json --contract docs/benchmarks/performance/P025-contract.json --output target/performance/P025-final/decision.json",
      "exit_code": 2,
      "error": "comparison is missing warm input/fixture coverage for memory64_i32_extended in run 1",
      "note": "Original failure retained; supplemental fixed-coverage timing evaluation is separate."
    },
    {
      "command": "cargo test --locked -p wtime-runtime",
      "exit_code": 0,
      "passed": 172,
      "failed": 0,
      "skipped": 0,
      "evidence": "target/performance/P025-rollback-runtime-tests.log"
    },
    {
      "command": "cargo test --locked -p wtime-runtime --features execution-reference,performance-counters --test performance",
      "exit_code": 0,
      "passed": 35,
      "failed": 0,
      "skipped": 0,
      "evidence": "target/performance/P025-rollback-performance-tests.log"
    },
    {
      "command": "WTIME_BENCH_MEMORY_DRIVER=/workspace/wtime/tools/wtime-bench/target/release/wtime-bench WTIME_CONFORMANCE_ENCODER=/workspace/wtime/target/debug/wtime-conformance python3 -m unittest discover -s tools/wtime-bench/tests -p test_p025*.py -v",
      "exit_code": 0,
      "passed": 18,
      "failed": 0,
      "skipped": 0,
      "evidence": "target/performance/P025-final-tool-tests-with-driver.log"
    },
    {
      "command": "python3 tools/checks/build_budget.py --output target/performance/P025-rollback-build.json",
      "exit_code": 0,
      "budget_pass": true,
      "evidence": "target/performance/P025-rollback-build.json"
    }
  ],
  "fixture_hashes": {
    "bulk_copy_aliased.wasm": "62090599a827936312ab6e8aef30c8dd58950e5541925dc1f104cbcae631ea9f",
    "bulk_copy_backward.wasm": "c471424ab8f70544c8910c159fb722762f50a69d8275672baadfdd4be610f24f",
    "bulk_copy_forward.wasm": "fa79408dcd1e5a6de0b30efec318fef7af1b32f38fa91aa85e096bc89e8242dd",
    "bulk_fill.wasm": "fce9add69d0ce7c47d47bf4df17d3af2a189bd0553f45dc49d26aec4d368bb91",
    "custom_page_one_byte.wasm": "29dd399af1a1ba4bf59db09733986c819b5dfc072dce44b3a398adda316ba3b9",
    "effective_address_overflow.wasm": "670a1ec729e5580947ddc083bef8455df6d256918740102938ce4b79cc083ec9",
    "growth.wasm": "077018a548b5a666bd3b909d844c7f620162312fa8032f927384ed5946766152",
    "memory64.wasm": "760627846a98bffa0faf30cc8cb953b9eb72580a24eda9954d8da9d24ee8a3ff",
    "multi_memory_second.wasm": "577f2cbd04a7c384a896c859ef04cd02573120f375c4d6df5b15790be610a310",
    "oob_order.wasm": "44cfc8f92c7be0e77a8625cfbe2f1306007ff3e10e8e26aa551a4ee44037dabf",
    "read_scan.wasm": "fa603f253df536b584a90272ab4bb9beb46cb4a0c8e51ca0a2ba3da3eda0ae18",
    "widths.wasm": "febd4bc1275c2db30f6d67c722a577c3e5fba64ee072c14175c5fb2f5769df74",
    "widths_shuffled.wasm": "e948f8be21a6e70d46be7dd3a1a3d78d700ba7535a6220bf8d213b2392eab511"
  },
  "limitations": [
    "No Wasmtime timing ratio is claimed.",
    "Historical compare source tracker omitted workloads.py; companion states the limitation without claiming start/end binding. Future tracker and fresh archive now bind imported helper modules.",
    "Original promotion gate exited2 due requiring undeclared warm stages for cold-only controls. It is preserved unchanged. A separate retrospective timing evaluator uses the corrected coverage rule with unchanged workloads, stages, sample counts and thresholds; it does not claim current-source validation for the historical candidate.",
    "Primary warm family CIs cross1 in both runs; the candidate has no supported5% gain, and cold controls also failed the supplemental timing gate.",
    "The decisive predeclared incremental release-build budget failed at ratio1.370325104100683 vs1.25; candidate removed from default path.",
    "Initial errors and the first no-driver tool check (12executed,6skipped) remain retained; final driver-enabled18tests all passed.",
    "The35-case performance suite is diagnostic/reference-enabled; default full172-case suite includes26performance cases."
  ],
  "paired_measurement": {
    "status": "complete",
    "process_exit_code": 0,
    "pairs": 4920,
    "groups_per_run": 41,
    "pairs_per_group": 60,
    "runs": 2,
    "correctness_probes": 26,
    "source_stable": true,
    "instrumented": false,
    "candidate_features": [],
    "raw_gate_exit_code": 2,
    "raw_gate_error": "comparison is missing warm input/fixture coverage for memory64_i32_extended in run 1",
    "supplemental_timing_decision": "rejected",
    "supplemental_timing_reasons": 59,
    "primary_family": [
      {
        "run": 1,
        "median_ratio": 0.9982708758576169,
        "ci95": {
          "lower": 0.963153549920476,
          "upper": 1.0367597448790369
        },
        "bootstrap_resamples": 10000,
        "bootstrap_seed": 6774883132656344605,
        "workloads": 3,
        "input_fixture_groups": 3,
        "required_median_max": 0.95,
        "required_upper_max_exclusive": 1.0
      },
      {
        "run": 2,
        "median_ratio": 0.9858802319649366,
        "ci95": {
          "lower": 0.960255963028989,
          "upper": 1.0115078940052107
        },
        "bootstrap_resamples": 10000,
        "bootstrap_seed": 6017303162609402348,
        "workloads": 3,
        "input_fixture_groups": 3,
        "required_median_max": 0.95,
        "required_upper_max_exclusive": 1.0
      }
    ],
    "final_promotion": "rejected"
  },
  "predeclared_build_gate": {
    "before_seconds": 17.194877675006865,
    "candidate_seconds": 23.562572540002293,
    "decision_sha256": "9609483a67baa9f466ff41352bcb3400ba1fe7fc9ab727fe8dc581dfa474f4ce",
    "failed_metric": "release_seconds",
    "interpretation": "Predeclared policy rejects promotion because incremental release build ratio exceeds 1.25. Final rollback/status record remains parent-owned.",
    "limit": 1.25,
    "other_five_metrics_passed": true,
    "passed": false,
    "policy": "predeclared P025-build-decision.md",
    "policy_sha256": "43971b285865f711ff4477ff85664450ece92535642d5dbfa7221770e88d9373",
    "ratio": 1.370325104100683
  },
  "schema": "wtime-p025-executed-evidence-v1",
  "source_and_binary": {
    "before_archive_file_count": 110,
    "before_build_sha256": "c39eaad74424808c6e331add0c375fe668ad9e330ee0cf308a1339545554a47b",
    "before_build_source_sha256": "fe7b23f2c63dc23c6d0ddea9758e193e88ff3aa9669de637dc830cc041e2f04b",
    "before_manifest_sha256": "35432e359d5af1557848c000f820597af7bdc4e569d4a2cc4a138ad6132ea0bc",
    "candidate_build_sha256": "83039315e19cc4d69cb2895ecde915245349ce4b36b5e6e1ee8f874205c61376",
    "candidate_build_source_sha256": "ce7539cbfd34643c7abf76a14e72af4fd486d512637e4755a2c13bc5e3928f3e",
    "candidate_retained_manifest_sha256": "605cac242e8a3137ce1488a5a64bd9f8c807f2ed647e3bb6d4f34ab4bd1f0c1e",
    "contract_sha256": "a5bf9557cfcd9c07ea8363afc198891803238686440e2199cde999b8937ddf2b",
    "policy_sha256": "43971b285865f711ff4477ff85664450ece92535642d5dbfa7221770e88d9373",
    "candidate_at_measurement_identities": {
      "crates/wtime-runtime/src/execution.rs": {
        "live_sha256": "01fe8140bead06e070f79555839b6cb63d5f3d00f8095505b55f557b23c7d723",
        "matches": true,
        "sha256": "01fe8140bead06e070f79555839b6cb63d5f3d00f8095505b55f557b23c7d723"
      },
      "crates/wtime-runtime/src/lib.rs": {
        "live_sha256": "13357c4735da348c4471dc2360153b186ff30679029d36662eb290f3860866cb",
        "matches": true,
        "sha256": "13357c4735da348c4471dc2360153b186ff30679029d36662eb290f3860866cb"
      },
      "crates/wtime-runtime/src/shared_memory.rs": {
        "live_sha256": "f64f46b42620a79f83617038e949bfee387291411b2fe3e01e7c0572b0c3a24a",
        "matches": true,
        "sha256": "f64f46b42620a79f83617038e949bfee387291411b2fe3e01e7c0572b0c3a24a"
      },
      "tools/wtime-bench/target/release/wtime-bench": {
        "live_sha256": "087863cb4794d681286832679505324201d16d45a4a5d5aea0828fd8d25ee068",
        "matches": true,
        "sha256": "087863cb4794d681286832679505324201d16d45a4a5d5aea0828fd8d25ee068"
      }
    },
    "identity_note": "Candidate identities describe immutable retained sources/binary at measurement; current production sources were restored byte for byte."
  },
  "status": "done",
  "task": "P025",
  "tests": {
    "before_focused_corrected": {
      "failed": 0,
      "log": "/workspace/wtime/target/performance/P025-before-tests-corrected.log",
      "passed": 4,
      "skipped": 0
    },
    "candidate_counter_diagnostic": {
      "failed": 0,
      "log": "/workspace/wtime/target/performance/P025-candidate-counter-tests.log",
      "passed": 4,
      "skipped": 0
    },
    "candidate_focused": {
      "failed": 0,
      "log": "/workspace/wtime/target/performance/P025-candidate-tests.log",
      "passed": 4,
      "skipped": 0
    },
    "default_runtime": {
      "failed": 0,
      "log": "/workspace/wtime/target/performance/P025-runtime-tests.log",
      "passed": 172,
      "skipped": 0
    },
    "performance_integration": {
      "failed": 0,
      "log": "/workspace/wtime/target/performance/P025-performance-tests.log",
      "passed": 35,
      "skipped": 0,
      "features": [
        "execution-reference",
        "performance-counters"
      ],
      "diagnostic_only": true
    },
    "tooling": {
      "failed": 0,
      "log": "/workspace/wtime/target/performance/P025-tool-tests.log",
      "passed": 12,
      "skipped": 0
    },
    "rollback_default": {
      "passed": 172,
      "failed": 0,
      "skipped": 0
    },
    "rollback_diagnostic_performance": {
      "passed": 35,
      "failed": 0,
      "skipped": 0,
      "features": [
        "execution-reference",
        "performance-counters"
      ]
    },
    "final_tooling_with_real_driver": {
      "passed": 18,
      "failed": 0,
      "skipped": 0
    },
    "source_binding_regressions": {
      "passed": 5,
      "failed": 0,
      "skipped": 0
    },
    "declared_stage_coverage_regressions": {
      "passed": 4,
      "failed": 0,
      "skipped": 0
    }
  },
  "outcome": "rejected",
  "rollback": {
    "sources": {
      "task": "P025",
      "restored_runtime_sources": {
        "crates/wtime-runtime/src/lib.rs": {
          "restored_sha256": "997a0ec8d835c83567e13aa4c755ed30ad0e44a0bdb9e689c14753abdd47bb45",
          "archive_sha256": "997a0ec8d835c83567e13aa4c755ed30ad0e44a0bdb9e689c14753abdd47bb45",
          "byte_exact": true
        },
        "crates/wtime-runtime/src/shared_memory.rs": {
          "restored_sha256": "6b1a5a02e49d2d73ee04865af8d6bac68c60305f1afc754ca9b70252750bce44",
          "archive_sha256": "6b1a5a02e49d2d73ee04865af8d6bac68c60305f1afc754ca9b70252750bce44",
          "byte_exact": true
        },
        "crates/wtime-runtime/src/execution.rs": {
          "restored_sha256": "1baf160513d2daf53706f2bc03a54f7d01777b75da65656c5a6bedb4e995f4a5",
          "archive_sha256": "1baf160513d2daf53706f2bc03a54f7d01777b75da65656c5a6bedb4e995f4a5",
          "byte_exact": true
        }
      }
    },
    "default_build": {
      "clean_seconds": 6.69618398899911,
      "unchanged_seconds": 0.09252880400163122,
      "leaf_seconds": 0.43569611400016584,
      "runtime_leaf_seconds": 1.0917017179963295,
      "release_seconds": 13.8411040639985,
      "stripped_bytes": 1000264,
      "clean_peak_rss_kib": 286848,
      "source_stable": true,
      "budget_pass": true,
      "default_dependency_closure_pass": true
    },
    "candidate_retained": true,
    "production_reverted": true
  }
}
```
