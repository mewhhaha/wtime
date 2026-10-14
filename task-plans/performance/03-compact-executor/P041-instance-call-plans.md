# P041 — Resolve direct guest call bindings once per instance

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

[P029 — Stop cloning function signatures into every activation](../02-structural-wins/P029-shared-function-metadata.md)

[P030 — Split checked public entry from validated guest call entry](../02-structural-wins/P030-validated-guest-frame-entry.md)

[P032 — Move guest arguments and tail calls without split_off allocation](../02-structural-wins/P032-call-argument-ranges.md)

[P038 — Lower constants, locals and common integer operations](P038-compact-scalar-lowering.md)

[P039 — Numeric and vector compact adapters](P039-compact-float-vector-adapters.md)

[P040 — Lower structured control to resolved jump and result plans](P040-compact-control-plans.md)

## Scope and ownership

Read EXECUTION.md, SOURCE_NOTES.md, BACKEND_CONTRACTS.md and C1,C2,C3,C4,C6,C7,C8.

- `crates/wtime-runtime/src/execution/compact/calls.rs`
- `crates/wtime-runtime/src/execution/compact/lower_scalar.rs`
- `crates/wtime-runtime/src/execution/compact/execute.rs`
- `crates/wtime-runtime/src/execution/compact/mod.rs`
- `crates/wtime-runtime/src/execution.rs`
- `crates/wtime-runtime/src/linker.rs`
- `crates/wtime-runtime/src/perf.rs`
- `crates/wtime-runtime/src/lib.rs`
- `crates/wtime-runtime/tests/performance.rs` (P041 registration)
- `crates/wtime-runtime/tests/performance/p041.rs`
- `crates/wtime-runtime/tests/performance/p041_private.rs` (private binding-plan regressions)
- `crates/wtime-runtime/tests/performance/p038.rs` (instrumented compact caller counters)
- `crates/wtime-runtime/tests/performance/p040.rs` (instrumented compact caller counters)
- `crates/wtime-runtime/tests/fixtures/performance/p041`
- `docs/benchmarks/performance/P041-contract.json`

One integrator owns lib.rs, dispatch, Cargo features, and tests/performance.rs.

P041's predeclared scalar workload roles are frozen in `docs/benchmarks/performance/P041-contract.json`.

## Input/output contract

Each Instance owns an immutable bounded direct-call plan for its local/import guest targets, with checked Store-local identities and signature arities. Engine-shared compact code stores only local function indices. The frozen scalar selection has 85 IDs: primary exactly scalar_direct_calls_n0, n1, n17, n32, n1024, n10000; all other79 rows are protected controls.

## Implementation recipe

1. Build guest target bindings during successful instantiation after ordinary linker checks; distinguish guest and host targets and retain canonical type context.
2. Route compact direct guest and tail calls through validated entry and argument-range paths, preserving logical depth, fuel, result movement, and host/root boundaries.
3. Keep plans Instance-local; failed instantiation publishes no partial plan and preserves imported-side-effect cleanup.
4. Keep Engine/Module payloads free of Func handles and Store-local roots; enforce bounded plan bytes/work and checked indices before publication.
5. Compare six direct-call sizes against79 protected controls with exact optimized/fallback counters and matching Compact policy.

## Meaningful cases

- Two Stores/instances sharing one Module call their own targets after instance growth and GC.
- Imports bind distinct provider exports; host calls still validate arguments/results and support reentry.
- Stale handles and failed instantiation preserve cleanup and rejection.
- Ordinary recursion, tail recursion, and tail-call result movement preserve depth/fuel limits.
- Six primary direct-call sizes, including n0 no-call control, retain exact compact/fallback counts.

## Verification

Run the source-bound P041 prepare/check producer after the predecessor closes. Bind all seven public P041 cases and all three private binding-plan cases to their source files, including call-target ownership, failed-instantiation, and host/reentry coverage. Check exact P040/P039/P038 regression sets, retain every Cargo test binary, run the full default suite unfiltered with at least228 passed and no ignored/filtered tests, and bind source maps/build/dependency gates.

```sh
cargo test --locked -p wtime-runtime --features compact-executor,execution-reference p041_ -- --nocapture
cargo test --locked -p wtime-runtime --features compact-executor,execution-reference p040_ -- --nocapture
cargo test --locked -p wtime-runtime --features compact-executor,execution-reference p039_ -- --nocapture
cargo test --locked -p wtime-runtime --features compact-executor,execution-reference p038_ -- --nocapture
cargo test --locked -p wtime-runtime
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml --features compact-executor -j 4
python3 tools/wtime-bench/compare.py --baseline target/performance/P041-before --candidate tools/wtime-bench/target/release/wtime-bench --candidate-mode compact --selection scalar --output target/performance/P041
python3 tools/wtime-bench/analyze.py --input target/performance/P041 --output target/performance/P041/analysis.json
python3 tools/checks/build_budget.py --output target/performance/P041/build.json
python3 tools/checks/build_budget.py --features wtime-runtime/compact-executor --output target/performance/P041/optional-build.json
python3 tools/wtime-bench/gates.py --task P041 --comparison target/performance/P041 --build target/performance/P041/build.json --optional-build target/performance/P041/optional-build.json --contract docs/benchmarks/performance/P041-contract.json --output target/performance/P041/decision.json
```

## Measurement and decision

Selection: `scalar`. Explicit execution setting: `compact`. Decision rule: `measured_promotion`.

Primary exactly the six scalar_direct_calls_n{0,1,17,32,1024,10000} rows; protect the other79 selected scalar rows. The n0 row remains a primary no-call control and may report zero compact steps when no guest call executes. Match policy, cold/preparation/retention and build budgets. Predeclaration is not a gain claim.

