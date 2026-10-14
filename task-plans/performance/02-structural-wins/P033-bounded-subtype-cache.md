# P033 — Cache immutable subtype answers with identity and work guards

Status: **done**

Outcome: **rejected**

Kind: `candidate` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P002 — Compare separately built baseline and candidate binaries](../00-measurement/P002-paired-driver.md)

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P011 — Measure reference, GC and exception paths](../00-measurement/P011-gc-exception-workloads.md)

[P016 — Add explicit executor policies and test integration hooks](../01-execution-contracts/P016-execution-policy.md)

[P029 — Stop cloning function signatures into every activation](P029-shared-function-metadata.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/canonical.rs`
- `crates/wtime-runtime/src/execution.rs`
- `crates/wtime-runtime/src/lib.rs`
- `crates/wtime-runtime/src/reference.rs`
- `crates/wtime-runtime/src/gc_exec.rs`
- `crates/wtime-runtime/src/linker.rs`
- `tools/wtime-bench/src/gc_warm_driver.rs`
- `tools/wtime-bench/gc_compare.py`
- `tools/wtime-bench/gc_cold_adapter.py`
- `tools/wtime-bench/gc_warm_workloads.py`
- `tools/wtime-bench/compare.py`
- `tools/wtime-bench/baseline.py`
- `tools/wtime-bench/workloads.json`
- `crates/wtime-runtime/tests/performance/p033.rs`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

A bounded Store-local cache keys checked Engine-bound actual/expected canonical IDs, exactness and immutable work policy; hits cannot bypass invalid identity or a lower work limit.

## Implementation recipe

1. Start with checked exact/equal-ID fast answers and memoize completed is_subtype queries; record both true and false results, never errors or incomplete walks.
2. Include configured max_type_work or required walk work in the entry so a hit cannot convert a formerly budget-limited query into success.
3. Bound entries with deterministic eviction and byte accounting; keep registry locking/chain traversal for misses and avoid slot-target caching in this patch.
4. Benchmark repeated indirect/call_ref/cast families and a type-pair-churn control, preserving canonical collision/equality tests.
5. Record the changed path with focused assertions/diagnostic counts and exact source/binary identities; compare the pre-edit archive, and revert a rejected structural optimization rather than adding a permanent toggle solely for testing.

## Meaningful cases

- Exactness and foreign Engine IDs cannot collide in the cache.
- A lower type-work budget still fails at the corresponding miss/hit policy.
- Many distinct type pairs keep bounded storage and changed table targets select the correct actual signature.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime p033_ -- --nocapture
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml -j 5
python3 tools/wtime-bench/compare.py --baseline target/performance/P033-before --candidate tools/wtime-bench/target/release/wtime-bench --candidate-mode decoded --selection gc-exceptions --pairs 30 --build-growth-explanation-file docs/benchmarks/performance/P033-build-decision.md --output target/performance/P033
python3 tools/wtime-bench/analyze.py --input target/performance/P033 --output target/performance/P033/analysis.json
python3 tools/checks/build_budget.py --output target/performance/P033-build.json
python3 tools/wtime-bench/gates.py --task P033 --comparison target/performance/P033 --build target/performance/P033-build.json --output target/performance/P033/decision.json
```

## Measurement and decision

Selection: `gc-exceptions`. Explicit execution setting: `decoded`. Decision rule: `measured_promotion`.

Use the paired protocol and family/control/cold/build rules in [MEASUREMENT.md](../MEASUREMENT.md). Predeclare the targeted family and held-out controls before changing the implementation. Keep instrumentation off in production timings. Adopt only with supported gains in both runs; otherwise record rejected/inconclusive and remove default-path cost. A necessary immutable-interface change may be contract_only with its separate justification.

## Before editing

Run these commands before changing the candidate sources. The archive is immutable and captures the immediate predecessor. Retain P001's original archive for milestone totals. For native work, use the named portable control and verify its actual policy/quantum; if this archive already exists with different sources, choose a new recorded run-specific directory and update the comparison path rather than overwriting it.

```sh
python3 tools/checks/build_budget.py --output target/performance/P033-before-build.json
python3 tools/wtime-bench/baseline.py --output target/performance/P033-before
```

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. Executed evidence is recorded below.

## Predeclared integration details

The candidate changes two focused mechanisms: a private bounded canonical-answer cache, and routing all seven checked Store subtype queries through it. This includes reference matching, casts and linker validation as well as indirect execution; the actual table target and handle checks stay ahead of every query. Cache storage is lazy, deterministic and capped at 64 slots and 8192 bytes including actual capacity and its header. Errors are never cached. Full Engine-bound IDs, exactness and the immutable work budget are keys; checked equal/exact queries bypass cache storage.

Before changing runtime production sources, install and validate the GC guest-stage measurement support, public regression cases and append-only registry. Preserve the original 45 cold GC/exception rows and append 36 fully encoded warm rows. The contract in docs/benchmarks/performance/P033-contract.json partitions 12 primary and 69 protected rows. Guest invocation intervals exclude and separately report every result check and collection reset; they are not end-to-end throughput. All 81 workloads must pass independent result and lifecycle checks before the predecessor archive. Then collect one quiet before-build sample, archive that immediate predecessor and implement the cache. The paired matrix has 450 groups and 13500 pairs across two 30-pair runs, with 72 shared-batch warm calibrations confirmed at 20 ms on both sides.

Use the measured build file target/performance/P033-build.json for the single quiet candidate sample and raw gate. All six relative build ratios must stay at or below 1.25; runtime-leaf time remains an extra measurement. No quiet sample or timing retry is allowed. Preserve failed tooling drafts and source-bound evidence; restore a non-adopted candidate with fresh modification times and execute restored checks. The original P001 archive remains immutable.

## Executed evidence

```json
{
  "artifacts": [
    {
      "bytes": 3017,
      "path": "/workspace/wtime/docs/benchmarks/performance/P033-build-decision.md",
      "sha256": "8db05578d91f7e9b7468c5c27cb491fc8d147d14373fce76f65213bf1fc8456d"
    },
    {
      "bytes": 26485,
      "path": "/workspace/wtime/docs/benchmarks/performance/P033-candidate.patch",
      "sha256": "ee5e6b1b688b699aa57478ba7fc72091c74f55087274e9d551bfb1ae292d68c4"
    },
    {
      "bytes": 2995,
      "path": "/workspace/wtime/docs/benchmarks/performance/P033-contract.json",
      "sha256": "99f9f88ea35fcc87c6339d9d91a179e7b900fb7aba2010faafaf41f8e27bf4d1"
    },
    {
      "bytes": 1349,
      "path": "/workspace/wtime/target/performance/P033-archive-hygiene/record.json",
      "sha256": "2034b1326d24c319e3d125684371e45291b88e481c4ad52e2097ccd9a064fc20"
    },
    {
      "bytes": 108607,
      "path": "/workspace/wtime/target/performance/P033-before-build.json",
      "sha256": "ef482d2cd3af23ae93dabf3806ba371abc42e19ea71f1e539a1ba2b87f54b879"
    },
    {
      "bytes": 67437,
      "path": "/workspace/wtime/target/performance/P033-before-corrected-command-record.json",
      "sha256": "1d044d84f7c58a7d358fa5ecd0207136d34127b5e87fd3fbead6dc287caa3a8f"
    },
    {
      "bytes": 50796,
      "path": "/workspace/wtime/target/performance/P033-before-corrected/manifest.json",
      "sha256": "d154ed03cd99ebffe86c97136981838c32371ccb961b8466c3248e260a3a2765"
    },
    {
      "bytes": 108608,
      "path": "/workspace/wtime/target/performance/P033-build.json",
      "sha256": "3f14437d79bc1082f84c6e724585fe45329df6503894fc7a42d6bd4920fe0035"
    },
    {
      "bytes": 1704,
      "path": "/workspace/wtime/target/performance/P033-candidate-checks-resume/candidate-build-budget.log",
      "sha256": "197891dbfcd75daff196921aa8d54411ad6d6a21984ef8f7b0e550a57d239079"
    },
    {
      "bytes": 76767,
      "path": "/workspace/wtime/target/performance/P033-candidate-checks-resume/commands.json",
      "sha256": "cc66f11d4bf5b139a64d06802b4b925b227bea9bf50390c4f8da66e39d4f4944"
    },
    {
      "bytes": 6141,
      "path": "/workspace/wtime/target/performance/P033-candidate-checks-resume/diagnostic_performance.log",
      "sha256": "0516c95c86ebaaf7c330eadc7c3eded34f493686940285739e92900ccb266d15"
    },
    {
      "bytes": 3084,
      "path": "/workspace/wtime/target/performance/P033-candidate-checks-resume/focused_diagnostic.log",
      "sha256": "2b31c9fe05c60952d21cfa667416123ff35a998403a37ef95558d268624e0b9d"
    },
    {
      "bytes": 16459,
      "path": "/workspace/wtime/target/performance/P033-candidate-checks-resume/full_default.log",
      "sha256": "83fde2146de343b13233300ab650c77f579155d0a26dfd5cd8fa4e74b404b17d"
    },
    {
      "bytes": 3084,
      "path": "/workspace/wtime/target/performance/P033-candidate-checks/focused_default.log",
      "sha256": "7f30f52ac822243b1b89ace6ce2e50e95931fecb8180cd2e49596c7503e65df1"
    },
    {
      "bytes": 17306368,
      "path": "/workspace/wtime/target/performance/P033-candidate/binary/wtime-bench",
      "sha256": "93e16937d4da2f1c90965cc5fbd8c73bb99b44734de1aa0e17e6819d818d6485"
    },
    {
      "bytes": 24493024,
      "path": "/workspace/wtime/target/performance/P033-candidate/binary/wtime-conformance",
      "sha256": "5123b38fd1e6b2358027778d94256202e53ff7024b3b05cd94aa5ea50d7949d4"
    },
    {
      "bytes": 3552,
      "path": "/workspace/wtime/target/performance/P033-candidate/retention.json",
      "sha256": "d67be6fcd0b378d0fe38a1442bba77a4362492357dfb83e98e9ef21b12366b5f"
    },
    {
      "bytes": 19201,
      "path": "/workspace/wtime/target/performance/P033-candidate/source-manifest.json",
      "sha256": "6d6996d13a684098870baea52eca24581580ab3f54e15ccf3ce2176b24b2b7e3"
    },
    {
      "bytes": 0,
      "path": "/workspace/wtime/target/performance/P033-comparison-run.log",
      "sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    },
    {
      "bytes": 1550,
      "path": "/workspace/wtime/target/performance/P033-incremental-build-decision.json",
      "sha256": "6cab78b5edbee04b6ee57d0cfbc7144aeddafe75bb856b407d1f4eb46df5dfe3"
    },
    {
      "bytes": 2439,
      "path": "/workspace/wtime/target/performance/P033-measure-corrected-command-record.json",
      "sha256": "afaf7ed5e124067e0c932e84dbb0b1e3d1964f573953d0a694f876acef9ca20e"
    },
    {
      "bytes": 2718,
      "path": "/workspace/wtime/target/performance/P033-measure-corrected-executed.py",
      "sha256": "9f438edd2dd9990e643a24ce8e5493b7834739006558f71b196bd34c0b90ac36"
    },
    {
      "bytes": 108605,
      "path": "/workspace/wtime/target/performance/P033-restored-build.json",
      "sha256": "87af2999f06ce431da053b03820eb29c106fa8fab9bd84e4b4fc9047d57eb89f"
    },
    {
      "bytes": 28109,
      "path": "/workspace/wtime/target/performance/P033-retain-candidate-premeasure-corrected.snapshot.py",
      "sha256": "af2566082fb0529092d059d12fb3952394754fabefb057b11c76399f0495511d"
    },
    {
      "bytes": 27939,
      "path": "/workspace/wtime/target/performance/P033-retain-candidate-premeasure.snapshot.py",
      "sha256": "366471a4abcdafe2c3d92fdadeda879c2e6b57ae44c25a36ea25585f57648339"
    },
    {
      "bytes": 75,
      "path": "/workspace/wtime/target/performance/P033-retention-corrected-run.log",
      "sha256": "f7a7d128a7e1916786de46e27649c9832a6e9cd9f1c2d45759ce61404e42bfe4"
    },
    {
      "bytes": 94,
      "path": "/workspace/wtime/target/performance/P033-retention-run.log",
      "sha256": "59b63d34dadd2975e2428f11e226fc611cfcc72fd96ee4696d213dc636b2c9e0"
    },
    {
      "bytes": 2743,
      "path": "/workspace/wtime/target/performance/P033-strict-build-executed.py",
      "sha256": "617d88287ef7f4a09caa3724005bda724011c02394f4ab0b4f9dbbd8f5d33fd2"
    },
    {
      "bytes": 698046,
      "path": "/workspace/wtime/target/performance/P033/analysis.json",
      "sha256": "c78b346c40cc5d1a1950f2cb15d4ce45989cc41090b4bcc7cc7bb142a8b964c3"
    },
    {
      "bytes": 0,
      "path": "/workspace/wtime/target/performance/P033/analysis.log",
      "sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    },
    {
      "bytes": 48095163,
      "path": "/workspace/wtime/target/performance/P033/checkpoint.jsonl",
      "sha256": "9e85ecd3a5127081ed231d341d5e98a9a8c1bd8fe0734c03a10356bef42843d3"
    },
    {
      "bytes": 9501,
      "path": "/workspace/wtime/target/performance/P033/commands.json",
      "sha256": "1ec50c7053443396da67a890b490d12d80d914aa527207698587ae44df8091d0"
    },
    {
      "bytes": 937877,
      "path": "/workspace/wtime/target/performance/P033/decision.json",
      "sha256": "952010ace0caeea0b3b80f5ef260bec420652214ac8cd2f755fc321581a96cc5"
    },
    {
      "bytes": 4446,
      "path": "/workspace/wtime/target/performance/P033/extraction.json",
      "sha256": "9456f342c7fe74e7129ce0729453c8507d48ee1565ffb225c3d9c293955f5b45"
    },
    {
      "bytes": 3810,
      "path": "/workspace/wtime/target/performance/P033/extraction.log",
      "sha256": "575acd5b0d9c623a0888e96085b77fd3dcc8fc35591288d093b3248c16720fd3"
    },
    {
      "bytes": 74,
      "path": "/workspace/wtime/target/performance/P033/raw-exits.json",
      "sha256": "aa4c00e52934794da9ea68d13b59e7ce7639b5a0f4c9b6efdc8e97dad0920d37"
    },
    {
      "bytes": 937877,
      "path": "/workspace/wtime/target/performance/P033/raw-gate.log",
      "sha256": "952010ace0caeea0b3b80f5ef260bec420652214ac8cd2f755fc321581a96cc5"
    },
    {
      "bytes": 79958310,
      "path": "/workspace/wtime/target/performance/P033/report.json",
      "sha256": "56a969b6ab813b3c6b30ee8495d44a740f3979cd804dd2af76844f5047be23a5"
    },
    {
      "bytes": 5627,
      "path": "/workspace/wtime/target/performance/P033/restore-commands.json",
      "sha256": "b14d4d98dc4be03d8d71b54eb2e8a8e199d3d4aa970ab3d428a289418f1268c7"
    },
    {
      "bytes": 6450,
      "path": "/workspace/wtime/target/performance/P033/restore-input-identities.json",
      "sha256": "90ad9a83ffd942943b305b22de8faf8f4afe5ac5a132bf6ba416eafe63404235"
    },
    {
      "bytes": 206,
      "path": "/workspace/wtime/target/performance/P033/restored-benchmark-build.log",
      "sha256": "cd468809f12bdd031b76e54afc739e87ba155107b21664bc2350f40499230845"
    },
    {
      "bytes": 1701,
      "path": "/workspace/wtime/target/performance/P033/restored-build-budget.log",
      "sha256": "88b5d3ca6c69b8ac28299844a103d3d0d6e774186e08fb25fb6609fc565fd926"
    },
    {
      "bytes": 15851,
      "path": "/workspace/wtime/target/performance/P033/restored-default-tests.log",
      "sha256": "8de6a363fab678bcde02e9a924aad872080b68632b324af9f96a1531c282aa49"
    },
    {
      "bytes": 6215,
      "path": "/workspace/wtime/target/performance/P033/restored-diagnostic-tests.log",
      "sha256": "7052cb563a2717b4bb5620a0e3b2abcf9f276d9ea294f8c14ac5d0a47bae015b"
    },
    {
      "bytes": 599,
      "path": "/workspace/wtime/target/performance/P033/restored-focused-default-tests.log",
      "sha256": "05ab3419321407fbd54cd1612b6301f1d949416c594f6507047e4ea00b5d57d5"
    },
    {
      "bytes": 599,
      "path": "/workspace/wtime/target/performance/P033/restored-focused-diagnostic-tests.log",
      "sha256": "a823dcfe0350a444f65d5572382a6976503fba1c45a8a76223ea134b56c5b0ea"
    },
    {
      "bytes": 8288,
      "path": "/workspace/wtime/target/performance/P033/rollback-identities.json",
      "sha256": "f0424c4c6cf7d8c7d1a157a5f390ce6850e761de71d5e6bac257028cafce45d8"
    },
    {
      "bytes": 22024,
      "path": "/workspace/wtime/target/performance/baseline/manifest.json",
      "sha256": "7349596fc806c96c74cb9c40f1c08402589c1683660b0b152c713264f5808472"
    },
    {
      "bytes": 629,
      "path": "/workspace/wtime/target/performance/P033/restore-initial-guard-failure.json",
      "sha256": "99b99eab5e1be3cbcd94ec1bb5a45dccfdd7a29ee0d018f1d6be145e4002ea78"
    },
    {
      "bytes": 45272,
      "path": "/workspace/wtime/target/performance/P033/restore-initial-guard.executed.py",
      "sha256": "a03a916fe09394a266d53e4c552e78a8a78129e4fbb14341046d4fcf3c1c22ee"
    },
    {
      "bytes": 430,
      "path": "/workspace/wtime/target/performance/P033/restore-second-guard-failure.json",
      "sha256": "86f907a43b5dc64ac142d832dbecd5a3e6423725d5329f53b5ce2deab783788e"
    },
    {
      "bytes": 45512,
      "path": "/workspace/wtime/target/performance/P033/restore-corrected-command-source.py",
      "sha256": "9245dd341c9b21b825b55ab94d3e303eb8f14f4658b333fce33b31d5ebc479af"
    },
    {
      "bytes": 45746,
      "path": "/workspace/wtime/target/performance/P033/restore-corrected2-command-source.py",
      "sha256": "4c24e047a7534cf872706a1a6b0a967d9a39313f0bb93cc448d0617b059a691d"
    },
    {
      "bytes": 8861,
      "path": "/workspace/wtime/target/performance/P033/final-decision.json",
      "sha256": "5cb7bf1750b3cdb11895ef22d475773a1d135cde166233cd4fe6dc57137546f4"
    },
    {
      "bytes": 8861,
      "path": "/workspace/wtime/docs/benchmarks/performance/P033-decision.json",
      "sha256": "5cb7bf1750b3cdb11895ef22d475773a1d135cde166233cd4fe6dc57137546f4"
    },
    {
      "bytes": 1507,
      "path": "/workspace/wtime/docs/benchmarks/performance/P033-extraction.json",
      "sha256": "40a8cfbefaf2ba6f0d6d72bd98063998b3da96ca5415be3c7097851c616a961d"
    },
    {
      "bytes": 1203,
      "path": "/workspace/wtime/docs/benchmarks/performance/P033-strict-build.json",
      "sha256": "ea1f92a89e9e8dba9944ef109c1077833ecb6ae1749ff104ffee01b8fd777fec"
    },
    {
      "bytes": 5113,
      "path": "/workspace/wtime/docs/benchmarks/performance/P033-rollback.json",
      "sha256": "73659d627d9c44b678bd49c40ea38deb60d02faad5179b62983b8f37d814bb7d"
    },
    {
      "bytes": 1727,
      "path": "/workspace/wtime/docs/benchmarks/performance/P033-evidence.md",
      "sha256": "2504e587f2f7b30681fc0e89a82fc40f662ae1ada04b91359db6ca899ddd9f39"
    },
    {
      "path": "/workspace/wtime/target/performance/P033/publish-evidence.executed.py",
      "sha256": "fc3cfa6e3dbcebbb75b90acb4a51e5886e81d62a08120be0298998d6b4020f28",
      "bytes": 14681
    }
  ],
  "baseline": {
    "archive_hygiene": {
      "bytes": 1349,
      "path": "/workspace/wtime/target/performance/P033-archive-hygiene/record.json",
      "sha256": "2034b1326d24c319e3d125684371e45291b88e481c4ad52e2097ccd9a064fc20"
    },
    "before_public_preflight": {
      "bytes": 4624,
      "path": "/workspace/wtime/target/performance/P033-public-preflight/corrected-report.json",
      "sha256": "42bc67c83696bcb6e2b8a166371f6ebcd96852c2b3cb4767f83c8ebffbdaf6ed"
    },
    "p001_manifest_sha256": "7349596fc806c96c74cb9c40f1c08402589c1683660b0b152c713264f5808472",
    "p033_manifest_sha256": "d154ed03cd99ebffe86c97136981838c32371ccb961b8466c3248e260a3a2765"
  },
  "candidate": {
    "binary": {
      "bytes": 17306368,
      "path": "/workspace/wtime/target/performance/P033-candidate/binary/wtime-bench",
      "sha256": "93e16937d4da2f1c90965cc5fbd8c73bb99b44734de1aa0e17e6819d818d6485"
    },
    "encoder": {
      "bytes": 24493024,
      "path": "/workspace/wtime/target/performance/P033-candidate/binary/wtime-conformance",
      "sha256": "5123b38fd1e6b2358027778d94256202e53ff7024b3b05cd94aa5ea50d7949d4"
    },
    "patch": {
      "bytes": 26485,
      "path": "/workspace/wtime/docs/benchmarks/performance/P033-candidate.patch",
      "sha256": "ee5e6b1b688b699aa57478ba7fc72091c74f55087274e9d551bfb1ae292d68c4"
    },
    "retention": {
      "bytes": 3552,
      "path": "/workspace/wtime/target/performance/P033-candidate/retention.json",
      "sha256": "d67be6fcd0b378d0fe38a1442bba77a4362492357dfb83e98e9ef21b12366b5f"
    },
    "runtime_delta": [
      "crates/wtime-runtime/src/canonical.rs",
      "crates/wtime-runtime/src/execution.rs",
      "crates/wtime-runtime/src/gc_exec.rs",
      "crates/wtime-runtime/src/lib.rs",
      "crates/wtime-runtime/src/linker.rs",
      "crates/wtime-runtime/src/reference.rs"
    ],
    "source_manifest": {
      "bytes": 19201,
      "path": "/workspace/wtime/target/performance/P033-candidate/source-manifest.json",
      "sha256": "6d6996d13a684098870baea52eca24581580ab3f54e15ccf3ce2176b24b2b7e3"
    }
  },
  "candidate_promoted": false,
  "comparison": {
    "bytes": 79958310,
    "path": "/workspace/wtime/target/performance/P033/report.json",
    "sha256": "56a969b6ab813b3c6b30ee8495d44a740f3979cd804dd2af76844f5047be23a5"
  },
  "finalizer": {
    "snapshot": {
      "bytes": 40003,
      "path": "/tmp/P033-complete.attempt-1.executed.py",
      "sha256": "54d6e438553ec36a98f213953bc05fecd813a7541b9d7c41b4de2c224b872ab5"
    },
    "source_sha256": "54d6e438553ec36a98f213953bc05fecd813a7541b9d7c41b4de2c224b872ab5"
  },
  "measurement_command": {
    "corrected_retention_helper": {
      "bytes": 28109,
      "path": "/workspace/wtime/target/performance/P033-retain-candidate-premeasure-corrected.snapshot.py",
      "sha256": "af2566082fb0529092d059d12fb3952394754fabefb057b11c76399f0495511d"
    },
    "corrected_retention_log": {
      "bytes": 75,
      "path": "/workspace/wtime/target/performance/P033-retention-corrected-run.log",
      "sha256": "f7a7d128a7e1916786de46e27649c9832a6e9cd9f1c2d45759ce61404e42bfe4"
    },
    "failed_retention_guard_log": {
      "bytes": 94,
      "path": "/workspace/wtime/target/performance/P033-retention-run.log",
      "sha256": "59b63d34dadd2975e2428f11e226fc611cfcc72fd96ee4696d213dc636b2c9e0"
    },
    "failed_retention_helper": {
      "bytes": 27939,
      "path": "/workspace/wtime/target/performance/P033-retain-candidate-premeasure.snapshot.py",
      "sha256": "366471a4abcdafe2c3d92fdadeda879c2e6b57ae44c25a36ea25585f57648339"
    },
    "helper": {
      "bytes": 2718,
      "path": "/workspace/wtime/target/performance/P033-measure-corrected-executed.py",
      "sha256": "9f438edd2dd9990e643a24ce8e5493b7834739006558f71b196bd34c0b90ac36"
    },
    "record": {
      "bytes": 2439,
      "path": "/workspace/wtime/target/performance/P033-measure-corrected-command-record.json",
      "sha256": "afaf7ed5e124067e0c932e84dbb0b1e3d1964f573953d0a694f876acef9ca20e"
    },
    "release_build_log": {
      "bytes": 206,
      "path": "/workspace/wtime/target/performance/P033-candidate-benchmark-build.log",
      "sha256": "dd5f2f12f4a68765f7f595bbefc61e98cc324c95f2a4053603c884e2147c3fb7"
    }
  },
  "outcome": "rejected",
  "pairing": {
    "correctness_probes": 162,
    "groups": 450,
    "observations": 27000,
    "pairs": 13500,
    "primary": 12,
    "protected": 69,
    "warm_calibrations": 72
  },
  "published_evidence": {
    "/workspace/wtime/docs/benchmarks/performance/P033-decision.json": {
      "bytes": 8861,
      "sha256": "5cb7bf1750b3cdb11895ef22d475773a1d135cde166233cd4fe6dc57137546f4"
    },
    "/workspace/wtime/docs/benchmarks/performance/P033-evidence.md": {
      "bytes": 1727,
      "sha256": "2504e587f2f7b30681fc0e89a82fc40f662ae1ada04b91359db6ca899ddd9f39"
    },
    "/workspace/wtime/docs/benchmarks/performance/P033-extraction.json": {
      "bytes": 1507,
      "sha256": "40a8cfbefaf2ba6f0d6d72bd98063998b3da96ca5415be3c7097851c616a961d"
    },
    "/workspace/wtime/docs/benchmarks/performance/P033-rollback.json": {
      "bytes": 5113,
      "sha256": "73659d627d9c44b678bd49c40ea38deb60d02faad5179b62983b8f37d814bb7d"
    },
    "/workspace/wtime/docs/benchmarks/performance/P033-strict-build.json": {
      "bytes": 1203,
      "sha256": "ea1f92a89e9e8dba9944ef109c1077833ecb6ae1749ff104ffee01b8fd777fec"
    },
    "/workspace/wtime/target/performance/P033/final-decision.json": {
      "bytes": 8861,
      "sha256": "5cb7bf1750b3cdb11895ef22d475773a1d135cde166233cd4fe6dc57137546f4"
    }
  },
  "raw_gate": {
    "analysis_groups": 450,
    "decision": "rejected",
    "exit_code": 1
  },
  "reason": "Strict six-metric build gate failed; candidate rejected.",
  "resolution": {
    "post_restore_build": {
      "bytes": 108605,
      "path": "/workspace/wtime/target/performance/P033-restored-build.json",
      "sha256": "87af2999f06ce431da053b03820eb29c106fa8fab9bd84e4b4fc9047d57eb89f"
    },
    "restore_commands": {
      "bytes": 5627,
      "path": "/workspace/wtime/target/performance/P033/restore-commands.json",
      "sha256": "b14d4d98dc4be03d8d71b54eb2e8a8e199d3d4aa970ab3d428a289418f1268c7"
    },
    "restored_tests": {
      "default": 219,
      "diagnostic_performance": 89,
      "focused_default": 4,
      "focused_diagnostic": 4
    },
    "rollback": {
      "bytes": 8288,
      "path": "/workspace/wtime/target/performance/P033/rollback-identities.json",
      "sha256": "f0424c4c6cf7d8c7d1a157a5f390ce6850e761de71d5e6bac257028cafce45d8"
    }
  },
  "speedup_claim": null,
  "status": "done",
  "strict_build_gate": {
    "checks": {
      "default_features": true,
      "hard_budgets": true,
      "passed": false,
      "ratios_under_1_25": false,
      "same_settings": true,
      "stable": true,
      "zero_default_dependencies": true
    },
    "input_hashes": {
      "before": "ef482d2cd3af23ae93dabf3806ba371abc42e19ea71f1e539a1ba2b87f54b879",
      "candidate": "3f14437d79bc1082f84c6e724585fe45329df6503894fc7a42d6bd4920fe0035",
      "policy": "8db05578d91f7e9b7468c5c27cb491fc8d147d14373fce76f65213bf1fc8456d"
    },
    "metrics": {
      "clean_peak_rss_kib": 0.9917718202546345,
      "clean_seconds": 0.9161518829576989,
      "leaf_seconds": 1.42966102255939,
      "release_seconds": 0.34593188559126475,
      "stripped_bytes": 1.0013831700979412,
      "unchanged_peak_rss_kib": 1.0111950364175883
    },
    "passed": false
  },
  "task": "P033"
}
```
