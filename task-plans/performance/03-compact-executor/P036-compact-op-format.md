# P036 — Define small immutable execution instructions and side tables

Status: **done**

Outcome: **complete**

Kind: `foundation` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P019 — Define bounded optimizer effect and source expansion metadata](../01-execution-contracts/P019-effect-certificates.md)

[P021 — Compute bounded per-function execution metadata](../01-execution-contracts/P021-function-execution-plans.md)

[P035 — Evaluate structural wins and choose the next bottleneck](../02-structural-wins/P035-structural-milestone.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/execution/compact/ir.rs` — private immutable factory and its `#[cfg(test)]` child tests.
- `crates/wtime-runtime/src/execution/compact/mod.rs` — nondefault module wiring.
- `crates/wtime-runtime/tests/performance/p036_private.rs` — private factory tests included as a child of `ir.rs` with `include!("../../../tests/performance/p036_private.rs")`.
- `crates/wtime-runtime/tests/performance/p036.rs` — public-API checks only: default Decoded behavior and decoded fallback after metadata admission declines.
- `crates/wtime-runtime/tests/fixtures/performance/compact/` — durable WAT/Wasm fixtures and license/provenance records.

Tests that inspect `CompactFunction`, `CompactOp`, `SideImmediate`, verifier fields, or `from_module` must be nested inside `ir.rs` as a private unit-test child (`#[cfg(test)] mod tests { use super::*; ... }`). Keep internal constructors and fields private; do not add a raw compact-construction API for integration tests. The public performance test must exercise only the stable public runtime surface.

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; this foundation task does not connect a compact dispatcher.

## Input/output contract

`CompactFunction` is private, immutable metadata built only from a validated `FunctionPlan`; decoded execution remains the default and fallback. `CompactOp` is a safe fixed-size record with a compile-time maximum of 24 bytes. Narrow inline forms preserve all bits; checked side indices retain every wider or compound immediate without truncation.

Construction has absolute ceilings of 100,000 work units and 1 MiB retained metadata even when a caller supplies `usize::MAX`. Work is charged as six visits per source instruction plus three per nested side-table element. Size every retained vector and nested payload with checked arithmetic, account actual capacities, and reject over-budget input before reserving/publishing metadata. Allocation or verification failure publishes no partial function. Verify source/module/engine identity and reject tampered op tags, side indices, origins, work, and retained-byte counts.

This task establishes factory metadata and decoded fallback only. It does not execute CompactOp, change Engine defaults, claim compact fuel/trap/next-PC behavior, or claim a speedup; live-path correctness belongs to P038–P045 and selection to P046.

## Implementation recipe

1. Record current `Instruction`, `Immediate`, `Value`, and compact record sizes. Use safe Rust and a compile-time `size_of::<CompactOp>() <= 24` assertion; encode narrow indices and numeric bit patterns losslessly.
2. Cover every admitted inline form (`None`, index, pair, block type, i32/i64 and f32/f64 bits) and every side-table class (`BlockType`, branches, catches, resume handlers, V128 bytes, memory/lane operands, type vectors, heap types, and cast metadata). Unknown shapes return `Declined::Unsupported`.
3. Enforce absolute 100,000-work/1-MiB-retained ceilings, clamp caller limits, include nested side-table storage and actual capacities, use fallible reservations, and decline oversized inputs before allocation/publication. Failed construction leaves only the decoded module available.
4. Verify exact source identity, tags, side references, origins/effects, and budget accounting from admitted module data; tests corrupt each representative field and prove rejection.
5. Freshly encode durable CC0/project-licensed WAT fixtures with the pinned encoder and validate under all-phase3-core before installing their bytes. Cover wide indices, memory64, large branch/catch/resume side tables, nonzero SIMD bytes, heap/cast metadata, and a long function that declines the hard work bound.

## Meaningful cases

- Lossless narrow forms and all side-table classes round-trip from admitted modules; wide memory/index and proposal metadata are retained or explicitly declined, never truncated.
- Large `br_table`, handler, resume, and type-vector inputs remain within checked work/byte ceilings; caller maxima cannot override either absolute ceiling, and an early decline leaves no retained partial metadata.
- Foreign source/module ownership and tampered tags, side indices, origins, work, or byte counts fail verification.
- Durable, freshly encoded and all-phase3-core-validated fixtures cover continuation handlers, nonzero SIMD shuffle bytes, cast-branch metadata, wide local indices, memory64, and bounded-work fallback.
- The public integration test observes Decoded as the default and confirms declined metadata leaves decoded invocation available. No compact dispatch, live fuel/trap behavior, or performance claim is made at this stage.

## Verification

Activate Rust as described in EXECUTION.md. Private factory tests must compile as a child module of `ir.rs`, which includes the separate `tests/performance/p036_private.rs`, so they can inspect representation invariants without widening the API. `tests/performance/p036.rs` contains public-surface fallback/default checks only. The planned focused command remains:

```sh
cargo test --locked -p wtime-runtime --features compact-executor,execution-reference p036_ -- --nocapture
```

Require at least three named meaningful cases, plus coverage of every inline and side-table class, absolute byte/work rejection including caller `usize::MAX`, ownership/tamper verification, and the decoded fallback. The durable fixture WAT and fresh Wasm hashes, pinned encoder identity/command, all-phase3-core validation command/result, and the focused test command/status/counts must be recorded. This factory-only test set does not prove that a compact dispatcher ran; defer compact fuel, cancellation, materialization, trap attribution, and speed claims to the tasks that integrate and select live execution.

## Measurement and decision

Selection: `scalar`. Explicit execution setting: `compact`. Decision rule: `integrity`.

This is a tooling/contract deliverable. Require correct nonzero checks and bounded/source-bound output; do not invent a runtime speedup requirement. If it changes production work, record cold/build/retention deltas under [MEASUREMENT.md](../MEASUREMENT.md).

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. Executed evidence is recorded below.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/P036/closure/artifacts/P036-before-command.json": "3908f4e0a5938469706ab1a9113e2d89bc7a78b34e1c7d59cef16ce80ab965fa",
    "/workspace/wtime/target/performance/P036/closure/artifacts/P036-before-manifest.json": "8b65ef3c1cf964b158f8c014c0badd77a783e384e0bd075e6ac217d4a294c729",
    "/workspace/wtime/target/performance/P036/closure/artifacts/build-budgets/default-command.json": "0e989587be5dc2dc5ce388036fa8c517c83aaec10282248762a3caa1b90a4639",
    "/workspace/wtime/target/performance/P036/closure/artifacts/build-budgets/default.json": "de9a4ba08a6319a587224d23e88e6fd7af2a38c2e3f272ffe3c70fc14d3066a8",
    "/workspace/wtime/target/performance/P036/closure/artifacts/build-budgets/optional-command.json": "3958f17e5cc87a2612017a6870021b0e144508d9fb3954bfc6a824f582813ed7",
    "/workspace/wtime/target/performance/P036/closure/artifacts/build-budgets/optional.json": "1dd4e1c2c7b38400898b95a914d2eadff45376652c8336eb7e625a5b69c1e5dc",
    "/workspace/wtime/target/performance/P036/closure/artifacts/check-result.json": "5853359f1847f57a8af5a9557e56454438b57491f6940c69c09f28d266ba0806",
    "/workspace/wtime/target/performance/P036/closure/artifacts/default-focused-tests.json": "aca3f36cfde103643a5880fd416d027709b05c814c7b2f834f497d19db31008a",
    "/workspace/wtime/target/performance/P036/closure/artifacts/dependency-budget.json": "6610be51974afd0a5277e268bf890f9d98fd8dbe1d09f722e846e90db9f41fb2",
    "/workspace/wtime/target/performance/P036/closure/artifacts/feature-focused-tests.json": "8ef9c9efabf7d7fca2591b1dab699f86a3352ac5d45173a67d88715a9bd91fef",
    "/workspace/wtime/target/performance/P036/closure/artifacts/fixture-commands/cast_branch-encode_log.json": "2daf47e8c70f767c8b6543e787d6fadff059df181247648d0cf60015215afadd",
    "/workspace/wtime/target/performance/P036/closure/artifacts/fixture-commands/cast_branch-validation_log.json": "4944a894d7d89f4021f0fbae2a22d17a05b79beca086cef3430b8d4502529714",
    "/workspace/wtime/target/performance/P036/closure/artifacts/fixture-commands/continuation_root-encode_log.json": "fd2c533e032087bc52126aa6e033894784d1d34124c58634d3a2f280cdc14e5a",
    "/workspace/wtime/target/performance/P036/closure/artifacts/fixture-commands/continuation_root-validation_log.json": "ba54544f34511b32d9dcb90514786d1b682ff0fbd8ee7fa996b8ebebd8e30309",
    "/workspace/wtime/target/performance/P036/closure/artifacts/fixture-commands/long_metadata-encode_log.json": "b8fa0fd1c07d1237aacfdfe421649164cf3f3ed25972b9166feaf9cc2ab102c1",
    "/workspace/wtime/target/performance/P036/closure/artifacts/fixture-commands/long_metadata-validation_log.json": "8eb4640955ddac4b61344415e3e2b4ed04887f6be8f84ad8918e9559656d2e6c",
    "/workspace/wtime/target/performance/P036/closure/artifacts/fixture-commands/shuffle-encode_log.json": "0243838914f6187b2a43cd8fc16f9ad186d962eff0797ffb3f0b80204afdf748",
    "/workspace/wtime/target/performance/P036/closure/artifacts/fixture-commands/shuffle-validation_log.json": "65f35ad88d1f2625ef8df9abd9000f0a6f8aa2c9f6e6219a9024fa6c41232228",
    "/workspace/wtime/target/performance/P036/closure/artifacts/fixture-provenance.json": "b3dc79bc7e5ed5e29ee84349fa0b8c8bfab00aa04be3ae68f74d5e6bc4f0d079",
    "/workspace/wtime/target/performance/P036/closure/artifacts/prepare-receipt.json": "5fcb182a190087b0bddc0f774ac0e7833870ecdd898209009e062c9040ebc02a",
    "/workspace/wtime/target/performance/P036/closure/artifacts/source-snapshots/source-after.json": "2500a5f447f5a1de6de3e373cbd6986d8a711e70778a1754afdbe428c35110cb",
    "/workspace/wtime/target/performance/P036/closure/artifacts/source-snapshots/source-before.json": "2500a5f447f5a1de6de3e373cbd6986d8a711e70778a1754afdbe428c35110cb",
    "/workspace/wtime/target/performance/P036/closure/artifacts/source-snapshots/workspace-source-after.json": "7b18494582bb8456dea9225c0c734bc2c3e7707854a0b62157bb4c435cc77227",
    "/workspace/wtime/target/performance/P036/closure/artifacts/source-snapshots/workspace-source-before.json": "7b18494582bb8456dea9225c0c734bc2c3e7707854a0b62157bb4c435cc77227",
    "/workspace/wtime/target/performance/P036/closure/artifacts/test-binaries/default-focused-tests/core-fc4384471282e053": "c923897c138aae271136b181d183d75ddf90b8d83ea58877217345db196d37be",
    "/workspace/wtime/target/performance/P036/closure/artifacts/test-binaries/default-focused-tests/exception_root_safety-863d9cca58697c87": "e98e817a1f0d1a56e97381f54bae77e983f4e3c66603a0bdb9495fea71fbe8f8",
    "/workspace/wtime/target/performance/P036/closure/artifacts/test-binaries/default-focused-tests/exceptions-dc82360c779184c7": "f09468f79a36d1dd076a93a06b737bbcbbbe1a951a7eed4e0e8ced1115467509",
    "/workspace/wtime/target/performance/P036/closure/artifacts/test-binaries/default-focused-tests/feature_profiles-da219ada3555dbdb": "ca5dc27573c22488f1db0b2738c736666e7bbd005eaddd61a6514bd204b4b4d2",
    "/workspace/wtime/target/performance/P036/closure/artifacts/test-binaries/default-focused-tests/performance-f1f918c868572e47": "51caee2cd69c6a59c088ef4ff984c41dd011734ae72566abe777ac9d698fe738",
    "/workspace/wtime/target/performance/P036/closure/artifacts/test-binaries/default-focused-tests/proposals-ce7c5c5922b88848": "f149ac28de490574abb069a401e6b8c2adc1b078f88107dae1240535bb8523d2",
    "/workspace/wtime/target/performance/P036/closure/artifacts/test-binaries/default-focused-tests/wtime_runtime-670f0c395211a4f7": "a8b498e76a0f3a8af6ecc311a255a280a67768250e189f206be9e7d0125dc0bb",
    "/workspace/wtime/target/performance/P036/closure/artifacts/test-binaries/feature-focused-tests/core-42d92ac3afc77304": "0d3a384b6a5b0d2e16bef7c8a683d270a530cd3471745a263770f582e78bba95",
    "/workspace/wtime/target/performance/P036/closure/artifacts/test-binaries/feature-focused-tests/exception_root_safety-db67ff33a61a98ad": "7c0e1892d236a812cf1ef462e172c274de821e893562e4837b109f484988b0ac",
    "/workspace/wtime/target/performance/P036/closure/artifacts/test-binaries/feature-focused-tests/exceptions-5d275ecd0f4f002b": "59f20d214f3fa589ef20e3ce9088ece65c6a8799d8aa14afa8a2ad9ddfc2619a",
    "/workspace/wtime/target/performance/P036/closure/artifacts/test-binaries/feature-focused-tests/feature_profiles-487957d4b265fccb": "ef9fd30b5a45f417ebb0a82744fc8bc99c02cd8b550bcb8b7004fe5186e89b99",
    "/workspace/wtime/target/performance/P036/closure/artifacts/test-binaries/feature-focused-tests/performance-1a202aeb20ffb000": "2e2d2eb7fd13b0974e1fede09cc123cba6a3fbed372ee83e3b2563ee6fe159a6",
    "/workspace/wtime/target/performance/P036/closure/artifacts/test-binaries/feature-focused-tests/proposals-2b774c23db30396c": "76644bfce66520eea91931700a6a680192d5c361a25ff3a95c78183522082432",
    "/workspace/wtime/target/performance/P036/closure/artifacts/test-binaries/feature-focused-tests/wtime_runtime-37e3c598ab512be0": "0cb983a810af108959bea966ae45a1fa5e5606dbea36dd059c7eceaddfacc36f",
    "/workspace/wtime/target/performance/P036/closure/factory-evidence.json": "66671c090a75b33550449f13cd65f7f41da47b291d1368e57f37777d2a428f6f",
    "/workspace/wtime/target/performance/P036/closure/inputs/check-result.json": "5853359f1847f57a8af5a9557e56454438b57491f6940c69c09f28d266ba0806",
    "/workspace/wtime/target/performance/P036/closure/inputs/closure-builder.py": "abec6d2a7f643d89c6a15a80cb98a68ac47c3e1ba8b7e52508941a489c113b16",
    "/workspace/wtime/target/performance/P036/closure/inputs/prepare-receipt.json": "5fcb182a190087b0bddc0f774ac0e7833870ecdd898209009e062c9040ebc02a",
    "/workspace/wtime/target/performance/P036/closure/verified-sources.json": "f4bc901212fe61df7e6286bb35a0266dc8636f047d363e425a9f778716b50a85",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/P036-before/crates/wtime-runtime/src/arena.rs": "cd929b222d1ccf871f06684510df6767a1e69af4d338458c0e98b88cd16c8441",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/P036-before/crates/wtime-runtime/src/canonical.rs": "0bed54a47fbca8a39c7359a9140a5284f0513a6744505ddb5a0b92d531d0776d",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/P036-before/crates/wtime-runtime/src/continuation.rs": "c30b6fb4b89999b2e6f494b9258ce9ea3ea1e39a5971643b43147026e0fc8f46",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/P036-before/crates/wtime-runtime/src/coverage.rs": "fa735d933bf1334d23c262aac2d35282a743a49aaddb0c2bbc68da4c308f3fbe",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/P036-before/crates/wtime-runtime/src/exceptions.rs": "08e306b7f60b25077f31706bf55cc4b521b0b2b713a61e8656e36fcc34c326ba",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/P036-before/crates/wtime-runtime/src/execution.rs": "d2ef4ca6928c9c9f9ca7c02f9a94d0a4b8bdfd5b560ed1d3d6dab93fcb3f751d",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/P036-before/crates/wtime-runtime/src/execution/accounting.rs": "c86761a09e3c98301f54c971aa93e08da97140832c0f58e279aca4a75bef7127",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/P036-before/crates/wtime-runtime/src/execution/cursor.rs": "ff93a3f0056d109366d2b77ea93d459f456d048c05b4bf05369305a3b491d3d3",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/P036-before/crates/wtime-runtime/src/execution/effects.rs": "640bf290be1691465e44f188213b644030dab6f8ca1599790f5138816e192bcd",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/P036-before/crates/wtime-runtime/src/execution/plan.rs": "68e43e6484a6c198529b333a6e5122526845544d660191121755c3108e125d7e",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/P036-before/crates/wtime-runtime/src/gc.rs": "7dcfcc9aaff6a333041d9d8a7323d1ce0fad90b382b5023e9caeeb52815b3471",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/P036-before/crates/wtime-runtime/src/gc_exec.rs": "56cf9173994d267fa4ead0af137b8f9f941f6b86a52d458f31be3ce7a32280fc",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/P036-before/crates/wtime-runtime/src/lib.rs": "997a0ec8d835c83567e13aa4c755ed30ad0e44a0bdb9e689c14753abdd47bb45",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/P036-before/crates/wtime-runtime/src/linker.rs": "39a2068993bcb65fb4434f42d02b56687e8ae31914840487ee4c35de07677806",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/P036-before/crates/wtime-runtime/src/numeric.rs": "5acd2cc6a4e9e9c23fd4bb25ab8a9fbc1fa1658d63f4a95e207a00963650dc92",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/P036-before/crates/wtime-runtime/src/perf.rs": "df87993399e014674e63c8f15ea9dfb74888fe86ba5bd39a9129a4048121b3e5",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/P036-before/crates/wtime-runtime/src/reference.rs": "0fc6c3d773332bc39bde1e627e6c683ccfc36d525cf1a6b410c8fd69fdbac287",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/P036-before/crates/wtime-runtime/src/shared_memory.rs": "6b1a5a02e49d2d73ee04865af8d6bac68c60305f1afc754ca9b70252750bce44",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/P036-before/crates/wtime-runtime/src/simd.rs": "84c223e6a546aa8955ff5214b9add03df4c94d25f3ffcd45b33a808d299334b1",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/P036-before/crates/wtime-runtime/src/wide.rs": "e0347c76d5a00a56396bd641ce4beb4f0d6a7500739f6de57283c1cbacc4224a",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/crates/wtime-runtime/src/arena.rs": "cd929b222d1ccf871f06684510df6767a1e69af4d338458c0e98b88cd16c8441",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/crates/wtime-runtime/src/canonical.rs": "0bed54a47fbca8a39c7359a9140a5284f0513a6744505ddb5a0b92d531d0776d",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/crates/wtime-runtime/src/continuation.rs": "c30b6fb4b89999b2e6f494b9258ce9ea3ea1e39a5971643b43147026e0fc8f46",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/crates/wtime-runtime/src/coverage.rs": "fa735d933bf1334d23c262aac2d35282a743a49aaddb0c2bbc68da4c308f3fbe",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/crates/wtime-runtime/src/exceptions.rs": "08e306b7f60b25077f31706bf55cc4b521b0b2b713a61e8656e36fcc34c326ba",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/crates/wtime-runtime/src/execution.rs": "0ddd21d14787a466dfd74eae92a0f16dafe32f39d43750c4c30be47bd488eed8",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/crates/wtime-runtime/src/execution/accounting.rs": "c86761a09e3c98301f54c971aa93e08da97140832c0f58e279aca4a75bef7127",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/crates/wtime-runtime/src/execution/compact/ir.rs": "14c77b1707b3935750c6f887962abcd25b9c53a207e77188d6be4af8147a3dce",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/crates/wtime-runtime/src/execution/compact/mod.rs": "ac75095371627384019c757d3b2144d4c19edbeceb8f81adb54e50bf3f842882",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/crates/wtime-runtime/src/execution/cursor.rs": "ff93a3f0056d109366d2b77ea93d459f456d048c05b4bf05369305a3b491d3d3",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/crates/wtime-runtime/src/execution/effects.rs": "640bf290be1691465e44f188213b644030dab6f8ca1599790f5138816e192bcd",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/crates/wtime-runtime/src/execution/plan.rs": "68e43e6484a6c198529b333a6e5122526845544d660191121755c3108e125d7e",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/crates/wtime-runtime/src/gc.rs": "7dcfcc9aaff6a333041d9d8a7323d1ce0fad90b382b5023e9caeeb52815b3471",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/crates/wtime-runtime/src/gc_exec.rs": "56cf9173994d267fa4ead0af137b8f9f941f6b86a52d458f31be3ce7a32280fc",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/crates/wtime-runtime/src/lib.rs": "997a0ec8d835c83567e13aa4c755ed30ad0e44a0bdb9e689c14753abdd47bb45",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/crates/wtime-runtime/src/linker.rs": "39a2068993bcb65fb4434f42d02b56687e8ae31914840487ee4c35de07677806",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/crates/wtime-runtime/src/numeric.rs": "5acd2cc6a4e9e9c23fd4bb25ab8a9fbc1fa1658d63f4a95e207a00963650dc92",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/crates/wtime-runtime/src/perf.rs": "df87993399e014674e63c8f15ea9dfb74888fe86ba5bd39a9129a4048121b3e5",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/crates/wtime-runtime/src/reference.rs": "0fc6c3d773332bc39bde1e627e6c683ccfc36d525cf1a6b410c8fd69fdbac287",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/crates/wtime-runtime/src/shared_memory.rs": "6b1a5a02e49d2d73ee04865af8d6bac68c60305f1afc754ca9b70252750bce44",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/crates/wtime-runtime/src/simd.rs": "84c223e6a546aa8955ff5214b9add03df4c94d25f3ffcd45b33a808d299334b1",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/crates/wtime-runtime/src/wide.rs": "e0347c76d5a00a56396bd641ce4beb4f0d6a7500739f6de57283c1cbacc4224a",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/crates/wtime-runtime/tests/fixtures/feature_profiles/reference_types.wasm": "9d151980b47655256f32fd09047416716848f02f62c541344f6a2dec6d171fb1",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/crates/wtime-runtime/tests/fixtures/performance/borrow/vector_immediates.wasm": "a3dcf7f75c3070220654c583586004eada4847dd4df59897bd1f648b0271dd0e",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/crates/wtime-runtime/tests/fixtures/performance/branch/try_table_normal.wasm": "97068a6310a3bc535369e2b6277b801875c14ca206a707fcb9bca9036f41d0df",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/crates/wtime-runtime/tests/fixtures/performance/compact/LICENSE-APACHE.txt": "a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/crates/wtime-runtime/tests/fixtures/performance/compact/LICENSE-MIT.txt": "a43aeeeebe46eda1a6fea77337eb7f9f37341b94afe7251d1f6d14fe03e702c5",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/crates/wtime-runtime/tests/fixtures/performance/compact/cast_branch.wasm": "853112cefe2506c80269f586cf71d22c982d404ef8e4557d9150c42c9fa28560",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/crates/wtime-runtime/tests/fixtures/performance/compact/cast_branch.wat": "5eebdbc85bdb66c826c536e9d474062c3887a712cc9d5f74c2a0c351330de8ae",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/crates/wtime-runtime/tests/fixtures/performance/compact/continuation_root.wasm": "f3a432da4b984b715e74e26016d593a61e5dd4b24e421d1434bf63bf83c39f31",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/crates/wtime-runtime/tests/fixtures/performance/compact/continuation_root.wat": "05d3278c9be02485919784bae44f7b5fcd55a63856296b0fb1e4a2c854379448",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/crates/wtime-runtime/tests/fixtures/performance/compact/gc-CC0-LICENSE.txt": "0960f19d7554375da3008da6277ead6b7ede71ba54ab2b35a7df5c2e964ca9e7",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/crates/wtime-runtime/tests/fixtures/performance/compact/long_metadata.wasm": "8ab166cf1fe217e49b98ffdc30a51aac2ca5270bad26bd4b84e06ec37d42f290",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/crates/wtime-runtime/tests/fixtures/performance/compact/long_metadata.wat": "1ab5c20da62a29ac5e27546a10a0d49c19cc9962bd9e7cdfa8a61ff24b9e3719",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/crates/wtime-runtime/tests/fixtures/performance/compact/provenance.json": "b3dc79bc7e5ed5e29ee84349fa0b8c8bfab00aa04be3ae68f74d5e6bc4f0d079",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/crates/wtime-runtime/tests/fixtures/performance/compact/shuffle.wasm": "b71c56264d750cc8f7300e3b9900534072d92838447a2f600c920e20625f5f77",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/crates/wtime-runtime/tests/fixtures/performance/compact/shuffle.wat": "ce2af411702310c035dc1a3133feaa096b40fe43def12806e1bac0915dfe877a",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/crates/wtime-runtime/tests/fixtures/performance/cursor/multivalue_branch.wasm": "251acf5ea9d683baa9671ab18b7e1a053e2bb1ebaedff8c77f215dc5fd32502d",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/crates/wtime-runtime/tests/fixtures/performance/p025/memory64_boundary.wasm": "ddcaada7e309cb9e9aabb0c0167a0eca875912161ecc83cfe64ae7c70b52b244",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/crates/wtime-runtime/tests/fixtures/performance/plans/wide_local.wasm": "1e91688442c4e20093aa3dd0ef572958c588014857f0a9448ac6b2382439995c",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/crates/wtime-runtime/tests/fixtures/performance/simd/lane_loads_oob.wasm": "5204ac6676557f4fe417c9cdb1d4c85a0ed53f8a7190506c81ff9e8acf9210d5",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/crates/wtime-runtime/tests/performance/p036.rs": "1dadaf679a3c12ba6070dc5c0c74e01aef0b072a8fbf691adf96b42fbf1c27d7",
    "/workspace/wtime/target/performance/P036/closure/verified-sources/crates/wtime-runtime/tests/performance/p036_private.rs": "a2cec4949872b46936509fa5e7becbc23440a18cba0bf273979687c548cb12e5"
  },
  "evidence": [
    {
      "bytes": 66505,
      "path": "/workspace/wtime/target/performance/P036/closure/factory-evidence.json",
      "sha256": "66671c090a75b33550449f13cd65f7f41da47b291d1368e57f37777d2a428f6f"
    }
  ],
  "outcome": "complete",
  "status": "done",
  "task": "P036"
}
```
