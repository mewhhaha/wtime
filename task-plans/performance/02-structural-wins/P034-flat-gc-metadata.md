# P034 — Replace repeated flattened type scans with immutable layout lookups

Status: **done**

Outcome: **rejected**

Kind: `candidate` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P002 — Compare separately built baseline and candidate binaries](../00-measurement/P002-paired-driver.md)

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P011 — Measure reference, GC and exception paths](../00-measurement/P011-gc-exception-workloads.md)

[P016 — Add explicit executor policies and test integration hooks](../01-execution-contracts/P016-execution-policy.md)

[P017 — Make every Module clone share immutable metadata](../01-execution-contracts/P017-shared-module-payload.md)

[P021 — Compute bounded per-function execution metadata](../01-execution-contracts/P021-function-execution-plans.md)

[P033 — Cache immutable subtype answers with identity and work guards](P033-bounded-subtype-cache.md)

P034 may start only after P033 has a finalized non-promoted closure, explicitly recorded `candidate_promoted: false` and `production_restored: true`, and its runtime sources have been restored and verified. P033 is the immediate predecessor; no P034 candidate work or archive may precede that closure.

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

The runtime change is restricted to the optional immutable module-local type-location table, its borrowed GC lookup integration, and focused tests. The measurement work appends its own encoded rows and keeps existing registry rows/selections unchanged.

- `crates/wtime-runtime/src/gc_layout.rs`
- `crates/wtime-runtime/src/gc_exec.rs`
- `crates/wtime-runtime/src/lib.rs`
- `crates/wtime-runtime/tests/performance.rs`
- `crates/wtime-runtime/tests/performance/p034.rs`
- `crates/wtime-runtime/tests/fixtures/performance/gc/packed_gc_types_16.wat`
- `crates/wtime-runtime/tests/fixtures/performance/gc/packed_gc_types_16.wasm`
- `crates/wtime-runtime/tests/fixtures/performance/gc/packed_gc_types_256.wat`
- `crates/wtime-runtime/tests/fixtures/performance/gc/packed_gc_types_256.wasm`
- `crates/wtime-runtime/tests/fixtures/performance/gc/packed_gc_types_1024.wat`
- `crates/wtime-runtime/tests/fixtures/performance/gc/packed_gc_types_1024.wasm`
- `crates/wtime-runtime/tests/fixtures/performance/gc/equal_heap_fields_256.wat`
- `crates/wtime-runtime/tests/fixtures/performance/gc/equal_heap_fields_256.wasm`
- `crates/wtime-runtime/tests/fixtures/performance/gc/canonical_seed.wat`
- `crates/wtime-runtime/tests/fixtures/performance/gc/canonical_seed.wasm`
- `crates/wtime-runtime/tests/fixtures/performance/gc/README.md`
- `crates/wtime-runtime/tests/fixtures/performance/gc/LICENSE.txt`
- `crates/wtime-runtime/tests/fixtures/performance/gc/manifest.json`
- `tools/wtime-bench/gc_layout_workloads.py`
- `tools/wtime-bench/src/gc_warm_driver.rs`
- `tools/wtime-bench/gc_compare.py`
- `tools/wtime-bench/compare.py`
- `tools/wtime-bench/baseline.py`
- `tools/wtime-bench/workloads.py`
- `tools/wtime-bench/workloads.json`
- `tools/wtime-bench/tests/test_p034_gc_layout_model.py`
- `tools/wtime-bench/tests/test_p034_gc_compare.py`
- `tools/wtime-bench/fixtures/gc/packed_gc_types_16.wat`
- `tools/wtime-bench/fixtures/gc/packed_gc_types_16.wasm`
- `tools/wtime-bench/fixtures/gc/packed_gc_types_256.wat`
- `tools/wtime-bench/fixtures/gc/packed_gc_types_256.wasm`
- `tools/wtime-bench/fixtures/gc/packed_gc_types_1024.wat`
- `tools/wtime-bench/fixtures/gc/packed_gc_types_1024.wasm`
- `tools/wtime-bench/fixtures/gc/README.md`
- `tools/wtime-bench/fixtures/gc/LICENSE.txt`
- `tools/wtime-bench/fixtures/gc/manifest.json`
- `docs/benchmarks/performance/P034-contract.json`
- `docs/benchmarks/performance/P034-measurement-contract.json`
- `docs/benchmarks/performance/P034-build-decision.md`
- `docs/benchmarks/performance/P034-evidence.json`

One integrator owns `lib.rs`, module wiring, `tests/performance.rs`, fixture registration, and benchmark driver/registry integration. Candidate table storage is optional and module-owned; it stores only checked local group/member indices, never canonical IDs or Store/GC handles.

## Input/output contract

After validation, a module may own an immutable optional GcLayouts table of checked u32 module-local (group, member) locations shared with Module clones. Preparation uses checked counts and arithmetic, at most 100,000 bounded traversal work units, fallible reservation, and a 1 MiB retained-storage ceiling that includes the table header and actual Vec capacity. Any limit, conversion, or reservation failure publishes no partial table and preserves the borrowed checked flattened-scan fallback. Local indices remain distinct from canonical heap identities; exactness, cross-module identity, validation, GC resource behavior, and Store ownership are unchanged. The predeclared gc-layouts selection has 117 workloads: 24 primary and 93 protected.

## Implementation recipe

1. Require finalized P033 non-promoted closure with candidate_promoted=false and production_restored=true; verify restored runtime sources and the P033 predecessor archive before any P034 setup or candidate work.
2. Append 36 benchmark workload rows across six array/struct families, backed by the three encoded packed-type modules, without changing the existing 350 workload rows or any existing selection; extend the GC stage driver and independent oracle across prepare, instantiate, first_call, warm_call, and load_and_call.
3. Predeclare gc-layouts as 117 selected workloads: 24 new primary rows at n=1,32,1024,10000 and 93 protected rows comprising the prior 81 plus 12 new n=0,17 controls; keep Wasmtime unsupported for these GC workloads.
4. Prepare an optional immutable module-local GcLayouts table of checked u32 group/member locations under the 100000-work and 1 MiB retained-byte caps; count the table header and actual Vec capacity, use fallible reservation, publish only a complete table, and retain the borrowed checked flattened-scan fallback on every decline.
5. Route GC type consumers through borrowed definitions without cloning recursive SubType values; preserve canonical identities, descriptor/exactness behavior, feature admission, GC allocation limits, and Store-local ownership.
6. Add four private metadata tests for local-index separation, complete borrowed fallback, reservation-failure transactional decline, and no optional allocation for MVP modules, plus the four named public behavior/resource tests.
7. Measure the 117-row gc-layouts selection in two matched runs with 30 pairs per group, 405 groups per run, and 144 warm-calibration records; keep the 20 ms calibration target, shared matched batches, independent result/lifecycle checks, and preparation/retained-byte reporting.
8. Record exact candidate and predecessor identities, focused test counts, fixture hashes, and build/comparison evidence; if the candidate is not adopted, restore the exact P033 production runtime source set and verify restored behavior.

The cold MVP path must retain `None` without allocating optional GC locations. `GcLayouts::prepare` must include `size_of::<GcLayouts>()` and actual `Vec::capacity() * size_of::<Location>()` in its 1 MiB check. All count, work, index conversions, and byte calculations are checked; failure or over-budget work declines the entire table and keeps GC execution on the borrowed flattened lookup. The 100,000 work bound covers both bounded passes, including per-group overhead.

## Meaningful cases

Four private metadata checks and four public regressions are required. Tests must check their semantic or structural assertion rather than merely compile or invoke the new API.

- `p034_local_indices_remain_distinct_from_canonical_identity`
- `p034_bounded_failure_uses_complete_borrowed_fallback`
- `p034_reservation_failure_declines_before_publishing_locations`
- `p034_mvp_modules_do_not_allocate_optional_gc_locations`
- `p034_high_local_array_and_struct_indices_keep_packed_field_semantics`
- `p034_equal_heap_type_fields_keep_high_module_local_reference_context`
- `p034_prior_engine_canonical_history_does_not_replace_module_local_indices`
- `p034_layout_preparation_preserves_gc_resource_failure_without_live_objects`

The public high-index cases exercise arrays and structs at local type indices 16, 256, and 1024 with packed-field semantics. Equal-heap fields and prior Engine canonical history cover module-local versus canonical identity behavior. The final public case confirms a GC resource limit still rejects allocation with no live objects. These cases do not by themselves prove optional layout decline; the private fallback/reservation tests cover that path.

## Verification

First run the model/registry correctness checks before changing runtime candidate sources. After the P033 non-promoted/restored closure gate, create exactly one immediate-predecessor archive, apply the candidate, then run the focused checks and selected measurements. The GC model tests independently verify all new outputs and lifecycle behavior; `test_p034_gc_compare.py` verifies append-only registry identity, the exact selection partition, and stage/group totals. The two Rust commands must report eight focused tests (four private and four public) with zero failures/ignored tests for their respective configurations.

Commands below describe required future verification; none has been run as P034 evidence.

```sh
python3 -B tools/wtime-bench/tests/test_p034_gc_layout_model.py
python3 -B tools/wtime-bench/tests/test_p034_gc_compare.py
cargo test --locked -p wtime-runtime p034_ -- --nocapture
cargo test --locked -p wtime-runtime --features execution-reference,performance-counters p034_ -- --nocapture
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml -j 5
python3 -B tools/wtime-bench/compare.py --baseline target/performance/P034-before --candidate tools/wtime-bench/target/release/wtime-bench --candidate-mode decoded --selection gc-layouts --output target/performance/P034 --pairs 30 --build-growth-explanation-file docs/benchmarks/performance/P034-build-decision.md
python3 -B tools/wtime-bench/analyze.py --input target/performance/P034 --output target/performance/P034/analysis.json
python3 -B tools/checks/build_budget.py --output target/performance/P034-build.json
python3 -B tools/wtime-bench/gates.py --task P034 --comparison target/performance/P034 --build target/performance/P034-build.json --contract docs/benchmarks/performance/P034-contract.json --output target/performance/P034/decision.json
```

## Measurement and decision

Selection: `gc-layouts`, exactly **117** rows: **24 primary** and **93 protected**. Primary consists of the six new array/struct type-count families at n=1, 32, 1024, and 10000. Protected consists of the unchanged 81 prior GC/exception rows plus the 12 new n=0 and n=17 rows. Preserve the previous `gc-exceptions` selection exactly. Wasmtime is unsupported for the new GC workload family; report those comparisons as unsupported rather than as zero or missing data.

The separate `docs/benchmarks/performance/P034-measurement-contract.json` predeclares the full collection: two runs, 405 groups per run, 30 paired observations per group (24,300 pairs total), and 144 warm-calibration records. It records 20,000,000 ns as the warm calibration target, a shared matched batch per comparison, and the batch cap `min(100000, 100000000 // max(n, 1))`. Each new invocation allocates one object even at n=0; setup/collection stays outside the guest interval. Keep preparation and actual retained bytes visible and charge the module-owned optional table once. This rich schedule is separate from the compact `P034-contract.json` consumed by the gates.

Use the paired protocol and cold/build rules in [MEASUREMENT.md](../MEASUREMENT.md). Keep instrumentation off in production timings. Adopt only with supported gains in both runs and no protected regression; otherwise record rejected/inconclusive and restore the exact P033 production runtime sources. A `contract_only` result needs a separate, measured justification and still requires removal of rejected default-path cost. Native AArch64 verification remains an external constraint and does not block this portable decoded lane.

## Before editing

These model and registry checks precede candidate-source edits. Only after finalized P033 non-promoted-and-restored evidence is verified, run the quiet predecessor build sample and capture immutable `P034-before`; the archive must represent the restored P033 source state. Retain P001's original archive. Do not overwrite an existing archive with different source identities; choose a new recorded run-specific path and update all comparison paths if necessary.

```sh
python3 -B tools/wtime-bench/tests/test_p034_gc_layout_model.py
python3 -B tools/wtime-bench/tests/test_p034_gc_compare.py
python3 -B tools/checks/build_budget.py --output target/performance/P034-before-build.json
python3 -B tools/wtime-bench/baseline.py --mode decoded --output target/performance/P034-before
```

## Completion evidence

Record actual command exits, settings, focused test names/counts, workload/group/pair/calibration counts, and source/binary/fixture hashes. Evidence must bind the immediate predecessor to the finalized P033 restored source state and the candidate to its exact archive and build. Do not mark P034 performed from staged drafts or predeclared totals.

