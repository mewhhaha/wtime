# P039 — Lower float/vector constants and barrier adapters

Status: **done**

Outcome: **inconclusive**

Kind: `candidate` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P002 — Compare separately built baseline and candidate binaries](../00-measurement/P002-paired-driver.md)

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P010 — Add SIMD and strict floating-point benchmarks](../00-measurement/P010-simd-float-workloads.md)

[P014 — Run source-bound correctness checks across execution tiers](../00-measurement/P014-tier-differential-runner.md)

[P016 — Add explicit executor policies and test integration hooks](../01-execution-contracts/P016-execution-policy.md)

[P026 — Use fixed buffers for SIMD and lane memory access](../02-structural-wins/P026-fixed-vector-reads.md)

[P036 — Define small immutable execution instructions and side tables](P036-compact-op-format.md)

[P037 — Map compact instructions to source events and control targets](P037-compact-source-maps.md)

[P038 — Lower constants, locals and common integer operations](P038-compact-scalar-lowering.md)

[P040 — Compact control predecessor](P040-compact-control-plans.md)

## Scope and ownership

Read EXECUTION.md, SOURCE_NOTES.md, BACKEND_CONTRACTS.md and C1,C2,C3,C4,C6,C7,C8.

- `crates/wtime-runtime/src/execution/compact/lower_numeric.rs` (new numeric adapter)
- `crates/wtime-runtime/src/execution/compact/lower_scalar.rs`
- `crates/wtime-runtime/src/execution/compact/ir.rs`
- `crates/wtime-runtime/src/execution/compact/execute.rs`
- `crates/wtime-runtime/src/numeric.rs`
- `crates/wtime-runtime/src/simd.rs`
- `crates/wtime-runtime/tests/performance.rs` (P039 registration)
- `crates/wtime-runtime/tests/performance/p039.rs`
- `crates/wtime-runtime/tests/fixtures/performance/p039` (four WAT/Wasm fixtures, manual oracles, provenance, and MIT/Apache license files)
- `docs/benchmarks/performance/P039-contract.json`

One integrator owns dispatch/features/registration.

P039's predeclared workload roles are frozen in `docs/benchmarks/performance/P039-contract.json`.

## Input/output contract

Explicit Compact float/vector constants and audited adapters preserve exact bits, signed zero, specification NaN allowed sets and original trap cursors. The SIMD selection has114 IDs: primary exactly simd_integer_lanes_n17 and simd_integer_lanes_n1000; all other112 protected, including n0 and scalar controls. P040 control predecessor enables loops. Reference/continuation helper functions still decline until P044.

## Implementation recipe

1. After closed P040, snapshot fresh Compact P039-before after prepared harness support; preserve immediate predecessor and compare matching compact policy.
2. Lower raw float/vector constants and route audited numeric/SIMD helpers through checked prepared operands.
3. Materialize exact original source state before one-source adapters; no double fuel debit or repeated side effects.
4. Preserve signed zero, NaN allowed sets, vector bytes and original operation traps; exclude unsupported root/control transitions.
5. Execute independent fixture oracles and compact/reference prefixes; report actual adapter/fallback coverage and measure both paired runs with unchanged cold/build gates.

## Meaningful cases

- Exact signed-zero and NaN constant bits; float operation NaN allowed sets.
- Nontrivial two-vector shuffle checks all16 bytes.
- Division/conversion traps retain original charged source operation and fuel priority.
- Unsupported complex helpers visibly decline; both exact primary SIMD rows execute compact work.

## Verification

Activate Rust per EXECUTION.md. Execute at least three meaningful named cases with actual compact adapter coverage; no unexecuted assertions count as evidence.

```sh
cargo test --locked -p wtime-runtime --features compact-executor,execution-reference p039_ -- --nocapture
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml --features compact-executor -j 4
python3 tools/wtime-bench/compare.py --baseline target/performance/P039-before --candidate tools/wtime-bench/target/release/wtime-bench --candidate-mode compact --selection simd --output target/performance/P039/comparison
python3 tools/wtime-bench/analyze.py --input target/performance/P039/comparison --output target/performance/P039/comparison/analysis.json
python3 tools/checks/build_budget.py --output target/performance/P039/build.json
python3 tools/checks/build_budget.py --features wtime-runtime/compact-executor --output target/performance/P039/optional-build.json
python3 tools/wtime-bench/gates.py --task P039 --comparison target/performance/P039/comparison --build target/performance/P039/build.json --optional-build target/performance/P039/optional-build.json --contract docs/benchmarks/performance/P039-contract.json --output target/performance/P039/decision.json
```

## Measurement and decision

Selection: `simd`. Explicit execution setting: `compact`. Decision rule: `measured_promotion`.

Primary exactly simd_integer_lanes_n17 and simd_integer_lanes_n1000; protect all other112 of114 IDs including n0/scalar controls. Match policy, preparation/cold/retention/default and optional build budgets. Predeclaration is not a gain claim.

## Before editing

Freeze an immutable immediate P040 compact-policy predecessor after support/preflight, before numeric candidate edits.

