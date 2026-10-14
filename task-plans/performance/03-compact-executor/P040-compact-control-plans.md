# P040 — Lower structured control to resolved jump and result plans

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

[P024 — Move branch results and share label metadata without allocation](../02-structural-wins/P024-in-place-branch-results.md)

[P037 — Map compact instructions to source events and control targets](P037-compact-source-maps.md)

[P038 — Lower constants, locals and common integer operations](P038-compact-scalar-lowering.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/execution/compact/lower_control.rs` — checked source/target, arity, and result-move control plans.
- `crates/wtime-runtime/src/execution/compact/execute.rs` — compact control dispatch and exact materialization/accounting integration.
- `crates/wtime-runtime/tests/performance/p040.rs` — named public behavioral and compact-path cases.
- `crates/wtime-runtime/tests/performance/p040_private.rs` — unpublished plan corruption and preparation-budget cases.
- `crates/wtime-runtime/tests/fixtures/performance/p040` — six fresh original WAT/Wasm fixtures, license, manual oracles, and provenance.
- `crates/wtime-runtime/src/execution/compact/lower_scalar.rs` — whole-function integer control admission and aggregate metadata budgets.
- `docs/benchmarks/performance/P040-contract.json` — immutable primary/protected workload partition.

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

P040's predeclared workload roles are frozen in `docs/benchmarks/performance/P040-contract.json`.

## Input/output contract

For explicitly selected `Compact`, admit the integer loop/branch families `scalar_br_table` and `scalar_multivalue_branch` as the measured primary workload set. These families exercise checked block/loop/if/else/end, `br_if`, `br_table`, and multi-value result movement. All other 73 IDs in the registered 85-row `scalar` selection are protected. Keep original labels, source events, branch results, fuel/cancellation boundaries and materialized state equivalent to Decoded/Reference; retain whole-function decoded fallback when a control shape or type is unsupported. Default/Portable remain Decoded, and no speedup is presumed.

## Implementation recipe

1. After P038, including when its optional compact implementation was retained following an adopted, rejected, or inconclusive scalar decision, freeze a fresh P040-before archive with `baseline.py --mode compact`; it must capture the immediate compact-policy predecessor. Predeclare the six `scalar_br_table` sizes and six `scalar_multivalue_branch` sizes as primary; protect the other 73 scalar-selection IDs, including n=0/n=17 for every other family. Compare the compact archive against the compact candidate under the same explicit policy.
2. Derive checked target/source cursors and parameter/result move plans from validated control metadata; retain compact label state needed for materialization.
3. Execute branch moves in place and record structural fuel/cancellation/stack events even when physical no-op labels are elided.
4. Handle multi-value loop parameters, unreachable polymorphic validation cases and br_table default targets without revalidating guest types at runtime.
5. Benchmark branch/switch/deep-nesting controls and verify materialized labels match decoded execution; record actual compact/fallback counts and run both fresh paired comparisons under the unchanged cold/build gates.

## Meaningful cases

- The six `scalar_br_table` rows and six `scalar_multivalue_branch` rows (n=0, 1, 17, 32, 1024, 10000 in each family) are primary; all other 73 IDs in the 85-row `scalar` selection are protected.
- Multi-value loops and nested if/else preserve every result and label parameter/result movement.
- Every br_table target/default, br_if taken/fallthrough, function-label return and zero-result return reaches the correct source cursor.
- Fuel stops on block/else/end and branch events preserve transient stack peaks and materialized cursor/state against Decoded and Reference.
- Unsupported control/type shapes decline the whole function; counters prove which selected cases actually ran compact control plans.

## Verification

Activate Rust as described in EXECUTION.md. The twelve primary workload IDs and exact protected complement are bound by `docs/benchmarks/performance/P040-contract.json`; contract roles select measured primary workloads; eligible integer control bodies in protected families may also use Compact. Keep the output comparison in a new subdirectory because `target/performance/P040` already holds preparation, fixture, and build records.

```sh
cargo test --locked -p wtime-runtime --features compact-executor,execution-reference p040_ -- --nocapture
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml --features compact-executor -j 4
python3 tools/wtime-bench/compare.py --baseline target/performance/P040-before --candidate tools/wtime-bench/target/release/wtime-bench --candidate-mode compact --selection scalar --output target/performance/P040/comparison
python3 tools/wtime-bench/analyze.py --input target/performance/P040/comparison --output target/performance/P040/comparison/analysis.json
python3 tools/checks/build_budget.py --output target/performance/P040/build.json
python3 tools/checks/build_budget.py --features wtime-runtime/compact-executor --output target/performance/P040/optional-build.json
python3 tools/wtime-bench/gates.py --task P040 --comparison target/performance/P040/comparison --build target/performance/P040/build.json --optional-build target/performance/P040/optional-build.json --contract docs/benchmarks/performance/P040-contract.json --output target/performance/P040/decision.json
```

Require at least three named focused cases, positive compact/fallback work counts, exact P040 source/binary/fixture/report identities, both fresh runs, and the unchanged hard/cold/build thresholds. Preserve unsupported shapes as decoded fallback. Record rejected or inconclusive outcomes honestly; this predeclaration is not a speed result.

## Measurement and decision

Selection: `scalar`. Explicit execution setting: `compact`. Decision rule: `measured_promotion`.

The primary family is exactly `scalar_br_table_n{0,1,17,32,1024,10000}` and `scalar_multivalue_branch_n{0,1,17,32,1024,10000}`. The family exercises loop/if control, multi-value results, `br_if`, and `br_table`. The other 73 rows in the 85-ID scalar selection are protected. Contract roles are immutable in `docs/benchmarks/performance/P040-contract.json`; current cold/default/optional-build limits remain unchanged. Promote only if both-run primary and protected thresholds pass; no promotion or speedup is asserted by predeclaration.

## Before editing

Run this after P038 has completed and before changing P040 candidate sources. This archive captures the immediate compact-policy predecessor, including P038’s retained opt-in compact implementation. Keep the original P001 archive separately for milestone totals.

```sh
python3 tools/wtime-bench/baseline.py --mode compact --output target/performance/P040-before
```

Verify that the archive manifest resolves to `compact`; an archive that resolves to Decoded is not this task’s predecessor.

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. Executed evidence is recorded below.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/crates/wtime-runtime/tests/performance/p040.rs": "0dd66f137da18081213bf740275ce9a07722beb456572529639637c00c143732",
    "/workspace/wtime/crates/wtime-runtime/tests/performance/p040_private.rs": "dcb3592b2b785c45fea707a9ba6ed6c4f5f69f2660b0e62e16fe68ab48c552d5",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/LICENSE.txt": "1126322e2cc8d165adc4c792eeb195717de2bcc7b39be1ce77959d78e87ef685",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/MANUAL_ORACLES.md": "8dbef840671d478c98fc34cd773548b8b18cca55453dc3605283fab31a0ef7aa",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/P038-decision.json": "fd4a0c0f02f163946e2af95ffd84a0db524f34408d2901193e57f57617788932",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/P038-scalar-evidence.json": "065c6bab3a71ebb70e70b077e2aaca73b1d9644511b891e1638085520e13118d",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/P040-before-command.json": "5460430134e0a4f34a1e1a1c94d7cf706143f3aef21458ef5c48674b18cd5852",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/P040-before-manifest.json": "6809a50a1caea384c0171c5e46ef9ef42c935987a0216529d23d141dccbab020",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/P040-check-result.json": "e5f1b00e4bdc79842f81f98af46189f79510c6e73ed42fdf1b27ecfec9702a72",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/P040-contract.json": "7b530ffb3af508df78ac490fa3656824c46848366e7a5fdeb85287eab4180339",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/P040-fixture-provenance.json": "cbf9d473d2ce5f97f7834c09375c98ee2bad31815c1c9d050d15cc5c31de0ff3",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/P040-prepare-receipt.json": "e53d55cd9dd88705a4500224b974ae8fa62bf3581997a85b3bc589e9df3e6c6a",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/P040-test-contract.json": "fa1b44094463ff1408f0590d705d26c3198b4ff38ecebf1208eac208892000d5",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/build-budgets/default-command.json": "2181cf542cdba8c240e10a5b3c491e0c13dbae7f4794c08c6005be264ed7ee4f",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/build-budgets/default.json": "f794621b1f234b5ddb36777c028de907874ca0ff64655530f33cfa7ce06ddfb4",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/build-budgets/optional-command.json": "bf618ba4a3eeaf262e275c155fbee389ae3d1c176e20078286ec60e13639fc33",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/build-budgets/optional.json": "c244329216b38f6c80ceaf65a99ef5a455ac54cac42698085974bef811330a50",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/comparison-report.json": "cc269b30e1de2fb3bb1f2e853d3bfea9c7a207e342f4945182619c59f5203b52",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-binary/wtime-bench": "008aa3c4d4f8a9765701c3acf8429fead8e24b0230d64f6de71dcffdbd4b6551",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-fixtures/accumulator.wat.wasm": "ef61bf7afc44af7b71ef78eadcba48c0d85949d7d9096df8fbbbab53c05036b1",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-fixtures/accumulator.wat.wat": "61aaec7a86d481378f31f3e27b0c1413684065841fa5909297181fcad9bef10d",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-fixtures/br_table.wat.wasm": "d43d5dd71d5ca1c3dae820aa85c13a2d69415e028b8d48097fed4558f743163a",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-fixtures/br_table.wat.wat": "637b96cd8926ffadf728af4949b43fa6b62fffb61968c14c716e0c203a1e5e6d",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-fixtures/branch_calls.wat.wasm": "9b58d26f201900b5b6cce684133a98ab68b3cf55d979b64ce895e914e9d5327b",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-fixtures/branch_calls.wat.wat": "d0af4fdf9ad5be38344267ae72fc1855edb5a8fa1b475fd0af4cb507730b2309",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-fixtures/branch_entropy.wat.wasm": "e4c87dbab50a2438cb1f749bc6fd6f9c834c81484acd42864735bd1ad1b6dec0",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-fixtures/branch_entropy.wat.wat": "6b4861d8bfbbd7e2718e577ec2d479bfff32e94dc47a3b7dda9a1b363baa98b6",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-fixtures/call_overhead.wat.wasm": "0815ca7d6026206d0773fe10c6b9643862bfffe8ba361c9789369ef9668ee746",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-fixtures/call_overhead.wat.wat": "9493b8f751690a2fd6e8b19dcee5e2c1d50b717253d292ce34d5692b306663a5",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-fixtures/call_ref.wat.wasm": "4f766dc9e29324ea942c892982363dec0ade98565288878a70967ce3b33f322b",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-fixtures/call_ref.wat.wat": "0323154b79000eeeb0a63310541ec91b6c4d91e32a6a3406125a30f2c8e53a12",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-fixtures/dependent.wat.wasm": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-fixtures/dependent.wat.wat": "0dd5e9778eb09bac9ee432e5fece3f432d034ea9878d2302972142501f0c60c6",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-fixtures/direct_calls.wat.wasm": "f6adbecce99c05bf06f213f4f5c0d5d9f001074acbbb2e6bbe814696bb866e45",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-fixtures/direct_calls.wat.wat": "0a075601a2a651f9cb95e8ee0404c64bbf076d17bf242b9c88fecbb8cb973826",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-fixtures/dynamic_api.wat.wasm": "c493d0aae68d0c5aa67fc0b562e616c45eb3d42b0302951158a763fa202ce589",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-fixtures/dynamic_api.wat.wat": "f02cc93a4a43691ce5be8782fa2df454fc2468a7935117ea18104a7dce22cb2d",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-fixtures/host_call.wat.wasm": "c60311d57c929c3ff1d29adb70501e7bbf8419517acbd86c8d072438b610ebb7",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-fixtures/host_call.wat.wat": "4c4026de620eff0b1e9cb4ed4e0440e6045d921267e3962715f6430ac68f5120",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-fixtures/indirect_calls.wat.wasm": "4b42e82f54ea6ab7d0f62c70766c904e41d6a408e57fa14b5abbf514c0da78a9",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-fixtures/indirect_calls.wat.wat": "29024ae23844664a9fea4a809d1ce414de40f243f7e678ce5ea6d811843f7797",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-fixtures/integer_loop.wat.wasm": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-fixtures/integer_loop.wat.wat": "791730112109f3d5751a7b3e2c1134e7e0e6f71a7f7ee01ea8481eb989ee3fa4",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-fixtures/multivalue_branch.wat.wasm": "03ffdc2041ae8ba0c8d078422d36152cf1d35f1293c94a4074fe7e83791743ae",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-fixtures/multivalue_branch.wat.wat": "a1f4958100af82889208bb28090d6af7c3e56cbeeccabad4106601c9a9c17014",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-fixtures/ordinary_recursion.wat.wasm": "e1bd7ea6ada8b15b2770ac387f8fc2256f20d7f87c4bc34e7559b239d8875ba1",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-fixtures/ordinary_recursion.wat.wat": "aca6a66637fa36b72f3c528dd5e3e44039b573f363b5b4fb77437e9278623903",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-fixtures/table_controls.wat.wasm": "458f2b111e48d2efca98083b8c56860c43e82be8752813d1e2814d23c28d4f43",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-fixtures/table_controls.wat.wat": "9b61bd9cbca804bf639085769fd766a2cf8a69d516fc7efb9e4edfe4ff6d18c9",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-fixtures/tail_calls.wat.wasm": "3e23be294e7a9eda62c89dbc366b6a6cab90edfbda532da27a7e4fc6d0aa4a56",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-fixtures/tail_calls.wat.wat": "9953f19384c51b579921e6f789b13a8b57c86ba86d760b77846e3e33825ece02",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-fixtures/try_table_normal.wat.wasm": "97068a6310a3bc535369e2b6277b801875c14ca206a707fcb9bca9036f41d0df",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-fixtures/try_table_normal.wat.wat": "ab92800dd4e4d4053498c317effadedf52568270ab7b476eef74b09638de8964",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/000-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/000-stdout.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/001-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/001-stdout.log": "009d962905920ad0e3ff46c6987fad36418982deb81796fd1f58e326d167c268",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/002-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/002-stdout.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/003-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/003-stdout.log": "009d962905920ad0e3ff46c6987fad36418982deb81796fd1f58e326d167c268",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/004-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/004-stdout.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/005-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/005-stdout.log": "009d962905920ad0e3ff46c6987fad36418982deb81796fd1f58e326d167c268",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/006-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/006-stdout.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/007-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/007-stdout.log": "009d962905920ad0e3ff46c6987fad36418982deb81796fd1f58e326d167c268",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/008-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/008-stdout.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/009-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/009-stdout.log": "009d962905920ad0e3ff46c6987fad36418982deb81796fd1f58e326d167c268",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/010-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/010-stdout.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/011-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/011-stdout.log": "009d962905920ad0e3ff46c6987fad36418982deb81796fd1f58e326d167c268",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/012-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/012-stdout.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/013-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/013-stdout.log": "009d962905920ad0e3ff46c6987fad36418982deb81796fd1f58e326d167c268",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/014-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/014-stdout.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/015-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/015-stdout.log": "009d962905920ad0e3ff46c6987fad36418982deb81796fd1f58e326d167c268",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/016-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/016-stdout.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/017-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/017-stdout.log": "009d962905920ad0e3ff46c6987fad36418982deb81796fd1f58e326d167c268",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/018-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/018-stdout.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/019-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/019-stdout.log": "009d962905920ad0e3ff46c6987fad36418982deb81796fd1f58e326d167c268",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/020-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/020-stdout.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/021-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/021-stdout.log": "009d962905920ad0e3ff46c6987fad36418982deb81796fd1f58e326d167c268",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/022-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/022-stdout.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/023-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/023-stdout.log": "009d962905920ad0e3ff46c6987fad36418982deb81796fd1f58e326d167c268",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/024-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/024-stdout.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/025-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/025-stdout.log": "009d962905920ad0e3ff46c6987fad36418982deb81796fd1f58e326d167c268",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/026-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/026-stdout.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/027-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/027-stdout.log": "009d962905920ad0e3ff46c6987fad36418982deb81796fd1f58e326d167c268",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/028-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/028-stdout.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/029-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/029-stdout.log": "009d962905920ad0e3ff46c6987fad36418982deb81796fd1f58e326d167c268",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/030-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/030-stdout.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/031-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/031-stdout.log": "009d962905920ad0e3ff46c6987fad36418982deb81796fd1f58e326d167c268",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/032-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/032-stdout.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/033-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/033-stdout.log": "009d962905920ad0e3ff46c6987fad36418982deb81796fd1f58e326d167c268",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/034-stderr.log": "182e0273ab1d85020feb801d5e48b162fe456a7158fff856876f75864df8bcb0",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/034-stdout.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/035-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/035-stdout.log": "5d61eaa55c9d12358f01919a9dfc9b8cbaf73dd46a1471925319f4496e53a009",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/036-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/036-stdout.log": "0437ba078e98725a8c7aa79950c9b4e748aa8d24b30dde0396063e5e2b7040f3",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/037-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/037-stdout.log": "3f4b675c82b7f36586133b62da2ad109c01601ad7bbeb1828fab36d02ae43fc6",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/038-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/038-stdout.log": "1b3df2684ba3ed063793275a431835573cd89d9cb5500f4b14c8f6ca3abe22a9",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/039-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/039-stdout.log": "09deb7ae1774178a51e8f48cf88926c00004758cbda9e4cef76723ec336f7881",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/040-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/040-stdout.log": "daccaa644f4e5720bba8c7de960b5369ad17ea25fee36b1c24496f58e8733f21",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/041-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/041-stdout.log": "1ea3726a65bd47df8940117e87d64eb64385edaf707b6e43b1c8abc171b29222",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/042-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/042-stdout.log": "a4d0521aea8b735ab990a472ff07504535c3638e19acccabd2d12ae1a73cd7e8",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/043-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/043-stdout.log": "dc10984ae1d65ce82b9f6bda8fc3174914f0091ac1784d5cf36f7daf533fdae1",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/044-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/044-stdout.log": "fee3bd23e709284479b8c1a0ef8f91ad849abb90bded308ee46e6b0e3c5b8423",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/045-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/045-stdout.log": "979275e62aa3e3edb43a3b33e0ea446ea97be97250886a9b6330e3d1a2e12caa",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/046-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/046-stdout.log": "097a8eca561ba9926edb79048903438d82bf8861088c87fee0c87659e10f9e4a",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/047-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/047-stdout.log": "551c9167b684babd74b1f18c38239a621d734bbb9feff23c35ac7f81e72da8f4",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/048-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/048-stdout.log": "1defda9c8a356dbe36eba5cfb6d2010c92fb22685c3f39c76a47539b807069bf",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/049-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/049-stdout.log": "9efd48f4a9009723c83ee8628281c5d26570b561aa5fe46548e97948ddead158",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/050-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/050-stdout.log": "758c8fc9926de040d750d276a8d0039898521336a313d12ac82f6c3d3776a6ae",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/051-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/051-stdout.log": "20b52f114794648795150ad773ee36794d77728ad01bfb2157d84933d41dc925",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/052-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/052-stdout.log": "aed3b2cb045ae5ca83cf4548c275bf25e23339bfa7c30d5ee8e6cedcf790540a",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/053-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/053-stdout.log": "1860ec4dcd9d76daca2210856ac142c44ce1d1f2354ae93015bc6b4f7eac5898",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/054-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/054-stdout.log": "1a1f1952915529be842560cf133489c8509b572a69f1c9d6cf1a65840b021f33",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/055-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/055-stdout.log": "6d2084f0711487cf44a6da1383d5cfbcef8ad5068c701cdfb8785494205a514a",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/056-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/056-stdout.log": "ee43aca04359cb789b24bbc8c896d97b731c12fd63d02f9ae9ffb0f4f0813b85",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/057-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/057-stdout.log": "5a0aa8f8bd2d9d69159e98e514e77b35f3ed8e8ad482f94f92c23c83685b2a03",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/058-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/058-stdout.log": "4183db3dff0b4bec79575468b6d210d307cb8426f6b9b19f243101ccd5c3c65e",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/059-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/059-stdout.log": "bb6bcd792ad15ca86cbc9fc3b0a0c5adc1009060bb851c1f3be00ed24feaf8bd",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/060-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/060-stdout.log": "704637658e44b196eddc8a15a841e47d91e0438f6a7feee195f3f22f563fabd0",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/061-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/061-stdout.log": "ccaf1d13b50a32e1cd14dac70a0a9ce1a2ada8209b9222328e94c15ee4d96cda",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/062-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/062-stdout.log": "7a87a7cb45aac1845a2bfaeb7be461a5ad26303f8439797a9fa4da72fb856196",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/063-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/063-stdout.log": "67ffc3178e3d90fe5f61ec223a82b3a9595471e779fbec9a5767dd8b6de1f661",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/064-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/064-stdout.log": "776e0150980777fce84cc69706d393e371974301f706e0820bfd1f49d418bfe3",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/065-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/065-stdout.log": "988fa6fa3a67bbee69ac4f1a9c5551cca911b148f8070930001db5c9e434532c",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/066-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/066-stdout.log": "332c4115ba201e11b65056b74f0bcb3d0eb8467c231f7354b3558ebc75e7ce3c",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/067-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/067-stdout.log": "ce7f4d24a83be485dddefc006c1ef316527a6c3ee24653893d69f8bb870ebdac",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/068-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/068-stdout.log": "e4c61d5fc3b6ad18854fc64c58f19ee5bcfe60463244cfdf3ec42abe2cf95b14",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/069-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/069-stdout.log": "90a81e14d67772e5b73df917e749816898c0091dd69a4eb331b7f689be3ed3d6",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/070-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/070-stdout.log": "2eae94bda49cd36807e151e93e4fa9ec7e4f42a99cc120b222411636c91166fd",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/071-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/071-stdout.log": "2dfd8c84154a30220bcbb7f1247fe08fbf9e084a970aec7c24548045f91fafc8",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/072-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/072-stdout.log": "ab3b81261438d3f633d7eb600b07e7a09a733d7fb7e4a744dc4f28294421320a",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/073-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/073-stdout.log": "bf53991fa2cd82da1af807f8fe47ea76e7bac8c8f0d591660f86b70a2f90aeb4",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/074-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/074-stdout.log": "8f05d6729dc62c209a0e14cb571c01f0e696db11efbf50b80601c7b4c50de5bd",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/075-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/075-stdout.log": "27fe5b02a91a5626b895fe13d3f90234079cb2afde37eced3d22795b6d69c8dc",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/076-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/076-stdout.log": "a4c36dbe96cc3913e285a04e7543bb66849675bc6a4341a78b2137280841dbca",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/077-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/077-stdout.log": "a86e0056d42476749f585a0a3eea29fcf387f5594b7ec64259d3a8104ade4f00",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/078-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/078-stdout.log": "8ca9973953a94b0367241f464ce263405c33cb521acfe2b546b39fbb03c86935",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/079-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/079-stdout.log": "7c4aabf023c52ce1a75828caff38b0ff3bb269e493032bf86bff63b20854fc5a",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/080-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/080-stdout.log": "875924a0dd0d51d33c97b5ec0693e7f6a86ec699300ccb0baf83425dd7a2be56",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/081-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/081-stdout.log": "ffb25ce9abd5020371fb0cf787cf83cb7975244a3f780c50bb1d414589191f0d",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/082-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/082-stdout.log": "7a714ba17bbcaa8f9b13739e9564d73b22aa7727ea2a45d19cd1ee463fd60c6e",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/083-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/083-stdout.log": "a239dd6124ef7e0a94db9638d90693b5bf2add8ee89de2d84d3d2a94daed0cb6",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/084-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/084-stdout.log": "d4b0db76c436e6895785e5f81c5f563480735f503c50aea3a0b25df2f2b033dc",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/085-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/085-stdout.log": "6acbd0c260b209dcee7a6bded83fc61b00f159efa17d39dc920c735dcd44ed0c",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/086-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/086-stdout.log": "a1209a748ce98b623b7398e3df9ff4afd85ed296649df988178dd38f99de4624",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/087-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/087-stdout.log": "ac4c36ae6ad60c42986e60c0fc7f370da7db542be2093a8c36072fb9b0972c73",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/088-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/088-stdout.log": "a09467cf7d59c3b71edc618b9c169cf159d1eb02008eff6e9e2213b5b84f3f06",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/089-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/089-stdout.log": "f1082dba0c766c2ed5f09fa1841123eb63bcb14b4f4bad1bed0069f6928de875",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/090-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/090-stdout.log": "85a21b0ff5ecff4f03f68d075b843d1f009984347c62a558c778f14823bdef71",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/091-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/091-stdout.log": "3f957b63e8504f533f1cac98d2bbe9f1ed9799c8e170d52a33b18c5ba22cb90e",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/092-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/092-stdout.log": "76c1cf4c99cc006817109521d8fffe8ba3fbf16cfc78bc5d262789a89eecaa34",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/093-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/093-stdout.log": "5f25e03cdad54bcdef3bb3a545451b32b3411497cf9c48d5decbc2862d393548",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/094-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/094-stdout.log": "c35286bb632e9cb13a78f9decd6165c285b6710fde7f9c3d89c5a71e1cc3323f",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/095-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/095-stdout.log": "79f20e818ca4965715a111dd964167e8569ef872c784d1aa8c8be03737884ae0",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/096-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/096-stdout.log": "fd8388d7232761305c72cd9b9cbf61f1aae8e841242fa4cd06d698a173267005",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/097-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/097-stdout.log": "75357edb65813f9d41f0b6c9d89c90757db3e631b0a65574ab698ca5f295c28f",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/098-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/098-stdout.log": "e8f0aa726a74a4a4023f472a47ce9c81e32f06663030d91f4d3955a40a0c634c",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/099-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/099-stdout.log": "fefede85a19595dff49fc06480d73cfd4844fe81818202c20080aca0cea10215",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/100-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/100-stdout.log": "b5e7fe215141967275b20f1320bcadc62f6b3ebb7db09dc405e374df50dd5119",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/101-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/101-stdout.log": "37bd7371785b4dad8743eb9ffaafa324ca23001893e7d5aae41e8fe4d3d24f06",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/102-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/102-stdout.log": "bce7e127819aee32c1b02e0da229a982d7e1c86bb7241874f11a6725139bad5b",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/103-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/103-stdout.log": "6f83d788ad42d4c364052257f056c980301f687e55b32fa4ff3d9c6ba2e64865",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/104-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/104-stdout.log": "2276e86fd2aefa9df640315dc4c9ec4f06ead175d4aca68e7e6f8a5193818281",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/105-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/105-stdout.log": "0269c8c9520a81abe53799a6a4bca90309b3faed30ccd67d21304dbed3b206ec",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/106-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/106-stdout.log": "e098469a5e5fe64b61d5a24538103cfe3b2a031d513f97f116ebd4c14d53cc4c",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/107-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/107-stdout.log": "3253606f73edfa0f7aeac01627312b2d07f3c36b0f5e4d3f55266b0eb720840c",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/108-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/108-stdout.log": "237f2984bf0e821bcf0546dc37db48e77ff5dd97e43f5deaac1f758d578eef20",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/109-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/109-stdout.log": "144a9c7c3e2042de10876668251dc8ec0aba30e331fc759c51fbc5676bb90010",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/110-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/110-stdout.log": "dfe85f2b1d37f081ceb77d646589bc0eb18dd99b0190bf8bd8ceb271f7936887",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/111-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/111-stdout.log": "08b67fbf4749b872c74de8910605abb6d46c038869cbc32ff75292be56b8806b",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/112-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/112-stdout.log": "0f4196ff569dc6a7513010916c0b9e63b847fd5865073c2fd4171bdfaf481e84",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/113-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/113-stdout.log": "2f505dbe98327b10979886855522dba1302116b243c09650aef6cdd5d12c835e",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/114-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/114-stdout.log": "901baccf54f676e113db35806664c0b14759156acb0f4655f886e4903e5c9e49",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/115-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/115-stdout.log": "9a9dc5f68bdd477de8706a303d56b967ecb038dc742dd84fd8801c7165ffdbe1",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/116-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/116-stdout.log": "2515e8d0f8fd46ffb7e414341f738f086a9dd883dd0936a82916ab9d9a00c639",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/117-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/117-stdout.log": "c563bbe2dae65820dcdc45636f6862c08089f45dc5b86f785fd19ac99478ed0f",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/118-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/118-stdout.log": "4c7ee9d1cab01b7b5fc5017cdd81529ab7ac0e222057d0e177787fdfbfb2b97c",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/119-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/119-stdout.log": "f45e12c0e045b3f2f3272c53e20e6424bcd9d71a40948e446c45bf9611e20249",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/120-stderr.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage-logs/120-stdout.log": "a1209a748ce98b623b7398e3df9ff4afd85ed296649df988178dd38f99de4624",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/coverage.json": "ba2a1dd71cb0041ece4abc34a3b9a1bf6bb74fab7e75717ba6532a51aa6e3029",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/decision.json": "3be5ddc7b87691e521fe648bee9e7edc4ff76e01ab25e6b42c2e5aa307cc8a07",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/dependency-budget.json": "9f9f5075f118b58955f5f99913dbbaa2168403f3848a2985d5af95cf09a14b02",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/prepare-fixture-logs/br_if_taken_untaken-encode.json": "53b9f745d90fb518a7948e1ceb51cd5b3f907716b423e1a7f496fdd24e54877f",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/prepare-fixture-logs/br_if_taken_untaken-validate.json": "49110c8aecd5f205a212cf9345026618c6f982cc7a39abd3810c73ca4c6dad9e",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/prepare-fixture-logs/br_table_three_paths-encode.json": "4612dd83453cc56d69654652c2d2d7dd470d20eb23504f8c5c516568eaa4fff0",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/prepare-fixture-logs/br_table_three_paths-validate.json": "82c74bd0953fa7eecf930f4f1d70695e4f83badb1ed00df67c7925eadcde3ecf",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/prepare-fixture-logs/function_branch_vs_return-encode.json": "0256c740aac7a76508ff682f6fa59b0409c30208ae34ab86ec74e4eb7ee86949",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/prepare-fixture-logs/function_branch_vs_return-validate.json": "2cd868b6e65e1382aaa7c5ce43b2106c9f08c8f4148bd060b89798f9388395ab",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/prepare-fixture-logs/multivalue_loop-encode.json": "0e87096bf7d93def7808de793fb378350c27d2b2a801a134f1de9aa5f2942cb8",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/prepare-fixture-logs/multivalue_loop-validate.json": "0a9257f2f21358553409921afb77d6029629d3346e883c53a7ee00c1cbac5aeb",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/prepare-fixture-logs/nested_if_results-encode.json": "9f95367e4c94b2a6e6bed2240a918b1b10774b410b7689e78078e747fdc75935",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/prepare-fixture-logs/nested_if_results-validate.json": "5e467a754663807367619f6d9fce8678762d4cabfc3c570693c97c7900725c4c",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/prepare-fixture-logs/unreachable_local_prefix-encode.json": "9ffb4d4380d4f0536ac8f0b4c71960481c7b36e9ea857495ed73295f44934047",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/prepare-fixture-logs/unreachable_local_prefix-validate.json": "954b8bd6bea85638acd740344ad041c99c58f356263274f3ae1c7be0ab7b4365",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/prepared-fixtures/br_if_taken_untaken.wasm": "1ef089715846b6f725ab36a33454f8cc191871f1892f02fec18b8aa46c3e5b57",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/prepared-fixtures/br_if_taken_untaken.wat": "7db3f3a11234cba76b65d84e2766fd27dd07996102a0ca57f4da26e7760b936d",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/prepared-fixtures/br_table_three_paths.wasm": "f775130cdd1cc7089a5497d3e59c8e6e9188d26ab4a958d53ff708f4d0992f34",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/prepared-fixtures/br_table_three_paths.wat": "e2a1798f06d9f5705c708e74daac16d35757d71cbb2b5f01ae48c79fead72c60",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/prepared-fixtures/function_branch_vs_return.wasm": "1fc5674a11b5aef598ae6ba62a76680f24cbb89a411cf382012e7fc5aa4c8c84",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/prepared-fixtures/function_branch_vs_return.wat": "9e729d5deb484252dbae149e43cebd5094c6b2c396ddb13d15a9b64456686922",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/prepared-fixtures/multivalue_loop.wasm": "ab8c4ca0f71156ed0ca30fc0c87abe17e940d3de5d1efafd9624dd8dd221dbce",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/prepared-fixtures/multivalue_loop.wat": "655a13e87c1ae2a49a2d84fa6cc6f7ffa9aaa615d8620888a03eee6724b736ff",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/prepared-fixtures/nested_if_results.wasm": "194ba33c860add868dfe77ed31fb9ab07190f0755e61b6a4735ff6702421f7e7",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/prepared-fixtures/nested_if_results.wat": "9c4eb9eb041478ef8c3bfd948fdbf51905735879243e4e6dca5afe2f1540be05",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/prepared-fixtures/unreachable_local_prefix.wasm": "d7733f5aad85961cbfdca38abc20b3fb50ae78d6f21be75105af071b71cbf354",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/prepared-fixtures/unreachable_local_prefix.wat": "80432900a5c3facc4f79cf92665b3a8825cfa170420b6d540398c6f40ab1d5df",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/source-snapshots/source-after.json": "c19edfa048da2f5e914dca2948bd681fda84160c9b70d909bcead6a9a69e60d7",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/source-snapshots/source-before.json": "c19edfa048da2f5e914dca2948bd681fda84160c9b70d909bcead6a9a69e60d7",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/source-snapshots/workspace-source-after.json": "812329b5063b893ca3a9869a5b694a8bb04e8af54b568ab7cf0af47f24dd4993",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/source-snapshots/workspace-source-before.json": "812329b5063b893ca3a9869a5b694a8bb04e8af54b568ab7cf0af47f24dd4993",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/test-binaries/default-runtime-suite/core-fc4384471282e053": "b61cb3fc4918f3e99a332b2a3772920c72410be31fe310b9eb0df2f073896160",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/test-binaries/default-runtime-suite/exception_root_safety-863d9cca58697c87": "e5b43c003b34359077f75af985f7c469152628ad77a0bd5b09873bd6914a8f9b",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/test-binaries/default-runtime-suite/exceptions-dc82360c779184c7": "3a36d4872a8aeebf84482dd4564d53c9e813dcec1bfb4049a38b48b5eea4babd",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/test-binaries/default-runtime-suite/feature_profiles-da219ada3555dbdb": "63ad2d54c9e47970728b57e524bda60d2948d02cfa926ac9bf1deb6944a785b7",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/test-binaries/default-runtime-suite/performance-f1f918c868572e47": "930ea13f929f222f9d8f4bffa65eb20b46782639d74b4dee6a008f136e431b8a",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/test-binaries/default-runtime-suite/proposals-ce7c5c5922b88848": "c169f2b6587f291c58163806585a23a6ea716eb1d695731f536879946074dd3a",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/test-binaries/default-runtime-suite/wtime_runtime-670f0c395211a4f7": "1b148f1dd6d8f4c6d96f7502a92b04d3264b4a54d2ff65cec14a07d1dfe4e9c9",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/test-binaries/feature-focused-tests/core-42d92ac3afc77304": "7d6a39ccfced03f9435dcf0bc8121bf8c6c4b5a22a03423ea97fca6e49d52303",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/test-binaries/feature-focused-tests/exception_root_safety-db67ff33a61a98ad": "3ebd51a0d360aff8bd11e14acb7fc3afb10e93d268e57a68b7bff5fae8b9e2b5",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/test-binaries/feature-focused-tests/exceptions-5d275ecd0f4f002b": "9c886848dd30e172893d2a6343f9d67f86f1f5a522f6fd9344f51c3518eed2ed",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/test-binaries/feature-focused-tests/feature_profiles-487957d4b265fccb": "8957e71bba2f37738c3234bb981c363ae68414e214af9c03e31d863d3bf17c70",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/test-binaries/feature-focused-tests/performance-1a202aeb20ffb000": "a4b67f6c0a0d232e14bc214020e568fb2b0324929941520778ae85099c535930",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/test-binaries/feature-focused-tests/proposals-2b774c23db30396c": "c29bb84abc3c14268132f92e694f5e12abaf434af663c0dff1c0e8cbe791f20b",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/test-binaries/feature-focused-tests/wtime_runtime-37e3c598ab512be0": "1fbe45baf1725cd59b2cbfe364757b6ed94320821d7608564c12c640bbb5a246",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/test-binaries/p038-regression-tests/core-42d92ac3afc77304": "7d6a39ccfced03f9435dcf0bc8121bf8c6c4b5a22a03423ea97fca6e49d52303",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/test-binaries/p038-regression-tests/exception_root_safety-db67ff33a61a98ad": "3ebd51a0d360aff8bd11e14acb7fc3afb10e93d268e57a68b7bff5fae8b9e2b5",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/test-binaries/p038-regression-tests/exceptions-5d275ecd0f4f002b": "9c886848dd30e172893d2a6343f9d67f86f1f5a522f6fd9344f51c3518eed2ed",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/test-binaries/p038-regression-tests/feature_profiles-487957d4b265fccb": "8957e71bba2f37738c3234bb981c363ae68414e214af9c03e31d863d3bf17c70",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/test-binaries/p038-regression-tests/performance-1a202aeb20ffb000": "a4b67f6c0a0d232e14bc214020e568fb2b0324929941520778ae85099c535930",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/test-binaries/p038-regression-tests/proposals-2b774c23db30396c": "c29bb84abc3c14268132f92e694f5e12abaf434af663c0dff1c0e8cbe791f20b",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/test-binaries/p038-regression-tests/wtime_runtime-37e3c598ab512be0": "1fbe45baf1725cd59b2cbfe364757b6ed94320821d7608564c12c640bbb5a246",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/test-logs/default-runtime-suite.json": "149ad35fe3fcc50af293891c4748f04cdad26bb2a81337ee2a7591533cd93b3c",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/test-logs/feature-focused-tests.json": "fd34a73a1f1bf118577a1529e345deb0ac26edca89c7dbc14a1cdacb68375586",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/test-logs/p038-regression-tests.json": "d5ac0eb002c6cd396440fb572ce13a5471ec3316cf7cb313332dffd6dc70e3e9",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/tools/wtime-cli": "c543c8a9690ddf9e5df10548b87bc476a7cc7bb9cc33031030003f4e01f776ab",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/artifacts/tools/wtime-conformance": "5123b38fd1e6b2358027778d94256202e53ff7024b3b05cd94aa5ea50d7949d4",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/control-evidence.json": "5151763755e40b83365407d0bb6b4f66c14856c98eb4f683132fdcfb5e9e7628",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/inputs/check-result.json": "e5f1b00e4bdc79842f81f98af46189f79510c6e73ed42fdf1b27ecfec9702a72",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/inputs/closure-builder.py": "facf2f386898d6f7be17604a742ae058e5c08d71c629f6784463b2efe23971b3",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/inputs/comparison-report.json": "cc269b30e1de2fb3bb1f2e853d3bfea9c7a207e342f4945182619c59f5203b52",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/inputs/coverage.json": "ba2a1dd71cb0041ece4abc34a3b9a1bf6bb74fab7e75717ba6532a51aa6e3029",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/inputs/decision.json": "3be5ddc7b87691e521fe648bee9e7edc4ff76e01ab25e6b42c2e5aa307cc8a07",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/inputs/prepare-receipt.json": "e53d55cd9dd88705a4500224b974ae8fa62bf3581997a85b3bc589e9df3e6c6a",
    "/workspace/wtime/target/performance/P040/closure-attempt-0003/inputs/test-contract.json": "fa1b44094463ff1408f0590d705d26c3198b4ff38ecebf1208eac208892000d5"
  },
  "evidence": [
    {
      "bytes": 250912,
      "path": "/workspace/wtime/target/performance/P040/closure-attempt-0003/control-evidence.json",
      "sha256": "5151763755e40b83365407d0bb6b4f66c14856c98eb4f683132fdcfb5e9e7628"
    }
  ],
  "outcome": "inconclusive",
  "status": "done",
  "task": "P040"
}
```