## Before editing

After closed P039, snapshot a fresh Compact P041-before at the 27-file runtime state before P041 source edits. Keep the P039 and P040 evidence/status DAG refs.

```sh
python3 tools/wtime-bench/baseline.py --mode compact --output target/performance/P041-before
```

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. Executed evidence is recorded below.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/P039-evidence.json": "3b804b9d36ce14be2584ad2a2e863b96e152a0c1a7c0df6dbfebfc6938575238",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/P040-evidence.json": "5151763755e40b83365407d0bb6b4f66c14856c98eb4f683132fdcfb5e9e7628",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/P041-before-command.json": "bbbaf6fed359d73ea61575016e08f3dcd05a7bd1012f7e89c0c3a6918bda0c87",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/P041-before-manifest.json": "f9ad128c2c10cfd04f7364c08b315f2033700637bec2fa1cc4e0a9bc1bee8aa8",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/P041-check-result.json": "abf172c8bb0d4225e646055adafe0aef4d59e9cfdbb04878b353624fac6c9520",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/P041-prepare-receipt.json": "4142077116c43807003302e8dd3240254e5e967a156de53539f2937e17c8724e",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/P041-test-contract.json": "d1623d5f4a3547fb2c16f8aa4407fc03af0805631c68a86bb6423aa0025e60b6",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/P041-workload-contract.json": "c6a0d0b3aa381e52a6017b9e5bb135fd1ab6ba66c427ab1eca509bf2c58c5199",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/SHA256SUMS": "84aafe0554438c5dd872e2c700424173ff186048b6e7b1a8131813217b285180",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/build-budget-commands/default.json": "56ebf36aed4b0e103ff852996983a716cdb33433248aa05a94d91ab627c03e08",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/build-budget-commands/optional.json": "416e33c1e89e666e83c64fc173efb83440a4da5e789ebb6ccca0c5cf5b40f78d",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/build-budgets/default.json": "e420c3aa55ed911a7604c38a8973626060567b8d8fa94d6082681f8477312b78",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/build-budgets/optional.json": "d049ab4d3dd1ef28a2072d792d4e47989dac3f2afdeed6db33e0d3f5b2120c7e",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/comparison-report.json": "182397a94c71f0364439c5f1d1a1e0eb8fe9408fed973219eec494f8100b2393",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-binary/wtime-bench": "ed1637be35eb648a88ca32aece423e94e3996e934b54c3b6de1b65399024809f",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-collector.py": "dc5c2aff063ffdebb5721af579a571337964909aa5f4ab035281d99fbc8593fe",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-fixtures/accumulator.wasm": "ef61bf7afc44af7b71ef78eadcba48c0d85949d7d9096df8fbbbab53c05036b1",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-fixtures/br_table.wasm": "d43d5dd71d5ca1c3dae820aa85c13a2d69415e028b8d48097fed4558f743163a",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-fixtures/branch_calls.wasm": "9b58d26f201900b5b6cce684133a98ab68b3cf55d979b64ce895e914e9d5327b",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-fixtures/branch_entropy.wasm": "e4c87dbab50a2438cb1f749bc6fd6f9c834c81484acd42864735bd1ad1b6dec0",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-fixtures/call_overhead.wasm": "0815ca7d6026206d0773fe10c6b9643862bfffe8ba361c9789369ef9668ee746",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-fixtures/call_ref.wasm": "4f766dc9e29324ea942c892982363dec0ade98565288878a70967ce3b33f322b",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-fixtures/dependent.wasm": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-fixtures/direct_calls.wasm": "f6adbecce99c05bf06f213f4f5c0d5d9f001074acbbb2e6bbe814696bb866e45",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-fixtures/dynamic_api.wasm": "c493d0aae68d0c5aa67fc0b562e616c45eb3d42b0302951158a763fa202ce589",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-fixtures/host_call.wasm": "c60311d57c929c3ff1d29adb70501e7bbf8419517acbd86c8d072438b610ebb7",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-fixtures/indirect_calls.wasm": "4b42e82f54ea6ab7d0f62c70766c904e41d6a408e57fa14b5abbf514c0da78a9",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-fixtures/integer_loop.wasm": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-fixtures/multivalue_branch.wasm": "03ffdc2041ae8ba0c8d078422d36152cf1d35f1293c94a4074fe7e83791743ae",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-fixtures/ordinary_recursion.wasm": "e1bd7ea6ada8b15b2770ac387f8fc2256f20d7f87c4bc34e7559b239d8875ba1",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-fixtures/table_controls.wasm": "458f2b111e48d2efca98083b8c56860c43e82be8752813d1e2814d23c28d4f43",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-fixtures/tail_calls.wasm": "3e23be294e7a9eda62c89dbc366b6a6cab90edfbda532da27a7e4fc6d0aa4a56",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-fixtures/try_table_normal.wasm": "97068a6310a3bc535369e2b6277b801875c14ca206a707fcb9bca9036f41d0df",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/00-encode-fixture-00-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/00-encode-fixture-00-stdout.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/00-validate-fixture-00-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/00-validate-fixture-00-stdout.log": "009d962905920ad0e3ff46c6987fad36418982deb81796fd1f58e326d167c268",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/000-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/000-stdout.log": "96b7819189e0b1a95b7af83c51c5c351317816ecb78840cba2653a6e9b842889",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/001-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/001-stdout.log": "c272e3e881ceccdff8eef01dcaa1c991744b91a2252e89f76f071fd2713f79d2",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/002-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/002-stdout.log": "0d6acbf324844622099986639343ba17265965ef6234829e6432a0ee34c24249",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/003-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/003-stdout.log": "a2e64272471adcd13e87b1844ee2e3f78955cb358486ada759d5f5a84b9470bd",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/004-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/004-stdout.log": "12a340d1c4c7bc46b48e4c3b30d205c78ee0d9a8e43032d25b2433f0073afcda",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/005-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/005-stdout.log": "74342d40b15e31888a2226726267cd93a040dd8c4c1a43c903c00ebac3988e8a",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/006-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/006-stdout.log": "36ffce3e71c98d5d13a71731ad9e47269c0598251f6e822fccea167962e8da3c",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/007-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/007-stdout.log": "05883c054a5a06725ab41604c92c328929048eaf843f2db69e82fbd58f982102",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/008-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/008-stdout.log": "fd58b39296f402bfb25821aae75d58e87cdc7bdf04d18a9ad93dc1e7ac865e71",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/009-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/009-stdout.log": "0b74ecd7eb6c32c94980912ee23138be161e46cd9922fb332477cae6c12d18c1",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/01-encode-fixture-01-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/01-encode-fixture-01-stdout.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/01-validate-fixture-01-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/01-validate-fixture-01-stdout.log": "009d962905920ad0e3ff46c6987fad36418982deb81796fd1f58e326d167c268",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/010-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/010-stdout.log": "48093dcf3af611e2de1fa93717fc0d79a510eb48b1844eb65ebbfcc957def13f",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/011-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/011-stdout.log": "21aa80e9c3cb67c858dfb31fefad03c72d5154d11faed562da3d4b278a999be1",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/012-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/012-stdout.log": "e8a6cbf347dfd258c84e46bb47770be45ba2489a437953d2df7bb3e2f285fd66",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/013-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/013-stdout.log": "f5b3f4c72d819544af9d91c00c23fc68b7962edf01653dbab0a4c43abb122814",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/014-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/014-stdout.log": "36a83d6bc3a1d759941fe3ab6b794b12cbe6154fb2aeafde328ba987d31502e4",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/015-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/015-stdout.log": "a187c72a9b8a1f85a19d97b635fd6e48e6f1e1c1e9461b5ecad6775dbd1d1db1",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/016-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/016-stdout.log": "8c64263da4e82c04705dbbb0e45edae61cd5de65f878c16860a91f8caef575d2",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/017-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/017-stdout.log": "b6fae2add8daa08c0dd0dcb1e3b2cd936a31104db663ae7653317992c0941757",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/018-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/018-stdout.log": "d4bf5dc303e06372b54679a9d58cf56399e6370056d5152e649a75a658211083",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/019-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/019-stdout.log": "eda72c399407ee0bada929ffb7dfd7ce75171a80bfc6f4dcb638580fc98d1abb",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/02-encode-fixture-02-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/02-encode-fixture-02-stdout.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/02-validate-fixture-02-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/02-validate-fixture-02-stdout.log": "009d962905920ad0e3ff46c6987fad36418982deb81796fd1f58e326d167c268",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/020-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/020-stdout.log": "8f0d6acca9bbd6a03680132731ba221450d90ac2288e150f88e3f0e5810fc12f",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/021-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/021-stdout.log": "5fa448c3263622291976b430863670697c4c655bc87ccabfdf81c3c2d1ae8a42",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/022-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/022-stdout.log": "7730e3dad94560e17e6d6cb19bdcfd1ad872a6ba1d20bc4d6b1e72b967cbdb93",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/023-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/023-stdout.log": "cb1a2437cd4707bf22679ca94978da7101cbc8042af52427b523fde6b656fbe7",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/024-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/024-stdout.log": "549a32ecf182f57a21b7f555767406e7d82cc1d6668c75158f6592db9dca9782",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/025-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/025-stdout.log": "101fb58c6d513a42285c7af12fe12194058446e3fd019a9ce4d7ccc7c8f13918",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/026-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/026-stdout.log": "d9ae1404938e2e35b45ed682b44f98ebbfede95126fefc67e40ccf57eb7f757e",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/027-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/027-stdout.log": "4abc1e105888c85f9673e9daa0f213b56690f27a82daf64d4f6c2eb9263b6d9c",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/028-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/028-stdout.log": "9a6666ccfe9daf33f2f82802c27bd9079c4ffd295e34a69879585389d17b62f1",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/029-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/029-stdout.log": "919a72f7fa56fb79c273369527ff579606bac2bc1edbb17054b7e54e8ec8acab",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/03-encode-fixture-03-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/03-encode-fixture-03-stdout.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/03-validate-fixture-03-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/03-validate-fixture-03-stdout.log": "009d962905920ad0e3ff46c6987fad36418982deb81796fd1f58e326d167c268",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/030-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/030-stdout.log": "cfe085e04aa2e9d3798bd4bfc3161ab357b48a3cddb47ba2f2c0bcb134f90711",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/031-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/031-stdout.log": "d8242007d51128a694ce3d3ed324b65861d0f49d00f3765817f2636b9711559a",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/032-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/032-stdout.log": "3851d2ebe6b49119b614c430a5484cd415f37a0f394c3a8a695ecd324e300e17",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/033-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/033-stdout.log": "14b4e91f5f891b637689d8fdbb66d2b2c54b4db4501ce4052cde01ce1383b947",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/034-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/034-stdout.log": "cb4556eeddf3c3a98ba2934cd43a13d4f0c69146d8bcb728da2f7761d3b768bb",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/035-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/035-stdout.log": "a4b9e417f17c55204154bea6d27596a3f2884eddb8d7b4e3fb05ed3d532c8933",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/036-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/036-stdout.log": "db8c1291929b15cf3dd7b4cafb49245b03ff7f1684439f4cd58c8d2c364496b1",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/037-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/037-stdout.log": "d335366ebb752643cf80086af70084db5207da9edd0fa47979ac79213c348b47",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/038-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/038-stdout.log": "131cc6a79ded0478fdfc5c5fe93d31acf37fe7a2edd0474f4aae15883b3a165d",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/039-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/039-stdout.log": "86f846c2faee21739e64bde190dbae0969be6fe8e2a051f65b6afec65d1203cb",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/04-encode-fixture-04-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/04-encode-fixture-04-stdout.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/04-validate-fixture-04-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/04-validate-fixture-04-stdout.log": "009d962905920ad0e3ff46c6987fad36418982deb81796fd1f58e326d167c268",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/040-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/040-stdout.log": "83cbf07625483424e8a4a95994d2cddf5a39c1a9cf57fff54a5e1e00a672a316",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/041-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/041-stdout.log": "e0e2a3a70afa22462ce702a635901728b72467517ebb1a9e1b3cb51ca160941a",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/042-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/042-stdout.log": "e4113c61c547b96ca9a73d196b444540fa96a145db546ad968b14a818b32ffaa",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/043-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/043-stdout.log": "f3f6523f6b28a93ad9e58b111f6837bfe758ebb907e92a81c1e9d0fc6c58125e",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/044-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/044-stdout.log": "674b44d9f5c212d08e72856c7c58dda7d25010201169ca49a22366d550c267b9",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/045-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/045-stdout.log": "f75d54d027a8808c330a72c37a790c7907fc1d6fb94dc371b5d81419c60d04c4",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/046-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/046-stdout.log": "01e031e31cbb10d816a7e8bebf26c4bb6d3fc2c5ad5faa3f8fbf68f51884d174",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/047-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/047-stdout.log": "1342fe55f5ecea52a73238236edb9c4f2ded9892b4b0f023fc9ee62ce57c4fd1",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/048-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/048-stdout.log": "62e1889c1e8f7a2bb6ae45628fe137cb038791c4ad2ca8d5d95e488568247819",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/049-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/049-stdout.log": "1669d78f841cc127657f85fb57170c6e7721b6d27b608fd824508fcfad09b241",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/05-encode-fixture-05-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/05-encode-fixture-05-stdout.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/05-validate-fixture-05-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/05-validate-fixture-05-stdout.log": "009d962905920ad0e3ff46c6987fad36418982deb81796fd1f58e326d167c268",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/050-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/050-stdout.log": "8dadf3331253406da56083354b8616bd481805dad45077c6fed72a17c04ca0c6",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/051-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/051-stdout.log": "3767392f03c1331ed0782a3b941c5be2618488cb81f448435804bec58c342752",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/052-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/052-stdout.log": "926f84b58dda5b34127b8edd32b80cc918110eb2b49adc97b9d4540cdaa4e637",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/053-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/053-stdout.log": "87fec322cc75d1c9038af2d9468c666efd434c399a1bbd86bec24013847109b3",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/054-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/054-stdout.log": "ba62b09159e61cc6a100489e866a09a12073e5b3f442a834704f6240a75f5fff",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/055-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/055-stdout.log": "f9a33f298aa678b1e5fa0a80c08aecf1104a2f73fd28aa73abae81a54a2e94b7",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/056-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/056-stdout.log": "53b24561be381fd4159581fce84075dc5166cd7bdef50303fa8cbd93dfa41b6e",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/057-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/057-stdout.log": "e34fd1c86988d0077a0dc567c0ea9f0bb7cdef6d2718aad631763ad17266c728",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/058-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/058-stdout.log": "af2240dc12f2bb3a5818cf7cce7b2e2b37a3cdb0d8445a663082e3c4028c9096",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/059-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/059-stdout.log": "35d8511d7e8a14dae9b171cc139061cdd912093be689609c6028bf156268ec00",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/06-encode-fixture-06-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/06-encode-fixture-06-stdout.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/06-validate-fixture-06-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/06-validate-fixture-06-stdout.log": "009d962905920ad0e3ff46c6987fad36418982deb81796fd1f58e326d167c268",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/060-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/060-stdout.log": "ed9bea12e3dc0415322bbb5e7d10931160d33a7073a592b9aed84a471370c03e",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/061-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/061-stdout.log": "198f02a2f116fc3b197a4e8d84f1365393f8ee4b4b4ef063dd7d6e4480578602",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/062-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/062-stdout.log": "4f62a84a3508326f80c61a68d1327a1045d1844683b8132c0cfb5d5ed7413cb4",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/063-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/063-stdout.log": "91acda47d1936d3c31907b4b1c95b318ea2ff56f27b7672aa211ad9bd2a9dc6d",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/064-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/064-stdout.log": "db2c5c5d43cabb130047f3a673f14c2daa672b1fa393e53db0c391d371594885",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/065-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/065-stdout.log": "45e8ea18f53fd5ad9ab9492717c508f18c8e382e78e1896719713a486fdc2b8f",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/066-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/066-stdout.log": "7084b02a09d5f1123cf22d1aa90e8e21fb1c66e56acbb572ab2f783e95aed5b1",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/067-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/067-stdout.log": "77cf0d69867406894dce89dc6b3acd77e67c37d0d498040066cc668c997dc33b",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/068-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/068-stdout.log": "1f14b84cd5bc4a1ae2ccc59da5b284a07684761f565d798a39542c680344908c",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/069-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/069-stdout.log": "f5642fd83abcf39dea640eac0ed834b960b64d3dd3c37f1dd9b8250d0cb83f10",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/07-encode-fixture-07-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/07-encode-fixture-07-stdout.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/07-validate-fixture-07-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/07-validate-fixture-07-stdout.log": "009d962905920ad0e3ff46c6987fad36418982deb81796fd1f58e326d167c268",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/070-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/070-stdout.log": "cb98286da9e21318a567c90b107adf68a07478a2926c218eaaab50c855ebab4e",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/071-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/071-stdout.log": "939beb936c2f7965e1eae33802051cf609146b9177fbf8f1d42927a60977a510",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/072-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/072-stdout.log": "299945b3cfee242b75115be9b3f66d2c96f4c1a35e3128a319aa71c7e22b9dce",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/073-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/073-stdout.log": "7178c033f02c2ab093390e9795b5f1d132dded515c54755fcd86e49e293ec78a",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/074-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/074-stdout.log": "d8bb175ba5d7bb6bc9c189eb62cf45c2076f427a5b46d8c364ac6266d3673050",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/075-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/075-stdout.log": "dbf68296d2515384168d778668c4fb0b90c5e861c97cb11ccbb4eb545257f2a2",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/076-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/076-stdout.log": "055569e1dd20785538b35c6cb529b42190e341854237762b8ee93ee92e8bed35",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/077-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/077-stdout.log": "838233c7ac446bfb1b7009309e419904ba7df9bad1b70f0722ec24ef3fe6be97",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/078-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/078-stdout.log": "3d2ab56c353171540673107ec9760d3aa5cee9b7d93a96af042715ff7e4cf418",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/079-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/079-stdout.log": "ca31b2464cd007af003dddccdaab629b6a4820de2416e6961382c8861d7a7e0b",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/08-encode-fixture-08-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/08-encode-fixture-08-stdout.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/08-validate-fixture-08-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/08-validate-fixture-08-stdout.log": "009d962905920ad0e3ff46c6987fad36418982deb81796fd1f58e326d167c268",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/080-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/080-stdout.log": "c647f40e5c0be4d0ed206bb83db9db9d3a4bea8ed6cd662f5169f2de67227168",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/081-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/081-stdout.log": "3523145a47c12a8baff22efb741c293abb7281c0e93c09b8ce9dce079d5d32e8",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/082-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/082-stdout.log": "d1d5fb8d57eb593287fc248c6c4b67a7197b5ad8bb9cc3f6d95d3f961500382e",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/083-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/083-stdout.log": "b67883d640e6fd651ea5e1d3a20237c6f7964bd7a14f6611eaf03e09d0898692",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/084-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/084-stdout.log": "22cb4f327d3928b90e2b73ec2d0657be5983d4b9ab8333a5df43be2a957e104b",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/09-encode-fixture-09-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/09-encode-fixture-09-stdout.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/09-validate-fixture-09-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/09-validate-fixture-09-stdout.log": "009d962905920ad0e3ff46c6987fad36418982deb81796fd1f58e326d167c268",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/10-encode-fixture-10-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/10-encode-fixture-10-stdout.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/10-validate-fixture-10-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/10-validate-fixture-10-stdout.log": "009d962905920ad0e3ff46c6987fad36418982deb81796fd1f58e326d167c268",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/11-encode-fixture-11-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/11-encode-fixture-11-stdout.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/11-validate-fixture-11-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/11-validate-fixture-11-stdout.log": "009d962905920ad0e3ff46c6987fad36418982deb81796fd1f58e326d167c268",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/12-encode-fixture-12-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/12-encode-fixture-12-stdout.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/12-validate-fixture-12-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/12-validate-fixture-12-stdout.log": "009d962905920ad0e3ff46c6987fad36418982deb81796fd1f58e326d167c268",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/13-encode-fixture-13-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/13-encode-fixture-13-stdout.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/13-validate-fixture-13-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/13-validate-fixture-13-stdout.log": "009d962905920ad0e3ff46c6987fad36418982deb81796fd1f58e326d167c268",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/14-encode-fixture-14-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/14-encode-fixture-14-stdout.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/14-validate-fixture-14-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/14-validate-fixture-14-stdout.log": "009d962905920ad0e3ff46c6987fad36418982deb81796fd1f58e326d167c268",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/15-encode-fixture-15-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/15-encode-fixture-15-stdout.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/15-validate-fixture-15-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/15-validate-fixture-15-stdout.log": "009d962905920ad0e3ff46c6987fad36418982deb81796fd1f58e326d167c268",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/16-encode-fixture-16-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/16-encode-fixture-16-stdout.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/16-validate-fixture-16-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/16-validate-fixture-16-stdout.log": "009d962905920ad0e3ff46c6987fad36418982deb81796fd1f58e326d167c268",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/build-stderr.log": "da08ca5258880e50721a9e7c312ec3d02e778741a4d4a4434f82f8ace4bd1abe",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-logs/build-stdout.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-tools/wtime-cli": "c543c8a9690ddf9e5df10548b87bc476a7cc7bb9cc33031030003f4e01f776ab",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage-tools/wtime-conformance": "5123b38fd1e6b2358027778d94256202e53ff7024b3b05cd94aa5ea50d7949d4",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/coverage.json": "963299850342fbc8e3515d41a410678b87ded455df446a6d8142c8d691ec490b",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/decision.json": "056d7e2aa46a37ad0fba0ae577296c3b06ceacf66eec04fd6569a4a9ea46db3e",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/dependency-budget.json": "9f9f5075f118b58955f5f99913dbbaa2168403f3848a2985d5af95cf09a14b02",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/durable-LICENSE.txt": "f7943250f31583965e5fe9da6f49a36e0ddbae187ca59e561956e61dbef282a2",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/durable-fixture-provenance.json": "fee713f1cb256cc0f7faa542ea068bc580a61fea6b2aae05f8eca5cc4f5f5820",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/durable-manual-oracles.json": "9a44dfa77a5c7035dd2598e9115f32c7e89327f3802260e9fbd37f1dcbc6450f",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/fixture-logs/direct-numeric-encode.json": "608e5fdb48ddade82b5b11c24445e6288b8f0019734aebeb1d6cdf8a2ba0df22",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/fixture-logs/direct-numeric-validate.json": "769e21e83cc2058c56094b438655419a301a9efdcfcf24495dddd92649ece609",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/fixture-logs/host-calls-encode.json": "450a62e10089d517bd2bcab5e5de4cdcd5d7d0048b3d97a8532e001d1b19b414",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/fixture-logs/host-calls-validate.json": "0d6bab651a19c2e9835e6e2c85605bf753973a1011dd0e83071745e78aa455b9",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/fixture-logs/host-gc-reentry-encode.json": "9349076db0af1a0fad922ad4ec5d4fbdb885f4857d0048cf3ea80d79a4907303",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/fixture-logs/host-gc-reentry-validate.json": "801c397dafb7b8fb53b003970b086c08cf4501edc99b7476d0e98ee57ab4d4fc",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/fixture-logs/import-consumer-encode.json": "8917d45090b254dc1fd29f87ab7a1e7ded567335cfcfc6df651857a07460fabf",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/fixture-logs/import-consumer-validate.json": "96ad57edc289a9afec77d9ad41bb60fd2efe0a0717cf674b74b6cffb70a9575d",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/fixture-logs/nonnumeric-target-encode.json": "084c9c574d04d39638ed25c45cba90fd5be155fc74660703556072849ee04cb8",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/fixture-logs/nonnumeric-target-validate.json": "c6901020242781a0c0dee78930bdd19ca5f37b00f181c156828b4bd744e0a6f9",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/fixture-logs/provider-distinct-encode.json": "9ecb8bfde31deb6e0eb7120911c46725c68a404e9047eed3a302adfd539c5ed7",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/fixture-logs/provider-distinct-validate.json": "68c2f0f72cd0230753be54dc968d6aa174ad67fe90c95ae81c91eca3356e9119",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/fixture-logs/start-failure-encode.json": "2c3c1ac4c610092309fc298784627848fb939f9ac3ccd80af720d4363100dc07",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/fixture-logs/start-failure-validate.json": "92f167fd152b1dc4ab49261d231633de8a5f507b791921b44c945d338b6cf01b",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/fixtures/direct-numeric.wasm": "f0f5af5ef6195d7d75fce063f5dd21c94c0ae1314f1e101551b1271cb5cb72a5",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/fixtures/direct-numeric.wat": "33277df6bb7b62b047a2685ecde658dca7e94ff7e00b0c74495e2131a3c6297c",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/fixtures/host-calls.wasm": "15b9cb899308cddd924567943f9f328bcab747da576ecc887404828fc882b5f7",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/fixtures/host-calls.wat": "38e399719239cb52b77343d7fd25d996569351813372c87bc604424c12c38a5c",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/fixtures/host-gc-reentry.wasm": "ee9d9230a09091049d3b16e67a3b101b614e4b78aa34a83ab5ed48495ad15225",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/fixtures/host-gc-reentry.wat": "5d587d3b03caf02dfc2873b1600eefc3365fa93aea056bf0fe8f32bfa859545c",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/fixtures/import-consumer.wasm": "c81d3905169fcaa100ed5f533859674789dc88e4dae8e2f4b6c493b4df5bbd6c",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/fixtures/import-consumer.wat": "3814afa605ed13f2a7c2ab680e673314498c40a1d50865911d8973b35f3cdb96",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/fixtures/nonnumeric-target.wasm": "35067829e8e146610779760880e5b6f66086bb9a2bf6c60a915b8dd31accdde9",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/fixtures/nonnumeric-target.wat": "55b87f0a8c2aec1755446296a17c1b5bb3723cffd47991426af6c5bcf5f9341a",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/fixtures/provider-distinct.wasm": "c437ceeb695be74ee0dd84b5f37ff8a940444db3a5a01427d87bb847c0b92dde",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/fixtures/provider-distinct.wat": "b96355180129458cb22b58efcd811ad468a3772d79af64d3b4064b600575aa37",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/fixtures/start-failure.wasm": "fe4eb2e8e5bb355cb9f419dd787e97b925518ae4b1335177229a99bce0a90a58",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/fixtures/start-failure.wat": "83f76fe5eddf2d7ab2dff6dd5c72421367ba0e371223b296bf4863df49798866",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/licenses/LICENSE.txt": "f7943250f31583965e5fe9da6f49a36e0ddbae187ca59e561956e61dbef282a2",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/manual-oracles.json": "9a44dfa77a5c7035dd2598e9115f32c7e89327f3802260e9fbd37f1dcbc6450f",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/preserved-initial-draft.wat": "3c78c3f245a6bacdcb24c64c30bca9e1336fc58807a0462882e085618f7951fc",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/production-build-logs/helper.executed.py": "2ea2135a38a8411a6c3bb6b4b0b963bd99be4dd9baf27596bc39eb705d0356e8",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/production-build-logs/protocol.stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/production-build-logs/protocol.stdout.json": "6cd26bb91d957f210d141ba1136291979c47f476d0577a888e5d7c1f7ec648e6",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/production-build-logs/stderr.log": "55a5f953542ab562c8bd046a76424c2789b954fdcc5fb85c8de2a55ec83a5e92",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/production-build-logs/stdout.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/production-build-record.json": "c9ee815a40c732a7688e1b4b6b8fbee316c5ea4c87d45fae736ff887c83de353",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/provenance.json": "c09721ab4e163eb5e1fddc815f8dba24ca83e159f2c7be22d6360c191cade8df",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/scalar_driver.staged.rs": "696390941c96748fb621b5947631a121c4b7a0a81d70babc947e150832a5b337",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/source-snapshots/source-after.json": "8969eff089de087579639a5bdf09e1498c154052984ace9d6fa31b9e284b2e08",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/source-snapshots/source-before.json": "8969eff089de087579639a5bdf09e1498c154052984ace9d6fa31b9e284b2e08",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/source-snapshots/workspace-source-after.json": "ea967972eb0414dd292bcc1018ac48e67d012eede9d192e024bb2f49833f8148",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/source-snapshots/workspace-source-before.json": "ea967972eb0414dd292bcc1018ac48e67d012eede9d192e024bb2f49833f8148",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/test-binaries/default-runtime-suite/core-fc4384471282e053": "70471f44c11995e9df3b3161a5cf8269af8c8e2f692a7c45e2b82c1bba4521e5",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/test-binaries/default-runtime-suite/exception_root_safety-863d9cca58697c87": "3fe682f52a96f23f3fcb510270358776be26e0c6ba567d6030d7e8eb3e26b370",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/test-binaries/default-runtime-suite/exceptions-dc82360c779184c7": "a22ce38076bbfadfcb5dbfbbd6785f350aecc692b3dc9c2699bb988d9f40c36b",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/test-binaries/default-runtime-suite/feature_profiles-da219ada3555dbdb": "1c762443888591a23490ae658c1610ef54aa2247d44f5acae2d4b02f0778a597",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/test-binaries/default-runtime-suite/performance-f1f918c868572e47": "062d74ae1696cf9f7664dbc04d93628b2b8f6dbf1d5f522230b01fc845c63be4",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/test-binaries/default-runtime-suite/proposals-ce7c5c5922b88848": "b1ed11f2e74a6407a695313447ac1b7f4de1c0d1ae80fc84cb50efc207a90243",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/test-binaries/default-runtime-suite/wtime_runtime-670f0c395211a4f7": "cd5ec1a0df0005fa4f32b1902e0449d0804d1fd289ea46e56bc5cd1a55310a88",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/test-binaries/p038-regression-tests/core-42d92ac3afc77304": "ff9fba8ac18a40bc5db70ae5d48ebd27faf89d8b8abcf27c9210d9897b9020a1",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/test-binaries/p038-regression-tests/exception_root_safety-db67ff33a61a98ad": "8ea1f6604bc1babed150796188e74002dadfbc86ab725b21a285ce63e389caf9",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/test-binaries/p038-regression-tests/exceptions-5d275ecd0f4f002b": "e870b07866855d42d1b1a23341cce15ac4fe77a12f787da5377409952ed47d70",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/test-binaries/p038-regression-tests/feature_profiles-487957d4b265fccb": "82646c66abae0ff19782bd5607d144db83dc76f58a7d90a92b0baa67837b0613",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/test-binaries/p038-regression-tests/performance-1a202aeb20ffb000": "784e6365fe1a6e43fc5dc122af1845b8358841008a19a4c3bc492cb11133c301",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/test-binaries/p038-regression-tests/proposals-2b774c23db30396c": "3519b3cf22c8bd470c474800257c4090c4e3d8667dc3b96e302f88631213739c",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/test-binaries/p038-regression-tests/wtime_runtime-37e3c598ab512be0": "3689a62e9fdf4bce9466f09be7fa31e97dfcf71dfbab184f6a54dc27ba01c333",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/test-binaries/p039-regression-tests/core-42d92ac3afc77304": "ff9fba8ac18a40bc5db70ae5d48ebd27faf89d8b8abcf27c9210d9897b9020a1",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/test-binaries/p039-regression-tests/exception_root_safety-db67ff33a61a98ad": "8ea1f6604bc1babed150796188e74002dadfbc86ab725b21a285ce63e389caf9",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/test-binaries/p039-regression-tests/exceptions-5d275ecd0f4f002b": "e870b07866855d42d1b1a23341cce15ac4fe77a12f787da5377409952ed47d70",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/test-binaries/p039-regression-tests/feature_profiles-487957d4b265fccb": "82646c66abae0ff19782bd5607d144db83dc76f58a7d90a92b0baa67837b0613",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/test-binaries/p039-regression-tests/performance-1a202aeb20ffb000": "784e6365fe1a6e43fc5dc122af1845b8358841008a19a4c3bc492cb11133c301",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/test-binaries/p039-regression-tests/proposals-2b774c23db30396c": "3519b3cf22c8bd470c474800257c4090c4e3d8667dc3b96e302f88631213739c",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/test-binaries/p039-regression-tests/wtime_runtime-37e3c598ab512be0": "3689a62e9fdf4bce9466f09be7fa31e97dfcf71dfbab184f6a54dc27ba01c333",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/test-binaries/p040-regression-tests/core-42d92ac3afc77304": "ff9fba8ac18a40bc5db70ae5d48ebd27faf89d8b8abcf27c9210d9897b9020a1",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/test-binaries/p040-regression-tests/exception_root_safety-db67ff33a61a98ad": "8ea1f6604bc1babed150796188e74002dadfbc86ab725b21a285ce63e389caf9",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/test-binaries/p040-regression-tests/exceptions-5d275ecd0f4f002b": "e870b07866855d42d1b1a23341cce15ac4fe77a12f787da5377409952ed47d70",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/test-binaries/p040-regression-tests/feature_profiles-487957d4b265fccb": "82646c66abae0ff19782bd5607d144db83dc76f58a7d90a92b0baa67837b0613",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/test-binaries/p040-regression-tests/performance-1a202aeb20ffb000": "784e6365fe1a6e43fc5dc122af1845b8358841008a19a4c3bc492cb11133c301",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/test-binaries/p040-regression-tests/proposals-2b774c23db30396c": "3519b3cf22c8bd470c474800257c4090c4e3d8667dc3b96e302f88631213739c",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/test-binaries/p040-regression-tests/wtime_runtime-37e3c598ab512be0": "3689a62e9fdf4bce9466f09be7fa31e97dfcf71dfbab184f6a54dc27ba01c333",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/test-binaries/p041-focused-tests/core-42d92ac3afc77304": "ff9fba8ac18a40bc5db70ae5d48ebd27faf89d8b8abcf27c9210d9897b9020a1",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/test-binaries/p041-focused-tests/exception_root_safety-db67ff33a61a98ad": "8ea1f6604bc1babed150796188e74002dadfbc86ab725b21a285ce63e389caf9",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/test-binaries/p041-focused-tests/exceptions-5d275ecd0f4f002b": "e870b07866855d42d1b1a23341cce15ac4fe77a12f787da5377409952ed47d70",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/test-binaries/p041-focused-tests/feature_profiles-487957d4b265fccb": "82646c66abae0ff19782bd5607d144db83dc76f58a7d90a92b0baa67837b0613",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/test-binaries/p041-focused-tests/performance-1a202aeb20ffb000": "784e6365fe1a6e43fc5dc122af1845b8358841008a19a4c3bc492cb11133c301",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/test-binaries/p041-focused-tests/proposals-2b774c23db30396c": "3519b3cf22c8bd470c474800257c4090c4e3d8667dc3b96e302f88631213739c",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/test-binaries/p041-focused-tests/wtime_runtime-37e3c598ab512be0": "3689a62e9fdf4bce9466f09be7fa31e97dfcf71dfbab184f6a54dc27ba01c333",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/test-logs/default-runtime-suite.json": "97896d92ee66cd306e61934217ad11100b584be6fa75a234caa1672f77eb0049",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/test-logs/p038-regression-tests.json": "5236063e3f94afd859a44a83767cb40f3a32cec51a4a3dab278a4f5027d2f843",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/test-logs/p039-regression-tests.json": "6c73ad47efa9743bd0de47b196e92b41f2b75ad8057a1ea1c1dfdc6d4b41c0bb",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/test-logs/p040-regression-tests.json": "561424c40480b2798db8f1d28bca3d8096f577700f1cd0448e2fc3747b5a81f9",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/test-logs/p041-focused-tests.json": "1909d5d0ff48de24a1c4151aeaf4cf3e2d26c55e96410facd681f19f4c512d8f",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/test-sources/p041.rs": "71b15997f183fa4044b64a3576c3ceca7c31f7463b4e1d10faeff8a8e51da28b",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/test-sources/p041_private.rs": "8b6d1bc9a67259fa72b972b78353caccb2851b157face5bd85ca208a506670e6",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/tools/encoder": "5123b38fd1e6b2358027778d94256202e53ff7024b3b05cd94aa5ea50d7949d4",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/artifacts/tools/validator": "c543c8a9690ddf9e5df10548b87bc476a7cc7bb9cc33031030003f4e01f776ab",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/inputs/check_result.json": "abf172c8bb0d4225e646055adafe0aef4d59e9cfdbb04878b353624fac6c9520",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/inputs/closure-builder.py": "7b9208716ba15c05b23a6908813d04ca9185dbed279780d558706f495a773003",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/inputs/comparison_report.json": "182397a94c71f0364439c5f1d1a1e0eb8fe9408fed973219eec494f8100b2393",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/inputs/coverage.json": "963299850342fbc8e3515d41a410678b87ded455df446a6d8142c8d691ec490b",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/inputs/decision.json": "056d7e2aa46a37ad0fba0ae577296c3b06ceacf66eec04fd6569a4a9ea46db3e",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/inputs/prepare_receipt.json": "4142077116c43807003302e8dd3240254e5e967a156de53539f2937e17c8724e",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/inputs/production_build_record.json": "c9ee815a40c732a7688e1b4b6b8fbee316c5ea4c87d45fae736ff887c83de353",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/inputs/test_contract.json": "d1623d5f4a3547fb2c16f8aa4407fc03af0805631c68a86bb6423aa0025e60b6",
    "/workspace/wtime/target/performance/P041/closure-attempt-0001/instance-call-evidence.json": "3f304d94c175ebef95bda7c28d60ed1461b279f692c9adc3dbcf646451c0415d",
    "/workspace/wtime/target/performance/P041/production/build/attempt-0001/binary/wtime-bench": "e1c02db5ac3739a3fd3860b9a7d639711c22b9ab25b728cc68b39b0ac3bb7716"
  },
  "evidence": [
    {
      "bytes": 189299,
      "path": "/workspace/wtime/target/performance/P041/closure-attempt-0001/instance-call-evidence.json",
      "sha256": "3f304d94c175ebef95bda7c28d60ed1461b279f692c9adc3dbcf646451c0415d"
    }
  ],
  "outcome": "inconclusive",
  "status": "done",
  "task": "P041"
}
```