```sh
python3 tools/wtime-bench/baseline.py --mode compact --output target/performance/P039-before
```

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. Executed evidence is recorded below.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/crates/wtime-runtime/tests/performance/p039.rs": "95bf392e7b99132efd76c088a6902645223a665dd32f2c74bf374a6da1522810",
    "/workspace/wtime/target/performance/P039/build.json": "065e7c4320a52499263e13e692eff66ad11b8aee1accbe8b2d2c46a9ba15b95a",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/P039-before-command.json": "60d4329cd6cf67825584dfebda3d60409002a9ceae2bd8037e80fe5a48b9d74b",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/P039-before-manifest.json": "d489acfd66634f2b295e09347e708a5566cc093c68b89f5cb66483b81c198f57",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/P039-check-result.json": "422dcd5d23c20793350a42c8958c2922d154a4b6398fd29072407ee2458799af",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/P039-fixture-provenance.json": "483448a357bbd8aff42aef02fe2a08a7bc0ab306a2595762f4c24388c1b6ed1a",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/P039-prepare-receipt.json": "227e4344bb4033cec13dbc3bce46ecd22159e8547151853839a48caf7362ed51",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/P039-test-contract.json": "24c75900624da40ce0827228cd10da29cffaeacb9688ad19540646d6640eb613",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/P039-workload-contract.json": "5ad4f7a630e175d3c18d132afb8eeaf39e4cd6312b1b80699c7d3d79e113a493",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/P040-check-result.json": "e5f1b00e4bdc79842f81f98af46189f79510c6e73ed42fdf1b27ecfec9702a72",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/P040-comparison.json": "cc269b30e1de2fb3bb1f2e853d3bfea9c7a207e342f4945182619c59f5203b52",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/P040-decision.json": "3be5ddc7b87691e521fe648bee9e7edc4ff76e01ab25e6b42c2e5aa307cc8a07",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/P040-evidence.json": "5151763755e40b83365407d0bb6b4f66c14856c98eb4f683132fdcfb5e9e7628",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/P040-test-binaries/default-runtime-suite/core-fc4384471282e053": "b61cb3fc4918f3e99a332b2a3772920c72410be31fe310b9eb0df2f073896160",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/P040-test-binaries/default-runtime-suite/exception_root_safety-863d9cca58697c87": "e5b43c003b34359077f75af985f7c469152628ad77a0bd5b09873bd6914a8f9b",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/P040-test-binaries/default-runtime-suite/exceptions-dc82360c779184c7": "3a36d4872a8aeebf84482dd4564d53c9e813dcec1bfb4049a38b48b5eea4babd",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/P040-test-binaries/default-runtime-suite/feature_profiles-da219ada3555dbdb": "63ad2d54c9e47970728b57e524bda60d2948d02cfa926ac9bf1deb6944a785b7",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/P040-test-binaries/default-runtime-suite/performance-f1f918c868572e47": "930ea13f929f222f9d8f4bffa65eb20b46782639d74b4dee6a008f136e431b8a",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/P040-test-binaries/default-runtime-suite/proposals-ce7c5c5922b88848": "c169f2b6587f291c58163806585a23a6ea716eb1d695731f536879946074dd3a",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/P040-test-binaries/default-runtime-suite/wtime_runtime-670f0c395211a4f7": "1b148f1dd6d8f4c6d96f7502a92b04d3264b4a54d2ff65cec14a07d1dfe4e9c9",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/P040-test-binaries/feature-focused-tests/core-42d92ac3afc77304": "7d6a39ccfced03f9435dcf0bc8121bf8c6c4b5a22a03423ea97fca6e49d52303",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/P040-test-binaries/feature-focused-tests/exception_root_safety-db67ff33a61a98ad": "3ebd51a0d360aff8bd11e14acb7fc3afb10e93d268e57a68b7bff5fae8b9e2b5",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/P040-test-binaries/feature-focused-tests/exceptions-5d275ecd0f4f002b": "9c886848dd30e172893d2a6343f9d67f86f1f5a522f6fd9344f51c3518eed2ed",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/P040-test-binaries/feature-focused-tests/feature_profiles-487957d4b265fccb": "8957e71bba2f37738c3234bb981c363ae68414e214af9c03e31d863d3bf17c70",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/P040-test-binaries/feature-focused-tests/performance-1a202aeb20ffb000": "a4b67f6c0a0d232e14bc214020e568fb2b0324929941520778ae85099c535930",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/P040-test-binaries/feature-focused-tests/proposals-2b774c23db30396c": "c29bb84abc3c14268132f92e694f5e12abaf434af663c0dff1c0e8cbe791f20b",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/P040-test-binaries/feature-focused-tests/wtime_runtime-37e3c598ab512be0": "1fbe45baf1725cd59b2cbfe364757b6ed94320821d7608564c12c640bbb5a246",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/P040-test-binaries/p038-regression-tests/core-42d92ac3afc77304": "7d6a39ccfced03f9435dcf0bc8121bf8c6c4b5a22a03423ea97fca6e49d52303",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/P040-test-binaries/p038-regression-tests/exception_root_safety-db67ff33a61a98ad": "3ebd51a0d360aff8bd11e14acb7fc3afb10e93d268e57a68b7bff5fae8b9e2b5",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/P040-test-binaries/p038-regression-tests/exceptions-5d275ecd0f4f002b": "9c886848dd30e172893d2a6343f9d67f86f1f5a522f6fd9344f51c3518eed2ed",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/P040-test-binaries/p038-regression-tests/feature_profiles-487957d4b265fccb": "8957e71bba2f37738c3234bb981c363ae68414e214af9c03e31d863d3bf17c70",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/P040-test-binaries/p038-regression-tests/performance-1a202aeb20ffb000": "a4b67f6c0a0d232e14bc214020e568fb2b0324929941520778ae85099c535930",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/P040-test-binaries/p038-regression-tests/proposals-2b774c23db30396c": "c29bb84abc3c14268132f92e694f5e12abaf434af663c0dff1c0e8cbe791f20b",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/P040-test-binaries/p038-regression-tests/wtime_runtime-37e3c598ab512be0": "1fbe45baf1725cd59b2cbfe364757b6ed94320821d7608564c12c640bbb5a246",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/build-budget-commands/default.json": "3f3275459c8e6a63845cda77b8c17f3d3500749cf40c5ad135d1d663be15b25c",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/build-budget-commands/optional.json": "f55ae7ff9ac39a1885e603a4ed0f88796c616232ae100532c6341b36e129d012",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/comparison-report.json": "4a213ef581e01946d498c201a46542911a336e421ae230906fadbdaf1cd2ffdb",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-binaries/diagnostic": "dca938110943b66a88edbd27813d716825ee7ab2464ae9261ffba6d1c2dbbb65",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-binaries/encoder": "5123b38fd1e6b2358027778d94256202e53ff7024b3b05cd94aa5ea50d7949d4",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-binaries/validator": "c543c8a9690ddf9e5df10548b87bc476a7cc7bb9cc33031030003f4e01f776ab",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-collector.py": "74ef34a816500bef255e1ab1af5572693ce925a07e76c21104537bbc99fe8f3b",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-fixtures/015fdae667ac84ce.wasm": "a92144289dae9ec957773fe0f5bfef50c2ef4d188f96338aff50a4cb55af5c2c",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-fixtures/2ffb638ce4f9db54.wasm": "71f6ccd9b822a455d0248276eacdf61a61f39a2435000c0d3f12a9d3e3f5a15c",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-fixtures/4a18933e2328ab06.wasm": "621b5c278b5a3c706996d06a19cf12578c57dbd92e7015d57e4e8a384018405c",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-fixtures/5075b488bced40cb.wasm": "45101f560f2675ae21c5b80b7e89974a3e0bff7814735bbf4609999ec2c44e19",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-fixtures/62be685b8ba28fc6.wasm": "9c957181e2ca5d74e9a3f9fec4e4cf5fb22e3eea231ccbc4c826c5c8847bfa01",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-fixtures/640dc537681c5ae0.wasm": "3a690996697e34089e18a5e9883ce551f6d600dece89e70580fd601ae2a82150",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-fixtures/78585cec511c2dc5.wasm": "95ad86610276d9db5ca8ea3d6426d087fca8a702225606139e79673de60003a7",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-fixtures/7ccf274f6ec47796.wasm": "ece84319c9f3b540e99b4ddaf2585f778dda6cd8735d11b27f7ba35e6d5fbef0",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-fixtures/89c60e4747c2b5af.wasm": "69ed757b2f76f1605f3b4c9332d995887848a44406d99b929df38676c4486b77",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-fixtures/9e4b2fc69ff94139.wasm": "22633c8544d3db06b0f47cef19966e357743e0db9a84815fc66d87b390a87bea",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-fixtures/a57926a31341aa5a.wasm": "12f3541a902e0ea324af36e547d6af9ea2879720ccefb7e4fdb8b37b024b1a12",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-fixtures/aee6348c8bdcef3c.wasm": "6d21067a0c105455857d2d0c46b6b142e98f0794d2f1870da0ea5f467768a506",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-fixtures/b1ee424758921112.wasm": "4b8677abfcf31c65bf72fb19952629c037e4903a8596a508f32af4ce38a25643",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-fixtures/b29f2ea1ba7e423a.wasm": "b71c56264d750cc8f7300e3b9900534072d92838447a2f600c920e20625f5f77",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-fixtures/c9f31a7a98d3da45.wasm": "0f79b71c6396e4e7e9c3f60828edfd1e8b185d83e65b2a1466292d29bb16dc3b",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-fixtures/d1a31cc79099012f.wasm": "9a866ed614814bc74a36aad06d6adefc51cd9bf914c8cd7901ee19a13649ce92",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-fixtures/d6b99562abf8e0b2.wasm": "f7a3e0e209078c5afac21d24285412d7206921427e73fae451e90fa2df8c485a",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-fixtures/eca13c3069aac0d1.wasm": "6f5956a62e16a5298cb1297d0691bfb26e50bc92243198413748b82580d2f5ac",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-fixtures/f0edf4419a1f3f54.wasm": "2017b5e0e15c1dd4406fc76832f92bcab1ea1f32187b1ca0dcc4c80e7aa121da",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-fixtures/f17addfa12a8b6f1.wasm": "8b91d640241294a1dee9cfbe8605287a0b4e3d5e215d0c9aea529f77ec6da841",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_f64_arithmetic_n0.json": "fb8cdc0f155949e4c5a1303ab0504a3cf2ff1c25ae0bec53a1fc31cde10cdabc",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_f64_arithmetic_n1000.json": "f06f7c0250e85fe00282c426d12450efa277ae0493b27339dd29f537b8c9db68",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_f64_arithmetic_n17.json": "287bd8f8efa68ab7162ee9cb7f84df060839527c54981266283934630766ca8b",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_f64_min_nan.json": "357159c377b1e9fb9e2715dd21864fd27112e737e9b90a1bade7eeecc4c1fb33",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_f64_min_negative_zero.json": "efb015db4dbf0a6772c220ec223a15f6c07e8e6a6c3996f87c8f83ceb15b5e16",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_f64_min_positive_value.json": "29bcfa231a1c2d5d2cd0eb4fe8f529d5d8f8441bcdf7a27686f132db1e182da6",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_f64_min_positive_zero.json": "41239e95001279a80b124ecbe3a01e778f59b30ee3bc114a7c1e82f9755cba6d",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_float_arithmetic_n0.json": "d4398f14c40b3ff26c5403f07324eae40e139a39ebf3fa24b6b31331f039aec0",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_float_arithmetic_n1000.json": "77830cb68158dd2c83618a05e03ae0e0ecfa01820044cf84587e12a7830f3696",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_float_arithmetic_n17.json": "080ea9a2f72329b79c3969058f065821dde52f1993bf2d083eb1f821f7d40a4c",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_float_convert_nan.json": "d063b8ab932e2b4be33804ccfe6916b0f9cf2b4032049947cb8f780b67f4fd47",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_float_convert_pos_inf.json": "65ad72a31c6784f3ba9edd2c656a4f16eaf1ec24b1386a9a9d0c2a6f0e7d05e7",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_float_max_zero.json": "24a829b921d7042e9b6a731ea7024d4f7a309f3512b64cc7465977d82382c688",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_float_min_zero.json": "45d8dc30d858556c51159ced661641020aa0747e967d855fa842e08c287876f5",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_float_nan_classify.json": "1cb863355a1f541a0a7b2900ede431b3c19583e15eb1d967397ab282fe1fb60d",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_integer_lanes_n0.json": "50f4732ef52cce58e3568e899c8565a137411a267e3732ed836666189fd9e82e",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_integer_lanes_n1000.json": "d36d0fc3cf0949423fb8a39192668245001d9fdc156d6400293601de8f274ce0",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_integer_lanes_n17.json": "3acdafd461abaaf314ed296815e47d7867f30f9a544016530f78fd5000d60a8b",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_integer_lanes_random_00.json": "2ed7f0304250a3260a3f039a721c79e50c0ef74b247fc4baddee6096c7e4dfac",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_integer_lanes_random_01.json": "43a6fcadd71ddc76c2118fde8ee7400e6952fa20e49fd5728eb904d69d23c149",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_integer_lanes_random_02.json": "5335cb3c269446b024daa9cf898946fb345536e2fc20b2887900b1e2713660c7",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_integer_lanes_random_03.json": "9664fc2727810f806a2e7208d970f1c05d7105cda15aab261ba9cc2aab88a02d",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_integer_lanes_random_04.json": "239458f86a2da3a57eb3e2bf442346972056e9492d99df7330a917015ae32626",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_integer_lanes_random_05.json": "d332e14948334e74e57fc576d0a5cbc0501cb407fbb07cf7c6c3373468cb4f91",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_integer_lanes_random_06.json": "840fcd0494019c4b78469e37ccc3682984e8f013000e1358343a23d6c4e9c60f",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_integer_lanes_random_07.json": "ba9ae563d1b6c6caf96bf72d7302053d44032d6102a4e30b41b0aca11b733b22",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_integer_lanes_random_08.json": "9c3d5280bd88acf9d41ed08462d841523e0179eb7f75b101f4f36fd291eaf4bb",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_integer_lanes_random_09.json": "c68ab5540f35e756077087dc820f27b6b9ce5a0982f4656c9430cd4af0429a58",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_integer_lanes_random_10.json": "91f283a2c03163585771423989333eb854b2d82decd13c76d5f340115cbb41c4",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_integer_lanes_random_11.json": "94f4a99ca8b8c4a25084a94e208451fcae29860e0d9b7ed0d71f1a19e9eba956",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_integer_lanes_random_12.json": "272d6ea66878cca2abad4e1b29993339f4662f67a031c74238444cd83f81cc00",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_integer_lanes_random_13.json": "0bbe0e82ea181eea059c276b57e62ef67b9d0833d0aff1ec3966f8db90a0fefa",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_integer_lanes_random_14.json": "3ec32cf59983e988881984931be45449071ea569ebdd75b9f012ced877dd3381",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_integer_lanes_random_15.json": "a34641ba86612b9c7c859fc07701826a9fe36decad2c93d3da9f8cf44cacb9ba",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_integer_lanes_random_16.json": "b9d4c56d58a324a69a8c55c604897a4610ada2a20d4d2f33a40c35ef2ac99d99",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_integer_lanes_random_17.json": "af6aadfa8e2c60b80969fd8478b1f4bd301c70dd0c28cf42e6b07b1c9c7ffe0a",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_integer_lanes_random_18.json": "6289dbc1c5b90992af1251fc6e13972a62740b1ee1a984fe738730dfc1d0ceab",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_integer_lanes_random_19.json": "8229d3ec12f528f63088b8362198fa84ffdb44177bf408ec5c78658c581870c7",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_integer_lanes_random_20.json": "58b63d3c0edfd924fb9b0468251b32cf30db3dea244e592c3b6c3370b004c7dd",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_integer_lanes_random_21.json": "a0ae1407787f508508e771043c83f322ad4a25693bb8f382ffda74a45d8fb328",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_integer_lanes_random_22.json": "75a4b65e1b53cf18b2c993ff5f1a62dd8c3069cfc4a2de0dbeae87d97dbade8a",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_integer_lanes_random_23.json": "47cedbd79102eb4930f949cb5f7bb977620a8ec48b7ac61cf7175d3ecf951b3a",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_integer_lanes_random_24.json": "c8d646e88c6fb5650a8f43059388816b9b59f92bd52ca4d3503246630098f5d9",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_integer_lanes_random_25.json": "99b0a0a40ad887acfeef489ce5bc7066e9b3458754d334352993d015ab5c8bd7",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_integer_lanes_random_26.json": "d8fcb5da1d3b4c12f11247a2a2fb46075704a1e98bb71eebd2c8a22ede5fa6d7",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_integer_lanes_random_27.json": "4b16c217e0d1efcc40b78ff5add2e4f8076ca7357b567dd546b50e7b9328f334",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_integer_lanes_random_28.json": "b2ff2c74954e7b7471e45283ceefe9d0b31a5d4544e14aa33fd91b14218d0060",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_integer_lanes_random_29.json": "16990ef5ea27b7f72e56c8b99388a98085ab6037ecdd1d48d7dfffd6ec41c642",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_lane_memory_footprint_16.json": "25c5f523a361a273768c901395e5c8505602a0d5439458496c2af62999b5a92d",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_lane_memory_footprint_4096.json": "696c4425ebc61c7ea94b0d8f55ff3ebd4aba2273cab2b3d076db5e80289bab6e",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_lane_memory_footprint_65536.json": "4992e1879ad0657cda0ced77879cc637aa9a5a46087e0a2cc321d946432db345",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_lane_memory_n0.json": "08f850e873c11f92687fc8c1c7517f549bd103200ad13ec4efeae27509e839c5",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_lane_memory_n1000.json": "0662f0721e260ba3fd55538d08ab240ef0d962316d70ff1b70059b61816717f7",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_lane_memory_n17.json": "5dd6b813bbab00a072a0b45e49415d23fe0e0d9e6987078611fccaf64135b71f",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_relaxed_swizzle_n0.json": "7b15ad73668f7d39b10f1f6799d61cfdcd0e5c70fb39cdefb7526c1c2d6165cb",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_relaxed_swizzle_n1000.json": "9e9bb5f2bf3dfbb93cfcde9a32c0c2e3e4d989748554954923d01a1da318db92",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_relaxed_swizzle_n17.json": "f63e614c2f062f3c67071734dba219092f1e97d7b6761d536d78f731eea1b0a9",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_relaxed_swizzle_oob_17.json": "d54f8184ceff1dbb210937b42f5cef9aa4700c75ed1cb4c0f67cd43beb6aa363",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_saturating_select_n0.json": "ebcf3ac2411ac9a98e4df0b1e97559d4971b2aa2f23e508f583d0d77666697b3",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_saturating_select_n1000.json": "c031740b8bb048eb422fd66d17f861adfc537bd10c39eda2a72adf051e3df166",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_saturating_select_n17.json": "dbdc8c855504341e85fac49bad4ae8f3f747a31eddeb12b757939e1d113a1a5c",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_scalar_f64_arithmetic_n0.json": "7b2b78d2720d9acfc28e7f5071ce359aaebcf40436f3c8dd08fe4019565cbd67",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_scalar_f64_arithmetic_n1000.json": "1569e15f54fd63ef7bab42049e345a2911d1544beb4d3a94066e7123fbc5276e",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_scalar_f64_arithmetic_n17.json": "ef88fca2e327c58d2cf18f1e7ec51d959dcd00022878a5ce139a55f664a5b84c",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_scalar_float_arithmetic_n0.json": "e20962ea0ba4ccfee99d9d6b92d7fadd6c1c19cfcc48582d38b74c6127e0af7d",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_scalar_float_arithmetic_n1000.json": "fb3c90f8186c9551ef6408cc89be1a99b195007694963ccf944885c95756b25a",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_scalar_float_arithmetic_n17.json": "b349fadd572f9e45a3912e416757eae3ff7c784f639d78331023fbcdeb0cafda",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_scalar_float_convert_nan.json": "ee86f4453cdfe2b32c639bb13bdc85cce5172bc5fed273d024d2a42a231183f8",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_scalar_float_convert_pos_inf.json": "5b985a29217e8731bd1ed430535399cbcd5260d2b141692f18187acdc32f3d7a",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_scalar_float_max_zero.json": "592d5daf48a69b9f5313ff621f274a0a844b249276da86034d455ba3ec8144ce",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_scalar_float_min_zero.json": "09050eae69cc03c38f2e9e7adfd9cf6226d3eb00e4595d319555c0883400417f",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_scalar_float_nan_classify.json": "ea1d793a88fe792e569351a774f6bd1f02584792879a0c1b04c75e3b9766eaa7",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_scalar_integer_lanes_n0.json": "0572b204531970844597030a6b558e3e0885715f19a280750f554f52080480a5",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_scalar_integer_lanes_n1000.json": "6bca8ea88e0aa1e76fa834aca04e19fa14269d992cc936137ca932b1d52078ee",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_scalar_integer_lanes_n17.json": "2a26a7a12b51399bfdd1ec30ce32ee90d198c4e479c2f6e6e62f9064c7ae4e63",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_scalar_integer_lanes_random_00.json": "11b82ea60af509a69f1d550eba20bb88f25c5067d369d328c7026aeeb440215b",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_scalar_integer_lanes_random_01.json": "5c5a7511511676ec08cf86408ad7f299aacf7ae00d89fb261c9d9951fb8a99a2",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_scalar_integer_lanes_random_02.json": "dbcefd268ca82c9f14c0c60a2a6fbdb0d127322f9ab2e04b30641e17db2cc58d",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_scalar_integer_lanes_random_03.json": "c4a1f12f8e853d48f27b9738a54df4c4dbf58fc37f369be881a01a95763692b5",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_scalar_integer_lanes_random_04.json": "5ef0673dff20f9acd593ccccc36cb3348f7e9246150c2765e562147bd6c63a68",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_scalar_integer_lanes_random_05.json": "fd488c4997f116c7f9f5cb8c4e991e994e4e81faddf46c77519109408f2eb3b8",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_scalar_integer_lanes_random_06.json": "06253a4f632b093ebf9fb87531e6376d9f4219d47924c6a13243e2749c34db88",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_scalar_integer_lanes_random_07.json": "da8679405d6462fbf415223a4d0da94319d5600111e22cf41812a3febddb00e7",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_scalar_integer_lanes_random_08.json": "0093a8576c16eccfd2652ec7e542760ff8a0c8a433c80b5899c1982c81382b11",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_scalar_integer_lanes_random_09.json": "baecf734fd01bc9515020aaa5f9b07b31ec809f68576e807cabc8259cad01348",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_scalar_integer_lanes_random_10.json": "637afb64c7f3750bec7e50e7925244fa317e9c29e3d854443b987fd1e78ebf45",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_scalar_integer_lanes_random_11.json": "a37ce7e1712f83f3246753dc23be686ac707c6642d9b47c0e9209a1f6963fd40",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_scalar_integer_lanes_random_12.json": "0ddacbd93333d85e6cf93651cbc4d68cefa0787b14b62af2937737313c481b3e",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_scalar_integer_lanes_random_13.json": "4209de8a242ddb24d95dda4341966b333afe42acfab73ad40798f478ab00fea5",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_scalar_integer_lanes_random_14.json": "5806c8bfa2976138895275c66ba497e47e535780707805daea8ae3a2fb0b003c",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_scalar_integer_lanes_random_15.json": "4ecfaf9933c0cf0b8926bbf73cd3a4c5b611918809c0f3ea4bdccd991120a281",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_scalar_integer_lanes_random_16.json": "335d3dbb04f986354a7adaab84d17d46c998846619814bc8677db8797f4f4c15",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_scalar_integer_lanes_random_17.json": "3ddc495071ef5096d12c9d87e9832bf8900d919f2fff6dd39239f98ede85fb33",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_scalar_integer_lanes_random_18.json": "2b26bb376984941d592cc77b8db93e1db8f099173b5509e52a62d88f7bc18573",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_scalar_integer_lanes_random_19.json": "71a797b104524f92702e1542b3de801ec2685608655c82ee53339f48297c3c15",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_scalar_integer_lanes_random_20.json": "02ca7e0279689db19680080f75b5c93f5f60aab3229102c1647f18667bb99dc7",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_scalar_integer_lanes_random_21.json": "188e464a299b941bd9f6d622781880b59a447a6b25ead11491fc8ce91e9f6430",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_scalar_integer_lanes_random_22.json": "64bc77caf466dbc95fcc08d9e1087684961da6910d368d9fc5b8093d24c01573",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_scalar_integer_lanes_random_23.json": "979f7d7f5ff9cf759044aca93e2c6990503bb7b610fed6f31596f7a3831b6314",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_scalar_integer_lanes_random_24.json": "8ddffc125b100fe6e0af4ba0a2ade6ed5a8c3c3ac58984d3ea49c848b31460ed",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_scalar_integer_lanes_random_25.json": "9e3de572bc4ed81f1c524616af6a9aa08087470a91f0daab5983cff77e566340",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_scalar_integer_lanes_random_26.json": "87eba784788d7aa72267946828748ac95b2f1728ade56ba7bff3d72fd230f49d",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_scalar_integer_lanes_random_27.json": "8bc1c7b204059684b93b1dbab8baed6dcdf16f1d9d5f2bd5b181a5f767ecf0ee",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_scalar_integer_lanes_random_28.json": "e5d1843f361f53bd3e70847ae69130d0ef10a7f2d7f6e98292c8c1f77ffb64bc",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_scalar_integer_lanes_random_29.json": "3d85748e92e750e5695fde12546a77edf7fe995a6e5997ac37622c886c76bd99",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_scalar_shuffle_n0.json": "c68cd7774b27f5ce3aa23b7ec903f7f4a338ecbf18dfc36b3d3c36ef85d017c3",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_scalar_shuffle_n1000.json": "af91f7a34c9142d31e4441ff2f0cfb78b79772ffd9a9671d09d1e647e238e684",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_scalar_shuffle_n17.json": "a4133797debe33278c5ed7d6ace55fd45a04055a361ffb267454018aa282e92e",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_shuffle_n0.json": "963a3e1a7e5cc80866123d798e1ae2163d0bd65814e5f928a9bfba96e64d9999",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_shuffle_n1000.json": "368de4225c45554d0d3879cc912e62d02be29e820b52792bfed25a95ddeb003f",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_shuffle_n17.json": "1036d7011a6ffcb74bf0192e5d5cb37fd410c70a19d90c789acd19e7516e6c7a",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_typed_f64_neg_1_75.json": "5da1103d6bd2c466e055df089325c37281517b010cf5d0929b5d94a33cff7dba",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_typed_f64_pos_3_5.json": "dd401ff7b1d6f714aa02d9c499cb3fea19da3074a3b810e7be7aded39c875a43",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/case-simd_typed_v128_lanes.json": "4d832eed270ec1ac88e08c785c29aa6c4f93a879f3fdf763e07297c2cb35ac0a",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/diagnostic-build.json": "f0150405f4ba57a40d6b446fe15193d6240ebe55e722dc2b5843ab630645d769",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/diagnostic-protocol.json": "054808ad168f893c7c6e557ae1a846d5e326c42b0f17dda0958d8524f3aee570",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/encode-f64_arithmetic.json": "db771cdb7eed0dd92fc7a937f85c7d218c71e9d30784022b102ee9aa396ccd54",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/encode-f64_minmax.json": "6852704757c8a64fa2bdc9f11eda41b879b30a06b8fe0ca8e0a4df990284a08b",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/encode-float_arithmetic.json": "d506838756eef32c063f2fbd8151979b075a777b2519c46d5c93f05578ed791f",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/encode-float_edges.json": "dcc9817bd9db4eb74b5b80a8980c3a61f4dc240d548edf141a9bd758c5c217c4",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/encode-integer_lanes.json": "31a8d23fc4fa7b33afbe9f524fd8d8ac4fec570006e973a5be0355d0b4ce6158",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/encode-integer_lanes_typed.json": "f935589c834b0d8c048ac59e2efd2b2af660e1f5046de555a5d1e9bcc2d3363e",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/encode-lane_memory.json": "b5e6e0341cb6a3e60d01adda822ca4ccb8ac705cc2836f98169af9d7e6f0bd6b",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/encode-lane_memory_footprint.json": "cd0bee68b843267f950b5853409d14454114310680a52095aea6f98bc2d0375f",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/encode-relaxed_swizzle.json": "dce43f9122fb3cf0ec5adbbf8cc6e0659be77d41169d58d3f1625add2ec0acec",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/encode-relaxed_swizzle_oob.json": "c7dd15cf2373b2a037837e1a1cc06946f6bbcfa6429a71a8a81549a2e9d6f644",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/encode-saturating_select.json": "3099efe2dede982bc04dfe89053da2c9b9e94495da7e4a52cc8384235dc1f273",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/encode-scalar_f64_arithmetic.json": "e6f5457d80699205bd4e65592dd88b6d9501196fee35474bfe9a6d43f8724296",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/encode-scalar_float_arithmetic.json": "719690cb6a6ec78d7f251947959760b3127b45bc685ebcb97610f9824dc5b4b4",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/encode-scalar_float_edges.json": "fdded6bddf9cf1527ef81290ad0c6683d78c1e64d7c802025c641ecd4116bbf9",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/encode-scalar_integer_lanes.json": "4034b5748ed0285552c754ee570ac61bcac83f2dea0a02e6ff5ca553917a9662",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/encode-scalar_integer_lanes_typed.json": "5e1e29808b19a25b3de74ec587583b80a28baafce93f35bf0f89c3a03cdeb045",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/encode-scalar_shuffle.json": "5f055f0d70620fc188d58d953ce0f0ae300832b3cb2b426a2de54bbd635d6673",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/encode-shuffle.json": "317ed530d8ff83eb0493bb90a89f1a43ed9c1b3a39aae1f289777beb57c47bec",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/encode-typed_f64.json": "dd464ffd1c791a13e91fcf197b634c07838dcb317a653422f61a668f3d24d249",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/encode-typed_v128.json": "1749c1a261d314621993ad7abc97e915e51185642fb3a78651948b1956872dfa",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/validate-f64_arithmetic.json": "6c71399324ba0f999a982b6418dff42e665ad00b25ea740eee2864eceb375cbe",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/validate-f64_minmax.json": "ceb984525dfc1de02eb7093d6abff5cd9cc4dceebdc7adacdadfe0a9b0b928fa",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/validate-float_arithmetic.json": "7f8fe5501d7c4bf3d3c92fecbbbba99c6800ae04d30e3c94ef1e074dbe7049dc",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/validate-float_edges.json": "abaaedab17b87f5e0d6000170c6b48c0689fadd14e0e8669e28f0c24e3bd63e0",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/validate-integer_lanes.json": "87a56ac62e41910f8aa0d4e9a83b931731ec8a23da4d6b1c86caf7548fbcaa02",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/validate-integer_lanes_typed.json": "759570908087ebe6a5cccf59c5c90d64e359fda694ffbb2531783b4e9219373e",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/validate-lane_memory.json": "d0cf0e70972426d528d72422b10a40abec5d8c5d411c6e0d6d2307e9120bf485",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/validate-lane_memory_footprint.json": "13cf5427ec87f41c0fe6b11dfd0861f1df9453257e4395fa3afa77402499fecd",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/validate-relaxed_swizzle.json": "c2390fd6574a7848bfa65853ae6d3af1e85af61c6cd1e0480e898276eebca6b8",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/validate-relaxed_swizzle_oob.json": "4099bde52d1c332757a5e7eff566a0e66580e998e4954583c4719de0e7971cde",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/validate-saturating_select.json": "2ba95f4294f73af8bcf2338505e6c55c4ed9f17b63ae0d74842b6db256ed2896",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/validate-scalar_f64_arithmetic.json": "9a42a4509db563b499951af7bc1c2c1b36b8ee17a582717582358f9ebfa171a1",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/validate-scalar_float_arithmetic.json": "00c8ab13534d4238f1c1760c8ec3f3fc6fc5b4c36b5ff51802ae87f548058f11",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/validate-scalar_float_edges.json": "3a997b89b5d1a5b59adefe6fb3e2e37d4ee5cfdeb75b6abbd1713c4a871ccd19",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/validate-scalar_integer_lanes.json": "e1bdf54750ff9345a1b0410bf7218a231ed1d4d58ff4655846056ec6e6a593fe",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/validate-scalar_integer_lanes_typed.json": "db1cf49b72e779551c37f906222a149652ecf4d2fad556366dc7c9215a6a6743",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/validate-scalar_shuffle.json": "73a5ab13ccbd0e1ab2ad1e095ee04fd81767bb107069190047a9c61bbdbc9d70",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/validate-shuffle.json": "7bf6343f9479bbcd0d245dfc79bf1549efe9a55b2948f37ab4585403478dd72d",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/validate-typed_f64.json": "86444a0ea231350ba258b34562e7833e0d633927e6db56e5da55ef64f121df12",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage-logs/validate-typed_v128.json": "d4a1f1e6f0367d7dd6876983980aa4c0cd48aee2949403f97e5ebb3c4cb269ad",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/coverage.json": "104aaf7416821de0baa3ee4ecd46d86d256c1d9c9b8f58894046ea66da7998f2",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/decision.json": "4532d86562538285ccf6682a9609b5997f469bf68e36a0d59217ebc851ec6a76",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/dependency-budget.json": "9f9f5075f118b58955f5f99913dbbaa2168403f3848a2985d5af95cf09a14b02",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-build-proof.json": "51219ece0a278e683cf876cbcc5ed6ff39a36ce371513b82f479778290d9a64c",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/Cargo.lock": "9d704c9854b30ca231109a54598c1c440a7d144b8ece5e21539971fead240820",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/Cargo.toml": "9656851608b4f5ecb5f973a5ad370c67aef45fc557d4c132469bbf012662efff",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-cli/Cargo.toml": "67dc333182e863f6bbe2a09d9c99e7519ff35a1bc41c551c49273cc1c9f935e8",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-cli/src/main.rs": "760feeee1801d17c5675ba1f35e5fed10b3e2a02eb58458435e7d9ad355e6dfe",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-cli/tests/metadata.rs": "38015f945e99ffca1c9dc18577575057050a91e4df215314d7636f3c9cc7141f",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-decode/Cargo.toml": "1ae571b0eb1a293cd0ef6a181e33d267c40b228cd1afd5dd44ef9a309d113fa1",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-decode/src/instructions.rs": "d352215a69fc6d33b188850974c364023510056c347d0ecf95ce9ba96cd05168",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-decode/src/lib.rs": "9a83e26510e41a1d73556ebc74f4402f0aecb3493068dd7f66d4851d451524d9",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-decode/src/metadata.rs": "e9b8d4d88fc5b34dccd0a1918346cfed3a03dd256129dca4c62f4e0bce8d036e",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-decode/src/module.rs": "51b8fb2669a8323f36c7b199847a002f5b751dc04a382584a7112d7dd7dde71d",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-decode/src/reader.rs": "0c061e8df82b02c1d0fb25ad2744991a2a8f0db7893ae5d5818ba89772651e6e",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/Cargo.toml": "2ee79daf9485c056793d57b1886f75e0e74b12a6ddbf69f4b18f20ecffe8171b",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/examples/embedding.rs": "d9baac3b51bacbee00c42a109831e8bd1a845158b5a0ceae46a911307d8b0907",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/examples/suspension.rs": "097175e5cbbcf1be4716d4c1b584702042558d299360392fa742a85ead331e1d",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/src/arena.rs": "cd929b222d1ccf871f06684510df6767a1e69af4d338458c0e98b88cd16c8441",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/src/canonical.rs": "0bed54a47fbca8a39c7359a9140a5284f0513a6744505ddb5a0b92d531d0776d",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/src/continuation.rs": "c30b6fb4b89999b2e6f494b9258ce9ea3ea1e39a5971643b43147026e0fc8f46",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/src/coverage.rs": "fa735d933bf1334d23c262aac2d35282a743a49aaddb0c2bbc68da4c308f3fbe",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/src/exceptions.rs": "08e306b7f60b25077f31706bf55cc4b521b0b2b713a61e8656e36fcc34c326ba",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/src/execution.rs": "241475afda76571e1ddc7886d343ca263cef4d7f66bbf95a481db6e633f1f3fe",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/src/execution/accounting.rs": "c86761a09e3c98301f54c971aa93e08da97140832c0f58e279aca4a75bef7127",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/src/execution/compact/execute.rs": "95b773913e411f8b816cc94a0def8d61e37e43d7caf503fdb95795e6e1237fbf",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/src/execution/compact/ir.rs": "a0b1a6c27d0799f5f6718c30a23ee56214cd940f17f62422fefc8f28c4f1ef16",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/src/execution/compact/lower_control.rs": "0f474dcb314ec814d0651b8f050e65ee459acf194890efb5ab89d493f115edd7",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/src/execution/compact/lower_numeric.rs": "478f4314b477a576a949d7dd5a1a6a5d6c4d69f88672fe632dc78693e0066547",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/src/execution/compact/lower_scalar.rs": "62201c38a46f7ee45c00b683c1f60f9b7264699dbc9f661fdd8b215a30020ec5",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/src/execution/compact/mod.rs": "3dec85fa82f2acfb44a093360c23eab1026bfd9d35a804f8204e256d45bcd0ef",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/src/execution/compact/source_map.rs": "cb64b63e549cf95d1687e15390f03cf1279832bfb25dbb800eb8951ce6a7013d",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/src/execution/cursor.rs": "ff93a3f0056d109366d2b77ea93d459f456d048c05b4bf05369305a3b491d3d3",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/src/execution/effects.rs": "640bf290be1691465e44f188213b644030dab6f8ca1599790f5138816e192bcd",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/src/execution/plan.rs": "68e43e6484a6c198529b333a6e5122526845544d660191121755c3108e125d7e",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/src/gc.rs": "7dcfcc9aaff6a333041d9d8a7323d1ce0fad90b382b5023e9caeeb52815b3471",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/src/gc_exec.rs": "56cf9173994d267fa4ead0af137b8f9f941f6b86a52d458f31be3ce7a32280fc",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/src/lib.rs": "dfa3a5be5e6fbbdefeb9182b83c046eb71f5557efc2b6c668b014adae1e27d6b",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/src/linker.rs": "39a2068993bcb65fb4434f42d02b56687e8ae31914840487ee4c35de07677806",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/src/numeric.rs": "5acd2cc6a4e9e9c23fd4bb25ab8a9fbc1fa1658d63f4a95e207a00963650dc92",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/src/perf.rs": "0df5f11b174b26d7e21ee00e64f3f22f2568364ae7ae96b77815b46c5ad19f16",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/src/reference.rs": "0fc6c3d773332bc39bde1e627e6c683ccfc36d525cf1a6b410c8fd69fdbac287",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/src/shared_memory.rs": "6b1a5a02e49d2d73ee04865af8d6bac68c60305f1afc754ca9b70252750bce44",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/src/simd.rs": "7582da6d9aa7acc7de81f3a63d4d76b43d8695e500f153e5eb22f909a3360633",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/src/wide.rs": "e0347c76d5a00a56396bd641ce4beb4f0d6a7500739f6de57283c1cbacc4224a",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/tests/core.rs": "54f6e18cff27f3da96ae0736d7910013883742f0215d35abb551ce324c78af1f",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/tests/exception_root_safety.rs": "91419a4df0834c064ac143e34f92a15224cea597716328a2074b0a351ff51213",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/tests/exceptions.rs": "3cc47afeeb6428154f6d0a30db6362a009c45d292278f5f03ca60c53c3342a93",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/tests/feature_profiles.rs": "dafe483c28f8683332d198935818fff885827dfc5cfd1ad62f27330490ebc14a",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/tests/performance.rs": "ac58c5b60e5610ea3127d938e14916cd0bc06d1cd2346b5f84fdf5fb33a88fc4",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/tests/performance/p016.rs": "37628b52fb270181126338a80b3a4c7ba7aa2e6ea7492f6f7fffb077238b6f8a",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/tests/performance/p017.rs": "4122d30ef0f042af886692d52de26c9a48d016f5f8e2a0bf67c6e9c9b84e9ef7",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/tests/performance/p018.rs": "e06da12e7539dbfc36bdfcc1981e87c8d926b08679f9bd2fce201987d5dafeae",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/tests/performance/p019.rs": "8294310dd4da5f9a7a05bf0bfee0d2be503ec52cdddbca4e21c6a3ee9d3b2715",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/tests/performance/p020.rs": "b6cb36f79e0d9702e7a8be4f81d254fcbd9d683d57c6baefe37f06951f611185",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/tests/performance/p021.rs": "fec19c012536565dbf2ba3e3f80f26b3fcc4e7d560a2218b5335552be2448a94",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/tests/performance/p022.rs": "e688034612986f5ccab2640e56c2ef0cadf4ad661e8b362a85455b29bffacad6",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/tests/performance/p023.rs": "6cbadd3bf64068c0ba819bed3be369c1334fb876075e9f3253c9d612666472f6",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/tests/performance/p024.rs": "bb48a497f6ef46705df70225077b224cbcec21c4d600a5f6fa22bf29bd428b59",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/tests/performance/p025.rs": "1af91ae70dae7ecd1d03abab59b76c5cc34aa3169b7154b2a9fc3034df680c2e",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/tests/performance/p026.rs": "10ecfc3d74d0e881e5f98d46f0ac33daa04085586bd2604f09cfcd5a161f2446",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/tests/performance/p027.rs": "5ad17c20bb89f535590936259526b11f09d9bde2cc17e2e94876d8083b995b71",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/tests/performance/p028.rs": "a20b1701af071f41de5493e8e69587fd0fedb2dcccfec45e9294e0773ed29edb",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/tests/performance/p029.rs": "fe926229024f5ca65b40bf38f3c62a9de0801ee4cdd49d9787565c3366d04871",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/tests/performance/p030.rs": "96ff18f747e5c9deb19359c13d5f34b2261f06474edbf69e6f4d559500cedba2",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/tests/performance/p031.rs": "b4b0f3466af3d54f2b82376dd8191682a1e8c5dfdb24da36424beeaf705286c5",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/tests/performance/p032.rs": "8cda54330226e6b8de7de2fd7565c3d1feeed95563174df6561794ce252d914a",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/tests/performance/p033.rs": "75e7d654b6e9d49b7e5929c756fb81909bf8c429d238bd24e5f29160609670c5",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/tests/performance/p034.rs": "9c8e581c4f1b5848bb4f3f38601675cccd361a91c936896b0db9db6bae8efeae",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/tests/performance/p036.rs": "1dadaf679a3c12ba6070dc5c0c74e01aef0b072a8fbf691adf96b42fbf1c27d7",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/tests/performance/p036_private.rs": "a2cec4949872b46936509fa5e7becbc23440a18cba0bf273979687c548cb12e5",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/tests/performance/p037.rs": "9f761a49e3ab4a57fe173bd406aff861f304f14f970d0ff8fc73659bf87be30e",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/tests/performance/p037_private.rs": "b35445fc643e895f66975832d43ece1525e389b876f9e87331de5042894b89ab",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/tests/performance/p038.rs": "6bacf31aeab861f7e33fceeb8466b45dddab2698e669b21f9f9d1b918318b31d",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/tests/performance/p039.rs": "95bf392e7b99132efd76c088a6902645223a665dd32f2c74bf374a6da1522810",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/tests/performance/p040.rs": "0dd66f137da18081213bf740275ce9a07722beb456572529639637c00c143732",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/tests/performance/p040_private.rs": "dcb3592b2b785c45fea707a9ba6ed6c4f5f69f2660b0e62e16fe68ab48c552d5",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-runtime/tests/proposals.rs": "8fffaa52d03e81fc7d88d0970617f1a0c8ba7602fc241ad9c65367133d2a9927",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-types/Cargo.toml": "e5c4d20d5d35fd4a15ce854f96602add82b86a1bbe4599aab69b0abcc7dc9031",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-types/src/error.rs": "a25659ee8aefcf08babfd2e4b027b698176adc100da5336a9a28f2f0664d8a2c",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-types/src/features.rs": "3c465ce7c325c9110d42a6df08ef20dfc90122e8a86a2fdbb85a6063e82b2dc5",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-types/src/indices.rs": "249359a286195771d58ad78daa775c8164de6419100fea38e6f6f172f92e2e40",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-types/src/lib.rs": "9f1ce2f00f37fba42288e5f4c78262234bf6ab453f7061d7b4907bbe4fdeca4d",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-types/src/types.rs": "d5d6a398f344494ada0d065afcab4bdee157f4803351a2ce6752ee3fc0d4e266",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-validate/Cargo.toml": "fed963beb5aa95d1c242f4131509edd95069e1b27c4e69f84f969357a04a0639",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-validate/src/acceptance.rs": "322c98e589f396f77097a7acf1dabc666f72ccbba8936703cb115ed7befd9621",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-validate/src/atomics.rs": "51d71f78adc9f140c79dad601afe64f4bc42fb2cf896a1d753a480f57f70bbc4",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-validate/src/continuations.rs": "185cf7f137c9dbd57062f95f3d36715e92420a44f5579fb09a34b7dd65029969",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-validate/src/exceptions.rs": "5afe91db3e03444019a0181f639f683c8969563ac20376c6485395c79f46c99b",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-validate/src/gc.rs": "e46a4440d3ffbea02c4905ffffa75e9582c3193697a8484ec464fdaa7a34f21f",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-validate/src/lib.rs": "6d6fd6442fdad200c494fc43775dc003ee18fbfa63632c7056e238777a283455",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-validate/src/types.rs": "2d6f8561b6d9d8eeff0bbb76510e9b9a6159d9646d7c01a01fda6f51207ec7a0",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-validate/tests/exceptions.rs": "061d15fe56d6028e54671c5dd5d86ee1069f2ac99183886a3c3441356d4da290",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/crates/wtime-validate/tests/policy.rs": "4a4c47509bd9da3850b1ab76235d585db2c7aba826f1f89ec9055a2d888774c3",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/rust-toolchain.toml": "10c67ce85265f8fc5b1a52b416af1a553e56ad96ca9843da6579e2854741e084",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/tools/wtime-bench/Cargo.lock": "4d1a59eab7ae2ce8c6abcd203afb3a20d0787e6cd536b86a919fd133196c5431",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/tools/wtime-bench/Cargo.toml": "389e92491b6caca59c924bd41a46bee79828b41459f2c3712411e3e40bcb5a3b",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/tools/wtime-bench/fixtures/simd/f64_arithmetic.wat": "1ded010deec79fb83fbee8fd7f14512270be182574bd216c7b691cc49c3e0c3b",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/tools/wtime-bench/fixtures/simd/f64_minmax.wat": "27da5ed0409e2c811187eeb54c37da2c4ed3a3592645af5b93e6908702ecdcfd",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/tools/wtime-bench/fixtures/simd/float_arithmetic.wat": "98b2b50a26ef07e2a35c811eae0398deedfce2b1f99c11c4fe2e5c6b85004372",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/tools/wtime-bench/fixtures/simd/float_edges.wat": "ba3bc8c3abb8f6cf8a9f75dd586d0d78d5bb7a178f67e16d6e41424bf6e6be26",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/tools/wtime-bench/fixtures/simd/integer_lanes.wat": "fd4846ae1a22a31b17f5dc344bb20c015388d675991c9ac3a7c809ef83e7ad44",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/tools/wtime-bench/fixtures/simd/integer_lanes_typed.wat": "19abdb4ac058a1b3fec2be1c463975e87ab5f29f124a72e39d1ce409ddb4eccb",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/tools/wtime-bench/fixtures/simd/lane_memory.wat": "4f35680b9f3cb94cba0296cccb0cf0a8f96588ea637bea83cd83a6caac059141",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/tools/wtime-bench/fixtures/simd/lane_memory_footprint.wat": "8a713b01f558578da5179920488b9ec580de5681341d8a874d2a6c58e240ae4e",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/tools/wtime-bench/fixtures/simd/relaxed_swizzle.wat": "cc13f0de9250eaf780cf6082345e9c9bfc38be317a47ef1ce306d994624dc6f5",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/tools/wtime-bench/fixtures/simd/relaxed_swizzle_oob.wat": "af53075fe822615e699161579301dd484114bdac67c4fa41621ac8b58ce6861c",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/tools/wtime-bench/fixtures/simd/saturating_select.wat": "a4edecbc5fa5ace4c9d4d69b29be6b3131f1500b6cae84d8ebafe813454f70c6",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/tools/wtime-bench/fixtures/simd/scalar_f64_arithmetic.wat": "d5bd87cd9029c7050048fea8b3932be7a0531d68cc1ab91b3578d059e7a8667a",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/tools/wtime-bench/fixtures/simd/scalar_float_arithmetic.wat": "a11d4969aaa9fb4066b72d0123f905bfa2e2e708f2f178234a0252a7bd727bab",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/tools/wtime-bench/fixtures/simd/scalar_float_edges.wat": "5bbb77c7bb55bb2f436b9fe97f4e9f6c4a512473d9943024981489aeaa2386ba",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/tools/wtime-bench/fixtures/simd/scalar_integer_lanes.wat": "60fa829c372957e8aa2533153ceff2977361ec361eb01b38d72415083caba53e",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/tools/wtime-bench/fixtures/simd/scalar_integer_lanes_typed.wat": "6b90a27d798fb8ac6254a8e8c19dce1be396600447c65c9cd3baad1c38799156",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/tools/wtime-bench/fixtures/simd/scalar_shuffle.wat": "485df07d8a162fcbcbcc537ce0c067124629eb67946ad1e432de41d9c1beef23",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/tools/wtime-bench/fixtures/simd/shuffle.wat": "ce2af411702310c035dc1a3133feaa096b40fe43def12806e1bac0915dfe877a",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/tools/wtime-bench/fixtures/simd/typed_f64.wat": "486ba5030a6eebb0c08bc2dd530b2492c916c0ecc07614ba92479541517e8e9f",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/tools/wtime-bench/fixtures/simd/typed_v128.wat": "faea7f37b608c510fbba7a2fc6bcb1491df12b6741421f8636c0a637353787d2",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/tools/wtime-bench/simd_scaling.py": "825c68bfe2136a3d7937c0873fbdf39b9ed84fdca3364f0d050d57e89b959a8b",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/tools/wtime-bench/simd_workloads.py": "c43645c67deb4efc566602800d50913a25d434c3146df5ee686fc4bb7b555f2b",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/tools/wtime-bench/src/continuation_driver.rs": "2cc5818b256463ef062ad1f36f8a858e3905ce961ffb8b56bae06b143965119a",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/tools/wtime-bench/src/gc_exception_driver.rs": "63c192dd2f0f5a376dba96b66ac3e55c40303512374b02fe96b782e33ace369f",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/tools/wtime-bench/src/gc_warm_driver.rs": "9fc4b6ef80c4e4aca86bff36a8fdec32a0d9b4871cf1d757e0d374dbf89bb5b7",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/tools/wtime-bench/src/main.rs": "9f9496929d3c60165fa061030336397d1825d9b0545a70aadeaf07208c3768e5",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/tools/wtime-bench/src/memory_driver.rs": "a7a9049232a73141d01040dbbf463b3bae2959ad3633001c55501f5359fecb0f",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/tools/wtime-bench/src/protocol.rs": "b196148f5aa437d7721c2fa058481891e14ca987e961efdf8e9a82bf346c0bd1",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/tools/wtime-bench/src/representative_driver.rs": "cdbf14e4faddd180ab7f7168e27f186cde9cf510d2e018eb5e8f302c6d0772a9",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/tools/wtime-bench/src/scalar_driver.rs": "696390941c96748fb621b5947631a121c4b7a0a81d70babc947e150832a5b337",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/tools/wtime-bench/src/thread_driver.rs": "96801dbc985a214e335c2be69dedce48a1cd2acf37055547f3d5fc87a21f4ee3",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/tools/wtime-bench/src/tier_state_driver.rs": "6215becc3a74f02bbea2dbb8d342afe879bd9b9787d2c3c777fea9b871678525",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/diagnostic-sources/tools/wtime-bench/workloads.json": "68625d67b23a57525e6414264bd179ad502d630ca73cd1c294e32c1490ed5f5a",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/licenses/LICENSE-APACHE": "a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/licenses/LICENSE-MIT": "a43aeeeebe46eda1a6fea77337eb7f9f37341b94afe7251d1f6d14fe03e702c5",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/manual-oracles.json": "6ecdf56a078416e95d7668164384e89aa661c045c46aaa06931ed54f1dbeba57",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/prepare-logs/encode-constant-bits.json": "017f9e51e8887c8440d87f6318b38e13c317c6ea01183d55538427b019bb6f4d",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/prepare-logs/encode-float-rules.json": "733f39d79ae66851d4c03895f854ad70a1e958347357c4b897157fd6d181cd35",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/prepare-logs/encode-shuffle.json": "b61992d71efa696095b60ba52cc4e55e76a61f2c58024f93cdc9637879e7ff76",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/prepare-logs/encode-traps.json": "3ca52418d041c727b288306af3f69809f38323c9659cebd247624a77158b0234",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/prepare-logs/validate-constant-bits.json": "a2dd1de61771c421f13794f1cedaef6ba756b261d53cffb549e8d550e8d815c2",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/prepare-logs/validate-float-rules.json": "17e459bb699a8f12b2957c39ee35e4d80489c609eccd060d7617e556610f65d4",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/prepare-logs/validate-shuffle.json": "0f7cf3992350ba10026309706e731b4b1187e873c8450c7b51b261f820d1e4a5",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/prepare-logs/validate-traps.json": "5b3fb93864dfc18d8d7316487286f542d64857f0beefbe16e3a823cff2ffad28",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/provenance.template.json": "be2364426edc81f396de71c0b1d978620bfc70c7c2ee4f284268b2b2e8ba5cc6",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/source-snapshots/source-after.json": "a4d051f845d328b79e76928c28294e49b807b3de8e1cf847bfcd30bbfd0f8229",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/source-snapshots/source-before.json": "a4d051f845d328b79e76928c28294e49b807b3de8e1cf847bfcd30bbfd0f8229",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/source-snapshots/workspace-source-after.json": "1257f301c948839ee46e44e54003fe728ddc608f6d5d6dfe9becf0a2ae66f3e5",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/source-snapshots/workspace-source-before.json": "1257f301c948839ee46e44e54003fe728ddc608f6d5d6dfe9becf0a2ae66f3e5",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/test-binaries/default-runtime-suite/core-fc4384471282e053": "b61cb3fc4918f3e99a332b2a3772920c72410be31fe310b9eb0df2f073896160",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/test-binaries/default-runtime-suite/exception_root_safety-863d9cca58697c87": "e5b43c003b34359077f75af985f7c469152628ad77a0bd5b09873bd6914a8f9b",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/test-binaries/default-runtime-suite/exceptions-dc82360c779184c7": "3a36d4872a8aeebf84482dd4564d53c9e813dcec1bfb4049a38b48b5eea4babd",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/test-binaries/default-runtime-suite/feature_profiles-da219ada3555dbdb": "63ad2d54c9e47970728b57e524bda60d2948d02cfa926ac9bf1deb6944a785b7",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/test-binaries/default-runtime-suite/performance-f1f918c868572e47": "930ea13f929f222f9d8f4bffa65eb20b46782639d74b4dee6a008f136e431b8a",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/test-binaries/default-runtime-suite/proposals-ce7c5c5922b88848": "c169f2b6587f291c58163806585a23a6ea716eb1d695731f536879946074dd3a",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/test-binaries/default-runtime-suite/wtime_runtime-670f0c395211a4f7": "1b148f1dd6d8f4c6d96f7502a92b04d3264b4a54d2ff65cec14a07d1dfe4e9c9",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/test-binaries/p038-regression-tests/core-42d92ac3afc77304": "60a94978f246f7c76498850c6985a97fe7dd9e489f83d242cb5a8a975f526dbe",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/test-binaries/p038-regression-tests/exception_root_safety-db67ff33a61a98ad": "e00a4d23ceed87269e7c18b51596743f03985109d9c7e76e3533ac6afed8c337",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/test-binaries/p038-regression-tests/exceptions-5d275ecd0f4f002b": "915a7c584c3ac728f607c7da785602833b33f0819a1db8cc0cb8fe748358e1f5",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/test-binaries/p038-regression-tests/feature_profiles-487957d4b265fccb": "24d7c5c7a6a5e9fdaee83439773e357ed47e387c9e63349fd2df63a21736131a",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/test-binaries/p038-regression-tests/performance-1a202aeb20ffb000": "1c8d8c31713f839db3f2ac3b2d36c0d39bd5f6a74fb36e374005b05ade9af1bc",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/test-binaries/p038-regression-tests/proposals-2b774c23db30396c": "3296633ebdd02e86022e3eab34a631a35eb33f658b0979017db1750d93307c2d",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/test-binaries/p038-regression-tests/wtime_runtime-37e3c598ab512be0": "3461938afcf989c7056802f88bbe817d5fe592fe2bacc78f9cb4eda1f9cec868",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/test-binaries/p039-focused-tests/core-42d92ac3afc77304": "60a94978f246f7c76498850c6985a97fe7dd9e489f83d242cb5a8a975f526dbe",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/test-binaries/p039-focused-tests/exception_root_safety-db67ff33a61a98ad": "e00a4d23ceed87269e7c18b51596743f03985109d9c7e76e3533ac6afed8c337",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/test-binaries/p039-focused-tests/exceptions-5d275ecd0f4f002b": "915a7c584c3ac728f607c7da785602833b33f0819a1db8cc0cb8fe748358e1f5",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/test-binaries/p039-focused-tests/feature_profiles-487957d4b265fccb": "24d7c5c7a6a5e9fdaee83439773e357ed47e387c9e63349fd2df63a21736131a",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/test-binaries/p039-focused-tests/performance-1a202aeb20ffb000": "1c8d8c31713f839db3f2ac3b2d36c0d39bd5f6a74fb36e374005b05ade9af1bc",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/test-binaries/p039-focused-tests/proposals-2b774c23db30396c": "3296633ebdd02e86022e3eab34a631a35eb33f658b0979017db1750d93307c2d",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/test-binaries/p039-focused-tests/wtime_runtime-37e3c598ab512be0": "3461938afcf989c7056802f88bbe817d5fe592fe2bacc78f9cb4eda1f9cec868",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/test-binaries/p040-regression-tests/core-42d92ac3afc77304": "60a94978f246f7c76498850c6985a97fe7dd9e489f83d242cb5a8a975f526dbe",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/test-binaries/p040-regression-tests/exception_root_safety-db67ff33a61a98ad": "e00a4d23ceed87269e7c18b51596743f03985109d9c7e76e3533ac6afed8c337",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/test-binaries/p040-regression-tests/exceptions-5d275ecd0f4f002b": "915a7c584c3ac728f607c7da785602833b33f0819a1db8cc0cb8fe748358e1f5",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/test-binaries/p040-regression-tests/feature_profiles-487957d4b265fccb": "24d7c5c7a6a5e9fdaee83439773e357ed47e387c9e63349fd2df63a21736131a",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/test-binaries/p040-regression-tests/performance-1a202aeb20ffb000": "1c8d8c31713f839db3f2ac3b2d36c0d39bd5f6a74fb36e374005b05ade9af1bc",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/test-binaries/p040-regression-tests/proposals-2b774c23db30396c": "3296633ebdd02e86022e3eab34a631a35eb33f658b0979017db1750d93307c2d",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/test-binaries/p040-regression-tests/wtime_runtime-37e3c598ab512be0": "3461938afcf989c7056802f88bbe817d5fe592fe2bacc78f9cb4eda1f9cec868",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/test-logs/P040 default-runtime-suite.json": "149ad35fe3fcc50af293891c4748f04cdad26bb2a81337ee2a7591533cd93b3c",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/test-logs/P040 feature-focused-tests.json": "fd34a73a1f1bf118577a1529e345deb0ac26edca89c7dbc14a1cdacb68375586",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/test-logs/P040 p038-regression-tests.json": "d5ac0eb002c6cd396440fb572ce13a5471ec3316cf7cb313332dffd6dc70e3e9",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/test-logs/default-runtime-suite.json": "bce40b928767f47581779daa45106f5435828e12f4d849e4b431393eb3da7d02",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/test-logs/p038-regression-tests.json": "3a4fb6daab872021601d2cfca999080c1eeef41e80d355c7923eb9271dbc863e",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/test-logs/p039-focused-tests.json": "77b661d33fde347193e59a3dc64d94fcf0e345e385ae2116f1832633077eb65f",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/test-logs/p040-regression-tests.json": "24bc6be05231fe67eee8aecb8121be9ac2ae6de4887518b8ca330e02562ecb6b",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/tools/encoder": "5123b38fd1e6b2358027778d94256202e53ff7024b3b05cd94aa5ea50d7949d4",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/artifacts/tools/validator": "c543c8a9690ddf9e5df10548b87bc476a7cc7bb9cc33031030003f4e01f776ab",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/inputs/check.json": "422dcd5d23c20793350a42c8958c2922d154a4b6398fd29072407ee2458799af",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/inputs/closure-builder.py": "4fac2d095ce7538c13f47558e97dd7b7e5eac441cdebd0beedfcadb1de3455ec",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/inputs/contract.json": "24c75900624da40ce0827228cd10da29cffaeacb9688ad19540646d6640eb613",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/inputs/coverage.json": "104aaf7416821de0baa3ee4ecd46d86d256c1d9c9b8f58894046ea66da7998f2",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/inputs/decision.json": "4532d86562538285ccf6682a9609b5997f469bf68e36a0d59217ebc851ec6a76",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/inputs/receipt.json": "227e4344bb4033cec13dbc3bce46ecd22159e8547151853839a48caf7362ed51",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/inputs/report.json": "4a213ef581e01946d498c201a46542911a336e421ae230906fadbdaf1cd2ffdb",
    "/workspace/wtime/target/performance/P039/closure-attempt-0003/simd-evidence.json": "3b804b9d36ce14be2584ad2a2e863b96e152a0c1a7c0df6dbfebfc6938575238",
    "/workspace/wtime/target/performance/P039/optional-build.json": "2e29cec1ef5d0f2422a38ce4600b00e085d1cc093e50c975f8b23d2722740a3d"
  },
  "evidence": [
    {
      "bytes": 217970,
      "path": "/workspace/wtime/target/performance/P039/closure-attempt-0003/simd-evidence.json",
      "sha256": "3b804b9d36ce14be2584ad2a2e863b96e152a0c1a7c0df6dbfebfc6938575238"
    }
  ],
  "outcome": "inconclusive",
  "status": "done",
  "task": "P039"
}
```
