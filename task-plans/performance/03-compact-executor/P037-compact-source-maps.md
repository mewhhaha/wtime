# P037 — Map compact instructions to source events and control targets

Status: **done**

Outcome: **complete**

Kind: `foundation` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P018 — Define source cursors and materialized guest state](../01-execution-contracts/P018-source-cursor.md)

[P019 — Define bounded optimizer effect and source expansion metadata](../01-execution-contracts/P019-effect-certificates.md)

[P020 — Centralize exact source instruction accounting](../01-execution-contracts/P020-exact-execution-accounting.md)

[P036 — Define small immutable execution instructions and side tables](P036-compact-op-format.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/execution/compact/ir.rs` — parent compact metadata and the private source-map child module.
- `crates/wtime-runtime/src/execution/compact/source_map.rs` — one-to-one metadata map, bounded verifier, and private map tests. P036 private factory tests use their separate `crates/wtime-runtime/tests/performance/p036_private.rs` include under `ir.rs`, separate from public `p036.rs`; P037 private tests use their separate `crates/wtime-runtime/tests/performance/p037_private.rs` include under `source_map.rs`, separate from public `p037.rs`; it must not become an external test target exposing internals.
- `crates/wtime-runtime/tests/performance/p037.rs` — public API checks only, including that default Decoded execution and source fuel behavior remain unaffected.
- `crates/wtime-runtime/tests/fixtures/performance/p037/control-edges.wat` — fresh control-edge fixture source.
- `crates/wtime-runtime/tests/fixtures/performance/p037/control-edges.wasm` — freshly encoded control-edge fixture.
- `crates/wtime-runtime/tests/fixtures/performance/p037/LICENSE-fixture.txt` — original CC0 fixture license.

No dispatch edits are part of P037. Dynamic exception, continuation, switch, reference-branch, and cast-branch functions decline as a whole until their runtime origins are modeled.

## Input/output contract

`SourceMap` is private metadata for an admitted `CompactFunction`, not an executable dispatcher. Begin with one compact event per original decoded instruction and explicit source-to-compact / compact-to-source entries including terminal `n`; terminal has no event. Preserve original source PC, opcode coverage identity, and span lookup through the immutable source module. Derive ordinary sequential, if/else/end, loop, block branch, `br_if`, `br_table`, return, and tail-transfer successors only from the validated decoded instruction and control tables. Reject branches outside the function or malformed spans.

Keep absolute aggregate ceilings of 100,000 work units and 1 MiB for CompactFunction retention, SourceMap retention, construction labels, and verification scratch, clamped even if callers request `usize::MAX`. Use checked arithmetic, fallible reserves and actual capacity accounting. Verify engine/module/source identity and reconstructed map contents; decline the entire function before publication when budgets, ownership, control structure, or supported-origin requirements fail.

Exception/catch/delegate/throw, continuation resume/suspend/switch, reference branches (`d5/d6`), and GC cast branches (`fb18/fb19`, `fb25/fb26`) remain explicit whole-function declines. P037 proves metadata targets and original-event associations only. It does not connect source-prefix accounting, coverage dispatch, trap attribution, fuel/cancellation stops, or materialization to a compact fetch; those live-path claims wait for P038–P045.

## Implementation recipe

1. Build one-to-one source/compact arrays with a terminal sentinel and a compact source-event coverage entry for each real original instruction. Keep spans resolvable from the validated source module; do not synthesize origins.
2. Construct successor spans from validated control metadata. Test if-false with/without else, reached else, fallthrough end, block exit, loop back-edge, both `br_if` alternatives, every `br_table` arm/default, return, tail transfer, and terminal no-event behavior.
3. Decline dynamic exception, continuation/switch, reference-branch and cast-branch functions whole; never guess a target for a runtime-selected handler or typed label.
4. Aggregate CompactFunction + SourceMap + temporary labels and verifier reconstruction under clamped 100,000-work/1-MiB byte limits. Check all length/index/byte arithmetic, actual capacities, ownership, spans, and targets; reserve first and publish once.
5. Freshly encode the CC0 control-edge WAT with the pinned encoder, validate it under all-phase3-core, and retain exact WAT/Wasm/encoder/CLI hashes and commands before installing it. Keep the three map tests private; the feature build has three public cases total (one default-policy case and two execution-reference cursor/store/branch cases), with one public case in the default-only build.

## Meaningful cases

- Independent expected tables prove if-false with and without else, reached else/end, block-exit versus loop-start, both `br_if` targets, all `br_table` arms/default, return/tail terminal edges, and no terminal event.
- Exactly three named private tests cover source-origin/span association, foreign-module/source identity, corrupt spans/targets, and aggregate byte/work boundaries including caller `usize::MAX` and no partial publication.
- Whole-function decline tests cover exceptions, continuation/switch origins, `d5/d6`, and GC cast branches; no guessed successor is accepted.
- A freshly encoded, all-phase3-core-validated CC0 WAT fixture has an independent control-edge oracle and recorded source/Wasm/encoder/CLI hashes.
- The feature build requires exactly three named public cases: one default-policy case and two `execution-reference` cases for decoded source-cursor/store/branch roundtrips with `ExecutionPolicy::Decoded`; the default-only build runs one public case; they do not execute compact code or establish compact dispatch, trap attribution, coverage dispatch, or fuel-stop materialization.

## Verification

Activate Rust as described in EXECUTION.md. Tests inspecting SourceMap fields or private CompactFunction data must be children of `source_map.rs`, optionally via the separate `tests/performance/p037_private.rs` included there; keep those separate from public `tests/performance/p037.rs` and do not widen raw metadata APIs. `tests/performance/p037.rs` is limited to public default/fallback/source-fuel checks.

```sh
cargo test --locked -p wtime-runtime --features compact-executor,execution-reference p037_ -- --nocapture
```

Require exactly three named private cases, exactly three named public cases in the feature build, and one public case in the default-only build, with explicit positive counts. Require independently specified ordinary successor tables, terminal behavior, source-event/span association, ownership/tamper rejection, whole-function unsupported declines, and aggregate clamped bounds. Fresh fixture encoding and all-phase3-core CLI validation must complete before fixture installation, with exact commands, statuses, and hashes recorded. Report this as bounded source-map metadata only: P037 has no compact dispatcher, and does not establish live fuel/cancellation prefixes, trap attribution, coverage, materialization, or speed.

## Measurement and decision

Selection: `integration`. Explicit runtime execution setting: `decoded`; private source-map construction is metadata verification, not compact execution. Decision rule: `integrity`.

Do not measure or report a P037 speedup. Any future live compact measurements belong to the dispatcher tasks and require their own matched execution-path evidence.

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. Executed evidence is recorded below.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/artifacts/P036-before-manifest.json": "8b65ef3c1cf964b158f8c014c0badd77a783e384e0bd075e6ac217d4a294c729",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/artifacts/P036-factory-evidence.json": "66671c090a75b33550449f13cd65f7f41da47b291d1368e57f37777d2a428f6f",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/artifacts/P037-before-command.json": "68e15a9ed38f7cde3dd7f9717cb898b50a0474a437369e2b05e10386ad8e03e5",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/artifacts/P037-before-manifest.json": "e6b8338b10492fe1e245c3215fda06409d7f4c06fa1b6e382a252d75475bc854",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/artifacts/P037-check-result.json": "974d1f5a7436944c59685988292160eb2b833a2c2a1329c6a6989f07ca9dd995",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/artifacts/P037-prepare-receipt.json": "0ea1f24680fa0068d1577e886a62118ce867992875b149c69841a3a1911fa2e1",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/artifacts/build-budgets/default-command.json": "08b00d5457652fd58ed1ebb5ecbf893c790185befabb8371c1a4b70dafae3749",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/artifacts/build-budgets/default.json": "32b1eff76f923db69473df0e7699eabc651584e074002ee59ca526ccd60b77a6",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/artifacts/build-budgets/optional-command.json": "88146582be9658b19cbca0ec4459ad2cd507e6edc8bf24b7636128110d99facc",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/artifacts/build-budgets/optional.json": "6bf18f8cbef6be52430c54e007ea332b6b2b0b0c78715536322739f8b1474451",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/artifacts/default-focused-tests.json": "8ede246a6f7af89ffc7601234e9fcbde28e20b969f82733d1c1b67bc2a9327e1",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/artifacts/dependency-budget.json": "6610be51974afd0a5277e268bf890f9d98fd8dbe1d09f722e846e90db9f41fb2",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/artifacts/encode-control-edges.json": "dc7e9b85565ad2c4b15d2544066f75fdcf52013fe90f4e3179131d29d1dc68ac",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/artifacts/feature-focused-tests.json": "10d61bd654fb650dc9c987e1cec8889a72d74b11dac10be89240a3e54da28753",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/artifacts/source-snapshots/after-snapshot-recovery/helper.executed.py": "ca1e490a7376653f6f824dcdd0f8ce5cbf9463a6e57d7a16877905133b78b746",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/artifacts/source-snapshots/after-snapshot-recovery/inputs/default_build.json": "32b1eff76f923db69473df0e7699eabc651584e074002ee59ca526ccd60b77a6",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/artifacts/source-snapshots/after-snapshot-recovery/inputs/optional_build.json": "6bf18f8cbef6be52430c54e007ea332b6b2b0b0c78715536322739f8b1474451",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/artifacts/source-snapshots/after-snapshot-recovery/inputs/result.json.json": "974d1f5a7436944c59685988292160eb2b833a2c2a1329c6a6989f07ca9dd995",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/artifacts/source-snapshots/after-snapshot-recovery/inputs/workspace-metadata-after.json.json": "72eeaaa4792f03cb62af14a960a1b4261a2901bb23d5439c2185d3a6931664e6",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/artifacts/source-snapshots/after-snapshot-recovery/inputs/workspace-source-before.json.json": "6d3bff512bf90d5a4669d02e8d502c209279fbcffa00e49e5ada5c264dfdf2a9",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/artifacts/source-snapshots/after-snapshot-recovery/record.json": "48d72f8d725c138bb1a459070bd4c8750d4016e6ad773908ec686ff7fd891a5a",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/artifacts/source-snapshots/after-snapshot-recovery/source_map_helper.py": "f755f23d99acdc76d5dc146132ab1c04679f12055f50ff7f7225ec2f346ccbcb",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/artifacts/source-snapshots/workspace-source-after.json": "6d3bff512bf90d5a4669d02e8d502c209279fbcffa00e49e5ada5c264dfdf2a9",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/artifacts/source-snapshots/workspace-source-before.json": "6d3bff512bf90d5a4669d02e8d502c209279fbcffa00e49e5ada5c264dfdf2a9",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/artifacts/test-binaries/default-focused-tests/core-fc4384471282e053": "c923897c138aae271136b181d183d75ddf90b8d83ea58877217345db196d37be",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/artifacts/test-binaries/default-focused-tests/exception_root_safety-863d9cca58697c87": "e98e817a1f0d1a56e97381f54bae77e983f4e3c66603a0bdb9495fea71fbe8f8",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/artifacts/test-binaries/default-focused-tests/exceptions-dc82360c779184c7": "f09468f79a36d1dd076a93a06b737bbcbbbe1a951a7eed4e0e8ced1115467509",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/artifacts/test-binaries/default-focused-tests/feature_profiles-da219ada3555dbdb": "ca5dc27573c22488f1db0b2738c736666e7bbd005eaddd61a6514bd204b4b4d2",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/artifacts/test-binaries/default-focused-tests/performance-f1f918c868572e47": "5d2baacca5acd144207c0a5811cdf0319bd53275cdbb733150a47b624de52b00",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/artifacts/test-binaries/default-focused-tests/proposals-ce7c5c5922b88848": "f149ac28de490574abb069a401e6b8c2adc1b078f88107dae1240535bb8523d2",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/artifacts/test-binaries/default-focused-tests/wtime_runtime-670f0c395211a4f7": "a8b498e76a0f3a8af6ecc311a255a280a67768250e189f206be9e7d0125dc0bb",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/artifacts/test-binaries/feature-focused-tests/core-42d92ac3afc77304": "0d3a384b6a5b0d2e16bef7c8a683d270a530cd3471745a263770f582e78bba95",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/artifacts/test-binaries/feature-focused-tests/exception_root_safety-db67ff33a61a98ad": "7c0e1892d236a812cf1ef462e172c274de821e893562e4837b109f484988b0ac",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/artifacts/test-binaries/feature-focused-tests/exceptions-5d275ecd0f4f002b": "59f20d214f3fa589ef20e3ce9088ece65c6a8799d8aa14afa8a2ad9ddfc2619a",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/artifacts/test-binaries/feature-focused-tests/feature_profiles-487957d4b265fccb": "ef9fd30b5a45f417ebb0a82744fc8bc99c02cd8b550bcb8b7004fe5186e89b99",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/artifacts/test-binaries/feature-focused-tests/performance-1a202aeb20ffb000": "d9e2105eed74da1dfc9792c4753f3b100d428cc9a2b000f5df6adaa39b3ba2d8",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/artifacts/test-binaries/feature-focused-tests/proposals-2b774c23db30396c": "76644bfce66520eea91931700a6a680192d5c361a25ff3a95c78183522082432",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/artifacts/test-binaries/feature-focused-tests/wtime_runtime-37e3c598ab512be0": "8cf0f312161106c547d90ac46b690ffc233e97ea491f9bde30ef0483db5edd9d",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/artifacts/validate-control-edges.json": "9eb5fef7d250605a03f8af2579b27791b8aa2109fa39991a90e62bf6394d29ba",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/inputs/check-result.json": "974d1f5a7436944c59685988292160eb2b833a2c2a1329c6a6989f07ca9dd995",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/inputs/closure-builder.py": "9ed8f70812904753287a9ad1eda9e4b0fd1239a405e4e4b834e6f8c5755a7ed0",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/inputs/prepare-receipt.json": "0ea1f24680fa0068d1577e886a62118ce867992875b149c69841a3a1911fa2e1",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/source-map-evidence.json": "f7682a5d3a6fe71a0ad3519735cd7b45690e45204d0a918ccd2918955ce45e9c",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources.json": "16d70f351214b5d4b296cb07f1c6cd67d2daabfbf92196de7968331e9a145a81",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/P036-before/crates/wtime-runtime/src/arena.rs": "cd929b222d1ccf871f06684510df6767a1e69af4d338458c0e98b88cd16c8441",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/P036-before/crates/wtime-runtime/src/canonical.rs": "0bed54a47fbca8a39c7359a9140a5284f0513a6744505ddb5a0b92d531d0776d",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/P036-before/crates/wtime-runtime/src/continuation.rs": "c30b6fb4b89999b2e6f494b9258ce9ea3ea1e39a5971643b43147026e0fc8f46",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/P036-before/crates/wtime-runtime/src/coverage.rs": "fa735d933bf1334d23c262aac2d35282a743a49aaddb0c2bbc68da4c308f3fbe",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/P036-before/crates/wtime-runtime/src/exceptions.rs": "08e306b7f60b25077f31706bf55cc4b521b0b2b713a61e8656e36fcc34c326ba",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/P036-before/crates/wtime-runtime/src/execution.rs": "d2ef4ca6928c9c9f9ca7c02f9a94d0a4b8bdfd5b560ed1d3d6dab93fcb3f751d",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/P036-before/crates/wtime-runtime/src/execution/accounting.rs": "c86761a09e3c98301f54c971aa93e08da97140832c0f58e279aca4a75bef7127",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/P036-before/crates/wtime-runtime/src/execution/cursor.rs": "ff93a3f0056d109366d2b77ea93d459f456d048c05b4bf05369305a3b491d3d3",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/P036-before/crates/wtime-runtime/src/execution/effects.rs": "640bf290be1691465e44f188213b644030dab6f8ca1599790f5138816e192bcd",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/P036-before/crates/wtime-runtime/src/execution/plan.rs": "68e43e6484a6c198529b333a6e5122526845544d660191121755c3108e125d7e",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/P036-before/crates/wtime-runtime/src/gc.rs": "7dcfcc9aaff6a333041d9d8a7323d1ce0fad90b382b5023e9caeeb52815b3471",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/P036-before/crates/wtime-runtime/src/gc_exec.rs": "56cf9173994d267fa4ead0af137b8f9f941f6b86a52d458f31be3ce7a32280fc",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/P036-before/crates/wtime-runtime/src/lib.rs": "997a0ec8d835c83567e13aa4c755ed30ad0e44a0bdb9e689c14753abdd47bb45",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/P036-before/crates/wtime-runtime/src/linker.rs": "39a2068993bcb65fb4434f42d02b56687e8ae31914840487ee4c35de07677806",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/P036-before/crates/wtime-runtime/src/numeric.rs": "5acd2cc6a4e9e9c23fd4bb25ab8a9fbc1fa1658d63f4a95e207a00963650dc92",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/P036-before/crates/wtime-runtime/src/perf.rs": "df87993399e014674e63c8f15ea9dfb74888fe86ba5bd39a9129a4048121b3e5",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/P036-before/crates/wtime-runtime/src/reference.rs": "0fc6c3d773332bc39bde1e627e6c683ccfc36d525cf1a6b410c8fd69fdbac287",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/P036-before/crates/wtime-runtime/src/shared_memory.rs": "6b1a5a02e49d2d73ee04865af8d6bac68c60305f1afc754ca9b70252750bce44",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/P036-before/crates/wtime-runtime/src/simd.rs": "84c223e6a546aa8955ff5214b9add03df4c94d25f3ffcd45b33a808d299334b1",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/P036-before/crates/wtime-runtime/src/wide.rs": "e0347c76d5a00a56396bd641ce4beb4f0d6a7500739f6de57283c1cbacc4224a",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/P037-before/crates/wtime-runtime/src/arena.rs": "cd929b222d1ccf871f06684510df6767a1e69af4d338458c0e98b88cd16c8441",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/P037-before/crates/wtime-runtime/src/canonical.rs": "0bed54a47fbca8a39c7359a9140a5284f0513a6744505ddb5a0b92d531d0776d",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/P037-before/crates/wtime-runtime/src/continuation.rs": "c30b6fb4b89999b2e6f494b9258ce9ea3ea1e39a5971643b43147026e0fc8f46",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/P037-before/crates/wtime-runtime/src/coverage.rs": "fa735d933bf1334d23c262aac2d35282a743a49aaddb0c2bbc68da4c308f3fbe",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/P037-before/crates/wtime-runtime/src/exceptions.rs": "08e306b7f60b25077f31706bf55cc4b521b0b2b713a61e8656e36fcc34c326ba",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/P037-before/crates/wtime-runtime/src/execution.rs": "0ddd21d14787a466dfd74eae92a0f16dafe32f39d43750c4c30be47bd488eed8",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/P037-before/crates/wtime-runtime/src/execution/accounting.rs": "c86761a09e3c98301f54c971aa93e08da97140832c0f58e279aca4a75bef7127",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/P037-before/crates/wtime-runtime/src/execution/compact/ir.rs": "14c77b1707b3935750c6f887962abcd25b9c53a207e77188d6be4af8147a3dce",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/P037-before/crates/wtime-runtime/src/execution/compact/mod.rs": "ac75095371627384019c757d3b2144d4c19edbeceb8f81adb54e50bf3f842882",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/P037-before/crates/wtime-runtime/src/execution/cursor.rs": "ff93a3f0056d109366d2b77ea93d459f456d048c05b4bf05369305a3b491d3d3",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/P037-before/crates/wtime-runtime/src/execution/effects.rs": "640bf290be1691465e44f188213b644030dab6f8ca1599790f5138816e192bcd",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/P037-before/crates/wtime-runtime/src/execution/plan.rs": "68e43e6484a6c198529b333a6e5122526845544d660191121755c3108e125d7e",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/P037-before/crates/wtime-runtime/src/gc.rs": "7dcfcc9aaff6a333041d9d8a7323d1ce0fad90b382b5023e9caeeb52815b3471",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/P037-before/crates/wtime-runtime/src/gc_exec.rs": "56cf9173994d267fa4ead0af137b8f9f941f6b86a52d458f31be3ce7a32280fc",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/P037-before/crates/wtime-runtime/src/lib.rs": "997a0ec8d835c83567e13aa4c755ed30ad0e44a0bdb9e689c14753abdd47bb45",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/P037-before/crates/wtime-runtime/src/linker.rs": "39a2068993bcb65fb4434f42d02b56687e8ae31914840487ee4c35de07677806",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/P037-before/crates/wtime-runtime/src/numeric.rs": "5acd2cc6a4e9e9c23fd4bb25ab8a9fbc1fa1658d63f4a95e207a00963650dc92",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/P037-before/crates/wtime-runtime/src/perf.rs": "df87993399e014674e63c8f15ea9dfb74888fe86ba5bd39a9129a4048121b3e5",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/P037-before/crates/wtime-runtime/src/reference.rs": "0fc6c3d773332bc39bde1e627e6c683ccfc36d525cf1a6b410c8fd69fdbac287",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/P037-before/crates/wtime-runtime/src/shared_memory.rs": "6b1a5a02e49d2d73ee04865af8d6bac68c60305f1afc754ca9b70252750bce44",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/P037-before/crates/wtime-runtime/src/simd.rs": "84c223e6a546aa8955ff5214b9add03df4c94d25f3ffcd45b33a808d299334b1",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/P037-before/crates/wtime-runtime/src/wide.rs": "e0347c76d5a00a56396bd641ce4beb4f0d6a7500739f6de57283c1cbacc4224a",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/current/crates/wtime-runtime/src/arena.rs": "cd929b222d1ccf871f06684510df6767a1e69af4d338458c0e98b88cd16c8441",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/current/crates/wtime-runtime/src/canonical.rs": "0bed54a47fbca8a39c7359a9140a5284f0513a6744505ddb5a0b92d531d0776d",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/current/crates/wtime-runtime/src/continuation.rs": "c30b6fb4b89999b2e6f494b9258ce9ea3ea1e39a5971643b43147026e0fc8f46",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/current/crates/wtime-runtime/src/coverage.rs": "fa735d933bf1334d23c262aac2d35282a743a49aaddb0c2bbc68da4c308f3fbe",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/current/crates/wtime-runtime/src/exceptions.rs": "08e306b7f60b25077f31706bf55cc4b521b0b2b713a61e8656e36fcc34c326ba",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/current/crates/wtime-runtime/src/execution.rs": "0ddd21d14787a466dfd74eae92a0f16dafe32f39d43750c4c30be47bd488eed8",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/current/crates/wtime-runtime/src/execution/accounting.rs": "c86761a09e3c98301f54c971aa93e08da97140832c0f58e279aca4a75bef7127",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/current/crates/wtime-runtime/src/execution/compact/ir.rs": "7c05c65eb35945ccecb71ceec7d963506f5303b80e5c7cc5d9ffe05aefdbe05d",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/current/crates/wtime-runtime/src/execution/compact/mod.rs": "ac75095371627384019c757d3b2144d4c19edbeceb8f81adb54e50bf3f842882",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/current/crates/wtime-runtime/src/execution/compact/source_map.rs": "91595a0b484dbb689e159537c2b91f4bab02ffd2fe4d15dbc7ca45a982c3dc8e",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/current/crates/wtime-runtime/src/execution/cursor.rs": "ff93a3f0056d109366d2b77ea93d459f456d048c05b4bf05369305a3b491d3d3",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/current/crates/wtime-runtime/src/execution/effects.rs": "640bf290be1691465e44f188213b644030dab6f8ca1599790f5138816e192bcd",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/current/crates/wtime-runtime/src/execution/plan.rs": "68e43e6484a6c198529b333a6e5122526845544d660191121755c3108e125d7e",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/current/crates/wtime-runtime/src/gc.rs": "7dcfcc9aaff6a333041d9d8a7323d1ce0fad90b382b5023e9caeeb52815b3471",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/current/crates/wtime-runtime/src/gc_exec.rs": "56cf9173994d267fa4ead0af137b8f9f941f6b86a52d458f31be3ce7a32280fc",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/current/crates/wtime-runtime/src/lib.rs": "997a0ec8d835c83567e13aa4c755ed30ad0e44a0bdb9e689c14753abdd47bb45",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/current/crates/wtime-runtime/src/linker.rs": "39a2068993bcb65fb4434f42d02b56687e8ae31914840487ee4c35de07677806",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/current/crates/wtime-runtime/src/numeric.rs": "5acd2cc6a4e9e9c23fd4bb25ab8a9fbc1fa1658d63f4a95e207a00963650dc92",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/current/crates/wtime-runtime/src/perf.rs": "df87993399e014674e63c8f15ea9dfb74888fe86ba5bd39a9129a4048121b3e5",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/current/crates/wtime-runtime/src/reference.rs": "0fc6c3d773332bc39bde1e627e6c683ccfc36d525cf1a6b410c8fd69fdbac287",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/current/crates/wtime-runtime/src/shared_memory.rs": "6b1a5a02e49d2d73ee04865af8d6bac68c60305f1afc754ca9b70252750bce44",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/current/crates/wtime-runtime/src/simd.rs": "84c223e6a546aa8955ff5214b9add03df4c94d25f3ffcd45b33a808d299334b1",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/current/crates/wtime-runtime/src/wide.rs": "e0347c76d5a00a56396bd641ce4beb4f0d6a7500739f6de57283c1cbacc4224a",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/current/crates/wtime-runtime/tests/fixtures/performance/branch/try_table_normal.wasm": "97068a6310a3bc535369e2b6277b801875c14ca206a707fcb9bca9036f41d0df",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/current/crates/wtime-runtime/tests/fixtures/performance/compact/cast_branch.wasm": "853112cefe2506c80269f586cf71d22c982d404ef8e4557d9150c42c9fa28560",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/current/crates/wtime-runtime/tests/fixtures/performance/compact/continuation_root.wasm": "f3a432da4b984b715e74e26016d593a61e5dd4b24e421d1434bf63bf83c39f31",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/current/crates/wtime-runtime/tests/fixtures/performance/cursor/multivalue_branch.wasm": "251acf5ea9d683baa9671ab18b7e1a053e2bb1ebaedff8c77f215dc5fd32502d",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/current/crates/wtime-runtime/tests/fixtures/performance/cursor/store_successor.wasm": "e05575761936f326eebf004bcfc6b015cb6dbf8ab14412aa7daae268cff3ce96",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/current/crates/wtime-runtime/tests/fixtures/performance/p037/LICENSE-fixture.txt": "521a08b283d7a32effe3955d15eea4d82427dc16f192e5180f023f77b5324778",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/current/crates/wtime-runtime/tests/fixtures/performance/p037/control-edges.wasm": "012b9605ee55420e4bdd056991b11ecc3cfe3b0605b5f32810471122a4ff410a",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/current/crates/wtime-runtime/tests/fixtures/performance/p037/control-edges.wat": "5a3fd7d66a7ac35cef97b8ff2224007be07985613640621007a046ff83ea4a2b",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/current/crates/wtime-runtime/tests/fixtures/performance/p037/provenance.json": "77c5d65822403fbcd2586f95fcc0af1a3dcee0f1af32d6b8c2ee0f86165e64f6",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/current/crates/wtime-runtime/tests/performance/p037.rs": "9f761a49e3ab4a57fe173bd406aff861f304f14f970d0ff8fc73659bf87be30e",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/current/crates/wtime-runtime/tests/performance/p037_private.rs": "b35445fc643e895f66975832d43ece1525e389b876f9e87331de5042894b89ab",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/fresh-encoded/P037/fixtures/control-edges.wasm": "012b9605ee55420e4bdd056991b11ecc3cfe3b0605b5f32810471122a4ff410a",
    "/workspace/wtime/target/performance/P037/closure-attempt-0003/verified-sources/staged/P037/fixtures/control-edges.wat": "5a3fd7d66a7ac35cef97b8ff2224007be07985613640621007a046ff83ea4a2b"
  },
  "evidence": [
    {
      "bytes": 90455,
      "path": "/workspace/wtime/target/performance/P037/closure-attempt-0003/source-map-evidence.json",
      "sha256": "f7682a5d3a6fe71a0ad3519735cd7b45690e45204d0a918ccd2918955ce45e9c"
    }
  ],
  "outcome": "complete",
  "status": "done",
  "task": "P037"
}
```