Update this file and TASKS.json only after the implementation and decision are complete. The task remains **todo/pending** with empty evidence until then. No commands in this planning file have been executed as P034 implementation evidence.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/tmp/P034-completion-attempts/attempt-0001/P034-complete.snapshot.py": "c5648bc5ebbc3f30f84e3b69cc8e40833e746c96bd7eb99139f4053e518de9ed",
    "/workspace/wtime/crates/wtime-runtime/src/gc_exec.rs": "56cf9173994d267fa4ead0af137b8f9f941f6b86a52d458f31be3ce7a32280fc",
    "/workspace/wtime/crates/wtime-runtime/src/lib.rs": "997a0ec8d835c83567e13aa4c755ed30ad0e44a0bdb9e689c14753abdd47bb45",
    "/workspace/wtime/docs/benchmarks/performance/P034-build-decision.md": "404eb8be2c91537e5cff5ed0daf9a883de80fdaf60601707cf44b72bf65fc69e",
    "/workspace/wtime/docs/benchmarks/performance/P034-candidate.patch": "499da996452956246f71df6ba860b3c042e0d47d2e87f7b7f554eef77025be76",
    "/workspace/wtime/docs/benchmarks/performance/P034-contract.json": "1e0038bec040eb356dff993f1f2c1d862f203ce1056fde6338d8ed530b6bc91e",
    "/workspace/wtime/docs/benchmarks/performance/P034-measurement-contract.json": "0471ddb2b3c6a43d5c65686af402c4bb3d132dfac7df8469b97b8a151d97506b",
    "/workspace/wtime/target/performance/P033-before-corrected/binary/wtime-bench": "56871bfaedf83bd239485a618ba927b3f6aecf00494a03bf0ed880a72d3da378",
    "/workspace/wtime/target/performance/P033-before-corrected/binary/wtime-conformance": "5123b38fd1e6b2358027778d94256202e53ff7024b3b05cd94aa5ea50d7949d4",
    "/workspace/wtime/target/performance/P033-before-corrected/fixtures/branch_calls.wasm": "9b58d26f201900b5b6cce684133a98ab68b3cf55d979b64ce895e914e9d5327b",
    "/workspace/wtime/target/performance/P033-before-corrected/fixtures/branch_calls.wat": "d0af4fdf9ad5be38344267ae72fc1855edb5a8fa1b475fd0af4cb507730b2309",
    "/workspace/wtime/target/performance/P033-before-corrected/fixtures/call_overhead.wasm": "0815ca7d6026206d0773fe10c6b9643862bfffe8ba361c9789369ef9668ee746",
    "/workspace/wtime/target/performance/P033-before-corrected/fixtures/call_overhead.wat": "9493b8f751690a2fd6e8b19dcee5e2c1d50b717253d292ce34d5692b306663a5",
    "/workspace/wtime/target/performance/P033-before-corrected/fixtures/integer_loop.wasm": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c",
    "/workspace/wtime/target/performance/P033-before-corrected/fixtures/integer_loop.wat": "791730112109f3d5751a7b3e2c1134e7e0e6f71a7f7ee01ea8481eb989ee3fa4",
    "/workspace/wtime/target/performance/P033-before-corrected/fixtures/memory_fill_scan.wasm": "1c933d404fe9feb93bb664af3bbb60a50319cdd6a2c29d442aa9ab7799f5e65f",
    "/workspace/wtime/target/performance/P033-before-corrected/fixtures/memory_fill_scan.wat": "c0a222245af932b6d43ca964fa710f56476dc90ebaca0b20915d35b242a6c87d",
    "/workspace/wtime/target/performance/P033-before-corrected/fixtures/prepare_large.wasm": "c489fe06970f8e1c60e4992774aaadb571f676833d6579de1f67a5facf3fe9b0",
    "/workspace/wtime/target/performance/P033-before-corrected/fixtures/prepare_large.wat": "aefc7237fa17bfbccd8c318bc104111c8e3296933a94017837d2c02130d638fd",
    "/workspace/wtime/target/performance/P033-before-corrected/historical_reports/wasmtime-comparison-run2.json": "6818fa194027b57b183ca3a42a55627c676ba857392497fa325345e0f3fd090b",
    "/workspace/wtime/target/performance/P033-before-corrected/historical_reports/wasmtime-comparison.json": "b1b69f3944b488a037c08305a43bd8de474837958c53217f1c3cb94b7d04354c",
    "/workspace/wtime/target/performance/P033-before-corrected/manifest.json": "d154ed03cd99ebffe86c97136981838c32371ccb961b8466c3248e260a3a2765",
    "/workspace/wtime/target/performance/P033-before-corrected/runs/historical-format.json": "ae1b82c765436bd81c40a37c895acbc04d08d67da6c19155f4bd793ef0258310",
    "/workspace/wtime/target/performance/P033-before-corrected/runs/mode-smoke.json": "895cd349f44d590554628fd95b326f0df9edc708d235e6ceb0c29ff0afbf46f6",
    "/workspace/wtime/target/performance/P033-before-corrected/source/Cargo.lock": "9d704c9854b30ca231109a54598c1c440a7d144b8ece5e21539971fead240820",
    "/workspace/wtime/target/performance/P033-before-corrected/source/Cargo.toml": "9656851608b4f5ecb5f973a5ad370c67aef45fc557d4c132469bbf012662efff",
    "/workspace/wtime/target/performance/P033-before-corrected/source/LICENSE-APACHE": "a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2",
    "/workspace/wtime/target/performance/P033-before-corrected/source/LICENSE-MIT": "a43aeeeebe46eda1a6fea77337eb7f9f37341b94afe7251d1f6d14fe03e702c5",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-cli/Cargo.toml": "67dc333182e863f6bbe2a09d9c99e7519ff35a1bc41c551c49273cc1c9f935e8",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-cli/src/main.rs": "760feeee1801d17c5675ba1f35e5fed10b3e2a02eb58458435e7d9ad355e6dfe",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-cli/tests/metadata.rs": "38015f945e99ffca1c9dc18577575057050a91e4df215314d7636f3c9cc7141f",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-decode/Cargo.toml": "1ae571b0eb1a293cd0ef6a181e33d267c40b228cd1afd5dd44ef9a309d113fa1",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-decode/src/instructions.rs": "d352215a69fc6d33b188850974c364023510056c347d0ecf95ce9ba96cd05168",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-decode/src/lib.rs": "9a83e26510e41a1d73556ebc74f4402f0aecb3493068dd7f66d4851d451524d9",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-decode/src/metadata.rs": "e9b8d4d88fc5b34dccd0a1918346cfed3a03dd256129dca4c62f4e0bce8d036e",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-decode/src/module.rs": "51b8fb2669a8323f36c7b199847a002f5b751dc04a382584a7112d7dd7dde71d",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-decode/src/reader.rs": "0c061e8df82b02c1d0fb25ad2744991a2a8f0db7893ae5d5818ba89772651e6e",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-runtime/Cargo.toml": "2ee79daf9485c056793d57b1886f75e0e74b12a6ddbf69f4b18f20ecffe8171b",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-runtime/examples/embedding.rs": "d9baac3b51bacbee00c42a109831e8bd1a845158b5a0ceae46a911307d8b0907",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-runtime/examples/suspension.rs": "097175e5cbbcf1be4716d4c1b584702042558d299360392fa742a85ead331e1d",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-runtime/src/arena.rs": "cd929b222d1ccf871f06684510df6767a1e69af4d338458c0e98b88cd16c8441",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-runtime/src/canonical.rs": "0bed54a47fbca8a39c7359a9140a5284f0513a6744505ddb5a0b92d531d0776d",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-runtime/src/continuation.rs": "c30b6fb4b89999b2e6f494b9258ce9ea3ea1e39a5971643b43147026e0fc8f46",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-runtime/src/coverage.rs": "fa735d933bf1334d23c262aac2d35282a743a49aaddb0c2bbc68da4c308f3fbe",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-runtime/src/exceptions.rs": "08e306b7f60b25077f31706bf55cc4b521b0b2b713a61e8656e36fcc34c326ba",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-runtime/src/execution.rs": "d2ef4ca6928c9c9f9ca7c02f9a94d0a4b8bdfd5b560ed1d3d6dab93fcb3f751d",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-runtime/src/execution/accounting.rs": "c86761a09e3c98301f54c971aa93e08da97140832c0f58e279aca4a75bef7127",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-runtime/src/execution/cursor.rs": "ff93a3f0056d109366d2b77ea93d459f456d048c05b4bf05369305a3b491d3d3",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-runtime/src/execution/effects.rs": "640bf290be1691465e44f188213b644030dab6f8ca1599790f5138816e192bcd",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-runtime/src/execution/plan.rs": "68e43e6484a6c198529b333a6e5122526845544d660191121755c3108e125d7e",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-runtime/src/gc.rs": "7dcfcc9aaff6a333041d9d8a7323d1ce0fad90b382b5023e9caeeb52815b3471",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-runtime/src/gc_exec.rs": "56cf9173994d267fa4ead0af137b8f9f941f6b86a52d458f31be3ce7a32280fc",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-runtime/src/lib.rs": "997a0ec8d835c83567e13aa4c755ed30ad0e44a0bdb9e689c14753abdd47bb45",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-runtime/src/linker.rs": "39a2068993bcb65fb4434f42d02b56687e8ae31914840487ee4c35de07677806",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-runtime/src/numeric.rs": "5acd2cc6a4e9e9c23fd4bb25ab8a9fbc1fa1658d63f4a95e207a00963650dc92",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-runtime/src/perf.rs": "df87993399e014674e63c8f15ea9dfb74888fe86ba5bd39a9129a4048121b3e5",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-runtime/src/reference.rs": "0fc6c3d773332bc39bde1e627e6c683ccfc36d525cf1a6b410c8fd69fdbac287",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-runtime/src/shared_memory.rs": "6b1a5a02e49d2d73ee04865af8d6bac68c60305f1afc754ca9b70252750bce44",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-runtime/src/simd.rs": "84c223e6a546aa8955ff5214b9add03df4c94d25f3ffcd45b33a808d299334b1",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-runtime/src/wide.rs": "e0347c76d5a00a56396bd641ce4beb4f0d6a7500739f6de57283c1cbacc4224a",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-runtime/tests/core.rs": "54f6e18cff27f3da96ae0736d7910013883742f0215d35abb551ce324c78af1f",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-runtime/tests/exception_root_safety.rs": "91419a4df0834c064ac143e34f92a15224cea597716328a2074b0a351ff51213",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-runtime/tests/exceptions.rs": "3cc47afeeb6428154f6d0a30db6362a009c45d292278f5f03ca60c53c3342a93",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-runtime/tests/feature_profiles.rs": "dafe483c28f8683332d198935818fff885827dfc5cfd1ad62f27330490ebc14a",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-runtime/tests/performance.rs": "1acd1c1fbbe1e4bc296bc2ea32c15f91ee527bffc6dc469e6fca86abd9add760",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-runtime/tests/performance/p016.rs": "5ff37ef565ffc5c64c743723f1bfdabe85a315e784f1c81384f17b48a71dc21f",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-runtime/tests/performance/p017.rs": "4122d30ef0f042af886692d52de26c9a48d016f5f8e2a0bf67c6e9c9b84e9ef7",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-runtime/tests/performance/p018.rs": "e06da12e7539dbfc36bdfcc1981e87c8d926b08679f9bd2fce201987d5dafeae",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-runtime/tests/performance/p019.rs": "8294310dd4da5f9a7a05bf0bfee0d2be503ec52cdddbca4e21c6a3ee9d3b2715",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-runtime/tests/performance/p020.rs": "b6cb36f79e0d9702e7a8be4f81d254fcbd9d683d57c6baefe37f06951f611185",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-runtime/tests/performance/p021.rs": "fec19c012536565dbf2ba3e3f80f26b3fcc4e7d560a2218b5335552be2448a94",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-runtime/tests/performance/p022.rs": "e688034612986f5ccab2640e56c2ef0cadf4ad661e8b362a85455b29bffacad6",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-runtime/tests/performance/p023.rs": "6cbadd3bf64068c0ba819bed3be369c1334fb876075e9f3253c9d612666472f6",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-runtime/tests/performance/p024.rs": "bb48a497f6ef46705df70225077b224cbcec21c4d600a5f6fa22bf29bd428b59",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-runtime/tests/performance/p025.rs": "1af91ae70dae7ecd1d03abab59b76c5cc34aa3169b7154b2a9fc3034df680c2e",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-runtime/tests/performance/p026.rs": "10ecfc3d74d0e881e5f98d46f0ac33daa04085586bd2604f09cfcd5a161f2446",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-runtime/tests/performance/p027.rs": "5ad17c20bb89f535590936259526b11f09d9bde2cc17e2e94876d8083b995b71",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-runtime/tests/performance/p028.rs": "a20b1701af071f41de5493e8e69587fd0fedb2dcccfec45e9294e0773ed29edb",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-runtime/tests/performance/p029.rs": "fe926229024f5ca65b40bf38f3c62a9de0801ee4cdd49d9787565c3366d04871",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-runtime/tests/performance/p030.rs": "96ff18f747e5c9deb19359c13d5f34b2261f06474edbf69e6f4d559500cedba2",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-runtime/tests/performance/p031.rs": "b4b0f3466af3d54f2b82376dd8191682a1e8c5dfdb24da36424beeaf705286c5",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-runtime/tests/performance/p032.rs": "8cda54330226e6b8de7de2fd7565c3d1feeed95563174df6561794ce252d914a",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-runtime/tests/performance/p033.rs": "75e7d654b6e9d49b7e5929c756fb81909bf8c429d238bd24e5f29160609670c5",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-runtime/tests/proposals.rs": "8fffaa52d03e81fc7d88d0970617f1a0c8ba7602fc241ad9c65367133d2a9927",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-types/Cargo.toml": "e5c4d20d5d35fd4a15ce854f96602add82b86a1bbe4599aab69b0abcc7dc9031",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-types/src/error.rs": "a25659ee8aefcf08babfd2e4b027b698176adc100da5336a9a28f2f0664d8a2c",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-types/src/features.rs": "3c465ce7c325c9110d42a6df08ef20dfc90122e8a86a2fdbb85a6063e82b2dc5",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-types/src/indices.rs": "249359a286195771d58ad78daa775c8164de6419100fea38e6f6f172f92e2e40",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-types/src/lib.rs": "9f1ce2f00f37fba42288e5f4c78262234bf6ab453f7061d7b4907bbe4fdeca4d",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-types/src/types.rs": "d5d6a398f344494ada0d065afcab4bdee157f4803351a2ce6752ee3fc0d4e266",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-validate/Cargo.toml": "fed963beb5aa95d1c242f4131509edd95069e1b27c4e69f84f969357a04a0639",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-validate/src/acceptance.rs": "322c98e589f396f77097a7acf1dabc666f72ccbba8936703cb115ed7befd9621",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-validate/src/atomics.rs": "51d71f78adc9f140c79dad601afe64f4bc42fb2cf896a1d753a480f57f70bbc4",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-validate/src/continuations.rs": "185cf7f137c9dbd57062f95f3d36715e92420a44f5579fb09a34b7dd65029969",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-validate/src/exceptions.rs": "5afe91db3e03444019a0181f639f683c8969563ac20376c6485395c79f46c99b",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-validate/src/gc.rs": "e46a4440d3ffbea02c4905ffffa75e9582c3193697a8484ec464fdaa7a34f21f",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-validate/src/lib.rs": "6d6fd6442fdad200c494fc43775dc003ee18fbfa63632c7056e238777a283455",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-validate/src/types.rs": "2d6f8561b6d9d8eeff0bbb76510e9b9a6159d9646d7c01a01fda6f51207ec7a0",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-validate/tests/exceptions.rs": "061d15fe56d6028e54671c5dd5d86ee1069f2ac99183886a3c3441356d4da290",
    "/workspace/wtime/target/performance/P033-before-corrected/source/crates/wtime-validate/tests/policy.rs": "4a4c47509bd9da3850b1ab76235d585db2c7aba826f1f89ec9055a2d888774c3",
    "/workspace/wtime/target/performance/P033-before-corrected/source/rust-toolchain.toml": "10c67ce85265f8fc5b1a52b416af1a553e56ad96ca9843da6579e2854741e084",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/Cargo.lock": "4d1a59eab7ae2ce8c6abcd203afb3a20d0787e6cd536b86a919fd133196c5431",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/Cargo.toml": "389e92491b6caca59c924bd41a46bee79828b41459f2c3712411e3e40bcb5a3b",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/README.md": "4a679dbd985d5534f4cbaf26aaa92dc397852b12a4af0687c73aea085c3608c7",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/analyze.py": "bbdbc3c63d4d55f80e9f61942795f09a5eb3de0489194842bd1175a895f84b36",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/baseline.py": "b70470f28f74b6847fbe7e321def21a4047c4384b93ea4461145b3cd98779fd8",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/checkpoint.py": "827a1f659ddd524f9e06c37e4fb47ecf3c8bdc1d2c7e454cf195a6e787328b37",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/compare.py": "6b70527977d238f19a8a73dab8cdcbee2f08564ad1cf0dadb830b956d8b9f847",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/continuation_scaling.py": "2c078358502ed394c29e8f99aac611eed1e31311749a5856683969a7aacd1961",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/continuation_workloads.py": "384f68be5a40308aee52e0b8d8288511f36a0c2347ba7ebb36fa0a7bfdb7b872",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/fixtures/exceptions/caught_throw.wat": "5433ec09235db09ebfd9e02c9ad89afb76732b7b42586c9204d0bbb5c67245f9",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/fixtures/exceptions/cross_call_catch.wat": "d8b1444bb104bc896f56ad1e67fca80a5977c58f52e05805b231c4753d2ea521",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/fixtures/exceptions/host_throw_catch.wat": "52c9f9b5da3990de5a948b6614680fc0099891c226d4b6903ed56560a3d2b087",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/fixtures/exceptions/normal_path.wat": "cd26d75fd8d2f01183fd3c40ad2b0eb5865d433a4efe2826cd86dda709e2d9cb",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/fixtures/exceptions/throw_ref_unwind.wat": "47a2d51cc83f365c3099c82712c57d422968535750eb91f078868c204ebf0f9f",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/fixtures/exceptions/uncaught_throw.wat": "e89d6eca82d1d903fcac6dd8bda281f6ec837f4c268513c416b979528a674fd8",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/fixtures/gc/continuation_root.wat": "05d3278c9be02485919784bae44f7b5fcd55a63856296b0fb1e4a2c854379448",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/fixtures/gc/descriptor_consumer.wat": "f80a41c42745545c0007917baeb5b905f3085a49b507cb8e9f4331fec1607118",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/fixtures/gc/descriptor_graph.wat": "f7bf5ef8fb302e705e232586f720e9d864e5431be67b29590ea9b7f98b9b1a93",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/fixtures/gc/descriptor_provider.wat": "3197bba5170209b278b8a436d09923fd383255d10b51fe0014383f5099dec9d3",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/fixtures/gc/host_reentry_collect.wat": "91d9a3ba857c39ace65f4a9e1e3ff91cdfe96ca0b52ad45539b63a61534ae7cd",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/fixtures/gc/nominal_exact_casts.wasm": "6fc8a2cac95a90fe0d1a3d7c1cf1a9f57041df41f6ec8ccae747c1368b9f03e2",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/fixtures/gc/nominal_exact_casts.wat": "278b748822210879d166c0131c10b88ac32b38bf8437c1ada05667e1d8156ada",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/fixtures/gc/packed_array.wat": "7580ef1c80342937d900b7acac0bde6e4cddf21fc03de15b9da0d7c91dad6a8d",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/fixtures/gc/root_lifecycle.wat": "79f4ac8511123ba14bdde5a31cd1953bae1c69f1dcba9ec5878785e1db447a51",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/fixtures/gc/struct_graph.wat": "9fe9e8f7c7c9cd995d063bf4a54eb8d4aab61eb526f101b8c50126c50ddbc8df",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/fixtures/gc/type_churn_1024.wasm": "09f4dcbb91c1575f2f4ad2115d6198ae7959f048c66028ada960b5b4a2061bfd",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/fixtures/gc/type_churn_1024.wat": "cc0e5ea4534c2464bf1b1b286ece900942882174da20af3d506ddff6f8ef813b",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/fixtures/gc/type_churn_256.wasm": "bd136a0aeae3917c1af98d7a50de8f8b78b7db1ed1e4b3a92ebc5fa3bb172f17",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/fixtures/gc/type_churn_256.wat": "f5431b25768d26b0022060c7c5d8fb58049c55393d3f68e1b9b3657db527c5b2",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/fixtures/gc/type_churn_64.wasm": "37663a4ba8344cc5d09120a231c4e71699d1a9038e9a9788a5b5bfc03fe2b930",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/fixtures/gc/type_churn_64.wat": "6688be1ceb64af3446c2027fa03639e136fec8d27646915888755f13c4fd1de0",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/fixtures/gc/type_repeat_128.wasm": "a85c15e59addfdd67d5d246a309fd5bbfe958b744a73863baec0500791d13084",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/fixtures/gc/type_repeat_128.wat": "bb8a419be00efc018b7715550878ec1aed52c20140acacf2b1aa840ca8ad94c0",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/fixtures/gc/typed_function_refs.wasm": "c07a43c30d251223ebd9769ef3742eb32ec5d3ef751d6a9f0e1507ad353b4cb7",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/fixtures/gc/typed_function_refs.wat": "5d62ffda642de10f8ddbfa828512e8bc7d36d3cbcd70ce2e52f5532c030d990b",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/fixtures/scalar/accumulator.wat": "61aaec7a86d481378f31f3e27b0c1413684065841fa5909297181fcad9bef10d",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/fixtures/scalar/br_table.wat": "637b96cd8926ffadf728af4949b43fa6b62fffb61968c14c716e0c203a1e5e6d",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/fixtures/scalar/branch_entropy.wat": "6b4861d8bfbbd7e2718e577ec2d479bfff32e94dc47a3b7dda9a1b363baa98b6",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/fixtures/scalar/call_ref.wat": "0323154b79000eeeb0a63310541ec91b6c4d91e32a6a3406125a30f2c8e53a12",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/fixtures/scalar/dependent.wat": "0dd5e9778eb09bac9ee432e5fece3f432d034ea9878d2302972142501f0c60c6",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/fixtures/scalar/direct_calls.wat": "0a075601a2a651f9cb95e8ee0404c64bbf076d17bf242b9c88fecbb8cb973826",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/fixtures/scalar/dynamic_api.wat": "f02cc93a4a43691ce5be8782fa2df454fc2468a7935117ea18104a7dce22cb2d",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/fixtures/scalar/host_call.wat": "4c4026de620eff0b1e9cb4ed4e0440e6045d921267e3962715f6430ac68f5120",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/fixtures/scalar/indirect_calls.wat": "29024ae23844664a9fea4a809d1ce414de40f243f7e678ce5ea6d811843f7797",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/fixtures/scalar/multivalue_branch.wat": "a1f4958100af82889208bb28090d6af7c3e56cbeeccabad4106601c9a9c17014",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/fixtures/scalar/ordinary_recursion.wat": "aca6a66637fa36b72f3c528dd5e3e44039b573f363b5b4fb77437e9278623903",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/fixtures/scalar/table_controls.wat": "9b61bd9cbca804bf639085769fd766a2cf8a69d516fc7efb9e4edfe4ff6d18c9",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/fixtures/scalar/tail_calls.wat": "9953f19384c51b579921e6f789b13a8b57c86ba86d760b77846e3e33825ece02",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/fixtures/scalar/try_table_normal.wat": "ab92800dd4e4d4053498c317effadedf52568270ab7b476eef74b09638de8964",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/gates.py": "f9a054c4cc6427172544dc0720d09e22496d9f562d0b3d05e27dbbc1a7875d89",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/gc_cold_adapter.py": "4f09ca9a30dff35caefb986660ecbb9266cbaf668d550d4e698d6a4e133f4690",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/gc_compare.py": "46c927b84816ed9b9477615d4c79b8383f27abe748890399c58c557325b0293f",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/gc_scaling.py": "58a6f5419f016a4f1f35512d8ae713041620449bd39d48a98685b251f5701f7e",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/gc_warm_workloads.py": "7082bd76a060337ecc491c69bf7c177648ac1cbfc8835a7dc7a75c26e00464fc",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/gc_workloads.py": "aedd39f75c5c407690251ccd40b8a00139d9a6b9b41f3a1b9980df90b37f543c",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/memory_scaling.py": "9dfac7c395c4207f43102a84f6f83858b42d2d52894eea9457d5416a76a9bf8b",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/memory_workloads.py": "99eaae89bd1d0a95c67cc8f08c22184b95701d97a14354b93a112246295b743b",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/p027_memory_workloads.py": "2b7bf4e7c9cb6c7674c4f465bdca0c9b498f6a1f8bb4843720d0dcc2aee66336",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/profile.py": "113475824a37c1e8618c046b0b0f87ab4e85b8a49bb3e65a85e0e6b742dacb84",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/representative_scaling.py": "a2fe1dfc4406aaf3f56124cf58423a56b8add89d0874de43968e21efac360b63",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/representative_workloads.py": "7ca9e9c08928d7098a85a228a8dbf2081361b3730010e460cf8ea60afdd1204e",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/run.py": "f793c150ae38031467c91a70cddf66e85b6cd6d9782e227b773b4da6ff564bea",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/scalar_workloads.py": "b564e862913ed3fbd9a54bec0c722962fe999c25024375de69c1ceac03d2682e",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/scaling.py": "0a9415ddef9c3900c0ce9c64e286fc767367d0a4ce3dddaf483d04494ef630a3",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/simd_scaling.py": "825c68bfe2136a3d7937c0873fbdf39b9ed84fdca3364f0d050d57e89b959a8b",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/simd_workloads.py": "c43645c67deb4efc566602800d50913a25d434c3146df5ee686fc4bb7b555f2b",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/src/continuation_driver.rs": "2cc5818b256463ef062ad1f36f8a858e3905ce961ffb8b56bae06b143965119a",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/src/gc_exception_driver.rs": "63c192dd2f0f5a376dba96b66ac3e55c40303512374b02fe96b782e33ace369f",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/src/gc_warm_driver.rs": "c2da97094a5a46411e6e940d1bda1b709925afec29e95e19f0d2a489903ab31b",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/src/main.rs": "9f9496929d3c60165fa061030336397d1825d9b0545a70aadeaf07208c3768e5",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/src/memory_driver.rs": "a7a9049232a73141d01040dbbf463b3bae2959ad3633001c55501f5359fecb0f",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/src/protocol.rs": "550dd14c99e4715ff16e8ecf8a1520dd8f5d25d32c27748259ccc1e9f434573d",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/src/representative_driver.rs": "26de3df2f0d5b6198448e5cb32e28d4c24086e8c2b9ae23dcfe55663920e40b7",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/src/scalar_driver.rs": "c43a3369b6dfac8b57aaf789dbabd9da8dab4f28751571a683b970dd09225a58",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/src/thread_driver.rs": "96801dbc985a214e335c2be69dedce48a1cd2acf37055547f3d5fc87a21f4ee3",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/src/tier_state_driver.rs": "6215becc3a74f02bbea2dbb8d342afe879bd9b9787d2c3c777fea9b871678525",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/thread_scaling.py": "b8384bab6beedb5fc524ac26a91d9033be48a3c4a0b385834e8d94a83bdbc35d",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/thread_workloads.py": "f50f832f5f0eed010b3277fdcb15cacd0b600298026df1ebac1e00265864f21c",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/tier_check.py": "680766b24821f50b47bfa43c8e1c757020b80257f962dd2ae2c06cdc37de96a5",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/workloads.json": "f9389b590d8600c96bb45b9c9d65fd67cb5574a8896543293ecc80ec352daeac",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-bench/workloads.py": "c08faf6b9295f22b3b554819b59d2367c5a436c16308029bccbf85095f425096",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-conformance/Cargo.toml": "37c21479a09390a590f0e0ccba31e62ebfddf4096fa3c16039042af9c223c68e",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-conformance/build.rs": "1e37a8f6ec1af7964228b03abb3e9675c2c5218edf3bb43a94d76243cc893ef1",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-conformance/src/adapter.rs": "b99ae377d141ca6369569cbb1960f00910cc8d0b92fd5007ced206f4ebe0f997",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-conformance/src/bin/wtime-allocation-check.rs": "1a20e70ac081c7792732bc1cffded19935e19d0141bca5ea78a6b6086f4c6034",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-conformance/src/bin/wtime-probe.rs": "039e380652f8ef356d0995c35a7f14b289479db40f59e9f88d6ccfa9ef9df785",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-conformance/src/cli.rs": "ab8466680604fc36dc5e1aafbe06d7fbecd8e1cb1f5ad321099cb3a5094017ae",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-conformance/src/corpus.rs": "b1d2a7947b5904b9bd80c0109bc8b5e0f5f8329ba92a693db3f4409cd93cbc1e",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-conformance/src/main.rs": "ecf37373cd8988e4f45569d49a0a971fc85a5b687c091ae2ec7964ec8f35c169",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-conformance/src/report.rs": "a5bb757ec60513a95f0b9573328a9ee77159a2e061741bb695b52c1fef4348f5",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-conformance/src/script.rs": "8c11af05587c584895b9e945c1d0075181b9fb108ef60ae311416cb2b1971888",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-conformance/src/spectest.rs": "1403166f4be6ce443b47d4a7ba4a44da553c67e280dfa2876b3f2ad1e64e8883",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-conformance/tests/annotations.rs": "5eb54a28e57e7e929bcb2342d65c28e23c2ae112b7c10b36f707fa580f489b08",
    "/workspace/wtime/target/performance/P033-before-corrected/source/tools/wtime-conformance/tests/interactions.rs": "fd36710c8cd2ef7f3a2883d3f0ab6cbfecace3ae80adf693095228e4f4641938",
    "/workspace/wtime/target/performance/P033/final-decision.json": "5cb7bf1750b3cdb11895ef22d475773a1d135cde166233cd4fe6dc57137546f4",
    "/workspace/wtime/target/performance/P033/rollback-identities.json": "f0424c4c6cf7d8c7d1a157a5f390ce6850e761de71d5e6bac257028cafce45d8",
    "/workspace/wtime/target/performance/P034-before-build.json": "cc67a47348d3e86fde0c4d13c5b2c51203bc441c0a46aa4b9648e40abff8bac4",
    "/workspace/wtime/target/performance/P034-before-capture/baseline_archive.log": "939a5f0b72b9ff926e028b1a7d868d94409ae659e21822d702e077ec51b15291",
    "/workspace/wtime/target/performance/P034-before-capture/before-executed.py": "6936995ae0e071febf3769619d80d1acaaa3a113d5ceb86bff4008b61285d77d",
    "/workspace/wtime/target/performance/P034-before-capture/before_build.log": "ebbb52bfd65e843a0858a2af2aaca4e75ae384f2c59d408e983c0b75adf5760f",
    "/workspace/wtime/target/performance/P034-before-capture/record.json": "e483e98644b7e3eb7c308abb0f6f0dc8fb745d4304895c38436141c0ce5de05f",
    "/workspace/wtime/target/performance/P034-before/binary/wtime-bench": "2d7db8e960b53ff2b346b0eeaf5762877e1cb347ffba4853fc264f7769e8f6b8",
    "/workspace/wtime/target/performance/P034-before/binary/wtime-conformance": "5123b38fd1e6b2358027778d94256202e53ff7024b3b05cd94aa5ea50d7949d4",
    "/workspace/wtime/target/performance/P034-before/fixtures/branch_calls.wasm": "9b58d26f201900b5b6cce684133a98ab68b3cf55d979b64ce895e914e9d5327b",
    "/workspace/wtime/target/performance/P034-before/fixtures/branch_calls.wat": "d0af4fdf9ad5be38344267ae72fc1855edb5a8fa1b475fd0af4cb507730b2309",
    "/workspace/wtime/target/performance/P034-before/fixtures/call_overhead.wasm": "0815ca7d6026206d0773fe10c6b9643862bfffe8ba361c9789369ef9668ee746",
    "/workspace/wtime/target/performance/P034-before/fixtures/call_overhead.wat": "9493b8f751690a2fd6e8b19dcee5e2c1d50b717253d292ce34d5692b306663a5",
    "/workspace/wtime/target/performance/P034-before/fixtures/integer_loop.wasm": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c",
    "/workspace/wtime/target/performance/P034-before/fixtures/integer_loop.wat": "791730112109f3d5751a7b3e2c1134e7e0e6f71a7f7ee01ea8481eb989ee3fa4",
    "/workspace/wtime/target/performance/P034-before/fixtures/memory_fill_scan.wasm": "1c933d404fe9feb93bb664af3bbb60a50319cdd6a2c29d442aa9ab7799f5e65f",
    "/workspace/wtime/target/performance/P034-before/fixtures/memory_fill_scan.wat": "c0a222245af932b6d43ca964fa710f56476dc90ebaca0b20915d35b242a6c87d",
    "/workspace/wtime/target/performance/P034-before/fixtures/prepare_large.wasm": "c489fe06970f8e1c60e4992774aaadb571f676833d6579de1f67a5facf3fe9b0",
    "/workspace/wtime/target/performance/P034-before/fixtures/prepare_large.wat": "aefc7237fa17bfbccd8c318bc104111c8e3296933a94017837d2c02130d638fd",
    "/workspace/wtime/target/performance/P034-before/historical_reports/wasmtime-comparison-run2.json": "6818fa194027b57b183ca3a42a55627c676ba857392497fa325345e0f3fd090b",
    "/workspace/wtime/target/performance/P034-before/historical_reports/wasmtime-comparison.json": "b1b69f3944b488a037c08305a43bd8de474837958c53217f1c3cb94b7d04354c",
    "/workspace/wtime/target/performance/P034-before/manifest.json": "616436907ab25bc65d1f21c11e42cc5a1d6e4b9baf3d2abe5c98622ddf18fef2",
    "/workspace/wtime/target/performance/P034-before/runs/historical-format.json": "24d20e58a04b97136feb05efdd58bdbe128b0edfbe2d1846bbd84694d5988abf",
    "/workspace/wtime/target/performance/P034-before/runs/mode-smoke.json": "3cd0acb7a211f13fdb0a7bb33d5ac524495c27360673a51eea51c73743c85725",
    "/workspace/wtime/target/performance/P034-before/source/Cargo.lock": "9d704c9854b30ca231109a54598c1c440a7d144b8ece5e21539971fead240820",
    "/workspace/wtime/target/performance/P034-before/source/Cargo.toml": "9656851608b4f5ecb5f973a5ad370c67aef45fc557d4c132469bbf012662efff",
    "/workspace/wtime/target/performance/P034-before/source/LICENSE-APACHE": "a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2",
    "/workspace/wtime/target/performance/P034-before/source/LICENSE-MIT": "a43aeeeebe46eda1a6fea77337eb7f9f37341b94afe7251d1f6d14fe03e702c5",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-cli/Cargo.toml": "67dc333182e863f6bbe2a09d9c99e7519ff35a1bc41c551c49273cc1c9f935e8",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-cli/src/main.rs": "760feeee1801d17c5675ba1f35e5fed10b3e2a02eb58458435e7d9ad355e6dfe",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-cli/tests/metadata.rs": "38015f945e99ffca1c9dc18577575057050a91e4df215314d7636f3c9cc7141f",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-decode/Cargo.toml": "1ae571b0eb1a293cd0ef6a181e33d267c40b228cd1afd5dd44ef9a309d113fa1",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-decode/src/instructions.rs": "d352215a69fc6d33b188850974c364023510056c347d0ecf95ce9ba96cd05168",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-decode/src/lib.rs": "9a83e26510e41a1d73556ebc74f4402f0aecb3493068dd7f66d4851d451524d9",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-decode/src/metadata.rs": "e9b8d4d88fc5b34dccd0a1918346cfed3a03dd256129dca4c62f4e0bce8d036e",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-decode/src/module.rs": "51b8fb2669a8323f36c7b199847a002f5b751dc04a382584a7112d7dd7dde71d",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-decode/src/reader.rs": "0c061e8df82b02c1d0fb25ad2744991a2a8f0db7893ae5d5818ba89772651e6e",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/Cargo.toml": "2ee79daf9485c056793d57b1886f75e0e74b12a6ddbf69f4b18f20ecffe8171b",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/examples/embedding.rs": "d9baac3b51bacbee00c42a109831e8bd1a845158b5a0ceae46a911307d8b0907",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/examples/suspension.rs": "097175e5cbbcf1be4716d4c1b584702042558d299360392fa742a85ead331e1d",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/src/arena.rs": "cd929b222d1ccf871f06684510df6767a1e69af4d338458c0e98b88cd16c8441",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/src/canonical.rs": "0bed54a47fbca8a39c7359a9140a5284f0513a6744505ddb5a0b92d531d0776d",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/src/continuation.rs": "c30b6fb4b89999b2e6f494b9258ce9ea3ea1e39a5971643b43147026e0fc8f46",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/src/coverage.rs": "fa735d933bf1334d23c262aac2d35282a743a49aaddb0c2bbc68da4c308f3fbe",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/src/exceptions.rs": "08e306b7f60b25077f31706bf55cc4b521b0b2b713a61e8656e36fcc34c326ba",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/src/execution.rs": "d2ef4ca6928c9c9f9ca7c02f9a94d0a4b8bdfd5b560ed1d3d6dab93fcb3f751d",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/src/execution/accounting.rs": "c86761a09e3c98301f54c971aa93e08da97140832c0f58e279aca4a75bef7127",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/src/execution/cursor.rs": "ff93a3f0056d109366d2b77ea93d459f456d048c05b4bf05369305a3b491d3d3",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/src/execution/effects.rs": "640bf290be1691465e44f188213b644030dab6f8ca1599790f5138816e192bcd",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/src/execution/plan.rs": "68e43e6484a6c198529b333a6e5122526845544d660191121755c3108e125d7e",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/src/gc.rs": "7dcfcc9aaff6a333041d9d8a7323d1ce0fad90b382b5023e9caeeb52815b3471",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/src/gc_exec.rs": "56cf9173994d267fa4ead0af137b8f9f941f6b86a52d458f31be3ce7a32280fc",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/src/lib.rs": "997a0ec8d835c83567e13aa4c755ed30ad0e44a0bdb9e689c14753abdd47bb45",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/src/linker.rs": "39a2068993bcb65fb4434f42d02b56687e8ae31914840487ee4c35de07677806",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/src/numeric.rs": "5acd2cc6a4e9e9c23fd4bb25ab8a9fbc1fa1658d63f4a95e207a00963650dc92",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/src/perf.rs": "df87993399e014674e63c8f15ea9dfb74888fe86ba5bd39a9129a4048121b3e5",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/src/reference.rs": "0fc6c3d773332bc39bde1e627e6c683ccfc36d525cf1a6b410c8fd69fdbac287",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/src/shared_memory.rs": "6b1a5a02e49d2d73ee04865af8d6bac68c60305f1afc754ca9b70252750bce44",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/src/simd.rs": "84c223e6a546aa8955ff5214b9add03df4c94d25f3ffcd45b33a808d299334b1",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/src/wide.rs": "e0347c76d5a00a56396bd641ce4beb4f0d6a7500739f6de57283c1cbacc4224a",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/tests/core.rs": "54f6e18cff27f3da96ae0736d7910013883742f0215d35abb551ce324c78af1f",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/tests/exception_root_safety.rs": "91419a4df0834c064ac143e34f92a15224cea597716328a2074b0a351ff51213",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/tests/exceptions.rs": "3cc47afeeb6428154f6d0a30db6362a009c45d292278f5f03ca60c53c3342a93",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/tests/feature_profiles.rs": "dafe483c28f8683332d198935818fff885827dfc5cfd1ad62f27330490ebc14a",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/tests/fixtures/performance/gc/canonical_seed.wasm": "596587cde1c31c6ff46516aea149ee457eb2785004d763e385b43289e578c51c",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/tests/fixtures/performance/gc/canonical_seed.wat": "c09ee55f2db8ddc880899ae8f57f34fdb89c20be8101fb1049e0408da6a605be",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/tests/fixtures/performance/gc/changed_indirect_target.wasm": "c501d42742682b643b9616347e1ed09cd63dc1c715e5cb338e23a3bb41734ae9",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/tests/fixtures/performance/gc/changed_indirect_target.wat": "3faed5bf7864b4f31f7f8fbf4867dce3a1418c47269bbd0989e2573fa67e1928",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/tests/fixtures/performance/gc/equal_heap_fields_256.wasm": "0378a4f1f4bf8599446c7330e991f43cfe8a32f85e57d146845c678c4d0e61d8",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/tests/fixtures/performance/gc/equal_heap_fields_256.wat": "500fcf1b0932bdd04ccfe25a2fe75e9f990f5687bb9f69fd681a22619a8eae14",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/tests/fixtures/performance/gc/nominal_exact_casts.wasm": "6fc8a2cac95a90fe0d1a3d7c1cf1a9f57041df41f6ec8ccae747c1368b9f03e2",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/tests/fixtures/performance/gc/nominal_exact_casts.wat": "278b748822210879d166c0131c10b88ac32b38bf8437c1ada05667e1d8156ada",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/tests/fixtures/performance/gc/packed_gc_types_1024.wasm": "29959efb8bb583a1426fbb05a3f4f88fc4fbf1f95b887c7f3157e9e9800d1d46",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/tests/fixtures/performance/gc/packed_gc_types_1024.wat": "5d192737237cae94eab827c0ad5bc712b7575c6295755f7b4c084a8237c58a2b",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/tests/fixtures/performance/gc/packed_gc_types_16.wasm": "709e64aec13594de819a90d324d75026e3179fb5b78d92d1f49c9427db398c9f",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/tests/fixtures/performance/gc/packed_gc_types_16.wat": "f95da25cedb52055b84f0c92346f19523cc6e613c60d7a6f899f8b5fc2f8e963",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/tests/fixtures/performance/gc/packed_gc_types_256.wasm": "e1194d66f564ba7c37445d91f70d3a47acfad2f1da37bf4f714617896289e471",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/tests/fixtures/performance/gc/packed_gc_types_256.wat": "847d7a5b472a1ef08af9510b80d64468aa9e92da19a72929e21a20a0aff2200c",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/tests/fixtures/performance/gc/typed_function_refs.wasm": "c07a43c30d251223ebd9769ef3742eb32ec5d3ef751d6a9f0e1507ad353b4cb7",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/tests/fixtures/performance/gc/typed_function_refs.wat": "5d62ffda642de10f8ddbfa828512e8bc7d36d3cbcd70ce2e52f5532c030d990b",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/tests/performance.rs": "f25d85650dd53b858cb76cf6e18a176aad7554bf93083353dee9c25d5e121f12",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/tests/performance/p016.rs": "5ff37ef565ffc5c64c743723f1bfdabe85a315e784f1c81384f17b48a71dc21f",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/tests/performance/p017.rs": "4122d30ef0f042af886692d52de26c9a48d016f5f8e2a0bf67c6e9c9b84e9ef7",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/tests/performance/p018.rs": "e06da12e7539dbfc36bdfcc1981e87c8d926b08679f9bd2fce201987d5dafeae",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/tests/performance/p019.rs": "8294310dd4da5f9a7a05bf0bfee0d2be503ec52cdddbca4e21c6a3ee9d3b2715",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/tests/performance/p020.rs": "b6cb36f79e0d9702e7a8be4f81d254fcbd9d683d57c6baefe37f06951f611185",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/tests/performance/p021.rs": "fec19c012536565dbf2ba3e3f80f26b3fcc4e7d560a2218b5335552be2448a94",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/tests/performance/p022.rs": "e688034612986f5ccab2640e56c2ef0cadf4ad661e8b362a85455b29bffacad6",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/tests/performance/p023.rs": "6cbadd3bf64068c0ba819bed3be369c1334fb876075e9f3253c9d612666472f6",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/tests/performance/p024.rs": "bb48a497f6ef46705df70225077b224cbcec21c4d600a5f6fa22bf29bd428b59",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/tests/performance/p025.rs": "1af91ae70dae7ecd1d03abab59b76c5cc34aa3169b7154b2a9fc3034df680c2e",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/tests/performance/p026.rs": "10ecfc3d74d0e881e5f98d46f0ac33daa04085586bd2604f09cfcd5a161f2446",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/tests/performance/p027.rs": "5ad17c20bb89f535590936259526b11f09d9bde2cc17e2e94876d8083b995b71",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/tests/performance/p028.rs": "a20b1701af071f41de5493e8e69587fd0fedb2dcccfec45e9294e0773ed29edb",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/tests/performance/p029.rs": "fe926229024f5ca65b40bf38f3c62a9de0801ee4cdd49d9787565c3366d04871",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/tests/performance/p030.rs": "96ff18f747e5c9deb19359c13d5f34b2261f06474edbf69e6f4d559500cedba2",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/tests/performance/p031.rs": "b4b0f3466af3d54f2b82376dd8191682a1e8c5dfdb24da36424beeaf705286c5",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/tests/performance/p032.rs": "8cda54330226e6b8de7de2fd7565c3d1feeed95563174df6561794ce252d914a",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/tests/performance/p033.rs": "75e7d654b6e9d49b7e5929c756fb81909bf8c429d238bd24e5f29160609670c5",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/tests/performance/p034.rs": "9c8e581c4f1b5848bb4f3f38601675cccd361a91c936896b0db9db6bae8efeae",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-runtime/tests/proposals.rs": "8fffaa52d03e81fc7d88d0970617f1a0c8ba7602fc241ad9c65367133d2a9927",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-types/Cargo.toml": "e5c4d20d5d35fd4a15ce854f96602add82b86a1bbe4599aab69b0abcc7dc9031",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-types/src/error.rs": "a25659ee8aefcf08babfd2e4b027b698176adc100da5336a9a28f2f0664d8a2c",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-types/src/features.rs": "3c465ce7c325c9110d42a6df08ef20dfc90122e8a86a2fdbb85a6063e82b2dc5",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-types/src/indices.rs": "249359a286195771d58ad78daa775c8164de6419100fea38e6f6f172f92e2e40",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-types/src/lib.rs": "9f1ce2f00f37fba42288e5f4c78262234bf6ab453f7061d7b4907bbe4fdeca4d",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-types/src/types.rs": "d5d6a398f344494ada0d065afcab4bdee157f4803351a2ce6752ee3fc0d4e266",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-validate/Cargo.toml": "fed963beb5aa95d1c242f4131509edd95069e1b27c4e69f84f969357a04a0639",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-validate/src/acceptance.rs": "322c98e589f396f77097a7acf1dabc666f72ccbba8936703cb115ed7befd9621",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-validate/src/atomics.rs": "51d71f78adc9f140c79dad601afe64f4bc42fb2cf896a1d753a480f57f70bbc4",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-validate/src/continuations.rs": "185cf7f137c9dbd57062f95f3d36715e92420a44f5579fb09a34b7dd65029969",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-validate/src/exceptions.rs": "5afe91db3e03444019a0181f639f683c8969563ac20376c6485395c79f46c99b",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-validate/src/gc.rs": "e46a4440d3ffbea02c4905ffffa75e9582c3193697a8484ec464fdaa7a34f21f",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-validate/src/lib.rs": "6d6fd6442fdad200c494fc43775dc003ee18fbfa63632c7056e238777a283455",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-validate/src/types.rs": "2d6f8561b6d9d8eeff0bbb76510e9b9a6159d9646d7c01a01fda6f51207ec7a0",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-validate/tests/exceptions.rs": "061d15fe56d6028e54671c5dd5d86ee1069f2ac99183886a3c3441356d4da290",
    "/workspace/wtime/target/performance/P034-before/source/crates/wtime-validate/tests/policy.rs": "4a4c47509bd9da3850b1ab76235d585db2c7aba826f1f89ec9055a2d888774c3",
    "/workspace/wtime/target/performance/P034-before/source/docs/benchmarks/performance/P033-contract.json": "99f9f88ea35fcc87c6339d9d91a179e7b900fb7aba2010faafaf41f8e27bf4d1",
    "/workspace/wtime/target/performance/P034-before/source/docs/benchmarks/performance/P034-contract.json": "1e0038bec040eb356dff993f1f2c1d862f203ce1056fde6338d8ed530b6bc91e",
    "/workspace/wtime/target/performance/P034-before/source/docs/benchmarks/performance/P034-measurement-contract.json": "0471ddb2b3c6a43d5c65686af402c4bb3d132dfac7df8469b97b8a151d97506b",
    "/workspace/wtime/target/performance/P034-before/source/docs/benchmarks/performance/P034-registry-preservation.json": "fa8d2565b2d12a2a004d8d64004397cce6f302d73714c7e89f49578bad5b9f33",
    "/workspace/wtime/target/performance/P034-before/source/rust-toolchain.toml": "10c67ce85265f8fc5b1a52b416af1a553e56ad96ca9843da6579e2854741e084",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/Cargo.lock": "4d1a59eab7ae2ce8c6abcd203afb3a20d0787e6cd536b86a919fd133196c5431",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/Cargo.toml": "389e92491b6caca59c924bd41a46bee79828b41459f2c3712411e3e40bcb5a3b",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/README.md": "4a679dbd985d5534f4cbaf26aaa92dc397852b12a4af0687c73aea085c3608c7",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/analyze.py": "bbdbc3c63d4d55f80e9f61942795f09a5eb3de0489194842bd1175a895f84b36",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/baseline.py": "4670c11583f94ab28589bf6a16b8d50917408cb85599f832b795e3e6767465fc",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/checkpoint.py": "827a1f659ddd524f9e06c37e4fb47ecf3c8bdc1d2c7e454cf195a6e787328b37",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/compare.py": "a33df09fa5672f748ebb810852709c3b9e3b9595fc16622b214efe5eb5a078ef",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/continuation_scaling.py": "2c078358502ed394c29e8f99aac611eed1e31311749a5856683969a7aacd1961",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/continuation_workloads.py": "384f68be5a40308aee52e0b8d8288511f36a0c2347ba7ebb36fa0a7bfdb7b872",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/fixtures/exceptions/caught_throw.wat": "5433ec09235db09ebfd9e02c9ad89afb76732b7b42586c9204d0bbb5c67245f9",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/fixtures/exceptions/cross_call_catch.wat": "d8b1444bb104bc896f56ad1e67fca80a5977c58f52e05805b231c4753d2ea521",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/fixtures/exceptions/host_throw_catch.wat": "52c9f9b5da3990de5a948b6614680fc0099891c226d4b6903ed56560a3d2b087",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/fixtures/exceptions/normal_path.wat": "cd26d75fd8d2f01183fd3c40ad2b0eb5865d433a4efe2826cd86dda709e2d9cb",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/fixtures/exceptions/throw_ref_unwind.wat": "47a2d51cc83f365c3099c82712c57d422968535750eb91f078868c204ebf0f9f",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/fixtures/exceptions/uncaught_throw.wat": "e89d6eca82d1d903fcac6dd8bda281f6ec837f4c268513c416b979528a674fd8",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/fixtures/gc/continuation_root.wat": "05d3278c9be02485919784bae44f7b5fcd55a63856296b0fb1e4a2c854379448",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/fixtures/gc/descriptor_consumer.wat": "f80a41c42745545c0007917baeb5b905f3085a49b507cb8e9f4331fec1607118",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/fixtures/gc/descriptor_graph.wat": "f7bf5ef8fb302e705e232586f720e9d864e5431be67b29590ea9b7f98b9b1a93",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/fixtures/gc/descriptor_provider.wat": "3197bba5170209b278b8a436d09923fd383255d10b51fe0014383f5099dec9d3",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/fixtures/gc/host_reentry_collect.wat": "91d9a3ba857c39ace65f4a9e1e3ff91cdfe96ca0b52ad45539b63a61534ae7cd",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/fixtures/gc/nominal_exact_casts.wasm": "6fc8a2cac95a90fe0d1a3d7c1cf1a9f57041df41f6ec8ccae747c1368b9f03e2",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/fixtures/gc/nominal_exact_casts.wat": "278b748822210879d166c0131c10b88ac32b38bf8437c1ada05667e1d8156ada",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/fixtures/gc/packed_array.wat": "7580ef1c80342937d900b7acac0bde6e4cddf21fc03de15b9da0d7c91dad6a8d",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/fixtures/gc/packed_gc_types_1024.wasm": "29959efb8bb583a1426fbb05a3f4f88fc4fbf1f95b887c7f3157e9e9800d1d46",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/fixtures/gc/packed_gc_types_1024.wat": "5d192737237cae94eab827c0ad5bc712b7575c6295755f7b4c084a8237c58a2b",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/fixtures/gc/packed_gc_types_16.wasm": "709e64aec13594de819a90d324d75026e3179fb5b78d92d1f49c9427db398c9f",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/fixtures/gc/packed_gc_types_16.wat": "f95da25cedb52055b84f0c92346f19523cc6e613c60d7a6f899f8b5fc2f8e963",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/fixtures/gc/packed_gc_types_256.wasm": "e1194d66f564ba7c37445d91f70d3a47acfad2f1da37bf4f714617896289e471",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/fixtures/gc/packed_gc_types_256.wat": "847d7a5b472a1ef08af9510b80d64468aa9e92da19a72929e21a20a0aff2200c",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/fixtures/gc/root_lifecycle.wat": "79f4ac8511123ba14bdde5a31cd1953bae1c69f1dcba9ec5878785e1db447a51",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/fixtures/gc/struct_graph.wat": "9fe9e8f7c7c9cd995d063bf4a54eb8d4aab61eb526f101b8c50126c50ddbc8df",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/fixtures/gc/type_churn_1024.wasm": "09f4dcbb91c1575f2f4ad2115d6198ae7959f048c66028ada960b5b4a2061bfd",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/fixtures/gc/type_churn_1024.wat": "cc0e5ea4534c2464bf1b1b286ece900942882174da20af3d506ddff6f8ef813b",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/fixtures/gc/type_churn_256.wasm": "bd136a0aeae3917c1af98d7a50de8f8b78b7db1ed1e4b3a92ebc5fa3bb172f17",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/fixtures/gc/type_churn_256.wat": "f5431b25768d26b0022060c7c5d8fb58049c55393d3f68e1b9b3657db527c5b2",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/fixtures/gc/type_churn_64.wasm": "37663a4ba8344cc5d09120a231c4e71699d1a9038e9a9788a5b5bfc03fe2b930",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/fixtures/gc/type_churn_64.wat": "6688be1ceb64af3446c2027fa03639e136fec8d27646915888755f13c4fd1de0",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/fixtures/gc/type_repeat_128.wasm": "a85c15e59addfdd67d5d246a309fd5bbfe958b744a73863baec0500791d13084",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/fixtures/gc/type_repeat_128.wat": "bb8a419be00efc018b7715550878ec1aed52c20140acacf2b1aa840ca8ad94c0",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/fixtures/gc/typed_function_refs.wasm": "c07a43c30d251223ebd9769ef3742eb32ec5d3ef751d6a9f0e1507ad353b4cb7",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/fixtures/gc/typed_function_refs.wat": "5d62ffda642de10f8ddbfa828512e8bc7d36d3cbcd70ce2e52f5532c030d990b",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/fixtures/scalar/accumulator.wat": "61aaec7a86d481378f31f3e27b0c1413684065841fa5909297181fcad9bef10d",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/fixtures/scalar/br_table.wat": "637b96cd8926ffadf728af4949b43fa6b62fffb61968c14c716e0c203a1e5e6d",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/fixtures/scalar/branch_entropy.wat": "6b4861d8bfbbd7e2718e577ec2d479bfff32e94dc47a3b7dda9a1b363baa98b6",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/fixtures/scalar/call_ref.wat": "0323154b79000eeeb0a63310541ec91b6c4d91e32a6a3406125a30f2c8e53a12",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/fixtures/scalar/dependent.wat": "0dd5e9778eb09bac9ee432e5fece3f432d034ea9878d2302972142501f0c60c6",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/fixtures/scalar/direct_calls.wat": "0a075601a2a651f9cb95e8ee0404c64bbf076d17bf242b9c88fecbb8cb973826",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/fixtures/scalar/dynamic_api.wat": "f02cc93a4a43691ce5be8782fa2df454fc2468a7935117ea18104a7dce22cb2d",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/fixtures/scalar/host_call.wat": "4c4026de620eff0b1e9cb4ed4e0440e6045d921267e3962715f6430ac68f5120",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/fixtures/scalar/indirect_calls.wat": "29024ae23844664a9fea4a809d1ce414de40f243f7e678ce5ea6d811843f7797",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/fixtures/scalar/multivalue_branch.wat": "a1f4958100af82889208bb28090d6af7c3e56cbeeccabad4106601c9a9c17014",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/fixtures/scalar/ordinary_recursion.wat": "aca6a66637fa36b72f3c528dd5e3e44039b573f363b5b4fb77437e9278623903",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/fixtures/scalar/table_controls.wat": "9b61bd9cbca804bf639085769fd766a2cf8a69d516fc7efb9e4edfe4ff6d18c9",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/fixtures/scalar/tail_calls.wat": "9953f19384c51b579921e6f789b13a8b57c86ba86d760b77846e3e33825ece02",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/fixtures/scalar/try_table_normal.wat": "ab92800dd4e4d4053498c317effadedf52568270ab7b476eef74b09638de8964",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/gates.py": "f9a054c4cc6427172544dc0720d09e22496d9f562d0b3d05e27dbbc1a7875d89",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/gc_cold_adapter.py": "4f09ca9a30dff35caefb986660ecbb9266cbaf668d550d4e698d6a4e133f4690",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/gc_compare.py": "fa3188d062f6deb7f30bcfa0f43ad164f344cc2407a84b967c88f4f1c0bf4114",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/gc_layout_workloads.py": "baec9bb0c29c923aece9125ed8507f466ea31f319048dd40cedcddd431c066bf",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/gc_scaling.py": "58a6f5419f016a4f1f35512d8ae713041620449bd39d48a98685b251f5701f7e",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/gc_warm_workloads.py": "7082bd76a060337ecc491c69bf7c177648ac1cbfc8835a7dc7a75c26e00464fc",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/gc_workloads.py": "aedd39f75c5c407690251ccd40b8a00139d9a6b9b41f3a1b9980df90b37f543c",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/memory_scaling.py": "9dfac7c395c4207f43102a84f6f83858b42d2d52894eea9457d5416a76a9bf8b",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/memory_workloads.py": "99eaae89bd1d0a95c67cc8f08c22184b95701d97a14354b93a112246295b743b",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/p027_memory_workloads.py": "2b7bf4e7c9cb6c7674c4f465bdca0c9b498f6a1f8bb4843720d0dcc2aee66336",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/profile.py": "113475824a37c1e8618c046b0b0f87ab4e85b8a49bb3e65a85e0e6b742dacb84",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/representative_scaling.py": "a2fe1dfc4406aaf3f56124cf58423a56b8add89d0874de43968e21efac360b63",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/representative_workloads.py": "7ca9e9c08928d7098a85a228a8dbf2081361b3730010e460cf8ea60afdd1204e",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/run.py": "f793c150ae38031467c91a70cddf66e85b6cd6d9782e227b773b4da6ff564bea",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/scalar_workloads.py": "b564e862913ed3fbd9a54bec0c722962fe999c25024375de69c1ceac03d2682e",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/scaling.py": "0a9415ddef9c3900c0ce9c64e286fc767367d0a4ce3dddaf483d04494ef630a3",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/simd_scaling.py": "825c68bfe2136a3d7937c0873fbdf39b9ed84fdca3364f0d050d57e89b959a8b",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/simd_workloads.py": "c43645c67deb4efc566602800d50913a25d434c3146df5ee686fc4bb7b555f2b",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/src/continuation_driver.rs": "2cc5818b256463ef062ad1f36f8a858e3905ce961ffb8b56bae06b143965119a",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/src/gc_exception_driver.rs": "63c192dd2f0f5a376dba96b66ac3e55c40303512374b02fe96b782e33ace369f",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/src/gc_warm_driver.rs": "9fc4b6ef80c4e4aca86bff36a8fdec32a0d9b4871cf1d757e0d374dbf89bb5b7",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/src/main.rs": "9f9496929d3c60165fa061030336397d1825d9b0545a70aadeaf07208c3768e5",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/src/memory_driver.rs": "a7a9049232a73141d01040dbbf463b3bae2959ad3633001c55501f5359fecb0f",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/src/protocol.rs": "550dd14c99e4715ff16e8ecf8a1520dd8f5d25d32c27748259ccc1e9f434573d",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/src/representative_driver.rs": "26de3df2f0d5b6198448e5cb32e28d4c24086e8c2b9ae23dcfe55663920e40b7",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/src/scalar_driver.rs": "c43a3369b6dfac8b57aaf789dbabd9da8dab4f28751571a683b970dd09225a58",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/src/thread_driver.rs": "96801dbc985a214e335c2be69dedce48a1cd2acf37055547f3d5fc87a21f4ee3",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/src/tier_state_driver.rs": "6215becc3a74f02bbea2dbb8d342afe879bd9b9787d2c3c777fea9b871678525",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/thread_scaling.py": "b8384bab6beedb5fc524ac26a91d9033be48a3c4a0b385834e8d94a83bdbc35d",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/thread_workloads.py": "f50f832f5f0eed010b3277fdcb15cacd0b600298026df1ebac1e00265864f21c",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/tier_check.py": "680766b24821f50b47bfa43c8e1c757020b80257f962dd2ae2c06cdc37de96a5",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/workloads.json": "68625d67b23a57525e6414264bd179ad502d630ca73cd1c294e32c1490ed5f5a",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-bench/workloads.py": "c08faf6b9295f22b3b554819b59d2367c5a436c16308029bccbf85095f425096",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-conformance/Cargo.toml": "37c21479a09390a590f0e0ccba31e62ebfddf4096fa3c16039042af9c223c68e",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-conformance/build.rs": "1e37a8f6ec1af7964228b03abb3e9675c2c5218edf3bb43a94d76243cc893ef1",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-conformance/src/adapter.rs": "b99ae377d141ca6369569cbb1960f00910cc8d0b92fd5007ced206f4ebe0f997",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-conformance/src/bin/wtime-allocation-check.rs": "1a20e70ac081c7792732bc1cffded19935e19d0141bca5ea78a6b6086f4c6034",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-conformance/src/bin/wtime-probe.rs": "039e380652f8ef356d0995c35a7f14b289479db40f59e9f88d6ccfa9ef9df785",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-conformance/src/cli.rs": "ab8466680604fc36dc5e1aafbe06d7fbecd8e1cb1f5ad321099cb3a5094017ae",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-conformance/src/corpus.rs": "b1d2a7947b5904b9bd80c0109bc8b5e0f5f8329ba92a693db3f4409cd93cbc1e",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-conformance/src/main.rs": "ecf37373cd8988e4f45569d49a0a971fc85a5b687c091ae2ec7964ec8f35c169",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-conformance/src/report.rs": "a5bb757ec60513a95f0b9573328a9ee77159a2e061741bb695b52c1fef4348f5",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-conformance/src/script.rs": "8c11af05587c584895b9e945c1d0075181b9fb108ef60ae311416cb2b1971888",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-conformance/src/spectest.rs": "1403166f4be6ce443b47d4a7ba4a44da553c67e280dfa2876b3f2ad1e64e8883",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-conformance/tests/annotations.rs": "5eb54a28e57e7e929bcb2342d65c28e23c2ae112b7c10b36f707fa580f489b08",
    "/workspace/wtime/target/performance/P034-before/source/tools/wtime-conformance/tests/interactions.rs": "fd36710c8cd2ef7f3a2883d3f0ab6cbfecace3ae80adf693095228e4f4641938",
    "/workspace/wtime/target/performance/P034-build.json": "d344e43109634e2daab191c3a6a117e70865e49083c9751fe652161018240eaf",
    "/workspace/wtime/target/performance/P034-candidate-checks/candidate_build_budget.log": "0837b3bae9e20ed9c429724de5e68414d783ba7a53210e7114463ab07948d32f",
    "/workspace/wtime/target/performance/P034-candidate-checks/checks-executed.py": "0a15771226a367106c266b12f27fb9c6e4e7667e9119874f83632df1654e4693",
    "/workspace/wtime/target/performance/P034-candidate-checks/commands.json": "d5e60f684ee11d20ba3c746ceef7878f0c1d6af08cf6b45ec2ba06c7c32f0cd1",
    "/workspace/wtime/target/performance/P034-candidate-checks/diagnostic_performance.log": "a693995be068de6891d1f113dfc5751e49aa1b555028afe929bbe332e2acc78e",
    "/workspace/wtime/target/performance/P034-candidate-checks/focused_default.log": "3e1e94634f97c104a692e452d79acbad79e1022ba764a323582c95205d17aac9",
    "/workspace/wtime/target/performance/P034-candidate-checks/focused_diagnostic.log": "4daaa03073e2ea96fb7a76d49c9c9826708e3898c144e79de0742134338508fd",
    "/workspace/wtime/target/performance/P034-candidate-checks/full_default.log": "dbbbc73fca8697135a68225e1289c204032e0f3041bbcde1270eed449761e155",
    "/workspace/wtime/target/performance/P034-candidate-checks/release_benchmark.log": "bf88a0238b816823b5aa6e1907a9345d2b10d12ac625f2a9f9679609b74357b6",
    "/workspace/wtime/target/performance/P034-candidate-focused-corrected.log": "a17c887174a14def0d66e716b030980c20c8be08aaabdf4d3fe494cc42ac6783",
    "/workspace/wtime/target/performance/P034-candidate-focused-initial.log": "ff70a1746a9afe0ac6913eb3be9aed137f9a12e6352eb2eeb038dbef3a93be03",
    "/workspace/wtime/target/performance/P034-candidate/binary/wtime-bench": "dca7cd8bc5a71887b004ada68c6367773bebaa28a6146a3474e77d813698c7ac",
    "/workspace/wtime/target/performance/P034-candidate/retention.json": "168bf0879de43243c643e58b7e895b9af5644d01d3916e7704663ecae5d6d76b",
    "/workspace/wtime/target/performance/P034-candidate/source-manifest.json": "cb4845aa17d9c33ce0ebfe1d929d4e59c3e0c4bf5e10b24f67964b9cdf2ca674",
    "/workspace/wtime/target/performance/P034-candidate/source/Cargo.lock": "9d704c9854b30ca231109a54598c1c440a7d144b8ece5e21539971fead240820",
    "/workspace/wtime/target/performance/P034-candidate/source/Cargo.toml": "9656851608b4f5ecb5f973a5ad370c67aef45fc557d4c132469bbf012662efff",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-cli/Cargo.toml": "67dc333182e863f6bbe2a09d9c99e7519ff35a1bc41c551c49273cc1c9f935e8",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-cli/src/main.rs": "760feeee1801d17c5675ba1f35e5fed10b3e2a02eb58458435e7d9ad355e6dfe",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-cli/tests/metadata.rs": "38015f945e99ffca1c9dc18577575057050a91e4df215314d7636f3c9cc7141f",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-decode/Cargo.toml": "1ae571b0eb1a293cd0ef6a181e33d267c40b228cd1afd5dd44ef9a309d113fa1",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-decode/src/instructions.rs": "d352215a69fc6d33b188850974c364023510056c347d0ecf95ce9ba96cd05168",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-decode/src/lib.rs": "9a83e26510e41a1d73556ebc74f4402f0aecb3493068dd7f66d4851d451524d9",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-decode/src/metadata.rs": "e9b8d4d88fc5b34dccd0a1918346cfed3a03dd256129dca4c62f4e0bce8d036e",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-decode/src/module.rs": "51b8fb2669a8323f36c7b199847a002f5b751dc04a382584a7112d7dd7dde71d",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-decode/src/reader.rs": "0c061e8df82b02c1d0fb25ad2744991a2a8f0db7893ae5d5818ba89772651e6e",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/Cargo.toml": "2ee79daf9485c056793d57b1886f75e0e74b12a6ddbf69f4b18f20ecffe8171b",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/examples/embedding.rs": "d9baac3b51bacbee00c42a109831e8bd1a845158b5a0ceae46a911307d8b0907",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/examples/suspension.rs": "097175e5cbbcf1be4716d4c1b584702042558d299360392fa742a85ead331e1d",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/src/arena.rs": "cd929b222d1ccf871f06684510df6767a1e69af4d338458c0e98b88cd16c8441",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/src/canonical.rs": "0bed54a47fbca8a39c7359a9140a5284f0513a6744505ddb5a0b92d531d0776d",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/src/continuation.rs": "c30b6fb4b89999b2e6f494b9258ce9ea3ea1e39a5971643b43147026e0fc8f46",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/src/coverage.rs": "fa735d933bf1334d23c262aac2d35282a743a49aaddb0c2bbc68da4c308f3fbe",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/src/exceptions.rs": "08e306b7f60b25077f31706bf55cc4b521b0b2b713a61e8656e36fcc34c326ba",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/src/execution.rs": "d2ef4ca6928c9c9f9ca7c02f9a94d0a4b8bdfd5b560ed1d3d6dab93fcb3f751d",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/src/execution/accounting.rs": "c86761a09e3c98301f54c971aa93e08da97140832c0f58e279aca4a75bef7127",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/src/execution/cursor.rs": "ff93a3f0056d109366d2b77ea93d459f456d048c05b4bf05369305a3b491d3d3",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/src/execution/effects.rs": "640bf290be1691465e44f188213b644030dab6f8ca1599790f5138816e192bcd",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/src/execution/plan.rs": "68e43e6484a6c198529b333a6e5122526845544d660191121755c3108e125d7e",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/src/gc.rs": "7dcfcc9aaff6a333041d9d8a7323d1ce0fad90b382b5023e9caeeb52815b3471",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/src/gc_exec.rs": "8f79c5f70dc3b8bb42b592f7f1ace1559a3b87dd6f29f941e02c5601f1b47441",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/src/gc_layout.rs": "1c7215c50ba69be54daf4a37371b7a5585b1c25b119b161af6cbaaf094ba023a",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/src/lib.rs": "cede5e2fb7e862b4c5e648fd6a59023a590958bc2f21dcc727b3e80681103825",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/src/linker.rs": "39a2068993bcb65fb4434f42d02b56687e8ae31914840487ee4c35de07677806",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/src/numeric.rs": "5acd2cc6a4e9e9c23fd4bb25ab8a9fbc1fa1658d63f4a95e207a00963650dc92",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/src/perf.rs": "df87993399e014674e63c8f15ea9dfb74888fe86ba5bd39a9129a4048121b3e5",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/src/reference.rs": "0fc6c3d773332bc39bde1e627e6c683ccfc36d525cf1a6b410c8fd69fdbac287",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/src/shared_memory.rs": "6b1a5a02e49d2d73ee04865af8d6bac68c60305f1afc754ca9b70252750bce44",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/src/simd.rs": "84c223e6a546aa8955ff5214b9add03df4c94d25f3ffcd45b33a808d299334b1",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/src/wide.rs": "e0347c76d5a00a56396bd641ce4beb4f0d6a7500739f6de57283c1cbacc4224a",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/tests/core.rs": "54f6e18cff27f3da96ae0736d7910013883742f0215d35abb551ce324c78af1f",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/tests/exception_root_safety.rs": "91419a4df0834c064ac143e34f92a15224cea597716328a2074b0a351ff51213",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/tests/exceptions.rs": "3cc47afeeb6428154f6d0a30db6362a009c45d292278f5f03ca60c53c3342a93",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/tests/feature_profiles.rs": "dafe483c28f8683332d198935818fff885827dfc5cfd1ad62f27330490ebc14a",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/tests/fixtures/performance/gc/canonical_seed.wasm": "596587cde1c31c6ff46516aea149ee457eb2785004d763e385b43289e578c51c",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/tests/fixtures/performance/gc/canonical_seed.wat": "c09ee55f2db8ddc880899ae8f57f34fdb89c20be8101fb1049e0408da6a605be",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/tests/fixtures/performance/gc/changed_indirect_target.wasm": "c501d42742682b643b9616347e1ed09cd63dc1c715e5cb338e23a3bb41734ae9",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/tests/fixtures/performance/gc/changed_indirect_target.wat": "3faed5bf7864b4f31f7f8fbf4867dce3a1418c47269bbd0989e2573fa67e1928",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/tests/fixtures/performance/gc/equal_heap_fields_256.wasm": "0378a4f1f4bf8599446c7330e991f43cfe8a32f85e57d146845c678c4d0e61d8",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/tests/fixtures/performance/gc/equal_heap_fields_256.wat": "500fcf1b0932bdd04ccfe25a2fe75e9f990f5687bb9f69fd681a22619a8eae14",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/tests/fixtures/performance/gc/nominal_exact_casts.wasm": "6fc8a2cac95a90fe0d1a3d7c1cf1a9f57041df41f6ec8ccae747c1368b9f03e2",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/tests/fixtures/performance/gc/nominal_exact_casts.wat": "278b748822210879d166c0131c10b88ac32b38bf8437c1ada05667e1d8156ada",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/tests/fixtures/performance/gc/packed_gc_types_1024.wasm": "29959efb8bb583a1426fbb05a3f4f88fc4fbf1f95b887c7f3157e9e9800d1d46",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/tests/fixtures/performance/gc/packed_gc_types_1024.wat": "5d192737237cae94eab827c0ad5bc712b7575c6295755f7b4c084a8237c58a2b",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/tests/fixtures/performance/gc/packed_gc_types_16.wasm": "709e64aec13594de819a90d324d75026e3179fb5b78d92d1f49c9427db398c9f",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/tests/fixtures/performance/gc/packed_gc_types_16.wat": "f95da25cedb52055b84f0c92346f19523cc6e613c60d7a6f899f8b5fc2f8e963",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/tests/fixtures/performance/gc/packed_gc_types_256.wasm": "e1194d66f564ba7c37445d91f70d3a47acfad2f1da37bf4f714617896289e471",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/tests/fixtures/performance/gc/packed_gc_types_256.wat": "847d7a5b472a1ef08af9510b80d64468aa9e92da19a72929e21a20a0aff2200c",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/tests/fixtures/performance/gc/typed_function_refs.wasm": "c07a43c30d251223ebd9769ef3742eb32ec5d3ef751d6a9f0e1507ad353b4cb7",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/tests/fixtures/performance/gc/typed_function_refs.wat": "5d62ffda642de10f8ddbfa828512e8bc7d36d3cbcd70ce2e52f5532c030d990b",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/tests/performance.rs": "f25d85650dd53b858cb76cf6e18a176aad7554bf93083353dee9c25d5e121f12",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/tests/performance/p016.rs": "5ff37ef565ffc5c64c743723f1bfdabe85a315e784f1c81384f17b48a71dc21f",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/tests/performance/p017.rs": "4122d30ef0f042af886692d52de26c9a48d016f5f8e2a0bf67c6e9c9b84e9ef7",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/tests/performance/p018.rs": "e06da12e7539dbfc36bdfcc1981e87c8d926b08679f9bd2fce201987d5dafeae",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/tests/performance/p019.rs": "8294310dd4da5f9a7a05bf0bfee0d2be503ec52cdddbca4e21c6a3ee9d3b2715",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/tests/performance/p020.rs": "b6cb36f79e0d9702e7a8be4f81d254fcbd9d683d57c6baefe37f06951f611185",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/tests/performance/p021.rs": "fec19c012536565dbf2ba3e3f80f26b3fcc4e7d560a2218b5335552be2448a94",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/tests/performance/p022.rs": "e688034612986f5ccab2640e56c2ef0cadf4ad661e8b362a85455b29bffacad6",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/tests/performance/p023.rs": "6cbadd3bf64068c0ba819bed3be369c1334fb876075e9f3253c9d612666472f6",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/tests/performance/p024.rs": "bb48a497f6ef46705df70225077b224cbcec21c4d600a5f6fa22bf29bd428b59",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/tests/performance/p025.rs": "1af91ae70dae7ecd1d03abab59b76c5cc34aa3169b7154b2a9fc3034df680c2e",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/tests/performance/p026.rs": "10ecfc3d74d0e881e5f98d46f0ac33daa04085586bd2604f09cfcd5a161f2446",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/tests/performance/p027.rs": "5ad17c20bb89f535590936259526b11f09d9bde2cc17e2e94876d8083b995b71",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/tests/performance/p028.rs": "a20b1701af071f41de5493e8e69587fd0fedb2dcccfec45e9294e0773ed29edb",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/tests/performance/p029.rs": "fe926229024f5ca65b40bf38f3c62a9de0801ee4cdd49d9787565c3366d04871",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/tests/performance/p030.rs": "96ff18f747e5c9deb19359c13d5f34b2261f06474edbf69e6f4d559500cedba2",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/tests/performance/p031.rs": "b4b0f3466af3d54f2b82376dd8191682a1e8c5dfdb24da36424beeaf705286c5",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/tests/performance/p032.rs": "8cda54330226e6b8de7de2fd7565c3d1feeed95563174df6561794ce252d914a",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/tests/performance/p033.rs": "75e7d654b6e9d49b7e5929c756fb81909bf8c429d238bd24e5f29160609670c5",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/tests/performance/p034.rs": "9c8e581c4f1b5848bb4f3f38601675cccd361a91c936896b0db9db6bae8efeae",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-runtime/tests/proposals.rs": "8fffaa52d03e81fc7d88d0970617f1a0c8ba7602fc241ad9c65367133d2a9927",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-types/Cargo.toml": "e5c4d20d5d35fd4a15ce854f96602add82b86a1bbe4599aab69b0abcc7dc9031",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-types/src/error.rs": "a25659ee8aefcf08babfd2e4b027b698176adc100da5336a9a28f2f0664d8a2c",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-types/src/features.rs": "3c465ce7c325c9110d42a6df08ef20dfc90122e8a86a2fdbb85a6063e82b2dc5",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-types/src/indices.rs": "249359a286195771d58ad78daa775c8164de6419100fea38e6f6f172f92e2e40",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-types/src/lib.rs": "9f1ce2f00f37fba42288e5f4c78262234bf6ab453f7061d7b4907bbe4fdeca4d",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-types/src/types.rs": "d5d6a398f344494ada0d065afcab4bdee157f4803351a2ce6752ee3fc0d4e266",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-validate/Cargo.toml": "fed963beb5aa95d1c242f4131509edd95069e1b27c4e69f84f969357a04a0639",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-validate/src/acceptance.rs": "322c98e589f396f77097a7acf1dabc666f72ccbba8936703cb115ed7befd9621",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-validate/src/atomics.rs": "51d71f78adc9f140c79dad601afe64f4bc42fb2cf896a1d753a480f57f70bbc4",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-validate/src/continuations.rs": "185cf7f137c9dbd57062f95f3d36715e92420a44f5579fb09a34b7dd65029969",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-validate/src/exceptions.rs": "5afe91db3e03444019a0181f639f683c8969563ac20376c6485395c79f46c99b",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-validate/src/gc.rs": "e46a4440d3ffbea02c4905ffffa75e9582c3193697a8484ec464fdaa7a34f21f",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-validate/src/lib.rs": "6d6fd6442fdad200c494fc43775dc003ee18fbfa63632c7056e238777a283455",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-validate/src/types.rs": "2d6f8561b6d9d8eeff0bbb76510e9b9a6159d9646d7c01a01fda6f51207ec7a0",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-validate/tests/exceptions.rs": "061d15fe56d6028e54671c5dd5d86ee1069f2ac99183886a3c3441356d4da290",
    "/workspace/wtime/target/performance/P034-candidate/source/crates/wtime-validate/tests/policy.rs": "4a4c47509bd9da3850b1ab76235d585db2c7aba826f1f89ec9055a2d888774c3",
    "/workspace/wtime/target/performance/P034-candidate/source/docs/benchmarks/performance/P033-contract.json": "99f9f88ea35fcc87c6339d9d91a179e7b900fb7aba2010faafaf41f8e27bf4d1",
    "/workspace/wtime/target/performance/P034-candidate/source/docs/benchmarks/performance/P034-contract.json": "1e0038bec040eb356dff993f1f2c1d862f203ce1056fde6338d8ed530b6bc91e",
    "/workspace/wtime/target/performance/P034-candidate/source/docs/benchmarks/performance/P034-measurement-contract.json": "0471ddb2b3c6a43d5c65686af402c4bb3d132dfac7df8469b97b8a151d97506b",
    "/workspace/wtime/target/performance/P034-candidate/source/docs/benchmarks/performance/P034-registry-preservation.json": "fa8d2565b2d12a2a004d8d64004397cce6f302d73714c7e89f49578bad5b9f33",
    "/workspace/wtime/target/performance/P034-candidate/source/rust-toolchain.toml": "10c67ce85265f8fc5b1a52b416af1a553e56ad96ca9843da6579e2854741e084",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/Cargo.lock": "4d1a59eab7ae2ce8c6abcd203afb3a20d0787e6cd536b86a919fd133196c5431",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/Cargo.toml": "389e92491b6caca59c924bd41a46bee79828b41459f2c3712411e3e40bcb5a3b",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/analyze.py": "bbdbc3c63d4d55f80e9f61942795f09a5eb3de0489194842bd1175a895f84b36",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/baseline.py": "4670c11583f94ab28589bf6a16b8d50917408cb85599f832b795e3e6767465fc",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/checkpoint.py": "827a1f659ddd524f9e06c37e4fb47ecf3c8bdc1d2c7e454cf195a6e787328b37",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/compare.py": "a33df09fa5672f748ebb810852709c3b9e3b9595fc16622b214efe5eb5a078ef",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/continuation_scaling.py": "2c078358502ed394c29e8f99aac611eed1e31311749a5856683969a7aacd1961",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/continuation_workloads.py": "384f68be5a40308aee52e0b8d8288511f36a0c2347ba7ebb36fa0a7bfdb7b872",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/fixtures/exceptions/caught_throw.wat": "5433ec09235db09ebfd9e02c9ad89afb76732b7b42586c9204d0bbb5c67245f9",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/fixtures/exceptions/cross_call_catch.wat": "d8b1444bb104bc896f56ad1e67fca80a5977c58f52e05805b231c4753d2ea521",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/fixtures/exceptions/host_throw_catch.wat": "52c9f9b5da3990de5a948b6614680fc0099891c226d4b6903ed56560a3d2b087",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/fixtures/exceptions/normal_path.wat": "cd26d75fd8d2f01183fd3c40ad2b0eb5865d433a4efe2826cd86dda709e2d9cb",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/fixtures/exceptions/throw_ref_unwind.wat": "47a2d51cc83f365c3099c82712c57d422968535750eb91f078868c204ebf0f9f",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/fixtures/exceptions/uncaught_throw.wat": "e89d6eca82d1d903fcac6dd8bda281f6ec837f4c268513c416b979528a674fd8",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/fixtures/gc/continuation_root.wat": "05d3278c9be02485919784bae44f7b5fcd55a63856296b0fb1e4a2c854379448",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/fixtures/gc/descriptor_consumer.wat": "f80a41c42745545c0007917baeb5b905f3085a49b507cb8e9f4331fec1607118",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/fixtures/gc/descriptor_graph.wat": "f7bf5ef8fb302e705e232586f720e9d864e5431be67b29590ea9b7f98b9b1a93",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/fixtures/gc/descriptor_provider.wat": "3197bba5170209b278b8a436d09923fd383255d10b51fe0014383f5099dec9d3",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/fixtures/gc/host_reentry_collect.wat": "91d9a3ba857c39ace65f4a9e1e3ff91cdfe96ca0b52ad45539b63a61534ae7cd",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/fixtures/gc/nominal_exact_casts.wasm": "6fc8a2cac95a90fe0d1a3d7c1cf1a9f57041df41f6ec8ccae747c1368b9f03e2",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/fixtures/gc/nominal_exact_casts.wat": "278b748822210879d166c0131c10b88ac32b38bf8437c1ada05667e1d8156ada",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/fixtures/gc/packed_array.wat": "7580ef1c80342937d900b7acac0bde6e4cddf21fc03de15b9da0d7c91dad6a8d",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/fixtures/gc/packed_gc_types_1024.wasm": "29959efb8bb583a1426fbb05a3f4f88fc4fbf1f95b887c7f3157e9e9800d1d46",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/fixtures/gc/packed_gc_types_1024.wat": "5d192737237cae94eab827c0ad5bc712b7575c6295755f7b4c084a8237c58a2b",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/fixtures/gc/packed_gc_types_16.wasm": "709e64aec13594de819a90d324d75026e3179fb5b78d92d1f49c9427db398c9f",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/fixtures/gc/packed_gc_types_16.wat": "f95da25cedb52055b84f0c92346f19523cc6e613c60d7a6f899f8b5fc2f8e963",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/fixtures/gc/packed_gc_types_256.wasm": "e1194d66f564ba7c37445d91f70d3a47acfad2f1da37bf4f714617896289e471",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/fixtures/gc/packed_gc_types_256.wat": "847d7a5b472a1ef08af9510b80d64468aa9e92da19a72929e21a20a0aff2200c",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/fixtures/gc/root_lifecycle.wat": "79f4ac8511123ba14bdde5a31cd1953bae1c69f1dcba9ec5878785e1db447a51",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/fixtures/gc/struct_graph.wat": "9fe9e8f7c7c9cd995d063bf4a54eb8d4aab61eb526f101b8c50126c50ddbc8df",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/fixtures/gc/type_churn_1024.wasm": "09f4dcbb91c1575f2f4ad2115d6198ae7959f048c66028ada960b5b4a2061bfd",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/fixtures/gc/type_churn_1024.wat": "cc0e5ea4534c2464bf1b1b286ece900942882174da20af3d506ddff6f8ef813b",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/fixtures/gc/type_churn_256.wasm": "bd136a0aeae3917c1af98d7a50de8f8b78b7db1ed1e4b3a92ebc5fa3bb172f17",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/fixtures/gc/type_churn_256.wat": "f5431b25768d26b0022060c7c5d8fb58049c55393d3f68e1b9b3657db527c5b2",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/fixtures/gc/type_churn_64.wasm": "37663a4ba8344cc5d09120a231c4e71699d1a9038e9a9788a5b5bfc03fe2b930",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/fixtures/gc/type_churn_64.wat": "6688be1ceb64af3446c2027fa03639e136fec8d27646915888755f13c4fd1de0",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/fixtures/gc/type_repeat_128.wasm": "a85c15e59addfdd67d5d246a309fd5bbfe958b744a73863baec0500791d13084",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/fixtures/gc/type_repeat_128.wat": "bb8a419be00efc018b7715550878ec1aed52c20140acacf2b1aa840ca8ad94c0",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/fixtures/gc/typed_function_refs.wasm": "c07a43c30d251223ebd9769ef3742eb32ec5d3ef751d6a9f0e1507ad353b4cb7",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/fixtures/gc/typed_function_refs.wat": "5d62ffda642de10f8ddbfa828512e8bc7d36d3cbcd70ce2e52f5532c030d990b",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/fixtures/scalar/accumulator.wat": "61aaec7a86d481378f31f3e27b0c1413684065841fa5909297181fcad9bef10d",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/fixtures/scalar/br_table.wat": "637b96cd8926ffadf728af4949b43fa6b62fffb61968c14c716e0c203a1e5e6d",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/fixtures/scalar/branch_entropy.wat": "6b4861d8bfbbd7e2718e577ec2d479bfff32e94dc47a3b7dda9a1b363baa98b6",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/fixtures/scalar/call_ref.wat": "0323154b79000eeeb0a63310541ec91b6c4d91e32a6a3406125a30f2c8e53a12",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/fixtures/scalar/dependent.wat": "0dd5e9778eb09bac9ee432e5fece3f432d034ea9878d2302972142501f0c60c6",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/fixtures/scalar/direct_calls.wat": "0a075601a2a651f9cb95e8ee0404c64bbf076d17bf242b9c88fecbb8cb973826",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/fixtures/scalar/dynamic_api.wat": "f02cc93a4a43691ce5be8782fa2df454fc2468a7935117ea18104a7dce22cb2d",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/fixtures/scalar/host_call.wat": "4c4026de620eff0b1e9cb4ed4e0440e6045d921267e3962715f6430ac68f5120",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/fixtures/scalar/indirect_calls.wat": "29024ae23844664a9fea4a809d1ce414de40f243f7e678ce5ea6d811843f7797",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/fixtures/scalar/multivalue_branch.wat": "a1f4958100af82889208bb28090d6af7c3e56cbeeccabad4106601c9a9c17014",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/fixtures/scalar/ordinary_recursion.wat": "aca6a66637fa36b72f3c528dd5e3e44039b573f363b5b4fb77437e9278623903",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/fixtures/scalar/table_controls.wat": "9b61bd9cbca804bf639085769fd766a2cf8a69d516fc7efb9e4edfe4ff6d18c9",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/fixtures/scalar/tail_calls.wat": "9953f19384c51b579921e6f789b13a8b57c86ba86d760b77846e3e33825ece02",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/fixtures/scalar/try_table_normal.wat": "ab92800dd4e4d4053498c317effadedf52568270ab7b476eef74b09638de8964",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/gates.py": "f9a054c4cc6427172544dc0720d09e22496d9f562d0b3d05e27dbbc1a7875d89",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/gc_cold_adapter.py": "4f09ca9a30dff35caefb986660ecbb9266cbaf668d550d4e698d6a4e133f4690",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/gc_compare.py": "fa3188d062f6deb7f30bcfa0f43ad164f344cc2407a84b967c88f4f1c0bf4114",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/gc_layout_workloads.py": "baec9bb0c29c923aece9125ed8507f466ea31f319048dd40cedcddd431c066bf",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/gc_scaling.py": "58a6f5419f016a4f1f35512d8ae713041620449bd39d48a98685b251f5701f7e",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/gc_warm_workloads.py": "7082bd76a060337ecc491c69bf7c177648ac1cbfc8835a7dc7a75c26e00464fc",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/gc_workloads.py": "aedd39f75c5c407690251ccd40b8a00139d9a6b9b41f3a1b9980df90b37f543c",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/memory_scaling.py": "9dfac7c395c4207f43102a84f6f83858b42d2d52894eea9457d5416a76a9bf8b",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/memory_workloads.py": "99eaae89bd1d0a95c67cc8f08c22184b95701d97a14354b93a112246295b743b",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/p027_memory_workloads.py": "2b7bf4e7c9cb6c7674c4f465bdca0c9b498f6a1f8bb4843720d0dcc2aee66336",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/profile.py": "113475824a37c1e8618c046b0b0f87ab4e85b8a49bb3e65a85e0e6b742dacb84",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/representative_scaling.py": "a2fe1dfc4406aaf3f56124cf58423a56b8add89d0874de43968e21efac360b63",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/representative_workloads.py": "7ca9e9c08928d7098a85a228a8dbf2081361b3730010e460cf8ea60afdd1204e",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/run.py": "f793c150ae38031467c91a70cddf66e85b6cd6d9782e227b773b4da6ff564bea",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/scalar_workloads.py": "b564e862913ed3fbd9a54bec0c722962fe999c25024375de69c1ceac03d2682e",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/scaling.py": "0a9415ddef9c3900c0ce9c64e286fc767367d0a4ce3dddaf483d04494ef630a3",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/simd_scaling.py": "825c68bfe2136a3d7937c0873fbdf39b9ed84fdca3364f0d050d57e89b959a8b",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/simd_workloads.py": "c43645c67deb4efc566602800d50913a25d434c3146df5ee686fc4bb7b555f2b",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/src/continuation_driver.rs": "2cc5818b256463ef062ad1f36f8a858e3905ce961ffb8b56bae06b143965119a",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/src/gc_exception_driver.rs": "63c192dd2f0f5a376dba96b66ac3e55c40303512374b02fe96b782e33ace369f",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/src/gc_warm_driver.rs": "9fc4b6ef80c4e4aca86bff36a8fdec32a0d9b4871cf1d757e0d374dbf89bb5b7",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/src/main.rs": "9f9496929d3c60165fa061030336397d1825d9b0545a70aadeaf07208c3768e5",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/src/memory_driver.rs": "a7a9049232a73141d01040dbbf463b3bae2959ad3633001c55501f5359fecb0f",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/src/protocol.rs": "550dd14c99e4715ff16e8ecf8a1520dd8f5d25d32c27748259ccc1e9f434573d",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/src/representative_driver.rs": "26de3df2f0d5b6198448e5cb32e28d4c24086e8c2b9ae23dcfe55663920e40b7",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/src/scalar_driver.rs": "c43a3369b6dfac8b57aaf789dbabd9da8dab4f28751571a683b970dd09225a58",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/src/thread_driver.rs": "96801dbc985a214e335c2be69dedce48a1cd2acf37055547f3d5fc87a21f4ee3",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/src/tier_state_driver.rs": "6215becc3a74f02bbea2dbb8d342afe879bd9b9787d2c3c777fea9b871678525",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/thread_scaling.py": "b8384bab6beedb5fc524ac26a91d9033be48a3c4a0b385834e8d94a83bdbc35d",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/thread_workloads.py": "f50f832f5f0eed010b3277fdcb15cacd0b600298026df1ebac1e00265864f21c",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/tier_check.py": "680766b24821f50b47bfa43c8e1c757020b80257f962dd2ae2c06cdc37de96a5",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/workloads.json": "68625d67b23a57525e6414264bd179ad502d630ca73cd1c294e32c1490ed5f5a",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-bench/workloads.py": "c08faf6b9295f22b3b554819b59d2367c5a436c16308029bccbf85095f425096",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-conformance/Cargo.toml": "37c21479a09390a590f0e0ccba31e62ebfddf4096fa3c16039042af9c223c68e",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-conformance/build.rs": "1e37a8f6ec1af7964228b03abb3e9675c2c5218edf3bb43a94d76243cc893ef1",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-conformance/src/adapter.rs": "b99ae377d141ca6369569cbb1960f00910cc8d0b92fd5007ced206f4ebe0f997",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-conformance/src/bin/wtime-allocation-check.rs": "1a20e70ac081c7792732bc1cffded19935e19d0141bca5ea78a6b6086f4c6034",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-conformance/src/bin/wtime-probe.rs": "039e380652f8ef356d0995c35a7f14b289479db40f59e9f88d6ccfa9ef9df785",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-conformance/src/cli.rs": "ab8466680604fc36dc5e1aafbe06d7fbecd8e1cb1f5ad321099cb3a5094017ae",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-conformance/src/corpus.rs": "b1d2a7947b5904b9bd80c0109bc8b5e0f5f8329ba92a693db3f4409cd93cbc1e",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-conformance/src/main.rs": "ecf37373cd8988e4f45569d49a0a971fc85a5b687c091ae2ec7964ec8f35c169",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-conformance/src/report.rs": "a5bb757ec60513a95f0b9573328a9ee77159a2e061741bb695b52c1fef4348f5",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-conformance/src/script.rs": "8c11af05587c584895b9e945c1d0075181b9fb108ef60ae311416cb2b1971888",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-conformance/src/spectest.rs": "1403166f4be6ce443b47d4a7ba4a44da553c67e280dfa2876b3f2ad1e64e8883",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-conformance/tests/annotations.rs": "5eb54a28e57e7e929bcb2342d65c28e23c2ae112b7c10b36f707fa580f489b08",
    "/workspace/wtime/target/performance/P034-candidate/source/tools/wtime-conformance/tests/interactions.rs": "fd36710c8cd2ef7f3a2883d3f0ab6cbfecace3ae80adf693095228e4f4641938",
    "/workspace/wtime/target/performance/P034-closure/record.json": "95322bd6842d540ce18b7e5dff1eb306c9f22bbbaf5135abc6c4e186cd68c6ff",
    "/workspace/wtime/target/performance/P034-closure/restore-reviewed-source.py": "57f705b64a44de7a9919b64c6dfc52207b0f06dc1c67dd934ef539ff4bc2f09e",
    "/workspace/wtime/target/performance/P034-closure/restore.log": "02884c3c2776bb23ae539805a07c7be8edaca1cb5ddbc6915628497f6a939fb0",
    "/workspace/wtime/target/performance/P034-comparison-run.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P034-incremental-build-decision.json": "881185a85e6162f841c356c1f1c5d6f0e9ff485d176bd320f8aafbb5c5d8f556",
    "/workspace/wtime/target/performance/P034-measure-command-record.json": "6de56d0cf72deb219741680b261769e70ed615397674e6369aab5eb42ec3abb7",
    "/workspace/wtime/target/performance/P034-measure.executed.py": "159901ce6ff2502a7b4da60fcf981ab244cb97843c1c3632c423dde3cb3e3b85",
    "/workspace/wtime/target/performance/P034-restored-build.json": "7cfa2ccd7004e9324a0e8ff2bdb0d24609ca7bfb34b98a84efb1f7dfa6bcbe33",
    "/workspace/wtime/target/performance/P034-runtime-install/candidate-before-boundary-test-fix.patch": "56a1da0551ae8cb087dce61c3b64179625caf4ce9ba1527e3baf42efa100a252",
    "/workspace/wtime/target/performance/P034-runtime-install/focused-boundary-test-failure.json": "572e1ba7021fb5b9819ce59678760162420e17fa4da8a1b075ecc1d36cfde1bc",
    "/workspace/wtime/target/performance/P034-runtime-install/gc_layout.initial-focused.rs": "b63067c873bf8fa4013349215a2681815695facd0566bf2bf9cc33c3abdc35d0",
    "/workspace/wtime/target/performance/P034-strict-build-executed.py": "7019c54abf367eab51d7921d0c92250351ccd25f2e98309db68df1f3d49e95fa",
    "/workspace/wtime/target/performance/P034/analysis.json": "f52fd901ddb53c7369c94b99e7570872e5d9b8d45a7605bd022d64b801811f49",
    "/workspace/wtime/target/performance/P034/analysis.log": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "/workspace/wtime/target/performance/P034/checkpoint.jsonl": "61d7a04fef2e501cbf0d49f55633dc460825c617660c339a2de652496475c673",
    "/workspace/wtime/target/performance/P034/commands.json": "951a4faa8bb0ab8cc6d1a6f0cb5317b8f68a148d0d92544dd0f16688b6367e12",
    "/workspace/wtime/target/performance/P034/decision.json": "9c95798c98fa5cbedc75a88d55a672f4f85dbfdddeeb5188c3e8662600b0c825",
    "/workspace/wtime/target/performance/P034/extract_executed.py": "093ba949e179f1f48786d886ae24cafcb04ecd6ec8c8eb10b954f4e08691b499",
    "/workspace/wtime/target/performance/P034/extraction.json": "51f03bb23e299cee2b0fe18bbf1b46c86862cbf7220b102d9774b25e3df44a4b",
    "/workspace/wtime/target/performance/P034/extraction.log": "47572555d7be31b421fd7b92df255a46e92100b9adaf38645f119ed33c4fdb7b",
    "/workspace/wtime/target/performance/P034/preliminary-decision.json": "0336c6de9b1bb174c4d8fc2aabc985e137720bbf0268d7baf34f114663acc776",
    "/workspace/wtime/target/performance/P034/preliminary-executed.py": "aa486603f9e9744606c6ec49ea0cf5a4cbd6119a47b141f7089ad3ef28cf7ce6",
    "/workspace/wtime/target/performance/P034/publish-evidence.executed.py": "5a8bc8b93378a455b4990068d69549718ea38e11546b0ba8563f8f23b3a05e5f",
    "/workspace/wtime/target/performance/P034/raw-gate.log": "9c95798c98fa5cbedc75a88d55a672f4f85dbfdddeeb5188c3e8662600b0c825",
    "/workspace/wtime/target/performance/P034/report.json": "3a86006a8fb50fa36236555e7b3b38d70dc7c68a09ab7cd562ef1fd0dafda62a",
    "/workspace/wtime/target/performance/P034/restore-commands.json": "bad7ffdafe4862a5aad3b20f3ba3993d1dd1147f03af6e95e28e50800d2d994a",
    "/workspace/wtime/target/performance/P034/restore-helper.snapshot.py": "aacc747fb5406139511489de5518a63fcd110a952ba2757b1e9009ba49dc330a",
    "/workspace/wtime/target/performance/P034/restore-initial-guard-failure.json": "b978d72cf72398fba30d604b19852bc096c6b67c3776b5870fe708d0ecb0582a",
    "/workspace/wtime/target/performance/P034/restore-input-identities.json": "2c51dbfc7bd4458f0a71dd0b1a29643988fa5577f329037f030ba88db75f339a",
    "/workspace/wtime/target/performance/P034/restored-benchmark-build.log": "83b2d5c6211e2a3690e08e3bf0365a644937c437da6169c78fed377261ebb394",
    "/workspace/wtime/target/performance/P034/restored-build-budget.log": "d8a8d3c779701cc600f2d422c5a58f353f0bb24f57d6740047c7307aac748a0e",
    "/workspace/wtime/target/performance/P034/restored-default-tests.log": "2ddb58d5ea9d9155a53e065d4501567903cf5465170302c72c061def08d5f8de",
    "/workspace/wtime/target/performance/P034/restored-diagnostic-tests.log": "e1f65c1685fc87a2cced6f7417131897b8ff95ecbab61b7951037b77a252b83a",
    "/workspace/wtime/target/performance/P034/restored-focused-default-tests.log": "8a551cc10c8cfefe76c3b10ec89911b71d4fbf484145d653c3bf2a6cf65a358b",
    "/workspace/wtime/target/performance/P034/restored-focused-diagnostic-tests.log": "e0fee3511a7fdae2daae006049764448304fdc5705a0893088e7d1940748687f",
    "/workspace/wtime/target/performance/P034/rollback-identities.json": "c8f2db2cb4b562c2b795dfa2c6d3a6cb7c32b76978be9c86199c136201c842ff",
    "/workspace/wtime/target/performance/baseline/binary/wtime-bench": "735e595f3a625da89401fdee298781b752fbe071e3d3d793df196c9e68b8e498",
    "/workspace/wtime/target/performance/baseline/binary/wtime-conformance": "36bede285fc5a1ad57b35566abf4bb68d3acf4e015944ff07e8dc6fb76e3bc14",
    "/workspace/wtime/target/performance/baseline/fixtures/branch_calls.wasm": "9b58d26f201900b5b6cce684133a98ab68b3cf55d979b64ce895e914e9d5327b",
    "/workspace/wtime/target/performance/baseline/fixtures/branch_calls.wat": "d0af4fdf9ad5be38344267ae72fc1855edb5a8fa1b475fd0af4cb507730b2309",
    "/workspace/wtime/target/performance/baseline/fixtures/call_overhead.wasm": "0815ca7d6026206d0773fe10c6b9643862bfffe8ba361c9789369ef9668ee746",
    "/workspace/wtime/target/performance/baseline/fixtures/call_overhead.wat": "9493b8f751690a2fd6e8b19dcee5e2c1d50b717253d292ce34d5692b306663a5",
    "/workspace/wtime/target/performance/baseline/fixtures/integer_loop.wasm": "560744f64244064c43a3ebe2b2d5f39fa326308e723ee12bc15aae81e22cf04c",
    "/workspace/wtime/target/performance/baseline/fixtures/integer_loop.wat": "791730112109f3d5751a7b3e2c1134e7e0e6f71a7f7ee01ea8481eb989ee3fa4",
    "/workspace/wtime/target/performance/baseline/fixtures/memory_fill_scan.wasm": "1c933d404fe9feb93bb664af3bbb60a50319cdd6a2c29d442aa9ab7799f5e65f",
    "/workspace/wtime/target/performance/baseline/fixtures/memory_fill_scan.wat": "c0a222245af932b6d43ca964fa710f56476dc90ebaca0b20915d35b242a6c87d",
    "/workspace/wtime/target/performance/baseline/fixtures/prepare_large.wasm": "c489fe06970f8e1c60e4992774aaadb571f676833d6579de1f67a5facf3fe9b0",
    "/workspace/wtime/target/performance/baseline/fixtures/prepare_large.wat": "aefc7237fa17bfbccd8c318bc104111c8e3296933a94017837d2c02130d638fd",
    "/workspace/wtime/target/performance/baseline/historical_reports/wasmtime-comparison-run2.json": "6818fa194027b57b183ca3a42a55627c676ba857392497fa325345e0f3fd090b",
    "/workspace/wtime/target/performance/baseline/historical_reports/wasmtime-comparison.json": "b1b69f3944b488a037c08305a43bd8de474837958c53217f1c3cb94b7d04354c",
    "/workspace/wtime/target/performance/baseline/manifest.json": "7349596fc806c96c74cb9c40f1c08402589c1683660b0b152c713264f5808472",
    "/workspace/wtime/target/performance/baseline/runs/historical-format.json": "8756385d77bd958f27431e50929f010a9a4fd4b373db7784e45d95ddea2662ed",
    "/workspace/wtime/target/performance/baseline/source/Cargo.lock": "9d704c9854b30ca231109a54598c1c440a7d144b8ece5e21539971fead240820",
    "/workspace/wtime/target/performance/baseline/source/Cargo.toml": "9656851608b4f5ecb5f973a5ad370c67aef45fc557d4c132469bbf012662efff",
    "/workspace/wtime/target/performance/baseline/source/LICENSE-APACHE": "a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2",
    "/workspace/wtime/target/performance/baseline/source/LICENSE-MIT": "a43aeeeebe46eda1a6fea77337eb7f9f37341b94afe7251d1f6d14fe03e702c5",
    "/workspace/wtime/target/performance/baseline/source/crates/wtime-cli/Cargo.toml": "67dc333182e863f6bbe2a09d9c99e7519ff35a1bc41c551c49273cc1c9f935e8",
    "/workspace/wtime/target/performance/baseline/source/crates/wtime-cli/src/main.rs": "760feeee1801d17c5675ba1f35e5fed10b3e2a02eb58458435e7d9ad355e6dfe",
    "/workspace/wtime/target/performance/baseline/source/crates/wtime-cli/tests/metadata.rs": "38015f945e99ffca1c9dc18577575057050a91e4df215314d7636f3c9cc7141f",
    "/workspace/wtime/target/performance/baseline/source/crates/wtime-decode/Cargo.toml": "1ae571b0eb1a293cd0ef6a181e33d267c40b228cd1afd5dd44ef9a309d113fa1",
    "/workspace/wtime/target/performance/baseline/source/crates/wtime-decode/src/instructions.rs": "d352215a69fc6d33b188850974c364023510056c347d0ecf95ce9ba96cd05168",
    "/workspace/wtime/target/performance/baseline/source/crates/wtime-decode/src/lib.rs": "9a83e26510e41a1d73556ebc74f4402f0aecb3493068dd7f66d4851d451524d9",
    "/workspace/wtime/target/performance/baseline/source/crates/wtime-decode/src/metadata.rs": "e9b8d4d88fc5b34dccd0a1918346cfed3a03dd256129dca4c62f4e0bce8d036e",
    "/workspace/wtime/target/performance/baseline/source/crates/wtime-decode/src/module.rs": "51b8fb2669a8323f36c7b199847a002f5b751dc04a382584a7112d7dd7dde71d",
    "/workspace/wtime/target/performance/baseline/source/crates/wtime-decode/src/reader.rs": "0c061e8df82b02c1d0fb25ad2744991a2a8f0db7893ae5d5818ba89772651e6e",
    "/workspace/wtime/target/performance/baseline/source/crates/wtime-runtime/Cargo.toml": "da3a83cae36bbab34d9d7838eb83b646f30b6a943709d35e0391713ddb7be14e",
    "/workspace/wtime/target/performance/baseline/source/crates/wtime-runtime/examples/embedding.rs": "d9baac3b51bacbee00c42a109831e8bd1a845158b5a0ceae46a911307d8b0907",
    "/workspace/wtime/target/performance/baseline/source/crates/wtime-runtime/examples/suspension.rs": "097175e5cbbcf1be4716d4c1b584702042558d299360392fa742a85ead331e1d",
    "/workspace/wtime/target/performance/baseline/source/crates/wtime-runtime/src/arena.rs": "cd929b222d1ccf871f06684510df6767a1e69af4d338458c0e98b88cd16c8441",
    "/workspace/wtime/target/performance/baseline/source/crates/wtime-runtime/src/canonical.rs": "0bed54a47fbca8a39c7359a9140a5284f0513a6744505ddb5a0b92d531d0776d",
    "/workspace/wtime/target/performance/baseline/source/crates/wtime-runtime/src/continuation.rs": "5d9d3b8dc1b79776817f3a0a0d9c951a2a5b1cfd82ac97b7b7999e763bd44211",
    "/workspace/wtime/target/performance/baseline/source/crates/wtime-runtime/src/coverage.rs": "fa735d933bf1334d23c262aac2d35282a743a49aaddb0c2bbc68da4c308f3fbe",
    "/workspace/wtime/target/performance/baseline/source/crates/wtime-runtime/src/exceptions.rs": "11d964449cfc6b193de8085eb04d4c064bfe6a35b405eeeb150fe1eb9903dbe1",
    "/workspace/wtime/target/performance/baseline/source/crates/wtime-runtime/src/execution.rs": "de12b5611cdd4735d479a50e762b7ff167a597a9ad6ca7200b2f55da360a6db0",
    "/workspace/wtime/target/performance/baseline/source/crates/wtime-runtime/src/gc.rs": "7dcfcc9aaff6a333041d9d8a7323d1ce0fad90b382b5023e9caeeb52815b3471",
    "/workspace/wtime/target/performance/baseline/source/crates/wtime-runtime/src/gc_exec.rs": "e95c94b089847067f5cd60494f7473122116854b2f7dc8c7782623c4e21a2639",
    "/workspace/wtime/target/performance/baseline/source/crates/wtime-runtime/src/lib.rs": "2df3f5229fe6abc733ebed62467d95a763b30a771e117a9393a4fc9e1b2336f6",
    "/workspace/wtime/target/performance/baseline/source/crates/wtime-runtime/src/linker.rs": "8aa0cd1a1b69e4df44d7eff844e11c252ab21c43bf9e8e0471acc4bf7a50e18a",
    "/workspace/wtime/target/performance/baseline/source/crates/wtime-runtime/src/numeric.rs": "5acd2cc6a4e9e9c23fd4bb25ab8a9fbc1fa1658d63f4a95e207a00963650dc92",
    "/workspace/wtime/target/performance/baseline/source/crates/wtime-runtime/src/shared_memory.rs": "6b1a5a02e49d2d73ee04865af8d6bac68c60305f1afc754ca9b70252750bce44",
    "/workspace/wtime/target/performance/baseline/source/crates/wtime-runtime/src/simd.rs": "84c223e6a546aa8955ff5214b9add03df4c94d25f3ffcd45b33a808d299334b1",
    "/workspace/wtime/target/performance/baseline/source/crates/wtime-runtime/src/wide.rs": "e0347c76d5a00a56396bd641ce4beb4f0d6a7500739f6de57283c1cbacc4224a",
    "/workspace/wtime/target/performance/baseline/source/crates/wtime-runtime/tests/core.rs": "54f6e18cff27f3da96ae0736d7910013883742f0215d35abb551ce324c78af1f",
    "/workspace/wtime/target/performance/baseline/source/crates/wtime-runtime/tests/exceptions.rs": "3cc47afeeb6428154f6d0a30db6362a009c45d292278f5f03ca60c53c3342a93",
    "/workspace/wtime/target/performance/baseline/source/crates/wtime-runtime/tests/feature_profiles.rs": "dafe483c28f8683332d198935818fff885827dfc5cfd1ad62f27330490ebc14a",
    "/workspace/wtime/target/performance/baseline/source/crates/wtime-runtime/tests/proposals.rs": "8fffaa52d03e81fc7d88d0970617f1a0c8ba7602fc241ad9c65367133d2a9927",
    "/workspace/wtime/target/performance/baseline/source/crates/wtime-types/Cargo.toml": "e5c4d20d5d35fd4a15ce854f96602add82b86a1bbe4599aab69b0abcc7dc9031",
    "/workspace/wtime/target/performance/baseline/source/crates/wtime-types/src/error.rs": "a25659ee8aefcf08babfd2e4b027b698176adc100da5336a9a28f2f0664d8a2c",
    "/workspace/wtime/target/performance/baseline/source/crates/wtime-types/src/features.rs": "3c465ce7c325c9110d42a6df08ef20dfc90122e8a86a2fdbb85a6063e82b2dc5",
    "/workspace/wtime/target/performance/baseline/source/crates/wtime-types/src/indices.rs": "249359a286195771d58ad78daa775c8164de6419100fea38e6f6f172f92e2e40",
    "/workspace/wtime/target/performance/baseline/source/crates/wtime-types/src/lib.rs": "9f1ce2f00f37fba42288e5f4c78262234bf6ab453f7061d7b4907bbe4fdeca4d",
    "/workspace/wtime/target/performance/baseline/source/crates/wtime-types/src/types.rs": "d5d6a398f344494ada0d065afcab4bdee157f4803351a2ce6752ee3fc0d4e266",
    "/workspace/wtime/target/performance/baseline/source/crates/wtime-validate/Cargo.toml": "fed963beb5aa95d1c242f4131509edd95069e1b27c4e69f84f969357a04a0639",
    "/workspace/wtime/target/performance/baseline/source/crates/wtime-validate/src/acceptance.rs": "322c98e589f396f77097a7acf1dabc666f72ccbba8936703cb115ed7befd9621",
    "/workspace/wtime/target/performance/baseline/source/crates/wtime-validate/src/atomics.rs": "51d71f78adc9f140c79dad601afe64f4bc42fb2cf896a1d753a480f57f70bbc4",
    "/workspace/wtime/target/performance/baseline/source/crates/wtime-validate/src/continuations.rs": "185cf7f137c9dbd57062f95f3d36715e92420a44f5579fb09a34b7dd65029969",
    "/workspace/wtime/target/performance/baseline/source/crates/wtime-validate/src/exceptions.rs": "5afe91db3e03444019a0181f639f683c8969563ac20376c6485395c79f46c99b",
    "/workspace/wtime/target/performance/baseline/source/crates/wtime-validate/src/gc.rs": "e46a4440d3ffbea02c4905ffffa75e9582c3193697a8484ec464fdaa7a34f21f",
    "/workspace/wtime/target/performance/baseline/source/crates/wtime-validate/src/lib.rs": "666fc61b81e1719a5ef50c52a656801b560e4f8377047008e94e18767116df88",
    "/workspace/wtime/target/performance/baseline/source/crates/wtime-validate/src/types.rs": "2d6f8561b6d9d8eeff0bbb76510e9b9a6159d9646d7c01a01fda6f51207ec7a0",
    "/workspace/wtime/target/performance/baseline/source/crates/wtime-validate/tests/exceptions.rs": "061d15fe56d6028e54671c5dd5d86ee1069f2ac99183886a3c3441356d4da290",
    "/workspace/wtime/target/performance/baseline/source/crates/wtime-validate/tests/policy.rs": "4a4c47509bd9da3850b1ab76235d585db2c7aba826f1f89ec9055a2d888774c3",
    "/workspace/wtime/target/performance/baseline/source/rust-toolchain.toml": "10c67ce85265f8fc5b1a52b416af1a553e56ad96ca9843da6579e2854741e084",
    "/workspace/wtime/target/performance/baseline/source/tools/wtime-bench/Cargo.lock": "4d1a59eab7ae2ce8c6abcd203afb3a20d0787e6cd536b86a919fd133196c5431",
    "/workspace/wtime/target/performance/baseline/source/tools/wtime-bench/Cargo.toml": "69a395a329b740b54d7e62a13470645a3f323238a9af59926fb945a1da231792",
    "/workspace/wtime/target/performance/baseline/source/tools/wtime-bench/README.md": "4a679dbd985d5534f4cbaf26aaa92dc397852b12a4af0687c73aea085c3608c7",
    "/workspace/wtime/target/performance/baseline/source/tools/wtime-bench/baseline.py": "56b6c15433809b3670482b94901c8d24a85980c190294e3501b9bb075091e533",
    "/workspace/wtime/target/performance/baseline/source/tools/wtime-bench/run.py": "f793c150ae38031467c91a70cddf66e85b6cd6d9782e227b773b4da6ff564bea",
    "/workspace/wtime/target/performance/baseline/source/tools/wtime-bench/src/main.rs": "07c35bc4a074139c6aa7fb44f1e0b8c6f9de3834ea11870b5a45301ab37e151a",
    "/workspace/wtime/target/performance/baseline/source/tools/wtime-conformance/Cargo.toml": "b5ecbd6c6fa3f2081f1b600433143b2458e1c359c0bda3968cc7d3201361b141",
    "/workspace/wtime/target/performance/baseline/source/tools/wtime-conformance/build.rs": "1e37a8f6ec1af7964228b03abb3e9675c2c5218edf3bb43a94d76243cc893ef1",
    "/workspace/wtime/target/performance/baseline/source/tools/wtime-conformance/src/adapter.rs": "d099a8b490b9b16eac942988ec32b6e59606da4be2faadfde79cd930e381070b",
    "/workspace/wtime/target/performance/baseline/source/tools/wtime-conformance/src/bin/wtime-allocation-check.rs": "1a20e70ac081c7792732bc1cffded19935e19d0141bca5ea78a6b6086f4c6034",
    "/workspace/wtime/target/performance/baseline/source/tools/wtime-conformance/src/bin/wtime-probe.rs": "8581162a05834657c2715c6593069a9d1f9421029c5e09a993e6bb4d02419de5",
    "/workspace/wtime/target/performance/baseline/source/tools/wtime-conformance/src/cli.rs": "fa89e465b879d94a185072e6587a1323b242390a5c5e62b388c147dc679e20e2",
    "/workspace/wtime/target/performance/baseline/source/tools/wtime-conformance/src/corpus.rs": "b1d2a7947b5904b9bd80c0109bc8b5e0f5f8329ba92a693db3f4409cd93cbc1e",
    "/workspace/wtime/target/performance/baseline/source/tools/wtime-conformance/src/main.rs": "ecf37373cd8988e4f45569d49a0a971fc85a5b687c091ae2ec7964ec8f35c169",
    "/workspace/wtime/target/performance/baseline/source/tools/wtime-conformance/src/report.rs": "5621e3ced6bb5b68fcd1151d54d7f1f5c410e328cec2a752e46d2de77b20ba60",
    "/workspace/wtime/target/performance/baseline/source/tools/wtime-conformance/src/script.rs": "8c11af05587c584895b9e945c1d0075181b9fb108ef60ae311416cb2b1971888",
    "/workspace/wtime/target/performance/baseline/source/tools/wtime-conformance/src/spectest.rs": "1403166f4be6ce443b47d4a7ba4a44da553c67e280dfa2876b3f2ad1e64e8883",
    "/workspace/wtime/target/performance/baseline/source/tools/wtime-conformance/tests/annotations.rs": "5eb54a28e57e7e929bcb2342d65c28e23c2ae112b7c10b36f707fa580f489b08",
    "/workspace/wtime/target/performance/baseline/source/tools/wtime-conformance/tests/interactions.rs": "fd36710c8cd2ef7f3a2883d3f0ab6cbfecace3ae80adf693095228e4f4641938"
  },
  "attempt": {
    "directory": "/tmp/P034-completion-attempts/attempt-0001",
    "helper_sha256": "c5648bc5ebbc3f30f84e3b69cc8e40833e746c96bd7eb99139f4053e518de9ed",
    "helper_snapshot": "/tmp/P034-completion-attempts/attempt-0001/P034-complete.snapshot.py",
    "number": 1
  },
  "before_archive": {
    "manifest": {
      "bytes": 59492,
      "path": "/workspace/wtime/target/performance/P034-before/manifest.json",
      "sha256": "616436907ab25bc65d1f21c11e42cc5a1d6e4b9baf3d2abe5c98622ddf18fef2"
    },
    "mode": "decoded",
    "path": "/workspace/wtime/target/performance/P034-before",
    "source_count": 199
  },
  "candidate_checks": {
    "candidate_binary": {
      "bytes": 17298672,
      "path": "/workspace/wtime/target/performance/P034-candidate/binary/wtime-bench",
      "sha256": "dca7cd8bc5a71887b004ada68c6367773bebaa28a6146a3474e77d813698c7ac"
    },
    "candidate_build": {
      "bytes": 112358,
      "path": "/workspace/wtime/target/performance/P034-build.json",
      "sha256": "d344e43109634e2daab191c3a6a117e70865e49083c9751fe652161018240eaf"
    },
    "candidate_gc_layout_sha256": "1c7215c50ba69be54daf4a37371b7a5585b1c25b119b161af6cbaaf094ba023a",
    "candidate_patch": {
      "bytes": 12749,
      "path": "/workspace/wtime/docs/benchmarks/performance/P034-candidate.patch",
      "sha256": "499da996452956246f71df6ba860b3c042e0d47d2e87f7b7f554eef77025be76"
    },
    "commands": {
      "candidate_build_budget": {
        "bytes": 1700,
        "exit_code": 0,
        "name": "candidate_build_budget",
        "path": "/workspace/wtime/target/performance/P034-candidate-checks/candidate_build_budget.log",
        "sha256": "0837b3bae9e20ed9c429724de5e68414d783ba7a53210e7114463ab07948d32f"
      },
      "diagnostic_performance": {
        "bytes": 6500,
        "exit_code": 0,
        "name": "diagnostic_performance",
        "path": "/workspace/wtime/target/performance/P034-candidate-checks/diagnostic_performance.log",
        "sha256": "a693995be068de6891d1f113dfc5751e49aa1b555028afe929bbe332e2acc78e"
      },
      "focused_default": {
        "bytes": 2428,
        "exit_code": 0,
        "name": "focused_default",
        "path": "/workspace/wtime/target/performance/P034-candidate-checks/focused_default.log",
        "sha256": "3e1e94634f97c104a692e452d79acbad79e1022ba764a323582c95205d17aac9"
      },
      "focused_diagnostic": {
        "bytes": 2428,
        "exit_code": 0,
        "name": "focused_diagnostic",
        "path": "/workspace/wtime/target/performance/P034-candidate-checks/focused_diagnostic.log",
        "sha256": "4daaa03073e2ea96fb7a76d49c9c9826708e3898c144e79de0742134338508fd"
      },
      "full_default": {
        "bytes": 16559,
        "exit_code": 0,
        "name": "full_default",
        "path": "/workspace/wtime/target/performance/P034-candidate-checks/full_default.log",
        "sha256": "dbbbc73fca8697135a68225e1289c204032e0f3041bbcde1270eed449761e155"
      },
      "release_benchmark": {
        "bytes": 206,
        "exit_code": 0,
        "name": "release_benchmark",
        "path": "/workspace/wtime/target/performance/P034-candidate-checks/release_benchmark.log",
        "sha256": "bf88a0238b816823b5aa6e1907a9345d2b10d12ac625f2a9f9679609b74357b6"
      }
    },
    "focused_retained_bytes": {
      "1024": 8240,
      "16": 176,
      "256": 2096
    },
    "full_source_count": 200,
    "full_sources": {
      "Cargo.lock": "9d704c9854b30ca231109a54598c1c440a7d144b8ece5e21539971fead240820",
      "Cargo.toml": "9656851608b4f5ecb5f973a5ad370c67aef45fc557d4c132469bbf012662efff",
      "LICENSE-APACHE": "a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2",
      "LICENSE-MIT": "a43aeeeebe46eda1a6fea77337eb7f9f37341b94afe7251d1f6d14fe03e702c5",
      "crates/wtime-cli/Cargo.toml": "67dc333182e863f6bbe2a09d9c99e7519ff35a1bc41c551c49273cc1c9f935e8",
      "crates/wtime-cli/src/main.rs": "760feeee1801d17c5675ba1f35e5fed10b3e2a02eb58458435e7d9ad355e6dfe",
      "crates/wtime-cli/tests/metadata.rs": "38015f945e99ffca1c9dc18577575057050a91e4df215314d7636f3c9cc7141f",
      "crates/wtime-decode/Cargo.toml": "1ae571b0eb1a293cd0ef6a181e33d267c40b228cd1afd5dd44ef9a309d113fa1",
      "crates/wtime-decode/src/instructions.rs": "d352215a69fc6d33b188850974c364023510056c347d0ecf95ce9ba96cd05168",
      "crates/wtime-decode/src/lib.rs": "9a83e26510e41a1d73556ebc74f4402f0aecb3493068dd7f66d4851d451524d9",
      "crates/wtime-decode/src/metadata.rs": "e9b8d4d88fc5b34dccd0a1918346cfed3a03dd256129dca4c62f4e0bce8d036e",
      "crates/wtime-decode/src/module.rs": "51b8fb2669a8323f36c7b199847a002f5b751dc04a382584a7112d7dd7dde71d",
      "crates/wtime-decode/src/reader.rs": "0c061e8df82b02c1d0fb25ad2744991a2a8f0db7893ae5d5818ba89772651e6e",
      "crates/wtime-runtime/Cargo.toml": "2ee79daf9485c056793d57b1886f75e0e74b12a6ddbf69f4b18f20ecffe8171b",
      "crates/wtime-runtime/examples/embedding.rs": "d9baac3b51bacbee00c42a109831e8bd1a845158b5a0ceae46a911307d8b0907",
      "crates/wtime-runtime/examples/suspension.rs": "097175e5cbbcf1be4716d4c1b584702042558d299360392fa742a85ead331e1d",
      "crates/wtime-runtime/src/arena.rs": "cd929b222d1ccf871f06684510df6767a1e69af4d338458c0e98b88cd16c8441",
      "crates/wtime-runtime/src/canonical.rs": "0bed54a47fbca8a39c7359a9140a5284f0513a6744505ddb5a0b92d531d0776d",
      "crates/wtime-runtime/src/continuation.rs": "c30b6fb4b89999b2e6f494b9258ce9ea3ea1e39a5971643b43147026e0fc8f46",
      "crates/wtime-runtime/src/coverage.rs": "fa735d933bf1334d23c262aac2d35282a743a49aaddb0c2bbc68da4c308f3fbe",
      "crates/wtime-runtime/src/exceptions.rs": "08e306b7f60b25077f31706bf55cc4b521b0b2b713a61e8656e36fcc34c326ba",
      "crates/wtime-runtime/src/execution.rs": "d2ef4ca6928c9c9f9ca7c02f9a94d0a4b8bdfd5b560ed1d3d6dab93fcb3f751d",
      "crates/wtime-runtime/src/execution/accounting.rs": "c86761a09e3c98301f54c971aa93e08da97140832c0f58e279aca4a75bef7127",
      "crates/wtime-runtime/src/execution/cursor.rs": "ff93a3f0056d109366d2b77ea93d459f456d048c05b4bf05369305a3b491d3d3",
      "crates/wtime-runtime/src/execution/effects.rs": "640bf290be1691465e44f188213b644030dab6f8ca1599790f5138816e192bcd",
      "crates/wtime-runtime/src/execution/plan.rs": "68e43e6484a6c198529b333a6e5122526845544d660191121755c3108e125d7e",
      "crates/wtime-runtime/src/gc.rs": "7dcfcc9aaff6a333041d9d8a7323d1ce0fad90b382b5023e9caeeb52815b3471",
      "crates/wtime-runtime/src/gc_exec.rs": "8f79c5f70dc3b8bb42b592f7f1ace1559a3b87dd6f29f941e02c5601f1b47441",
      "crates/wtime-runtime/src/gc_layout.rs": "1c7215c50ba69be54daf4a37371b7a5585b1c25b119b161af6cbaaf094ba023a",
      "crates/wtime-runtime/src/lib.rs": "cede5e2fb7e862b4c5e648fd6a59023a590958bc2f21dcc727b3e80681103825",
      "crates/wtime-runtime/src/linker.rs": "39a2068993bcb65fb4434f42d02b56687e8ae31914840487ee4c35de07677806",
      "crates/wtime-runtime/src/numeric.rs": "5acd2cc6a4e9e9c23fd4bb25ab8a9fbc1fa1658d63f4a95e207a00963650dc92",
      "crates/wtime-runtime/src/perf.rs": "df87993399e014674e63c8f15ea9dfb74888fe86ba5bd39a9129a4048121b3e5",
      "crates/wtime-runtime/src/reference.rs": "0fc6c3d773332bc39bde1e627e6c683ccfc36d525cf1a6b410c8fd69fdbac287",
      "crates/wtime-runtime/src/shared_memory.rs": "6b1a5a02e49d2d73ee04865af8d6bac68c60305f1afc754ca9b70252750bce44",
      "crates/wtime-runtime/src/simd.rs": "84c223e6a546aa8955ff5214b9add03df4c94d25f3ffcd45b33a808d299334b1",
      "crates/wtime-runtime/src/wide.rs": "e0347c76d5a00a56396bd641ce4beb4f0d6a7500739f6de57283c1cbacc4224a",
      "crates/wtime-runtime/tests/core.rs": "54f6e18cff27f3da96ae0736d7910013883742f0215d35abb551ce324c78af1f",
      "crates/wtime-runtime/tests/exception_root_safety.rs": "91419a4df0834c064ac143e34f92a15224cea597716328a2074b0a351ff51213",
      "crates/wtime-runtime/tests/exceptions.rs": "3cc47afeeb6428154f6d0a30db6362a009c45d292278f5f03ca60c53c3342a93",
      "crates/wtime-runtime/tests/feature_profiles.rs": "dafe483c28f8683332d198935818fff885827dfc5cfd1ad62f27330490ebc14a",
      "crates/wtime-runtime/tests/fixtures/performance/gc/canonical_seed.wasm": "596587cde1c31c6ff46516aea149ee457eb2785004d763e385b43289e578c51c",
      "crates/wtime-runtime/tests/fixtures/performance/gc/canonical_seed.wat": "c09ee55f2db8ddc880899ae8f57f34fdb89c20be8101fb1049e0408da6a605be",
      "crates/wtime-runtime/tests/fixtures/performance/gc/changed_indirect_target.wasm": "c501d42742682b643b9616347e1ed09cd63dc1c715e5cb338e23a3bb41734ae9",
      "crates/wtime-runtime/tests/fixtures/performance/gc/changed_indirect_target.wat": "3faed5bf7864b4f31f7f8fbf4867dce3a1418c47269bbd0989e2573fa67e1928",
      "crates/wtime-runtime/tests/fixtures/performance/gc/equal_heap_fields_256.wasm": "0378a4f1f4bf8599446c7330e991f43cfe8a32f85e57d146845c678c4d0e61d8",
      "crates/wtime-runtime/tests/fixtures/performance/gc/equal_heap_fields_256.wat": "500fcf1b0932bdd04ccfe25a2fe75e9f990f5687bb9f69fd681a22619a8eae14",
      "crates/wtime-runtime/tests/fixtures/performance/gc/nominal_exact_casts.wasm": "6fc8a2cac95a90fe0d1a3d7c1cf1a9f57041df41f6ec8ccae747c1368b9f03e2",
      "crates/wtime-runtime/tests/fixtures/performance/gc/nominal_exact_casts.wat": "278b748822210879d166c0131c10b88ac32b38bf8437c1ada05667e1d8156ada",
      "crates/wtime-runtime/tests/fixtures/performance/gc/packed_gc_types_1024.wasm": "29959efb8bb583a1426fbb05a3f4f88fc4fbf1f95b887c7f3157e9e9800d1d46",
      "crates/wtime-runtime/tests/fixtures/performance/gc/packed_gc_types_1024.wat": "5d192737237cae94eab827c0ad5bc712b7575c6295755f7b4c084a8237c58a2b",
      "crates/wtime-runtime/tests/fixtures/performance/gc/packed_gc_types_16.wasm": "709e64aec13594de819a90d324d75026e3179fb5b78d92d1f49c9427db398c9f",
      "crates/wtime-runtime/tests/fixtures/performance/gc/packed_gc_types_16.wat": "f95da25cedb52055b84f0c92346f19523cc6e613c60d7a6f899f8b5fc2f8e963",
      "crates/wtime-runtime/tests/fixtures/performance/gc/packed_gc_types_256.wasm": "e1194d66f564ba7c37445d91f70d3a47acfad2f1da37bf4f714617896289e471",
      "crates/wtime-runtime/tests/fixtures/performance/gc/packed_gc_types_256.wat": "847d7a5b472a1ef08af9510b80d64468aa9e92da19a72929e21a20a0aff2200c",
      "crates/wtime-runtime/tests/fixtures/performance/gc/typed_function_refs.wasm": "c07a43c30d251223ebd9769ef3742eb32ec5d3ef751d6a9f0e1507ad353b4cb7",
      "crates/wtime-runtime/tests/fixtures/performance/gc/typed_function_refs.wat": "5d62ffda642de10f8ddbfa828512e8bc7d36d3cbcd70ce2e52f5532c030d990b",
      "crates/wtime-runtime/tests/performance.rs": "f25d85650dd53b858cb76cf6e18a176aad7554bf93083353dee9c25d5e121f12",
      "crates/wtime-runtime/tests/performance/p016.rs": "5ff37ef565ffc5c64c743723f1bfdabe85a315e784f1c81384f17b48a71dc21f",
      "crates/wtime-runtime/tests/performance/p017.rs": "4122d30ef0f042af886692d52de26c9a48d016f5f8e2a0bf67c6e9c9b84e9ef7",
      "crates/wtime-runtime/tests/performance/p018.rs": "e06da12e7539dbfc36bdfcc1981e87c8d926b08679f9bd2fce201987d5dafeae",
      "crates/wtime-runtime/tests/performance/p019.rs": "8294310dd4da5f9a7a05bf0bfee0d2be503ec52cdddbca4e21c6a3ee9d3b2715",
      "crates/wtime-runtime/tests/performance/p020.rs": "b6cb36f79e0d9702e7a8be4f81d254fcbd9d683d57c6baefe37f06951f611185",
      "crates/wtime-runtime/tests/performance/p021.rs": "fec19c012536565dbf2ba3e3f80f26b3fcc4e7d560a2218b5335552be2448a94",
      "crates/wtime-runtime/tests/performance/p022.rs": "e688034612986f5ccab2640e56c2ef0cadf4ad661e8b362a85455b29bffacad6",
      "crates/wtime-runtime/tests/performance/p023.rs": "6cbadd3bf64068c0ba819bed3be369c1334fb876075e9f3253c9d612666472f6",
      "crates/wtime-runtime/tests/performance/p024.rs": "bb48a497f6ef46705df70225077b224cbcec21c4d600a5f6fa22bf29bd428b59",
      "crates/wtime-runtime/tests/performance/p025.rs": "1af91ae70dae7ecd1d03abab59b76c5cc34aa3169b7154b2a9fc3034df680c2e",
      "crates/wtime-runtime/tests/performance/p026.rs": "10ecfc3d74d0e881e5f98d46f0ac33daa04085586bd2604f09cfcd5a161f2446",
      "crates/wtime-runtime/tests/performance/p027.rs": "5ad17c20bb89f535590936259526b11f09d9bde2cc17e2e94876d8083b995b71",
      "crates/wtime-runtime/tests/performance/p028.rs": "a20b1701af071f41de5493e8e69587fd0fedb2dcccfec45e9294e0773ed29edb",
      "crates/wtime-runtime/tests/performance/p029.rs": "fe926229024f5ca65b40bf38f3c62a9de0801ee4cdd49d9787565c3366d04871",
      "crates/wtime-runtime/tests/performance/p030.rs": "96ff18f747e5c9deb19359c13d5f34b2261f06474edbf69e6f4d559500cedba2",
      "crates/wtime-runtime/tests/performance/p031.rs": "b4b0f3466af3d54f2b82376dd8191682a1e8c5dfdb24da36424beeaf705286c5",
      "crates/wtime-runtime/tests/performance/p032.rs": "8cda54330226e6b8de7de2fd7565c3d1feeed95563174df6561794ce252d914a",
      "crates/wtime-runtime/tests/performance/p033.rs": "75e7d654b6e9d49b7e5929c756fb81909bf8c429d238bd24e5f29160609670c5",
      "crates/wtime-runtime/tests/performance/p034.rs": "9c8e581c4f1b5848bb4f3f38601675cccd361a91c936896b0db9db6bae8efeae",
      "crates/wtime-runtime/tests/proposals.rs": "8fffaa52d03e81fc7d88d0970617f1a0c8ba7602fc241ad9c65367133d2a9927",
      "crates/wtime-types/Cargo.toml": "e5c4d20d5d35fd4a15ce854f96602add82b86a1bbe4599aab69b0abcc7dc9031",
      "crates/wtime-types/src/error.rs": "a25659ee8aefcf08babfd2e4b027b698176adc100da5336a9a28f2f0664d8a2c",
      "crates/wtime-types/src/features.rs": "3c465ce7c325c9110d42a6df08ef20dfc90122e8a86a2fdbb85a6063e82b2dc5",
      "crates/wtime-types/src/indices.rs": "249359a286195771d58ad78daa775c8164de6419100fea38e6f6f172f92e2e40",
      "crates/wtime-types/src/lib.rs": "9f1ce2f00f37fba42288e5f4c78262234bf6ab453f7061d7b4907bbe4fdeca4d",
      "crates/wtime-types/src/types.rs": "d5d6a398f344494ada0d065afcab4bdee157f4803351a2ce6752ee3fc0d4e266",
      "crates/wtime-validate/Cargo.toml": "fed963beb5aa95d1c242f4131509edd95069e1b27c4e69f84f969357a04a0639",
      "crates/wtime-validate/src/acceptance.rs": "322c98e589f396f77097a7acf1dabc666f72ccbba8936703cb115ed7befd9621",
      "crates/wtime-validate/src/atomics.rs": "51d71f78adc9f140c79dad601afe64f4bc42fb2cf896a1d753a480f57f70bbc4",
      "crates/wtime-validate/src/continuations.rs": "185cf7f137c9dbd57062f95f3d36715e92420a44f5579fb09a34b7dd65029969",
      "crates/wtime-validate/src/exceptions.rs": "5afe91db3e03444019a0181f639f683c8969563ac20376c6485395c79f46c99b",
      "crates/wtime-validate/src/gc.rs": "e46a4440d3ffbea02c4905ffffa75e9582c3193697a8484ec464fdaa7a34f21f",
      "crates/wtime-validate/src/lib.rs": "6d6fd6442fdad200c494fc43775dc003ee18fbfa63632c7056e238777a283455",
      "crates/wtime-validate/src/types.rs": "2d6f8561b6d9d8eeff0bbb76510e9b9a6159d9646d7c01a01fda6f51207ec7a0",
      "crates/wtime-validate/tests/exceptions.rs": "061d15fe56d6028e54671c5dd5d86ee1069f2ac99183886a3c3441356d4da290",
      "crates/wtime-validate/tests/policy.rs": "4a4c47509bd9da3850b1ab76235d585db2c7aba826f1f89ec9055a2d888774c3",
      "docs/benchmarks/performance/P033-contract.json": "99f9f88ea35fcc87c6339d9d91a179e7b900fb7aba2010faafaf41f8e27bf4d1",
      "docs/benchmarks/performance/P034-contract.json": "1e0038bec040eb356dff993f1f2c1d862f203ce1056fde6338d8ed530b6bc91e",
      "docs/benchmarks/performance/P034-measurement-contract.json": "0471ddb2b3c6a43d5c65686af402c4bb3d132dfac7df8469b97b8a151d97506b",
      "docs/benchmarks/performance/P034-registry-preservation.json": "fa8d2565b2d12a2a004d8d64004397cce6f302d73714c7e89f49578bad5b9f33",
      "rust-toolchain.toml": "10c67ce85265f8fc5b1a52b416af1a553e56ad96ca9843da6579e2854741e084",
      "tools/wtime-bench/Cargo.lock": "4d1a59eab7ae2ce8c6abcd203afb3a20d0787e6cd536b86a919fd133196c5431",
      "tools/wtime-bench/Cargo.toml": "389e92491b6caca59c924bd41a46bee79828b41459f2c3712411e3e40bcb5a3b",
      "tools/wtime-bench/README.md": "4a679dbd985d5534f4cbaf26aaa92dc397852b12a4af0687c73aea085c3608c7",
      "tools/wtime-bench/analyze.py": "bbdbc3c63d4d55f80e9f61942795f09a5eb3de0489194842bd1175a895f84b36",
      "tools/wtime-bench/baseline.py": "4670c11583f94ab28589bf6a16b8d50917408cb85599f832b795e3e6767465fc",
      "tools/wtime-bench/checkpoint.py": "827a1f659ddd524f9e06c37e4fb47ecf3c8bdc1d2c7e454cf195a6e787328b37",
      "tools/wtime-bench/compare.py": "a33df09fa5672f748ebb810852709c3b9e3b9595fc16622b214efe5eb5a078ef",
      "tools/wtime-bench/continuation_scaling.py": "2c078358502ed394c29e8f99aac611eed1e31311749a5856683969a7aacd1961",
      "tools/wtime-bench/continuation_workloads.py": "384f68be5a40308aee52e0b8d8288511f36a0c2347ba7ebb36fa0a7bfdb7b872",
      "tools/wtime-bench/fixtures/exceptions/caught_throw.wat": "5433ec09235db09ebfd9e02c9ad89afb76732b7b42586c9204d0bbb5c67245f9",
      "tools/wtime-bench/fixtures/exceptions/cross_call_catch.wat": "d8b1444bb104bc896f56ad1e67fca80a5977c58f52e05805b231c4753d2ea521",
      "tools/wtime-bench/fixtures/exceptions/host_throw_catch.wat": "52c9f9b5da3990de5a948b6614680fc0099891c226d4b6903ed56560a3d2b087",
      "tools/wtime-bench/fixtures/exceptions/normal_path.wat": "cd26d75fd8d2f01183fd3c40ad2b0eb5865d433a4efe2826cd86dda709e2d9cb",
      "tools/wtime-bench/fixtures/exceptions/throw_ref_unwind.wat": "47a2d51cc83f365c3099c82712c57d422968535750eb91f078868c204ebf0f9f",
      "tools/wtime-bench/fixtures/exceptions/uncaught_throw.wat": "e89d6eca82d1d903fcac6dd8bda281f6ec837f4c268513c416b979528a674fd8",
      "tools/wtime-bench/fixtures/gc/continuation_root.wat": "05d3278c9be02485919784bae44f7b5fcd55a63856296b0fb1e4a2c854379448",
      "tools/wtime-bench/fixtures/gc/descriptor_consumer.wat": "f80a41c42745545c0007917baeb5b905f3085a49b507cb8e9f4331fec1607118",
      "tools/wtime-bench/fixtures/gc/descriptor_graph.wat": "f7bf5ef8fb302e705e232586f720e9d864e5431be67b29590ea9b7f98b9b1a93",
      "tools/wtime-bench/fixtures/gc/descriptor_provider.wat": "3197bba5170209b278b8a436d09923fd383255d10b51fe0014383f5099dec9d3",
      "tools/wtime-bench/fixtures/gc/host_reentry_collect.wat": "91d9a3ba857c39ace65f4a9e1e3ff91cdfe96ca0b52ad45539b63a61534ae7cd",
      "tools/wtime-bench/fixtures/gc/nominal_exact_casts.wasm": "6fc8a2cac95a90fe0d1a3d7c1cf1a9f57041df41f6ec8ccae747c1368b9f03e2",
      "tools/wtime-bench/fixtures/gc/nominal_exact_casts.wat": "278b748822210879d166c0131c10b88ac32b38bf8437c1ada05667e1d8156ada",
      "tools/wtime-bench/fixtures/gc/packed_array.wat": "7580ef1c80342937d900b7acac0bde6e4cddf21fc03de15b9da0d7c91dad6a8d",
      "tools/wtime-bench/fixtures/gc/packed_gc_types_1024.wasm": "29959efb8bb583a1426fbb05a3f4f88fc4fbf1f95b887c7f3157e9e9800d1d46",
      "tools/wtime-bench/fixtures/gc/packed_gc_types_1024.wat": "5d192737237cae94eab827c0ad5bc712b7575c6295755f7b4c084a8237c58a2b",
      "tools/wtime-bench/fixtures/gc/packed_gc_types_16.wasm": "709e64aec13594de819a90d324d75026e3179fb5b78d92d1f49c9427db398c9f",
      "tools/wtime-bench/fixtures/gc/packed_gc_types_16.wat": "f95da25cedb52055b84f0c92346f19523cc6e613c60d7a6f899f8b5fc2f8e963",
      "tools/wtime-bench/fixtures/gc/packed_gc_types_256.wasm": "e1194d66f564ba7c37445d91f70d3a47acfad2f1da37bf4f714617896289e471",
      "tools/wtime-bench/fixtures/gc/packed_gc_types_256.wat": "847d7a5b472a1ef08af9510b80d64468aa9e92da19a72929e21a20a0aff2200c",
      "tools/wtime-bench/fixtures/gc/root_lifecycle.wat": "79f4ac8511123ba14bdde5a31cd1953bae1c69f1dcba9ec5878785e1db447a51",
      "tools/wtime-bench/fixtures/gc/struct_graph.wat": "9fe9e8f7c7c9cd995d063bf4a54eb8d4aab61eb526f101b8c50126c50ddbc8df",
      "tools/wtime-bench/fixtures/gc/type_churn_1024.wasm": "09f4dcbb91c1575f2f4ad2115d6198ae7959f048c66028ada960b5b4a2061bfd",
      "tools/wtime-bench/fixtures/gc/type_churn_1024.wat": "cc0e5ea4534c2464bf1b1b286ece900942882174da20af3d506ddff6f8ef813b",
      "tools/wtime-bench/fixtures/gc/type_churn_256.wasm": "bd136a0aeae3917c1af98d7a50de8f8b78b7db1ed1e4b3a92ebc5fa3bb172f17",
      "tools/wtime-bench/fixtures/gc/type_churn_256.wat": "f5431b25768d26b0022060c7c5d8fb58049c55393d3f68e1b9b3657db527c5b2",
      "tools/wtime-bench/fixtures/gc/type_churn_64.wasm": "37663a4ba8344cc5d09120a231c4e71699d1a9038e9a9788a5b5bfc03fe2b930",
      "tools/wtime-bench/fixtures/gc/type_churn_64.wat": "6688be1ceb64af3446c2027fa03639e136fec8d27646915888755f13c4fd1de0",
      "tools/wtime-bench/fixtures/gc/type_repeat_128.wasm": "a85c15e59addfdd67d5d246a309fd5bbfe958b744a73863baec0500791d13084",
      "tools/wtime-bench/fixtures/gc/type_repeat_128.wat": "bb8a419be00efc018b7715550878ec1aed52c20140acacf2b1aa840ca8ad94c0",
      "tools/wtime-bench/fixtures/gc/typed_function_refs.wasm": "c07a43c30d251223ebd9769ef3742eb32ec5d3ef751d6a9f0e1507ad353b4cb7",
      "tools/wtime-bench/fixtures/gc/typed_function_refs.wat": "5d62ffda642de10f8ddbfa828512e8bc7d36d3cbcd70ce2e52f5532c030d990b",
      "tools/wtime-bench/fixtures/scalar/accumulator.wat": "61aaec7a86d481378f31f3e27b0c1413684065841fa5909297181fcad9bef10d",
      "tools/wtime-bench/fixtures/scalar/br_table.wat": "637b96cd8926ffadf728af4949b43fa6b62fffb61968c14c716e0c203a1e5e6d",
      "tools/wtime-bench/fixtures/scalar/branch_entropy.wat": "6b4861d8bfbbd7e2718e577ec2d479bfff32e94dc47a3b7dda9a1b363baa98b6",
      "tools/wtime-bench/fixtures/scalar/call_ref.wat": "0323154b79000eeeb0a63310541ec91b6c4d91e32a6a3406125a30f2c8e53a12",
      "tools/wtime-bench/fixtures/scalar/dependent.wat": "0dd5e9778eb09bac9ee432e5fece3f432d034ea9878d2302972142501f0c60c6",
      "tools/wtime-bench/fixtures/scalar/direct_calls.wat": "0a075601a2a651f9cb95e8ee0404c64bbf076d17bf242b9c88fecbb8cb973826",
      "tools/wtime-bench/fixtures/scalar/dynamic_api.wat": "f02cc93a4a43691ce5be8782fa2df454fc2468a7935117ea18104a7dce22cb2d",
      "tools/wtime-bench/fixtures/scalar/host_call.wat": "4c4026de620eff0b1e9cb4ed4e0440e6045d921267e3962715f6430ac68f5120",
      "tools/wtime-bench/fixtures/scalar/indirect_calls.wat": "29024ae23844664a9fea4a809d1ce414de40f243f7e678ce5ea6d811843f7797",
      "tools/wtime-bench/fixtures/scalar/multivalue_branch.wat": "a1f4958100af82889208bb28090d6af7c3e56cbeeccabad4106601c9a9c17014",
      "tools/wtime-bench/fixtures/scalar/ordinary_recursion.wat": "aca6a66637fa36b72f3c528dd5e3e44039b573f363b5b4fb77437e9278623903",
      "tools/wtime-bench/fixtures/scalar/table_controls.wat": "9b61bd9cbca804bf639085769fd766a2cf8a69d516fc7efb9e4edfe4ff6d18c9",
      "tools/wtime-bench/fixtures/scalar/tail_calls.wat": "9953f19384c51b579921e6f789b13a8b57c86ba86d760b77846e3e33825ece02",
      "tools/wtime-bench/fixtures/scalar/try_table_normal.wat": "ab92800dd4e4d4053498c317effadedf52568270ab7b476eef74b09638de8964",
      "tools/wtime-bench/gates.py": "f9a054c4cc6427172544dc0720d09e22496d9f562d0b3d05e27dbbc1a7875d89",
      "tools/wtime-bench/gc_cold_adapter.py": "4f09ca9a30dff35caefb986660ecbb9266cbaf668d550d4e698d6a4e133f4690",
      "tools/wtime-bench/gc_compare.py": "fa3188d062f6deb7f30bcfa0f43ad164f344cc2407a84b967c88f4f1c0bf4114",
      "tools/wtime-bench/gc_layout_workloads.py": "baec9bb0c29c923aece9125ed8507f466ea31f319048dd40cedcddd431c066bf",
      "tools/wtime-bench/gc_scaling.py": "58a6f5419f016a4f1f35512d8ae713041620449bd39d48a98685b251f5701f7e",
      "tools/wtime-bench/gc_warm_workloads.py": "7082bd76a060337ecc491c69bf7c177648ac1cbfc8835a7dc7a75c26e00464fc",
      "tools/wtime-bench/gc_workloads.py": "aedd39f75c5c407690251ccd40b8a00139d9a6b9b41f3a1b9980df90b37f543c",
      "tools/wtime-bench/memory_scaling.py": "9dfac7c395c4207f43102a84f6f83858b42d2d52894eea9457d5416a76a9bf8b",
      "tools/wtime-bench/memory_workloads.py": "99eaae89bd1d0a95c67cc8f08c22184b95701d97a14354b93a112246295b743b",
      "tools/wtime-bench/p027_memory_workloads.py": "2b7bf4e7c9cb6c7674c4f465bdca0c9b498f6a1f8bb4843720d0dcc2aee66336",
      "tools/wtime-bench/profile.py": "113475824a37c1e8618c046b0b0f87ab4e85b8a49bb3e65a85e0e6b742dacb84",
      "tools/wtime-bench/representative_scaling.py": "a2fe1dfc4406aaf3f56124cf58423a56b8add89d0874de43968e21efac360b63",
      "tools/wtime-bench/representative_workloads.py": "7ca9e9c08928d7098a85a228a8dbf2081361b3730010e460cf8ea60afdd1204e",
      "tools/wtime-bench/run.py": "f793c150ae38031467c91a70cddf66e85b6cd6d9782e227b773b4da6ff564bea",
      "tools/wtime-bench/scalar_workloads.py": "b564e862913ed3fbd9a54bec0c722962fe999c25024375de69c1ceac03d2682e",
      "tools/wtime-bench/scaling.py": "0a9415ddef9c3900c0ce9c64e286fc767367d0a4ce3dddaf483d04494ef630a3",
      "tools/wtime-bench/simd_scaling.py": "825c68bfe2136a3d7937c0873fbdf39b9ed84fdca3364f0d050d57e89b959a8b",
      "tools/wtime-bench/simd_workloads.py": "c43645c67deb4efc566602800d50913a25d434c3146df5ee686fc4bb7b555f2b",
      "tools/wtime-bench/src/continuation_driver.rs": "2cc5818b256463ef062ad1f36f8a858e3905ce961ffb8b56bae06b143965119a",
      "tools/wtime-bench/src/gc_exception_driver.rs": "63c192dd2f0f5a376dba96b66ac3e55c40303512374b02fe96b782e33ace369f",
      "tools/wtime-bench/src/gc_warm_driver.rs": "9fc4b6ef80c4e4aca86bff36a8fdec32a0d9b4871cf1d757e0d374dbf89bb5b7",
      "tools/wtime-bench/src/main.rs": "9f9496929d3c60165fa061030336397d1825d9b0545a70aadeaf07208c3768e5",
      "tools/wtime-bench/src/memory_driver.rs": "a7a9049232a73141d01040dbbf463b3bae2959ad3633001c55501f5359fecb0f",
      "tools/wtime-bench/src/protocol.rs": "550dd14c99e4715ff16e8ecf8a1520dd8f5d25d32c27748259ccc1e9f434573d",
      "tools/wtime-bench/src/representative_driver.rs": "26de3df2f0d5b6198448e5cb32e28d4c24086e8c2b9ae23dcfe55663920e40b7",
      "tools/wtime-bench/src/scalar_driver.rs": "c43a3369b6dfac8b57aaf789dbabd9da8dab4f28751571a683b970dd09225a58",
      "tools/wtime-bench/src/thread_driver.rs": "96801dbc985a214e335c2be69dedce48a1cd2acf37055547f3d5fc87a21f4ee3",
      "tools/wtime-bench/src/tier_state_driver.rs": "6215becc3a74f02bbea2dbb8d342afe879bd9b9787d2c3c777fea9b871678525",
      "tools/wtime-bench/thread_scaling.py": "b8384bab6beedb5fc524ac26a91d9033be48a3c4a0b385834e8d94a83bdbc35d",
      "tools/wtime-bench/thread_workloads.py": "f50f832f5f0eed010b3277fdcb15cacd0b600298026df1ebac1e00265864f21c",
      "tools/wtime-bench/tier_check.py": "680766b24821f50b47bfa43c8e1c757020b80257f962dd2ae2c06cdc37de96a5",
      "tools/wtime-bench/workloads.json": "68625d67b23a57525e6414264bd179ad502d630ca73cd1c294e32c1490ed5f5a",
      "tools/wtime-bench/workloads.py": "c08faf6b9295f22b3b554819b59d2367c5a436c16308029bccbf85095f425096",
      "tools/wtime-conformance/Cargo.toml": "37c21479a09390a590f0e0ccba31e62ebfddf4096fa3c16039042af9c223c68e",
      "tools/wtime-conformance/build.rs": "1e37a8f6ec1af7964228b03abb3e9675c2c5218edf3bb43a94d76243cc893ef1",
      "tools/wtime-conformance/src/adapter.rs": "b99ae377d141ca6369569cbb1960f00910cc8d0b92fd5007ced206f4ebe0f997",
      "tools/wtime-conformance/src/bin/wtime-allocation-check.rs": "1a20e70ac081c7792732bc1cffded19935e19d0141bca5ea78a6b6086f4c6034",
      "tools/wtime-conformance/src/bin/wtime-probe.rs": "039e380652f8ef356d0995c35a7f14b289479db40f59e9f88d6ccfa9ef9df785",
      "tools/wtime-conformance/src/cli.rs": "ab8466680604fc36dc5e1aafbe06d7fbecd8e1cb1f5ad321099cb3a5094017ae",
      "tools/wtime-conformance/src/corpus.rs": "b1d2a7947b5904b9bd80c0109bc8b5e0f5f8329ba92a693db3f4409cd93cbc1e",
      "tools/wtime-conformance/src/main.rs": "ecf37373cd8988e4f45569d49a0a971fc85a5b687c091ae2ec7964ec8f35c169",
      "tools/wtime-conformance/src/report.rs": "a5bb757ec60513a95f0b9573328a9ee77159a2e061741bb695b52c1fef4348f5",
      "tools/wtime-conformance/src/script.rs": "8c11af05587c584895b9e945c1d0075181b9fb108ef60ae311416cb2b1971888",
      "tools/wtime-conformance/src/spectest.rs": "1403166f4be6ce443b47d4a7ba4a44da553c67e280dfa2876b3f2ad1e64e8883",
      "tools/wtime-conformance/tests/annotations.rs": "5eb54a28e57e7e929bcb2342d65c28e23c2ae112b7c10b36f707fa580f489b08",
      "tools/wtime-conformance/tests/interactions.rs": "fd36710c8cd2ef7f3a2883d3f0ab6cbfecace3ae80adf693095228e4f4641938"
    },
    "record": {
      "bytes": 99681,
      "path": "/workspace/wtime/target/performance/P034-candidate-checks/commands.json",
      "sha256": "d5e60f684ee11d20ba3c746ceef7878f0c1d6af08cf6b45ec2ba06c7c32f0cd1"
    },
    "retention": {
      "bytes": 1032,
      "path": "/workspace/wtime/target/performance/P034-candidate/retention.json",
      "sha256": "168bf0879de43243c643e58b7e895b9af5644d01d3916e7704663ecae5d6d76b"
    },
    "runtime_delta": [
      "crates/wtime-runtime/src/gc_exec.rs",
      "crates/wtime-runtime/src/gc_layout.rs",
      "crates/wtime-runtime/src/lib.rs"
    ]
  },
  "candidate_promoted": false,
  "decision_basis": "The one predeclared quiet build pair failed the leaf_seconds relative limit; all other recomputed build checks are recorded, the comparison/raw gate did not promote the candidate, and the exact predecessor runtime was freshly restored and revalidated.",
  "documentation_summary": "P034 was rejected because the sole quiet build pair exceeded the predeclared leaf-time relative limit (1.691x against 1.25x). The paired GC comparison and correctness checks completed, but no speedup claim is made. The candidate was not promoted; gc_exec.rs and lib.rs were restored byte-for-byte to P034-before, the candidate-only gc_layout.rs was removed, and fresh restored checks passed.",
  "measurement": {
    "analysis": {
      "bytes": 1226775,
      "path": "/workspace/wtime/target/performance/P034/analysis.json",
      "sha256": "f52fd901ddb53c7369c94b99e7570872e5d9b8d45a7605bd022d64b801811f49"
    },
    "candidate_source_manifest": {
      "bytes": 23194,
      "path": "/workspace/wtime/target/performance/P034-candidate/source-manifest.json",
      "sha256": "cb4845aa17d9c33ce0ebfe1d929d4e59c3e0c4bf5e10b24f67964b9cdf2ca674"
    },
    "compact_contract": {
      "bytes": 4144,
      "path": "/workspace/wtime/docs/benchmarks/performance/P034-contract.json",
      "sha256": "1e0038bec040eb356dff993f1f2c1d862f203ce1056fde6338d8ed530b6bc91e"
    },
    "comparison_report": {
      "bytes": 135181121,
      "path": "/workspace/wtime/target/performance/P034/report.json",
      "sha256": "3a86006a8fb50fa36236555e7b3b38d70dc7c68a09ab7cd562ef1fd0dafda62a"
    },
    "extraction": {
      "bytes": 6678,
      "path": "/workspace/wtime/target/performance/P034/extraction.json",
      "sha256": "51f03bb23e299cee2b0fe18bbf1b46c86862cbf7220b102d9774b25e3df44a4b"
    },
    "measurement_counts": {
      "correctness_probes": 234,
      "groups_per_run": 405,
      "groups_total": 810,
      "layout_workload_count": 36,
      "legacy_workload_count": 45,
      "pair_event_count": 24300,
      "pair_events_per_run": 12150,
      "pairs_per_group": 30,
      "primary_workload_count": 24,
      "protected_workload_count": 93,
      "side_observation_count": 48600,
      "warm_calibration_records": 144,
      "warm_workload_count": 36
    },
    "measurement_log": {
      "bytes": 0,
      "exit_code": 0,
      "name": "comparison",
      "path": "/workspace/wtime/target/performance/P034-comparison-run.log",
      "sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    },
    "measurement_wrapper": {
      "bytes": 1689,
      "path": "/workspace/wtime/target/performance/P034-measure-command-record.json",
      "sha256": "6de56d0cf72deb219741680b261769e70ed615397674e6369aab5eb42ec3abb7"
    },
    "pair_journal": {
      "bytes": 82339773,
      "path": "/workspace/wtime/target/performance/P034/checkpoint.jsonl",
      "sha256": "61d7a04fef2e501cbf0d49f55633dc460825c617660c339a2de652496475c673"
    },
    "postprocess": {
      "bytes": 11867,
      "path": "/workspace/wtime/target/performance/P034/commands.json",
      "sha256": "951a4faa8bb0ab8cc6d1a6f0cb5317b8f68a148d0d92544dd0f16688b6367e12"
    },
    "raw_gate_decision": {
      "bytes": 1644954,
      "path": "/workspace/wtime/target/performance/P034/decision.json",
      "sha256": "9c95798c98fa5cbedc75a88d55a672f4f85dbfdddeeb5188c3e8662600b0c825"
    },
    "raw_gate_decision_name": "rejected",
    "raw_gate_exit": 1,
    "rich_measurement_contract": {
      "bytes": 1621,
      "path": "/workspace/wtime/docs/benchmarks/performance/P034-measurement-contract.json",
      "sha256": "0471ddb2b3c6a43d5c65686af402c4bb3d132dfac7df8469b97b8a151d97506b"
    },
    "source_inventory": {
      "archive_sources": 199,
      "candidate_comparison_sources": 197,
      "candidate_full_sources": 200,
      "changed_runtime_sources": [
        "crates/wtime-runtime/src/gc_exec.rs",
        "crates/wtime-runtime/src/gc_layout.rs",
        "crates/wtime-runtime/src/lib.rs"
      ],
      "unchanged_omissions": [
        "LICENSE-APACHE",
        "LICENSE-MIT",
        "tools/wtime-bench/README.md"
      ]
    }
  },
  "no_speedup_claim": true,
  "outcome": "rejected",
  "p001_archive_unchanged": {
    "artifact": {
      "bytes": 22024,
      "path": "/workspace/wtime/target/performance/baseline/manifest.json",
      "sha256": "7349596fc806c96c74cb9c40f1c08402589c1683660b0b152c713264f5808472"
    },
    "manifest_sha256": "7349596fc806c96c74cb9c40f1c08402589c1683660b0b152c713264f5808472"
  },
  "p033_predecessor": {
    "archive_manifest": {
      "bytes": 50796,
      "path": "/workspace/wtime/target/performance/P033-before-corrected/manifest.json",
      "sha256": "d154ed03cd99ebffe86c97136981838c32371ccb961b8466c3248e260a3a2765"
    },
    "final_decision": {
      "bytes": 8861,
      "path": "/workspace/wtime/target/performance/P033/final-decision.json",
      "sha256": "5cb7bf1750b3cdb11895ef22d475773a1d135cde166233cd4fe6dc57137546f4"
    },
    "rollback_identity": {
      "bytes": 8288,
      "path": "/workspace/wtime/target/performance/P033/rollback-identities.json",
      "sha256": "f0424c4c6cf7d8c7d1a157a5f390ce6850e761de71d5e6bac257028cafce45d8"
    }
  },
  "preserved_failure_history": {
    "boundary_correction_patch": {
      "bytes": 12043,
      "path": "/workspace/wtime/target/performance/P034-runtime-install/candidate-before-boundary-test-fix.patch",
      "sha256": "56a1da0551ae8cb087dce61c3b64179625caf4ce9ba1527e3baf42efa100a252"
    },
    "boundary_failure_record": {
      "bytes": 568,
      "path": "/workspace/wtime/target/performance/P034-runtime-install/focused-boundary-test-failure.json",
      "sha256": "572e1ba7021fb5b9819ce59678760162420e17fa4da8a1b075ecc1d36cfde1bc"
    },
    "corrected_followup": {
      "bytes": 2243,
      "failed": 0,
      "ignored": 0,
      "passed": 8,
      "path": "/workspace/wtime/target/performance/P034-candidate-focused-corrected.log",
      "preserved": true,
      "private_tests": {
        "failed": 0,
        "ignored": 0,
        "names": [
          "p034_bounded_failure_uses_complete_borrowed_fallback",
          "p034_local_indices_remain_distinct_from_canonical_identity",
          "p034_mvp_modules_do_not_allocate_optional_gc_locations",
          "p034_reservation_failure_declines_before_publishing_locations"
        ],
        "passed": 4
      },
      "public_tests": {
        "failed": 0,
        "ignored": 0,
        "names": [
          "p034_equal_heap_type_fields_keep_high_module_local_reference_context",
          "p034_high_local_array_and_struct_indices_keep_packed_field_semantics",
          "p034_layout_preparation_preserves_gc_resource_failure_without_live_objects",
          "p034_prior_engine_canonical_history_does_not_replace_module_local_indices"
        ],
        "passed": 4
      },
      "sha256": "a17c887174a14def0d66e716b030980c20c8be08aaabdf4d3fe494cc42ac6783"
    },
    "failed_source_snapshot": {
      "bytes": 7752,
      "path": "/workspace/wtime/target/performance/P034-runtime-install/gc_layout.initial-focused.rs",
      "sha256": "b63067c873bf8fa4013349215a2681815695facd0566bf2bf9cc33c3abdc35d0"
    },
    "initial_failure": {
      "bytes": 1273,
      "failed": 1,
      "failed_case": "p034_bounded_failure_uses_complete_borrowed_fallback",
      "ignored": 0,
      "passed": 3,
      "path": "/workspace/wtime/target/performance/P034-candidate-focused-initial.log",
      "preserved": true,
      "public_tests_ran": false,
      "sha256": "ff70a1746a9afe0ac6913eb3be9aed137f9a12e6352eb2eeb038dbef3a93be03"
    }
  },
  "published_evidence": {
    "/workspace/wtime/docs/benchmarks/performance/P034-decision.md": {
      "bytes": 1003,
      "sha256": "405c796616063388ff4bb7393242065aa1e29a34ad34b698619e837dca4a646c"
    },
    "/workspace/wtime/docs/benchmarks/performance/P034-evidence.md": {
      "bytes": 5289,
      "sha256": "7ab6547bc20020b1960a4596e3ca70079e59551ed485b92cd618fd388e1fb421"
    },
    "/workspace/wtime/docs/benchmarks/performance/P034-extraction.md": {
      "bytes": 878,
      "sha256": "af2975e11d384f01ff235591b319d35e1b4a856384397d0566149da14b0e3804"
    },
    "/workspace/wtime/docs/benchmarks/performance/P034-rollback.md": {
      "bytes": 1817,
      "sha256": "ed7cd81ff33990e880936a8cfc4726b5b1c923aba2ef25ad388b954926c3abe5"
    },
    "/workspace/wtime/docs/benchmarks/performance/P034-strict-build.md": {
      "bytes": 1245,
      "sha256": "ff13d602d529afd6d7c439fe9e80cfc88be636d0c8323cc337817193c20a44fb"
    },
    "/workspace/wtime/target/performance/P034/final-decision.json": {
      "bytes": 18017,
      "sha256": "97c79fb1c651a7fc9b7ab001bc2784c99699e7cf5bf62aaec3e15754809664a0"
    }
  },
  "restoration": {
    "candidate_only_module_absent": true,
    "command_record": {
      "bytes": 5681,
      "path": "/workspace/wtime/target/performance/P034/restore-commands.json",
      "sha256": "bad7ffdafe4862a5aad3b20f3ba3993d1dd1147f03af6e95e28e50800d2d994a"
    },
    "commands": {
      "restored_benchmark_build": {
        "bytes": 206,
        "exit_code": 0,
        "name": "restored_benchmark_build",
        "path": "/workspace/wtime/target/performance/P034/restored-benchmark-build.log",
        "sha256": "83b2d5c6211e2a3690e08e3bf0365a644937c437da6169c78fed377261ebb394"
      },
      "restored_build_budget": {
        "bytes": 1702,
        "exit_code": 0,
        "name": "restored_build_budget",
        "path": "/workspace/wtime/target/performance/P034/restored-build-budget.log",
        "sha256": "d8a8d3c779701cc600f2d422c5a58f353f0bb24f57d6740047c7307aac748a0e"
      },
      "restored_default_tests": {
        "bytes": 16211,
        "exit_code": 0,
        "name": "restored_default_tests",
        "path": "/workspace/wtime/target/performance/P034/restored-default-tests.log",
        "sha256": "2ddb58d5ea9d9155a53e065d4501567903cf5465170302c72c061def08d5f8de"
      },
      "restored_diagnostic_performance": {
        "bytes": 6574,
        "exit_code": 0,
        "name": "restored_diagnostic_performance",
        "path": "/workspace/wtime/target/performance/P034/restored-diagnostic-tests.log",
        "sha256": "e1f65c1685fc87a2cced6f7417131897b8ff95ecbab61b7951037b77a252b83a"
      },
      "restored_focused_default": {
        "bytes": 629,
        "exit_code": 0,
        "name": "restored_focused_default",
        "path": "/workspace/wtime/target/performance/P034/restored-focused-default-tests.log",
        "sha256": "8a551cc10c8cfefe76c3b10ec89911b71d4fbf484145d653c3bf2a6cf65a358b"
      },
      "restored_focused_diagnostic": {
        "bytes": 629,
        "exit_code": 0,
        "name": "restored_focused_diagnostic",
        "path": "/workspace/wtime/target/performance/P034/restored-focused-diagnostic-tests.log",
        "sha256": "e0fee3511a7fdae2daae006049764448304fdc5705a0893088e7d1940748687f"
      }
    },
    "helper_snapshot": {
      "bytes": 46936,
      "path": "/workspace/wtime/target/performance/P034/restore-helper.snapshot.py",
      "sha256": "aacc747fb5406139511489de5518a63fcd110a952ba2757b1e9009ba49dc330a"
    },
    "initial_restore_guard_failure": {
      "candidate_checks_sha256": "d5e60f684ee11d20ba3c746ceef7878f0c1d6af08cf6b45ec2ba06c7c32f0cd1",
      "command_log": {
        "bytes": 72,
        "path": "/workspace/wtime/target/performance/P034-closure/restore.log",
        "sha256": "02884c3c2776bb23ae539805a07c7be8edaca1cb5ddbc6915628497f6a939fb0"
      },
      "exit_code": 1,
      "failure_record": {
        "bytes": 24888,
        "path": "/workspace/wtime/target/performance/P034/restore-initial-guard-failure.json",
        "sha256": "b978d72cf72398fba30d604b19852bc096c6b67c3776b5870fe708d0ecb0582a"
      },
      "full_candidate_source_count": 200,
      "helper_source": {
        "bytes": 46520,
        "path": "/workspace/wtime/target/performance/P034-closure/restore-reviewed-source.py",
        "sha256": "57f705b64a44de7a9919b64c6dfc52207b0f06dc1c67dd934ef539ff4bc2f09e"
      },
      "root_command_record": {
        "bytes": 4315,
        "path": "/workspace/wtime/target/performance/P034-closure/record.json",
        "sha256": "95322bd6842d540ce18b7e5dff1eb306c9f22bbbaf5135abc6c4e186cd68c6ff"
      },
      "source_mutations": false
    },
    "input_identity": {
      "bytes": 6584,
      "path": "/workspace/wtime/target/performance/P034/restore-input-identities.json",
      "sha256": "2c51dbfc7bd4458f0a71dd0b1a29643988fa5577f329037f030ba88db75f339a"
    },
    "p001_manifest_sha256": "7349596fc806c96c74cb9c40f1c08402589c1683660b0b152c713264f5808472",
    "preliminary_decision": {
      "bytes": 5252,
      "path": "/workspace/wtime/target/performance/P034/preliminary-decision.json",
      "sha256": "0336c6de9b1bb174c4d8fc2aabc985e137720bbf0268d7baf34f114663acc776"
    },
    "preliminary_helper": {
      "bytes": 13895,
      "path": "/workspace/wtime/target/performance/P034/preliminary-executed.py",
      "sha256": "aa486603f9e9744606c6ec49ea0cf5a4cbd6119a47b141f7089ad3ef28cf7ce6"
    },
    "preliminary_outcome": "rejected",
    "restored_build": {
      "bytes": 112134,
      "path": "/workspace/wtime/target/performance/P034-restored-build.json",
      "sha256": "7cfa2ccd7004e9324a0e8ff2bdb0d24609ca7bfb34b98a84efb1f7dfa6bcbe33"
    },
    "restored_runtime": {
      "crates/wtime-runtime/src/gc_exec.rs": {
        "byte_identical": true,
        "fresh_mtime_ns": 1791759498988850161,
        "sha256": "56cf9173994d267fa4ead0af137b8f9f941f6b86a52d458f31be3ce7a32280fc"
      },
      "crates/wtime-runtime/src/lib.rs": {
        "byte_identical": true,
        "fresh_mtime_ns": 1791759498992580104,
        "sha256": "997a0ec8d835c83567e13aa4c755ed30ad0e44a0bdb9e689c14753abdd47bb45"
      }
    },
    "rollback_identity": {
      "bytes": 4964,
      "path": "/workspace/wtime/target/performance/P034/rollback-identities.json",
      "sha256": "c8f2db2cb4b562c2b795dfa2c6d3a6cb7c32b76978be9c86199c136201c842ff"
    }
  },
  "schema": "wtime-p034-completion-v1",
  "status": "done",
  "strict_build": {
    "artifacts": {
      "before_build": {
        "bytes": 112136,
        "path": "/workspace/wtime/target/performance/P034-before-build.json",
        "sha256": "cc67a47348d3e86fde0c4d13c5b2c51203bc441c0a46aa4b9648e40abff8bac4"
      },
      "candidate_build": {
        "bytes": 112358,
        "path": "/workspace/wtime/target/performance/P034-build.json",
        "sha256": "d344e43109634e2daab191c3a6a117e70865e49083c9751fe652161018240eaf"
      },
      "policy": {
        "bytes": 2914,
        "path": "/workspace/wtime/docs/benchmarks/performance/P034-build-decision.md",
        "sha256": "404eb8be2c91537e5cff5ed0daf9a883de80fdaf60601707cf44b72bf65fc69e"
      },
      "strict_decision": {
        "bytes": 1550,
        "path": "/workspace/wtime/target/performance/P034-incremental-build-decision.json",
        "sha256": "881185a85e6162f841c356c1f1c5d6f0e9ff485d176bd320f8aafbb5c5d8f556"
      },
      "strict_helper_snapshot": {
        "bytes": 2743,
        "path": "/workspace/wtime/target/performance/P034-strict-build-executed.py",
        "sha256": "7019c54abf367eab51d7921d0c92250351ccd25f2e98309db68df1f3d49e95fa"
      }
    },
    "default_dependency_closure_passed": true,
    "failing_metric": "leaf_seconds",
    "hard_budgets_passed": true,
    "limit": 1.25,
    "passed": false,
    "ratio": 1.6911309918435735,
    "ratios": {
      "clean_peak_rss_kib": 1.001142421934501,
      "clean_seconds": 1.0471659172207848,
      "leaf_seconds": 1.6911309918435735,
      "release_seconds": 0.8888346420954268,
      "stripped_bytes": 0.9989046572056766,
      "unchanged_peak_rss_kib": 1.0075696134090295
    },
    "relative_limits": {
      "clean_peak_rss_kib": 1.25,
      "clean_seconds": 1.25,
      "leaf_seconds": 1.25,
      "release_seconds": 1.25,
      "stripped_bytes": 1.25,
      "unchanged_peak_rss_kib": 1.25
    },
    "runtime_leaf_extra_measurement": {
      "before_seconds": 1.0239517750014784,
      "candidate_seconds": 0.9837666509993142,
      "ratio": 0.9607548666028669,
      "used_as_gate": false
    },
    "settings_match": true,
    "single_quiet_pair": true,
    "source_inputs_stable": true,
    "status": "failed_relative_leaf_budget"
  },
  "task": "P034",
  "task_index_update": {
    "evidence_artifact": "/tmp/P034-completion.json",
    "outcome": "rejected",
    "status": "done"
  }
}
```
