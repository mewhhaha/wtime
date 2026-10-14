# P038 — Lower constants, locals and common integer operations

Status: **done**

Outcome: **inconclusive**

Kind: `candidate` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P002 — Compare separately built baseline and candidate binaries](../00-measurement/P002-paired-driver.md)

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P008 — Add scaling, branch and call benchmark families](../00-measurement/P008-scalar-workloads.md)

[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)

[P016 — Add explicit executor policies and test integration hooks](../01-execution-contracts/P016-execution-policy.md)

[P023 — Fetch ordinary instructions without cloning their payload](../02-structural-wins/P023-borrow-instructions.md)

[P028 — Execute common scalar operations in place on the operand stack](../02-structural-wins/P028-in-place-numeric-stack.md)

[P036 — Define small immutable execution instructions and side tables](P036-compact-op-format.md)

[P037 — Map compact instructions to source events and control targets](P037-compact-source-maps.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/execution/compact/lower_scalar.rs` — admission and scalar leaf lowering.
- `crates/wtime-runtime/src/execution/compact/execute.rs` — direct execution for admitted scalar tags.
- `crates/wtime-runtime/tests/performance/p038.rs` — public compact-path, fallback, fuel, cursor, and lifecycle acceptance cases.
- `docs/benchmarks/performance/P038-contract.json` — immutable predeclared primary/protected scalar workload partition.

This stage admits only supported straight-line integer function bodies. A caller may contain a loop and directly call an admitted leaf; that caller loop remains decoded. Lowering loops or other structured control belongs to P040. One integrator owns `lib.rs`, dispatch, Cargo features and `tests/performance.rs`; leaf owners submit a bounded integration hunk.

## Input/output contract

For an explicitly requested `Compact` policy, admit eligible straight-line integer bodies into typed compact operations with bounded operands and verified P037 source events. Execute those tags with existing numeric, local, stack, fuel, cancellation, materialization, and result semantics. Decline an unsupported function body as a whole and execute it decoded. Default and Portable policy stay Decoded; no loop, block, branch, or other structured-control operation is lowered in P038.

## Implementation recipe

1. Prepare and validate the compact scalar harness support before taking the fresh `target/performance/P038-before` archive; freeze those Rust harness sources with the archive before any runtime-core patch.
2. Admit only straight-line integer leaf bodies; lower i32/i64 constants, numeric local.get/set/tee, drop/select and the supported common nontrapping integer operations into typed tags. Keep indices and operands in checked bounded fields.
3. Execute supported tags through direct safe local/operand access while preserving each original source event and existing checks. Decline the entire function before dispatch if any instruction or type shape is unsupported.
4. Preserve reference locals/selects and wide or unsupported immediates through decoded fallback. Leave loops and every structured-control instruction decoded; P040 owns their lowering.
5. Prove direct dispatch and fallback with named test counters, and compare exact results, each fuel prefix, cancellation, materialization cursor, and transient stack behavior against Decoded and Reference.
6. Predeclare the six `scalar_direct_calls` workloads as the primary family, including n=0 and n=17. Protect every other workload in the `scalar` selection. Keep the existing cold/default/optional-build gates unchanged; do not promote from counters or one favorable row.

## Meaningful cases

- All six `scalar_direct_calls` rows (n=0, 1, 17, 32, 1024, 10000) are primary. Prove the directly called straight-line integer leaf dispatches compact tags while its caller loop remains decoded.
- All remaining rows in the 85-workload `scalar` selection are protected controls, including the other scalar families and the three existing non-scalar scalar-selection controls. n=0 and n=17 in direct calls are part of the primary family, not held-out controls.
- Wide locals, reference local/select shapes, and any body containing unsupported or structured-control operations visibly fall back for the whole function.
- Every executed source event, wrapping boundary, early return, fuel prefix, cancellation boundary, materialization cursor, and transient stack peak matches Decoded and Reference; direct dispatch/fallback counters prove the tested tier.
- Cold/default preparation, first-use and optional-build budgets retain the existing P038 rules. The contract only classifies warm workload roles and does not claim a win.

## Verification

Activate Rust as described in EXECUTION.md. Focused tests must prove both direct compact dispatch and whole-function decoded fallback; decoded and reference results, fuel/cancellation/materialization behavior, and stack limits must agree.

```sh
cargo test --locked -p wtime-runtime --features compact-executor,execution-reference p038_ -- --nocapture
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml --features compact-executor -j 4
python3 tools/wtime-bench/compare.py --baseline target/performance/P038-before --candidate tools/wtime-bench/target/release/wtime-bench --candidate-mode compact --selection scalar --output target/performance/P038/comparison
python3 tools/wtime-bench/analyze.py --input target/performance/P038/comparison --output target/performance/P038/comparison/analysis.json
python3 tools/checks/build_budget.py --output target/performance/P038/build.json
python3 tools/checks/build_budget.py --features wtime-runtime/compact-executor --output target/performance/P038/optional-build.json
python3 tools/wtime-bench/gates.py --task P038 --comparison target/performance/P038/comparison --build target/performance/P038/build.json --optional-build target/performance/P038/optional-build.json --contract docs/benchmarks/performance/P038-contract.json --output target/performance/P038/decision.json
```

Require source-bound named cases and positive work counts. The contract file must have exactly the six primary direct-call IDs and partition all remaining selected `scalar` IDs as protected. Preserve cold/default and optional-build gates. Report prepared, first-call, and warm stages separately; a measured promotion requires both fresh runs to pass the existing primary-family and protected-control thresholds. No speed result is asserted by this predeclaration.

## Measurement and decision

Selection: `scalar`. Explicit execution setting: `compact`. Decision rule: `measured_promotion`.

The P038 primary family is exactly `scalar_direct_calls_n0`, `_n1`, `_n17`, `_n32`, `_n1024`, and `_n10000`. The family includes the two short rows n=0 and n=17. Every other workload in the registered 85-workload `scalar` selection is protected. The six direct-call fixtures exercise a decoded caller loop calling eligible straight-line integer leaves; P038 makes no claim that loops or structured control execute compactly. Predeclared roles are recorded in `docs/benchmarks/performance/P038-contract.json` and bound by the gate's contract hash.

Cold/default, first-use, and optional-build gate limits remain the existing P038 limits. Warm adoption still requires both-run primary-family improvement and protected-control bounds under [MEASUREMENT.md](../MEASUREMENT.md). No promotion or performance result is asserted by this plan update.

## Before editing

Run these commands before changing the candidate sources. The archive is immutable and captures the immediate predecessor. Retain P001's original archive for milestone totals. For native work, use the named portable control and verify its actual policy/quantum; if this archive already exists with different sources, choose a new recorded run-specific directory and update the comparison path rather than overwriting it.

```sh
python3 tools/wtime-bench/baseline.py --output target/performance/P038-before
```

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. Executed evidence is recorded below.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/P038/closure/artifacts/P037-source-map-evidence.json": "f7682a5d3a6fe71a0ad3519735cd7b45690e45204d0a918ccd2918955ce45e9c",
    "/workspace/wtime/target/performance/P038/closure/artifacts/P038-check-result.json": "a4e2b08a6ada93deeb586d75d711da7bd91a3356800633d10db4c0686a7afda7",
    "/workspace/wtime/target/performance/P038/closure/artifacts/P038-prepare-receipt.json": "902e63fb45919696f2a9706ef6e8058112e00fbfc1b8f58a162467ece47aaa7e",
    "/workspace/wtime/target/performance/P038/closure/artifacts/build-budgets/default-command.json": "ad8217f569d46193b8431ec8deeabf76c2810e215538ed609330137a3f6b4cc6",
    "/workspace/wtime/target/performance/P038/closure/artifacts/build-budgets/default.json": "a6c734f85c855911f55c9b8df7408700e9a9f1cf7af214c65148c9e2a8861146",
    "/workspace/wtime/target/performance/P038/closure/artifacts/build-budgets/optional-command.json": "2947ed70c2d25a7d1bacb5693a9717660353ee4b4c8615236362642ed0a4f30f",
    "/workspace/wtime/target/performance/P038/closure/artifacts/build-budgets/optional.json": "250c818d2de8ce3ed479e829f1f7a3b25796c771e07ae4d7380d9c640e281a6b",
    "/workspace/wtime/target/performance/P038/closure/artifacts/comparison-report.json": "2fba9d1c6954618e745416b006a8dadbea55cb816ef24c723ed90ce7afc7ecb1",
    "/workspace/wtime/target/performance/P038/closure/artifacts/coverage.json": "ee585fbb244ddfca425b4dcf481fb639447f32f2d067e67345916a0db0149c22",
    "/workspace/wtime/target/performance/P038/closure/artifacts/decision.json": "fd4a0c0f02f163946e2af95ffd84a0db524f34408d2901193e57f57617788932",
    "/workspace/wtime/target/performance/P038/closure/artifacts/default-runtime-suite.json": "b500e55535a8f266fbf2929cdf8624fe40eac13122de91fc8d1033ef8b607e66",
    "/workspace/wtime/target/performance/P038/closure/artifacts/dependency-budget.json": "6610be51974afd0a5277e268bf890f9d98fd8dbe1d09f722e846e90db9f41fb2",
    "/workspace/wtime/target/performance/P038/closure/artifacts/feature-focused-tests.json": "11aefbadf068aceaad57d846d8d9e9bb74fd23258e3c29f19119c7df1e7793ec",
    "/workspace/wtime/target/performance/P038/closure/artifacts/source-snapshots/workspace-metadata-after.json": "72eeaaa4792f03cb62af14a960a1b4261a2901bb23d5439c2185d3a6931664e6",
    "/workspace/wtime/target/performance/P038/closure/artifacts/source-snapshots/workspace-source-after.json": "660522c0117ab60765ddc67c8c06a0567ab9035441f5b5e64f9b697c772500d4",
    "/workspace/wtime/target/performance/P038/closure/artifacts/source-snapshots/workspace-source-before.json": "660522c0117ab60765ddc67c8c06a0567ab9035441f5b5e64f9b697c772500d4",
    "/workspace/wtime/target/performance/P038/closure/artifacts/test-binaries/default-runtime-suite/core-fc4384471282e053": "15671b035ee5e52f72d378fb7ad51c8eff21ac2135921aeaad7fb748c6689ba2",
    "/workspace/wtime/target/performance/P038/closure/artifacts/test-binaries/default-runtime-suite/exception_root_safety-863d9cca58697c87": "ec7f1279da9e5d8e097221235d09f80b1fc238e32a04af7227ffa23f32d5e631",
    "/workspace/wtime/target/performance/P038/closure/artifacts/test-binaries/default-runtime-suite/exceptions-dc82360c779184c7": "e3648bd61e1db0efe561946ed82d6c1f8d0ec36646ee8126e33ad1be2744ed50",
    "/workspace/wtime/target/performance/P038/closure/artifacts/test-binaries/default-runtime-suite/feature_profiles-da219ada3555dbdb": "8db36de184b3ef9fcba417678082a363101c75a79d802638127c1f0b547afdcd",
    "/workspace/wtime/target/performance/P038/closure/artifacts/test-binaries/default-runtime-suite/performance-f1f918c868572e47": "e995e4583f280f7740ff92ae5f7863622eee622a44312ebda38ac0a3ea299322",
    "/workspace/wtime/target/performance/P038/closure/artifacts/test-binaries/default-runtime-suite/proposals-ce7c5c5922b88848": "66a685a00b415084c808c6caed9bcc6ad2d621df30b1c3ef5f7f756b9a484146",
    "/workspace/wtime/target/performance/P038/closure/artifacts/test-binaries/default-runtime-suite/wtime_runtime-670f0c395211a4f7": "d39235cb27c4c589cdb9ae62d4b498c87083fc46b7ba81d5907af76b60b514f9",
    "/workspace/wtime/target/performance/P038/closure/artifacts/test-binaries/feature-focused-tests/core-42d92ac3afc77304": "3806521436faffb487a1956f033568e1c97a7dd4ecb78d63dacaaa273faf25ab",
    "/workspace/wtime/target/performance/P038/closure/artifacts/test-binaries/feature-focused-tests/exception_root_safety-db67ff33a61a98ad": "a7b672eb81bd30993f13e199ae6d9ef4403909f9407c6d778c90b2a0c760d43f",
    "/workspace/wtime/target/performance/P038/closure/artifacts/test-binaries/feature-focused-tests/exceptions-5d275ecd0f4f002b": "202343a6a0a40ec27671e884e182f14293abb479eff21c65143565e94c82b2e1",
    "/workspace/wtime/target/performance/P038/closure/artifacts/test-binaries/feature-focused-tests/feature_profiles-487957d4b265fccb": "3ca3e38435dc6a9ba97dd2a1670cac503403569ec948265cdd0a797ac8379358",
    "/workspace/wtime/target/performance/P038/closure/artifacts/test-binaries/feature-focused-tests/performance-1a202aeb20ffb000": "3877579a6e8ed13387568bc234f47a909660370c7ea80cbb791693575a7e3e90",
    "/workspace/wtime/target/performance/P038/closure/artifacts/test-binaries/feature-focused-tests/proposals-2b774c23db30396c": "6ebb27c78179432d07088c45ad5801c73fbfacabebdbf3a95825db7539a3d01b",
    "/workspace/wtime/target/performance/P038/closure/artifacts/test-binaries/feature-focused-tests/wtime_runtime-37e3c598ab512be0": "ca450ca20bf0fddffa274edeeb012b9c65757d2fc01c69ae4a8a2f1fa3cf052f",
    "/workspace/wtime/target/performance/P038/closure/inputs/check-result.json": "a4e2b08a6ada93deeb586d75d711da7bd91a3356800633d10db4c0686a7afda7",
    "/workspace/wtime/target/performance/P038/closure/inputs/closure-builder.py": "dbaad1497fab8a9354942c85310c7a6754846c3ba64f32b792115808d483fadf",
    "/workspace/wtime/target/performance/P038/closure/inputs/comparison-report.json": "2fba9d1c6954618e745416b006a8dadbea55cb816ef24c723ed90ce7afc7ecb1",
    "/workspace/wtime/target/performance/P038/closure/inputs/coverage.json": "ee585fbb244ddfca425b4dcf481fb639447f32f2d067e67345916a0db0149c22",
    "/workspace/wtime/target/performance/P038/closure/inputs/decision.json": "fd4a0c0f02f163946e2af95ffd84a0db524f34408d2901193e57f57617788932",
    "/workspace/wtime/target/performance/P038/closure/inputs/prepare-receipt.json": "902e63fb45919696f2a9706ef6e8058112e00fbfc1b8f58a162467ece47aaa7e",
    "/workspace/wtime/target/performance/P038/closure/scalar-evidence.json": "065c6bab3a71ebb70e70b077e2aaca73b1d9644511b891e1638085520e13118d"
  },
  "evidence": [
    {
      "bytes": 27472,
      "path": "/workspace/wtime/target/performance/P038/closure/scalar-evidence.json",
      "sha256": "065c6bab3a71ebb70e70b077e2aaca73b1d9644511b891e1638085520e13118d"
    }
  ],
  "outcome": "inconclusive",
  "status": "done",
  "task": "P038"
}
```
