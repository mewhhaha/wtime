# P032 — Move guest arguments and tail calls without split_off allocation

Status: **done**

Outcome: **rejected**

Kind: `candidate` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P002 — Compare separately built baseline and candidate binaries](../00-measurement/P002-paired-driver.md)

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P008 — Add scaling, branch and call benchmark families](../00-measurement/P008-scalar-workloads.md)

[P016 — Add explicit executor policies and test integration hooks](../01-execution-contracts/P016-execution-policy.md)

[P024 — Move branch results and share label metadata without allocation](P024-in-place-branch-results.md)

[P030 — Split checked public entry from validated guest call entry](P030-validated-guest-frame-entry.md)

[P031 — Reuse invocation scratch capacity safely under host reentry](P031-activation-scratch-pool.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/execution.rs`
- `crates/wtime-runtime/tests/performance.rs`
- `crates/wtime-runtime/tests/performance/p032.rs`
- `docs/benchmarks/performance/P032-candidate.patch`
- `docs/benchmarks/performance/P032-contract.json`
- `docs/benchmarks/performance/P032-build-decision.md`
- `docs/benchmarks/performance/P032-evidence.json`

One integrator owns the guest argument-range change in execution.rs, its private path proof, and public test registration; rebase after the final P031 decision because dispatch/scratch ownership still affects this change even if P031 is rejected.

## Input/output contract

Validated guest calls read/copy arguments from an existing stack range into activation-local storage; tail calls replace logical frame state while preserving argument overlap, limits and roots. Host calls keep an owned/rooted safe boundary.

## Implementation recipe

1. Replace guest-call stack.split_off with a checked argument range and pre-reserved callee locals; truncate operands only after successful setup.
2. Handle tail-call argument/local overlap via safe copy ordering or a bounded reusable scratch slice; preserve logical tail-depth semantics.
3. Keep host arguments alive/rooted independently of mutable Store reentry and leave indirect target validation intact.
4. Measure tiny/deep/direct/tail call families and confirm allocation events vanish only on the intended guest path.
5. Record the changed path with focused assertions/diagnostic counts and exact source/binary identities; compare the pre-edit archive, and revert a rejected structural optimization rather than adding a permanent toggle solely for testing.

## Meaningful cases

- Zero/multivalue arguments and overlapping tail arguments preserve order.
- Frame allocation/depth failure does not partially consume or publish guest state.
- Host reentry, caught exceptions and continuation capture retain all argument/local references.

## Verification

Write the current build-budget report to `target/performance/P032-build.json` before starting the comparison. The comparison output directory must not exist at launch; the final gate reads the root-level build report.

P031 remains a prerequisite even if its measurement rejects the scratch-pool change: rebase the P032 guest-call range against the resulting invocation/dispatch code before editing.

Planned candidate-path tests: eight default private tests and nine with the diagnostic `performance-counters` feature. The public-API predecessor draft has four default tests and a fifth under `execution-reference`; these assert behavior only and do not prove argument-range execution. P031’s exact shared 85-workload/170-record calibration is inherited unchanged; no benchmark tooling or calibration rule is planned here. All counts are plans, not test results.

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime p032_ -- --nocapture
cargo test --locked -p wtime-runtime --features performance-counters p032_ -- --nocapture
cargo test --locked -p wtime-runtime --features execution-reference,performance-counters p032_ -- --nocapture
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml -j 4
python3 tools/checks/build_budget.py --output target/performance/P032-build.json
python3 tools/wtime-bench/compare.py --baseline target/performance/P032-before --candidate tools/wtime-bench/target/release/wtime-bench --candidate-mode decoded --selection scalar --pairs 30 --build-growth-explanation-file docs/benchmarks/performance/P032-build-decision.md --output target/performance/P032
python3 tools/wtime-bench/analyze.py --input target/performance/P032 --output target/performance/P032/analysis.json
python3 tools/wtime-bench/gates.py --task P032 --comparison target/performance/P032 --build target/performance/P032-build.json --contract docs/benchmarks/performance/P032-contract.json --output target/performance/P032/decision.json
```

## Measurement and decision

Selection: `scalar`. Explicit execution setting: `decoded`. Decision rule: `measured_promotion`.

Use the paired protocol and family/control/cold/build rules in [MEASUREMENT.md](../MEASUREMENT.md). Predeclare the targeted family and held-out controls before changing the implementation. Keep instrumentation off in production timings. Adopt only with supported gains in both runs; otherwise record rejected/inconclusive and remove default-path cost. A necessary immutable-interface change may be contract_only with its separate justification.

## Before editing

Run these commands before changing the candidate sources. The archive is immutable and captures the immediate predecessor. Retain P001's original archive for milestone totals. For native work, use the named portable control and verify its actual policy/quantum; if this archive already exists with different sources, choose a new recorded run-specific directory and update the comparison path rather than overwriting it.

```sh
python3 tools/wtime-bench/baseline.py --output target/performance/P032-before
```

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. Executed evidence is recorded below.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/docs/benchmarks/performance/P031-evidence.json": {
      "sha256": "bacd17f95e897f0e7765771f9815f61f095de9b9a9a3dbce8eac8c4658734bef",
      "bytes": 1388485
    },
    "/workspace/wtime/docs/benchmarks/performance/P032-build-decision.md": {
      "sha256": "56832a2eca477eadb1661829e8fdc48e5d4aec5019064ff5d287b4ddf88d6537",
      "bytes": 3145
    },
    "/workspace/wtime/docs/benchmarks/performance/P032-candidate.patch": {
      "sha256": "7dbba6380988c4028b367b15f5cc170cf0d46415c36dfbeabb5a0c15c6a36bef",
      "bytes": 13834
    },
    "/workspace/wtime/docs/benchmarks/performance/P032-contract.json": {
      "sha256": "7529b639b19822f51a990ce265c6f81e639ffd2ef14705a3497032701445d3ee",
      "bytes": 2757
    },
    "/workspace/wtime/docs/benchmarks/performance/P032-evidence.json": {
      "sha256": "38e50bfcaf8b335535a2dc9f7fcaeb25dc6cb8d90a4b11dc29e0baa4e0a91412",
      "bytes": 1441822
    },
    "/workspace/wtime/target/performance/P031/restored-default-tests.log": {
      "sha256": "eeb778cb5449fe81a6b71f85dba2d438f72bb9e16ae91a7a74806a5873066879",
      "bytes": 15202
    },
    "/workspace/wtime/target/performance/P031/restored-diagnostic-tests.log": {
      "sha256": "6b4484529b6be3aeaff9e51ff311ab79de8fcc6b4ed44184cabc6dd6debb6508",
      "bytes": 5487
    },
    "/workspace/wtime/target/performance/P032-before-build.json": {
      "sha256": "04b548f10ac97c3bcf17b80d971ca4e8e0a07877bd433b954a19cd030dab9b1f",
      "bytes": 106542
    },
    "/workspace/wtime/target/performance/P032-before/manifest.json": {
      "sha256": "02c5f98b5a9492dd532ce86eb473d2ae265c95437e8ab53dc4108cc2c85ec316",
      "bytes": 38138
    },
    "/workspace/wtime/target/performance/P032-build.json": {
      "sha256": "61481dea21134d6bba5b8deab314fcc45eeb3fbc478b42beb6c92ad0136dcecd",
      "bytes": 106543
    },
    "/workspace/wtime/target/performance/P032-candidate-checks/diagnostic_performance.log": {
      "sha256": "b03ffce672a4cad4fcaecb3113536c3e85ff1ac5ea964f7cc9f5ea30671dd611",
      "bytes": 5812
    },
    "/workspace/wtime/target/performance/P032-candidate-checks/focused_default.log": {
      "sha256": "834f0c3cfda4a94712be563f724f5e567b3da5b4cd38cddb75142fd2cf8d2120",
      "bytes": 2242
    },
    "/workspace/wtime/target/performance/P032-candidate-checks/focused_diagnostic.log": {
      "sha256": "12a3e77ac9d2fab2be3b4cb62e0083c1d615efc0be3c1fa4eba55bc85f964379",
      "bytes": 2319
    },
    "/workspace/wtime/target/performance/P032-candidate-checks/full_default.log": {
      "sha256": "6bc3b1872445cd2e3ccf3f1999dd28fc4b6c3452eb540451ca14098d673ee2c4",
      "bytes": 15907
    },
    "/workspace/wtime/target/performance/P032-candidate-retained/source-manifest.json": {
      "sha256": "51eac715a502e4781e7b567fd428978227a79e26f52de906d2662534c47659eb",
      "bytes": 15375
    },
    "/workspace/wtime/target/performance/P032-incremental-build-decision.json": {
      "sha256": "f540cfa80a8e7f1ba1edc75a4ede49066f24f0afa757ea8736f3fed19252a88f",
      "bytes": 1417
    },
    "/workspace/wtime/target/performance/P032-preflight/default.log": {
      "sha256": "7e9bd46df8af41d6e2619eef89a4de1f667e640f601e8328920145f312e87f1d",
      "bytes": 1856
    },
    "/workspace/wtime/target/performance/P032-preflight/diagnostic.log": {
      "sha256": "67c8313cd168bd1a0ae828591bfc7b4561fe261b0a46f9980bd00e494b71bda8",
      "bytes": 1934
    },
    "/workspace/wtime/target/performance/P032/analysis.json": {
      "sha256": "9bd9d3d47f5ae008023218146cb8b403e0db6cdbeb8f0c5bbec048035a35c4ae",
      "bytes": 1265020
    },
    "/workspace/wtime/target/performance/P032/analysis.log": {
      "sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
      "bytes": 0
    },
    "/workspace/wtime/target/performance/P032/checkpoint.jsonl": {
      "sha256": "b835e5991537be83e8309ba4b28d31b3c0145650f90dfc1ecbb73a676f4f7091",
      "bytes": 36221301
    },
    "/workspace/wtime/target/performance/P032/commands.json": {
      "sha256": "c9a19cc8ef0d9b169961af6ba14b54f6df08e8ef0582a5d8cc1fefa1bbe463e2",
      "bytes": 12344
    },
    "/workspace/wtime/target/performance/P032/decision.json": {
      "sha256": "f6f186d070a156fab1cfba9066c15f5b380b1d13998e0d62b4704812ddc9e866",
      "bytes": 1692112
    },
    "/workspace/wtime/target/performance/P032/development-provenance.json": {
      "sha256": "80e19b68b3c79b7a34c15def5959de47ad99d985ba767a882cbfe552852a1a07",
      "bytes": 510053
    },
    "/workspace/wtime/target/performance/P032/extraction.json": {
      "sha256": "395158467e8f5d38d67315b8f1a642c2b72239a2c874b2f181caba09dcd1676f",
      "bytes": 499736
    },
    "/workspace/wtime/target/performance/P032/extraction.log": {
      "sha256": "8c36cdfbaa3dd44dc027a5d850c9f727f8cc1eeb00cedd27aa7d2f3c826aaa87",
      "bytes": 314737
    },
    "/workspace/wtime/target/performance/P032/final-decision.json": {
      "sha256": "4554f80c036d364d8e6ec3822a3f4d74c799e54351e8fabe73ed27624c18eafa",
      "bytes": 11869
    },
    "/workspace/wtime/target/performance/P032/raw-exits.json": {
      "sha256": "fd7c49ee3eb6283187d0558584cc595d11270dba06581f6993ab04551e8275eb",
      "bytes": 131
    },
    "/workspace/wtime/target/performance/P032/raw-gate.log": {
      "sha256": "f6f186d070a156fab1cfba9066c15f5b380b1d13998e0d62b4704812ddc9e866",
      "bytes": 1692112
    },
    "/workspace/wtime/target/performance/P032/report.json": {
      "sha256": "a70f150636b6f6a60db5ae3592454cb68b8469f724a727e1f61e2a73e5785f2e",
      "bytes": 60179014
    },
    "/workspace/wtime/target/performance/P032/restored-build.json": {
      "sha256": "4d29ad6391ce5444f3799f24bd00fc5189e2bcf8ed7d9507a7f0c4186577be2d",
      "bytes": 106538
    },
    "/workspace/wtime/target/performance/P032/restored-default-tests.log": {
      "sha256": "2d7b81883f26f25b4a9f1eb964e6d59eeb844564391d2f703143d70d36ad8c62",
      "bytes": 15522
    },
    "/workspace/wtime/target/performance/P032/restored-diagnostic-tests.log": {
      "sha256": "f144e3692cf3ff0f3494c6c4dbcd43e3b5c631d056340add54a1b466c3f8dfaf",
      "bytes": 5886
    },
    "/workspace/wtime/target/performance/P032/restored-focused-default-tests.log": {
      "sha256": "c6eb9c579811ec1cefeb2a490a66d1d6ba045823c79efc17c572b32a677e9af1",
      "bytes": 1782
    },
    "/workspace/wtime/target/performance/P032/restored-focused-diagnostic-tests.log": {
      "sha256": "e3448fadc22c20b500aab4c0ff3be003173b14b38eea7111db6e95bbddca500d",
      "bytes": 1934
    },
    "/workspace/wtime/target/performance/P032/rollback-identities.json": {
      "sha256": "ef24c0e553fb89a15c720210115a4bea824df3b4dd86591eaa1e7c66acc34b48",
      "bytes": 7462
    },
    "/workspace/wtime/target/performance/P032/task-precompletion/P032-call-argument-ranges.md": {
      "sha256": "45acf36a90d5c52cdd76aacaea46a8d2c6014daf75c9cbeab7a7d8966b65a9c0",
      "bytes": 6985
    },
    "/workspace/wtime/target/performance/P032/task-precompletion/TASKS.json": {
      "sha256": "03905f86cd0e9487ec0048c9fdf8a73b5696f57c4da1241646e2c5c31f96b246",
      "bytes": 2397675
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/000-P032-complete.py": {
      "sha256": "b2376e82232db6739c57b87a715f6e6ea0b633fdbe8d435196a3cc616d350733",
      "bytes": 58850
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/001-P032-finalize-command.py": {
      "sha256": "91c12873f9d05807ddc35970cfad84e602c5e409e9998ddaa71ee25da3363c3e",
      "bytes": 10122
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/002-P032-complete-help.txt": {
      "sha256": "9fa88f9ad528ac01a442429763e5fa79ad262e2452cad462e14741a3b6c8efe1",
      "bytes": 4492
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/003-P032-complete-pre-parent.placeholder.py": {
      "sha256": "7c4b4573f33189ed8a845a5f9f09f4f58e9ae6bdd4a1c86644d13e6c77e53d3d",
      "bytes": 24
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/004-P032-complete.placeholder.py": {
      "sha256": "7c4b4573f33189ed8a845a5f9f09f4f58e9ae6bdd4a1c86644d13e6c77e53d3d",
      "bytes": 24
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/005-P032-finalize-command-help.txt": {
      "sha256": "907c646ea2f394168e3dd21bb9b8ad5c50e702f74bca3eb7750ec3adb0311dcc",
      "bytes": 1086
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/006-P032-finalize-command-review.placeholder.md": {
      "sha256": "7c4b4573f33189ed8a845a5f9f09f4f58e9ae6bdd4a1c86644d13e6c77e53d3d",
      "bytes": 24
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/007-P032-finalize-command.byte-exact.py": {
      "sha256": "27921d4cae0d3b48d1c0ee4730a40b7346c546ac50968592f05545609cf24b49",
      "bytes": 10133
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/008-P032-finalize-command.placeholder.py": {
      "sha256": "7c4b4573f33189ed8a845a5f9f09f4f58e9ae6bdd4a1c86644d13e6c77e53d3d",
      "bytes": 24
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/009-P032-parent-counts-review.placeholder.md": {
      "sha256": "7c4b4573f33189ed8a845a5f9f09f4f58e9ae6bdd4a1c86644d13e6c77e53d3d",
      "bytes": 24
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/010-P032-restore-pre-parent-shape.placeholder.py": {
      "sha256": "7c4b4573f33189ed8a845a5f9f09f4f58e9ae6bdd4a1c86644d13e6c77e53d3d",
      "bytes": 24
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/011-P032-restore.placeholder.py": {
      "sha256": "7c4b4573f33189ed8a845a5f9f09f4f58e9ae6bdd4a1c86644d13e6c77e53d3d",
      "bytes": 24
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/012-actual-artifact-compatibility-review.md": {
      "sha256": "6cd57a287e178af78e68b382658f3b52525af6cef4368bc047ae5cab7a2fb8e5",
      "bytes": 2826
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/013-actual-artifact-compatibility.log": {
      "sha256": "4408680e0e76cb8da8b90a0bcee0475173d9e0efe2fe12467a4878b550832775",
      "bytes": 15268
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/014-recovery-manifest.txt": {
      "sha256": "1705561519a2396fa86e2777d165449a9b0e9931713af0086a0795bbc2678a4b",
      "bytes": 824
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/015-test_reconstructed_finalizer.py": {
      "sha256": "ac3a696b72edac33cce40bab4798a0423500d642a47bcddcc21fa898956579ae",
      "bytes": 3089
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/016-P032-process-closed-before-command-schema-fix.py.placeholder": {
      "sha256": "7c4b4573f33189ed8a845a5f9f09f4f58e9ae6bdd4a1c86644d13e6c77e53d3d",
      "bytes": 24
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/017-P032-process-closed-review.md.placeholder": {
      "sha256": "7c4b4573f33189ed8a845a5f9f09f4f58e9ae6bdd4a1c86644d13e6c77e53d3d",
      "bytes": 24
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/018-P032-process-closed.py.placeholder": {
      "sha256": "7c4b4573f33189ed8a845a5f9f09f4f58e9ae6bdd4a1c86644d13e6c77e53d3d",
      "bytes": 24
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/019-test_P032_process_closed.py.placeholder": {
      "sha256": "7c4b4573f33189ed8a845a5f9f09f4f58e9ae6bdd4a1c86644d13e6c77e53d3d",
      "bytes": 24
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/020-failed-synthetic-main.log": {
      "sha256": "e87da0866ffa1544f09df112f44c77cdb0c135366882841a1aa675f7a05e653e",
      "bytes": 729
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/021-history_inventory.json": {
      "sha256": "4f77f6510c9b870ca79a5d7bbb5d8c1e9fca83e3fb6f7454c98c22320804b0b7",
      "bytes": 4198
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/022-history_inventory.md": {
      "sha256": "de01ecfd2cc0fa66d484a7b47f5cd2a4cb7e4597daac77530adcc7295f079f42",
      "bytes": 1108
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/023-P032-restore-pre-parent-shape.placeholder.py": {
      "sha256": "7c4b4573f33189ed8a845a5f9f09f4f58e9ae6bdd4a1c86644d13e6c77e53d3d",
      "bytes": 24
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/024-P032-restore-recovery-review.md": {
      "sha256": "6be2d9e2bf1326070ac899513e868e906c9f80d7d587f4820a2efb11cd2ccff5",
      "bytes": 2397
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/025-P032-restore.placeholder.py": {
      "sha256": "7c4b4573f33189ed8a845a5f9f09f4f58e9ae6bdd4a1c86644d13e6c77e53d3d",
      "bytes": 24
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/026-test_P032_restore_safety.py": {
      "sha256": "d7677e730bce90806322f4bf8a64c31fe1dfe37457f1178c6e67dc96b5cf74ae",
      "bytes": 13785
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/027-P032-allocation-semantics.md.overwritten": {
      "sha256": "7c4b4573f33189ed8a845a5f9f09f4f58e9ae6bdd4a1c86644d13e6c77e53d3d",
      "bytes": 24
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/028-P032-before.py.overwritten": {
      "sha256": "7c4b4573f33189ed8a845a5f9f09f4f58e9ae6bdd4a1c86644d13e6c77e53d3d",
      "bytes": 24
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/029-P032-candidate-checks.py.overwritten": {
      "sha256": "7c4b4573f33189ed8a845a5f9f09f4f58e9ae6bdd4a1c86644d13e6c77e53d3d",
      "bytes": 24
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/030-P032-failed-guard-reconstructed.py.overwritten": {
      "sha256": "7c4b4573f33189ed8a845a5f9f09f4f58e9ae6bdd4a1c86644d13e6c77e53d3d",
      "bytes": 24
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/031-P032-guard-recovery-review.md.overwritten": {
      "sha256": "7c4b4573f33189ed8a845a5f9f09f4f58e9ae6bdd4a1c86644d13e6c77e53d3d",
      "bytes": 24
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/032-P032-install-candidate.py.overwritten": {
      "sha256": "7c4b4573f33189ed8a845a5f9f09f4f58e9ae6bdd4a1c86644d13e6c77e53d3d",
      "bytes": 24
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/033-P032-preflight.py.overwritten": {
      "sha256": "7c4b4573f33189ed8a845a5f9f09f4f58e9ae6bdd4a1c86644d13e6c77e53d3d",
      "bytes": 24
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/034-P032-record-build-decision.py.overwritten": {
      "sha256": "7c4b4573f33189ed8a845a5f9f09f4f58e9ae6bdd4a1c86644d13e6c77e53d3d",
      "bytes": 24
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/035-P032-retain-candidate.py.overwritten": {
      "sha256": "7c4b4573f33189ed8a845a5f9f09f4f58e9ae6bdd4a1c86644d13e6c77e53d3d",
      "bytes": 24
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/036-P032-before.cpython-312.pyc": {
      "sha256": "f592e5a5f512b184062c345ed81b8c6bdd816acb1d230d7283794dae89f61bd7",
      "bytes": 21166
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/037-P032-candidate-checks.cpython-312.pyc": {
      "sha256": "a3c07d25e685a770f1c937a79bf2f7657ba96e1b6001b255c430c957a2ff5f79",
      "bytes": 26333
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/038-P032-complete.cpython-312.pyc": {
      "sha256": "11adcbb361856c737796d50f95694b650ae436df7c72bfbbaf33fe7894a1e6fd",
      "bytes": 78825
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/039-P032-finalize-command.cpython-312.pyc": {
      "sha256": "0a52edade7985c35e593e37539427a5dfd6123a7bd295cc6820a52040702815e",
      "bytes": 13863
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/040-P032-install-candidate.cpython-312.pyc": {
      "sha256": "315f1e45004298905b6c7c619fff5c52b77b0791a2f417f2d975bbca95fda6d1",
      "bytes": 6733
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/041-P032-preflight.cpython-312.pyc": {
      "sha256": "74e9ab4576a9e393926e9a51d5db9b1df44bce3cfbbc4d50ebc29f0d8a361cd3",
      "bytes": 22765
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/042-P032-process-closed.cpython-312.pyc": {
      "sha256": "c4df06194999cd4676e1818d53e62dbf156c0e427aadf138f247cbd60fb7942d",
      "bytes": 15849
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/043-P032-record-build-decision.cpython-312.pyc": {
      "sha256": "bd575199eed8ebed8cd768d1e9e82e30e54d4d4dbcd2e5fc85e349f41851078e",
      "bytes": 13501
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/044-P032-restore.cpython-312.pyc": {
      "sha256": "b2c683507b7f94466064b6c5e4cfa44ea13c44095d23fa293fe74f49c6846a9d",
      "bytes": 41592
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/045-P032-retain-candidate.cpython-312.pyc": {
      "sha256": "a0699203801886cc340d432f056f2315d11ad6652756ef3b2bfaa8a95c8a6b36",
      "bytes": 4496
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/046-P032-retain-history.cpython-312.pyc": {
      "sha256": "074b190d9db2d809d755cfe9f965dbdef754ac6968a270dde5777f67ac1e1b6c",
      "bytes": 29904
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/047-P032-task-update.cpython-312.pyc": {
      "sha256": "f78a6a44ca2fe5d5e202609b69d028f77b2097f10934b98ccaa26bc73c6e4fcb",
      "bytes": 10861
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/048-extract_p032.cpython-312.pyc": {
      "sha256": "2f8d322c9fd040e816f966ec2b6f6fede8fe376beb60076f8a41fa5207d828bd",
      "bytes": 59399
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/049-test_extract_p032.cpython-312.pyc": {
      "sha256": "e84efac10c0c4cc99a95aee073ade078e1dab68324aaada4d0cec27e6cbf125d",
      "bytes": 17992
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/050-test_p032_before_synthetic.cpython-312.pyc": {
      "sha256": "da2f8f5cd2c93dc4f25a56f0256c3e4bbbe70dbc3ea21eedd411a50da9ff6bd4",
      "bytes": 7484
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/051-test_p032_candidate_synthetic.cpython-312.pyc": {
      "sha256": "eef26ad45a8a9c95048098de3169238a1e4c7b1b92ee1575d5e6e248004d35fd",
      "bytes": 10234
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/052-recovered-executed-copies.json": {
      "sha256": "166add5ce033bd974809c8102022507b68e30d155626544c90a6bf666537b5af",
      "bytes": 7159
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/053-P032-process-closed.py": {
      "sha256": "c9344dec719d054512947a242d0b543ec1aba0ef3e29744fad252e3b991fe893",
      "bytes": 11759
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/054-P032-record-history-root.py": {
      "sha256": "e3f67bcac3b7f479cd8e212dcdb0fc1891be3a44819cadb3a1e8b9f27a2f153a",
      "bytes": 6421
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/055-P032-restore.py": {
      "sha256": "fb13014edbcfdd46f8eb0a03e92edff25ad451a20d369463b29124e7caa098ee",
      "bytes": 37238
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/056-extract_p032.py": {
      "sha256": "bf5cfe29e187e15611515ee41b61f32ccce7db6601def5b94ac1815956b4c898",
      "bytes": 47418
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/057-extract_executed.py": {
      "sha256": "bf5cfe29e187e15611515ee41b61f32ccce7db6601def5b94ac1815956b4c898",
      "bytes": 47418
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/058-finalizer-wrapper-command.json": {
      "sha256": "5539fd0d09734784bc953f72c8a68be6b65cbcd7d27dc17f6210a8f87562af83",
      "bytes": 6672
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/059-finalizer-wrapper.log": {
      "sha256": "f5c8d4dc050d5445bbd3bc3ddfec8d9e33a351673a7ff8e2651cf47bb85b57c6",
      "bytes": 177
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/060-finalizer_executed.py": {
      "sha256": "b2376e82232db6739c57b87a715f6e6ea0b633fdbe8d435196a3cc616d350733",
      "bytes": 58850
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/061-process_executed.py": {
      "sha256": "c9344dec719d054512947a242d0b543ec1aba0ef3e29744fad252e3b991fe893",
      "bytes": 11759
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/062-restore_executed.py": {
      "sha256": "fb13014edbcfdd46f8eb0a03e92edff25ad451a20d369463b29124e7caa098ee",
      "bytes": 37238
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/063-wrapper_executed.py": {
      "sha256": "91c12873f9d05807ddc35970cfad84e602c5e409e9998ddaa71ee25da3363c3e",
      "bytes": 10122
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/064-P032-before-command-record.json": {
      "sha256": "1f9b4e1bd49a8859ffbca50b4e7e0941d71c46af6591ad41260448492c4235a0",
      "bytes": 46129
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/065-P032-before-execute-run.log": {
      "sha256": "9440cbafe71316b501b94ec5d3f194d63d9ecbf3a33ca2cc8d1ebbdf722ae932",
      "bytes": 130
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/066-P032-before-run.log": {
      "sha256": "bcdc674e8580f6b924d8aa97f15b61798fabcc640a3f09b1230903144bec0604",
      "bytes": 104
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/067-P032-build-decision-run.log": {
      "sha256": "f540cfa80a8e7f1ba1edc75a4ede49066f24f0afa757ea8736f3fed19252a88f",
      "bytes": 1417
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/068-P032-candidate-bench-build.log": {
      "sha256": "491f4a2ed0d4e0082271ce2bf1ac7e5e61f4aa719898121690e1e865127c4f67",
      "bytes": 206
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/069-allocation_semantics_review.md": {
      "sha256": "7e835cde375abc2443d0e2b9359a56422cb7442b4bc6ba106d2adec96e73bd76",
      "bytes": 1346
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/070-before_executed.py": {
      "sha256": "d5994d7c537b70d463f380616048eba3bf057b93469ee7fe76b4edada6611aef",
      "bytes": 15720
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/071-build_decision_executed.py": {
      "sha256": "5b8221fcb6cc54aab934a746f704323eafbb245ccf7dec706f1855e22af314d0",
      "bytes": 8047
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/072-checks_executed.py": {
      "sha256": "3684057cac98079abded91aa4053759d9e2944527039a3e43bed014319824e25",
      "bytes": 20143
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/073-commands.json": {
      "sha256": "b145f0f936d70220d8ae330fc8aac65cb84d9eeecc5c8eb0ad54695ee38e5009",
      "bytes": 116570
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/074-failed_preedit_guard_executed.py": {
      "sha256": "223c028991c82ce18c76bc2b4a417c411703a970c230385474930829d4e6ac81",
      "bytes": 17067
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/075-guard-correction.json": {
      "sha256": "0274cce31d302c405a097b113ee8a89048db351f1bcf9d30c17ba74b6d2c667c",
      "bytes": 1195
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/076-guard_recovery_review.md": {
      "sha256": "c7d22551f1e13d3949931116d5be01eecfcf19913e919f0c451d2a2ac6c465d2",
      "bytes": 1590
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/077-retention_executed.py": {
      "sha256": "22a46b08ba64ee934ff680ad1beecc5b74617be680fd213c20844e17762b8687",
      "bytes": 2826
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/078-P032-candidate-checks-retry-run.log": {
      "sha256": "61d6b1b82d7b7fdc1e873b42c39511258e04396d8096cbdde1871de91cb9981e",
      "bytes": 18983
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/079-P032-candidate-checks-run.log": {
      "sha256": "9b0631c2bbde470cca0b75c39795f8500822d23142120b458a6eaa267c150f9c",
      "bytes": 104
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/080-install_executed.py": {
      "sha256": "ffd3c1741c252e8ec48af6be20ab08ce6ebaf0e566394f70059d2926abb9d10f",
      "bytes": 3411
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/081-private_tests_executed.rs": {
      "sha256": "9ff90fd720b5319c607ca19f4d72996fa6a4b7fedaaf5ffabec4358590fbd7fa",
      "bytes": 6824
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/082-stage_executed.py": {
      "sha256": "60cf0e52a18a5e02b7f97830dc633c8951ceba098df59b58495d86151267f4a1",
      "bytes": 5332
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/083-P032-candidate-retain-run.log": {
      "sha256": "8dbb314cb6df2f674b2e0c8befe50b2e8cd21c9f7db1ebcf02ca6851e754f951",
      "bytes": 76
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/084-retention.json": {
      "sha256": "40b32ca75dad75895869820dcdbfde3cb836f32ed3152f764896435f95b09c79",
      "bytes": 540
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/085-P032-install-run.log": {
      "sha256": "876641430b7411bc8f831b1a6fabaf70b884cfda654f43102e0857afe4404c56",
      "bytes": 108
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/086-preflight_executed.py": {
      "sha256": "ca61acb32f03fe1c42bbbda875b26f681af7a9dd63ccb3efb0ff237c35377be3",
      "bytes": 17370
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/087-report.json": {
      "sha256": "54a0a1bb04d5a232899186e7b4066cd641fd02cea4b47bd0549bdffed3b8545d",
      "bytes": 12714
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/088-task_update_executed.py": {
      "sha256": "8686c1c7335e87b4bf2439350b487d4f73c31cd2f6fbf821ac86061e2565e404",
      "bytes": 8204
    },
    "/workspace/wtime/target/performance/P032/tooling-provenance/089-P032-preflight-run.log": {
      "sha256": "a6e61bbf6ebe97affd69a5f23957b01d6b5cee0e2cefeec62397d9b02faccdae",
      "bytes": 138
    }
  },
  "artifacts": [
    "/workspace/wtime/docs/benchmarks/performance/P031-evidence.json",
    "/workspace/wtime/docs/benchmarks/performance/P032-build-decision.md",
    "/workspace/wtime/docs/benchmarks/performance/P032-candidate.patch",
    "/workspace/wtime/docs/benchmarks/performance/P032-contract.json",
    "/workspace/wtime/docs/benchmarks/performance/P032-evidence.json",
    "/workspace/wtime/target/performance/P031/restored-default-tests.log",
    "/workspace/wtime/target/performance/P031/restored-diagnostic-tests.log",
    "/workspace/wtime/target/performance/P032-before-build.json",
    "/workspace/wtime/target/performance/P032-before/manifest.json",
    "/workspace/wtime/target/performance/P032-build.json",
    "/workspace/wtime/target/performance/P032-candidate-checks/diagnostic_performance.log",
    "/workspace/wtime/target/performance/P032-candidate-checks/focused_default.log",
    "/workspace/wtime/target/performance/P032-candidate-checks/focused_diagnostic.log",
    "/workspace/wtime/target/performance/P032-candidate-checks/full_default.log",
    "/workspace/wtime/target/performance/P032-candidate-retained/source-manifest.json",
    "/workspace/wtime/target/performance/P032-incremental-build-decision.json",
    "/workspace/wtime/target/performance/P032-preflight/default.log",
    "/workspace/wtime/target/performance/P032-preflight/diagnostic.log",
    "/workspace/wtime/target/performance/P032/analysis.json",
    "/workspace/wtime/target/performance/P032/analysis.log",
    "/workspace/wtime/target/performance/P032/checkpoint.jsonl",
    "/workspace/wtime/target/performance/P032/commands.json",
    "/workspace/wtime/target/performance/P032/decision.json",
    "/workspace/wtime/target/performance/P032/development-provenance.json",
    "/workspace/wtime/target/performance/P032/extraction.json",
    "/workspace/wtime/target/performance/P032/extraction.log",
    "/workspace/wtime/target/performance/P032/final-decision.json",
    "/workspace/wtime/target/performance/P032/raw-exits.json",
    "/workspace/wtime/target/performance/P032/raw-gate.log",
    "/workspace/wtime/target/performance/P032/report.json",
    "/workspace/wtime/target/performance/P032/restored-build.json",
    "/workspace/wtime/target/performance/P032/restored-default-tests.log",
    "/workspace/wtime/target/performance/P032/restored-diagnostic-tests.log",
    "/workspace/wtime/target/performance/P032/restored-focused-default-tests.log",
    "/workspace/wtime/target/performance/P032/restored-focused-diagnostic-tests.log",
    "/workspace/wtime/target/performance/P032/rollback-identities.json",
    "/workspace/wtime/target/performance/P032/task-precompletion/P032-call-argument-ranges.md",
    "/workspace/wtime/target/performance/P032/task-precompletion/TASKS.json",
    "/workspace/wtime/target/performance/P032/tooling-provenance/000-P032-complete.py",
    "/workspace/wtime/target/performance/P032/tooling-provenance/001-P032-finalize-command.py",
    "/workspace/wtime/target/performance/P032/tooling-provenance/002-P032-complete-help.txt",
    "/workspace/wtime/target/performance/P032/tooling-provenance/003-P032-complete-pre-parent.placeholder.py",
    "/workspace/wtime/target/performance/P032/tooling-provenance/004-P032-complete.placeholder.py",
    "/workspace/wtime/target/performance/P032/tooling-provenance/005-P032-finalize-command-help.txt",
    "/workspace/wtime/target/performance/P032/tooling-provenance/006-P032-finalize-command-review.placeholder.md",
    "/workspace/wtime/target/performance/P032/tooling-provenance/007-P032-finalize-command.byte-exact.py",
    "/workspace/wtime/target/performance/P032/tooling-provenance/008-P032-finalize-command.placeholder.py",
    "/workspace/wtime/target/performance/P032/tooling-provenance/009-P032-parent-counts-review.placeholder.md",
    "/workspace/wtime/target/performance/P032/tooling-provenance/010-P032-restore-pre-parent-shape.placeholder.py",
    "/workspace/wtime/target/performance/P032/tooling-provenance/011-P032-restore.placeholder.py",
    "/workspace/wtime/target/performance/P032/tooling-provenance/012-actual-artifact-compatibility-review.md",
    "/workspace/wtime/target/performance/P032/tooling-provenance/013-actual-artifact-compatibility.log",
    "/workspace/wtime/target/performance/P032/tooling-provenance/014-recovery-manifest.txt",
    "/workspace/wtime/target/performance/P032/tooling-provenance/015-test_reconstructed_finalizer.py",
    "/workspace/wtime/target/performance/P032/tooling-provenance/016-P032-process-closed-before-command-schema-fix.py.placeholder",
    "/workspace/wtime/target/performance/P032/tooling-provenance/017-P032-process-closed-review.md.placeholder",
    "/workspace/wtime/target/performance/P032/tooling-provenance/018-P032-process-closed.py.placeholder",
    "/workspace/wtime/target/performance/P032/tooling-provenance/019-test_P032_process_closed.py.placeholder",
    "/workspace/wtime/target/performance/P032/tooling-provenance/020-failed-synthetic-main.log",
    "/workspace/wtime/target/performance/P032/tooling-provenance/021-history_inventory.json",
    "/workspace/wtime/target/performance/P032/tooling-provenance/022-history_inventory.md",
    "/workspace/wtime/target/performance/P032/tooling-provenance/023-P032-restore-pre-parent-shape.placeholder.py",
    "/workspace/wtime/target/performance/P032/tooling-provenance/024-P032-restore-recovery-review.md",
    "/workspace/wtime/target/performance/P032/tooling-provenance/025-P032-restore.placeholder.py",
    "/workspace/wtime/target/performance/P032/tooling-provenance/026-test_P032_restore_safety.py",
    "/workspace/wtime/target/performance/P032/tooling-provenance/027-P032-allocation-semantics.md.overwritten",
    "/workspace/wtime/target/performance/P032/tooling-provenance/028-P032-before.py.overwritten",
    "/workspace/wtime/target/performance/P032/tooling-provenance/029-P032-candidate-checks.py.overwritten",
    "/workspace/wtime/target/performance/P032/tooling-provenance/030-P032-failed-guard-reconstructed.py.overwritten",
    "/workspace/wtime/target/performance/P032/tooling-provenance/031-P032-guard-recovery-review.md.overwritten",
    "/workspace/wtime/target/performance/P032/tooling-provenance/032-P032-install-candidate.py.overwritten",
    "/workspace/wtime/target/performance/P032/tooling-provenance/033-P032-preflight.py.overwritten",
    "/workspace/wtime/target/performance/P032/tooling-provenance/034-P032-record-build-decision.py.overwritten",
    "/workspace/wtime/target/performance/P032/tooling-provenance/035-P032-retain-candidate.py.overwritten",
    "/workspace/wtime/target/performance/P032/tooling-provenance/036-P032-before.cpython-312.pyc",
    "/workspace/wtime/target/performance/P032/tooling-provenance/037-P032-candidate-checks.cpython-312.pyc",
    "/workspace/wtime/target/performance/P032/tooling-provenance/038-P032-complete.cpython-312.pyc",
    "/workspace/wtime/target/performance/P032/tooling-provenance/039-P032-finalize-command.cpython-312.pyc",
    "/workspace/wtime/target/performance/P032/tooling-provenance/040-P032-install-candidate.cpython-312.pyc",
    "/workspace/wtime/target/performance/P032/tooling-provenance/041-P032-preflight.cpython-312.pyc",
    "/workspace/wtime/target/performance/P032/tooling-provenance/042-P032-process-closed.cpython-312.pyc",
    "/workspace/wtime/target/performance/P032/tooling-provenance/043-P032-record-build-decision.cpython-312.pyc",
    "/workspace/wtime/target/performance/P032/tooling-provenance/044-P032-restore.cpython-312.pyc",
    "/workspace/wtime/target/performance/P032/tooling-provenance/045-P032-retain-candidate.cpython-312.pyc",
    "/workspace/wtime/target/performance/P032/tooling-provenance/046-P032-retain-history.cpython-312.pyc",
    "/workspace/wtime/target/performance/P032/tooling-provenance/047-P032-task-update.cpython-312.pyc",
    "/workspace/wtime/target/performance/P032/tooling-provenance/048-extract_p032.cpython-312.pyc",
    "/workspace/wtime/target/performance/P032/tooling-provenance/049-test_extract_p032.cpython-312.pyc",
    "/workspace/wtime/target/performance/P032/tooling-provenance/050-test_p032_before_synthetic.cpython-312.pyc",
    "/workspace/wtime/target/performance/P032/tooling-provenance/051-test_p032_candidate_synthetic.cpython-312.pyc",
    "/workspace/wtime/target/performance/P032/tooling-provenance/052-recovered-executed-copies.json",
    "/workspace/wtime/target/performance/P032/tooling-provenance/053-P032-process-closed.py",
    "/workspace/wtime/target/performance/P032/tooling-provenance/054-P032-record-history-root.py",
    "/workspace/wtime/target/performance/P032/tooling-provenance/055-P032-restore.py",
    "/workspace/wtime/target/performance/P032/tooling-provenance/056-extract_p032.py",
    "/workspace/wtime/target/performance/P032/tooling-provenance/057-extract_executed.py",
    "/workspace/wtime/target/performance/P032/tooling-provenance/058-finalizer-wrapper-command.json",
    "/workspace/wtime/target/performance/P032/tooling-provenance/059-finalizer-wrapper.log",
    "/workspace/wtime/target/performance/P032/tooling-provenance/060-finalizer_executed.py",
    "/workspace/wtime/target/performance/P032/tooling-provenance/061-process_executed.py",
    "/workspace/wtime/target/performance/P032/tooling-provenance/062-restore_executed.py",
    "/workspace/wtime/target/performance/P032/tooling-provenance/063-wrapper_executed.py",
    "/workspace/wtime/target/performance/P032/tooling-provenance/064-P032-before-command-record.json",
    "/workspace/wtime/target/performance/P032/tooling-provenance/065-P032-before-execute-run.log",
    "/workspace/wtime/target/performance/P032/tooling-provenance/066-P032-before-run.log",
    "/workspace/wtime/target/performance/P032/tooling-provenance/067-P032-build-decision-run.log",
    "/workspace/wtime/target/performance/P032/tooling-provenance/068-P032-candidate-bench-build.log",
    "/workspace/wtime/target/performance/P032/tooling-provenance/069-allocation_semantics_review.md",
    "/workspace/wtime/target/performance/P032/tooling-provenance/070-before_executed.py",
    "/workspace/wtime/target/performance/P032/tooling-provenance/071-build_decision_executed.py",
    "/workspace/wtime/target/performance/P032/tooling-provenance/072-checks_executed.py",
    "/workspace/wtime/target/performance/P032/tooling-provenance/073-commands.json",
    "/workspace/wtime/target/performance/P032/tooling-provenance/074-failed_preedit_guard_executed.py",
    "/workspace/wtime/target/performance/P032/tooling-provenance/075-guard-correction.json",
    "/workspace/wtime/target/performance/P032/tooling-provenance/076-guard_recovery_review.md",
    "/workspace/wtime/target/performance/P032/tooling-provenance/077-retention_executed.py",
    "/workspace/wtime/target/performance/P032/tooling-provenance/078-P032-candidate-checks-retry-run.log",
    "/workspace/wtime/target/performance/P032/tooling-provenance/079-P032-candidate-checks-run.log",
    "/workspace/wtime/target/performance/P032/tooling-provenance/080-install_executed.py",
    "/workspace/wtime/target/performance/P032/tooling-provenance/081-private_tests_executed.rs",
    "/workspace/wtime/target/performance/P032/tooling-provenance/082-stage_executed.py",
    "/workspace/wtime/target/performance/P032/tooling-provenance/083-P032-candidate-retain-run.log",
    "/workspace/wtime/target/performance/P032/tooling-provenance/084-retention.json",
    "/workspace/wtime/target/performance/P032/tooling-provenance/085-P032-install-run.log",
    "/workspace/wtime/target/performance/P032/tooling-provenance/086-preflight_executed.py",
    "/workspace/wtime/target/performance/P032/tooling-provenance/087-report.json",
    "/workspace/wtime/target/performance/P032/tooling-provenance/088-task_update_executed.py",
    "/workspace/wtime/target/performance/P032/tooling-provenance/089-P032-preflight-run.log"
  ],
  "candidate_promoted": false,
  "commands": [
    {
      "checkpoint_path": "/workspace/wtime/target/performance/P032/checkpoint.jsonl",
      "checkpoint_sha256": "b835e5991537be83e8309ba4b28d31b3c0145650f90dfc1ecbb73a676f4f7091",
      "command": ". /workspace/.setup/rust-env.sh\npython3 tools/wtime-bench/compare.py --baseline target/performance/P032-before --candidate tools/wtime-bench/target/release/wtime-bench --candidate-mode decoded --selection scalar --pairs 30 --build-growth-explanation-file docs/benchmarks/performance/P032-build-decision.md --output target/performance/P032 > target/performance/P032-run.log 2>&1",
      "exit_attested_by": "comparison-exit-0",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P032-run.log",
      "log_sha256": "a6651069c51ef66aeac29ba445569051f42bc608a7da852b9c2a5b9abea3b7c5",
      "name": "comparison",
      "report_path": "/workspace/wtime/target/performance/P032/report.json",
      "report_sha256": "a70f150636b6f6a60db5ae3592454cb68b8469f724a727e1f61e2a73e5785f2e"
    },
    {
      "argv": [
        "python3",
        "/tmp/extract_p032.py",
        "--report",
        "/workspace/wtime/target/performance/P032/report.json",
        "--checkpoint",
        "/workspace/wtime/target/performance/P032/checkpoint.jsonl",
        "--archive",
        "/workspace/wtime/target/performance/P032-before",
        "--candidate-dir",
        "/workspace/wtime/target/performance/P032-candidate-retained",
        "--candidate-binary",
        "/workspace/wtime/target/performance/P032-candidate-retained/tools/wtime-bench/target/release/wtime-bench",
        "--output",
        "/workspace/wtime/target/performance/P032/extraction.json",
        "--contract",
        "/workspace/wtime/docs/benchmarks/performance/P032-contract.json"
      ],
      "command": ". /workspace/.setup/rust-env.sh\npython3 /tmp/extract_p032.py --report /workspace/wtime/target/performance/P032/report.json --checkpoint /workspace/wtime/target/performance/P032/checkpoint.jsonl --archive /workspace/wtime/target/performance/P032-before --candidate-dir /workspace/wtime/target/performance/P032-candidate-retained --candidate-binary /workspace/wtime/target/performance/P032-candidate-retained/tools/wtime-bench/target/release/wtime-bench --output /workspace/wtime/target/performance/P032/extraction.json --contract /workspace/wtime/docs/benchmarks/performance/P032-contract.json",
      "exit_code": 0,
      "log_bytes": 314737,
      "log_path": "/workspace/wtime/target/performance/P032/extraction.log",
      "log_sha256": "8c36cdfbaa3dd44dc027a5d850c9f727f8cc1eeb00cedd27aa7d2f3c826aaa87",
      "name": "extraction"
    },
    {
      "argv": [
        "python3",
        "tools/wtime-bench/analyze.py",
        "--input",
        "/workspace/wtime/target/performance/P032",
        "--output",
        "/workspace/wtime/target/performance/P032/analysis.json"
      ],
      "command": ". /workspace/.setup/rust-env.sh\npython3 tools/wtime-bench/analyze.py --input /workspace/wtime/target/performance/P032 --output /workspace/wtime/target/performance/P032/analysis.json",
      "exit_code": 0,
      "log_bytes": 0,
      "log_path": "/workspace/wtime/target/performance/P032/analysis.log",
      "log_sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
      "name": "analysis"
    },
    {
      "argv": [
        "python3",
        "tools/wtime-bench/gates.py",
        "--task",
        "P032",
        "--comparison",
        "/workspace/wtime/target/performance/P032",
        "--build",
        "/workspace/wtime/target/performance/P032/comparison-build.json",
        "--contract",
        "/workspace/wtime/docs/benchmarks/performance/P032-contract.json",
        "--output",
        "/workspace/wtime/target/performance/P032/decision.json"
      ],
      "command": ". /workspace/.setup/rust-env.sh\npython3 tools/wtime-bench/gates.py --task P032 --comparison /workspace/wtime/target/performance/P032 --build /workspace/wtime/target/performance/P032/comparison-build.json --contract /workspace/wtime/docs/benchmarks/performance/P032-contract.json --output /workspace/wtime/target/performance/P032/decision.json",
      "exit_code": 1,
      "log_bytes": 1692112,
      "log_path": "/workspace/wtime/target/performance/P032/raw-gate.log",
      "log_sha256": "f6f186d070a156fab1cfba9066c15f5b380b1d13998e0d62b4704812ddc9e866",
      "name": "raw_gate"
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
      "log_path": "/workspace/wtime/target/performance/P032/restored-default-tests.log",
      "log_sha256": "2d7b81883f26f25b4a9f1eb964e6d59eeb844564391d2f703143d70d36ad8c62",
      "name": "restored_default_tests",
      "observed_tests": {
        "failed": 0,
        "ignored": 0,
        "passed": 215,
        "path": "/workspace/wtime/target/performance/P032/restored-default-tests.log",
        "sha256": "2d7b81883f26f25b4a9f1eb964e6d59eeb844564391d2f703143d70d36ad8c62",
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
      "log_path": "/workspace/wtime/target/performance/P032/restored-diagnostic-tests.log",
      "log_sha256": "f144e3692cf3ff0f3494c6c4dbcd43e3b5c631d056340add54a1b466c3f8dfaf",
      "name": "restored_diagnostic_tests",
      "observed_tests": {
        "failed": 0,
        "ignored": 0,
        "passed": 85,
        "path": "/workspace/wtime/target/performance/P032/restored-diagnostic-tests.log",
        "sha256": "f144e3692cf3ff0f3494c6c4dbcd43e3b5c631d056340add54a1b466c3f8dfaf",
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
        "p032_",
        "--",
        "--nocapture"
      ],
      "command": ". /workspace/.setup/rust-env.sh\ncargo test --locked -p wtime-runtime p032_ -- --nocapture",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P032/restored-focused-default-tests.log",
      "log_sha256": "c6eb9c579811ec1cefeb2a490a66d1d6ba045823c79efc17c572b32a677e9af1",
      "name": "restored_focused_default_tests",
      "observed_tests": {
        "failed": 0,
        "ignored": 0,
        "passed": 4,
        "path": "/workspace/wtime/target/performance/P032/restored-focused-default-tests.log",
        "sha256": "c6eb9c579811ec1cefeb2a490a66d1d6ba045823c79efc17c572b32a677e9af1",
        "suite_summaries": 7,
        "test_names": [
          "p032::p032_imported_host_owns_arguments_during_nested_guest_reentry",
          "p032::p032_public_reference_arguments_keep_nominal_and_store_checks",
          "p032::p032_guest_frame_and_recursive_multivalue_preserve_exact_arguments",
          "p032::p032_tail_recursive_arguments_overlap_safely_at_depth_two"
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
        "p032_",
        "--",
        "--nocapture"
      ],
      "command": ". /workspace/.setup/rust-env.sh\ncargo test --locked -p wtime-runtime --features execution-reference,performance-counters p032_ -- --nocapture",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P032/restored-focused-diagnostic-tests.log",
      "log_sha256": "e3448fadc22c20b500aab4c0ff3be003173b14b38eea7111db6e95bbddca500d",
      "name": "restored_focused_diagnostic_tests",
      "observed_tests": {
        "failed": 0,
        "ignored": 0,
        "passed": 5,
        "path": "/workspace/wtime/target/performance/P032/restored-focused-diagnostic-tests.log",
        "sha256": "e3448fadc22c20b500aab4c0ff3be003173b14b38eea7111db6e95bbddca500d",
        "suite_summaries": 7,
        "test_names": [
          "p032::p032_imported_host_owns_arguments_during_nested_guest_reentry",
          "p032::p032_reference_policy_repeats_public_guest_argument_results",
          "p032::p032_tail_recursive_arguments_overlap_safely_at_depth_two",
          "p032::p032_public_reference_arguments_keep_nominal_and_store_checks",
          "p032::p032_guest_frame_and_recursive_multivalue_preserve_exact_arguments"
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
        "5"
      ],
      "command": ". /workspace/.setup/rust-env.sh\ncargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml -j 5",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P032/restored-benchmark-build.log",
      "log_sha256": "312d5758957e7e6cbd5c9d810af38f0d3c95ea98bfe18fdd29e2f304d7d67b42",
      "name": "rollback_benchmark_build"
    },
    {
      "argv": [
        "python3",
        "tools/checks/build_budget.py",
        "--output",
        "target/performance/P032/restored-build.json"
      ],
      "command": ". /workspace/.setup/rust-env.sh\npython3 tools/checks/build_budget.py --output target/performance/P032/restored-build.json",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P032/restored-build.log",
      "log_sha256": "a249fd77b5584f51bfaedbb1f18a8c6c430b173bf0b80626bf977e4c5286d368",
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
    "primary_pair_event_count": 4800,
    "primary_workload_count": 16,
    "primary_workload_ids": [
      "scalar_call_ref_n1",
      "scalar_call_ref_n10000",
      "scalar_call_ref_n1024",
      "scalar_call_ref_n32",
      "scalar_direct_calls_n1",
      "scalar_direct_calls_n10000",
      "scalar_direct_calls_n1024",
      "scalar_direct_calls_n32",
      "scalar_indirect_calls_n1",
      "scalar_indirect_calls_n10000",
      "scalar_indirect_calls_n1024",
      "scalar_indirect_calls_n32",
      "scalar_tail_calls_n1",
      "scalar_tail_calls_n10000",
      "scalar_tail_calls_n1024",
      "scalar_tail_calls_n32"
    ],
    "protected_pair_event_count": 20700,
    "protected_workload_count": 69,
    "protected_workload_ids": [
      "branch_calls",
      "call_overhead",
      "integer_loop",
      "scalar_accumulator_n0",
      "scalar_accumulator_n1",
      "scalar_accumulator_n10000",
      "scalar_accumulator_n1024",
      "scalar_accumulator_n17",
      "scalar_accumulator_n32",
      "scalar_br_table_n0",
      "scalar_br_table_n1",
      "scalar_br_table_n10000",
      "scalar_br_table_n1024",
      "scalar_br_table_n17",
      "scalar_br_table_n32",
      "scalar_branch_entropy_n0",
      "scalar_branch_entropy_n1",
      "scalar_branch_entropy_n10000",
      "scalar_branch_entropy_n1024",
      "scalar_branch_entropy_n17",
      "scalar_branch_entropy_n32",
      "scalar_call_ref_n0",
      "scalar_call_ref_n17",
      "scalar_dependent_n0",
      "scalar_dependent_n1",
      "scalar_dependent_n10000",
      "scalar_dependent_n1024",
      "scalar_dependent_n17",
      "scalar_dependent_n32",
      "scalar_direct_calls_n0",
      "scalar_direct_calls_n17",
      "scalar_dynamic_api_n0",
      "scalar_dynamic_api_n1",
      "scalar_dynamic_api_n10000",
      "scalar_dynamic_api_n1024",
      "scalar_dynamic_api_n17",
      "scalar_dynamic_api_n32",
      "scalar_host_call_n0",
      "scalar_host_call_n1",
      "scalar_host_call_n10000",
      "scalar_host_call_n1024",
      "scalar_host_call_n17",
      "scalar_host_call_n32",
      "scalar_indirect_calls_n0",
      "scalar_indirect_calls_n17",
      "scalar_multivalue_branch_n0",
      "scalar_multivalue_branch_n1",
      "scalar_multivalue_branch_n10000",
      "scalar_multivalue_branch_n1024",
      "scalar_multivalue_branch_n17",
      "scalar_multivalue_branch_n32",
      "scalar_ordinary_recursion_n0",
      "scalar_ordinary_recursion_n1",
      "scalar_ordinary_recursion_n17",
      "scalar_ordinary_recursion_n32",
      "scalar_table_mutation_n0",
      "scalar_table_mutation_n1",
      "scalar_table_mutation_n10000",
      "scalar_table_mutation_n1024",
      "scalar_table_mutation_n17",
      "scalar_table_mutation_n32",
      "scalar_tail_calls_n0",
      "scalar_tail_calls_n17",
      "scalar_try_table_normal_n0",
      "scalar_try_table_normal_n1",
      "scalar_try_table_normal_n10000",
      "scalar_try_table_normal_n1024",
      "scalar_try_table_normal_n17",
      "scalar_try_table_normal_n32"
    ],
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
    "warm_calibration_attempt_checks": 220,
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
                "calibrated_total_ns": 24822335.0,
                "ns_per_operation": 6205583.75,
                "observed_batch": 4,
                "requested_batch": 4,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 33351774.0,
                "ns_per_operation": 8337943.5,
                "observed_batch": 4,
                "requested_batch": 4,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 24822335.0,
            "ns_per_operation": 6205583.75,
            "observed_batch": 4,
            "requested_batch": 4,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 33351774.0,
            "ns_per_operation": 8337943.5,
            "observed_batch": 4,
            "requested_batch": 4,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 17998272.998999998,
            "ns_per_operation": 5999424.333,
            "observed_batch": 3,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 28206817.0,
            "ns_per_operation": 7051704.25,
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
            "batch": 47962,
            "sides": [
              {
                "calibrated_total_ns": 21643044.348,
                "ns_per_operation": 451.254,
                "observed_batch": 47962,
                "requested_batch": 47962,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 15566354.872,
                "ns_per_operation": 324.556,
                "observed_batch": 47962,
                "requested_batch": 47962,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 67785,
            "sides": [
              {
                "calibrated_total_ns": 23133190.305000003,
                "ns_per_operation": 341.273,
                "observed_batch": 67785,
                "requested_batch": 67785,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 24592330.215,
                "ns_per_operation": 362.799,
                "observed_batch": 67785,
                "requested_batch": 67785,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 23133190.305000003,
            "ns_per_operation": 341.273,
            "observed_batch": 67785,
            "requested_batch": 67785,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 24592330.215,
            "ns_per_operation": 362.799,
            "observed_batch": 67785,
            "requested_batch": 67785,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 18676306.876000002,
            "ns_per_operation": 389.398,
            "observed_batch": 47962,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 17076966.912,
            "ns_per_operation": 383.373,
            "observed_batch": 44544,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 67785,
        "source_sha256": "9493b8f751690a2fd6e8b19dcee5e2c1d50b717253d292ce34d5692b306663a5",
        "wasm_sha256": "0815ca7d6026206d0773fe10c6b9643862bfffe8ba361c9789369ef9668ee746",
        "workload_kind": "legacy_single_case_control"
      },
      "0:integer_loop": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 4,
            "sides": [
              {
                "calibrated_total_ns": 32970446.0,
                "ns_per_operation": 8242611.5,
                "observed_batch": 4,
                "requested_batch": 4,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 30619113.0,
                "ns_per_operation": 7654778.25,
                "observed_batch": 4,
                "requested_batch": 4,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 32970446.0,
            "ns_per_operation": 8242611.5,
            "observed_batch": 4,
            "requested_batch": 4,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 30619113.0,
            "ns_per_operation": 7654778.25,
            "observed_batch": 4,
            "requested_batch": 4,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 27666485.0,
            "ns_per_operation": 6916621.25,
            "observed_batch": 4,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 19669098.0,
            "ns_per_operation": 6556366.0,
            "observed_batch": 3,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 4,
        "source_sha256": "791730112109f3d5751a7b3e2c1134e7e0e6f71a7f7ee01ea8481eb989ee3fa4",
        "wasm_sha256": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c",
        "workload_kind": "legacy_single_case_control"
      },
      "0:scalar_accumulator_n0": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 9976,
            "sides": [
              {
                "calibrated_total_ns": 10153722.440000001,
                "ns_per_operation": 1017.815,
                "observed_batch": 9976,
                "requested_batch": 9976,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 7160034.576,
                "ns_per_operation": 717.726,
                "observed_batch": 9976,
                "requested_batch": 9976,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 30653,
            "sides": [
              {
                "calibrated_total_ns": 24424034.523000002,
                "ns_per_operation": 796.791,
                "observed_batch": 30653,
                "requested_batch": 30653,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 21765530.486,
                "ns_per_operation": 710.062,
                "observed_batch": 30653,
                "requested_batch": 30653,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 24424034.523000002,
            "ns_per_operation": 796.791,
            "observed_batch": 30653,
            "requested_batch": 30653,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 21765530.486,
            "ns_per_operation": 710.062,
            "observed_batch": 30653,
            "requested_batch": 30653,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 7290982.828000001,
            "ns_per_operation": 780.118,
            "observed_batch": 9346,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 7934830.592,
            "ns_per_operation": 795.392,
            "observed_batch": 9976,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 30653,
        "source_sha256": "61aaec7a86d481378f31f3e27b0c1413684065841fa5909297181fcad9bef10d",
        "wasm_sha256": "ef61bf7afc44af7b71ef78eadcba48c0d85949d7d9096df8fbbbab53c05036b1",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_accumulator_n1": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 7746,
            "sides": [
              {
                "calibrated_total_ns": 9869062.41,
                "ns_per_operation": 1274.085,
                "observed_batch": 7746,
                "requested_batch": 7746,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 13720892.592,
                "ns_per_operation": 1771.352,
                "observed_batch": 7746,
                "requested_batch": 7746,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 17268,
            "sides": [
              {
                "calibrated_total_ns": 21152039.435999997,
                "ns_per_operation": 1224.927,
                "observed_batch": 17268,
                "requested_batch": 17268,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 20864112.803999998,
                "ns_per_operation": 1208.253,
                "observed_batch": 17268,
                "requested_batch": 17268,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 21152039.435999997,
            "ns_per_operation": 1224.927,
            "observed_batch": 17268,
            "requested_batch": 17268,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 20864112.803999998,
            "ns_per_operation": 1208.253,
            "observed_batch": 17268,
            "requested_batch": 17268,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 10181175.508,
            "ns_per_operation": 1334.711,
            "observed_batch": 7628,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 9703592.358,
            "ns_per_operation": 1252.723,
            "observed_batch": 7746,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 17268,
        "source_sha256": "61aaec7a86d481378f31f3e27b0c1413684065841fa5909297181fcad9bef10d",
        "wasm_sha256": "ef61bf7afc44af7b71ef78eadcba48c0d85949d7d9096df8fbbbab53c05036b1",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_accumulator_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 3,
            "sides": [
              {
                "calibrated_total_ns": 36308520.999,
                "ns_per_operation": 12102840.333,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 35947692.0,
                "ns_per_operation": 11982564.0,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 36308520.999,
            "ns_per_operation": 12102840.333,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 35947692.0,
            "ns_per_operation": 11982564.0,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 24724932.0,
            "ns_per_operation": 8241644.0,
            "observed_batch": 3,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 21881295.998999998,
            "ns_per_operation": 7293765.333,
            "observed_batch": 3,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 3,
        "source_sha256": "61aaec7a86d481378f31f3e27b0c1413684065841fa5909297181fcad9bef10d",
        "wasm_sha256": "ef61bf7afc44af7b71ef78eadcba48c0d85949d7d9096df8fbbbab53c05036b1",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_accumulator_n1024": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 37,
            "sides": [
              {
                "calibrated_total_ns": 23808216.988,
                "ns_per_operation": 643465.324,
                "observed_batch": 37,
                "requested_batch": 37,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 27249084.009999998,
                "ns_per_operation": 736461.73,
                "observed_batch": 37,
                "requested_batch": 37,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 23808216.988,
            "ns_per_operation": 643465.324,
            "observed_batch": 37,
            "requested_batch": 37,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27249084.009999998,
            "ns_per_operation": 736461.73,
            "observed_batch": 37,
            "requested_batch": 37,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 3360846.0,
            "ns_per_operation": 560141.0,
            "observed_batch": 6,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 29256343.001,
            "ns_per_operation": 790711.973,
            "observed_batch": 37,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 37,
        "source_sha256": "61aaec7a86d481378f31f3e27b0c1413684065841fa5909297181fcad9bef10d",
        "wasm_sha256": "ef61bf7afc44af7b71ef78eadcba48c0d85949d7d9096df8fbbbab53c05036b1",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_accumulator_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 2282,
            "sides": [
              {
                "calibrated_total_ns": 33815769.078,
                "ns_per_operation": 14818.479,
                "observed_batch": 2282,
                "requested_batch": 2282,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 30086196.07,
                "ns_per_operation": 13184.135,
                "observed_batch": 2282,
                "requested_batch": 2282,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 33815769.078,
            "ns_per_operation": 14818.479,
            "observed_batch": 2282,
            "requested_batch": 2282,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 30086196.07,
            "ns_per_operation": 13184.135,
            "observed_batch": 2282,
            "requested_batch": 2282,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 40643498.42,
            "ns_per_operation": 18424.07,
            "observed_batch": 2206,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 46557642.404,
            "ns_per_operation": 20402.122,
            "observed_batch": 2282,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 2282,
        "source_sha256": "61aaec7a86d481378f31f3e27b0c1413684065841fa5909297181fcad9bef10d",
        "wasm_sha256": "ef61bf7afc44af7b71ef78eadcba48c0d85949d7d9096df8fbbbab53c05036b1",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_accumulator_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1306,
            "sides": [
              {
                "calibrated_total_ns": 32550221.599999998,
                "ns_per_operation": 24923.6,
                "observed_batch": 1306,
                "requested_batch": 1306,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 27653904.836000003,
                "ns_per_operation": 21174.506,
                "observed_batch": 1306,
                "requested_batch": 1306,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 32550221.599999998,
            "ns_per_operation": 24923.6,
            "observed_batch": 1306,
            "requested_batch": 1306,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27653904.836000003,
            "ns_per_operation": 21174.506,
            "observed_batch": 1306,
            "requested_batch": 1306,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 32426643.57,
            "ns_per_operation": 25136.933,
            "observed_batch": 1290,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 28598279.966000002,
            "ns_per_operation": 21897.611,
            "observed_batch": 1306,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1306,
        "source_sha256": "61aaec7a86d481378f31f3e27b0c1413684065841fa5909297181fcad9bef10d",
        "wasm_sha256": "ef61bf7afc44af7b71ef78eadcba48c0d85949d7d9096df8fbbbab53c05036b1",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_br_table_n0": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 29941,
            "sides": [
              {
                "calibrated_total_ns": 23989298.079,
                "ns_per_operation": 801.219,
                "observed_batch": 29941,
                "requested_batch": 29941,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 21836031.182,
                "ns_per_operation": 729.302,
                "observed_batch": 29941,
                "requested_batch": 29941,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 23989298.079,
            "ns_per_operation": 801.219,
            "observed_batch": 29941,
            "requested_batch": 29941,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 21836031.182,
            "ns_per_operation": 729.302,
            "observed_batch": 29941,
            "requested_batch": 29941,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 21446132.441,
            "ns_per_operation": 791.341,
            "observed_batch": 27101,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 21183946.143,
            "ns_per_operation": 707.523,
            "observed_batch": 29941,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 29941,
        "source_sha256": "637b96cd8926ffadf728af4949b43fa6b62fffb61968c14c716e0c203a1e5e6d",
        "wasm_sha256": "d43d5dd71d5ca1c3dae820aa85c13a2d69415e028b8d48097fed4558f743163a",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_br_table_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 13150,
            "sides": [
              {
                "calibrated_total_ns": 29859336.8,
                "ns_per_operation": 2270.672,
                "observed_batch": 13150,
                "requested_batch": 13150,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 29962801.0,
                "ns_per_operation": 2278.54,
                "observed_batch": 13150,
                "requested_batch": 13150,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 29859336.8,
            "ns_per_operation": 2270.672,
            "observed_batch": 13150,
            "requested_batch": 13150,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 29962801.0,
            "ns_per_operation": 2278.54,
            "observed_batch": 13150,
            "requested_batch": 13150,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 28158489.5,
            "ns_per_operation": 2141.33,
            "observed_batch": 13150,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 26888061.616000004,
            "ns_per_operation": 2138.896,
            "observed_batch": 12571,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 13150,
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
                "calibrated_total_ns": 24157541.0,
                "ns_per_operation": 12078770.5,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 22549293.0,
                "ns_per_operation": 11274646.5,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 24157541.0,
            "ns_per_operation": 12078770.5,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22549293.0,
            "ns_per_operation": 11274646.5,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 37826978.0,
            "ns_per_operation": 18913489.0,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 20679611.0,
            "ns_per_operation": 10339805.5,
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
            "batch": 16,
            "sides": [
              {
                "calibrated_total_ns": 24769812.0,
                "ns_per_operation": 1548113.25,
                "observed_batch": 16,
                "requested_batch": 16,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 27113228.992,
                "ns_per_operation": 1694576.812,
                "observed_batch": 16,
                "requested_batch": 16,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 24769812.0,
            "ns_per_operation": 1548113.25,
            "observed_batch": 16,
            "requested_batch": 16,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27113228.992,
            "ns_per_operation": 1694576.812,
            "observed_batch": 16,
            "requested_batch": 16,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 26036744.006,
            "ns_per_operation": 2002826.462,
            "observed_batch": 13,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22880240.0,
            "ns_per_operation": 1430015.0,
            "observed_batch": 16,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 16,
        "source_sha256": "637b96cd8926ffadf728af4949b43fa6b62fffb61968c14c716e0c203a1e5e6d",
        "wasm_sha256": "d43d5dd71d5ca1c3dae820aa85c13a2d69415e028b8d48097fed4558f743163a",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_br_table_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1413,
            "sides": [
              {
                "calibrated_total_ns": 57777301.529999994,
                "ns_per_operation": 40889.81,
                "observed_batch": 1413,
                "requested_batch": 1413,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 66807873.549,
                "ns_per_operation": 47280.873,
                "observed_batch": 1413,
                "requested_batch": 1413,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 57777301.529999994,
            "ns_per_operation": 40889.81,
            "observed_batch": 1413,
            "requested_batch": 1413,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 66807873.549,
            "ns_per_operation": 47280.873,
            "observed_batch": 1413,
            "requested_batch": 1413,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 55238247.144999996,
            "ns_per_operation": 45840.869,
            "observed_batch": 1205,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 56282057.865,
            "ns_per_operation": 39831.605,
            "observed_batch": 1413,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1413,
        "source_sha256": "637b96cd8926ffadf728af4949b43fa6b62fffb61968c14c716e0c203a1e5e6d",
        "wasm_sha256": "d43d5dd71d5ca1c3dae820aa85c13a2d69415e028b8d48097fed4558f743163a",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_br_table_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 767,
            "sides": [
              {
                "calibrated_total_ns": 46401422.197,
                "ns_per_operation": 60497.291,
                "observed_batch": 767,
                "requested_batch": 767,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 40843997.909,
                "ns_per_operation": 53251.627,
                "observed_batch": 767,
                "requested_batch": 767,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 46401422.197,
            "ns_per_operation": 60497.291,
            "observed_batch": 767,
            "requested_batch": 767,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 40843997.909,
            "ns_per_operation": 53251.627,
            "observed_batch": 767,
            "requested_batch": 767,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 17387708.03,
            "ns_per_operation": 44019.514,
            "observed_batch": 395,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 34678320.378,
            "ns_per_operation": 45212.934,
            "observed_batch": 767,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 767,
        "source_sha256": "637b96cd8926ffadf728af4949b43fa6b62fffb61968c14c716e0c203a1e5e6d",
        "wasm_sha256": "d43d5dd71d5ca1c3dae820aa85c13a2d69415e028b8d48097fed4558f743163a",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_branch_entropy_n0": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 24184,
            "sides": [
              {
                "calibrated_total_ns": 21133188.400000002,
                "ns_per_operation": 873.85,
                "observed_batch": 24184,
                "requested_batch": 24184,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 25663456.2,
                "ns_per_operation": 1061.175,
                "observed_batch": 24184,
                "requested_batch": 24184,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 21133188.400000002,
            "ns_per_operation": 873.85,
            "observed_batch": 24184,
            "requested_batch": 24184,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 25663456.2,
            "ns_per_operation": 1061.175,
            "observed_batch": 24184,
            "requested_batch": 24184,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 23617992.648999996,
            "ns_per_operation": 1052.167,
            "observed_batch": 22447,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 20749460.871999998,
            "ns_per_operation": 857.983,
            "observed_batch": 24184,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 24184,
        "source_sha256": "6b4861d8bfbbd7e2718e577ec2d479bfff32e94dc47a3b7dda9a1b363baa98b6",
        "wasm_sha256": "e4c87dbab50a2438cb1f749bc6fd6f9c834c81484acd42864735bd1ad1b6dec0",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_branch_entropy_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 12691,
            "sides": [
              {
                "calibrated_total_ns": 27585474.875,
                "ns_per_operation": 2173.625,
                "observed_batch": 12691,
                "requested_batch": 12691,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 28990330.502,
                "ns_per_operation": 2284.322,
                "observed_batch": 12691,
                "requested_batch": 12691,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 27585474.875,
            "ns_per_operation": 2173.625,
            "observed_batch": 12691,
            "requested_batch": 12691,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 28990330.502,
            "ns_per_operation": 2284.322,
            "observed_batch": 12691,
            "requested_batch": 12691,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 28869461.417999998,
            "ns_per_operation": 2274.798,
            "observed_batch": 12691,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 65111.01000000001,
            "ns_per_operation": 2170.367,
            "observed_batch": 30,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 12691,
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
                "calibrated_total_ns": 28993574.0,
                "ns_per_operation": 14496787.0,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 30346326.0,
                "ns_per_operation": 15173163.0,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 28993574.0,
            "ns_per_operation": 14496787.0,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 30346326.0,
            "ns_per_operation": 15173163.0,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 27815609.0,
            "ns_per_operation": 13907804.5,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 31359900.0,
            "ns_per_operation": 15679950.0,
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
            "batch": 21,
            "sides": [
              {
                "calibrated_total_ns": 90941504.00999999,
                "ns_per_operation": 4330547.81,
                "observed_batch": 21,
                "requested_batch": 21,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 50505408.99599999,
                "ns_per_operation": 2405019.476,
                "observed_batch": 21,
                "requested_batch": 21,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 90941504.00999999,
            "ns_per_operation": 4330547.81,
            "observed_batch": 21,
            "requested_batch": 21,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 50505408.99599999,
            "ns_per_operation": 2405019.476,
            "observed_batch": 21,
            "requested_batch": 21,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 29174011.994999997,
            "ns_per_operation": 1944934.133,
            "observed_batch": 15,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 35614320.0,
            "ns_per_operation": 1695920.0,
            "observed_batch": 21,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 21,
        "source_sha256": "6b4861d8bfbbd7e2718e577ec2d479bfff32e94dc47a3b7dda9a1b363baa98b6",
        "wasm_sha256": "e4c87dbab50a2438cb1f749bc6fd6f9c834c81484acd42864735bd1ad1b6dec0",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_branch_entropy_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1420,
            "sides": [
              {
                "calibrated_total_ns": 30723688.000000004,
                "ns_per_operation": 21636.4,
                "observed_batch": 1420,
                "requested_batch": 1420,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 34363883.56,
                "ns_per_operation": 24199.918,
                "observed_batch": 1420,
                "requested_batch": 1420,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 30723688.000000004,
            "ns_per_operation": 21636.4,
            "observed_batch": 1420,
            "requested_batch": 1420,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 34363883.56,
            "ns_per_operation": 24199.918,
            "observed_batch": 1420,
            "requested_batch": 1420,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 31289238.036,
            "ns_per_operation": 22128.174,
            "observed_batch": 1414,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 38601345.32,
            "ns_per_operation": 27184.046,
            "observed_batch": 1420,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1420,
        "source_sha256": "6b4861d8bfbbd7e2718e577ec2d479bfff32e94dc47a3b7dda9a1b363baa98b6",
        "wasm_sha256": "e4c87dbab50a2438cb1f749bc6fd6f9c834c81484acd42864735bd1ad1b6dec0",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_branch_entropy_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 773,
            "sides": [
              {
                "calibrated_total_ns": 37311924.632,
                "ns_per_operation": 48268.984,
                "observed_batch": 773,
                "requested_batch": 773,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 39123276.980000004,
                "ns_per_operation": 50612.26,
                "observed_batch": 773,
                "requested_batch": 773,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 37311924.632,
            "ns_per_operation": 48268.984,
            "observed_batch": 773,
            "requested_batch": 773,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 39123276.980000004,
            "ns_per_operation": 50612.26,
            "observed_batch": 773,
            "requested_batch": 773,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 41862440.348000005,
            "ns_per_operation": 54225.959,
            "observed_batch": 772,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 34229468.863,
            "ns_per_operation": 44281.331,
            "observed_batch": 773,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 773,
        "source_sha256": "6b4861d8bfbbd7e2718e577ec2d479bfff32e94dc47a3b7dda9a1b363baa98b6",
        "wasm_sha256": "e4c87dbab50a2438cb1f749bc6fd6f9c834c81484acd42864735bd1ad1b6dec0",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_call_ref_n0": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 23256,
            "sides": [
              {
                "calibrated_total_ns": 16137524.448,
                "ns_per_operation": 693.908,
                "observed_batch": 23256,
                "requested_batch": 23256,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 16794111.096,
                "ns_per_operation": 722.141,
                "observed_batch": 23256,
                "requested_batch": 23256,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 31705,
            "sides": [
              {
                "calibrated_total_ns": 22369652.98,
                "ns_per_operation": 705.556,
                "observed_batch": 31705,
                "requested_batch": 31705,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 31737465.92,
                "ns_per_operation": 1001.024,
                "observed_batch": 31705,
                "requested_batch": 31705,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 22369652.98,
            "ns_per_operation": 705.556,
            "observed_batch": 31705,
            "requested_batch": 31705,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 31737465.92,
            "ns_per_operation": 1001.024,
            "observed_batch": 31705,
            "requested_batch": 31705,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 12261296.019000001,
            "ns_per_operation": 678.657,
            "observed_batch": 18067,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 17771188.68,
            "ns_per_operation": 764.155,
            "observed_batch": 23256,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 31705,
        "source_sha256": "0323154b79000eeeb0a63310541ec91b6c4d91e32a6a3406125a30f2c8e53a12",
        "wasm_sha256": "4f766dc9e29324ea942c892982363dec0ade98565288878a70967ce3b33f322b",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_call_ref_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 12361,
            "sides": [
              {
                "calibrated_total_ns": 27643942.624,
                "ns_per_operation": 2236.384,
                "observed_batch": 12361,
                "requested_batch": 12361,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 25368814.047000002,
                "ns_per_operation": 2052.327,
                "observed_batch": 12361,
                "requested_batch": 12361,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 27643942.624,
            "ns_per_operation": 2236.384,
            "observed_batch": 12361,
            "requested_batch": 12361,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 25368814.047000002,
            "ns_per_operation": 2052.327,
            "observed_batch": 12361,
            "requested_batch": 12361,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 35899806.631,
            "ns_per_operation": 2999.399,
            "observed_batch": 11969,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 23625060.138,
            "ns_per_operation": 1911.258,
            "observed_batch": 12361,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 12361,
        "source_sha256": "0323154b79000eeeb0a63310541ec91b6c4d91e32a6a3406125a30f2c8e53a12",
        "wasm_sha256": "4f766dc9e29324ea942c892982363dec0ade98565288878a70967ce3b33f322b",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_call_ref_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 3,
            "sides": [
              {
                "calibrated_total_ns": 29733882.999,
                "ns_per_operation": 9911294.333,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 29361239.001,
                "ns_per_operation": 9787079.667,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 29733882.999,
            "ns_per_operation": 9911294.333,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 29361239.001,
            "ns_per_operation": 9787079.667,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 30094061.001,
            "ns_per_operation": 10031353.667,
            "observed_batch": 3,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 31367112.999,
            "ns_per_operation": 10455704.333,
            "observed_batch": 3,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 3,
        "source_sha256": "0323154b79000eeeb0a63310541ec91b6c4d91e32a6a3406125a30f2c8e53a12",
        "wasm_sha256": "4f766dc9e29324ea942c892982363dec0ade98565288878a70967ce3b33f322b",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_call_ref_n1024": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 22,
            "sides": [
              {
                "calibrated_total_ns": 23966419.994,
                "ns_per_operation": 1089382.727,
                "observed_batch": 22,
                "requested_batch": 22,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 22046252.998,
                "ns_per_operation": 1002102.409,
                "observed_batch": 22,
                "requested_batch": 22,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 23966419.994,
            "ns_per_operation": 1089382.727,
            "observed_batch": 22,
            "requested_batch": 22,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22046252.998,
            "ns_per_operation": 1002102.409,
            "observed_batch": 22,
            "requested_batch": 22,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 22663414.004,
            "ns_per_operation": 1030155.182,
            "observed_batch": 22,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 15061477.998000002,
            "ns_per_operation": 1075819.857,
            "observed_batch": 14,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 22,
        "source_sha256": "0323154b79000eeeb0a63310541ec91b6c4d91e32a6a3406125a30f2c8e53a12",
        "wasm_sha256": "4f766dc9e29324ea942c892982363dec0ade98565288878a70967ce3b33f322b",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_call_ref_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1427,
            "sides": [
              {
                "calibrated_total_ns": 26718119.132999998,
                "ns_per_operation": 18723.279,
                "observed_batch": 1427,
                "requested_batch": 1427,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 25604258.586000003,
                "ns_per_operation": 17942.718,
                "observed_batch": 1427,
                "requested_batch": 1427,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 26718119.132999998,
            "ns_per_operation": 18723.279,
            "observed_batch": 1427,
            "requested_batch": 1427,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 25604258.586000003,
            "ns_per_operation": 17942.718,
            "observed_batch": 1427,
            "requested_batch": 1427,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 25639712.400999997,
            "ns_per_operation": 17967.563,
            "observed_batch": 1427,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 7121109.915,
            "ns_per_operation": 17159.301,
            "observed_batch": 415,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1427,
        "source_sha256": "0323154b79000eeeb0a63310541ec91b6c4d91e32a6a3406125a30f2c8e53a12",
        "wasm_sha256": "4f766dc9e29324ea942c892982363dec0ade98565288878a70967ce3b33f322b",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_call_ref_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 853,
            "sides": [
              {
                "calibrated_total_ns": 40721036.889,
                "ns_per_operation": 47738.613,
                "observed_batch": 853,
                "requested_batch": 853,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 32107372.09,
                "ns_per_operation": 37640.53,
                "observed_batch": 853,
                "requested_batch": 853,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 40721036.889,
            "ns_per_operation": 47738.613,
            "observed_batch": 853,
            "requested_batch": 853,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 32107372.09,
            "ns_per_operation": 37640.53,
            "observed_batch": 853,
            "requested_batch": 853,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 30216123.63,
            "ns_per_operation": 35674.29,
            "observed_batch": 847,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 30511641.959,
            "ns_per_operation": 35769.803,
            "observed_batch": 853,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 853,
        "source_sha256": "0323154b79000eeeb0a63310541ec91b6c4d91e32a6a3406125a30f2c8e53a12",
        "wasm_sha256": "4f766dc9e29324ea942c892982363dec0ade98565288878a70967ce3b33f322b",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_dependent_n0": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 24184,
            "sides": [
              {
                "calibrated_total_ns": 19179459.776,
                "ns_per_operation": 793.064,
                "observed_batch": 24184,
                "requested_batch": 24184,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 35356306.664,
                "ns_per_operation": 1461.971,
                "observed_batch": 24184,
                "requested_batch": 24184,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 27741,
            "sides": [
              {
                "calibrated_total_ns": 41239049.334,
                "ns_per_operation": 1486.574,
                "observed_batch": 27741,
                "requested_batch": 27741,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 20823559.722000003,
                "ns_per_operation": 750.642,
                "observed_batch": 27741,
                "requested_batch": 27741,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 41239049.334,
            "ns_per_operation": 1486.574,
            "observed_batch": 27741,
            "requested_batch": 27741,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 20823559.722000003,
            "ns_per_operation": 750.642,
            "observed_batch": 27741,
            "requested_batch": 27741,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 20081571.344,
            "ns_per_operation": 830.366,
            "observed_batch": 24184,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 19343864.666,
            "ns_per_operation": 828.857,
            "observed_batch": 23338,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 27741,
        "source_sha256": "0dd5e9778eb09bac9ee432e5fece3f432d034ea9878d2302972142501f0c60c6",
        "wasm_sha256": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_dependent_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 16195,
            "sides": [
              {
                "calibrated_total_ns": 27000417.365000002,
                "ns_per_operation": 1667.207,
                "observed_batch": 16195,
                "requested_batch": 16195,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 32569829.28,
                "ns_per_operation": 2011.104,
                "observed_batch": 16195,
                "requested_batch": 16195,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 27000417.365000002,
            "ns_per_operation": 1667.207,
            "observed_batch": 16195,
            "requested_batch": 16195,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 32569829.28,
            "ns_per_operation": 2011.104,
            "observed_batch": 16195,
            "requested_batch": 16195,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 28436953.105,
            "ns_per_operation": 1798.555,
            "observed_batch": 15811,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27648978.529999997,
            "ns_per_operation": 1707.254,
            "observed_batch": 16195,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 16195,
        "source_sha256": "0dd5e9778eb09bac9ee432e5fece3f432d034ea9878d2302972142501f0c60c6",
        "wasm_sha256": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_dependent_n10000": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 3,
            "sides": [
              {
                "calibrated_total_ns": 23134349.001000002,
                "ns_per_operation": 7711449.667,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 19838070.998999998,
                "ns_per_operation": 6612690.333,
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
                "calibrated_total_ns": 26126023.0,
                "ns_per_operation": 6531505.75,
                "observed_batch": 4,
                "requested_batch": 4,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 27058087.0,
                "ns_per_operation": 6764521.75,
                "observed_batch": 4,
                "requested_batch": 4,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 26126023.0,
            "ns_per_operation": 6531505.75,
            "observed_batch": 4,
            "requested_batch": 4,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27058087.0,
            "ns_per_operation": 6764521.75,
            "observed_batch": 4,
            "requested_batch": 4,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 14782026.0,
            "ns_per_operation": 7391013.0,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 20956730.001000002,
            "ns_per_operation": 6985576.667,
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
            "batch": 23,
            "sides": [
              {
                "calibrated_total_ns": 23041441.009,
                "ns_per_operation": 1001801.783,
                "observed_batch": 23,
                "requested_batch": 23,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 39592634.010000005,
                "ns_per_operation": 1721418.87,
                "observed_batch": 23,
                "requested_batch": 23,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 23041441.009,
            "ns_per_operation": 1001801.783,
            "observed_batch": 23,
            "requested_batch": 23,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 39592634.010000005,
            "ns_per_operation": 1721418.87,
            "observed_batch": 23,
            "requested_batch": 23,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 18556391.007000003,
            "ns_per_operation": 806799.609,
            "observed_batch": 23,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 3840052.0,
            "ns_per_operation": 960013.0,
            "observed_batch": 4,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 23,
        "source_sha256": "0dd5e9778eb09bac9ee432e5fece3f432d034ea9878d2302972142501f0c60c6",
        "wasm_sha256": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_dependent_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 2354,
            "sides": [
              {
                "calibrated_total_ns": 35905766.798,
                "ns_per_operation": 15253.087,
                "observed_batch": 2354,
                "requested_batch": 2354,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 33059173.466,
                "ns_per_operation": 14043.829,
                "observed_batch": 2354,
                "requested_batch": 2354,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 35905766.798,
            "ns_per_operation": 15253.087,
            "observed_batch": 2354,
            "requested_batch": 2354,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 33059173.466,
            "ns_per_operation": 14043.829,
            "observed_batch": 2354,
            "requested_batch": 2354,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 32519630.019999996,
            "ns_per_operation": 18797.474,
            "observed_batch": 1730,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 33657423.734,
            "ns_per_operation": 14297.971,
            "observed_batch": 2354,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 2354,
        "source_sha256": "0dd5e9778eb09bac9ee432e5fece3f432d034ea9878d2302972142501f0c60c6",
        "wasm_sha256": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_dependent_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1324,
            "sides": [
              {
                "calibrated_total_ns": 39698605.484,
                "ns_per_operation": 29983.841,
                "observed_batch": 1324,
                "requested_batch": 1324,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 42800233.04,
                "ns_per_operation": 32326.46,
                "observed_batch": 1324,
                "requested_batch": 1324,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 39698605.484,
            "ns_per_operation": 29983.841,
            "observed_batch": 1324,
            "requested_batch": 1324,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 42800233.04,
            "ns_per_operation": 32326.46,
            "observed_batch": 1324,
            "requested_batch": 1324,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 28501876.428000003,
            "ns_per_operation": 21527.097,
            "observed_batch": 1324,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 3129300.9930000002,
            "ns_per_operation": 26296.647,
            "observed_batch": 119,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1324,
        "source_sha256": "0dd5e9778eb09bac9ee432e5fece3f432d034ea9878d2302972142501f0c60c6",
        "wasm_sha256": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_direct_calls_n0": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 26042,
            "sides": [
              {
                "calibrated_total_ns": 28476979.084,
                "ns_per_operation": 1093.502,
                "observed_batch": 26042,
                "requested_batch": 26042,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 29772985.256,
                "ns_per_operation": 1143.268,
                "observed_batch": 26042,
                "requested_batch": 26042,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 28476979.084,
            "ns_per_operation": 1093.502,
            "observed_batch": 26042,
            "requested_batch": 26042,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 29772985.256,
            "ns_per_operation": 1143.268,
            "observed_batch": 26042,
            "requested_batch": 26042,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 31013279.448,
            "ns_per_operation": 1218.788,
            "observed_batch": 25446,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 38601770.138,
            "ns_per_operation": 1482.289,
            "observed_batch": 26042,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 26042,
        "source_sha256": "0a075601a2a651f9cb95e8ee0404c64bbf076d17bf242b9c88fecbb8cb973826",
        "wasm_sha256": "f6adbecce99c05bf06f213f4f5c0d5d9f001074acbbb2e6bbe814696bb866e45",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_direct_calls_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 13004,
            "sides": [
              {
                "calibrated_total_ns": 24791605.84,
                "ns_per_operation": 1906.46,
                "observed_batch": 13004,
                "requested_batch": 13004,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 29998589.495999996,
                "ns_per_operation": 2306.874,
                "observed_batch": 13004,
                "requested_batch": 13004,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 24791605.84,
            "ns_per_operation": 1906.46,
            "observed_batch": 13004,
            "requested_batch": 13004,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 29998589.495999996,
            "ns_per_operation": 2306.874,
            "observed_batch": 13004,
            "requested_batch": 13004,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 17500557.696,
            "ns_per_operation": 2037.792,
            "observed_batch": 8588,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 25903915.984,
            "ns_per_operation": 1991.996,
            "observed_batch": 13004,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 13004,
        "source_sha256": "0a075601a2a651f9cb95e8ee0404c64bbf076d17bf242b9c88fecbb8cb973826",
        "wasm_sha256": "f6adbecce99c05bf06f213f4f5c0d5d9f001074acbbb2e6bbe814696bb866e45",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_direct_calls_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 2,
            "sides": [
              {
                "calibrated_total_ns": 23535691.0,
                "ns_per_operation": 11767845.5,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 22268059.0,
                "ns_per_operation": 11134029.5,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 23535691.0,
            "ns_per_operation": 11767845.5,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22268059.0,
            "ns_per_operation": 11134029.5,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 23038549.0,
            "ns_per_operation": 11519274.5,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 20728745.0,
            "ns_per_operation": 10364372.5,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 2,
        "source_sha256": "0a075601a2a651f9cb95e8ee0404c64bbf076d17bf242b9c88fecbb8cb973826",
        "wasm_sha256": "f6adbecce99c05bf06f213f4f5c0d5d9f001074acbbb2e6bbe814696bb866e45",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_direct_calls_n1024": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 26,
            "sides": [
              {
                "calibrated_total_ns": 28735317.987999998,
                "ns_per_operation": 1105204.538,
                "observed_batch": 26,
                "requested_batch": 26,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 26520797.004,
                "ns_per_operation": 1020030.654,
                "observed_batch": 26,
                "requested_batch": 26,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 28735317.987999998,
            "ns_per_operation": 1105204.538,
            "observed_batch": 26,
            "requested_batch": 26,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 26520797.004,
            "ns_per_operation": 1020030.654,
            "observed_batch": 26,
            "requested_batch": 26,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 31408892.007999998,
            "ns_per_operation": 1208034.308,
            "observed_batch": 26,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 23290901.998999998,
            "ns_per_operation": 1012647.913,
            "observed_batch": 23,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 26,
        "source_sha256": "0a075601a2a651f9cb95e8ee0404c64bbf076d17bf242b9c88fecbb8cb973826",
        "wasm_sha256": "f6adbecce99c05bf06f213f4f5c0d5d9f001074acbbb2e6bbe814696bb866e45",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_direct_calls_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1700,
            "sides": [
              {
                "calibrated_total_ns": 49631727.8,
                "ns_per_operation": 29195.134,
                "observed_batch": 1700,
                "requested_batch": 1700,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 31445989.7,
                "ns_per_operation": 18497.641,
                "observed_batch": 1700,
                "requested_batch": 1700,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 49631727.8,
            "ns_per_operation": 29195.134,
            "observed_batch": 1700,
            "requested_batch": 1700,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 31445989.7,
            "ns_per_operation": 18497.641,
            "observed_batch": 1700,
            "requested_batch": 1700,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 40482506.300000004,
            "ns_per_operation": 23813.239,
            "observed_batch": 1700,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 28446182.128,
            "ns_per_operation": 18863.516,
            "observed_batch": 1508,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1700,
        "source_sha256": "0a075601a2a651f9cb95e8ee0404c64bbf076d17bf242b9c88fecbb8cb973826",
        "wasm_sha256": "f6adbecce99c05bf06f213f4f5c0d5d9f001074acbbb2e6bbe814696bb866e45",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_direct_calls_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 941,
            "sides": [
              {
                "calibrated_total_ns": 33746186.227,
                "ns_per_operation": 35862.047,
                "observed_batch": 941,
                "requested_batch": 941,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 31628635.107,
                "ns_per_operation": 33611.727,
                "observed_batch": 941,
                "requested_batch": 941,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 33746186.227,
            "ns_per_operation": 35862.047,
            "observed_batch": 941,
            "requested_batch": 941,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 31628635.107,
            "ns_per_operation": 33611.727,
            "observed_batch": 941,
            "requested_batch": 941,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 38510802.341,
            "ns_per_operation": 40925.401,
            "observed_batch": 941,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 44173744.293000005,
            "ns_per_operation": 47345.921,
            "observed_batch": 933,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 941,
        "source_sha256": "0a075601a2a651f9cb95e8ee0404c64bbf076d17bf242b9c88fecbb8cb973826",
        "wasm_sha256": "f6adbecce99c05bf06f213f4f5c0d5d9f001074acbbb2e6bbe814696bb866e45",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_dynamic_api_n0": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 56023,
            "sides": [
              {
                "calibrated_total_ns": 18998239.645,
                "ns_per_operation": 339.115,
                "observed_batch": 56023,
                "requested_batch": 56023,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 22575476.264000002,
                "ns_per_operation": 402.968,
                "observed_batch": 56023,
                "requested_batch": 56023,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 64875,
            "sides": [
              {
                "calibrated_total_ns": 30979369.5,
                "ns_per_operation": 477.524,
                "observed_batch": 64875,
                "requested_batch": 64875,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 37543357.125,
                "ns_per_operation": 578.703,
                "observed_batch": 64875,
                "requested_batch": 64875,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 30979369.5,
            "ns_per_operation": 477.524,
            "observed_batch": 64875,
            "requested_batch": 64875,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 37543357.125,
            "ns_per_operation": 578.703,
            "observed_batch": 64875,
            "requested_batch": 64875,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 13377982.482,
            "ns_per_operation": 328.423,
            "observed_batch": 40734,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 18636499.134,
            "ns_per_operation": 332.658,
            "observed_batch": 56023,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 64875,
        "source_sha256": "f02cc93a4a43691ce5be8782fa2df454fc2468a7935117ea18104a7dce22cb2d",
        "wasm_sha256": "c493d0aae68d0c5aa67fc0b562e616c45eb3d42b0302951158a763fa202ce589",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_dynamic_api_n1": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 51680,
            "sides": [
              {
                "calibrated_total_ns": 23959881.6,
                "ns_per_operation": 463.62,
                "observed_batch": 51680,
                "requested_batch": 51680,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 16828868.48,
                "ns_per_operation": 325.636,
                "observed_batch": 51680,
                "requested_batch": 51680,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 67561,
            "sides": [
              {
                "calibrated_total_ns": 20779669.209000003,
                "ns_per_operation": 307.569,
                "observed_batch": 67561,
                "requested_batch": 67561,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 23694993.92,
                "ns_per_operation": 350.72,
                "observed_batch": 67561,
                "requested_batch": 67561,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 20779669.209000003,
            "ns_per_operation": 307.569,
            "observed_batch": 67561,
            "requested_batch": 67561,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 23694993.92,
            "ns_per_operation": 350.72,
            "observed_batch": 67561,
            "requested_batch": 67561,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 17834302.88,
            "ns_per_operation": 345.091,
            "observed_batch": 51680,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 17231400.623,
            "ns_per_operation": 402.349,
            "observed_batch": 42827,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 67561,
        "source_sha256": "f02cc93a4a43691ce5be8782fa2df454fc2468a7935117ea18104a7dce22cb2d",
        "wasm_sha256": "c493d0aae68d0c5aa67fc0b562e616c45eb3d42b0302951158a763fa202ce589",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_dynamic_api_n10000": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 47962,
            "sides": [
              {
                "calibrated_total_ns": 15794126.41,
                "ns_per_operation": 329.305,
                "observed_batch": 47962,
                "requested_batch": 47962,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 16410054.413999999,
                "ns_per_operation": 342.147,
                "observed_batch": 47962,
                "requested_batch": 47962,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 66808,
            "sides": [
              {
                "calibrated_total_ns": 23424421.384,
                "ns_per_operation": 350.623,
                "observed_batch": 66808,
                "requested_batch": 66808,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 30275782.208,
                "ns_per_operation": 453.176,
                "observed_batch": 66808,
                "requested_batch": 66808,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 23424421.384,
            "ns_per_operation": 350.623,
            "observed_batch": 66808,
            "requested_batch": 66808,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 30275782.208,
            "ns_per_operation": 453.176,
            "observed_batch": 66808,
            "requested_batch": 66808,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 16246141.865999999,
            "ns_per_operation": 342.789,
            "observed_batch": 47394,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 17267039.43,
            "ns_per_operation": 360.015,
            "observed_batch": 47962,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 66808,
        "source_sha256": "f02cc93a4a43691ce5be8782fa2df454fc2468a7935117ea18104a7dce22cb2d",
        "wasm_sha256": "c493d0aae68d0c5aa67fc0b562e616c45eb3d42b0302951158a763fa202ce589",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_dynamic_api_n1024": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 52357,
            "sides": [
              {
                "calibrated_total_ns": 21314482.343,
                "ns_per_operation": 407.099,
                "observed_batch": 52357,
                "requested_batch": 52357,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 24584072.279000003,
                "ns_per_operation": 469.547,
                "observed_batch": 52357,
                "requested_batch": 52357,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 21314482.343,
            "ns_per_operation": 407.099,
            "observed_batch": 52357,
            "requested_batch": 52357,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 24584072.279000003,
            "ns_per_operation": 469.547,
            "observed_batch": 52357,
            "requested_batch": 52357,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 19263355.86,
            "ns_per_operation": 439.201,
            "observed_batch": 43860,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22232509.981,
            "ns_per_operation": 424.633,
            "observed_batch": 52357,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 52357,
        "source_sha256": "f02cc93a4a43691ce5be8782fa2df454fc2468a7935117ea18104a7dce22cb2d",
        "wasm_sha256": "c493d0aae68d0c5aa67fc0b562e616c45eb3d42b0302951158a763fa202ce589",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_dynamic_api_n17": {
        "attempt_count": 3,
        "attempts": [
          {
            "batch": 46729,
            "sides": [
              {
                "calibrated_total_ns": 16795804.47,
                "ns_per_operation": 359.43,
                "observed_batch": 46729,
                "requested_batch": 46729,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 16129168.556,
                "ns_per_operation": 345.164,
                "observed_batch": 46729,
                "requested_batch": 46729,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 63738,
            "sides": [
              {
                "calibrated_total_ns": 20558245.734,
                "ns_per_operation": 322.543,
                "observed_batch": 63738,
                "requested_batch": 63738,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 19335814.632,
                "ns_per_operation": 303.364,
                "observed_batch": 63738,
                "requested_batch": 63738,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 72521,
            "sides": [
              {
                "calibrated_total_ns": 22324429.514,
                "ns_per_operation": 307.834,
                "observed_batch": 72521,
                "requested_batch": 72521,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 23389037.794,
                "ns_per_operation": 322.514,
                "observed_batch": 72521,
                "requested_batch": 72521,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 22324429.514,
            "ns_per_operation": 307.834,
            "observed_batch": 72521,
            "requested_batch": 72521,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 23389037.794,
            "ns_per_operation": 322.514,
            "observed_batch": 72521,
            "requested_batch": 72521,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 2266103.642,
            "ns_per_operation": 296.494,
            "observed_batch": 7643,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 16583514.623,
            "ns_per_operation": 354.887,
            "observed_batch": 46729,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 72521,
        "source_sha256": "f02cc93a4a43691ce5be8782fa2df454fc2468a7935117ea18104a7dce22cb2d",
        "wasm_sha256": "c493d0aae68d0c5aa67fc0b562e616c45eb3d42b0302951158a763fa202ce589",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_dynamic_api_n32": {
        "attempt_count": 3,
        "attempts": [
          {
            "batch": 50000,
            "sides": [
              {
                "calibrated_total_ns": 26096050.000000004,
                "ns_per_operation": 521.921,
                "observed_batch": 50000,
                "requested_batch": 50000,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 18067350.0,
                "ns_per_operation": 361.347,
                "observed_batch": 50000,
                "requested_batch": 50000,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 60884,
            "sides": [
              {
                "calibrated_total_ns": 17759923.684,
                "ns_per_operation": 291.701,
                "observed_batch": 60884,
                "requested_batch": 60884,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 23203379.472,
                "ns_per_operation": 381.108,
                "observed_batch": 60884,
                "requested_batch": 60884,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 75420,
            "sides": [
              {
                "calibrated_total_ns": 26818371.54,
                "ns_per_operation": 355.587,
                "observed_batch": 75420,
                "requested_batch": 75420,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 22312931.58,
                "ns_per_operation": 295.849,
                "observed_batch": 75420,
                "requested_batch": 75420,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 26818371.54,
            "ns_per_operation": 355.587,
            "observed_batch": 75420,
            "requested_batch": 75420,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22312931.58,
            "ns_per_operation": 295.849,
            "observed_batch": 75420,
            "requested_batch": 75420,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 15745362.520000001,
            "ns_per_operation": 330.646,
            "observed_batch": 47620,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 15938749.999999998,
            "ns_per_operation": 318.775,
            "observed_batch": 50000,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 75420,
        "source_sha256": "f02cc93a4a43691ce5be8782fa2df454fc2468a7935117ea18104a7dce22cb2d",
        "wasm_sha256": "c493d0aae68d0c5aa67fc0b562e616c45eb3d42b0302951158a763fa202ce589",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_host_call_n0": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 28450,
            "sides": [
              {
                "calibrated_total_ns": 17080213.55,
                "ns_per_operation": 600.359,
                "observed_batch": 28450,
                "requested_batch": 28450,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 18123560.400000002,
                "ns_per_operation": 637.032,
                "observed_batch": 28450,
                "requested_batch": 28450,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 36645,
            "sides": [
              {
                "calibrated_total_ns": 24652374.075,
                "ns_per_operation": 672.735,
                "observed_batch": 36645,
                "requested_batch": 36645,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 22982534.715,
                "ns_per_operation": 627.167,
                "observed_batch": 36645,
                "requested_batch": 36645,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 24652374.075,
            "ns_per_operation": 672.735,
            "observed_batch": 36645,
            "requested_batch": 36645,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22982534.715,
            "ns_per_operation": 627.167,
            "observed_batch": 36645,
            "requested_batch": 36645,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 18682432.200000003,
            "ns_per_operation": 656.676,
            "observed_batch": 28450,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 16953723.204,
            "ns_per_operation": 660.346,
            "observed_batch": 25674,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 36645,
        "source_sha256": "4c4026de620eff0b1e9cb4ed4e0440e6045d921267e3962715f6430ac68f5120",
        "wasm_sha256": "c60311d57c929c3ff1d29adb70501e7bbf8419517acbd86c8d072438b610ebb7",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_host_call_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 12587,
            "sides": [
              {
                "calibrated_total_ns": 22073884.835,
                "ns_per_operation": 1753.705,
                "observed_batch": 12587,
                "requested_batch": 12587,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 21023826.947,
                "ns_per_operation": 1670.281,
                "observed_batch": 12587,
                "requested_batch": 12587,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 22073884.835,
            "ns_per_operation": 1753.705,
            "observed_batch": 12587,
            "requested_batch": 12587,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 21023826.947,
            "ns_per_operation": 1670.281,
            "observed_batch": 12587,
            "requested_batch": 12587,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 8170303.341999999,
            "ns_per_operation": 2102.497,
            "observed_batch": 3886,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 19833726.097,
            "ns_per_operation": 1575.731,
            "observed_batch": 12587,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 12587,
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
                "calibrated_total_ns": 67792991.001,
                "ns_per_operation": 22597663.667,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 24269844.0,
                "ns_per_operation": 8089948.0,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 67792991.001,
            "ns_per_operation": 22597663.667,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 24269844.0,
            "ns_per_operation": 8089948.0,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 24109350.0,
            "ns_per_operation": 8036450.0,
            "observed_batch": 3,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 23151864.998999998,
            "ns_per_operation": 7717288.333,
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
            "batch": 37,
            "sides": [
              {
                "calibrated_total_ns": 28324892.014,
                "ns_per_operation": 765537.622,
                "observed_batch": 37,
                "requested_batch": 37,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 27380973.988,
                "ns_per_operation": 740026.324,
                "observed_batch": 37,
                "requested_batch": 37,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 28324892.014,
            "ns_per_operation": 765537.622,
            "observed_batch": 37,
            "requested_batch": 37,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27380973.988,
            "ns_per_operation": 740026.324,
            "observed_batch": 37,
            "requested_batch": 37,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 23115084.993,
            "ns_per_operation": 700457.121,
            "observed_batch": 33,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27852729.982,
            "ns_per_operation": 752776.486,
            "observed_batch": 37,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 37,
        "source_sha256": "4c4026de620eff0b1e9cb4ed4e0440e6045d921267e3962715f6430ac68f5120",
        "wasm_sha256": "c60311d57c929c3ff1d29adb70501e7bbf8419517acbd86c8d072438b610ebb7",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_host_call_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 2126,
            "sides": [
              {
                "calibrated_total_ns": 30155853.69,
                "ns_per_operation": 14184.315,
                "observed_batch": 2126,
                "requested_batch": 2126,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 30809585.933999997,
                "ns_per_operation": 14491.809,
                "observed_batch": 2126,
                "requested_batch": 2126,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 30155853.69,
            "ns_per_operation": 14184.315,
            "observed_batch": 2126,
            "requested_batch": 2126,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 30809585.933999997,
            "ns_per_operation": 14491.809,
            "observed_batch": 2126,
            "requested_batch": 2126,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 29947726.263,
            "ns_per_operation": 15256.101,
            "observed_batch": 1963,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 28395855.22,
            "ns_per_operation": 13356.47,
            "observed_batch": 2126,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 2126,
        "source_sha256": "4c4026de620eff0b1e9cb4ed4e0440e6045d921267e3962715f6430ac68f5120",
        "wasm_sha256": "c60311d57c929c3ff1d29adb70501e7bbf8419517acbd86c8d072438b610ebb7",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_host_call_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1175,
            "sides": [
              {
                "calibrated_total_ns": 33042714.325,
                "ns_per_operation": 28121.459,
                "observed_batch": 1175,
                "requested_batch": 1175,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 30338155.724999998,
                "ns_per_operation": 25819.707,
                "observed_batch": 1175,
                "requested_batch": 1175,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 33042714.325,
            "ns_per_operation": 28121.459,
            "observed_batch": 1175,
            "requested_batch": 1175,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 30338155.724999998,
            "ns_per_operation": 25819.707,
            "observed_batch": 1175,
            "requested_batch": 1175,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 28234449.792,
            "ns_per_operation": 24509.071,
            "observed_batch": 1152,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 28893922.1,
            "ns_per_operation": 24590.572,
            "observed_batch": 1175,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1175,
        "source_sha256": "4c4026de620eff0b1e9cb4ed4e0440e6045d921267e3962715f6430ac68f5120",
        "wasm_sha256": "c60311d57c929c3ff1d29adb70501e7bbf8419517acbd86c8d072438b610ebb7",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_indirect_calls_n0": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 22806,
            "sides": [
              {
                "calibrated_total_ns": 17933612.13,
                "ns_per_operation": 786.355,
                "observed_batch": 22806,
                "requested_batch": 22806,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 16871057.784,
                "ns_per_operation": 739.764,
                "observed_batch": 22806,
                "requested_batch": 22806,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 29740,
            "sides": [
              {
                "calibrated_total_ns": 25507403.2,
                "ns_per_operation": 857.68,
                "observed_batch": 29740,
                "requested_batch": 29740,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 27160887.72,
                "ns_per_operation": 913.278,
                "observed_batch": 29740,
                "requested_batch": 29740,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 25507403.2,
            "ns_per_operation": 857.68,
            "observed_batch": 29740,
            "requested_batch": 29740,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27160887.72,
            "ns_per_operation": 913.278,
            "observed_batch": 29740,
            "requested_batch": 29740,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 17757048.078,
            "ns_per_operation": 778.613,
            "observed_batch": 22806,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 17643336.675,
            "ns_per_operation": 837.169,
            "observed_batch": 21075,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 29740,
        "source_sha256": "29024ae23844664a9fea4a809d1ce414de40f243f7e678ce5ea6d811843f7797",
        "wasm_sha256": "4b42e82f54ea6ab7d0f62c70766c904e41d6a408e57fa14b5abbf514c0da78a9",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_indirect_calls_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 12821,
            "sides": [
              {
                "calibrated_total_ns": 48997233.619,
                "ns_per_operation": 3821.639,
                "observed_batch": 12821,
                "requested_batch": 12821,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 24808032.413,
                "ns_per_operation": 1934.953,
                "observed_batch": 12821,
                "requested_batch": 12821,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 48997233.619,
            "ns_per_operation": 3821.639,
            "observed_batch": 12821,
            "requested_batch": 12821,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 24808032.413,
            "ns_per_operation": 1934.953,
            "observed_batch": 12821,
            "requested_batch": 12821,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 46301484.465,
            "ns_per_operation": 3849.795,
            "observed_batch": 12027,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 38626134.404,
            "ns_per_operation": 3012.724,
            "observed_batch": 12821,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 12821,
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
                "calibrated_total_ns": 32064974.001,
                "ns_per_operation": 10688324.667,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 30625596.0,
                "ns_per_operation": 10208532.0,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 32064974.001,
            "ns_per_operation": 10688324.667,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 30625596.0,
            "ns_per_operation": 10208532.0,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 32446191.0,
            "ns_per_operation": 10815397.0,
            "observed_batch": 3,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 23809231.0,
            "ns_per_operation": 11904615.5,
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
            "batch": 26,
            "sides": [
              {
                "calibrated_total_ns": 36486822.996,
                "ns_per_operation": 1403339.346,
                "observed_batch": 26,
                "requested_batch": 26,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 27957596.004,
                "ns_per_operation": 1075292.154,
                "observed_batch": 26,
                "requested_batch": 26,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 36486822.996,
            "ns_per_operation": 1403339.346,
            "observed_batch": 26,
            "requested_batch": 26,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27957596.004,
            "ns_per_operation": 1075292.154,
            "observed_batch": 26,
            "requested_batch": 26,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 25603450.992000002,
            "ns_per_operation": 1219211.952,
            "observed_batch": 21,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 33443633.99,
            "ns_per_operation": 1286293.615,
            "observed_batch": 26,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 26,
        "source_sha256": "29024ae23844664a9fea4a809d1ce414de40f243f7e678ce5ea6d811843f7797",
        "wasm_sha256": "4b42e82f54ea6ab7d0f62c70766c904e41d6a408e57fa14b5abbf514c0da78a9",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_indirect_calls_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1530,
            "sides": [
              {
                "calibrated_total_ns": 28855743.39,
                "ns_per_operation": 18859.963,
                "observed_batch": 1530,
                "requested_batch": 1530,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 28999078.380000003,
                "ns_per_operation": 18953.646,
                "observed_batch": 1530,
                "requested_batch": 1530,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 28855743.39,
            "ns_per_operation": 18859.963,
            "observed_batch": 1530,
            "requested_batch": 1530,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 28999078.380000003,
            "ns_per_operation": 18953.646,
            "observed_batch": 1530,
            "requested_batch": 1530,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 4140179.9660000005,
            "ns_per_operation": 18733.846,
            "observed_batch": 221,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 28007213.04,
            "ns_per_operation": 18305.368,
            "observed_batch": 1530,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1530,
        "source_sha256": "29024ae23844664a9fea4a809d1ce414de40f243f7e678ce5ea6d811843f7797",
        "wasm_sha256": "4b42e82f54ea6ab7d0f62c70766c904e41d6a408e57fa14b5abbf514c0da78a9",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_indirect_calls_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 858,
            "sides": [
              {
                "calibrated_total_ns": 27397866.21,
                "ns_per_operation": 31932.245,
                "observed_batch": 858,
                "requested_batch": 858,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 35865850.02,
                "ns_per_operation": 41801.69,
                "observed_batch": 858,
                "requested_batch": 858,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 27397866.21,
            "ns_per_operation": 31932.245,
            "observed_batch": 858,
            "requested_batch": 858,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 35865850.02,
            "ns_per_operation": 41801.69,
            "observed_batch": 858,
            "requested_batch": 858,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 1776629.0129999998,
            "ns_per_operation": 34835.863,
            "observed_batch": 51,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 26321715.42,
            "ns_per_operation": 30677.99,
            "observed_batch": 858,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 858,
        "source_sha256": "29024ae23844664a9fea4a809d1ce414de40f243f7e678ce5ea6d811843f7797",
        "wasm_sha256": "4b42e82f54ea6ab7d0f62c70766c904e41d6a408e57fa14b5abbf514c0da78a9",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_multivalue_branch_n0": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 26774,
            "sides": [
              {
                "calibrated_total_ns": 18717756.948,
                "ns_per_operation": 699.102,
                "observed_batch": 26774,
                "requested_batch": 26774,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 17671455.802,
                "ns_per_operation": 660.023,
                "observed_batch": 26774,
                "requested_batch": 26774,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 33333,
            "sides": [
              {
                "calibrated_total_ns": 31879014.54,
                "ns_per_operation": 956.38,
                "observed_batch": 33333,
                "requested_batch": 33333,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 30932790.669000003,
                "ns_per_operation": 927.993,
                "observed_batch": 33333,
                "requested_batch": 33333,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 31879014.54,
            "ns_per_operation": 956.38,
            "observed_batch": 33333,
            "requested_batch": 33333,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 30932790.669000003,
            "ns_per_operation": 927.993,
            "observed_batch": 33333,
            "requested_batch": 33333,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 9271539.914,
            "ns_per_operation": 737.534,
            "observed_batch": 12571,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 20204597.49,
            "ns_per_operation": 754.635,
            "observed_batch": 26774,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 33333,
        "source_sha256": "a1f4958100af82889208bb28090d6af7c3e56cbeeccabad4106601c9a9c17014",
        "wasm_sha256": "03ffdc2041ae8ba0c8d078422d36152cf1d35f1293c94a4074fe7e83791743ae",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_multivalue_branch_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 11717,
            "sides": [
              {
                "calibrated_total_ns": 22149336.403,
                "ns_per_operation": 1890.359,
                "observed_batch": 11717,
                "requested_batch": 11717,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 24905432.577000003,
                "ns_per_operation": 2125.581,
                "observed_batch": 11717,
                "requested_batch": 11717,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 22149336.403,
            "ns_per_operation": 1890.359,
            "observed_batch": 11717,
            "requested_batch": 11717,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 24905432.577000003,
            "ns_per_operation": 2125.581,
            "observed_batch": 11717,
            "requested_batch": 11717,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 23577955.062,
            "ns_per_operation": 2012.286,
            "observed_batch": 11717,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22568819.21,
            "ns_per_operation": 1930.609,
            "observed_batch": 11690,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 11717,
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
                "calibrated_total_ns": 34395988.0,
                "ns_per_operation": 17197994.0,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 30253650.0,
                "ns_per_operation": 15126825.0,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 34395988.0,
            "ns_per_operation": 17197994.0,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 30253650.0,
            "ns_per_operation": 15126825.0,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 29838743.0,
            "ns_per_operation": 14919371.5,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27523730.0,
            "ns_per_operation": 13761865.0,
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
            "batch": 13,
            "sides": [
              {
                "calibrated_total_ns": 25476301.994,
                "ns_per_operation": 1959715.538,
                "observed_batch": 13,
                "requested_batch": 13,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 36845005.002000004,
                "ns_per_operation": 2834231.154,
                "observed_batch": 13,
                "requested_batch": 13,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 25476301.994,
            "ns_per_operation": 1959715.538,
            "observed_batch": 13,
            "requested_batch": 13,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 36845005.002000004,
            "ns_per_operation": 2834231.154,
            "observed_batch": 13,
            "requested_batch": 13,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 22668543.001000002,
            "ns_per_operation": 1743734.077,
            "observed_batch": 13,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 16822390.0,
            "ns_per_operation": 1682239.0,
            "observed_batch": 10,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 13,
        "source_sha256": "a1f4958100af82889208bb28090d6af7c3e56cbeeccabad4106601c9a9c17014",
        "wasm_sha256": "03ffdc2041ae8ba0c8d078422d36152cf1d35f1293c94a4074fe7e83791743ae",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_multivalue_branch_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1229,
            "sides": [
              {
                "calibrated_total_ns": 32145877.625,
                "ns_per_operation": 26156.125,
                "observed_batch": 1229,
                "requested_batch": 1229,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 31264773.113,
                "ns_per_operation": 25439.197,
                "observed_batch": 1229,
                "requested_batch": 1229,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 32145877.625,
            "ns_per_operation": 26156.125,
            "observed_batch": 1229,
            "requested_batch": 1229,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 31264773.113,
            "ns_per_operation": 25439.197,
            "observed_batch": 1229,
            "requested_batch": 1229,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 44516491.503,
            "ns_per_operation": 36459.043,
            "observed_batch": 1221,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 32194677.527999997,
            "ns_per_operation": 26195.832,
            "observed_batch": 1229,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1229,
        "source_sha256": "a1f4958100af82889208bb28090d6af7c3e56cbeeccabad4106601c9a9c17014",
        "wasm_sha256": "03ffdc2041ae8ba0c8d078422d36152cf1d35f1293c94a4074fe7e83791743ae",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_multivalue_branch_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 668,
            "sides": [
              {
                "calibrated_total_ns": 30121210.176,
                "ns_per_operation": 45091.632,
                "observed_batch": 668,
                "requested_batch": 668,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 33654338.328,
                "ns_per_operation": 50380.746,
                "observed_batch": 668,
                "requested_batch": 668,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 30121210.176,
            "ns_per_operation": 45091.632,
            "observed_batch": 668,
            "requested_batch": 668,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 33654338.328,
            "ns_per_operation": 50380.746,
            "observed_batch": 668,
            "requested_batch": 668,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 30417449.860000003,
            "ns_per_operation": 47232.065,
            "observed_batch": 644,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 32128592.928000003,
            "ns_per_operation": 48096.696,
            "observed_batch": 668,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 668,
        "source_sha256": "a1f4958100af82889208bb28090d6af7c3e56cbeeccabad4106601c9a9c17014",
        "wasm_sha256": "03ffdc2041ae8ba0c8d078422d36152cf1d35f1293c94a4074fe7e83791743ae",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_ordinary_recursion_n0": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 8792,
            "sides": [
              {
                "calibrated_total_ns": 8479207.016,
                "ns_per_operation": 964.423,
                "observed_batch": 8792,
                "requested_batch": 8792,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 11332070.343999999,
                "ns_per_operation": 1288.907,
                "observed_batch": 8792,
                "requested_batch": 8792,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 22812,
            "sides": [
              {
                "calibrated_total_ns": 25724248.356,
                "ns_per_operation": 1127.663,
                "observed_batch": 22812,
                "requested_batch": 22812,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 22038536.328,
                "ns_per_operation": 966.094,
                "observed_batch": 22812,
                "requested_batch": 22812,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 25724248.356,
            "ns_per_operation": 1127.663,
            "observed_batch": 22812,
            "requested_batch": 22812,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22038536.328,
            "ns_per_operation": 966.094,
            "observed_batch": 22812,
            "requested_batch": 22812,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 9220058.722000001,
            "ns_per_operation": 1062.586,
            "observed_batch": 8677,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 9739733.64,
            "ns_per_operation": 1107.795,
            "observed_batch": 8792,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 22812,
        "source_sha256": "aca6a66637fa36b72f3c528dd5e3e44039b573f363b5b4fb77437e9278623903",
        "wasm_sha256": "e1bd7ea6ada8b15b2770ac387f8fc2256f20d7f87c4bc34e7559b239d8875ba1",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_ordinary_recursion_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 14772,
            "sides": [
              {
                "calibrated_total_ns": 29471897.867999997,
                "ns_per_operation": 1995.119,
                "observed_batch": 14772,
                "requested_batch": 14772,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 26778179.352,
                "ns_per_operation": 1812.766,
                "observed_batch": 14772,
                "requested_batch": 14772,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 29471897.867999997,
            "ns_per_operation": 1995.119,
            "observed_batch": 14772,
            "requested_batch": 14772,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 26778179.352,
            "ns_per_operation": 1812.766,
            "observed_batch": 14772,
            "requested_batch": 14772,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 23849556.492,
            "ns_per_operation": 1614.511,
            "observed_batch": 14772,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 21161753.655,
            "ns_per_operation": 1554.297,
            "observed_batch": 13615,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 14772,
        "source_sha256": "aca6a66637fa36b72f3c528dd5e3e44039b573f363b5b4fb77437e9278623903",
        "wasm_sha256": "e1bd7ea6ada8b15b2770ac387f8fc2256f20d7f87c4bc34e7559b239d8875ba1",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_ordinary_recursion_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1734,
            "sides": [
              {
                "calibrated_total_ns": 80792527.302,
                "ns_per_operation": 46593.153,
                "observed_batch": 1734,
                "requested_batch": 1734,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 40953402.186000004,
                "ns_per_operation": 23617.879,
                "observed_batch": 1734,
                "requested_batch": 1734,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 80792527.302,
            "ns_per_operation": 46593.153,
            "observed_batch": 1734,
            "requested_batch": 1734,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 40953402.186000004,
            "ns_per_operation": 23617.879,
            "observed_batch": 1734,
            "requested_batch": 1734,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 32553601.002000004,
            "ns_per_operation": 18773.703,
            "observed_batch": 1734,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 28464227.786000002,
            "ns_per_operation": 16963.187,
            "observed_batch": 1678,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1734,
        "source_sha256": "aca6a66637fa36b72f3c528dd5e3e44039b573f363b5b4fb77437e9278623903",
        "wasm_sha256": "e1bd7ea6ada8b15b2770ac387f8fc2256f20d7f87c4bc34e7559b239d8875ba1",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_ordinary_recursion_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1020,
            "sides": [
              {
                "calibrated_total_ns": 41030865.78,
                "ns_per_operation": 40226.339,
                "observed_batch": 1020,
                "requested_batch": 1020,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 34469979.96,
                "ns_per_operation": 33794.098,
                "observed_batch": 1020,
                "requested_batch": 1020,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 41030865.78,
            "ns_per_operation": 40226.339,
            "observed_batch": 1020,
            "requested_batch": 1020,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 34469979.96,
            "ns_per_operation": 33794.098,
            "observed_batch": 1020,
            "requested_batch": 1020,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 28750711.236,
            "ns_per_operation": 32523.429,
            "observed_batch": 884,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 36861981.95999999,
            "ns_per_operation": 36139.198,
            "observed_batch": 1020,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1020,
        "source_sha256": "aca6a66637fa36b72f3c528dd5e3e44039b573f363b5b4fb77437e9278623903",
        "wasm_sha256": "e1bd7ea6ada8b15b2770ac387f8fc2256f20d7f87c4bc34e7559b239d8875ba1",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_table_mutation_n0": {
        "attempt_count": 3,
        "attempts": [
          {
            "batch": 24243,
            "sides": [
              {
                "calibrated_total_ns": 19970486.409,
                "ns_per_operation": 823.763,
                "observed_batch": 24243,
                "requested_batch": 24243,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 19355102.097,
                "ns_per_operation": 798.379,
                "observed_batch": 24243,
                "requested_batch": 24243,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 27556,
            "sides": [
              {
                "calibrated_total_ns": 19793805.472,
                "ns_per_operation": 718.312,
                "observed_batch": 27556,
                "requested_batch": 27556,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 19409481.94,
                "ns_per_operation": 704.365,
                "observed_batch": 27556,
                "requested_batch": 27556,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 31234,
            "sides": [
              {
                "calibrated_total_ns": 22337682.248,
                "ns_per_operation": 715.172,
                "observed_batch": 31234,
                "requested_batch": 31234,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 28066278.954,
                "ns_per_operation": 898.581,
                "observed_batch": 31234,
                "requested_batch": 31234,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 22337682.248,
            "ns_per_operation": 715.172,
            "observed_batch": 31234,
            "requested_batch": 31234,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 28066278.954,
            "ns_per_operation": 898.581,
            "observed_batch": 31234,
            "requested_batch": 31234,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 18356120.796,
            "ns_per_operation": 757.172,
            "observed_batch": 24243,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 16696373.078000002,
            "ns_per_operation": 722.099,
            "observed_batch": 23122,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 31234,
        "source_sha256": "9b61bd9cbca804bf639085769fd766a2cf8a69d516fc7efb9e4edfe4ff6d18c9",
        "wasm_sha256": "458f2b111e48d2efca98083b8c56860c43e82be8752813d1e2814d23c28d4f43",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_table_mutation_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 14483,
            "sides": [
              {
                "calibrated_total_ns": 27783202.356000002,
                "ns_per_operation": 1918.332,
                "observed_batch": 14483,
                "requested_batch": 14483,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 27122661.691999998,
                "ns_per_operation": 1872.724,
                "observed_batch": 14483,
                "requested_batch": 14483,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 27783202.356000002,
            "ns_per_operation": 1918.332,
            "observed_batch": 14483,
            "requested_batch": 14483,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27122661.691999998,
            "ns_per_operation": 1872.724,
            "observed_batch": 14483,
            "requested_batch": 14483,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 24465118.09,
            "ns_per_operation": 1689.23,
            "observed_batch": 14483,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 20279101.914,
            "ns_per_operation": 1919.279,
            "observed_batch": 10566,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 14483,
        "source_sha256": "9b61bd9cbca804bf639085769fd766a2cf8a69d516fc7efb9e4edfe4ff6d18c9",
        "wasm_sha256": "458f2b111e48d2efca98083b8c56860c43e82be8752813d1e2814d23c28d4f43",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_table_mutation_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 3,
            "sides": [
              {
                "calibrated_total_ns": 26697752.001,
                "ns_per_operation": 8899250.667,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 28296221.001,
                "ns_per_operation": 9432073.667,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 26697752.001,
            "ns_per_operation": 8899250.667,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 28296221.001,
            "ns_per_operation": 9432073.667,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 27266816.001,
            "ns_per_operation": 9088938.667,
            "observed_batch": 3,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 28280439.0,
            "ns_per_operation": 9426813.0,
            "observed_batch": 3,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 3,
        "source_sha256": "9b61bd9cbca804bf639085769fd766a2cf8a69d516fc7efb9e4edfe4ff6d18c9",
        "wasm_sha256": "458f2b111e48d2efca98083b8c56860c43e82be8752813d1e2814d23c28d4f43",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_table_mutation_n1024": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 15,
            "sides": [
              {
                "calibrated_total_ns": 17348430.0,
                "ns_per_operation": 1156562.0,
                "observed_batch": 15,
                "requested_batch": 15,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 15656520.0,
                "ns_per_operation": 1043768.0,
                "observed_batch": 15,
                "requested_batch": 15,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 22,
            "sides": [
              {
                "calibrated_total_ns": 22540143.009999998,
                "ns_per_operation": 1024551.955,
                "observed_batch": 22,
                "requested_batch": 22,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 25567517.008,
                "ns_per_operation": 1162159.864,
                "observed_batch": 22,
                "requested_batch": 22,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 22540143.009999998,
            "ns_per_operation": 1024551.955,
            "observed_batch": 22,
            "requested_batch": 22,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 25567517.008,
            "ns_per_operation": 1162159.864,
            "observed_batch": 22,
            "requested_batch": 22,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 15427905.0,
            "ns_per_operation": 1028527.0,
            "observed_batch": 15,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 14504737.002,
            "ns_per_operation": 1036052.643,
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
      "0:scalar_table_mutation_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1685,
            "sides": [
              {
                "calibrated_total_ns": 41868372.815,
                "ns_per_operation": 24847.699,
                "observed_batch": 1685,
                "requested_batch": 1685,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 68276553.85,
                "ns_per_operation": 40520.21,
                "observed_batch": 1685,
                "requested_batch": 1685,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 41868372.815,
            "ns_per_operation": 24847.699,
            "observed_batch": 1685,
            "requested_batch": 1685,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 68276553.85,
            "ns_per_operation": 40520.21,
            "observed_batch": 1685,
            "requested_batch": 1685,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 34222763.717999995,
            "ns_per_operation": 20407.134,
            "observed_batch": 1677,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 71378404.635,
            "ns_per_operation": 42361.071,
            "observed_batch": 1685,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1685,
        "source_sha256": "9b61bd9cbca804bf639085769fd766a2cf8a69d516fc7efb9e4edfe4ff6d18c9",
        "wasm_sha256": "458f2b111e48d2efca98083b8c56860c43e82be8752813d1e2814d23c28d4f43",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_table_mutation_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 959,
            "sides": [
              {
                "calibrated_total_ns": 28756815.668,
                "ns_per_operation": 29986.252,
                "observed_batch": 959,
                "requested_batch": 959,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 30632417.319000002,
                "ns_per_operation": 31942.041,
                "observed_batch": 959,
                "requested_batch": 959,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 28756815.668,
            "ns_per_operation": 29986.252,
            "observed_batch": 959,
            "requested_batch": 959,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 30632417.319000002,
            "ns_per_operation": 31942.041,
            "observed_batch": 959,
            "requested_batch": 959,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 30708217.638,
            "ns_per_operation": 32021.082,
            "observed_batch": 959,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 6452098.0200000005,
            "ns_per_operation": 30291.54,
            "observed_batch": 213,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 959,
        "source_sha256": "9b61bd9cbca804bf639085769fd766a2cf8a69d516fc7efb9e4edfe4ff6d18c9",
        "wasm_sha256": "458f2b111e48d2efca98083b8c56860c43e82be8752813d1e2814d23c28d4f43",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_tail_calls_n0": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 9667,
            "sides": [
              {
                "calibrated_total_ns": 8868331.794,
                "ns_per_operation": 917.382,
                "observed_batch": 9667,
                "requested_batch": 9667,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 8849258.803,
                "ns_per_operation": 915.409,
                "observed_batch": 9667,
                "requested_batch": 9667,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 24033,
            "sides": [
              {
                "calibrated_total_ns": 27802503.951,
                "ns_per_operation": 1156.847,
                "observed_batch": 24033,
                "requested_batch": 24033,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 26149562.277,
                "ns_per_operation": 1088.069,
                "observed_batch": 24033,
                "requested_batch": 24033,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 27802503.951,
            "ns_per_operation": 1156.847,
            "observed_batch": 24033,
            "requested_batch": 24033,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 26149562.277,
            "ns_per_operation": 1088.069,
            "observed_batch": 24033,
            "requested_batch": 24033,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 8306634.384000001,
            "ns_per_operation": 1100.508,
            "observed_batch": 7548,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 12405206.751,
            "ns_per_operation": 1283.253,
            "observed_batch": 9667,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 24033,
        "source_sha256": "9953f19384c51b579921e6f789b13a8b57c86ba86d760b77846e3e33825ece02",
        "wasm_sha256": "3e23be294e7a9eda62c89dbc366b6a6cab90edfbda532da27a7e4fc6d0aa4a56",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_tail_calls_n1": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 7318,
            "sides": [
              {
                "calibrated_total_ns": 11072675.532,
                "ns_per_operation": 1513.074,
                "observed_batch": 7318,
                "requested_batch": 7318,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 9949333.26,
                "ns_per_operation": 1359.57,
                "observed_batch": 7318,
                "requested_batch": 7318,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 16182,
            "sides": [
              {
                "calibrated_total_ns": 24766292.088,
                "ns_per_operation": 1530.484,
                "observed_batch": 16182,
                "requested_batch": 16182,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 22512624.948,
                "ns_per_operation": 1391.214,
                "observed_batch": 16182,
                "requested_batch": 16182,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 24766292.088,
            "ns_per_operation": 1530.484,
            "observed_batch": 16182,
            "requested_batch": 16182,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22512624.948,
            "ns_per_operation": 1391.214,
            "observed_batch": 16182,
            "requested_batch": 16182,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 13887932.086000001,
            "ns_per_operation": 1897.777,
            "observed_batch": 7318,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 10400376.014,
            "ns_per_operation": 1425.881,
            "observed_batch": 7294,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 16182,
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
                "calibrated_total_ns": 25535937.0,
                "ns_per_operation": 6383984.25,
                "observed_batch": 4,
                "requested_batch": 4,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 35964137.0,
                "ns_per_operation": 8991034.25,
                "observed_batch": 4,
                "requested_batch": 4,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 25535937.0,
            "ns_per_operation": 6383984.25,
            "observed_batch": 4,
            "requested_batch": 4,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 35964137.0,
            "ns_per_operation": 8991034.25,
            "observed_batch": 4,
            "requested_batch": 4,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 31296333.0,
            "ns_per_operation": 10432111.0,
            "observed_batch": 3,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 25661127.0,
            "ns_per_operation": 6415281.75,
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
            "batch": 43,
            "sides": [
              {
                "calibrated_total_ns": 31139615.988,
                "ns_per_operation": 724177.116,
                "observed_batch": 43,
                "requested_batch": 43,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 27283558.996,
                "ns_per_operation": 634501.372,
                "observed_batch": 43,
                "requested_batch": 43,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 31139615.988,
            "ns_per_operation": 724177.116,
            "observed_batch": 43,
            "requested_batch": 43,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27283558.996,
            "ns_per_operation": 634501.372,
            "observed_batch": 43,
            "requested_batch": 43,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 29369475.021,
            "ns_per_operation": 683011.047,
            "observed_batch": 43,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 26627508.985,
            "ns_per_operation": 719662.405,
            "observed_batch": 37,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 43,
        "source_sha256": "9953f19384c51b579921e6f789b13a8b57c86ba86d760b77846e3e33825ece02",
        "wasm_sha256": "3e23be294e7a9eda62c89dbc366b6a6cab90edfbda532da27a7e4fc6d0aa4a56",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_tail_calls_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1997,
            "sides": [
              {
                "calibrated_total_ns": 29991840.656,
                "ns_per_operation": 15018.448,
                "observed_batch": 1997,
                "requested_batch": 1997,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 33088021.408000004,
                "ns_per_operation": 16568.864,
                "observed_batch": 1997,
                "requested_batch": 1997,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 29991840.656,
            "ns_per_operation": 15018.448,
            "observed_batch": 1997,
            "requested_batch": 1997,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 33088021.408000004,
            "ns_per_operation": 16568.864,
            "observed_batch": 1997,
            "requested_batch": 1997,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 29256916.985,
            "ns_per_operation": 14739.001,
            "observed_batch": 1985,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 25476068.49,
            "ns_per_operation": 12757.17,
            "observed_batch": 1997,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1997,
        "source_sha256": "9953f19384c51b579921e6f789b13a8b57c86ba86d760b77846e3e33825ece02",
        "wasm_sha256": "3e23be294e7a9eda62c89dbc366b6a6cab90edfbda532da27a7e4fc6d0aa4a56",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_tail_calls_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1213,
            "sides": [
              {
                "calibrated_total_ns": 26963638.718,
                "ns_per_operation": 22228.886,
                "observed_batch": 1213,
                "requested_batch": 1213,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 26368584.586,
                "ns_per_operation": 21738.322,
                "observed_batch": 1213,
                "requested_batch": 1213,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 26963638.718,
            "ns_per_operation": 22228.886,
            "observed_batch": 1213,
            "requested_batch": 1213,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 26368584.586,
            "ns_per_operation": 21738.322,
            "observed_batch": 1213,
            "requested_batch": 1213,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 26313155.338,
            "ns_per_operation": 21692.626,
            "observed_batch": 1213,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 24101211.96,
            "ns_per_operation": 20134.68,
            "observed_batch": 1197,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1213,
        "source_sha256": "9953f19384c51b579921e6f789b13a8b57c86ba86d760b77846e3e33825ece02",
        "wasm_sha256": "3e23be294e7a9eda62c89dbc366b6a6cab90edfbda532da27a7e4fc6d0aa4a56",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_try_table_normal_n0": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 25907,
            "sides": [
              {
                "calibrated_total_ns": 15827544.859000001,
                "ns_per_operation": 610.937,
                "observed_batch": 25907,
                "requested_batch": 25907,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 16198921.704000002,
                "ns_per_operation": 625.272,
                "observed_batch": 25907,
                "requested_batch": 25907,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 36011,
            "sides": [
              {
                "calibrated_total_ns": 24312142.441,
                "ns_per_operation": 675.131,
                "observed_batch": 36011,
                "requested_batch": 36011,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 27566492.522,
                "ns_per_operation": 765.502,
                "observed_batch": 36011,
                "requested_batch": 36011,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 24312142.441,
            "ns_per_operation": 675.131,
            "observed_batch": 36011,
            "requested_batch": 36011,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27566492.522,
            "ns_per_operation": 765.502,
            "observed_batch": 36011,
            "requested_batch": 36011,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 16993800.277999997,
            "ns_per_operation": 655.954,
            "observed_batch": 25907,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 16001947.68,
            "ns_per_operation": 702.456,
            "observed_batch": 22780,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 36011,
        "source_sha256": "ab92800dd4e4d4053498c317effadedf52568270ab7b476eef74b09638de8964",
        "wasm_sha256": "97068a6310a3bc535369e2b6277b801875c14ca206a707fcb9bca9036f41d0df",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_try_table_normal_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 15095,
            "sides": [
              {
                "calibrated_total_ns": 21825211.415,
                "ns_per_operation": 1445.857,
                "observed_batch": 15095,
                "requested_batch": 15095,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 21547569.08,
                "ns_per_operation": 1427.464,
                "observed_batch": 15095,
                "requested_batch": 15095,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 21825211.415,
            "ns_per_operation": 1445.857,
            "observed_batch": 15095,
            "requested_batch": 15095,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 21547569.08,
            "ns_per_operation": 1427.464,
            "observed_batch": 15095,
            "requested_batch": 15095,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 19509023.573,
            "ns_per_operation": 1351.883,
            "observed_batch": 14431,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27283231.325,
            "ns_per_operation": 1807.435,
            "observed_batch": 15095,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 15095,
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
                "calibrated_total_ns": 21918026.001000002,
                "ns_per_operation": 7306008.667,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 20809250.001000002,
                "ns_per_operation": 6936416.667,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 21918026.001000002,
            "ns_per_operation": 7306008.667,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 20809250.001000002,
            "ns_per_operation": 6936416.667,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 21293394.998999998,
            "ns_per_operation": 7097798.333,
            "observed_batch": 3,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 19611204.998999998,
            "ns_per_operation": 6537068.333,
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
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 24,
            "sides": [
              {
                "calibrated_total_ns": 17852205.0,
                "ns_per_operation": 743841.875,
                "observed_batch": 24,
                "requested_batch": 24,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 20555631.0,
                "ns_per_operation": 856484.625,
                "observed_batch": 24,
                "requested_batch": 24,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 30,
            "sides": [
              {
                "calibrated_total_ns": 31129350.0,
                "ns_per_operation": 1037645.0,
                "observed_batch": 30,
                "requested_batch": 30,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 24346728.0,
                "ns_per_operation": 811557.6,
                "observed_batch": 30,
                "requested_batch": 30,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 31129350.0,
            "ns_per_operation": 1037645.0,
            "observed_batch": 30,
            "requested_batch": 30,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 24346728.0,
            "ns_per_operation": 811557.6,
            "observed_batch": 30,
            "requested_batch": 30,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 18925732.008,
            "ns_per_operation": 788572.167,
            "observed_batch": 24,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 16717459.991999999,
            "ns_per_operation": 696560.833,
            "observed_batch": 24,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 30,
        "source_sha256": "ab92800dd4e4d4053498c317effadedf52568270ab7b476eef74b09638de8964",
        "wasm_sha256": "97068a6310a3bc535369e2b6277b801875c14ca206a707fcb9bca9036f41d0df",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_try_table_normal_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 2072,
            "sides": [
              {
                "calibrated_total_ns": 32626625.752,
                "ns_per_operation": 15746.441,
                "observed_batch": 2072,
                "requested_batch": 2072,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 34794870.152,
                "ns_per_operation": 16792.891,
                "observed_batch": 2072,
                "requested_batch": 2072,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 32626625.752,
            "ns_per_operation": 15746.441,
            "observed_batch": 2072,
            "requested_batch": 2072,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 34794870.152,
            "ns_per_operation": 16792.891,
            "observed_batch": 2072,
            "requested_batch": 2072,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 29133770.400000002,
            "ns_per_operation": 14060.7,
            "observed_batch": 2072,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27182192.036999997,
            "ns_per_operation": 14224.067,
            "observed_batch": 1911,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 2072,
        "source_sha256": "ab92800dd4e4d4053498c317effadedf52568270ab7b476eef74b09638de8964",
        "wasm_sha256": "97068a6310a3bc535369e2b6277b801875c14ca206a707fcb9bca9036f41d0df",
        "workload_kind": "typed_scalar_driver"
      },
      "0:scalar_try_table_normal_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1144,
            "sides": [
              {
                "calibrated_total_ns": 36807229.888,
                "ns_per_operation": 32174.152,
                "observed_batch": 1144,
                "requested_batch": 1144,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 32865859.312000003,
                "ns_per_operation": 28728.898,
                "observed_batch": 1144,
                "requested_batch": 1144,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 36807229.888,
            "ns_per_operation": 32174.152,
            "observed_batch": 1144,
            "requested_batch": 1144,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 32865859.312000003,
            "ns_per_operation": 28728.898,
            "observed_batch": 1144,
            "requested_batch": 1144,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 51402277.784,
            "ns_per_operation": 44932.061,
            "observed_batch": 1144,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 29604170.224,
            "ns_per_operation": 26060.009,
            "observed_batch": 1136,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1144,
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
                "calibrated_total_ns": 21041022.998999998,
                "ns_per_operation": 7013674.333,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 19356404.001000002,
                "ns_per_operation": 6452134.667,
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
                "calibrated_total_ns": 27182091.0,
                "ns_per_operation": 6795522.75,
                "observed_batch": 4,
                "requested_batch": 4,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 26108479.0,
                "ns_per_operation": 6527119.75,
                "observed_batch": 4,
                "requested_batch": 4,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 27182091.0,
            "ns_per_operation": 6795522.75,
            "observed_batch": 4,
            "requested_batch": 4,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 26108479.0,
            "ns_per_operation": 6527119.75,
            "observed_batch": 4,
            "requested_batch": 4,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 18683568.998999998,
            "ns_per_operation": 6227856.333,
            "observed_batch": 3,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 21011267.001000002,
            "ns_per_operation": 7003755.667,
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
        "attempt_count": 3,
        "attempts": [
          {
            "batch": 54348,
            "sides": [
              {
                "calibrated_total_ns": 19750932.768,
                "ns_per_operation": 363.416,
                "observed_batch": 54348,
                "requested_batch": 54348,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 42461548.919999994,
                "ns_per_operation": 781.29,
                "observed_batch": 54348,
                "requested_batch": 54348,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 60537,
            "sides": [
              {
                "calibrated_total_ns": 22682003.16,
                "ns_per_operation": 374.68,
                "observed_batch": 60537,
                "requested_batch": 60537,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 19497514.812000003,
                "ns_per_operation": 322.076,
                "observed_batch": 60537,
                "requested_batch": 60537,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 68307,
            "sides": [
              {
                "calibrated_total_ns": 23418235.266000003,
                "ns_per_operation": 342.838,
                "observed_batch": 68307,
                "requested_batch": 68307,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 27385847.361,
                "ns_per_operation": 400.923,
                "observed_batch": 68307,
                "requested_batch": 68307,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 23418235.266000003,
            "ns_per_operation": 342.838,
            "observed_batch": 68307,
            "requested_batch": 68307,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27385847.361,
            "ns_per_operation": 400.923,
            "observed_batch": 68307,
            "requested_batch": 68307,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 19627128.024,
            "ns_per_operation": 361.138,
            "observed_batch": 54348,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 11008934.272,
            "ns_per_operation": 309.344,
            "observed_batch": 35588,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 68307,
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
                "calibrated_total_ns": 20151786.998999998,
                "ns_per_operation": 6717262.333,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 39277818.999,
                "ns_per_operation": 13092606.333,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 20151786.998999998,
            "ns_per_operation": 6717262.333,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 39277818.999,
            "ns_per_operation": 13092606.333,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 21569796.998999998,
            "ns_per_operation": 7189932.333,
            "observed_batch": 3,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 20420946.0,
            "ns_per_operation": 6806982.0,
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
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 9104,
            "sides": [
              {
                "calibrated_total_ns": 6206196.800000001,
                "ns_per_operation": 681.7,
                "observed_batch": 9104,
                "requested_batch": 9104,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 7385519.856000001,
                "ns_per_operation": 811.239,
                "observed_batch": 9104,
                "requested_batch": 9104,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 32273,
            "sides": [
              {
                "calibrated_total_ns": 26010004.801,
                "ns_per_operation": 805.937,
                "observed_batch": 32273,
                "requested_batch": 32273,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 24573824.028,
                "ns_per_operation": 761.436,
                "observed_batch": 32273,
                "requested_batch": 32273,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 26010004.801,
            "ns_per_operation": 805.937,
            "observed_batch": 32273,
            "requested_batch": 32273,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 24573824.028,
            "ns_per_operation": 761.436,
            "observed_batch": 32273,
            "requested_batch": 32273,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 7599000.5,
            "ns_per_operation": 835.055,
            "observed_batch": 9100,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 6488712.128,
            "ns_per_operation": 712.732,
            "observed_batch": 9104,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 32273,
        "source_sha256": "61aaec7a86d481378f31f3e27b0c1413684065841fa5909297181fcad9bef10d",
        "wasm_sha256": "ef61bf7afc44af7b71ef78eadcba48c0d85949d7d9096df8fbbbab53c05036b1",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_accumulator_n1": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 8085,
            "sides": [
              {
                "calibrated_total_ns": 9447540.795,
                "ns_per_operation": 1168.527,
                "observed_batch": 8085,
                "requested_batch": 8085,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 10148574.975,
                "ns_per_operation": 1255.235,
                "observed_batch": 8085,
                "requested_batch": 8085,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 18828,
            "sides": [
              {
                "calibrated_total_ns": 22560556.86,
                "ns_per_operation": 1198.245,
                "observed_batch": 18828,
                "requested_batch": 18828,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 20193707.808000002,
                "ns_per_operation": 1072.536,
                "observed_batch": 18828,
                "requested_batch": 18828,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 22560556.86,
            "ns_per_operation": 1198.245,
            "observed_batch": 18828,
            "requested_batch": 18828,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 20193707.808000002,
            "ns_per_operation": 1072.536,
            "observed_batch": 18828,
            "requested_batch": 18828,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 12935964.535,
            "ns_per_operation": 2300.545,
            "observed_batch": 5623,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 9576520.8,
            "ns_per_operation": 1184.48,
            "observed_batch": 8085,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 18828,
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
                "calibrated_total_ns": 33493332.0,
                "ns_per_operation": 8373333.0,
                "observed_batch": 4,
                "requested_batch": 4,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 28356935.0,
                "ns_per_operation": 7089233.75,
                "observed_batch": 4,
                "requested_batch": 4,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 33493332.0,
            "ns_per_operation": 8373333.0,
            "observed_batch": 4,
            "requested_batch": 4,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 28356935.0,
            "ns_per_operation": 7089233.75,
            "observed_batch": 4,
            "requested_batch": 4,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 24669827.0,
            "ns_per_operation": 6167456.75,
            "observed_batch": 4,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 19402905.0,
            "ns_per_operation": 6467635.0,
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
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 37,
            "sides": [
              {
                "calibrated_total_ns": 38782366.997,
                "ns_per_operation": 1048172.081,
                "observed_batch": 37,
                "requested_batch": 37,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 33715392.007,
                "ns_per_operation": 911226.811,
                "observed_batch": 37,
                "requested_batch": 37,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 38782366.997,
            "ns_per_operation": 1048172.081,
            "observed_batch": 37,
            "requested_batch": 37,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 33715392.007,
            "ns_per_operation": 911226.811,
            "observed_batch": 37,
            "requested_batch": 37,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 26321025.997,
            "ns_per_operation": 711379.081,
            "observed_batch": 37,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 25547964.012000002,
            "ns_per_operation": 709665.667,
            "observed_batch": 36,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 37,
        "source_sha256": "61aaec7a86d481378f31f3e27b0c1413684065841fa5909297181fcad9bef10d",
        "wasm_sha256": "ef61bf7afc44af7b71ef78eadcba48c0d85949d7d9096df8fbbbab53c05036b1",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_accumulator_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 2227,
            "sides": [
              {
                "calibrated_total_ns": 33057095.833,
                "ns_per_operation": 14843.779,
                "observed_batch": 2227,
                "requested_batch": 2227,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 34983486.465,
                "ns_per_operation": 15708.795,
                "observed_batch": 2227,
                "requested_batch": 2227,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 33057095.833,
            "ns_per_operation": 14843.779,
            "observed_batch": 2227,
            "requested_batch": 2227,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 34983486.465,
            "ns_per_operation": 15708.795,
            "observed_batch": 2227,
            "requested_batch": 2227,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 28759597.55,
            "ns_per_operation": 18318.215,
            "observed_batch": 1570,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 37510303.021,
            "ns_per_operation": 16843.423,
            "observed_batch": 2227,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 2227,
        "source_sha256": "61aaec7a86d481378f31f3e27b0c1413684065841fa5909297181fcad9bef10d",
        "wasm_sha256": "ef61bf7afc44af7b71ef78eadcba48c0d85949d7d9096df8fbbbab53c05036b1",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_accumulator_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1325,
            "sides": [
              {
                "calibrated_total_ns": 27017812.65,
                "ns_per_operation": 20390.802,
                "observed_batch": 1325,
                "requested_batch": 1325,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 34265879.325,
                "ns_per_operation": 25861.041,
                "observed_batch": 1325,
                "requested_batch": 1325,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 27017812.65,
            "ns_per_operation": 20390.802,
            "observed_batch": 1325,
            "requested_batch": 1325,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 34265879.325,
            "ns_per_operation": 25861.041,
            "observed_batch": 1325,
            "requested_batch": 1325,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 29837380.849999998,
            "ns_per_operation": 22518.778,
            "observed_batch": 1325,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 44066338.428,
            "ns_per_operation": 33845.114,
            "observed_batch": 1302,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1325,
        "source_sha256": "61aaec7a86d481378f31f3e27b0c1413684065841fa5909297181fcad9bef10d",
        "wasm_sha256": "ef61bf7afc44af7b71ef78eadcba48c0d85949d7d9096df8fbbbab53c05036b1",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_br_table_n0": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 26991,
            "sides": [
              {
                "calibrated_total_ns": 20395128.356999997,
                "ns_per_operation": 755.627,
                "observed_batch": 26991,
                "requested_batch": 26991,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 16621246.737,
                "ns_per_operation": 615.807,
                "observed_batch": 26991,
                "requested_batch": 26991,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 35726,
            "sides": [
              {
                "calibrated_total_ns": 26668280.042,
                "ns_per_operation": 746.467,
                "observed_batch": 35726,
                "requested_batch": 35726,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 25391432.801999997,
                "ns_per_operation": 710.727,
                "observed_batch": 35726,
                "requested_batch": 35726,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 26668280.042,
            "ns_per_operation": 746.467,
            "observed_batch": 35726,
            "requested_batch": 35726,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 25391432.801999997,
            "ns_per_operation": 710.727,
            "observed_batch": 35726,
            "requested_batch": 35726,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 19289981.862,
            "ns_per_operation": 714.682,
            "observed_batch": 26991,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 14077887.702,
            "ns_per_operation": 658.122,
            "observed_batch": 21391,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 35726,
        "source_sha256": "637b96cd8926ffadf728af4949b43fa6b62fffb61968c14c716e0c203a1e5e6d",
        "wasm_sha256": "d43d5dd71d5ca1c3dae820aa85c13a2d69415e028b8d48097fed4558f743163a",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_br_table_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 13290,
            "sides": [
              {
                "calibrated_total_ns": 26118066.18,
                "ns_per_operation": 1965.242,
                "observed_batch": 13290,
                "requested_batch": 13290,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 43483763.64,
                "ns_per_operation": 3271.916,
                "observed_batch": 13290,
                "requested_batch": 13290,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 26118066.18,
            "ns_per_operation": 1965.242,
            "observed_batch": 13290,
            "requested_batch": 13290,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 43483763.64,
            "ns_per_operation": 3271.916,
            "observed_batch": 13290,
            "requested_batch": 13290,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 27317130.57,
            "ns_per_operation": 2255.005,
            "observed_batch": 12114,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22014512.88,
            "ns_per_operation": 1656.472,
            "observed_batch": 13290,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 13290,
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
                "calibrated_total_ns": 25899065.0,
                "ns_per_operation": 12949532.5,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 26326415.0,
                "ns_per_operation": 13163207.5,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 25899065.0,
            "ns_per_operation": 12949532.5,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 26326415.0,
            "ns_per_operation": 13163207.5,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 32834055.0,
            "ns_per_operation": 16417027.5,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27369505.0,
            "ns_per_operation": 27369505.0,
            "observed_batch": 1,
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
            "batch": 20,
            "sides": [
              {
                "calibrated_total_ns": 25608892.0,
                "ns_per_operation": 1280444.6,
                "observed_batch": 20,
                "requested_batch": 20,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 24393247.0,
                "ns_per_operation": 1219662.35,
                "observed_batch": 20,
                "requested_batch": 20,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 25608892.0,
            "ns_per_operation": 1280444.6,
            "observed_batch": 20,
            "requested_batch": 20,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 24393247.0,
            "ns_per_operation": 1219662.35,
            "observed_batch": 20,
            "requested_batch": 20,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 20508917.994000003,
            "ns_per_operation": 1139384.333,
            "observed_batch": 18,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 23776827.0,
            "ns_per_operation": 1188841.35,
            "observed_batch": 20,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 20,
        "source_sha256": "637b96cd8926ffadf728af4949b43fa6b62fffb61968c14c716e0c203a1e5e6d",
        "wasm_sha256": "d43d5dd71d5ca1c3dae820aa85c13a2d69415e028b8d48097fed4558f743163a",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_br_table_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1402,
            "sides": [
              {
                "calibrated_total_ns": 39915617.166,
                "ns_per_operation": 28470.483,
                "observed_batch": 1402,
                "requested_batch": 1402,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 34198683.168,
                "ns_per_operation": 24392.784,
                "observed_batch": 1402,
                "requested_batch": 1402,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 39915617.166,
            "ns_per_operation": 28470.483,
            "observed_batch": 1402,
            "requested_batch": 1402,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 34198683.168,
            "ns_per_operation": 24392.784,
            "observed_batch": 1402,
            "requested_batch": 1402,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 25776803.468000002,
            "ns_per_operation": 24023.116,
            "observed_batch": 1073,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 37007292.944,
            "ns_per_operation": 26396.072,
            "observed_batch": 1402,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1402,
        "source_sha256": "637b96cd8926ffadf728af4949b43fa6b62fffb61968c14c716e0c203a1e5e6d",
        "wasm_sha256": "d43d5dd71d5ca1c3dae820aa85c13a2d69415e028b8d48097fed4558f743163a",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_br_table_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 755,
            "sides": [
              {
                "calibrated_total_ns": 39773474.745000005,
                "ns_per_operation": 52680.099,
                "observed_batch": 755,
                "requested_batch": 755,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 39802400.305,
                "ns_per_operation": 52718.411,
                "observed_batch": 755,
                "requested_batch": 755,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 39773474.745000005,
            "ns_per_operation": 52680.099,
            "observed_batch": 755,
            "requested_batch": 755,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 39802400.305,
            "ns_per_operation": 52718.411,
            "observed_batch": 755,
            "requested_batch": 755,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 18989511.156,
            "ns_per_operation": 47953.311,
            "observed_batch": 396,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 34857006.1,
            "ns_per_operation": 46168.22,
            "observed_batch": 755,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 755,
        "source_sha256": "637b96cd8926ffadf728af4949b43fa6b62fffb61968c14c716e0c203a1e5e6d",
        "wasm_sha256": "d43d5dd71d5ca1c3dae820aa85c13a2d69415e028b8d48097fed4558f743163a",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_branch_entropy_n0": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 22651,
            "sides": [
              {
                "calibrated_total_ns": 16538899.462000001,
                "ns_per_operation": 730.162,
                "observed_batch": 22651,
                "requested_batch": 22651,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 20117440.348,
                "ns_per_operation": 888.148,
                "observed_batch": 22651,
                "requested_batch": 22651,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 30131,
            "sides": [
              {
                "calibrated_total_ns": 27057909.179,
                "ns_per_operation": 898.009,
                "observed_batch": 30131,
                "requested_batch": 30131,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 21771304.705,
                "ns_per_operation": 722.555,
                "observed_batch": 30131,
                "requested_batch": 30131,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 27057909.179,
            "ns_per_operation": 898.009,
            "observed_batch": 30131,
            "requested_batch": 30131,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 21771304.705,
            "ns_per_operation": 722.555,
            "observed_batch": 30131,
            "requested_batch": 30131,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 13636533.18,
            "ns_per_operation": 743.865,
            "observed_batch": 18332,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 17963420.852,
            "ns_per_operation": 793.052,
            "observed_batch": 22651,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 30131,
        "source_sha256": "6b4861d8bfbbd7e2718e577ec2d479bfff32e94dc47a3b7dda9a1b363baa98b6",
        "wasm_sha256": "e4c87dbab50a2438cb1f749bc6fd6f9c834c81484acd42864735bd1ad1b6dec0",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_branch_entropy_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 12331,
            "sides": [
              {
                "calibrated_total_ns": 21865292.207,
                "ns_per_operation": 1773.197,
                "observed_batch": 12331,
                "requested_batch": 12331,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 22865619.919999998,
                "ns_per_operation": 1854.32,
                "observed_batch": 12331,
                "requested_batch": 12331,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 21865292.207,
            "ns_per_operation": 1773.197,
            "observed_batch": 12331,
            "requested_batch": 12331,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22865619.919999998,
            "ns_per_operation": 1854.32,
            "observed_batch": 12331,
            "requested_batch": 12331,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 503320.974,
            "ns_per_operation": 1639.482,
            "observed_batch": 307,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 24361949.777,
            "ns_per_operation": 1975.667,
            "observed_batch": 12331,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 12331,
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
                "calibrated_total_ns": 24385960.0,
                "ns_per_operation": 12192980.0,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 23460417.0,
                "ns_per_operation": 11730208.5,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 24385960.0,
            "ns_per_operation": 12192980.0,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 23460417.0,
            "ns_per_operation": 11730208.5,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 21547262.0,
            "ns_per_operation": 10773631.0,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 23579652.0,
            "ns_per_operation": 11789826.0,
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
            "batch": 10,
            "sides": [
              {
                "calibrated_total_ns": 26831259.0,
                "ns_per_operation": 2683125.9,
                "observed_batch": 10,
                "requested_batch": 10,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 14606736.0,
                "ns_per_operation": 1460673.6,
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
                "calibrated_total_ns": 84551494.0,
                "ns_per_operation": 5284468.375,
                "observed_batch": 16,
                "requested_batch": 16,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 40379808.992,
                "ns_per_operation": 2523738.062,
                "observed_batch": 16,
                "requested_batch": 16,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 84551494.0,
            "ns_per_operation": 5284468.375,
            "observed_batch": 16,
            "requested_batch": 16,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 40379808.992,
            "ns_per_operation": 2523738.062,
            "observed_batch": 16,
            "requested_batch": 16,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 13096451.0,
            "ns_per_operation": 2619290.2,
            "observed_batch": 5,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 20396512.0,
            "ns_per_operation": 2039651.2,
            "observed_batch": 10,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 16,
        "source_sha256": "6b4861d8bfbbd7e2718e577ec2d479bfff32e94dc47a3b7dda9a1b363baa98b6",
        "wasm_sha256": "e4c87dbab50a2438cb1f749bc6fd6f9c834c81484acd42864735bd1ad1b6dec0",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_branch_entropy_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1429,
            "sides": [
              {
                "calibrated_total_ns": 35490353.912999995,
                "ns_per_operation": 24835.797,
                "observed_batch": 1429,
                "requested_batch": 1429,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 27572987.987,
                "ns_per_operation": 19295.303,
                "observed_batch": 1429,
                "requested_batch": 1429,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 35490353.912999995,
            "ns_per_operation": 24835.797,
            "observed_batch": 1429,
            "requested_batch": 1429,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27572987.987,
            "ns_per_operation": 19295.303,
            "observed_batch": 1429,
            "requested_batch": 1429,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 5698881.0,
            "ns_per_operation": 22795.524,
            "observed_batch": 250,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 25623717.667000003,
            "ns_per_operation": 17931.223,
            "observed_batch": 1429,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1429,
        "source_sha256": "6b4861d8bfbbd7e2718e577ec2d479bfff32e94dc47a3b7dda9a1b363baa98b6",
        "wasm_sha256": "e4c87dbab50a2438cb1f749bc6fd6f9c834c81484acd42864735bd1ad1b6dec0",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_branch_entropy_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 764,
            "sides": [
              {
                "calibrated_total_ns": 75932481.66,
                "ns_per_operation": 99388.065,
                "observed_batch": 764,
                "requested_batch": 764,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 69342015.964,
                "ns_per_operation": 90761.801,
                "observed_batch": 764,
                "requested_batch": 764,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 75932481.66,
            "ns_per_operation": 99388.065,
            "observed_batch": 764,
            "requested_batch": 764,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 69342015.964,
            "ns_per_operation": 90761.801,
            "observed_batch": 764,
            "requested_batch": 764,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 74076565.984,
            "ns_per_operation": 96958.856,
            "observed_batch": 764,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 51115573.04,
            "ns_per_operation": 91277.809,
            "observed_batch": 560,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 764,
        "source_sha256": "6b4861d8bfbbd7e2718e577ec2d479bfff32e94dc47a3b7dda9a1b363baa98b6",
        "wasm_sha256": "e4c87dbab50a2438cb1f749bc6fd6f9c834c81484acd42864735bd1ad1b6dec0",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_call_ref_n0": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 25907,
            "sides": [
              {
                "calibrated_total_ns": 21537939.892,
                "ns_per_operation": 831.356,
                "observed_batch": 25907,
                "requested_batch": 25907,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 19937742.223,
                "ns_per_operation": 769.589,
                "observed_batch": 25907,
                "requested_batch": 25907,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 28587,
            "sides": [
              {
                "calibrated_total_ns": 21285994.548,
                "ns_per_operation": 744.604,
                "observed_batch": 28587,
                "requested_batch": 28587,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 22217130.312000003,
                "ns_per_operation": 777.176,
                "observed_batch": 28587,
                "requested_batch": 28587,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 21285994.548,
            "ns_per_operation": 744.604,
            "observed_batch": 28587,
            "requested_batch": 28587,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22217130.312000003,
            "ns_per_operation": 777.176,
            "observed_batch": 28587,
            "requested_batch": 28587,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 19018484.142,
            "ns_per_operation": 734.106,
            "observed_batch": 25907,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 19168484.741,
            "ns_per_operation": 742.763,
            "observed_batch": 25807,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 28587,
        "source_sha256": "0323154b79000eeeb0a63310541ec91b6c4d91e32a6a3406125a30f2c8e53a12",
        "wasm_sha256": "4f766dc9e29324ea942c892982363dec0ade98565288878a70967ce3b33f322b",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_call_ref_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 12945,
            "sides": [
              {
                "calibrated_total_ns": 28420346.205000002,
                "ns_per_operation": 2195.469,
                "observed_batch": 12945,
                "requested_batch": 12945,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 22903264.875,
                "ns_per_operation": 1769.275,
                "observed_batch": 12945,
                "requested_batch": 12945,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 28420346.205000002,
            "ns_per_operation": 2195.469,
            "observed_batch": 12945,
            "requested_batch": 12945,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22903264.875,
            "ns_per_operation": 1769.275,
            "observed_batch": 12945,
            "requested_batch": 12945,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 35719242.06,
            "ns_per_operation": 2759.308,
            "observed_batch": 12945,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22083951.276,
            "ns_per_operation": 1859.388,
            "observed_batch": 11877,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 12945,
        "source_sha256": "0323154b79000eeeb0a63310541ec91b6c4d91e32a6a3406125a30f2c8e53a12",
        "wasm_sha256": "4f766dc9e29324ea942c892982363dec0ade98565288878a70967ce3b33f322b",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_call_ref_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 3,
            "sides": [
              {
                "calibrated_total_ns": 28749273.999,
                "ns_per_operation": 9583091.333,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 31205889.0,
                "ns_per_operation": 10401963.0,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 28749273.999,
            "ns_per_operation": 9583091.333,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 31205889.0,
            "ns_per_operation": 10401963.0,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 29966517.0,
            "ns_per_operation": 9988839.0,
            "observed_batch": 3,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 20135934.0,
            "ns_per_operation": 10067967.0,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 3,
        "source_sha256": "0323154b79000eeeb0a63310541ec91b6c4d91e32a6a3406125a30f2c8e53a12",
        "wasm_sha256": "4f766dc9e29324ea942c892982363dec0ade98565288878a70967ce3b33f322b",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_call_ref_n1024": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 16,
            "sides": [
              {
                "calibrated_total_ns": 21118796.992,
                "ns_per_operation": 1319924.812,
                "observed_batch": 16,
                "requested_batch": 16,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 20937652.0,
                "ns_per_operation": 1308603.25,
                "observed_batch": 16,
                "requested_batch": 16,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 21118796.992,
            "ns_per_operation": 1319924.812,
            "observed_batch": 16,
            "requested_batch": 16,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 20937652.0,
            "ns_per_operation": 1308603.25,
            "observed_batch": 16,
            "requested_batch": 16,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 18750725.0,
            "ns_per_operation": 1339337.5,
            "observed_batch": 14,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 20765764.992,
            "ns_per_operation": 1297860.312,
            "observed_batch": 16,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 16,
        "source_sha256": "0323154b79000eeeb0a63310541ec91b6c4d91e32a6a3406125a30f2c8e53a12",
        "wasm_sha256": "4f766dc9e29324ea942c892982363dec0ade98565288878a70967ce3b33f322b",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_call_ref_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1531,
            "sides": [
              {
                "calibrated_total_ns": 29793780.54,
                "ns_per_operation": 19460.34,
                "observed_batch": 1531,
                "requested_batch": 1531,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 33500555.066,
                "ns_per_operation": 21881.486,
                "observed_batch": 1531,
                "requested_batch": 1531,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 29793780.54,
            "ns_per_operation": 19460.34,
            "observed_batch": 1531,
            "requested_batch": 1531,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 33500555.066,
            "ns_per_operation": 21881.486,
            "observed_batch": 1531,
            "requested_batch": 1531,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 30332351.126999997,
            "ns_per_operation": 19812.117,
            "observed_batch": 1531,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 29046404.652,
            "ns_per_operation": 19021.876,
            "observed_batch": 1527,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1531,
        "source_sha256": "0323154b79000eeeb0a63310541ec91b6c4d91e32a6a3406125a30f2c8e53a12",
        "wasm_sha256": "4f766dc9e29324ea942c892982363dec0ade98565288878a70967ce3b33f322b",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_call_ref_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 859,
            "sides": [
              {
                "calibrated_total_ns": 29172826.279000003,
                "ns_per_operation": 33961.381,
                "observed_batch": 859,
                "requested_batch": 859,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 28499885.679,
                "ns_per_operation": 33177.981,
                "observed_batch": 859,
                "requested_batch": 859,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 29172826.279000003,
            "ns_per_operation": 33961.381,
            "observed_batch": 859,
            "requested_batch": 859,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 28499885.679,
            "ns_per_operation": 33177.981,
            "observed_batch": 859,
            "requested_batch": 859,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 11328354.152,
            "ns_per_operation": 33917.228,
            "observed_batch": 334,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 32476499.047,
            "ns_per_operation": 37807.333,
            "observed_batch": 859,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 859,
        "source_sha256": "0323154b79000eeeb0a63310541ec91b6c4d91e32a6a3406125a30f2c8e53a12",
        "wasm_sha256": "4f766dc9e29324ea942c892982363dec0ade98565288878a70967ce3b33f322b",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_dependent_n0": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 22806,
            "sides": [
              {
                "calibrated_total_ns": 19299805.56,
                "ns_per_operation": 846.26,
                "observed_batch": 22806,
                "requested_batch": 22806,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 16072984.62,
                "ns_per_operation": 704.77,
                "observed_batch": 22806,
                "requested_batch": 22806,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 31216,
            "sides": [
              {
                "calibrated_total_ns": 20924740.336,
                "ns_per_operation": 670.321,
                "observed_batch": 31216,
                "requested_batch": 31216,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 22565609.376,
                "ns_per_operation": 722.886,
                "observed_batch": 31216,
                "requested_batch": 31216,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 20924740.336,
            "ns_per_operation": 670.321,
            "observed_batch": 31216,
            "requested_batch": 31216,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22565609.376,
            "ns_per_operation": 722.886,
            "observed_batch": 31216,
            "requested_batch": 31216,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 34106053.716000006,
            "ns_per_operation": 1495.486,
            "observed_batch": 22806,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 19898807.496,
            "ns_per_operation": 887.468,
            "observed_batch": 22422,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 31216,
        "source_sha256": "0dd5e9778eb09bac9ee432e5fece3f432d034ea9878d2302972142501f0c60c6",
        "wasm_sha256": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_dependent_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 14663,
            "sides": [
              {
                "calibrated_total_ns": 23892625.35,
                "ns_per_operation": 1629.45,
                "observed_batch": 14663,
                "requested_batch": 14663,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 21429270.676,
                "ns_per_operation": 1461.452,
                "observed_batch": 14663,
                "requested_batch": 14663,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 23892625.35,
            "ns_per_operation": 1629.45,
            "observed_batch": 14663,
            "requested_batch": 14663,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 21429270.676,
            "ns_per_operation": 1461.452,
            "observed_batch": 14663,
            "requested_batch": 14663,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 21832473.85,
            "ns_per_operation": 1488.95,
            "observed_batch": 14663,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 20821113.235,
            "ns_per_operation": 1470.937,
            "observed_batch": 14155,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 14663,
        "source_sha256": "0dd5e9778eb09bac9ee432e5fece3f432d034ea9878d2302972142501f0c60c6",
        "wasm_sha256": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_dependent_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 4,
            "sides": [
              {
                "calibrated_total_ns": 53794034.0,
                "ns_per_operation": 13448508.5,
                "observed_batch": 4,
                "requested_batch": 4,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 50393957.0,
                "ns_per_operation": 12598489.25,
                "observed_batch": 4,
                "requested_batch": 4,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 53794034.0,
            "ns_per_operation": 13448508.5,
            "observed_batch": 4,
            "requested_batch": 4,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 50393957.0,
            "ns_per_operation": 12598489.25,
            "observed_batch": 4,
            "requested_batch": 4,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 32358342.0,
            "ns_per_operation": 8089585.5,
            "observed_batch": 4,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 36167791.0,
            "ns_per_operation": 9041947.75,
            "observed_batch": 4,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 4,
        "source_sha256": "0dd5e9778eb09bac9ee432e5fece3f432d034ea9878d2302972142501f0c60c6",
        "wasm_sha256": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_dependent_n1024": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 29,
            "sides": [
              {
                "calibrated_total_ns": 30654404.992000002,
                "ns_per_operation": 1057048.448,
                "observed_batch": 29,
                "requested_batch": 29,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 28205984.988,
                "ns_per_operation": 972620.172,
                "observed_batch": 29,
                "requested_batch": 29,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 30654404.992000002,
            "ns_per_operation": 1057048.448,
            "observed_batch": 29,
            "requested_batch": 29,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 28205984.988,
            "ns_per_operation": 972620.172,
            "observed_batch": 29,
            "requested_batch": 29,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 24589549.998,
            "ns_per_operation": 945751.923,
            "observed_batch": 26,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 29129733.008,
            "ns_per_operation": 1004473.552,
            "observed_batch": 29,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 29,
        "source_sha256": "0dd5e9778eb09bac9ee432e5fece3f432d034ea9878d2302972142501f0c60c6",
        "wasm_sha256": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_dependent_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 2374,
            "sides": [
              {
                "calibrated_total_ns": 29550576.286,
                "ns_per_operation": 12447.589,
                "observed_batch": 2374,
                "requested_batch": 2374,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 57363842.754,
                "ns_per_operation": 24163.371,
                "observed_batch": 2374,
                "requested_batch": 2374,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 29550576.286,
            "ns_per_operation": 12447.589,
            "observed_batch": 2374,
            "requested_batch": 2374,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 57363842.754,
            "ns_per_operation": 24163.371,
            "observed_batch": 2374,
            "requested_batch": 2374,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 25281668.996999998,
            "ns_per_operation": 10680.891,
            "observed_batch": 2367,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 25221243.055999998,
            "ns_per_operation": 10623.944,
            "observed_batch": 2374,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 2374,
        "source_sha256": "0dd5e9778eb09bac9ee432e5fece3f432d034ea9878d2302972142501f0c60c6",
        "wasm_sha256": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_dependent_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1317,
            "sides": [
              {
                "calibrated_total_ns": 32003579.388,
                "ns_per_operation": 24300.364,
                "observed_batch": 1317,
                "requested_batch": 1317,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 28734797.241,
                "ns_per_operation": 21818.373,
                "observed_batch": 1317,
                "requested_batch": 1317,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 32003579.388,
            "ns_per_operation": 24300.364,
            "observed_batch": 1317,
            "requested_batch": 1317,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 28734797.241,
            "ns_per_operation": 21818.373,
            "observed_batch": 1317,
            "requested_batch": 1317,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 28763220.107,
            "ns_per_operation": 22176.731,
            "observed_batch": 1297,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 33148687.182,
            "ns_per_operation": 25169.846,
            "observed_batch": 1317,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1317,
        "source_sha256": "0dd5e9778eb09bac9ee432e5fece3f432d034ea9878d2302972142501f0c60c6",
        "wasm_sha256": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_direct_calls_n0": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 23924,
            "sides": [
              {
                "calibrated_total_ns": 16162456.3,
                "ns_per_operation": 675.575,
                "observed_batch": 23924,
                "requested_batch": 23924,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 15608735.319999998,
                "ns_per_operation": 652.43,
                "observed_batch": 23924,
                "requested_batch": 23924,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 33721,
            "sides": [
              {
                "calibrated_total_ns": 25784627.766,
                "ns_per_operation": 764.646,
                "observed_batch": 33721,
                "requested_batch": 33721,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 23857742.384,
                "ns_per_operation": 707.504,
                "observed_batch": 33721,
                "requested_batch": 33721,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 25784627.766,
            "ns_per_operation": 764.646,
            "observed_batch": 33721,
            "requested_batch": 33721,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 23857742.384,
            "ns_per_operation": 707.504,
            "observed_batch": 33721,
            "requested_batch": 33721,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 21179630.112,
            "ns_per_operation": 885.288,
            "observed_batch": 23924,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 17243859.766000003,
            "ns_per_operation": 722.498,
            "observed_batch": 23867,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 33721,
        "source_sha256": "0a075601a2a651f9cb95e8ee0404c64bbf076d17bf242b9c88fecbb8cb973826",
        "wasm_sha256": "f6adbecce99c05bf06f213f4f5c0d5d9f001074acbbb2e6bbe814696bb866e45",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_direct_calls_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 13150,
            "sides": [
              {
                "calibrated_total_ns": 23464268.25,
                "ns_per_operation": 1784.355,
                "observed_batch": 13150,
                "requested_batch": 13150,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 25871678.200000003,
                "ns_per_operation": 1967.428,
                "observed_batch": 13150,
                "requested_batch": 13150,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 23464268.25,
            "ns_per_operation": 1784.355,
            "observed_batch": 13150,
            "requested_batch": 13150,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 25871678.200000003,
            "ns_per_operation": 1967.428,
            "observed_batch": 13150,
            "requested_batch": 13150,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 25645180.518,
            "ns_per_operation": 2116.987,
            "observed_batch": 12114,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 28886013.250000004,
            "ns_per_operation": 2196.655,
            "observed_batch": 13150,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 13150,
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
                "calibrated_total_ns": 37303095.999,
                "ns_per_operation": 12434365.333,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 35894583.0,
                "ns_per_operation": 11964861.0,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 37303095.999,
            "ns_per_operation": 12434365.333,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 35894583.0,
            "ns_per_operation": 11964861.0,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 32636795.001,
            "ns_per_operation": 10878931.667,
            "observed_batch": 3,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 30319781.0,
            "ns_per_operation": 15159890.5,
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
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 26,
            "sides": [
              {
                "calibrated_total_ns": 23138593.01,
                "ns_per_operation": 889945.885,
                "observed_batch": 26,
                "requested_batch": 26,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 24990456.998,
                "ns_per_operation": 961171.423,
                "observed_batch": 26,
                "requested_batch": 26,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 23138593.01,
            "ns_per_operation": 889945.885,
            "observed_batch": 26,
            "requested_batch": 26,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 24990456.998,
            "ns_per_operation": 961171.423,
            "observed_batch": 26,
            "requested_batch": 26,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 25116964.002,
            "ns_per_operation": 966037.077,
            "observed_batch": 26,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 21513995.99,
            "ns_per_operation": 935391.13,
            "observed_batch": 23,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 26,
        "source_sha256": "0a075601a2a651f9cb95e8ee0404c64bbf076d17bf242b9c88fecbb8cb973826",
        "wasm_sha256": "f6adbecce99c05bf06f213f4f5c0d5d9f001074acbbb2e6bbe814696bb866e45",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_direct_calls_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1688,
            "sides": [
              {
                "calibrated_total_ns": 27937289.575999998,
                "ns_per_operation": 16550.527,
                "observed_batch": 1688,
                "requested_batch": 1688,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 26840278.632,
                "ns_per_operation": 15900.639,
                "observed_batch": 1688,
                "requested_batch": 1688,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 27937289.575999998,
            "ns_per_operation": 16550.527,
            "observed_batch": 1688,
            "requested_batch": 1688,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 26840278.632,
            "ns_per_operation": 15900.639,
            "observed_batch": 1688,
            "requested_batch": 1688,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 25023978.816,
            "ns_per_operation": 14824.632,
            "observed_batch": 1688,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 24552340.542,
            "ns_per_operation": 14693.202,
            "observed_batch": 1671,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1688,
        "source_sha256": "0a075601a2a651f9cb95e8ee0404c64bbf076d17bf242b9c88fecbb8cb973826",
        "wasm_sha256": "f6adbecce99c05bf06f213f4f5c0d5d9f001074acbbb2e6bbe814696bb866e45",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_direct_calls_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 941,
            "sides": [
              {
                "calibrated_total_ns": 32062471.865,
                "ns_per_operation": 34072.765,
                "observed_batch": 941,
                "requested_batch": 941,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 33289980.017,
                "ns_per_operation": 35377.237,
                "observed_batch": 941,
                "requested_batch": 941,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 32062471.865,
            "ns_per_operation": 34072.765,
            "observed_batch": 941,
            "requested_batch": 941,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 33289980.017,
            "ns_per_operation": 35377.237,
            "observed_batch": 941,
            "requested_batch": 941,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 31128121.912,
            "ns_per_operation": 33079.832,
            "observed_batch": 941,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 31361588.125,
            "ns_per_operation": 37830.625,
            "observed_batch": 829,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 941,
        "source_sha256": "0a075601a2a651f9cb95e8ee0404c64bbf076d17bf242b9c88fecbb8cb973826",
        "wasm_sha256": "f6adbecce99c05bf06f213f4f5c0d5d9f001074acbbb2e6bbe814696bb866e45",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_dynamic_api_n0": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 51283,
            "sides": [
              {
                "calibrated_total_ns": 19595849.696000002,
                "ns_per_operation": 382.112,
                "observed_batch": 51283,
                "requested_batch": 51283,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 16665590.359000001,
                "ns_per_operation": 324.973,
                "observed_batch": 51283,
                "requested_batch": 51283,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 67698,
            "sides": [
              {
                "calibrated_total_ns": 23595054.732,
                "ns_per_operation": 348.534,
                "observed_batch": 67698,
                "requested_batch": 67698,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 25741284.426000003,
                "ns_per_operation": 380.237,
                "observed_batch": 67698,
                "requested_batch": 67698,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 23595054.732,
            "ns_per_operation": 348.534,
            "observed_batch": 67698,
            "requested_batch": 67698,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 25741284.426000003,
            "ns_per_operation": 380.237,
            "observed_batch": 67698,
            "requested_batch": 67698,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 14979890.913,
            "ns_per_operation": 313.079,
            "observed_batch": 47847,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 16306045.246,
            "ns_per_operation": 317.962,
            "observed_batch": 51283,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 67698,
        "source_sha256": "f02cc93a4a43691ce5be8782fa2df454fc2468a7935117ea18104a7dce22cb2d",
        "wasm_sha256": "c493d0aae68d0c5aa67fc0b562e616c45eb3d42b0302951158a763fa202ce589",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_dynamic_api_n1": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 50252,
            "sides": [
              {
                "calibrated_total_ns": 17754333.112,
                "ns_per_operation": 353.306,
                "observed_batch": 50252,
                "requested_batch": 50252,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 17714181.764,
                "ns_per_operation": 352.507,
                "observed_batch": 50252,
                "requested_batch": 50252,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 62411,
            "sides": [
              {
                "calibrated_total_ns": 21762715.7,
                "ns_per_operation": 348.7,
                "observed_batch": 62411,
                "requested_batch": 62411,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 21978096.061,
                "ns_per_operation": 352.151,
                "observed_batch": 62411,
                "requested_batch": 62411,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 21762715.7,
            "ns_per_operation": 348.7,
            "observed_batch": 62411,
            "requested_batch": 62411,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 21978096.061,
            "ns_per_operation": 352.151,
            "observed_batch": 62411,
            "requested_batch": 62411,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 16076418.832,
            "ns_per_operation": 319.916,
            "observed_batch": 50252,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 14261028.237,
            "ns_per_operation": 331.567,
            "observed_batch": 43011,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 62411,
        "source_sha256": "f02cc93a4a43691ce5be8782fa2df454fc2468a7935117ea18104a7dce22cb2d",
        "wasm_sha256": "c493d0aae68d0c5aa67fc0b562e616c45eb3d42b0302951158a763fa202ce589",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_dynamic_api_n10000": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 54496,
            "sides": [
              {
                "calibrated_total_ns": 17617194.4,
                "ns_per_operation": 323.275,
                "observed_batch": 54496,
                "requested_batch": 54496,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 17674960.16,
                "ns_per_operation": 324.335,
                "observed_batch": 54496,
                "requested_batch": 54496,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 68054,
            "sides": [
              {
                "calibrated_total_ns": 23570639.007999998,
                "ns_per_operation": 346.352,
                "observed_batch": 68054,
                "requested_batch": 68054,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 40268708.718,
                "ns_per_operation": 591.717,
                "observed_batch": 68054,
                "requested_batch": 68054,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 23570639.007999998,
            "ns_per_operation": 346.352,
            "observed_batch": 68054,
            "requested_batch": 68054,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 40268708.718,
            "ns_per_operation": 591.717,
            "observed_batch": 68054,
            "requested_batch": 68054,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 17275415.94,
            "ns_per_operation": 323.91,
            "observed_batch": 53334,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 19984827.616,
            "ns_per_operation": 366.721,
            "observed_batch": 54496,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 68054,
        "source_sha256": "f02cc93a4a43691ce5be8782fa2df454fc2468a7935117ea18104a7dce22cb2d",
        "wasm_sha256": "c493d0aae68d0c5aa67fc0b562e616c45eb3d42b0302951158a763fa202ce589",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_dynamic_api_n1024": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 47282,
            "sides": [
              {
                "calibrated_total_ns": 15220832.311999999,
                "ns_per_operation": 321.916,
                "observed_batch": 47282,
                "requested_batch": 47282,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 18220260.546,
                "ns_per_operation": 385.353,
                "observed_batch": 47282,
                "requested_batch": 47282,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 68341,
            "sides": [
              {
                "calibrated_total_ns": 25261635.581,
                "ns_per_operation": 369.641,
                "observed_batch": 68341,
                "requested_batch": 68341,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 22417898.229999997,
                "ns_per_operation": 328.03,
                "observed_batch": 68341,
                "requested_batch": 68341,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 25261635.581,
            "ns_per_operation": 369.641,
            "observed_batch": 68341,
            "requested_batch": 68341,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22417898.229999997,
            "ns_per_operation": 328.03,
            "observed_batch": 68341,
            "requested_batch": 68341,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 17368286.188,
            "ns_per_operation": 367.334,
            "observed_batch": 47282,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 6798256.050000001,
            "ns_per_operation": 386.814,
            "observed_batch": 17575,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 68341,
        "source_sha256": "f02cc93a4a43691ce5be8782fa2df454fc2468a7935117ea18104a7dce22cb2d",
        "wasm_sha256": "c493d0aae68d0c5aa67fc0b562e616c45eb3d42b0302951158a763fa202ce589",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_dynamic_api_n17": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 53334,
            "sides": [
              {
                "calibrated_total_ns": 17833769.586,
                "ns_per_operation": 334.379,
                "observed_batch": 53334,
                "requested_batch": 53334,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 18359696.16,
                "ns_per_operation": 344.24,
                "observed_batch": 53334,
                "requested_batch": 53334,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 65794,
            "sides": [
              {
                "calibrated_total_ns": 23376673.994,
                "ns_per_operation": 355.301,
                "observed_batch": 65794,
                "requested_batch": 65794,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 20172571.987999998,
                "ns_per_operation": 306.602,
                "observed_batch": 65794,
                "requested_batch": 65794,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 23376673.994,
            "ns_per_operation": 355.301,
            "observed_batch": 65794,
            "requested_batch": 65794,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 20172571.987999998,
            "ns_per_operation": 306.602,
            "observed_batch": 65794,
            "requested_batch": 65794,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 17353656.918,
            "ns_per_operation": 325.377,
            "observed_batch": 53334,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 15175576.197,
            "ns_per_operation": 308.817,
            "observed_batch": 49141,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 65794,
        "source_sha256": "f02cc93a4a43691ce5be8782fa2df454fc2468a7935117ea18104a7dce22cb2d",
        "wasm_sha256": "c493d0aae68d0c5aa67fc0b562e616c45eb3d42b0302951158a763fa202ce589",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_dynamic_api_n32": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 58480,
            "sides": [
              {
                "calibrated_total_ns": 21802455.12,
                "ns_per_operation": 372.819,
                "observed_batch": 58480,
                "requested_batch": 58480,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 19538986.72,
                "ns_per_operation": 334.114,
                "observed_batch": 58480,
                "requested_batch": 58480,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 65846,
            "sides": [
              {
                "calibrated_total_ns": 25018385.237999998,
                "ns_per_operation": 379.953,
                "observed_batch": 65846,
                "requested_batch": 65846,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 27138758.13,
                "ns_per_operation": 412.155,
                "observed_batch": 65846,
                "requested_batch": 65846,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 25018385.237999998,
            "ns_per_operation": 379.953,
            "observed_batch": 65846,
            "requested_batch": 65846,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27138758.13,
            "ns_per_operation": 412.155,
            "observed_batch": 65846,
            "requested_batch": 65846,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 16883056.342,
            "ns_per_operation": 318.242,
            "observed_batch": 53051,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 21030928.48,
            "ns_per_operation": 359.626,
            "observed_batch": 58480,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 65846,
        "source_sha256": "f02cc93a4a43691ce5be8782fa2df454fc2468a7935117ea18104a7dce22cb2d",
        "wasm_sha256": "c493d0aae68d0c5aa67fc0b562e616c45eb3d42b0302951158a763fa202ce589",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_host_call_n0": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 27933,
            "sides": [
              {
                "calibrated_total_ns": 19908742.956,
                "ns_per_operation": 712.732,
                "observed_batch": 27933,
                "requested_batch": 27933,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 16767537.441000002,
                "ns_per_operation": 600.277,
                "observed_batch": 27933,
                "requested_batch": 27933,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 36650,
            "sides": [
              {
                "calibrated_total_ns": 23240717.9,
                "ns_per_operation": 634.126,
                "observed_batch": 36650,
                "requested_batch": 36650,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 33230115.2,
                "ns_per_operation": 906.688,
                "observed_batch": 36650,
                "requested_batch": 36650,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 23240717.9,
            "ns_per_operation": 634.126,
            "observed_batch": 36650,
            "requested_batch": 36650,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 33230115.2,
            "ns_per_operation": 906.688,
            "observed_batch": 36650,
            "requested_batch": 36650,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 17193136.59,
            "ns_per_operation": 685.122,
            "observed_batch": 25095,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 30712501.098,
            "ns_per_operation": 1099.506,
            "observed_batch": 27933,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 36650,
        "source_sha256": "4c4026de620eff0b1e9cb4ed4e0440e6045d921267e3962715f6430ac68f5120",
        "wasm_sha256": "c60311d57c929c3ff1d29adb70501e7bbf8419517acbd86c8d072438b610ebb7",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_host_call_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 14610,
            "sides": [
              {
                "calibrated_total_ns": 24997286.31,
                "ns_per_operation": 1710.971,
                "observed_batch": 14610,
                "requested_batch": 14610,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 21164864.16,
                "ns_per_operation": 1448.656,
                "observed_batch": 14610,
                "requested_batch": 14610,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 24997286.31,
            "ns_per_operation": 1710.971,
            "observed_batch": 14610,
            "requested_batch": 14610,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 21164864.16,
            "ns_per_operation": 1448.656,
            "observed_batch": 14610,
            "requested_batch": 14610,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 19273565.04,
            "ns_per_operation": 1491.762,
            "observed_batch": 12920,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 21428530.83,
            "ns_per_operation": 1466.703,
            "observed_batch": 14610,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 14610,
        "source_sha256": "4c4026de620eff0b1e9cb4ed4e0440e6045d921267e3962715f6430ac68f5120",
        "wasm_sha256": "c60311d57c929c3ff1d29adb70501e7bbf8419517acbd86c8d072438b610ebb7",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_host_call_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 4,
            "sides": [
              {
                "calibrated_total_ns": 25886193.0,
                "ns_per_operation": 6471548.25,
                "observed_batch": 4,
                "requested_batch": 4,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 24309641.0,
                "ns_per_operation": 6077410.25,
                "observed_batch": 4,
                "requested_batch": 4,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 25886193.0,
            "ns_per_operation": 6471548.25,
            "observed_batch": 4,
            "requested_batch": 4,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 24309641.0,
            "ns_per_operation": 6077410.25,
            "observed_batch": 4,
            "requested_batch": 4,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 24474276.0,
            "ns_per_operation": 6118569.0,
            "observed_batch": 4,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 26511934.0,
            "ns_per_operation": 6627983.5,
            "observed_batch": 4,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 4,
        "source_sha256": "4c4026de620eff0b1e9cb4ed4e0440e6045d921267e3962715f6430ac68f5120",
        "wasm_sha256": "c60311d57c929c3ff1d29adb70501e7bbf8419517acbd86c8d072438b610ebb7",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_host_call_n1024": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 36,
            "sides": [
              {
                "calibrated_total_ns": 26891214.984,
                "ns_per_operation": 746978.194,
                "observed_batch": 36,
                "requested_batch": 36,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 24416219.016,
                "ns_per_operation": 678228.306,
                "observed_batch": 36,
                "requested_batch": 36,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 26891214.984,
            "ns_per_operation": 746978.194,
            "observed_batch": 36,
            "requested_batch": 36,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 24416219.016,
            "ns_per_operation": 678228.306,
            "observed_batch": 36,
            "requested_batch": 36,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 24717559.008,
            "ns_per_operation": 772423.719,
            "observed_batch": 32,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 32164192.008,
            "ns_per_operation": 893449.778,
            "observed_batch": 36,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 36,
        "source_sha256": "4c4026de620eff0b1e9cb4ed4e0440e6045d921267e3962715f6430ac68f5120",
        "wasm_sha256": "c60311d57c929c3ff1d29adb70501e7bbf8419517acbd86c8d072438b610ebb7",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_host_call_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 2079,
            "sides": [
              {
                "calibrated_total_ns": 58088580.165,
                "ns_per_operation": 27940.635,
                "observed_batch": 2079,
                "requested_batch": 2079,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 27649292.517,
                "ns_per_operation": 13299.323,
                "observed_batch": 2079,
                "requested_batch": 2079,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 58088580.165,
            "ns_per_operation": 27940.635,
            "observed_batch": 2079,
            "requested_batch": 2079,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27649292.517,
            "ns_per_operation": 13299.323,
            "observed_batch": 2079,
            "requested_batch": 2079,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 40887656.370000005,
            "ns_per_operation": 19752.491,
            "observed_batch": 2070,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 26804763.216,
            "ns_per_operation": 12893.104,
            "observed_batch": 2079,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 2079,
        "source_sha256": "4c4026de620eff0b1e9cb4ed4e0440e6045d921267e3962715f6430ac68f5120",
        "wasm_sha256": "c60311d57c929c3ff1d29adb70501e7bbf8419517acbd86c8d072438b610ebb7",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_host_call_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1168,
            "sides": [
              {
                "calibrated_total_ns": 27124307.488,
                "ns_per_operation": 23222.866,
                "observed_batch": 1168,
                "requested_batch": 1168,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 29794367.168,
                "ns_per_operation": 25508.876,
                "observed_batch": 1168,
                "requested_batch": 1168,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 27124307.488,
            "ns_per_operation": 23222.866,
            "observed_batch": 1168,
            "requested_batch": 1168,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 29794367.168,
            "ns_per_operation": 25508.876,
            "observed_batch": 1168,
            "requested_batch": 1168,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 30628709.671,
            "ns_per_operation": 26610.521,
            "observed_batch": 1151,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 30633826.288,
            "ns_per_operation": 26227.591,
            "observed_batch": 1168,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1168,
        "source_sha256": "4c4026de620eff0b1e9cb4ed4e0440e6045d921267e3962715f6430ac68f5120",
        "wasm_sha256": "c60311d57c929c3ff1d29adb70501e7bbf8419517acbd86c8d072438b610ebb7",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_indirect_calls_n0": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 24391,
            "sides": [
              {
                "calibrated_total_ns": 33464256.871999998,
                "ns_per_operation": 1371.992,
                "observed_batch": 24391,
                "requested_batch": 24391,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 41857639.01,
                "ns_per_operation": 1716.11,
                "observed_batch": 24391,
                "requested_batch": 24391,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 33464256.871999998,
            "ns_per_operation": 1371.992,
            "observed_batch": 24391,
            "requested_batch": 24391,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 41857639.01,
            "ns_per_operation": 1716.11,
            "observed_batch": 24391,
            "requested_batch": 24391,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 30277133.684,
            "ns_per_operation": 1241.324,
            "observed_batch": 24391,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 31742534.290000003,
            "ns_per_operation": 1342.69,
            "observed_batch": 23641,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 24391,
        "source_sha256": "29024ae23844664a9fea4a809d1ce414de40f243f7e678ce5ea6d811843f7797",
        "wasm_sha256": "4b42e82f54ea6ab7d0f62c70766c904e41d6a408e57fa14b5abbf514c0da78a9",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_indirect_calls_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 13699,
            "sides": [
              {
                "calibrated_total_ns": 22671940.893,
                "ns_per_operation": 1655.007,
                "observed_batch": 13699,
                "requested_batch": 13699,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 22290026.472,
                "ns_per_operation": 1627.128,
                "observed_batch": 13699,
                "requested_batch": 13699,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 22671940.893,
            "ns_per_operation": 1655.007,
            "observed_batch": 13699,
            "requested_batch": 13699,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22290026.472,
            "ns_per_operation": 1627.128,
            "observed_batch": 13699,
            "requested_batch": 13699,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 26053295.022,
            "ns_per_operation": 1965.693,
            "observed_batch": 13254,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 23801587.831,
            "ns_per_operation": 1737.469,
            "observed_batch": 13699,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 13699,
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
                "calibrated_total_ns": 21151290.0,
                "ns_per_operation": 10575645.0,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 22201332.0,
                "ns_per_operation": 11100666.0,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 21151290.0,
            "ns_per_operation": 10575645.0,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22201332.0,
            "ns_per_operation": 11100666.0,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 21560743.0,
            "ns_per_operation": 10780371.5,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 21312929.0,
            "ns_per_operation": 10656464.5,
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
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 24,
            "sides": [
              {
                "calibrated_total_ns": 29584744.007999998,
                "ns_per_operation": 1232697.667,
                "observed_batch": 24,
                "requested_batch": 24,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 33279022.007999998,
                "ns_per_operation": 1386625.917,
                "observed_batch": 24,
                "requested_batch": 24,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 29584744.007999998,
            "ns_per_operation": 1232697.667,
            "observed_batch": 24,
            "requested_batch": 24,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 33279022.007999998,
            "ns_per_operation": 1386625.917,
            "observed_batch": 24,
            "requested_batch": 24,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 32527906.007999998,
            "ns_per_operation": 1355329.417,
            "observed_batch": 24,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 25143886.005,
            "ns_per_operation": 1197327.905,
            "observed_batch": 21,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 24,
        "source_sha256": "29024ae23844664a9fea4a809d1ce414de40f243f7e678ce5ea6d811843f7797",
        "wasm_sha256": "4b42e82f54ea6ab7d0f62c70766c904e41d6a408e57fa14b5abbf514c0da78a9",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_indirect_calls_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1507,
            "sides": [
              {
                "calibrated_total_ns": 31919025.556,
                "ns_per_operation": 21180.508,
                "observed_batch": 1507,
                "requested_batch": 1507,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 39304689.391,
                "ns_per_operation": 26081.413,
                "observed_batch": 1507,
                "requested_batch": 1507,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 31919025.556,
            "ns_per_operation": 21180.508,
            "observed_batch": 1507,
            "requested_batch": 1507,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 39304689.391,
            "ns_per_operation": 26081.413,
            "observed_batch": 1507,
            "requested_batch": 1507,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 25513439.8,
            "ns_per_operation": 22108.7,
            "observed_batch": 1154,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 35402409.339,
            "ns_per_operation": 23491.977,
            "observed_batch": 1507,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1507,
        "source_sha256": "29024ae23844664a9fea4a809d1ce414de40f243f7e678ce5ea6d811843f7797",
        "wasm_sha256": "4b42e82f54ea6ab7d0f62c70766c904e41d6a408e57fa14b5abbf514c0da78a9",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_indirect_calls_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 869,
            "sides": [
              {
                "calibrated_total_ns": 41238320.265999995,
                "ns_per_operation": 47454.914,
                "observed_batch": 869,
                "requested_batch": 869,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 42927073.167,
                "ns_per_operation": 49398.243,
                "observed_batch": 869,
                "requested_batch": 869,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 41238320.265999995,
            "ns_per_operation": 47454.914,
            "observed_batch": 869,
            "requested_batch": 869,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 42927073.167,
            "ns_per_operation": 49398.243,
            "observed_batch": 869,
            "requested_batch": 869,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 600311.0,
            "ns_per_operation": 24012.44,
            "observed_batch": 25,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 43829488.824,
            "ns_per_operation": 50436.696,
            "observed_batch": 869,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 869,
        "source_sha256": "29024ae23844664a9fea4a809d1ce414de40f243f7e678ce5ea6d811843f7797",
        "wasm_sha256": "4b42e82f54ea6ab7d0f62c70766c904e41d6a408e57fa14b5abbf514c0da78a9",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_multivalue_branch_n0": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 24876,
            "sides": [
              {
                "calibrated_total_ns": 20506356.468,
                "ns_per_operation": 824.343,
                "observed_batch": 24876,
                "requested_batch": 24876,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 24631593.299999997,
                "ns_per_operation": 990.175,
                "observed_batch": 24876,
                "requested_batch": 24876,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 20506356.468,
            "ns_per_operation": 824.343,
            "observed_batch": 24876,
            "requested_batch": 24876,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 24631593.299999997,
            "ns_per_operation": 990.175,
            "observed_batch": 24876,
            "requested_batch": 24876,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 27069889.068,
            "ns_per_operation": 1088.193,
            "observed_batch": 24876,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 28400355.167999998,
            "ns_per_operation": 1177.168,
            "observed_batch": 24126,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 24876,
        "source_sha256": "a1f4958100af82889208bb28090d6af7c3e56cbeeccabad4106601c9a9c17014",
        "wasm_sha256": "03ffdc2041ae8ba0c8d078422d36152cf1d35f1293c94a4074fe7e83791743ae",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_multivalue_branch_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 11429,
            "sides": [
              {
                "calibrated_total_ns": 23617228.47,
                "ns_per_operation": 2066.43,
                "observed_batch": 11429,
                "requested_batch": 11429,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 25699706.56,
                "ns_per_operation": 2248.64,
                "observed_batch": 11429,
                "requested_batch": 11429,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 23617228.47,
            "ns_per_operation": 2066.43,
            "observed_batch": 11429,
            "requested_batch": 11429,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 25699706.56,
            "ns_per_operation": 2248.64,
            "observed_batch": 11429,
            "requested_batch": 11429,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 26059732.902,
            "ns_per_operation": 2300.877,
            "observed_batch": 11326,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 33993091.836,
            "ns_per_operation": 2974.284,
            "observed_batch": 11429,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 11429,
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
                "calibrated_total_ns": 29942419.0,
                "ns_per_operation": 14971209.5,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 36680274.0,
                "ns_per_operation": 18340137.0,
                "observed_batch": 2,
                "requested_batch": 2,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 29942419.0,
            "ns_per_operation": 14971209.5,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 36680274.0,
            "ns_per_operation": 18340137.0,
            "observed_batch": 2,
            "requested_batch": 2,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 29620749.0,
            "ns_per_operation": 14810374.5,
            "observed_batch": 2,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 26364045.0,
            "ns_per_operation": 13182022.5,
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
            "batch": 15,
            "sides": [
              {
                "calibrated_total_ns": 20870101.005,
                "ns_per_operation": 1391340.067,
                "observed_batch": 15,
                "requested_batch": 15,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 22563387.0,
                "ns_per_operation": 1504225.8,
                "observed_batch": 15,
                "requested_batch": 15,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 20870101.005,
            "ns_per_operation": 1391340.067,
            "observed_batch": 15,
            "requested_batch": 15,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22563387.0,
            "ns_per_operation": 1504225.8,
            "observed_batch": 15,
            "requested_batch": 15,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 16055142.996000001,
            "ns_per_operation": 1337928.583,
            "observed_batch": 12,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22492554.0,
            "ns_per_operation": 1499503.6,
            "observed_batch": 15,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 15,
        "source_sha256": "a1f4958100af82889208bb28090d6af7c3e56cbeeccabad4106601c9a9c17014",
        "wasm_sha256": "03ffdc2041ae8ba0c8d078422d36152cf1d35f1293c94a4074fe7e83791743ae",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_multivalue_branch_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1203,
            "sides": [
              {
                "calibrated_total_ns": 33562150.536,
                "ns_per_operation": 27898.712,
                "observed_batch": 1203,
                "requested_batch": 1203,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 35243062.736999996,
                "ns_per_operation": 29295.979,
                "observed_batch": 1203,
                "requested_batch": 1203,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 33562150.536,
            "ns_per_operation": 27898.712,
            "observed_batch": 1203,
            "requested_batch": 1203,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 35243062.736999996,
            "ns_per_operation": 29295.979,
            "observed_batch": 1203,
            "requested_batch": 1203,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 45007997.796,
            "ns_per_operation": 37413.132,
            "observed_batch": 1203,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 30143496.58,
            "ns_per_operation": 29756.66,
            "observed_batch": 1013,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1203,
        "source_sha256": "a1f4958100af82889208bb28090d6af7c3e56cbeeccabad4106601c9a9c17014",
        "wasm_sha256": "03ffdc2041ae8ba0c8d078422d36152cf1d35f1293c94a4074fe7e83791743ae",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_multivalue_branch_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 673,
            "sides": [
              {
                "calibrated_total_ns": 28635503.92,
                "ns_per_operation": 42549.04,
                "observed_batch": 673,
                "requested_batch": 673,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 30852161.327999998,
                "ns_per_operation": 45842.736,
                "observed_batch": 673,
                "requested_batch": 673,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 28635503.92,
            "ns_per_operation": 42549.04,
            "observed_batch": 673,
            "requested_batch": 673,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 30852161.327999998,
            "ns_per_operation": 45842.736,
            "observed_batch": 673,
            "requested_batch": 673,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 28057432.689,
            "ns_per_operation": 42318.903,
            "observed_batch": 663,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 28628109.669,
            "ns_per_operation": 42538.053,
            "observed_batch": 673,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 673,
        "source_sha256": "a1f4958100af82889208bb28090d6af7c3e56cbeeccabad4106601c9a9c17014",
        "wasm_sha256": "03ffdc2041ae8ba0c8d078422d36152cf1d35f1293c94a4074fe7e83791743ae",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_ordinary_recursion_n0": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 9658,
            "sides": [
              {
                "calibrated_total_ns": 7252366.044,
                "ns_per_operation": 750.918,
                "observed_batch": 9658,
                "requested_batch": 9658,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 7679906.388,
                "ns_per_operation": 795.186,
                "observed_batch": 9658,
                "requested_batch": 9658,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 29298,
            "sides": [
              {
                "calibrated_total_ns": 29364418.566000003,
                "ns_per_operation": 1002.267,
                "observed_batch": 29298,
                "requested_batch": 29298,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 26237853.198000003,
                "ns_per_operation": 895.551,
                "observed_batch": 29298,
                "requested_batch": 29298,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 29364418.566000003,
            "ns_per_operation": 1002.267,
            "observed_batch": 29298,
            "requested_batch": 29298,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 26237853.198000003,
            "ns_per_operation": 895.551,
            "observed_batch": 29298,
            "requested_batch": 29298,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 8684029.332,
            "ns_per_operation": 899.154,
            "observed_batch": 9658,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 7990104.87,
            "ns_per_operation": 900.801,
            "observed_batch": 8870,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 29298,
        "source_sha256": "aca6a66637fa36b72f3c528dd5e3e44039b573f363b5b4fb77437e9278623903",
        "wasm_sha256": "e1bd7ea6ada8b15b2770ac387f8fc2256f20d7f87c4bc34e7559b239d8875ba1",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_ordinary_recursion_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 15924,
            "sides": [
              {
                "calibrated_total_ns": 31582308.060000002,
                "ns_per_operation": 1983.315,
                "observed_batch": 15924,
                "requested_batch": 15924,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 23551134.204,
                "ns_per_operation": 1478.971,
                "observed_batch": 15924,
                "requested_batch": 15924,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 31582308.060000002,
            "ns_per_operation": 1983.315,
            "observed_batch": 15924,
            "requested_batch": 15924,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 23551134.204,
            "ns_per_operation": 1478.971,
            "observed_batch": 15924,
            "requested_batch": 15924,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 36816558.708,
            "ns_per_operation": 2312.017,
            "observed_batch": 15924,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 24093547.374,
            "ns_per_operation": 1728.623,
            "observed_batch": 13938,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 15924,
        "source_sha256": "aca6a66637fa36b72f3c528dd5e3e44039b573f363b5b4fb77437e9278623903",
        "wasm_sha256": "e1bd7ea6ada8b15b2770ac387f8fc2256f20d7f87c4bc34e7559b239d8875ba1",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_ordinary_recursion_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1872,
            "sides": [
              {
                "calibrated_total_ns": 37106392.752000004,
                "ns_per_operation": 19821.791,
                "observed_batch": 1872,
                "requested_batch": 1872,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 31337246.304,
                "ns_per_operation": 16739.982,
                "observed_batch": 1872,
                "requested_batch": 1872,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 37106392.752000004,
            "ns_per_operation": 19821.791,
            "observed_batch": 1872,
            "requested_batch": 1872,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 31337246.304,
            "ns_per_operation": 16739.982,
            "observed_batch": 1872,
            "requested_batch": 1872,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 28731691.872,
            "ns_per_operation": 15348.126,
            "observed_batch": 1872,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 30280098.189999998,
            "ns_per_operation": 17666.335,
            "observed_batch": 1714,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1872,
        "source_sha256": "aca6a66637fa36b72f3c528dd5e3e44039b573f363b5b4fb77437e9278623903",
        "wasm_sha256": "e1bd7ea6ada8b15b2770ac387f8fc2256f20d7f87c4bc34e7559b239d8875ba1",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_ordinary_recursion_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1012,
            "sides": [
              {
                "calibrated_total_ns": 31506842.928,
                "ns_per_operation": 31133.244,
                "observed_batch": 1012,
                "requested_batch": 1012,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 39195007.940000005,
                "ns_per_operation": 38730.245,
                "observed_batch": 1012,
                "requested_batch": 1012,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 31506842.928,
            "ns_per_operation": 31133.244,
            "observed_batch": 1012,
            "requested_batch": 1012,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 39195007.940000005,
            "ns_per_operation": 38730.245,
            "observed_batch": 1012,
            "requested_batch": 1012,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 25418557.836,
            "ns_per_operation": 29116.332,
            "observed_batch": 873,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 34943504.86,
            "ns_per_operation": 34529.155,
            "observed_batch": 1012,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1012,
        "source_sha256": "aca6a66637fa36b72f3c528dd5e3e44039b573f363b5b4fb77437e9278623903",
        "wasm_sha256": "e1bd7ea6ada8b15b2770ac387f8fc2256f20d7f87c4bc34e7559b239d8875ba1",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_table_mutation_n0": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 22754,
            "sides": [
              {
                "calibrated_total_ns": 19320080.09,
                "ns_per_operation": 849.085,
                "observed_batch": 22754,
                "requested_batch": 22754,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 16223397.214,
                "ns_per_operation": 712.991,
                "observed_batch": 22754,
                "requested_batch": 22754,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 30856,
            "sides": [
              {
                "calibrated_total_ns": 25412939.888,
                "ns_per_operation": 823.598,
                "observed_batch": 30856,
                "requested_batch": 30856,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 23980882.072,
                "ns_per_operation": 777.187,
                "observed_batch": 30856,
                "requested_batch": 30856,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 25412939.888,
            "ns_per_operation": 823.598,
            "observed_batch": 30856,
            "requested_batch": 30856,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 23980882.072,
            "ns_per_operation": 777.187,
            "observed_batch": 30856,
            "requested_batch": 30856,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 16279701.095999999,
            "ns_per_operation": 722.002,
            "observed_batch": 22548,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 21512018.418,
            "ns_per_operation": 945.417,
            "observed_batch": 22754,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 30856,
        "source_sha256": "9b61bd9cbca804bf639085769fd766a2cf8a69d516fc7efb9e4edfe4ff6d18c9",
        "wasm_sha256": "458f2b111e48d2efca98083b8c56860c43e82be8752813d1e2814d23c28d4f43",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_table_mutation_n1": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 14483,
            "sides": [
              {
                "calibrated_total_ns": 23851907.87,
                "ns_per_operation": 1646.89,
                "observed_batch": 14483,
                "requested_batch": 14483,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 29803016.019,
                "ns_per_operation": 2057.793,
                "observed_batch": 14483,
                "requested_batch": 14483,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 23851907.87,
            "ns_per_operation": 1646.89,
            "observed_batch": 14483,
            "requested_batch": 14483,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 29803016.019,
            "ns_per_operation": 2057.793,
            "observed_batch": 14483,
            "requested_batch": 14483,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 22545412.844,
            "ns_per_operation": 1632.188,
            "observed_batch": 13813,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22691616.808000002,
            "ns_per_operation": 1566.776,
            "observed_batch": 14483,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 14483,
        "source_sha256": "9b61bd9cbca804bf639085769fd766a2cf8a69d516fc7efb9e4edfe4ff6d18c9",
        "wasm_sha256": "458f2b111e48d2efca98083b8c56860c43e82be8752813d1e2814d23c28d4f43",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_table_mutation_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 3,
            "sides": [
              {
                "calibrated_total_ns": 30399009.0,
                "ns_per_operation": 10133003.0,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 27696102.999,
                "ns_per_operation": 9232034.333,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 30399009.0,
            "ns_per_operation": 10133003.0,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27696102.999,
            "ns_per_operation": 9232034.333,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 27581688.999,
            "ns_per_operation": 9193896.333,
            "observed_batch": 3,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 35313183.0,
            "ns_per_operation": 11771061.0,
            "observed_batch": 3,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 3,
        "source_sha256": "9b61bd9cbca804bf639085769fd766a2cf8a69d516fc7efb9e4edfe4ff6d18c9",
        "wasm_sha256": "458f2b111e48d2efca98083b8c56860c43e82be8752813d1e2814d23c28d4f43",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_table_mutation_n1024": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 16,
            "sides": [
              {
                "calibrated_total_ns": 14684600.0,
                "ns_per_operation": 917787.5,
                "observed_batch": 16,
                "requested_batch": 16,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 16276186.0,
                "ns_per_operation": 1017261.625,
                "observed_batch": 16,
                "requested_batch": 16,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 24,
            "sides": [
              {
                "calibrated_total_ns": 21651181.992,
                "ns_per_operation": 902132.583,
                "observed_batch": 24,
                "requested_batch": 24,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 22312972.008,
                "ns_per_operation": 929707.167,
                "observed_batch": 24,
                "requested_batch": 24,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 21651181.992,
            "ns_per_operation": 902132.583,
            "observed_batch": 24,
            "requested_batch": 24,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22312972.008,
            "ns_per_operation": 929707.167,
            "observed_batch": 24,
            "requested_batch": 24,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 23840963.008,
            "ns_per_operation": 1490060.188,
            "observed_batch": 16,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 14063267.008,
            "ns_per_operation": 878954.188,
            "observed_batch": 16,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 24,
        "source_sha256": "9b61bd9cbca804bf639085769fd766a2cf8a69d516fc7efb9e4edfe4ff6d18c9",
        "wasm_sha256": "458f2b111e48d2efca98083b8c56860c43e82be8752813d1e2814d23c28d4f43",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_table_mutation_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1736,
            "sides": [
              {
                "calibrated_total_ns": 32271863.288,
                "ns_per_operation": 18589.783,
                "observed_batch": 1736,
                "requested_batch": 1736,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 58478435.96,
                "ns_per_operation": 33685.735,
                "observed_batch": 1736,
                "requested_batch": 1736,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 32271863.288,
            "ns_per_operation": 18589.783,
            "observed_batch": 1736,
            "requested_batch": 1736,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 58478435.96,
            "ns_per_operation": 33685.735,
            "observed_batch": 1736,
            "requested_batch": 1736,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 32525444.279999997,
            "ns_per_operation": 18735.855,
            "observed_batch": 1736,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 30381267.327999998,
            "ns_per_operation": 18335.104,
            "observed_batch": 1657,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1736,
        "source_sha256": "9b61bd9cbca804bf639085769fd766a2cf8a69d516fc7efb9e4edfe4ff6d18c9",
        "wasm_sha256": "458f2b111e48d2efca98083b8c56860c43e82be8752813d1e2814d23c28d4f43",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_table_mutation_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 950,
            "sides": [
              {
                "calibrated_total_ns": 25639233.650000002,
                "ns_per_operation": 26988.667,
                "observed_batch": 950,
                "requested_batch": 950,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 27195514.15,
                "ns_per_operation": 28626.857,
                "observed_batch": 950,
                "requested_batch": 950,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 25639233.650000002,
            "ns_per_operation": 26988.667,
            "observed_batch": 950,
            "requested_batch": 950,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27195514.15,
            "ns_per_operation": 28626.857,
            "observed_batch": 950,
            "requested_batch": 950,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 27881970.180999998,
            "ns_per_operation": 29380.369,
            "observed_batch": 949,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27136383.3,
            "ns_per_operation": 28564.614,
            "observed_batch": 950,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 950,
        "source_sha256": "9b61bd9cbca804bf639085769fd766a2cf8a69d516fc7efb9e4edfe4ff6d18c9",
        "wasm_sha256": "458f2b111e48d2efca98083b8c56860c43e82be8752813d1e2814d23c28d4f43",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_tail_calls_n0": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 9320,
            "sides": [
              {
                "calibrated_total_ns": 7909958.56,
                "ns_per_operation": 848.708,
                "observed_batch": 9320,
                "requested_batch": 9320,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 8417618.96,
                "ns_per_operation": 903.178,
                "observed_batch": 9320,
                "requested_batch": 9320,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 25922,
            "sides": [
              {
                "calibrated_total_ns": 21088687.568,
                "ns_per_operation": 813.544,
                "observed_batch": 25922,
                "requested_batch": 25922,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 22979749.312,
                "ns_per_operation": 886.496,
                "observed_batch": 25922,
                "requested_batch": 25922,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 21088687.568,
            "ns_per_operation": 813.544,
            "observed_batch": 25922,
            "requested_batch": 25922,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22979749.312,
            "ns_per_operation": 886.496,
            "observed_batch": 25922,
            "requested_batch": 25922,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 7374847.362,
            "ns_per_operation": 843.611,
            "observed_batch": 8742,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 7909669.640000001,
            "ns_per_operation": 848.677,
            "observed_batch": 9320,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 25922,
        "source_sha256": "9953f19384c51b579921e6f789b13a8b57c86ba86d760b77846e3e33825ece02",
        "wasm_sha256": "3e23be294e7a9eda62c89dbc366b6a6cab90edfbda532da27a7e4fc6d0aa4a56",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_tail_calls_n1": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 7568,
            "sides": [
              {
                "calibrated_total_ns": 16189049.36,
                "ns_per_operation": 2139.145,
                "observed_batch": 7568,
                "requested_batch": 7568,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 12573982.256000001,
                "ns_per_operation": 1661.467,
                "observed_batch": 7568,
                "requested_batch": 7568,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 13242,
            "sides": [
              {
                "calibrated_total_ns": 25507283.322,
                "ns_per_operation": 1926.241,
                "observed_batch": 13242,
                "requested_batch": 13242,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 25912938.75,
                "ns_per_operation": 1956.875,
                "observed_batch": 13242,
                "requested_batch": 13242,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 25507283.322,
            "ns_per_operation": 1926.241,
            "observed_batch": 13242,
            "requested_batch": 13242,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 25912938.75,
            "ns_per_operation": 1956.875,
            "observed_batch": 13242,
            "requested_batch": 13242,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 7464548.766,
            "ns_per_operation": 1828.202,
            "observed_batch": 4083,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 17639744.144,
            "ns_per_operation": 2330.833,
            "observed_batch": 7568,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 13242,
        "source_sha256": "9953f19384c51b579921e6f789b13a8b57c86ba86d760b77846e3e33825ece02",
        "wasm_sha256": "3e23be294e7a9eda62c89dbc366b6a6cab90edfbda532da27a7e4fc6d0aa4a56",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_tail_calls_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 4,
            "sides": [
              {
                "calibrated_total_ns": 27208159.0,
                "ns_per_operation": 6802039.75,
                "observed_batch": 4,
                "requested_batch": 4,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 27969892.0,
                "ns_per_operation": 6992473.0,
                "observed_batch": 4,
                "requested_batch": 4,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 27208159.0,
            "ns_per_operation": 6802039.75,
            "observed_batch": 4,
            "requested_batch": 4,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27969892.0,
            "ns_per_operation": 6992473.0,
            "observed_batch": 4,
            "requested_batch": 4,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 27660621.0,
            "ns_per_operation": 6915155.25,
            "observed_batch": 4,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 20452065.998999998,
            "ns_per_operation": 6817355.333,
            "observed_batch": 3,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 4,
        "source_sha256": "9953f19384c51b579921e6f789b13a8b57c86ba86d760b77846e3e33825ece02",
        "wasm_sha256": "3e23be294e7a9eda62c89dbc366b6a6cab90edfbda532da27a7e4fc6d0aa4a56",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_tail_calls_n1024": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 23,
            "sides": [
              {
                "calibrated_total_ns": 17291646.008,
                "ns_per_operation": 751810.696,
                "observed_batch": 23,
                "requested_batch": 23,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 15866495.007000001,
                "ns_per_operation": 689847.609,
                "observed_batch": 23,
                "requested_batch": 23,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 32,
            "sides": [
              {
                "calibrated_total_ns": 24476590.016,
                "ns_per_operation": 764893.438,
                "observed_batch": 32,
                "requested_batch": 32,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 23834012.0,
                "ns_per_operation": 744812.875,
                "observed_batch": 32,
                "requested_batch": 32,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 24476590.016,
            "ns_per_operation": 764893.438,
            "observed_batch": 32,
            "requested_batch": 32,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 23834012.0,
            "ns_per_operation": 744812.875,
            "observed_batch": 32,
            "requested_batch": 32,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 18447892.003000002,
            "ns_per_operation": 802082.261,
            "observed_batch": 23,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 6888905.001,
            "ns_per_operation": 626264.091,
            "observed_batch": 11,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 32,
        "source_sha256": "9953f19384c51b579921e6f789b13a8b57c86ba86d760b77846e3e33825ece02",
        "wasm_sha256": "3e23be294e7a9eda62c89dbc366b6a6cab90edfbda532da27a7e4fc6d0aa4a56",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_tail_calls_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1994,
            "sides": [
              {
                "calibrated_total_ns": 27308212.847999997,
                "ns_per_operation": 13695.192,
                "observed_batch": 1994,
                "requested_batch": 1994,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 23945710.708,
                "ns_per_operation": 12008.882,
                "observed_batch": 1994,
                "requested_batch": 1994,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 27308212.847999997,
            "ns_per_operation": 13695.192,
            "observed_batch": 1994,
            "requested_batch": 1994,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 23945710.708,
            "ns_per_operation": 12008.882,
            "observed_batch": 1994,
            "requested_batch": 1994,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 23981549.937,
            "ns_per_operation": 12406.389,
            "observed_batch": 1933,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 25614305.86,
            "ns_per_operation": 12845.69,
            "observed_batch": 1994,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1994,
        "source_sha256": "9953f19384c51b579921e6f789b13a8b57c86ba86d760b77846e3e33825ece02",
        "wasm_sha256": "3e23be294e7a9eda62c89dbc366b6a6cab90edfbda532da27a7e4fc6d0aa4a56",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_tail_calls_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 902,
            "sides": [
              {
                "calibrated_total_ns": 34978342.300000004,
                "ns_per_operation": 38778.65,
                "observed_batch": 902,
                "requested_batch": 902,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 24940576.012000002,
                "ns_per_operation": 27650.306,
                "observed_batch": 902,
                "requested_batch": 902,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 34978342.300000004,
            "ns_per_operation": 38778.65,
            "observed_batch": 902,
            "requested_batch": 902,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 24940576.012000002,
            "ns_per_operation": 27650.306,
            "observed_batch": 902,
            "requested_batch": 902,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 15315209.136,
            "ns_per_operation": 31512.776,
            "observed_batch": 486,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 39407746.796000004,
            "ns_per_operation": 43689.298,
            "observed_batch": 902,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 902,
        "source_sha256": "9953f19384c51b579921e6f789b13a8b57c86ba86d760b77846e3e33825ece02",
        "wasm_sha256": "3e23be294e7a9eda62c89dbc366b6a6cab90edfbda532da27a7e4fc6d0aa4a56",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_try_table_normal_n0": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 27323,
            "sides": [
              {
                "calibrated_total_ns": 15673155.875,
                "ns_per_operation": 573.625,
                "observed_batch": 27323,
                "requested_batch": 27323,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 18059519.372,
                "ns_per_operation": 660.964,
                "observed_batch": 27323,
                "requested_batch": 27323,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 38353,
            "sides": [
              {
                "calibrated_total_ns": 29564486.755999997,
                "ns_per_operation": 770.852,
                "observed_batch": 38353,
                "requested_batch": 38353,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 25397126.481999997,
                "ns_per_operation": 662.194,
                "observed_batch": 38353,
                "requested_batch": 38353,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 29564486.755999997,
            "ns_per_operation": 770.852,
            "observed_batch": 38353,
            "requested_batch": 38353,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 25397126.481999997,
            "ns_per_operation": 662.194,
            "observed_batch": 38353,
            "requested_batch": 38353,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 15417931.731999999,
            "ns_per_operation": 564.284,
            "observed_batch": 27323,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 14922273.600000001,
            "ns_per_operation": 609.57,
            "observed_batch": 24480,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 38353,
        "source_sha256": "ab92800dd4e4d4053498c317effadedf52568270ab7b476eef74b09638de8964",
        "wasm_sha256": "97068a6310a3bc535369e2b6277b801875c14ca206a707fcb9bca9036f41d0df",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_try_table_normal_n1": {
        "attempt_count": 2,
        "attempts": [
          {
            "batch": 14674,
            "sides": [
              {
                "calibrated_total_ns": 20798472.706,
                "ns_per_operation": 1417.369,
                "observed_batch": 14674,
                "requested_batch": 14674,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 19959061.21,
                "ns_per_operation": 1360.165,
                "observed_batch": 14674,
                "requested_batch": 14674,
                "side": "candidate"
              }
            ]
          },
          {
            "batch": 16175,
            "sides": [
              {
                "calibrated_total_ns": 34592631.400000006,
                "ns_per_operation": 2138.648,
                "observed_batch": 16175,
                "requested_batch": 16175,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 26472732.875,
                "ns_per_operation": 1636.645,
                "observed_batch": 16175,
                "requested_batch": 16175,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 34592631.400000006,
            "ns_per_operation": 2138.648,
            "observed_batch": 16175,
            "requested_batch": 16175,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 26472732.875,
            "ns_per_operation": 1636.645,
            "observed_batch": 16175,
            "requested_batch": 16175,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 30782852.686000004,
            "ns_per_operation": 2133.106,
            "observed_batch": 14431,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 21143355.728,
            "ns_per_operation": 1440.872,
            "observed_batch": 14674,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 16175,
        "source_sha256": "ab92800dd4e4d4053498c317effadedf52568270ab7b476eef74b09638de8964",
        "wasm_sha256": "97068a6310a3bc535369e2b6277b801875c14ca206a707fcb9bca9036f41d0df",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_try_table_normal_n10000": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 3,
            "sides": [
              {
                "calibrated_total_ns": 23459025.998999998,
                "ns_per_operation": 7819675.333,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 21982539.0,
                "ns_per_operation": 7327513.0,
                "observed_batch": 3,
                "requested_batch": 3,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 23459025.998999998,
            "ns_per_operation": 7819675.333,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 21982539.0,
            "ns_per_operation": 7327513.0,
            "observed_batch": 3,
            "requested_batch": 3,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 22772496.998999998,
            "ns_per_operation": 7590832.333,
            "observed_batch": 3,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 22186064.001000002,
            "ns_per_operation": 7395354.667,
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
      "1:scalar_try_table_normal_n1024": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 30,
            "sides": [
              {
                "calibrated_total_ns": 22579314.0,
                "ns_per_operation": 752643.8,
                "observed_batch": 30,
                "requested_batch": 30,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 24625674.99,
                "ns_per_operation": 820855.833,
                "observed_batch": 30,
                "requested_batch": 30,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 22579314.0,
            "ns_per_operation": 752643.8,
            "observed_batch": 30,
            "requested_batch": 30,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 24625674.99,
            "ns_per_operation": 820855.833,
            "observed_batch": 30,
            "requested_batch": 30,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 20200656.007999998,
            "ns_per_operation": 776948.308,
            "observed_batch": 26,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 24425534.009999998,
            "ns_per_operation": 814184.467,
            "observed_batch": 30,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 30,
        "source_sha256": "ab92800dd4e4d4053498c317effadedf52568270ab7b476eef74b09638de8964",
        "wasm_sha256": "97068a6310a3bc535369e2b6277b801875c14ca206a707fcb9bca9036f41d0df",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_try_table_normal_n17": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 2058,
            "sides": [
              {
                "calibrated_total_ns": 31134243.084,
                "ns_per_operation": 15128.398,
                "observed_batch": 2058,
                "requested_batch": 2058,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 31071111.876,
                "ns_per_operation": 15097.722,
                "observed_batch": 2058,
                "requested_batch": 2058,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 31134243.084,
            "ns_per_operation": 15128.398,
            "observed_batch": 2058,
            "requested_batch": 2058,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 31071111.876,
            "ns_per_operation": 15097.722,
            "observed_batch": 2058,
            "requested_batch": 2058,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 37567981.206,
            "ns_per_operation": 18254.607,
            "observed_batch": 2058,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 3366300.0280000004,
            "ns_per_operation": 17352.062,
            "observed_batch": 194,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 2058,
        "source_sha256": "ab92800dd4e4d4053498c317effadedf52568270ab7b476eef74b09638de8964",
        "wasm_sha256": "97068a6310a3bc535369e2b6277b801875c14ca206a707fcb9bca9036f41d0df",
        "workload_kind": "typed_scalar_driver"
      },
      "1:scalar_try_table_normal_n32": {
        "attempt_count": 1,
        "attempts": [
          {
            "batch": 1157,
            "sides": [
              {
                "calibrated_total_ns": 54046134.571,
                "ns_per_operation": 46712.303,
                "observed_batch": 1157,
                "requested_batch": 1157,
                "side": "baseline"
              },
              {
                "calibrated_total_ns": 32743382.308,
                "ns_per_operation": 28300.244,
                "observed_batch": 1157,
                "requested_batch": 1157,
                "side": "candidate"
              }
            ]
          }
        ],
        "confirmed_sides": [
          {
            "calibrated_total_ns": 54046134.571,
            "ns_per_operation": 46712.303,
            "observed_batch": 1157,
            "requested_batch": 1157,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 32743382.308,
            "ns_per_operation": 28300.244,
            "observed_batch": 1157,
            "requested_batch": 1157,
            "side": "candidate"
          }
        ],
        "initial_estimates": [
          {
            "calibrated_total_ns": 31750854.135,
            "ns_per_operation": 27489.917,
            "observed_batch": 1155,
            "requested_batch": 0,
            "side": "baseline"
          },
          {
            "calibrated_total_ns": 27264626.324,
            "ns_per_operation": 23564.932,
            "observed_batch": 1157,
            "requested_batch": 0,
            "side": "candidate"
          }
        ],
        "paired_batch": 1157,
        "source_sha256": "ab92800dd4e4d4053498c317effadedf52568270ab7b476eef74b09638de8964",
        "wasm_sha256": "97068a6310a3bc535369e2b6277b801875c14ca206a707fcb9bca9036f41d0df",
        "workload_kind": "typed_scalar_driver"
      }
    },
    "warm_observed_under_20ms_legacy_count": 9,
    "warm_observed_under_20ms_total_count": 292,
    "warm_observed_under_20ms_typed_count": 283,
    "warm_timed_duration_summary": {
      "0:branch_calls:baseline": {
        "batch": 4,
        "calibration_class": "paired_shared_20ms_legacy",
        "estimated_total_ns": {
          "max": 48396984.0,
          "median": 26880668.5,
          "min": 24135960.0,
          "p95": 36210134.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:branch_calls:candidate": {
        "batch": 4,
        "calibration_class": "paired_shared_20ms_legacy",
        "estimated_total_ns": {
          "max": 41830370.0,
          "median": 27057107.5,
          "min": 23241179.0,
          "p95": 37670419.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:call_overhead:baseline": {
        "batch": 67785,
        "calibration_class": "paired_shared_20ms_legacy",
        "estimated_total_ns": {
          "max": 160129590.05999997,
          "median": 34668706.035,
          "min": 22522379.67,
          "p95": 156287875.185
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:call_overhead:candidate": {
        "batch": 67785,
        "calibration_class": "paired_shared_20ms_legacy",
        "estimated_total_ns": {
          "max": 189647381.73,
          "median": 34324697.16,
          "min": 22441579.95,
          "p95": 113891204.655
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:integer_loop:baseline": {
        "batch": 4,
        "calibration_class": "paired_shared_20ms_legacy",
        "estimated_total_ns": {
          "max": 44767263.0,
          "median": 29540821.5,
          "min": 24744887.0,
          "p95": 38741804.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:integer_loop:candidate": {
        "batch": 4,
        "calibration_class": "paired_shared_20ms_legacy",
        "estimated_total_ns": {
          "max": 43100244.0,
          "median": 28624896.5,
          "min": 25132299.0,
          "p95": 35771877.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_accumulator_n0:baseline": {
        "batch": 30653,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 50302799.12,
          "median": 23355118.4335,
          "min": 20951110.929,
          "p95": 30410442.811
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_accumulator_n0:candidate": {
        "batch": 30653,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 30068508.596,
          "median": 23048450.494999997,
          "min": 21053522.601999998,
          "p95": 27919334.807
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_accumulator_n10000:baseline": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 59265872.001,
          "median": 23945437.5,
          "min": 19796994.998999998,
          "p95": 50864253.0
        },
        "observations": 30,
        "observed_under_20ms_count": 1
      },
      "0:scalar_accumulator_n10000:candidate": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 33269577.999,
          "median": 22841553.998999998,
          "min": 19339053.998999998,
          "p95": 30723969.0
        },
        "observations": 30,
        "observed_under_20ms_count": 3
      },
      "0:scalar_accumulator_n1024:baseline": {
        "batch": 37,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 43776542.008,
          "median": 29695528.005999997,
          "min": 25122657.009999998,
          "p95": 41279559.008999996
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_accumulator_n1024:candidate": {
        "batch": 37,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 46062384.988,
          "median": 27482690.503,
          "min": 24649234.018,
          "p95": 44825893.014
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_accumulator_n17:baseline": {
        "batch": 2282,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 42875076.314,
          "median": 31014228.42,
          "min": 27897650.816,
          "p95": 40260461.122
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_accumulator_n17:candidate": {
        "batch": 2282,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 60426533.916,
          "median": 31222651.467,
          "min": 27413316.854,
          "p95": 44871974.644
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_accumulator_n1:baseline": {
        "batch": 17268,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 32342515.031999998,
          "median": 22341657.858,
          "min": 20574787.464,
          "p95": 29119753.656000003
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_accumulator_n1:candidate": {
        "batch": 17268,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 56359850.975999996,
          "median": 21627047.580000002,
          "min": 20316682.667999998,
          "p95": 30629149.536
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_accumulator_n32:baseline": {
        "batch": 1306,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 59424414.398,
          "median": 27220380.095,
          "min": 24283598.138,
          "p95": 31966328.59
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_accumulator_n32:candidate": {
        "batch": 1306,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 34774717.034,
          "median": 26838611.481,
          "min": 24298463.03,
          "p95": 33706338.51
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_br_table_n0:baseline": {
        "batch": 29941,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 39187828.735,
          "median": 22999628.265,
          "min": 18233619.885,
          "p95": 38700179.668
        },
        "observations": 30,
        "observed_under_20ms_count": 7
      },
      "0:scalar_br_table_n0:candidate": {
        "batch": 29941,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 34896085.794999994,
          "median": 20382126.163000003,
          "min": 17767588.22,
          "p95": 32345861.119999997
        },
        "observations": 30,
        "observed_under_20ms_count": 12
      },
      "0:scalar_br_table_n10000:baseline": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 57624045.0,
          "median": 24375821.5,
          "min": 20334004.0,
          "p95": 46991855.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_br_table_n10000:candidate": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 55952667.0,
          "median": 23818782.0,
          "min": 20036351.0,
          "p95": 35051921.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_br_table_n1024:baseline": {
        "batch": 16,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 41392551.008,
          "median": 26716547.496,
          "min": 19273671.008,
          "p95": 40406268.0
        },
        "observations": 30,
        "observed_under_20ms_count": 1
      },
      "0:scalar_br_table_n1024:candidate": {
        "batch": 16,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 45155516.992,
          "median": 26838479.008,
          "min": 14389535.008,
          "p95": 42576398.0
        },
        "observations": 30,
        "observed_under_20ms_count": 2
      },
      "0:scalar_br_table_n17:baseline": {
        "batch": 1413,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 73404471.114,
          "median": 32667725.623500004,
          "min": 28408106.421,
          "p95": 64933481.007
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_br_table_n17:candidate": {
        "batch": 1413,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 70083018.207,
          "median": 33797603.519999996,
          "min": 27616402.521,
          "p95": 67583346.318
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_br_table_n1:baseline": {
        "batch": 13150,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 88468676.4,
          "median": 28074053.349999998,
          "min": 25546005.3,
          "p95": 49712496.7
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_br_table_n1:candidate": {
        "batch": 13150,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 40591525.2,
          "median": 27520918.325000003,
          "min": 25426682.2,
          "p95": 36778064.65
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_br_table_n32:baseline": {
        "batch": 767,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 70730478.884,
          "median": 42238861.808,
          "min": 32104127.25,
          "p95": 70512084.071
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_br_table_n32:candidate": {
        "batch": 767,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 69764963.944,
          "median": 40582073.9285,
          "min": 30313910.9,
          "p95": 63939641.896
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_branch_entropy_n0:baseline": {
        "batch": 24184,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 36384029.928,
          "median": 22540092.6,
          "min": 19300694.168,
          "p95": 34528246.504
        },
        "observations": 30,
        "observed_under_20ms_count": 3
      },
      "0:scalar_branch_entropy_n0:candidate": {
        "batch": 24184,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 40382273.912,
          "median": 22139387.904,
          "min": 19490683.672,
          "p95": 37648369.448
        },
        "observations": 30,
        "observed_under_20ms_count": 1
      },
      "0:scalar_branch_entropy_n10000:baseline": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 61870289.0,
          "median": 28426566.0,
          "min": 23579792.0,
          "p95": 50960363.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_branch_entropy_n10000:candidate": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 74752433.0,
          "median": 28617575.5,
          "min": 22849051.0,
          "p95": 32750635.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_branch_entropy_n1024:baseline": {
        "batch": 21,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 211474022.004,
          "median": 31818857.5005,
          "min": 27223815.003,
          "p95": 80846410.008
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_branch_entropy_n1024:candidate": {
        "batch": 21,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 44043406.995000005,
          "median": 31711557.003,
          "min": 27032495.007,
          "p95": 43395537.003
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_branch_entropy_n17:baseline": {
        "batch": 1420,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 87981386.66,
          "median": 36263610.68000001,
          "min": 31101713.3,
          "p95": 53132480.8
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_branch_entropy_n17:candidate": {
        "batch": 1420,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 46674508.24,
          "median": 35759736.46,
          "min": 30716937.32,
          "p95": 45045312.42
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_branch_entropy_n1:baseline": {
        "batch": 12691,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 44173754.065000005,
          "median": 32984994.0805,
          "min": 26634550.936,
          "p95": 41701699.557000004
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_branch_entropy_n1:candidate": {
        "batch": 12691,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 48661101.300000004,
          "median": 32315416.920499995,
          "min": 27552897.078,
          "p95": 42610146.719
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_branch_entropy_n32:baseline": {
        "batch": 773,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 56902347.323,
          "median": 38193989.9075,
          "min": 34609719.931,
          "p95": 51934543.007999994
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_branch_entropy_n32:candidate": {
        "batch": 773,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 51051147.786,
          "median": 38394230.533,
          "min": 33656851.334,
          "p95": 50096361.376
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_call_ref_n0:baseline": {
        "batch": 31705,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 38290699.190000005,
          "median": 24684276.505000003,
          "min": 21067465.220000003,
          "p95": 31581699.255
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_call_ref_n0:candidate": {
        "batch": 31705,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 169211106.84,
          "median": 24162126.86,
          "min": 21430709.405,
          "p95": 64178148.74
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_call_ref_n10000:baseline": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 115344755.00100002,
          "median": 32150087.5005,
          "min": 27283386.0,
          "p95": 103578702.99899998
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_call_ref_n10000:candidate": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 54140259.999,
          "median": 31654876.5,
          "min": 26832807.999,
          "p95": 53826999.999
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_call_ref_n1024:baseline": {
        "batch": 22,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 52653785.997999996,
          "median": 24831413.002,
          "min": 21341287.990000002,
          "p95": 46980909.997999996
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_call_ref_n1024:candidate": {
        "batch": 22,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 46816576.994,
          "median": 23558434.493,
          "min": 20912364.990000002,
          "p95": 36081368.004
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_call_ref_n17:baseline": {
        "batch": 1427,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 40823918.524,
          "median": 27656548.581,
          "min": 24717422.323,
          "p95": 36857711.870000005
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_call_ref_n17:candidate": {
        "batch": 1427,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 38512034.397,
          "median": 27555474.171,
          "min": 25363058.483999997,
          "p95": 35120256.604
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_call_ref_n1:baseline": {
        "batch": 12361,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 34457832.625,
          "median": 26088106.8175,
          "min": 22962411.650000002,
          "p95": 33526839.188
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_call_ref_n1:candidate": {
        "batch": 12361,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 41413070.661,
          "median": 25903285.145499997,
          "min": 22677490.599999998,
          "p95": 38314514.069
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_call_ref_n32:baseline": {
        "batch": 853,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 48214082.321,
          "median": 30824933.505000003,
          "min": 28210822.028,
          "p95": 36632329.811
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_call_ref_n32:candidate": {
        "batch": 853,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 62132280.307,
          "median": 30995269.927999996,
          "min": 27700177.843,
          "p95": 35731241.083
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_dependent_n0:baseline": {
        "batch": 27741,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 67507473.831,
          "median": 25256918.2845,
          "min": 19576324.362,
          "p95": 59998068.353999995
        },
        "observations": 30,
        "observed_under_20ms_count": 1
      },
      "0:scalar_dependent_n0:candidate": {
        "batch": 27741,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 83455108.94700001,
          "median": 24617391.141,
          "min": 20843422.277999997,
          "p95": 78679190.646
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_dependent_n10000:baseline": {
        "batch": 4,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 183830251.0,
          "median": 30686435.0,
          "min": 26449985.0,
          "p95": 52548794.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_dependent_n10000:candidate": {
        "batch": 4,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 70180402.0,
          "median": 29939639.0,
          "min": 24259961.0,
          "p95": 58280842.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_dependent_n1024:baseline": {
        "batch": 23,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 34434979.0,
          "median": 25122119.0005,
          "min": 16857534.003000002,
          "p95": 33675594.992
        },
        "observations": 30,
        "observed_under_20ms_count": 3
      },
      "0:scalar_dependent_n1024:candidate": {
        "batch": 23,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 45441844.993,
          "median": 22140262.4955,
          "min": 16128080.998999998,
          "p95": 36455161.989
        },
        "observations": 30,
        "observed_under_20ms_count": 4
      },
      "0:scalar_dependent_n17:baseline": {
        "batch": 2354,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 48022913.532000005,
          "median": 38075853.486,
          "min": 32049201.536,
          "p95": 44591247.624000005
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_dependent_n17:candidate": {
        "batch": 2354,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 94361452.96599999,
          "median": 36990646.539000005,
          "min": 32277530.12,
          "p95": 55237810.54
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_dependent_n1:baseline": {
        "batch": 16195,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 49302195.574999996,
          "median": 29184191.652500004,
          "min": 25776221.12,
          "p95": 40930821.93
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_dependent_n1:candidate": {
        "batch": 16195,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 41588614.245,
          "median": 28696503.52,
          "min": 25204489.035,
          "p95": 41579172.559999995
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_dependent_n32:baseline": {
        "batch": 1324,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 54395831.66,
          "median": 32690396.106,
          "min": 28455319.292,
          "p95": 49270460.836
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_dependent_n32:candidate": {
        "batch": 1324,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 68319627.348,
          "median": 32520783.1,
          "min": 28496789.62,
          "p95": 51240805.86
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_direct_calls_n0:baseline": {
        "batch": 26042,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 52057645.496,
          "median": 20224542.724999998,
          "min": 17984839.578,
          "p95": 37404879.817999996
        },
        "observations": 30,
        "observed_under_20ms_count": 14
      },
      "0:scalar_direct_calls_n0:candidate": {
        "batch": 26042,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 53188519.346,
          "median": 21358437.447,
          "min": 17588662.632,
          "p95": 52123010.916
        },
        "observations": 30,
        "observed_under_20ms_count": 13
      },
      "0:scalar_direct_calls_n10000:baseline": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 84873768.0,
          "median": 22153424.0,
          "min": 18293068.0,
          "p95": 64328365.0
        },
        "observations": 30,
        "observed_under_20ms_count": 6
      },
      "0:scalar_direct_calls_n10000:candidate": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 46621299.0,
          "median": 22061133.5,
          "min": 17457156.0,
          "p95": 45920091.0
        },
        "observations": 30,
        "observed_under_20ms_count": 9
      },
      "0:scalar_direct_calls_n1024:baseline": {
        "batch": 26,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 59462582.997999996,
          "median": 28812272.501999997,
          "min": 25597546.0,
          "p95": 47601159.008
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_direct_calls_n1024:candidate": {
        "batch": 26,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 61222293.99000001,
          "median": 31229511.001,
          "min": 25813387.002,
          "p95": 54262771.004
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_direct_calls_n17:baseline": {
        "batch": 1700,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 94521098.2,
          "median": 34585095.8,
          "min": 28546005.6,
          "p95": 49495984.5
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_direct_calls_n17:candidate": {
        "batch": 1700,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 53396024.199999996,
          "median": 37596873.599999994,
          "min": 29072786.299999997,
          "p95": 51618320.6
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_direct_calls_n1:baseline": {
        "batch": 13004,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 34183926.896000005,
          "median": 25896367.162,
          "min": 23196938.323999997,
          "p95": 32213261.724
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_direct_calls_n1:candidate": {
        "batch": 13004,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 57474129.908,
          "median": 25645435.476,
          "min": 23226470.408,
          "p95": 33921181.076
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_direct_calls_n32:baseline": {
        "batch": 941,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 41947306.111,
          "median": 36292508.702,
          "min": 31106528.785,
          "p95": 41540943.73
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_direct_calls_n32:candidate": {
        "batch": 941,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 69566435.25899999,
          "median": 35126340.105500005,
          "min": 29865830.636,
          "p95": 64405868.929000005
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_dynamic_api_n0:baseline": {
        "batch": 64875,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 36424717.5,
          "median": 23147302.6875,
          "min": 20490768.75,
          "p95": 34900025.25
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_dynamic_api_n0:candidate": {
        "batch": 64875,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 47680919.25,
          "median": 22708877.4375,
          "min": 20326635.0,
          "p95": 35739442.875
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_dynamic_api_n10000:baseline": {
        "batch": 66808,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 30057587.28,
          "median": 22821545.992,
          "min": 20226255.616,
          "p95": 29835250.256
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_dynamic_api_n10000:candidate": {
        "batch": 66808,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 54519804.136,
          "median": 22827224.672,
          "min": 20483199.184,
          "p95": 31952529.392
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_dynamic_api_n1024:baseline": {
        "batch": 52357,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 42320686.669999994,
          "median": 20831462.839500003,
          "min": 17249013.65,
          "p95": 30050771.362999998
        },
        "observations": 30,
        "observed_under_20ms_count": 14
      },
      "0:scalar_dynamic_api_n1024:candidate": {
        "batch": 52357,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 45802008.314,
          "median": 20265902.5255,
          "min": 17321789.88,
          "p95": 32785325.116
        },
        "observations": 30,
        "observed_under_20ms_count": 14
      },
      "0:scalar_dynamic_api_n17:baseline": {
        "batch": 72521,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 34945476.707,
          "median": 22602547.549000002,
          "min": 19516706.478,
          "p95": 33528053.762000002
        },
        "observations": 30,
        "observed_under_20ms_count": 1
      },
      "0:scalar_dynamic_api_n17:candidate": {
        "batch": 72521,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 37934864.848,
          "median": 23220897.8555,
          "min": 20367812.934,
          "p95": 29533161.956
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_dynamic_api_n1:baseline": {
        "batch": 67561,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 29416532.327,
          "median": 22374987.101999998,
          "min": 19710583.945,
          "p95": 28707547.193
        },
        "observations": 30,
        "observed_under_20ms_count": 1
      },
      "0:scalar_dynamic_api_n1:candidate": {
        "batch": 67561,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 44744974.69,
          "median": 23933686.933,
          "min": 20232019.743,
          "p95": 34027165.211
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_dynamic_api_n32:baseline": {
        "batch": 75420,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 32960727.18,
          "median": 26236279.98,
          "min": 22348454.4,
          "p95": 32949338.759999998
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_dynamic_api_n32:candidate": {
        "batch": 75420,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 46708586.46,
          "median": 25556029.29,
          "min": 22782722.759999998,
          "p95": 37107243.36
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_host_call_n0:baseline": {
        "batch": 36645,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 38992991.730000004,
          "median": 24528587.265,
          "min": 21230830.425,
          "p95": 37006392.99
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_host_call_n0:candidate": {
        "batch": 36645,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 60173691.794999994,
          "median": 24955153.387500003,
          "min": 21829609.725,
          "p95": 42193419.45
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_host_call_n10000:baseline": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 46440294.0,
          "median": 26301440.001,
          "min": 21758468.001000002,
          "p95": 38940191.001
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_host_call_n10000:candidate": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 42550302.999,
          "median": 25652444.5005,
          "min": 22521224.001000002,
          "p95": 29964969.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_host_call_n1024:baseline": {
        "batch": 37,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 128911815.99499999,
          "median": 30565734.484,
          "min": 27361773.985,
          "p95": 115011975.009
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_host_call_n1024:candidate": {
        "batch": 37,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 48727024.015,
          "median": 29536676.997500002,
          "min": 25487635.999,
          "p95": 45560999.986
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_host_call_n17:baseline": {
        "batch": 2126,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 50338388.386,
          "median": 31981302.133,
          "min": 28284335.89,
          "p95": 47881117.192
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_host_call_n17:candidate": {
        "batch": 2126,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 44197275.81,
          "median": 30386121.813,
          "min": 27347875.41,
          "p95": 42088764.286
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_host_call_n1:baseline": {
        "batch": 12587,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 39912873.52,
          "median": 20431426.085500002,
          "min": 17474179.664,
          "p95": 33153302.084
        },
        "observations": 30,
        "observed_under_20ms_count": 12
      },
      "0:scalar_host_call_n1:candidate": {
        "batch": 12587,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 30594742.593999997,
          "median": 19991485.2615,
          "min": 17609187.826,
          "p95": 29471113.691
        },
        "observations": 30,
        "observed_under_20ms_count": 15
      },
      "0:scalar_host_call_n32:baseline": {
        "batch": 1175,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 55498183.975,
          "median": 32276583.775,
          "min": 28452229.025000002,
          "p95": 50086535.825
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_host_call_n32:candidate": {
        "batch": 1175,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 43318931.8,
          "median": 31777759.85,
          "min": 27652666.200000003,
          "p95": 35524644.5
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_indirect_calls_n0:baseline": {
        "batch": 29740,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 26606296.2,
          "median": 22042470.15,
          "min": 19449038.060000002,
          "p95": 26407157.16
        },
        "observations": 30,
        "observed_under_20ms_count": 1
      },
      "0:scalar_indirect_calls_n0:candidate": {
        "batch": 29740,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 48310726.64,
          "median": 22403052.779999997,
          "min": 20116165.74,
          "p95": 44895147.12
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_indirect_calls_n10000:baseline": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 59147690.001,
          "median": 30509879.001,
          "min": 27765788.001,
          "p95": 56413338.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_indirect_calls_n10000:candidate": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 36487202.001,
          "median": 29928908.5005,
          "min": 26493630.0,
          "p95": 35546337.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_indirect_calls_n1024:baseline": {
        "batch": 26,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 78871890.994,
          "median": 31604528.501,
          "min": 24811132.007999998,
          "p95": 62990674.993999995
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_indirect_calls_n1024:candidate": {
        "batch": 26,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 88440625.988,
          "median": 29670415.501999997,
          "min": 24360985.012000002,
          "p95": 68808746.994
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_indirect_calls_n17:baseline": {
        "batch": 1530,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 35806377.33,
          "median": 28199892.825,
          "min": 25219566.810000002,
          "p95": 34688882.160000004
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_indirect_calls_n17:candidate": {
        "batch": 1530,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 50549631.75,
          "median": 28197826.56,
          "min": 25492523.4,
          "p95": 32186571.749999996
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_indirect_calls_n1:baseline": {
        "batch": 12821,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 40723265.374,
          "median": 25084453.173,
          "min": 22228152.330000002,
          "p95": 38491116.453
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_indirect_calls_n1:candidate": {
        "batch": 12821,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 43389623.102000006,
          "median": 25716348.979000002,
          "min": 21557088.369,
          "p95": 38575324.781
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_indirect_calls_n32:baseline": {
        "batch": 858,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 48706267.896,
          "median": 31686600.231,
          "min": 27406381.86,
          "p95": 47253274.067999996
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_indirect_calls_n32:candidate": {
        "batch": 858,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 60641462.31,
          "median": 32430514.974,
          "min": 26967266.04,
          "p95": 49872800.406
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_multivalue_branch_n0:baseline": {
        "batch": 33333,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 160466261.988,
          "median": 25917374.156999998,
          "min": 22118678.811,
          "p95": 36830431.692
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_multivalue_branch_n0:candidate": {
        "batch": 33333,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 80126698.725,
          "median": 25396646.031000003,
          "min": 22799372.004,
          "p95": 79482771.831
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_multivalue_branch_n10000:baseline": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 39887511.0,
          "median": 31609122.5,
          "min": 28209430.0,
          "p95": 39278470.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_multivalue_branch_n10000:candidate": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 46636332.0,
          "median": 30883066.0,
          "min": 26984961.0,
          "p95": 43493540.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_multivalue_branch_n1024:baseline": {
        "batch": 13,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 39484992.001,
          "median": 21941316.0005,
          "min": 19488298.998999998,
          "p95": 31918124.004
        },
        "observations": 30,
        "observed_under_20ms_count": 2
      },
      "0:scalar_multivalue_branch_n1024:candidate": {
        "batch": 13,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 116630520.006,
          "median": 23026272.503,
          "min": 19356290.005,
          "p95": 32226371.996
        },
        "observations": 30,
        "observed_under_20ms_count": 3
      },
      "0:scalar_multivalue_branch_n17:baseline": {
        "batch": 1229,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 116967266.735,
          "median": 34805938.1295,
          "min": 29568434.801999997,
          "p95": 51133304.522
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_multivalue_branch_n17:candidate": {
        "batch": 1229,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 100867734.206,
          "median": 37847968.7615,
          "min": 30307908.125,
          "p95": 80235666.883
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_multivalue_branch_n1:baseline": {
        "batch": 11717,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 59943539.282,
          "median": 30638789.1585,
          "min": 22715478.408999998,
          "p95": 44363081.155
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_multivalue_branch_n1:candidate": {
        "batch": 11717,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 58602223.99,
          "median": 25925514.597,
          "min": 22666981.746,
          "p95": 56089489.906
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_multivalue_branch_n32:baseline": {
        "batch": 668,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 72872248.924,
          "median": 33039559.892,
          "min": 30113270.327999998,
          "p95": 41384889.236
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_multivalue_branch_n32:candidate": {
        "batch": 668,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 48511658.324,
          "median": 32536366.18,
          "min": 28469596.876,
          "p95": 42654787.964
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_ordinary_recursion_n0:baseline": {
        "batch": 22812,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 66668822.796,
          "median": 22196030.376,
          "min": 19014212.616,
          "p95": 60647253.216000006
        },
        "observations": 30,
        "observed_under_20ms_count": 5
      },
      "0:scalar_ordinary_recursion_n0:candidate": {
        "batch": 22812,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 139315849.56,
          "median": 23313065.58,
          "min": 19203734.712,
          "p95": 77705199.96
        },
        "observations": 30,
        "observed_under_20ms_count": 5
      },
      "0:scalar_ordinary_recursion_n17:baseline": {
        "batch": 1734,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 174241076.70000002,
          "median": 31023901.748999998,
          "min": 27538729.080000002,
          "p95": 112097903.20199999
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_ordinary_recursion_n17:candidate": {
        "batch": 1734,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 283358223.49799997,
          "median": 28152687.327,
          "min": 24283450.998,
          "p95": 187235511.438
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_ordinary_recursion_n1:baseline": {
        "batch": 14772,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 632367752.88,
          "median": 32422169.093999997,
          "min": 22413481.740000002,
          "p95": 540631978.416
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_ordinary_recursion_n1:candidate": {
        "batch": 14772,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 2383139402.2559996,
          "median": 27377885.621999998,
          "min": 22075867.68,
          "p95": 239077803.29999998
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_ordinary_recursion_n32:baseline": {
        "batch": 1020,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 58169125.08,
          "median": 34026972.03,
          "min": 30734772.6,
          "p95": 41125229.04
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_ordinary_recursion_n32:candidate": {
        "batch": 1020,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 80800291.44,
          "median": 35026413.93000001,
          "min": 30966254.46,
          "p95": 50623419.06
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_table_mutation_n0:baseline": {
        "batch": 31234,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 61590449.536,
          "median": 24335705.611,
          "min": 21279911.604000002,
          "p95": 60489919.546000004
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_table_mutation_n0:candidate": {
        "batch": 31234,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 55381567.611999996,
          "median": 24098655.167999998,
          "min": 21547305.878,
          "p95": 54604122.118
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_table_mutation_n10000:baseline": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 40227258.999,
          "median": 28636267.9995,
          "min": 26304786.999,
          "p95": 38654190.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_table_mutation_n10000:candidate": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 37777481.001,
          "median": 28610941.5,
          "min": 25510668.0,
          "p95": 32691699.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_table_mutation_n1024:baseline": {
        "batch": 22,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 29121081.0,
          "median": 22224786.001000002,
          "min": 20364946.998,
          "p95": 28146687.008
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_table_mutation_n1024:candidate": {
        "batch": 22,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 39963858.0,
          "median": 22637458.998000003,
          "min": 19346173.990000002,
          "p95": 27775593.01
        },
        "observations": 30,
        "observed_under_20ms_count": 1
      },
      "0:scalar_table_mutation_n17:baseline": {
        "batch": 1685,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 571685221.1550001,
          "median": 32005933.015,
          "min": 27799515.865,
          "p95": 477973172.74
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_table_mutation_n17:candidate": {
        "batch": 1685,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 50423867.64,
          "median": 32470005.604999997,
          "min": 27482885.83,
          "p95": 46332187.195
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_table_mutation_n1:baseline": {
        "batch": 14483,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 88531914.12799999,
          "median": 27792688.721,
          "min": 23856991.403,
          "p95": 60354006.573
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_table_mutation_n1:candidate": {
        "batch": 14483,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 96748786.246,
          "median": 26876103.1,
          "min": 23697301.845,
          "p95": 68573717.359
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_table_mutation_n32:baseline": {
        "batch": 959,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 36604855.462,
          "median": 32332132.136499997,
          "min": 28090860.175,
          "p95": 35762695.227
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_table_mutation_n32:candidate": {
        "batch": 959,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 60500256.544,
          "median": 32446644.887999997,
          "min": 29372767.942,
          "p95": 55956695.794999994
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_tail_calls_n0:baseline": {
        "batch": 24033,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 45638378.604,
          "median": 23396401.8795,
          "min": 20708899.638,
          "p95": 37291621.572
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_tail_calls_n0:candidate": {
        "batch": 24033,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 45050267.061000004,
          "median": 24994247.901,
          "min": 20420647.836,
          "p95": 44902055.55
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_tail_calls_n10000:baseline": {
        "batch": 4,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 62620183.0,
          "median": 29090321.5,
          "min": 25407963.0,
          "p95": 39790398.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_tail_calls_n10000:candidate": {
        "batch": 4,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 73320884.0,
          "median": 29782621.0,
          "min": 24424058.0,
          "p95": 39807045.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_tail_calls_n1024:baseline": {
        "batch": 43,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 43089513.014,
          "median": 29938987.489,
          "min": 27348894.013,
          "p95": 41563183.982
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_tail_calls_n1024:candidate": {
        "batch": 43,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 55227398.988,
          "median": 30561706.501500003,
          "min": 27143487.012,
          "p95": 38191117.016
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_tail_calls_n17:baseline": {
        "batch": 1997,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 49834088.572,
          "median": 29022241.240000002,
          "min": 22809096.957000002,
          "p95": 41992159.137
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_tail_calls_n17:candidate": {
        "batch": 1997,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 55743592.999000005,
          "median": 29018547.7885,
          "min": 24056954.359,
          "p95": 39056569.137
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_tail_calls_n1:baseline": {
        "batch": 16182,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 50019694.74,
          "median": 26608904.064,
          "min": 21934717.182,
          "p95": 46604710.188
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_tail_calls_n1:candidate": {
        "batch": 16182,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 31183960.014,
          "median": 24776413.928999998,
          "min": 22937370.084,
          "p95": 29798052.624
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_tail_calls_n32:baseline": {
        "batch": 1213,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 69374223.50999999,
          "median": 27323326.575499997,
          "min": 24410870.514000002,
          "p95": 42809604.780999996
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_tail_calls_n32:candidate": {
        "batch": 1213,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 47846253.706,
          "median": 26979445.321,
          "min": 24133664.836999997,
          "p95": 45764380.592999995
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_try_table_normal_n0:baseline": {
        "batch": 36011,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 35572205.965,
          "median": 24856880.838,
          "min": 21882480.270999998,
          "p95": 28033267.104000002
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_try_table_normal_n0:candidate": {
        "batch": 36011,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 40502147.875999995,
          "median": 23458357.641999997,
          "min": 21487799.711,
          "p95": 31103961.085
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_try_table_normal_n10000:baseline": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 41294028.999,
          "median": 24289061.498999998,
          "min": 19367771.001000002,
          "p95": 33109965.0
        },
        "observations": 30,
        "observed_under_20ms_count": 1
      },
      "0:scalar_try_table_normal_n10000:candidate": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 33394365.999,
          "median": 23186513.001000002,
          "min": 20120208.0,
          "p95": 29186385.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_try_table_normal_n1024:baseline": {
        "batch": 30,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 49585460.01,
          "median": 23285454.0,
          "min": 21489834.0,
          "p95": 40602035.01
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_try_table_normal_n1024:candidate": {
        "batch": 30,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 58528598.01,
          "median": 23933314.5,
          "min": 21032742.0,
          "p95": 48577934.01
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_try_table_normal_n17:baseline": {
        "batch": 2072,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 39881549.344,
          "median": 31152097.312,
          "min": 28559627.487999998,
          "p95": 36878858.744
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_try_table_normal_n17:candidate": {
        "batch": 2072,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 41811362.488,
          "median": 30850601.628,
          "min": 28894168.464,
          "p95": 37348788.344
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_try_table_normal_n1:baseline": {
        "batch": 15095,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 35078576.129999995,
          "median": 22993617.247500002,
          "min": 20732484.365000002,
          "p95": 33495714.430000003
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_try_table_normal_n1:candidate": {
        "batch": 15095,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 28521700.6,
          "median": 22172600.197499998,
          "min": 20321387.134999998,
          "p95": 24701956.135
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_try_table_normal_n32:baseline": {
        "batch": 1144,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 52030974.424,
          "median": 33332919.048,
          "min": 27735464.328,
          "p95": 42277206.4
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "0:scalar_try_table_normal_n32:candidate": {
        "batch": 1144,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 51975445.808000006,
          "median": 31764452.148000002,
          "min": 26646761.856,
          "p95": 46599619.352
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:branch_calls:baseline": {
        "batch": 4,
        "calibration_class": "paired_shared_20ms_legacy",
        "estimated_total_ns": {
          "max": 32831148.0,
          "median": 26292486.0,
          "min": 22793904.0,
          "p95": 31589950.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:branch_calls:candidate": {
        "batch": 4,
        "calibration_class": "paired_shared_20ms_legacy",
        "estimated_total_ns": {
          "max": 34031016.0,
          "median": 27034549.0,
          "min": 23760885.0,
          "p95": 33706502.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:call_overhead:baseline": {
        "batch": 68307,
        "calibration_class": "paired_shared_20ms_legacy",
        "estimated_total_ns": {
          "max": 35464174.716,
          "median": 23778076.542,
          "min": 20614574.451,
          "p95": 30692042.775
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:call_overhead:candidate": {
        "batch": 68307,
        "calibration_class": "paired_shared_20ms_legacy",
        "estimated_total_ns": {
          "max": 44288141.283,
          "median": 24324908.230499998,
          "min": 21145319.841,
          "p95": 40069432.655999996
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:integer_loop:baseline": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms_legacy",
        "estimated_total_ns": {
          "max": 222438068.001,
          "median": 23160580.0005,
          "min": 19323035.001000002,
          "p95": 41243900.001
        },
        "observations": 30,
        "observed_under_20ms_count": 2
      },
      "1:integer_loop:candidate": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms_legacy",
        "estimated_total_ns": {
          "max": 152516777.00100002,
          "median": 21956407.0005,
          "min": 19461962.001000002,
          "p95": 27193440.999
        },
        "observations": 30,
        "observed_under_20ms_count": 7
      },
      "1:scalar_accumulator_n0:baseline": {
        "batch": 32273,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 30441507.25,
          "median": 23061011.016500004,
          "min": 21327805.688,
          "p95": 30176449.101
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_accumulator_n0:candidate": {
        "batch": 32273,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 55901451.039000005,
          "median": 23482641.625,
          "min": 21360724.148,
          "p95": 33791218.739
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_accumulator_n10000:baseline": {
        "batch": 4,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 50604938.0,
          "median": 27606921.0,
          "min": 24103823.0,
          "p95": 35850986.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_accumulator_n10000:candidate": {
        "batch": 4,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 63567885.0,
          "median": 27759063.5,
          "min": 23064251.0,
          "p95": 54350414.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_accumulator_n1024:baseline": {
        "batch": 37,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 79223410.99,
          "median": 27844430.512,
          "min": 23292641.006,
          "p95": 45271301.012
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_accumulator_n1024:candidate": {
        "batch": 37,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 41412719.012,
          "median": 26224579.0035,
          "min": 22826324.012,
          "p95": 34338589.001
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_accumulator_n17:baseline": {
        "batch": 2227,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 420358476.22999996,
          "median": 33193913.805,
          "min": 24668447.822,
          "p95": 60919144.054
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_accumulator_n17:candidate": {
        "batch": 2227,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 90898775.311,
          "median": 33152688.6945,
          "min": 24026221.108,
          "p95": 80396637.49000001
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_accumulator_n1:baseline": {
        "batch": 18828,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 29405306.808000002,
          "median": 22266614.123999998,
          "min": 20224453.932,
          "p95": 27290432.88
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_accumulator_n1:candidate": {
        "batch": 18828,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 28224621.756,
          "median": 23181654.924000002,
          "min": 20197492.236,
          "p95": 27826880.256
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_accumulator_n32:baseline": {
        "batch": 1325,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 116457729.45,
          "median": 36358787.05,
          "min": 26348367.0,
          "p95": 114800684.45
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_accumulator_n32:candidate": {
        "batch": 1325,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 372242023.2,
          "median": 33380814.4375,
          "min": 24184860.624999996,
          "p95": 366700340.55
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_br_table_n0:baseline": {
        "batch": 35726,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 142651345.728,
          "median": 24767549.663999997,
          "min": 22469617.618,
          "p95": 41086221.862
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_br_table_n0:candidate": {
        "batch": 35726,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 44304348.49,
          "median": 24314740.477,
          "min": 21774139.576,
          "p95": 33878108.376
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_br_table_n10000:baseline": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 88137189.0,
          "median": 28711532.0,
          "min": 18824882.0,
          "p95": 43456522.0
        },
        "observations": 30,
        "observed_under_20ms_count": 1
      },
      "1:scalar_br_table_n10000:candidate": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 191630209.0,
          "median": 28134816.5,
          "min": 22381587.0,
          "p95": 70857601.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_br_table_n1024:baseline": {
        "batch": 20,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 45112055.0,
          "median": 28079492.0,
          "min": 21653329.0,
          "p95": 43748391.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_br_table_n1024:candidate": {
        "batch": 20,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 45385157.0,
          "median": 28795190.5,
          "min": 21782317.0,
          "p95": 38046373.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_br_table_n17:baseline": {
        "batch": 1402,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 40372503.730000004,
          "median": 31513961.823,
          "min": 27898839.63,
          "p95": 38170882.844000004
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_br_table_n17:candidate": {
        "batch": 1402,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 68115920.444,
          "median": 30635424.46,
          "min": 26852716.3,
          "p95": 46307490.787999995
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_br_table_n1:baseline": {
        "batch": 13290,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 55185489.03,
          "median": 29674696.11,
          "min": 21972131.07,
          "p95": 47152640.91
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_br_table_n1:candidate": {
        "batch": 13290,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 146983306.67999998,
          "median": 29523980.865000002,
          "min": 22388865.6,
          "p95": 76292334.78
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_br_table_n32:baseline": {
        "batch": 755,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 85707249.67999999,
          "median": 30107407.555,
          "min": 26849929.855000004,
          "p95": 59395899.075
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_br_table_n32:candidate": {
        "batch": 755,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 71265371.1,
          "median": 29298330.3025,
          "min": 26054953.360000003,
          "p95": 46790653.125
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_branch_entropy_n0:baseline": {
        "batch": 30131,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 82660360.946,
          "median": 27388099.7425,
          "min": 20800905.719,
          "p95": 58048395.954
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_branch_entropy_n0:candidate": {
        "batch": 30131,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 83641245.52,
          "median": 25044179.1215,
          "min": 21566715.215,
          "p95": 65981797.861
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_branch_entropy_n10000:baseline": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 39410681.0,
          "median": 25404919.0,
          "min": 20550764.0,
          "p95": 34906460.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_branch_entropy_n10000:candidate": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 41873826.0,
          "median": 24742324.5,
          "min": 20117546.0,
          "p95": 32554253.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_branch_entropy_n1024:baseline": {
        "batch": 16,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 160112879.008,
          "median": 27567023.0,
          "min": 16551668.0,
          "p95": 93991696.992
        },
        "observations": 30,
        "observed_under_20ms_count": 6
      },
      "1:scalar_branch_entropy_n1024:candidate": {
        "batch": 16,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 73996535.008,
          "median": 26917978.0,
          "min": 16482590.0,
          "p95": 56516603.008
        },
        "observations": 30,
        "observed_under_20ms_count": 4
      },
      "1:scalar_branch_entropy_n17:baseline": {
        "batch": 1429,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 209238968.579,
          "median": 35412418.3965,
          "min": 26709982.020000003,
          "p95": 59945874.083000004
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_branch_entropy_n17:candidate": {
        "batch": 1429,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 57420869.66599999,
          "median": 32840384.875,
          "min": 25848353.608999997,
          "p95": 57220222.346999995
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_branch_entropy_n1:baseline": {
        "batch": 12331,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 34754578.232,
          "median": 23956438.6765,
          "min": 21408330.009,
          "p95": 33676318.599
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_branch_entropy_n1:candidate": {
        "batch": 12331,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 63488471.728,
          "median": 22706950.777500004,
          "min": 21182179.469,
          "p95": 33585426.798
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_branch_entropy_n32:baseline": {
        "batch": 764,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 83854762.852,
          "median": 32254487.824,
          "min": 27049261.088,
          "p95": 71265534.17999999
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_branch_entropy_n32:candidate": {
        "batch": 764,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 67455751.152,
          "median": 32488684.002,
          "min": 26124542.228000004,
          "p95": 43023954.064
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_call_ref_n0:baseline": {
        "batch": 28587,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 31173237.303000003,
          "median": 21374056.8015,
          "min": 19337447.454,
          "p95": 28517133.372
        },
        "observations": 30,
        "observed_under_20ms_count": 2
      },
      "1:scalar_call_ref_n0:candidate": {
        "batch": 28587,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 26285574.978,
          "median": 21439964.13,
          "min": 19540786.785,
          "p95": 25286259.219
        },
        "observations": 30,
        "observed_under_20ms_count": 3
      },
      "1:scalar_call_ref_n10000:baseline": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 65942298.999,
          "median": 30536532.999,
          "min": 27376343.001,
          "p95": 55086720.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_call_ref_n10000:candidate": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 46268654.001,
          "median": 31251607.5,
          "min": 26231243.001,
          "p95": 36876507.999
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_call_ref_n1024:baseline": {
        "batch": 16,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 36387850.0,
          "median": 19178588.0,
          "min": 16743860.0,
          "p95": 33476451.008
        },
        "observations": 30,
        "observed_under_20ms_count": 18
      },
      "1:scalar_call_ref_n1024:candidate": {
        "batch": 16,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 34532246.0,
          "median": 19155494.0,
          "min": 16100816.0,
          "p95": 30015300.992
        },
        "observations": 30,
        "observed_under_20ms_count": 19
      },
      "1:scalar_call_ref_n17:baseline": {
        "batch": 1531,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 48219008.817,
          "median": 32347740.3895,
          "min": 27527332.539,
          "p95": 44994852.952
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_call_ref_n17:candidate": {
        "batch": 1531,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 61826204.589999996,
          "median": 29852353.538000003,
          "min": 25332621.074,
          "p95": 60136975.74
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_call_ref_n1:baseline": {
        "batch": 12945,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 272045712.225,
          "median": 24816768.884999998,
          "min": 20443430.085,
          "p95": 178932741.465
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_call_ref_n1:candidate": {
        "batch": 12945,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 153403673.955,
          "median": 24197913.7425,
          "min": 20382433.245,
          "p95": 145658123.82
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_call_ref_n32:baseline": {
        "batch": 859,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 51572476.213,
          "median": 32241134.335500002,
          "min": 27174938.309,
          "p95": 44556157.341
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_call_ref_n32:candidate": {
        "batch": 859,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 48785662.886,
          "median": 30418287.802,
          "min": 26093905.707,
          "p95": 43142937.413
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_dependent_n0:baseline": {
        "batch": 31216,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 37864976.784,
          "median": 21953510.439999998,
          "min": 18890050.24,
          "p95": 36001006.992
        },
        "observations": 30,
        "observed_under_20ms_count": 4
      },
      "1:scalar_dependent_n0:candidate": {
        "batch": 31216,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 210456242.96,
          "median": 21996307.576,
          "min": 18577515.648000002,
          "p95": 180528464.84800002
        },
        "observations": 30,
        "observed_under_20ms_count": 6
      },
      "1:scalar_dependent_n10000:baseline": {
        "batch": 4,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 115766036.0,
          "median": 30469241.5,
          "min": 24011198.0,
          "p95": 51220727.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_dependent_n10000:candidate": {
        "batch": 4,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 58401743.0,
          "median": 30019807.5,
          "min": 24223539.0,
          "p95": 57050520.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_dependent_n1024:baseline": {
        "batch": 29,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 189000424.986,
          "median": 31620678.003,
          "min": 21904449.998,
          "p95": 91482202.99599999
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_dependent_n1024:candidate": {
        "batch": 29,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 151289977.01099998,
          "median": 30570039.497,
          "min": 23210419.003999997,
          "p95": 82227283.994
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_dependent_n17:baseline": {
        "batch": 2374,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 52911799.838,
          "median": 26281854.857,
          "min": 23426762.57,
          "p95": 34052504.063999996
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_dependent_n17:candidate": {
        "batch": 2374,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 83044291.004,
          "median": 27180920.706,
          "min": 23449769.003999997,
          "p95": 45075603.304
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_dependent_n1:baseline": {
        "batch": 14663,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 34052456.757,
          "median": 24032605.6795,
          "min": 20659111.264000002,
          "p95": 33669561.838
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_dependent_n1:candidate": {
        "batch": 14663,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 50825799.706,
          "median": 22892366.8105,
          "min": 20869657.281,
          "p95": 31724822.811000004
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_dependent_n32:baseline": {
        "batch": 1317,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 54967669.82700001,
          "median": 31724406.3375,
          "min": 27806795.580000002,
          "p95": 40017677.16
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_dependent_n32:candidate": {
        "batch": 1317,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 101663644.58399999,
          "median": 29899550.724,
          "min": 27794403.926999997,
          "p95": 49094940.302999996
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_direct_calls_n0:baseline": {
        "batch": 33721,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 88877933.885,
          "median": 24710344.148000002,
          "min": 21800592.779,
          "p95": 49499190.784
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_direct_calls_n0:candidate": {
        "batch": 33721,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 63722776.02599999,
          "median": 24291242.6995,
          "min": 21866011.518999998,
          "p95": 60243679.293
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_direct_calls_n10000:baseline": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 38305286.001,
          "median": 30551692.5,
          "min": 27535299.999,
          "p95": 36024966.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_direct_calls_n10000:candidate": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 41151614.001,
          "median": 31211364.999,
          "min": 27365412.999,
          "p95": 40938111.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_direct_calls_n1024:baseline": {
        "batch": 26,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 40622291.996,
          "median": 25907349.987999998,
          "min": 23480626.987999998,
          "p95": 35742407.012
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_direct_calls_n1024:candidate": {
        "batch": 26,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 45317256.997999996,
          "median": 24928215.507,
          "min": 22660180.998,
          "p95": 35925664.996
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_direct_calls_n17:baseline": {
        "batch": 1688,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 60211878.272,
          "median": 28355337.968000002,
          "min": 25127341.808,
          "p95": 41903225.968
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_direct_calls_n17:candidate": {
        "batch": 1688,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 62015872.567999996,
          "median": 27957402.096,
          "min": 24141536.304,
          "p95": 51999584.408
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_direct_calls_n1:baseline": {
        "batch": 13150,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 37123120.65,
          "median": 25647063.05,
          "min": 22062741.25,
          "p95": 32477344.000000004
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_direct_calls_n1:candidate": {
        "batch": 13150,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 47213207.7,
          "median": 25452837.549999997,
          "min": 22884300.65,
          "p95": 38222513.25
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_direct_calls_n32:baseline": {
        "batch": 941,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 101359042.43900001,
          "median": 32115548.4995,
          "min": 28292429.704,
          "p95": 73779675.246
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_direct_calls_n32:candidate": {
        "batch": 941,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 72166152.897,
          "median": 31172108.016000003,
          "min": 28496982.402000003,
          "p95": 64168895.017
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_dynamic_api_n0:baseline": {
        "batch": 67698,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 53081257.122,
          "median": 25830713.483999997,
          "min": 21534801.498,
          "p95": 32578714.728
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_dynamic_api_n0:candidate": {
        "batch": 67698,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 37652882.922,
          "median": 24135386.319,
          "min": 22329576.018,
          "p95": 29996916.101999998
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_dynamic_api_n10000:baseline": {
        "batch": 68054,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 27612706.338,
          "median": 23835913.5,
          "min": 21778164.702,
          "p95": 27043638.79
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_dynamic_api_n10000:candidate": {
        "batch": 68054,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 55659801.357999995,
          "median": 23141014.106,
          "min": 21366982.434,
          "p95": 26998178.718
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_dynamic_api_n1024:baseline": {
        "batch": 68341,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 57886672.207,
          "median": 23991142.2205,
          "min": 21496114.821999997,
          "p95": 31324165.691
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_dynamic_api_n1024:candidate": {
        "batch": 68341,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 28121638.09,
          "median": 24428490.450000003,
          "min": 21550309.235,
          "p95": 28065871.834
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_dynamic_api_n17:baseline": {
        "batch": 65794,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 32892328.626,
          "median": 21141059.668,
          "min": 19611546.55,
          "p95": 25569785.396
        },
        "observations": 30,
        "observed_under_20ms_count": 2
      },
      "1:scalar_dynamic_api_n17:candidate": {
        "batch": 65794,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 35846873.99,
          "median": 21706295.922,
          "min": 19663786.986,
          "p95": 30570063.601999998
        },
        "observations": 30,
        "observed_under_20ms_count": 4
      },
      "1:scalar_dynamic_api_n1:baseline": {
        "batch": 62411,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 38195157.534,
          "median": 25166330.7905,
          "min": 20054464.219,
          "p95": 37843783.604
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_dynamic_api_n1:candidate": {
        "batch": 62411,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 52854939.735,
          "median": 23983923.189999998,
          "min": 18675118.708,
          "p95": 45525641.539
        },
        "observations": 30,
        "observed_under_20ms_count": 2
      },
      "1:scalar_dynamic_api_n32:baseline": {
        "batch": 65846,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 29624444.63,
          "median": 24073429.292,
          "min": 20386184.983999997,
          "p95": 28493539.580000002
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_dynamic_api_n32:candidate": {
        "batch": 65846,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 60736811.322000004,
          "median": 24684776.479000002,
          "min": 20951407.048,
          "p95": 40102057.688
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_host_call_n0:baseline": {
        "batch": 36650,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 45919041.55,
          "median": 25270065.049999997,
          "min": 20970287.05,
          "p95": 45698152.00000001
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_host_call_n0:candidate": {
        "batch": 36650,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 53160311.900000006,
          "median": 26805535.125,
          "min": 20140384.45,
          "p95": 47684728.6
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_host_call_n10000:baseline": {
        "batch": 4,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 39005678.0,
          "median": 26861782.0,
          "min": 24365322.0,
          "p95": 36894727.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_host_call_n10000:candidate": {
        "batch": 4,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 36626304.0,
          "median": 27140855.0,
          "min": 24486048.0,
          "p95": 34267770.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_host_call_n1024:baseline": {
        "batch": 36,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 46336442.004,
          "median": 25926347.987999998,
          "min": 22912823.987999998,
          "p95": 41646717.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_host_call_n1024:candidate": {
        "batch": 36,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 61854521.004,
          "median": 25252179.012000002,
          "min": 22218302.016,
          "p95": 45939364.992
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_host_call_n17:baseline": {
        "batch": 2079,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 52403286.474,
          "median": 29078507.303999998,
          "min": 23830955.378999997,
          "p95": 51593087.699999996
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_host_call_n17:candidate": {
        "batch": 2079,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 97580162.295,
          "median": 27525905.946000002,
          "min": 24392954.816999998,
          "p95": 51799293.315
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_host_call_n1:baseline": {
        "batch": 14610,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 36705550.38,
          "median": 22669825.65,
          "min": 19764758.64,
          "p95": 34739979.42
        },
        "observations": 30,
        "observed_under_20ms_count": 1
      },
      "1:scalar_host_call_n1:candidate": {
        "batch": 14610,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 44620868.52,
          "median": 22249430.205,
          "min": 19489608.51,
          "p95": 33877112.82
        },
        "observations": 30,
        "observed_under_20ms_count": 1
      },
      "1:scalar_host_call_n32:baseline": {
        "batch": 1168,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 52040145.392000005,
          "median": 27464593.776,
          "min": 25385079.568,
          "p95": 41796318.192
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_host_call_n32:candidate": {
        "batch": 1168,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 43579671.984,
          "median": 27213747.088,
          "min": 25092279.488,
          "p95": 35836703.312
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_indirect_calls_n0:baseline": {
        "batch": 24391,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 39004184.702,
          "median": 22438402.886,
          "min": 20187698.97,
          "p95": 29259614.336999997
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_indirect_calls_n0:candidate": {
        "batch": 24391,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 29489036.082999997,
          "median": 21803627.111,
          "min": 19062981.178,
          "p95": 28702255.596
        },
        "observations": 30,
        "observed_under_20ms_count": 6
      },
      "1:scalar_indirect_calls_n10000:baseline": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 95990251.0,
          "median": 26145789.5,
          "min": 20564599.0,
          "p95": 56780762.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_indirect_calls_n10000:candidate": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 51311486.0,
          "median": 24438458.0,
          "min": 20732946.0,
          "p95": 35956204.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_indirect_calls_n1024:baseline": {
        "batch": 24,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 43424503.992,
          "median": 26646004.5,
          "min": 22580866.008,
          "p95": 40496311.008
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_indirect_calls_n1024:candidate": {
        "batch": 24,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 83904897.0,
          "median": 26334084.0,
          "min": 22803925.992,
          "p95": 42620479.992
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_indirect_calls_n17:baseline": {
        "batch": 1507,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 135710711.90600002,
          "median": 37273420.668,
          "min": 28374088.358,
          "p95": 52312591.969000004
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_indirect_calls_n17:candidate": {
        "batch": 1507,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 67967654.579,
          "median": 37193857.8495,
          "min": 30567181.755,
          "p95": 54604235.631
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_indirect_calls_n1:baseline": {
        "batch": 13699,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 30337188.944999997,
          "median": 25452063.899499997,
          "min": 23418645.985000003,
          "p95": 29031139.983999997
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_indirect_calls_n1:candidate": {
        "batch": 13699,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 62355710.956,
          "median": 24430330.834,
          "min": 22123528.825999998,
          "p95": 29585963.183000002
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_indirect_calls_n32:baseline": {
        "batch": 869,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 70749417.75,
          "median": 40159578.86049999,
          "min": 27933541.823,
          "p95": 67041657.18799999
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_indirect_calls_n32:candidate": {
        "batch": 869,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 79206989.796,
          "median": 43415226.530499995,
          "min": 30370529.794,
          "p95": 74009439.966
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_multivalue_branch_n0:baseline": {
        "batch": 24876,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 126406448.71200001,
          "median": 20321813.862000003,
          "min": 16711846.056000002,
          "p95": 41529934.728
        },
        "observations": 30,
        "observed_under_20ms_count": 12
      },
      "1:scalar_multivalue_branch_n0:candidate": {
        "batch": 24876,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 44406346.607999995,
          "median": 21065904.774,
          "min": 16962372.252,
          "p95": 31059029.304
        },
        "observations": 30,
        "observed_under_20ms_count": 9
      },
      "1:scalar_multivalue_branch_n10000:baseline": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 338587580.0,
          "median": 28776800.5,
          "min": 24105074.0,
          "p95": 34772041.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_multivalue_branch_n10000:candidate": {
        "batch": 2,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 154695280.0,
          "median": 28903827.5,
          "min": 19702282.0,
          "p95": 50762967.0
        },
        "observations": 30,
        "observed_under_20ms_count": 1
      },
      "1:scalar_multivalue_branch_n1024:baseline": {
        "batch": 15,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 56194641.0,
          "median": 20984382.502499998,
          "min": 18305743.005000003,
          "p95": 34742943.0
        },
        "observations": 30,
        "observed_under_20ms_count": 8
      },
      "1:scalar_multivalue_branch_n1024:candidate": {
        "batch": 15,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 53934630.0,
          "median": 21086042.505,
          "min": 18611098.005000003,
          "p95": 29310808.995
        },
        "observations": 30,
        "observed_under_20ms_count": 10
      },
      "1:scalar_multivalue_branch_n17:baseline": {
        "batch": 1203,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 539889565.2179999,
          "median": 35104572.7755,
          "min": 25531290.654,
          "p95": 60174727.665
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_multivalue_branch_n17:candidate": {
        "batch": 1203,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 92424673.47000001,
          "median": 34885720.6095,
          "min": 25396546.233,
          "p95": 89664418.842
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_multivalue_branch_n1:baseline": {
        "batch": 11429,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 36463401.611999996,
          "median": 25070831.549500003,
          "min": 22510535.542,
          "p95": 31248543.205
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_multivalue_branch_n1:candidate": {
        "batch": 11429,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 66242003.98199999,
          "median": 24641084.006,
          "min": 22200626.778,
          "p95": 37949045.893
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_multivalue_branch_n32:baseline": {
        "batch": 673,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 56581410.986999996,
          "median": 29925153.629999995,
          "min": 26839707.062000003,
          "p95": 42277740.879
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_multivalue_branch_n32:candidate": {
        "batch": 673,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 71770208.676,
          "median": 28692879.5255,
          "min": 25563199.696,
          "p95": 69178534.805
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_ordinary_recursion_n0:baseline": {
        "batch": 29298,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 47573565.036,
          "median": 26588271.927,
          "min": 21411271.38,
          "p95": 38972844.155999996
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_ordinary_recursion_n0:candidate": {
        "batch": 29298,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 34565165.142,
          "median": 25301694.203999996,
          "min": 21671261.832,
          "p95": 34378800.563999996
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_ordinary_recursion_n17:baseline": {
        "batch": 1872,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 50987894.256,
          "median": 32193559.008,
          "min": 27915503.616,
          "p95": 43516246.176
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_ordinary_recursion_n17:candidate": {
        "batch": 1872,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 50865107.904,
          "median": 27458671.031999998,
          "min": 23805080.207999997,
          "p95": 50538112.56
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_ordinary_recursion_n1:baseline": {
        "batch": 15924,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 179784969.636,
          "median": 29851082.628,
          "min": 22559403.408,
          "p95": 97842900.10800001
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_ordinary_recursion_n1:candidate": {
        "batch": 15924,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 67676331.19199999,
          "median": 27089534.586000003,
          "min": 22919206.188,
          "p95": 37218607.86
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_ordinary_recursion_n32:baseline": {
        "batch": 1012,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 47191388.684,
          "median": 30848371.466,
          "min": 27277637.244000003,
          "p95": 44746599.084
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_ordinary_recursion_n32:candidate": {
        "batch": 1012,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 58102330.440000005,
          "median": 31466222.766000003,
          "min": 28534935.924000002,
          "p95": 43674381.036
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_table_mutation_n0:baseline": {
        "batch": 30856,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 36180264.511999995,
          "median": 25981708.536,
          "min": 21594571.6,
          "p95": 33691296.128
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_table_mutation_n0:candidate": {
        "batch": 30856,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 39788534.296,
          "median": 26125158.08,
          "min": 23186586.92,
          "p95": 36792725.256000005
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_table_mutation_n10000:baseline": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 50167809.0,
          "median": 28611569.001,
          "min": 23438546.001000002,
          "p95": 35496948.999
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_table_mutation_n10000:candidate": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 41252573.001,
          "median": 29124905.001,
          "min": 25394835.999,
          "p95": 39926901.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_table_mutation_n1024:baseline": {
        "batch": 24,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 32924583.0,
          "median": 21864061.008,
          "min": 19519348.008,
          "p95": 31361337.0
        },
        "observations": 30,
        "observed_under_20ms_count": 3
      },
      "1:scalar_table_mutation_n1024:candidate": {
        "batch": 24,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 30716791.992000002,
          "median": 21360369.0,
          "min": 19791478.992,
          "p95": 25868184.0
        },
        "observations": 30,
        "observed_under_20ms_count": 4
      },
      "1:scalar_table_mutation_n17:baseline": {
        "batch": 1736,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 37848181.728,
          "median": 33569519.816,
          "min": 30152446.856,
          "p95": 37417042.656
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_table_mutation_n17:candidate": {
        "batch": 1736,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 47275904.704,
          "median": 33678475.516,
          "min": 30049285.055999998,
          "p95": 43216577.712
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_table_mutation_n1:baseline": {
        "batch": 14483,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 28915454.33,
          "median": 23735435.584,
          "min": 20609033.823,
          "p95": 28885315.207000002
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_table_mutation_n1:candidate": {
        "batch": 14483,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 34017626.951,
          "median": 23477826.463,
          "min": 21184863.42,
          "p95": 32029067.602
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_table_mutation_n32:baseline": {
        "batch": 950,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 58483040.25,
          "median": 28395091.5,
          "min": 24522552.35,
          "p95": 38814183.25
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_table_mutation_n32:candidate": {
        "batch": 950,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 38478170.15,
          "median": 27933705.950000003,
          "min": 25253756.9,
          "p95": 34969439.2
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_tail_calls_n0:baseline": {
        "batch": 25922,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 31319478.84,
          "median": 21864610.794,
          "min": 19755882.016000003,
          "p95": 25576252.364
        },
        "observations": 30,
        "observed_under_20ms_count": 1
      },
      "1:scalar_tail_calls_n0:candidate": {
        "batch": 25922,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 41076286.342,
          "median": 22657448.125,
          "min": 20037628.233999997,
          "p95": 31211435.944
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_tail_calls_n10000:baseline": {
        "batch": 4,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 41096494.0,
          "median": 26550774.0,
          "min": 23409943.0,
          "p95": 33973429.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_tail_calls_n10000:candidate": {
        "batch": 4,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 33860733.0,
          "median": 27146582.0,
          "min": 23357536.0,
          "p95": 33571205.0
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_tail_calls_n1024:baseline": {
        "batch": 32,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 30410336.992,
          "median": 22358663.008,
          "min": 20839028.992,
          "p95": 28208062.016
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_tail_calls_n1024:candidate": {
        "batch": 32,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 33412804.992,
          "median": 22785210.0,
          "min": 20451012.992,
          "p95": 25768217.984
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_tail_calls_n17:baseline": {
        "batch": 1994,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 28721358.654,
          "median": 25579287.232,
          "min": 23543969.558,
          "p95": 28596869.246000003
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_tail_calls_n17:candidate": {
        "batch": 1994,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 54554294.65,
          "median": 25579387.928999998,
          "min": 22989256.704,
          "p95": 33089584.544000003
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_tail_calls_n1:baseline": {
        "batch": 13242,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 47407141.278000005,
          "median": 25603320.927,
          "min": 22501587.678,
          "p95": 36702652.769999996
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_tail_calls_n1:candidate": {
        "batch": 13242,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 41691245.22,
          "median": 25929398.556,
          "min": 22641701.279999997,
          "p95": 37570467.239999995
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_tail_calls_n32:baseline": {
        "batch": 902,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 85003773.734,
          "median": 24743825.007,
          "min": 22974086.124,
          "p95": 35345766.588
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_tail_calls_n32:candidate": {
        "batch": 902,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 39553942.054,
          "median": 24881170.292,
          "min": 22352341.132000003,
          "p95": 33960778.962000005
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_try_table_normal_n0:baseline": {
        "batch": 38353,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 65529667.329,
          "median": 26450012.2145,
          "min": 22923703.158999998,
          "p95": 52900101.135000005
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_try_table_normal_n0:candidate": {
        "batch": 38353,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 42032011.525,
          "median": 27341431.816999998,
          "min": 22664628.643999998,
          "p95": 40074167.581
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_try_table_normal_n10000:baseline": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 38331738.0,
          "median": 24866803.5,
          "min": 20279342.001000002,
          "p95": 35189048.001
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_try_table_normal_n10000:candidate": {
        "batch": 3,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 61249670.001,
          "median": 24089532.0,
          "min": 20305764.998999998,
          "p95": 49282766.001
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_try_table_normal_n1024:baseline": {
        "batch": 30,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 48346779.0,
          "median": 22688419.5,
          "min": 20380313.009999998,
          "p95": 31010417.01
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_try_table_normal_n1024:candidate": {
        "batch": 30,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 29713931.009999998,
          "median": 22592377.995,
          "min": 20260700.01,
          "p95": 26975708.009999998
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_try_table_normal_n17:baseline": {
        "batch": 2058,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 41939800.896,
          "median": 34789840.701000005,
          "min": 30973836.39,
          "p95": 41402913.972
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_try_table_normal_n17:candidate": {
        "batch": 2058,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 66574392.234,
          "median": 32959163.265,
          "min": 29849979.053999998,
          "p95": 45878928.144
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_try_table_normal_n1:baseline": {
        "batch": 16175,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 48858965.225,
          "median": 24065334.8375,
          "min": 20399230.65,
          "p95": 37617307.875
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_try_table_normal_n1:candidate": {
        "batch": 16175,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 41405137.025,
          "median": 22360473.6625,
          "min": 19760576.95,
          "p95": 31840099.3
        },
        "observations": 30,
        "observed_under_20ms_count": 1
      },
      "1:scalar_try_table_normal_n32:baseline": {
        "batch": 1157,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 49296177.51499999,
          "median": 30659830.097000003,
          "min": 26964888.119,
          "p95": 36162267.557000004
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      },
      "1:scalar_try_table_normal_n32:candidate": {
        "batch": 1157,
        "calibration_class": "paired_shared_20ms_typed",
        "estimated_total_ns": {
          "max": 47164688.48,
          "median": 28956979.480000004,
          "min": 26699539.425,
          "p95": 37408965.139
        },
        "observations": 30,
        "observed_under_20ms_count": 0
      }
    },
    "workload_count": 85
  },
  "outcome": "rejected",
  "raw_gate_decision": "inconclusive",
  "reason": "P032 candidate failed the predeclared strict incremental build gate.",
  "schema": "wtime-p032-completion-v1",
  "status": "done",
  "strict_build_gate_passed": false,
  "task": "P032",
  "task_index_update_performed": false,
  "development_provenance": {
    "path": "/workspace/wtime/target/performance/P032/development-provenance.json",
    "sha256": "80e19b68b3c79b7a34c15def5959de47ad99d985ba767a882cbfe552852a1a07",
    "bytes": 510053
  },
  "development_failures": [
    {
      "kind": "candidate preflight guard rejected live source against predecessor before work",
      "actual_helper_exits": [
        1,
        0
      ],
      "record": {
        "path": "/workspace/wtime/target/performance/P032-candidate-checks/guard-correction.json",
        "sha256": "0274cce31d302c405a097b113ee8a89048db351f1bcf9d30c17ba74b6d2c667c",
        "bytes": 1195
      },
      "correction": "Compared preflight to frozen predecessor identities and checked candidate changes separately; successful semantic commands and the quiet sample ran once."
    },
    {
      "kind": "supporting provenance copy referenced missing helper filename",
      "actual_python_exit": 1,
      "exception": "FileNotFoundError",
      "record": {
        "path": "/workspace/wtime/target/performance/P032-candidate-checks/guard-correction.json",
        "sha256": "0274cce31d302c405a097b113ee8a89048db351f1bcf9d30c17ba74b6d2c667c",
        "bytes": 1195
      },
      "correction": "Recovered the exact previously executed failed helper hash from its recorded identity and completed remaining supporting copies without restarting timing."
    },
    {
      "kind": "temporary synthetic test overwrote helper drafts",
      "placeholder_sha256": "7c4b4573f33189ed8a845a5f9f09f4f58e9ae6bdd4a1c86644d13e6c77e53d3d",
      "cause": "The test wrote absolute /tmp provenance input paths instead of confining fixture writes to its fresh temporary root.",
      "observed_scope": "Temporary helper drafts, tests and reviews only; measured repository sources, archived binaries, running comparison and durable executed evidence were intact.",
      "correction": "Stopped writes, retained placeholders/bytecode and inventory, restored retained executed copies byte-for-byte, and reconstructed unavailable staged sources with fresh review. Hash equality is claimed only for verified exact recoveries. No comparison or quiet build sample was retried.",
      "stderr_limit": "The original failed synthetic process output appeared in the tool transcript; reconstructed descriptions are not original captured stderr."
    }
  ]
}
```
