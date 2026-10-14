# P021 — Compute bounded per-function execution metadata

Status: **done**

Outcome: **complete**

Kind: `foundation` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P016 — Add explicit executor policies and test integration hooks](P016-execution-policy.md)

[P018 — Define source cursors and materialized guest state](P018-source-cursor.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C6, C7** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/execution/plan.rs`
- `crates/wtime-runtime/src/lib.rs`
- `crates/wtime-runtime/tests/performance/p021.rs`

- `crates/wtime-runtime/src/execution.rs`
- `crates/wtime-runtime/src/execution/effects.rs`
- `crates/wtime-validate/src/lib.rs`
- `crates/wtime-runtime/tests/fixtures/performance/plans/*`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

FunctionPlan stores validated parameter/local/result layouts, maximum nesting and stack peak, source effects and immutable type/control lookups; Store-specific handles are excluded.

## Implementation recipe

1. Inspect validated metadata already available before adding any duplicate pass; derive missing local counts/nesting/stack bounds with checked arithmetic and bounded work.
2. Represent label arities, catch/handler side-table ranges and numeric/reference slot classes explicitly; exact/plain and foreign module contexts remain in canonical-aware metadata.
3. Reserve immutable plans during preparation and make them available to frame/control construction; any optional analysis failure declines that optimization without bypassing required admission.
4. Measure retained bytes and preparation subphases, documenting how immutable plans are charged once and per-activation storage separately.

## Meaningful cases

- Deep/multivalue nesting produces the correct source stack peak and arities.
- Wide local/type indices are retained or decline narrow forms safely.
- Analysis overflow/work exhaustion cannot create a partially usable plan.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime p021_ -- --nocapture
```

## Measurement and decision

Selection: `scalar`. Explicit execution setting: `decoded`. Decision rule: `integrity`.

This is a tooling/contract deliverable. Require correct nonzero checks and bounded/source-bound output; do not invent a runtime speedup requirement. If it changes production work, record cold/build/retention deltas under [MEASUREMENT.md](../MEASUREMENT.md).

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. Executed evidence is recorded below.

## Executed evidence

```json
{
  "artifacts": [
    "/workspace/wtime/target/performance/P021-final/report.json",
    "/workspace/wtime/target/performance/P021-final/analysis.json",
    "/workspace/wtime/target/performance/P021-final/build.json",
    "/workspace/wtime/target/performance/P021-final/runtime-tests.log",
    "/workspace/wtime/target/performance/P021-final/phase-probe.log",
    "/workspace/wtime/target/performance/P021-final/structuredphase-probe.json",
    "/workspace/wtime/target/performance/P021-final/evidence.txt",
    "/workspace/wtime/target/performance/P021-final/source.sha256",
    "/workspace/wtime/target/performance/P021-final/source-after-verify.log",
    "/workspace/wtime/target/performance/P021-final/preparation-probe.rs",
    "/workspace/wtime/target/performance/P021-tests.log",
    "/workspace/wtime/target/performance/P021-validator-tests.log",
    "/workspace/wtime/target/performance/P021-before/manifest.json"
  ],
  "commands": [
    {
      "command": "cargo test --locked -p wtime-runtime p021_ -- --nocapture",
      "exit_code": 0,
      "passed": 5,
      "failed": 0
    },
    {
      "command": "cargo test --locked -p wtime-validate",
      "exit_code": 0,
      "passed": 90,
      "failed": 0
    },
    {
      "command": "cargo test --locked -p wtime-runtime --features execution-reference,performance-counters",
      "exit_code": 0,
      "passed": 167,
      "failed": 0
    },
    {
      "command": "python3 tools/wtime-bench/compare.py --baseline target/performance/P021-before --candidate tools/wtime-bench/profiling/target/release/wtime-bench --selection original-mvp --output target/performance/P021-final",
      "exit_code": 0,
      "valid_pairs": 1260,
      "runs": 2,
      "pairs_per_group": 30
    },
    {
      "command": "python3 tools/wtime-bench/analyze.py --input target/performance/P021-final/report.json --output target/performance/P021-final/analysis.json",
      "exit_code": 0
    },
    {
      "command": "python3 tools/checks/build_budget.py --output target/performance/P021-final/build.json",
      "exit_code": 0
    },
    {
      "command": "cargo test --locked --offline -p wtime-runtime p021_preparation_phase_probe -- --nocapture",
      "cwd": "/tmp/P021-phase-probe",
      "exit_code": 0,
      "passed": 1,
      "diagnostic_fixtures": 3
    },
    {
      "command": "python3 tools/checks/dependency_budget.py",
      "exit_code": 0,
      "production_violations": 0
    }
  ],
  "hashes": {
    "target/performance/P021-final/report.json": "0707d7f2765dc4b70868f043b65e00842e6a6e6769f793d555490a99cc07ba8a",
    "target/performance/P021-final/analysis.json": "ece16b9210e2c6ae1a8d6d47ccb2b506035596cdb9afc752f16935cdcc10fb49",
    "target/performance/P021-final/build.json": "f560a0df7ecef1a21e10eff598834793f8c0e4f729f12276aeacbd44081301a0",
    "target/performance/P021-final/runtime-tests.log": "e8509cc7aca20208460f38267e95eb4a99b2fab7120a2e8de83181c0aa4d9383",
    "target/performance/P021-final/phase-probe.log": "abf7f8ea836c4c1bd757ce1b4144be8a671b2d2f7dbc5806117fe74999362a07",
    "target/performance/P021-final/structuredphase-probe.json": "6868b375aeab1f0c483dcafb2dc810eee8a5099a301167ea771a59b65da1f49a",
    "target/performance/P021-final/evidence.txt": "dad2c015dc26fd4c52fd2d49f4d312d4e0ad7dc5b0187755a425e3bc77a310cb",
    "target/performance/P021-final/source.sha256": "1f61d292176e270e9fdcc8242dd8b7237704bf75b6e7c27806b621503d536521",
    "target/performance/P021-final/source-after-verify.log": "c42709c03e15b228aa0b33c2b286e2c6077f4a167653cf1d75684c16c03b1dc5",
    "target/performance/P021-final/preparation-probe.rs": "1847a416f6df80c0d36bbddb556e2bc4742859ee2294a681c29c6778f347037c",
    "target/performance/P021-tests.log": "18cd3f789ca2e64b6a6e6979ca1f342e7eb08c2355c95208a04e5c66a9c7914e",
    "target/performance/P021-validator-tests.log": "3b9e33469f18b01b08438471f0038096ec22912d89fd0d05ee984c874b930b1a",
    "target/performance/P021-before/manifest.json": "cb3a81e044e796a24f2de1a9109bee9d153dccf0ef2be488e58471255d1b4fd2"
  },
  "fixture_hashes": {
    "crates/wtime-runtime/tests/fixtures/performance/plans/nested_multivalue.wasm": "5309a91f0a9b8935b2ab4deb7c55aa40ef04d66fc1acf7c32ca01c51bf399224",
    "crates/wtime-runtime/tests/fixtures/performance/plans/wide_local.wasm": "1e91688442c4e20093aa3dd0ef572958c588014857f0a9448ac6b2382439995c",
    "crates/wtime-runtime/tests/fixtures/performance/plans/wide_local.wat": "cd02ca29cb31089e9e0fdcbbb056f52317142927c202dc93a8230c4ffd85b81d",
    "crates/wtime-runtime/tests/fixtures/performance/plans/nested_multivalue.wat": "060c0c0710ee22ded2aee339d75ee80934e216d963295124574c7f2110d86cd1"
  },
  "settings": {
    "selection": "original-mvp",
    "execution_policy": "decoded",
    "instrumented": false,
    "fuel": false,
    "quantum": 1
  },
  "limitations": [
    "Function bounds are per-body validation high-water allocation hints, including the implicit function control. Unreachable polymorphism and cumulative caller operands prevent using these bounds to remove source resource checks. P019 per-source transient peaks remain unknown; no optimized tier is enabled.",
    "Layouts and exact nominal types, effects, control and catch/resume handler views borrow original immutable Module-owned storage; plans contain no Store handles. Analysis is capped at 100,000 conservative work units and 1 MiB, declines transactionally, and frame setup uses original decoded locals when unavailable.",
    "Capacity accounting includes plan storage and validator bounds payload, excludes allocator overhead and fixed ModuleData headers; Module clones share one payload, per-activation Vecs remain separate.",
    "Cold preparation median ratios ranged approximately 0.99\u20131.25 across two runs; memory preparation run two had a broad CI (upper 3.06), so performance is inconclusive. No runtime speedup is claimed for this foundation. Default build passed absolute budgets at 8.10s clean, 0.34s CLI leaf, 1.13s runtime leaf, 14.94s release, 1,000,264 stripped bytes, zero external core dependencies. Relative clean build vs P020 5.74s is 1.41; foundation adds validation bounds and plan/effect metadata, while cloud variation prevents attributing the entire delta to code. Future integration gates must evaluate combined costs.",
    "Preparation subphases were executed in a byte-verified isolated source copy with an injected private test-only diagnostic helper, not the production timing binary. Debug single observations cover validation/control/plans on three fixtures; they are not production latency estimates.",
    "Five private P021 cases run in the existing library unit-test target via include from tests/performance/p021.rs to preserve the private planning boundary; broader runtime tests include planned GC-local initialization and host reentrant collection."
  ]
}
```
