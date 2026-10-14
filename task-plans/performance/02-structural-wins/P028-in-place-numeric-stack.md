# P028 — Execute common scalar operations in place on the operand stack

Status: **done**

Outcome: **rejected**

Kind: `candidate` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P002 — Compare separately built baseline and candidate binaries](../00-measurement/P002-paired-driver.md)

[P004 — Compute uncertainty and preparation break-even](../00-measurement/P004-paired-statistics.md)

[P005 — Bind speed decisions to build and dependency budgets](../00-measurement/P005-performance-gates.md)

[P008 — Add scaling, branch and call benchmark families](../00-measurement/P008-scalar-workloads.md)

[P016 — Add explicit executor policies and test integration hooks](../01-execution-contracts/P016-execution-policy.md)

[P020 — Centralize exact source instruction accounting](../01-execution-contracts/P020-exact-execution-accounting.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C2, C3, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/numeric.rs`
- `crates/wtime-runtime/tests/performance.rs`
- `crates/wtime-runtime/tests/performance/p028.rs`
- `crates/wtime-runtime/tests/fixtures/performance/numeric/integer_numeric.wat`
- `crates/wtime-runtime/tests/fixtures/performance/numeric/integer_numeric.wasm`
- `crates/wtime-runtime/tests/fixtures/performance/numeric/LICENSE.txt`
- `crates/wtime-runtime/tests/fixtures/performance/numeric/README.md`
- `crates/wtime-runtime/tests/fixtures/performance/numeric/manifest.json`
- `crates/wtime-runtime/tests/fixtures/performance/numeric/source-hashes.sha256`
- `docs/benchmarks/performance/P028-contract.json`
- `docs/benchmarks/performance/P028-build-decision.md`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

Common i32/i64 unary/binary operations overwrite existing checked stack slots and truncate rather than repeated pop/push; unsupported numeric families keep the existing helper path.

## Implementation recipe

1. Start with add/sub/mul/bitwise/shifts/comparisons and one unary family, using safe last/slice access and Value matching; do not change public Value layout.
2. Preserve wrapping, masking, signedness and underflow/type behavior; leave division/conversion/float operations on existing helpers until independently covered.
3. Apply source accounting at the unchanged dispatch boundary and preserve source opcode coverage.
4. Benchmark dependency chains and mixed scalar controls; inspect code size/build deltas before adding more opcode specializations.
5. Record the changed path with focused assertions/diagnostic counts and exact source/binary identities; compare the pre-edit archive, and revert a rejected structural optimization rather than adding a permanent toggle solely for testing.

## Meaningful cases

- Random boundary inputs preserve i32/i64 wrap, shifts and comparisons.
- A short underflow/type-corrupted internal test returns an error without memory unsafety.
- Fuel interruption at each original arithmetic instruction matches decoded execution.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime p028_ -- --nocapture
cargo test --locked -p wtime-runtime --features execution-reference,performance-counters p028_ -- --nocapture
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml -j 4
python3 tools/wtime-bench/compare.py --baseline target/performance/P028-before --candidate tools/wtime-bench/target/release/wtime-bench --candidate-mode decoded --selection scalar --pairs 30 --build-growth-explanation-file docs/benchmarks/performance/P028-build-decision.md --output target/performance/P028
python3 tools/wtime-bench/analyze.py --input target/performance/P028 --output target/performance/P028/analysis.json
python3 tools/checks/build_budget.py --output target/performance/P028/build.json
python3 tools/wtime-bench/gates.py --task P028 --comparison target/performance/P028 --build target/performance/P028/build.json --contract docs/benchmarks/performance/P028-contract.json --output target/performance/P028/decision.json
```

## Measurement and decision

Selection: `scalar`. Explicit execution setting: `decoded`. Decision rule: `measured_promotion`. The predeclared primary family is the eight `scalar_accumulator` and `scalar_dependent` rows at n=1, 32, 1024, and 10000; n=0 and n=17 in those families are held-out protected controls. All 85 selected scalar workloads remain selected with their registered stages. The selected primary timings support i32 dependency-chain evidence only; they do not support performance claims for i64 or operations absent from their timed WATs. See `docs/benchmarks/performance/P028-build-decision.md` for exact roles and limits.

Use the paired protocol and family/control/cold/build rules in [MEASUREMENT.md](../MEASUREMENT.md). Predeclare the targeted family and held-out controls before changing the implementation. Keep instrumentation off in production timings. Adopt only with supported gains in both runs; otherwise record rejected/inconclusive and remove default-path cost. A necessary immutable-interface change may be contract_only with its separate justification.

## Before editing

Run these commands before changing the candidate sources. The archive is immutable and captures the immediate predecessor. Retain P001's original archive for milestone totals. For native work, use the named portable control and verify its actual policy/quantum; if this archive already exists with different sources, choose a new recorded run-specific directory and update the comparison path rather than overwriting it.

```sh
python3 tools/wtime-bench/baseline.py --output target/performance/P028-before
```

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; preserve the declared 85-workload scalar selection and report i64/absent-op performance coverage as unavailable; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. Executed evidence is recorded below.

## Executed evidence

```json
{
  "artifact_hashes": {
    "/workspace/wtime/docs/benchmarks/performance/P028-build-decision.md": {
      "bytes": 6768,
      "sha256": "bb939a330182ad3b05fbe20f8ad8909b3f3128a94127d9f01fbc7bd7de0e1274"
    },
    "/workspace/wtime/docs/benchmarks/performance/P028-candidate.patch": {
      "bytes": 16542,
      "sha256": "c46420d7871110d2e9eec1c8a0ff0c727f684ecb952c4f6f13c7f4159f9de0a2"
    },
    "/workspace/wtime/docs/benchmarks/performance/P028-contract.json": {
      "bytes": 2757,
      "sha256": "0ba9945b059f3bde5af4e9f47c9ff97e75323fc297df8eb138a9b73b6b999303"
    },
    "/workspace/wtime/docs/benchmarks/performance/P028-evidence.json": {
      "bytes": 612819,
      "sha256": "79884b14f50472d491db94a3f332cc2d074d57435f57335bc387e4ea41055e2c"
    },
    "/workspace/wtime/target/performance/P028-before-build.json": {
      "bytes": 91506,
      "sha256": "aaba64db6dec19ce0a30ff00cef6cde6b682427788a659781931352cc6feb7b4"
    },
    "/workspace/wtime/target/performance/P028-before/manifest.json": {
      "bytes": 36758,
      "sha256": "d3c0184847518532e4213ab3e82a427b4484ca4632b25631e1c4a3e1946a3c9d"
    },
    "/workspace/wtime/target/performance/P028-build.json": {
      "bytes": 91509,
      "sha256": "28a79c7b19b5cd234d054a6474d7b15000efc2849df93bd2a14235e5c2ec70ef"
    },
    "/workspace/wtime/target/performance/P028-candidate-diagnostic-tests.log": {
      "bytes": 1975,
      "sha256": "af9b83e4d244bc6958c667cc311a83d134bc027d84a0fc1bfef2a2b1ecae7768"
    },
    "/workspace/wtime/target/performance/P028-candidate-format-option-failure.rs": {
      "bytes": 23192,
      "sha256": "4942e015240ea1c1c86e1ad1a683d9d59f38e15021595098ca1f148eb3313a48"
    },
    "/workspace/wtime/target/performance/P028-candidate-retained/source-manifest.json": {
      "bytes": 14770,
      "sha256": "a8e38e0d6c1ef3fc18a134a28ff9a90df585394ee8cf2d7cbd1f93a30b88e6b8"
    },
    "/workspace/wtime/target/performance/P028-candidate-tests-final.log": {
      "bytes": 1906,
      "sha256": "598349145b54e1603b7ce00a7028c68a45f5f79fdd74fd7ca9f6fd519e5dbc16"
    },
    "/workspace/wtime/target/performance/P028-candidate-tests.log": {
      "bytes": 1063,
      "sha256": "21e71c6db69cc3fe071894cd671586488534ae3662901def79ecb71c1bd96403"
    },
    "/workspace/wtime/target/performance/P028-incremental-build-decision.json": {
      "bytes": 1088,
      "sha256": "13eec2338a836b097ecb5df91b7f37cc91b6318537553bfb306c148913949279"
    },
    "/workspace/wtime/target/performance/P028-performance-tests.log": {
      "bytes": 4317,
      "sha256": "336bccc0737e4785bb070a9eb54ee6b1ac30316ca6090275b8724eb9d4e54455"
    },
    "/workspace/wtime/target/performance/P028-runtime-tests.log": {
      "bytes": 14442,
      "sha256": "aea985acea740da29d9596e01135af110ec0b7cede5def9e5fdeac5124236621"
    },
    "/workspace/wtime/target/performance/P028/analysis.json": {
      "bytes": 1267115,
      "sha256": "36e17e6f3fa86eb4c752612d8f13754a68120d6b2dcdee4788503babe6e4e3f9"
    },
    "/workspace/wtime/target/performance/P028/checkpoint.jsonl": {
      "bytes": 36226703,
      "sha256": "620cc427375dcd74dc7559d0c5323a5d34236417342c27bda25c80d252572b70"
    },
    "/workspace/wtime/target/performance/P028/commands.json": {
      "bytes": 10361,
      "sha256": "b2a9104612e0324941ebffce9a5bc62221a887c3b39e4e89fdc77edf370261ec"
    },
    "/workspace/wtime/target/performance/P028/comparison-build.json": {
      "bytes": 91509,
      "sha256": "28a79c7b19b5cd234d054a6474d7b15000efc2849df93bd2a14235e5c2ec70ef"
    },
    "/workspace/wtime/target/performance/P028/complete_group_count_failed.py": {
      "bytes": 43558,
      "sha256": "9cd50d87f939d211cf07102a3275e91cea2e585e2c84aad0a095fc159070fb35"
    },
    "/workspace/wtime/target/performance/P028/decision.json": {
      "bytes": 1699979,
      "sha256": "ac2a4f671db319e086ac82b77dd4e8928e2c8b7ffe794bbc50842e3e394722fd"
    },
    "/workspace/wtime/target/performance/P028/extract_executed.py": {
      "bytes": 31057,
      "sha256": "a081b70085481bdcf63437045aac5076b7f2fcb0ade9d00416fd1855ea61f648"
    },
    "/workspace/wtime/target/performance/P028/extract_failed_executed.py": {
      "bytes": 31010,
      "sha256": "28e330a0a15e7a22c8118782a2120f102777393d3dfc400646f0336418c2b72e"
    },
    "/workspace/wtime/target/performance/P028/extraction-retry.log": {
      "bytes": 76373,
      "sha256": "1197e067fe4c6a8696877a88f0013ee26c3ccccfaac5709a190864a45963ed5a"
    },
    "/workspace/wtime/target/performance/P028/extraction.json": {
      "bytes": 115408,
      "sha256": "309a0023675aa5117928dd5de9af8da2966d1a6f431fee7fc69f03af11627626"
    },
    "/workspace/wtime/target/performance/P028/extraction.log": {
      "bytes": 96,
      "sha256": "e5adf0afeaa14a3ced9945673702217059d120dbaeaf6d8c1d935f75f6412e08"
    },
    "/workspace/wtime/target/performance/P028/final-decision.json": {
      "bytes": 5888,
      "sha256": "1afc936f1d80f1091e3e945df1190cfafcd1db7e149f7825ffab660755994a54"
    },
    "/workspace/wtime/target/performance/P028/finalizer.log": {
      "bytes": 77,
      "sha256": "d5b4ef8dbdba68350e0255eec7b055d3d4c458ac04145e33956a1f1e77f7a265"
    },
    "/workspace/wtime/target/performance/P028/process_failed_executed.py": {
      "bytes": 3290,
      "sha256": "38f529bc6c422587db4859ca757b58b4dbb45074a10b4909e22b320226cc6f76"
    },
    "/workspace/wtime/target/performance/P028/process_resume_executed.py": {
      "bytes": 2474,
      "sha256": "42838feee5b141c215074117511be9f7e4366ea191df7905be52fcd7c3e5f7e3"
    },
    "/workspace/wtime/target/performance/P028/process_resume_log_collision_failed.py": {
      "bytes": 2468,
      "sha256": "cd7e91e002dfb0f872e201de8f98800b716ee8fa661282ff4694844963629b5c"
    },
    "/workspace/wtime/target/performance/P028/raw-exits.json": {
      "bytes": 39,
      "sha256": "8cb285167680019e8683e276900144b811a534abc0797ee27c6fde696476ba90"
    },
    "/workspace/wtime/target/performance/P028/report.json": {
      "bytes": 59842022,
      "sha256": "13a195a5b30575c930af94de6d84e74c255948f59fee22f057790e99608c702f"
    },
    "/workspace/wtime/target/performance/P028/restore_executed.py": {
      "bytes": 2793,
      "sha256": "63a316bb7ceae4a40f2939c66fa555ac95d5c07d905a1dac31925c3361a405ae"
    },
    "/workspace/wtime/target/performance/P028/restored-build.json": {
      "bytes": 91508,
      "sha256": "d794fda64c0b19197ed29247ec624fb2b2676192653ccf4d74ca7736b9a3054f"
    },
    "/workspace/wtime/target/performance/P028/restored-default-tests.log": {
      "bytes": 14051,
      "sha256": "5b4aa977ff45d12dbe82833bda744dd73353a6d020cf0adb8abeabedfc814628"
    },
    "/workspace/wtime/target/performance/P028/restored-diagnostic-tests.log": {
      "bytes": 4392,
      "sha256": "19e64866637315284e002d035c61615e8373baacf4b6718dad8668f8de811b14"
    },
    "/workspace/wtime/target/performance/P028/rollback-identities.json": {
      "bytes": 562,
      "sha256": "04e0372c2cc62e36c2418b09d796e3ade179181183ab0dc0dbfa4e77317b93c2"
    },
    "/workspace/wtime/target/performance/P028/task-precompletion/P028-in-place-numeric-stack.md": {
      "bytes": 6637,
      "sha256": "7ce8ec9fb129a545e70d22e190952a95940d30141aa94749dfb56eb193e470ce"
    },
    "/workspace/wtime/target/performance/P028/task-precompletion/TASKS.json": {
      "bytes": 901015,
      "sha256": "8fef2d5aaf3ebd3ef4b96ac5cee3060c94656dfa817259bb4f79abf922fbd0e1"
    }
  },
  "artifacts": [
    "/workspace/wtime/docs/benchmarks/performance/P028-build-decision.md",
    "/workspace/wtime/docs/benchmarks/performance/P028-candidate.patch",
    "/workspace/wtime/docs/benchmarks/performance/P028-contract.json",
    "/workspace/wtime/docs/benchmarks/performance/P028-evidence.json",
    "/workspace/wtime/target/performance/P028-before-build.json",
    "/workspace/wtime/target/performance/P028-before/manifest.json",
    "/workspace/wtime/target/performance/P028-build.json",
    "/workspace/wtime/target/performance/P028-candidate-diagnostic-tests.log",
    "/workspace/wtime/target/performance/P028-candidate-format-option-failure.rs",
    "/workspace/wtime/target/performance/P028-candidate-retained/source-manifest.json",
    "/workspace/wtime/target/performance/P028-candidate-tests-final.log",
    "/workspace/wtime/target/performance/P028-candidate-tests.log",
    "/workspace/wtime/target/performance/P028-incremental-build-decision.json",
    "/workspace/wtime/target/performance/P028-performance-tests.log",
    "/workspace/wtime/target/performance/P028-runtime-tests.log",
    "/workspace/wtime/target/performance/P028/analysis.json",
    "/workspace/wtime/target/performance/P028/checkpoint.jsonl",
    "/workspace/wtime/target/performance/P028/commands.json",
    "/workspace/wtime/target/performance/P028/comparison-build.json",
    "/workspace/wtime/target/performance/P028/complete_group_count_failed.py",
    "/workspace/wtime/target/performance/P028/decision.json",
    "/workspace/wtime/target/performance/P028/extract_executed.py",
    "/workspace/wtime/target/performance/P028/extract_failed_executed.py",
    "/workspace/wtime/target/performance/P028/extraction-retry.log",
    "/workspace/wtime/target/performance/P028/extraction.json",
    "/workspace/wtime/target/performance/P028/extraction.log",
    "/workspace/wtime/target/performance/P028/final-decision.json",
    "/workspace/wtime/target/performance/P028/finalizer.log",
    "/workspace/wtime/target/performance/P028/process_failed_executed.py",
    "/workspace/wtime/target/performance/P028/process_resume_executed.py",
    "/workspace/wtime/target/performance/P028/process_resume_log_collision_failed.py",
    "/workspace/wtime/target/performance/P028/raw-exits.json",
    "/workspace/wtime/target/performance/P028/report.json",
    "/workspace/wtime/target/performance/P028/restore_executed.py",
    "/workspace/wtime/target/performance/P028/restored-build.json",
    "/workspace/wtime/target/performance/P028/restored-default-tests.log",
    "/workspace/wtime/target/performance/P028/restored-diagnostic-tests.log",
    "/workspace/wtime/target/performance/P028/rollback-identities.json",
    "/workspace/wtime/target/performance/P028/task-precompletion/P028-in-place-numeric-stack.md",
    "/workspace/wtime/target/performance/P028/task-precompletion/TASKS.json"
  ],
  "candidate_promoted": false,
  "commands": [
    {
      "command": ". /workspace/.setup/rust-env.sh\npython3 tools/wtime-bench/compare.py --baseline target/performance/P028-before --candidate tools/wtime-bench/target/release/wtime-bench --candidate-mode decoded --selection scalar --pairs 30 --build-growth-explanation-file docs/benchmarks/performance/P028-build-decision.md --output target/performance/P028 > target/performance/P028-run.log 2>&1",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P028-run.log",
      "log_sha256": "a6651069c51ef66aeac29ba445569051f42bc608a7da852b9c2a5b9abea3b7c5",
      "name": "comparison"
    },
    {
      "command": ". /workspace/.setup/rust-env.sh\npython3 /tmp/extract_p028.py --report /workspace/wtime/target/performance/P028/report.json --checkpoint /workspace/wtime/target/performance/P028/checkpoint.jsonl --archive /workspace/wtime/target/performance/P028-before --candidate-dir /workspace/wtime/target/performance/P028-candidate-retained --output /workspace/wtime/target/performance/P028/extraction.json",
      "exit_code": 1,
      "log_path": "/workspace/wtime/target/performance/P028/extraction.log",
      "log_sha256": "e5adf0afeaa14a3ced9945673702217059d120dbaeaf6d8c1d935f75f6412e08",
      "name": "extraction_failed",
      "stdout_path": "/workspace/wtime/target/performance/P028/extraction.log",
      "stdout_sha256": "e5adf0afeaa14a3ced9945673702217059d120dbaeaf6d8c1d935f75f6412e08"
    },
    {
      "command": ". /workspace/.setup/rust-env.sh\npython3 /tmp/extract_p028.py --report /workspace/wtime/target/performance/P028/report.json --checkpoint /workspace/wtime/target/performance/P028/checkpoint.jsonl --archive /workspace/wtime/target/performance/P028-before --candidate-dir /workspace/wtime/target/performance/P028-candidate-retained --output /workspace/wtime/target/performance/P028/extraction.json",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P028/extraction-retry.log",
      "log_sha256": "1197e067fe4c6a8696877a88f0013ee26c3ccccfaac5709a190864a45963ed5a",
      "name": "extraction",
      "stdout_path": "/workspace/wtime/target/performance/P028/extraction-retry.log",
      "stdout_sha256": "1197e067fe4c6a8696877a88f0013ee26c3ccccfaac5709a190864a45963ed5a"
    },
    {
      "command": ". /workspace/.setup/rust-env.sh\npython3 tools/wtime-bench/analyze.py --input target/performance/P028 --output target/performance/P028/analysis.json",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P028/analysis.log",
      "log_sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
      "name": "analysis",
      "stdout_path": "/workspace/wtime/target/performance/P028/analysis.log",
      "stdout_sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    },
    {
      "command": ". /workspace/.setup/rust-env.sh\npython3 tools/wtime-bench/gates.py --task P028 --comparison target/performance/P028 --build target/performance/P028/comparison-build.json --contract docs/benchmarks/performance/P028-contract.json --output target/performance/P028/decision.json",
      "exit_code": 1,
      "log_path": "/workspace/wtime/target/performance/P028/raw-gate.log",
      "log_sha256": "ac2a4f671db319e086ac82b77dd4e8928e2c8b7ffe794bbc50842e3e394722fd",
      "name": "raw_gate",
      "stdout_path": "/workspace/wtime/target/performance/P028/raw-gate.log",
      "stdout_sha256": "ac2a4f671db319e086ac82b77dd4e8928e2c8b7ffe794bbc50842e3e394722fd"
    },
    {
      "command": ". /workspace/.setup/rust-env.sh\ncargo test --locked -p wtime-runtime",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P028/restored-default-tests.log",
      "log_sha256": "5b4aa977ff45d12dbe82833bda744dd73353a6d020cf0adb8abeabedfc814628",
      "name": "restored_default_tests"
    },
    {
      "command": ". /workspace/.setup/rust-env.sh\ncargo test --locked -p wtime-runtime --features execution-reference,performance-counters --test performance",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P028/restored-diagnostic-tests.log",
      "log_sha256": "19e64866637315284e002d035c61615e8373baacf4b6718dad8668f8de811b14",
      "name": "restored_diagnostic_tests"
    },
    {
      "command": ". /workspace/.setup/rust-env.sh\ncargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml -j 4",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P028/restored-benchmark-build.log",
      "log_sha256": "b27a726b15b82f381527e3d14d6704853f06e1d63463befcba1d01ce02d81f02",
      "name": "rollback_benchmark_build"
    },
    {
      "command": ". /workspace/.setup/rust-env.sh\npython3 tools/checks/build_budget.py --output target/performance/P028/restored-build.json",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P028/restored-build.log",
      "log_sha256": "154403a8179e75ec2b8131de7f6469bfe14343a5caa9d104a8e06dff4179b6a6",
      "name": "rollback_build"
    },
    {
      "command": ". /workspace/.setup/rust-env.sh\ncargo test --locked -p wtime-runtime p028_ -- --nocapture",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P028-before-tests.log",
      "log_sha256": "1b60ad244f6ee6ee425caa6a53b379783e5f9c214276dacbff05f26e2cb16e89",
      "name": "before_focused_default",
      "provenance": "actual previously executed command, retained log; recorded after measurement"
    },
    {
      "command": ". /workspace/.setup/rust-env.sh\ncargo test --locked -p wtime-runtime --features execution-reference,performance-counters p028_ -- --nocapture",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P028-before-diagnostic-tests.log",
      "log_sha256": "3d9061a646cb738fa45ecccac52a230c20c081009ead791ea704c85ede51bfb7",
      "name": "before_focused_diagnostic",
      "provenance": "actual previously executed command, retained log; recorded after measurement"
    },
    {
      "command": ". /workspace/.setup/rust-env.sh\ncargo test --locked -p wtime-runtime p028_ -- --nocapture",
      "exit_code": 101,
      "log_path": "/workspace/wtime/target/performance/P028-candidate-tests.log",
      "log_sha256": "21e71c6db69cc3fe071894cd671586488534ae3662901def79ecb71c1bd96403",
      "name": "candidate_focused_compile_failure",
      "provenance": "actual previously executed command, retained log; recorded after measurement"
    },
    {
      "command": ". /workspace/.setup/rust-env.sh\ncargo test --locked -p wtime-runtime p028_ -- --nocapture",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P028-candidate-tests-final.log",
      "log_sha256": "598349145b54e1603b7ce00a7028c68a45f5f79fdd74fd7ca9f6fd519e5dbc16",
      "name": "candidate_focused_default",
      "provenance": "actual previously executed command, retained log; recorded after measurement"
    },
    {
      "command": ". /workspace/.setup/rust-env.sh\ncargo test --locked -p wtime-runtime --features execution-reference,performance-counters p028_ -- --nocapture",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P028-candidate-diagnostic-tests.log",
      "log_sha256": "af9b83e4d244bc6958c667cc311a83d134bc027d84a0fc1bfef2a2b1ecae7768",
      "name": "candidate_focused_diagnostic",
      "provenance": "actual previously executed command, retained log; recorded after measurement"
    },
    {
      "command": ". /workspace/.setup/rust-env.sh\ncargo test --locked -p wtime-runtime",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P028-runtime-tests.log",
      "log_sha256": "aea985acea740da29d9596e01135af110ec0b7cede5def9e5fdeac5124236621",
      "name": "candidate_default",
      "provenance": "actual previously executed command, retained log; recorded after measurement"
    },
    {
      "command": ". /workspace/.setup/rust-env.sh\ncargo test --locked -p wtime-runtime --features execution-reference,performance-counters --test performance",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P028-performance-tests.log",
      "log_sha256": "336bccc0737e4785bb070a9eb54ee6b1ac30316ca6090275b8724eb9d4e54455",
      "name": "candidate_diagnostic",
      "provenance": "actual previously executed command, retained log; recorded after measurement"
    },
    {
      "command": ". /workspace/.setup/rust-env.sh\ncargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml -j 4",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P028-candidate-bench-build.log",
      "log_sha256": "aec7502d5b6434e6e5eb31a8f469e1973933ca402065baf59bc18d2dfb57a5b9",
      "name": "candidate_benchmark_build",
      "provenance": "actual previously executed command, retained log; recorded after measurement"
    },
    {
      "command": ". /workspace/.setup/rust-env.sh\npython3 tools/checks/build_budget.py --output target/performance/P028-before-build.json",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P028-before-build.log",
      "log_sha256": "48b37a8a4a3a06dd1b6a524564c47bc11e387099ed53cfc602f0de323e002d42",
      "name": "before_quiet_build",
      "provenance": "actual previously executed command, retained log; recorded after measurement"
    },
    {
      "command": ". /workspace/.setup/rust-env.sh\npython3 tools/checks/build_budget.py --output target/performance/P028-build.json",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P028-build.log",
      "log_sha256": "beea082d42665e0867268bb96a310d37072ae8764d720617aaa180fbccd9bbf5",
      "name": "candidate_quiet_build",
      "provenance": "actual previously executed command, retained log; recorded after measurement"
    },
    {
      "command": ". /workspace/.setup/rust-env.sh\npython3 tools/wtime-bench/baseline.py --output target/performance/P028-before",
      "exit_code": 0,
      "log_path": "/workspace/wtime/target/performance/P028-baseline.log",
      "log_sha256": "8c564e95603cedfc309d7b87bc1fc1d34022bd516cbbc9898ff922455afe0ac5",
      "name": "predecessor_archive",
      "provenance": "actual previously executed command, retained log; recorded after measurement"
    }
  ],
  "counts": {
    "analysis_group_count": 850,
    "correctness_probe_kinds": {},
    "correctness_probe_records": 0,
    "expected_pair_ids_sha256": "e2d25b4fb5e5913269e686e33e2b8957e5998391b8ae2e2103a30a433d37b7ad",
    "fixture_files_verified": 85,
    "group_count_total": 850,
    "observation_side_checks": 51000,
    "pair_event_count": 25500,
    "pairs_per_group": 30,
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
    "warm_calibration_policy_limit": "legacy common MIN batch from one-shot estimates; no guarantee that both actual timers reached 20 ms",
    "warm_calibration_records": 0,
    "warm_estimated_under_20ms_observation_count": 3493,
    "warm_observed_duration_summary": {
      "0:branch_calls:baseline": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 38504126.0,
          "median": 18779418.5,
          "min": 13670407.0,
          "p95": 33543012.0
        },
        "estimated_under_20ms_count": 17,
        "observations": 30
      },
      "0:branch_calls:candidate": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 74216205.0,
          "median": 16778786.5,
          "min": 13684940.0,
          "p95": 51486884.0
        },
        "estimated_under_20ms_count": 23,
        "observations": 30
      },
      "0:call_overhead:baseline": {
        "batch": 44445,
        "estimated_total_ns": {
          "max": 41519985.66,
          "median": 23604850.6125,
          "min": 17921824.02,
          "p95": 31920176.775000002
        },
        "estimated_under_20ms_count": 10,
        "observations": 30
      },
      "0:call_overhead:candidate": {
        "batch": 44445,
        "estimated_total_ns": {
          "max": 45354211.365,
          "median": 21179575.8525,
          "min": 16251892.035,
          "p95": 43647834.48
        },
        "estimated_under_20ms_count": 13,
        "observations": 30
      },
      "0:integer_loop:baseline": {
        "batch": 1,
        "estimated_total_ns": {
          "max": 20391600.0,
          "median": 8189690.5,
          "min": 6843072.0,
          "p95": 13380367.0
        },
        "estimated_under_20ms_count": 29,
        "observations": 30
      },
      "0:integer_loop:candidate": {
        "batch": 1,
        "estimated_total_ns": {
          "max": 13133069.0,
          "median": 8631343.5,
          "min": 6835526.0,
          "p95": 11856189.0
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "0:scalar_accumulator_n0:baseline": {
        "batch": 9819,
        "estimated_total_ns": {
          "max": 9884649.834,
          "median": 6960389.6205,
          "min": 5786886.564,
          "p95": 9245413.296
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "0:scalar_accumulator_n0:candidate": {
        "batch": 9819,
        "estimated_total_ns": {
          "max": 8976333.42,
          "median": 6741553.5675,
          "min": 5555865.131999999,
          "p95": 8886273.552000001
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "0:scalar_accumulator_n10000:baseline": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 50369031.0,
          "median": 27648748.0,
          "min": 23084601.0,
          "p95": 35641333.0
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_accumulator_n10000:candidate": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 73613114.0,
          "median": 29305607.5,
          "min": 24025587.0,
          "p95": 38704936.0
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_accumulator_n1024:baseline": {
        "batch": 37,
        "estimated_total_ns": {
          "max": 58208162.016,
          "median": 47919513.5005,
          "min": 21927043.008,
          "p95": 58010280.984
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_accumulator_n1024:candidate": {
        "batch": 37,
        "estimated_total_ns": {
          "max": 69031149.01,
          "median": 44805612.5,
          "min": 22320941.012,
          "p95": 63204345.017
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_accumulator_n17:baseline": {
        "batch": 2154,
        "estimated_total_ns": {
          "max": 36587505.822000004,
          "median": 28396876.665,
          "min": 24409240.007999998,
          "p95": 34574666.058
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_accumulator_n17:candidate": {
        "batch": 2154,
        "estimated_total_ns": {
          "max": 40107051.354,
          "median": 26158690.806,
          "min": 22462291.104,
          "p95": 29744359.830000002
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_accumulator_n1:baseline": {
        "batch": 7080,
        "estimated_total_ns": {
          "max": 28188786.36,
          "median": 12097199.52,
          "min": 8936135.28,
          "p95": 26313223.56
        },
        "estimated_under_20ms_count": 27,
        "observations": 30
      },
      "0:scalar_accumulator_n1:candidate": {
        "batch": 7080,
        "estimated_total_ns": {
          "max": 19551023.52,
          "median": 11029857.66,
          "min": 9666111.6,
          "p95": 19383255.84
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "0:scalar_accumulator_n32:baseline": {
        "batch": 1250,
        "estimated_total_ns": {
          "max": 57046222.5,
          "median": 29955316.25,
          "min": 24813386.25,
          "p95": 53696343.75
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_accumulator_n32:candidate": {
        "batch": 1250,
        "estimated_total_ns": {
          "max": 61462643.75,
          "median": 29034478.125,
          "min": 24124195.0,
          "p95": 56274492.5
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_br_table_n0:baseline": {
        "batch": 25707,
        "estimated_total_ns": {
          "max": 31417784.343,
          "median": 22453740.589500003,
          "min": 18067882.173,
          "p95": 29596828.998000003
        },
        "estimated_under_20ms_count": 4,
        "observations": 30
      },
      "0:scalar_br_table_n0:candidate": {
        "batch": 25707,
        "estimated_total_ns": {
          "max": 80977024.293,
          "median": 21050588.262,
          "min": 18242278.461000003,
          "p95": 40109423.871
        },
        "estimated_under_20ms_count": 11,
        "observations": 30
      },
      "0:scalar_br_table_n10000:baseline": {
        "batch": 1,
        "estimated_total_ns": {
          "max": 482702608.0,
          "median": 28013119.5,
          "min": 17515176.0,
          "p95": 96864744.0
        },
        "estimated_under_20ms_count": 2,
        "observations": 30
      },
      "0:scalar_br_table_n10000:candidate": {
        "batch": 1,
        "estimated_total_ns": {
          "max": 138859109.0,
          "median": 28804232.5,
          "min": 14679071.0,
          "p95": 58734534.0
        },
        "estimated_under_20ms_count": 6,
        "observations": 30
      },
      "0:scalar_br_table_n1024:baseline": {
        "batch": 12,
        "estimated_total_ns": {
          "max": 174707177.004,
          "median": 18671533.002,
          "min": 15447612.0,
          "p95": 41368842.0
        },
        "estimated_under_20ms_count": 19,
        "observations": 30
      },
      "0:scalar_br_table_n1024:candidate": {
        "batch": 12,
        "estimated_total_ns": {
          "max": 100506786.99599999,
          "median": 18514949.502,
          "min": 12104256.0,
          "p95": 51154697.00400001
        },
        "estimated_under_20ms_count": 20,
        "observations": 30
      },
      "0:scalar_br_table_n17:baseline": {
        "batch": 70,
        "estimated_total_ns": {
          "max": 4016712.98,
          "median": 1978915.505,
          "min": 1257183.97,
          "p95": 3539101.02
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "0:scalar_br_table_n17:candidate": {
        "batch": 70,
        "estimated_total_ns": {
          "max": 4887448.0200000005,
          "median": 1970731.0,
          "min": 1319016.02,
          "p95": 3953782.98
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "0:scalar_br_table_n1:baseline": {
        "batch": 12739,
        "estimated_total_ns": {
          "max": 58179344.214,
          "median": 37721472.0085,
          "min": 29407994.239000004,
          "p95": 56160212.714
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_br_table_n1:candidate": {
        "batch": 12739,
        "estimated_total_ns": {
          "max": 239609608.982,
          "median": 35349100.7775,
          "min": 30795933.767,
          "p95": 91258272.388
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_br_table_n32:baseline": {
        "batch": 751,
        "estimated_total_ns": {
          "max": 46205196.144999996,
          "median": 38055571.698,
          "min": 33464085.368,
          "p95": 45716313.92
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_br_table_n32:candidate": {
        "batch": 751,
        "estimated_total_ns": {
          "max": 67991821.048,
          "median": 39183599.6075,
          "min": 31795677.286,
          "p95": 53765902.163
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_branch_entropy_n0:baseline": {
        "batch": 24722,
        "estimated_total_ns": {
          "max": 28479966.498,
          "median": 23209483.318,
          "min": 20561707.674,
          "p95": 28462784.708
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_branch_entropy_n0:candidate": {
        "batch": 24722,
        "estimated_total_ns": {
          "max": 31651378.823999997,
          "median": 23358309.758,
          "min": 20482893.938,
          "p95": 30328974.321999997
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_branch_entropy_n10000:baseline": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 46283612.0,
          "median": 31588793.0,
          "min": 27056887.0,
          "p95": 40872022.0
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_branch_entropy_n10000:candidate": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 59886522.0,
          "median": 30211909.0,
          "min": 25140646.0,
          "p95": 43372206.0
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_branch_entropy_n1024:baseline": {
        "batch": 9,
        "estimated_total_ns": {
          "max": 28927872.999,
          "median": 12602730.501,
          "min": 11083966.002,
          "p95": 20474217.0
        },
        "estimated_under_20ms_count": 28,
        "observations": 30
      },
      "0:scalar_branch_entropy_n1024:candidate": {
        "batch": 9,
        "estimated_total_ns": {
          "max": 29211879.996,
          "median": 12628303.5015,
          "min": 11210562.999,
          "p95": 19836633.996
        },
        "estimated_under_20ms_count": 29,
        "observations": 30
      },
      "0:scalar_branch_entropy_n17:baseline": {
        "batch": 1081,
        "estimated_total_ns": {
          "max": 44648042.496999994,
          "median": 32433081.931,
          "min": 27434079.587,
          "p95": 39479170.732
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_branch_entropy_n17:candidate": {
        "batch": 1081,
        "estimated_total_ns": {
          "max": 70419220.865,
          "median": 32817882.258,
          "min": 26248410.680999998,
          "p95": 47450209.616
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_branch_entropy_n1:baseline": {
        "batch": 235,
        "estimated_total_ns": {
          "max": 1583131.08,
          "median": 529164.54,
          "min": 301955.965,
          "p95": 1251875.0799999998
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "0:scalar_branch_entropy_n1:candidate": {
        "batch": 235,
        "estimated_total_ns": {
          "max": 1656636.965,
          "median": 484750.01,
          "min": 290060.03,
          "p95": 946603.03
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "0:scalar_branch_entropy_n32:baseline": {
        "batch": 83,
        "estimated_total_ns": {
          "max": 21772743.015,
          "median": 3841966.001,
          "min": 2983012.032,
          "p95": 5247684.96
        },
        "estimated_under_20ms_count": 29,
        "observations": 30
      },
      "0:scalar_branch_entropy_n32:candidate": {
        "batch": 83,
        "estimated_total_ns": {
          "max": 7124373.973,
          "median": 3754867.0045,
          "min": 2956914.0100000002,
          "p95": 6503643.035
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "0:scalar_call_ref_n0:baseline": {
        "batch": 22754,
        "estimated_total_ns": {
          "max": 48549185.870000005,
          "median": 24220074.351,
          "min": 21148978.348,
          "p95": 45089644.956
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_call_ref_n0:candidate": {
        "batch": 22754,
        "estimated_total_ns": {
          "max": 39970427.282,
          "median": 24089875.963,
          "min": 20997482.216,
          "p95": 31147495.520000003
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_call_ref_n10000:baseline": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 39929277.0,
          "median": 25965918.0,
          "min": 22704268.0,
          "p95": 30366432.0
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_call_ref_n10000:candidate": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 37322519.0,
          "median": 24200434.0,
          "min": 21995164.0,
          "p95": 30572498.0
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_call_ref_n1024:baseline": {
        "batch": 17,
        "estimated_total_ns": {
          "max": 42844409.001,
          "median": 25323412.993,
          "min": 22523512.007,
          "p95": 31005709.998
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_call_ref_n1024:candidate": {
        "batch": 17,
        "estimated_total_ns": {
          "max": 42120702.001,
          "median": 24676653.501000002,
          "min": 21199619.004,
          "p95": 40060094.006
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_call_ref_n17:baseline": {
        "batch": 1537,
        "estimated_total_ns": {
          "max": 83346786.79900001,
          "median": 49293478.2045,
          "min": 40100832.599,
          "p95": 77077128.638
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_call_ref_n17:candidate": {
        "batch": 1537,
        "estimated_total_ns": {
          "max": 160425128.13,
          "median": 45767800.0145,
          "min": 36500055.052,
          "p95": 79773063.52600001
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_call_ref_n1:baseline": {
        "batch": 12407,
        "estimated_total_ns": {
          "max": 46853348.148,
          "median": 32208299.046,
          "min": 26784765.101,
          "p95": 40837218.662
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_call_ref_n1:candidate": {
        "batch": 12407,
        "estimated_total_ns": {
          "max": 42778889.348,
          "median": 30243563.746999998,
          "min": 26940994.044999998,
          "p95": 41006499.77
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_call_ref_n32:baseline": {
        "batch": 851,
        "estimated_total_ns": {
          "max": 85680816.00999999,
          "median": 39779884.795499995,
          "min": 34646132.409,
          "p95": 81349904.257
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_call_ref_n32:candidate": {
        "batch": 851,
        "estimated_total_ns": {
          "max": 93153047.03999999,
          "median": 38791080.238,
          "min": 33985999.795,
          "p95": 92323082.058
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_dependent_n0:baseline": {
        "batch": 21835,
        "estimated_total_ns": {
          "max": 43071764.67,
          "median": 19529988.225,
          "min": 16497521.59,
          "p95": 24541732.105
        },
        "estimated_under_20ms_count": 18,
        "observations": 30
      },
      "0:scalar_dependent_n0:candidate": {
        "batch": 21835,
        "estimated_total_ns": {
          "max": 32275383.415,
          "median": 19725236.795,
          "min": 16523134.045,
          "p95": 29176975.080000002
        },
        "estimated_under_20ms_count": 19,
        "observations": 30
      },
      "0:scalar_dependent_n10000:baseline": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 33335203.0,
          "median": 17089390.5,
          "min": 13897995.0,
          "p95": 32694840.0
        },
        "estimated_under_20ms_count": 25,
        "observations": 30
      },
      "0:scalar_dependent_n10000:candidate": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 25850345.0,
          "median": 16195617.0,
          "min": 13649239.0,
          "p95": 24146112.0
        },
        "estimated_under_20ms_count": 25,
        "observations": 30
      },
      "0:scalar_dependent_n1024:baseline": {
        "batch": 23,
        "estimated_total_ns": {
          "max": 25238544.0,
          "median": 20527897.003,
          "min": 18242776.002,
          "p95": 24395332.996
        },
        "estimated_under_20ms_count": 9,
        "observations": 30
      },
      "0:scalar_dependent_n1024:candidate": {
        "batch": 23,
        "estimated_total_ns": {
          "max": 27563789.996,
          "median": 19851746.498999998,
          "min": 16888862.992999997,
          "p95": 27186328.992
        },
        "estimated_under_20ms_count": 16,
        "observations": 30
      },
      "0:scalar_dependent_n17:baseline": {
        "batch": 2340,
        "estimated_total_ns": {
          "max": 109134820.08,
          "median": 33567697.8,
          "min": 29214811.08,
          "p95": 62781958.980000004
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_dependent_n17:candidate": {
        "batch": 2340,
        "estimated_total_ns": {
          "max": 43182357.66,
          "median": 31452914.61,
          "min": 28124979.48,
          "p95": 42791622.12
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_dependent_n1:baseline": {
        "batch": 10549,
        "estimated_total_ns": {
          "max": 27177589.131,
          "median": 16963614.821999997,
          "min": 15531303.249,
          "p95": 24616049.304
        },
        "estimated_under_20ms_count": 25,
        "observations": 30
      },
      "0:scalar_dependent_n1:candidate": {
        "batch": 10549,
        "estimated_total_ns": {
          "max": 23635182.186,
          "median": 17289199.158,
          "min": 15254750.665000001,
          "p95": 23602543.580000002
        },
        "estimated_under_20ms_count": 26,
        "observations": 30
      },
      "0:scalar_dependent_n32:baseline": {
        "batch": 1196,
        "estimated_total_ns": {
          "max": 57852138.188,
          "median": 33532531.266,
          "min": 28541286.592,
          "p95": 55867242.236
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_dependent_n32:candidate": {
        "batch": 1196,
        "estimated_total_ns": {
          "max": 38749906.052,
          "median": 31683792.737999998,
          "min": 26882487.216,
          "p95": 38378467.92
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_direct_calls_n0:baseline": {
        "batch": 4989,
        "estimated_total_ns": {
          "max": 7674947.886,
          "median": 4784949.9,
          "min": 3753369.3809999996,
          "p95": 6966320.3040000005
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "0:scalar_direct_calls_n0:candidate": {
        "batch": 4989,
        "estimated_total_ns": {
          "max": 7071004.490999999,
          "median": 4802840.454,
          "min": 3674383.533,
          "p95": 6083297.238000001
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "0:scalar_direct_calls_n10000:baseline": {
        "batch": 1,
        "estimated_total_ns": {
          "max": 31737318.0,
          "median": 12692523.0,
          "min": 9670730.0,
          "p95": 22640075.0
        },
        "estimated_under_20ms_count": 28,
        "observations": 30
      },
      "0:scalar_direct_calls_n10000:candidate": {
        "batch": 1,
        "estimated_total_ns": {
          "max": 25352669.0,
          "median": 11727008.5,
          "min": 10489425.0,
          "p95": 19922131.0
        },
        "estimated_under_20ms_count": 29,
        "observations": 30
      },
      "0:scalar_direct_calls_n1024:baseline": {
        "batch": 3,
        "estimated_total_ns": {
          "max": 19439160.0,
          "median": 4377939.0,
          "min": 2991488.001,
          "p95": 10507035.999
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "0:scalar_direct_calls_n1024:candidate": {
        "batch": 3,
        "estimated_total_ns": {
          "max": 12824294.001000002,
          "median": 4778002.0005,
          "min": 3115241.001,
          "p95": 9883808.001
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "0:scalar_direct_calls_n17:baseline": {
        "batch": 533,
        "estimated_total_ns": {
          "max": 17382852.123,
          "median": 11840793.276,
          "min": 10047918.790000001,
          "p95": 16459983.943
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "0:scalar_direct_calls_n17:candidate": {
        "batch": 533,
        "estimated_total_ns": {
          "max": 13205360.155,
          "median": 11281666.682,
          "min": 10018046.805,
          "p95": 13090282.257000001
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "0:scalar_direct_calls_n1:baseline": {
        "batch": 13124,
        "estimated_total_ns": {
          "max": 50096329.096,
          "median": 32490115.623999998,
          "min": 26669175.408,
          "p95": 40840969.32
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_direct_calls_n1:candidate": {
        "batch": 13124,
        "estimated_total_ns": {
          "max": 53250774.364,
          "median": 33000704.844,
          "min": 25568833.0,
          "p95": 49042117.548
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_direct_calls_n32:baseline": {
        "batch": 924,
        "estimated_total_ns": {
          "max": 111720096.18,
          "median": 47607166.992,
          "min": 36981076.44,
          "p95": 102067638.288
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_direct_calls_n32:candidate": {
        "batch": 924,
        "estimated_total_ns": {
          "max": 100315783.26,
          "median": 45623594.478,
          "min": 34859429.22,
          "p95": 88066842.86400001
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_dynamic_api_n0:baseline": {
        "batch": 47620,
        "estimated_total_ns": {
          "max": 36421109.36,
          "median": 21074445.29,
          "min": 16846098.82,
          "p95": 31089002.72
        },
        "estimated_under_20ms_count": 12,
        "observations": 30
      },
      "0:scalar_dynamic_api_n0:candidate": {
        "batch": 47620,
        "estimated_total_ns": {
          "max": 29412731.099999998,
          "median": 20610650.3,
          "min": 16982672.98,
          "p95": 29000960.96
        },
        "estimated_under_20ms_count": 11,
        "observations": 30
      },
      "0:scalar_dynamic_api_n10000:baseline": {
        "batch": 48544,
        "estimated_total_ns": {
          "max": 55996814.688,
          "median": 20722486.992,
          "min": 16642533.696,
          "p95": 40056567.04
        },
        "estimated_under_20ms_count": 11,
        "observations": 30
      },
      "0:scalar_dynamic_api_n10000:candidate": {
        "batch": 48544,
        "estimated_total_ns": {
          "max": 42766438.752,
          "median": 21186106.464,
          "min": 16421658.496,
          "p95": 34038033.376
        },
        "estimated_under_20ms_count": 11,
        "observations": 30
      },
      "0:scalar_dynamic_api_n1024:baseline": {
        "batch": 50762,
        "estimated_total_ns": {
          "max": 59562455.654,
          "median": 21850655.186,
          "min": 18726761.706,
          "p95": 36602193.91
        },
        "estimated_under_20ms_count": 5,
        "observations": 30
      },
      "0:scalar_dynamic_api_n1024:candidate": {
        "batch": 50762,
        "estimated_total_ns": {
          "max": 38357290.06,
          "median": 21534154.116,
          "min": 18956053.66,
          "p95": 30257603.816
        },
        "estimated_under_20ms_count": 7,
        "observations": 30
      },
      "0:scalar_dynamic_api_n17:baseline": {
        "batch": 35088,
        "estimated_total_ns": {
          "max": 23627838.144,
          "median": 16385622.311999999,
          "min": 12282308.784,
          "p95": 22226072.544
        },
        "estimated_under_20ms_count": 26,
        "observations": 30
      },
      "0:scalar_dynamic_api_n17:candidate": {
        "batch": 35088,
        "estimated_total_ns": {
          "max": 28542649.391999997,
          "median": 15984461.208,
          "min": 12834102.672,
          "p95": 24339563.136
        },
        "estimated_under_20ms_count": 25,
        "observations": 30
      },
      "0:scalar_dynamic_api_n1:baseline": {
        "batch": 45872,
        "estimated_total_ns": {
          "max": 103309202.768,
          "median": 25276205.952,
          "min": 20402856.416,
          "p95": 36349752.624
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_dynamic_api_n1:candidate": {
        "batch": 45872,
        "estimated_total_ns": {
          "max": 35703049.168,
          "median": 24206035.128,
          "min": 19777712.8,
          "p95": 35494102.208
        },
        "estimated_under_20ms_count": 1,
        "observations": 30
      },
      "0:scalar_dynamic_api_n32:baseline": {
        "batch": 46621,
        "estimated_total_ns": {
          "max": 42335551.059,
          "median": 23727151.876999997,
          "min": 19658816.933,
          "p95": 29450159.353
        },
        "estimated_under_20ms_count": 1,
        "observations": 30
      },
      "0:scalar_dynamic_api_n32:candidate": {
        "batch": 46621,
        "estimated_total_ns": {
          "max": 41237766.372,
          "median": 23566262.806,
          "min": 20221345.919,
          "p95": 36845881.688
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_host_call_n0:baseline": {
        "batch": 26247,
        "estimated_total_ns": {
          "max": 33406079.226,
          "median": 22137835.2975,
          "min": 19028523.813,
          "p95": 31269048.485999998
        },
        "estimated_under_20ms_count": 3,
        "observations": 30
      },
      "0:scalar_host_call_n0:candidate": {
        "batch": 26247,
        "estimated_total_ns": {
          "max": 31603750.229999997,
          "median": 22465161.6345,
          "min": 19093905.09,
          "p95": 30828650.073
        },
        "estimated_under_20ms_count": 1,
        "observations": 30
      },
      "0:scalar_host_call_n10000:baseline": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 31977494.0,
          "median": 19418862.0,
          "min": 17476218.0,
          "p95": 24905244.0
        },
        "estimated_under_20ms_count": 18,
        "observations": 30
      },
      "0:scalar_host_call_n10000:candidate": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 41413736.0,
          "median": 20139654.0,
          "min": 17328369.0,
          "p95": 27390023.0
        },
        "estimated_under_20ms_count": 13,
        "observations": 30
      },
      "0:scalar_host_call_n1024:baseline": {
        "batch": 16,
        "estimated_total_ns": {
          "max": 25784822.0,
          "median": 16336538.504,
          "min": 14434990.0,
          "p95": 23823668.0
        },
        "estimated_under_20ms_count": 28,
        "observations": 30
      },
      "0:scalar_host_call_n1024:candidate": {
        "batch": 16,
        "estimated_total_ns": {
          "max": 38469159.008,
          "median": 16462568.496,
          "min": 13525576.992,
          "p95": 33398392.992
        },
        "estimated_under_20ms_count": 28,
        "observations": 30
      },
      "0:scalar_host_call_n17:baseline": {
        "batch": 1679,
        "estimated_total_ns": {
          "max": 446599276.84900004,
          "median": 55379567.6585,
          "min": 30860387.701,
          "p95": 238275516.70200002
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_host_call_n17:candidate": {
        "batch": 1679,
        "estimated_total_ns": {
          "max": 509530060.53900003,
          "median": 59463519.407000005,
          "min": 45624272.92,
          "p95": 286681907.73300004
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_host_call_n1:baseline": {
        "batch": 5138,
        "estimated_total_ns": {
          "max": 30439043.123999998,
          "median": 12787534.039,
          "min": 8398549.11,
          "p95": 29703548.7
        },
        "estimated_under_20ms_count": 26,
        "observations": 30
      },
      "0:scalar_host_call_n1:candidate": {
        "batch": 5138,
        "estimated_total_ns": {
          "max": 31519662.146,
          "median": 12294304.022,
          "min": 8615968.718,
          "p95": 28023751.532
        },
        "estimated_under_20ms_count": 23,
        "observations": 30
      },
      "0:scalar_host_call_n32:baseline": {
        "batch": 1147,
        "estimated_total_ns": {
          "max": 49431760.055,
          "median": 39140139.1075,
          "min": 25295983.880000003,
          "p95": 46854652.927
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_host_call_n32:candidate": {
        "batch": 1147,
        "estimated_total_ns": {
          "max": 56282819.73,
          "median": 37563765.3925,
          "min": 33605980.528000005,
          "p95": 45553091.235999994
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_indirect_calls_n0:baseline": {
        "batch": 2298,
        "estimated_total_ns": {
          "max": 13395441.852,
          "median": 3233518.098,
          "min": 1440577.134,
          "p95": 9302830.241999999
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "0:scalar_indirect_calls_n0:candidate": {
        "batch": 2298,
        "estimated_total_ns": {
          "max": 9453305.58,
          "median": 3417032.931,
          "min": 2063702.814,
          "p95": 7220419.41
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "0:scalar_indirect_calls_n10000:baseline": {
        "batch": 1,
        "estimated_total_ns": {
          "max": 35584702.0,
          "median": 15359081.5,
          "min": 11848351.0,
          "p95": 31294730.0
        },
        "estimated_under_20ms_count": 22,
        "observations": 30
      },
      "0:scalar_indirect_calls_n10000:candidate": {
        "batch": 1,
        "estimated_total_ns": {
          "max": 29793924.0,
          "median": 15505142.5,
          "min": 11145049.0,
          "p95": 28973549.0
        },
        "estimated_under_20ms_count": 24,
        "observations": 30
      },
      "0:scalar_indirect_calls_n1024:baseline": {
        "batch": 9,
        "estimated_total_ns": {
          "max": 14399768.997000001,
          "median": 11870985.0015,
          "min": 9720947.997000001,
          "p95": 13996967.004
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "0:scalar_indirect_calls_n1024:candidate": {
        "batch": 9,
        "estimated_total_ns": {
          "max": 27203814.999,
          "median": 11472012.0,
          "min": 9649733.004,
          "p95": 23832954.0
        },
        "estimated_under_20ms_count": 28,
        "observations": 30
      },
      "0:scalar_indirect_calls_n17:baseline": {
        "batch": 1538,
        "estimated_total_ns": {
          "max": 466854273.996,
          "median": 60955081.834,
          "min": 39143384.23,
          "p95": 324945165.658
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_indirect_calls_n17:candidate": {
        "batch": 1538,
        "estimated_total_ns": {
          "max": 485366532.49799997,
          "median": 58214707.269999996,
          "min": 28113878.69,
          "p95": 147244679.49400002
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_indirect_calls_n1:baseline": {
        "batch": 12747,
        "estimated_total_ns": {
          "max": 61576958.394,
          "median": 32000820.873000003,
          "min": 26678030.589,
          "p95": 60607714.755
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_indirect_calls_n1:candidate": {
        "batch": 12747,
        "estimated_total_ns": {
          "max": 43288659.036,
          "median": 31399551.2565,
          "min": 26264581.644,
          "p95": 36321314.547
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_indirect_calls_n32:baseline": {
        "batch": 264,
        "estimated_total_ns": {
          "max": 16308285.048,
          "median": 11814242.484000001,
          "min": 10215598.008,
          "p95": 15011533.944
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "0:scalar_indirect_calls_n32:candidate": {
        "batch": 264,
        "estimated_total_ns": {
          "max": 19534455.072,
          "median": 12003150.588,
          "min": 9630084.024,
          "p95": 14411063.040000001
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "0:scalar_multivalue_branch_n0:baseline": {
        "batch": 21906,
        "estimated_total_ns": {
          "max": 37431746.063999996,
          "median": 23456824.317,
          "min": 19861118.712,
          "p95": 34990300.458
        },
        "estimated_under_20ms_count": 3,
        "observations": 30
      },
      "0:scalar_multivalue_branch_n0:candidate": {
        "batch": 21906,
        "estimated_total_ns": {
          "max": 89202108.24,
          "median": 23326209.792,
          "min": 18839948.616,
          "p95": 40647459.24
        },
        "estimated_under_20ms_count": 3,
        "observations": 30
      },
      "0:scalar_multivalue_branch_n10000:baseline": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 80896994.0,
          "median": 33394985.5,
          "min": 29277365.0,
          "p95": 47142891.0
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_multivalue_branch_n10000:candidate": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 60536239.0,
          "median": 33040465.0,
          "min": 27972534.0,
          "p95": 43935781.0
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_multivalue_branch_n1024:baseline": {
        "batch": 9,
        "estimated_total_ns": {
          "max": 35220396.996,
          "median": 18254613.501000002,
          "min": 14046958.998,
          "p95": 34770977.001
        },
        "estimated_under_20ms_count": 19,
        "observations": 30
      },
      "0:scalar_multivalue_branch_n1024:candidate": {
        "batch": 9,
        "estimated_total_ns": {
          "max": 37580409.999,
          "median": 16409686.000500001,
          "min": 12781823.004,
          "p95": 32875893.0
        },
        "estimated_under_20ms_count": 21,
        "observations": 30
      },
      "0:scalar_multivalue_branch_n17:baseline": {
        "batch": 923,
        "estimated_total_ns": {
          "max": 41716179.362,
          "median": 32236152.507,
          "min": 26507072.123999998,
          "p95": 39582163.673
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_multivalue_branch_n17:candidate": {
        "batch": 923,
        "estimated_total_ns": {
          "max": 85051269.34200001,
          "median": 31288661.625,
          "min": 25747286.214,
          "p95": 47209975.046000004
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_multivalue_branch_n1:baseline": {
        "batch": 11568,
        "estimated_total_ns": {
          "max": 46595279.328,
          "median": 29806155.072,
          "min": 25998698.256,
          "p95": 40493078.352
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_multivalue_branch_n1:candidate": {
        "batch": 11568,
        "estimated_total_ns": {
          "max": 47505715.63200001,
          "median": 29967956.688,
          "min": 24730625.664,
          "p95": 36422218.176
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_multivalue_branch_n32:baseline": {
        "batch": 672,
        "estimated_total_ns": {
          "max": 61842820.703999996,
          "median": 39000615.024000004,
          "min": 32046626.304,
          "p95": 51696451.296000004
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_multivalue_branch_n32:candidate": {
        "batch": 672,
        "estimated_total_ns": {
          "max": 61873662.81600001,
          "median": 39118356.480000004,
          "min": 32066481.888,
          "p95": 53654684.832
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_ordinary_recursion_n0:baseline": {
        "batch": 8719,
        "estimated_total_ns": {
          "max": 32433563.968,
          "median": 10395572.1505,
          "min": 8362262.115,
          "p95": 29747335.976999998
        },
        "estimated_under_20ms_count": 25,
        "observations": 30
      },
      "0:scalar_ordinary_recursion_n0:candidate": {
        "batch": 8719,
        "estimated_total_ns": {
          "max": 38195323.3,
          "median": 11261351.412500001,
          "min": 9351467.541,
          "p95": 20903192.169999998
        },
        "estimated_under_20ms_count": 28,
        "observations": 30
      },
      "0:scalar_ordinary_recursion_n17:baseline": {
        "batch": 1835,
        "estimated_total_ns": {
          "max": 109892444.985,
          "median": 43883663.504999995,
          "min": 39002855.269999996,
          "p95": 62748199.339999996
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_ordinary_recursion_n17:candidate": {
        "batch": 1835,
        "estimated_total_ns": {
          "max": 102447336.18499999,
          "median": 43725327.7775,
          "min": 37850027.53,
          "p95": 71142935.32
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_ordinary_recursion_n1:baseline": {
        "batch": 14761,
        "estimated_total_ns": {
          "max": 57932895.247,
          "median": 35551329.7235,
          "min": 28545353.196,
          "p95": 52223148.554000005
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_ordinary_recursion_n1:candidate": {
        "batch": 14761,
        "estimated_total_ns": {
          "max": 63581103.331,
          "median": 36846932.2155,
          "min": 30128573.773000002,
          "p95": 59270138.52
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_ordinary_recursion_n32:baseline": {
        "batch": 874,
        "estimated_total_ns": {
          "max": 76605503.058,
          "median": 38994827.992,
          "min": 32194011.996,
          "p95": 64530944.84199999
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_ordinary_recursion_n32:candidate": {
        "batch": 874,
        "estimated_total_ns": {
          "max": 71654661.012,
          "median": 39752747.245000005,
          "min": 33689032.695999995,
          "p95": 58533828.655999996
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_table_mutation_n0:baseline": {
        "batch": 22003,
        "estimated_total_ns": {
          "max": 356824763.288,
          "median": 55763677.100999996,
          "min": 37792506.821,
          "p95": 273702621.996
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_table_mutation_n0:candidate": {
        "batch": 22003,
        "estimated_total_ns": {
          "max": 131944487.977,
          "median": 53861528.7525,
          "min": 30376263.652999997,
          "p95": 93011455.651
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_table_mutation_n10000:baseline": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 97343632.0,
          "median": 26963768.5,
          "min": 22015037.0,
          "p95": 63363403.0
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_table_mutation_n10000:candidate": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 54969969.0,
          "median": 24871008.5,
          "min": 20271623.0,
          "p95": 54530624.0
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_table_mutation_n1024:baseline": {
        "batch": 14,
        "estimated_total_ns": {
          "max": 28996942.996,
          "median": 19951910.503,
          "min": 17039794.002,
          "p95": 26854298.996
        },
        "estimated_under_20ms_count": 15,
        "observations": 30
      },
      "0:scalar_table_mutation_n1024:candidate": {
        "batch": 14,
        "estimated_total_ns": {
          "max": 32881130.002000004,
          "median": 20514060.504,
          "min": 16092211.001999998,
          "p95": 31190061.994
        },
        "estimated_under_20ms_count": 13,
        "observations": 30
      },
      "0:scalar_table_mutation_n17:baseline": {
        "batch": 1734,
        "estimated_total_ns": {
          "max": 202463279.22,
          "median": 85652250.441,
          "min": 38270550.449999996,
          "p95": 137663848.34399998
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_table_mutation_n17:candidate": {
        "batch": 1734,
        "estimated_total_ns": {
          "max": 121256478.51,
          "median": 80276891.19,
          "min": 35944073.861999996,
          "p95": 119243673.852
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_table_mutation_n1:baseline": {
        "batch": 13784,
        "estimated_total_ns": {
          "max": 122844703.432,
          "median": 33572503.375999995,
          "min": 28731920.96,
          "p95": 88029172.72
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_table_mutation_n1:candidate": {
        "batch": 13784,
        "estimated_total_ns": {
          "max": 57562659.416,
          "median": 32868933.556,
          "min": 29305611.04,
          "p95": 57464558.68799999
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_table_mutation_n32:baseline": {
        "batch": 951,
        "estimated_total_ns": {
          "max": 58582865.780999996,
          "median": 39976300.192499995,
          "min": 33497026.401,
          "p95": 49149989.028
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_table_mutation_n32:candidate": {
        "batch": 951,
        "estimated_total_ns": {
          "max": 62032786.608,
          "median": 39402643.203,
          "min": 33331708.365,
          "p95": 52622533.194
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_tail_calls_n0:baseline": {
        "batch": 8651,
        "estimated_total_ns": {
          "max": 23996585.001000002,
          "median": 10597660.9965,
          "min": 8932892.835,
          "p95": 17863614.268999998
        },
        "estimated_under_20ms_count": 29,
        "observations": 30
      },
      "0:scalar_tail_calls_n0:candidate": {
        "batch": 8651,
        "estimated_total_ns": {
          "max": 22073597.466,
          "median": 10202405.4575,
          "min": 8097405.208000001,
          "p95": 21946064.424000002
        },
        "estimated_under_20ms_count": 28,
        "observations": 30
      },
      "0:scalar_tail_calls_n10000:baseline": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 44171558.0,
          "median": 20936043.0,
          "min": 16033083.0,
          "p95": 39479144.0
        },
        "estimated_under_20ms_count": 14,
        "observations": 30
      },
      "0:scalar_tail_calls_n10000:candidate": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 42900492.0,
          "median": 19737756.0,
          "min": 15590245.0,
          "p95": 42277560.0
        },
        "estimated_under_20ms_count": 16,
        "observations": 30
      },
      "0:scalar_tail_calls_n1024:baseline": {
        "batch": 17,
        "estimated_total_ns": {
          "max": 37975905.005,
          "median": 18038208.997500002,
          "min": 14097228.002,
          "p95": 29567362.999
        },
        "estimated_under_20ms_count": 19,
        "observations": 30
      },
      "0:scalar_tail_calls_n1024:candidate": {
        "batch": 17,
        "estimated_total_ns": {
          "max": 29166966.997,
          "median": 16434802.504500002,
          "min": 13191668.992999999,
          "p95": 28002374.007
        },
        "estimated_under_20ms_count": 26,
        "observations": 30
      },
      "0:scalar_tail_calls_n17:baseline": {
        "batch": 2008,
        "estimated_total_ns": {
          "max": 105070808.8,
          "median": 35065672.876,
          "min": 30099456.152,
          "p95": 83754633.8
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_tail_calls_n17:candidate": {
        "batch": 2008,
        "estimated_total_ns": {
          "max": 112286685.31199999,
          "median": 34399924.492,
          "min": 27773892.96,
          "p95": 86247473.432
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_tail_calls_n1:baseline": {
        "batch": 5531,
        "estimated_total_ns": {
          "max": 118702162.688,
          "median": 12919661.0185,
          "min": 9605892.347000001,
          "p95": 32932370.464
        },
        "estimated_under_20ms_count": 26,
        "observations": 30
      },
      "0:scalar_tail_calls_n1:candidate": {
        "batch": 5531,
        "estimated_total_ns": {
          "max": 106693498.852,
          "median": 10695977.7785,
          "min": 8914445.444,
          "p95": 20376215.062
        },
        "estimated_under_20ms_count": 28,
        "observations": 30
      },
      "0:scalar_tail_calls_n32:baseline": {
        "batch": 1052,
        "estimated_total_ns": {
          "max": 44295550.923999995,
          "median": 34338452.453999996,
          "min": 27607681.236,
          "p95": 41434811.556
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_tail_calls_n32:candidate": {
        "batch": 1052,
        "estimated_total_ns": {
          "max": 46339084.068,
          "median": 32076026.514,
          "min": 26045781.044000003,
          "p95": 40702754.212
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_try_table_normal_n0:baseline": {
        "batch": 16143,
        "estimated_total_ns": {
          "max": 23397938.630999997,
          "median": 16814984.661,
          "min": 13695301.482,
          "p95": 22928145.045
        },
        "estimated_under_20ms_count": 23,
        "observations": 30
      },
      "0:scalar_try_table_normal_n0:candidate": {
        "batch": 16143,
        "estimated_total_ns": {
          "max": 28043103.023999996,
          "median": 16407164.052,
          "min": 12790567.046999998,
          "p95": 24236034.762000002
        },
        "estimated_under_20ms_count": 23,
        "observations": 30
      },
      "0:scalar_try_table_normal_n10000:baseline": {
        "batch": 3,
        "estimated_total_ns": {
          "max": 70354689.0,
          "median": 32358345.0,
          "min": 27948933.0,
          "p95": 43357580.001
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_try_table_normal_n10000:candidate": {
        "batch": 3,
        "estimated_total_ns": {
          "max": 44675357.001,
          "median": 30781666.9995,
          "min": 27440766.0,
          "p95": 38125356.0
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_try_table_normal_n1024:baseline": {
        "batch": 9,
        "estimated_total_ns": {
          "max": 24324059.997,
          "median": 10915124.9985,
          "min": 7566148.998,
          "p95": 24191891.001
        },
        "estimated_under_20ms_count": 25,
        "observations": 30
      },
      "0:scalar_try_table_normal_n1024:candidate": {
        "batch": 9,
        "estimated_total_ns": {
          "max": 39726578.996999994,
          "median": 9847818.499499999,
          "min": 7144335.0,
          "p95": 24441359.004
        },
        "estimated_under_20ms_count": 27,
        "observations": 30
      },
      "0:scalar_try_table_normal_n17:baseline": {
        "batch": 1599,
        "estimated_total_ns": {
          "max": 85043518.56,
          "median": 57773844.765,
          "min": 33772563.747,
          "p95": 77817877.41
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_try_table_normal_n17:candidate": {
        "batch": 1599,
        "estimated_total_ns": {
          "max": 114520770.156,
          "median": 55503284.7525,
          "min": 29940836.873999998,
          "p95": 78056884.737
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_try_table_normal_n1:baseline": {
        "batch": 14685,
        "estimated_total_ns": {
          "max": 102148067.01,
          "median": 49018794.33,
          "min": 28491293.655,
          "p95": 81022137.9
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_try_table_normal_n1:candidate": {
        "batch": 14685,
        "estimated_total_ns": {
          "max": 156491745.135,
          "median": 49416970.7625,
          "min": 25186126.020000003,
          "p95": 79934640.225
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_try_table_normal_n32:baseline": {
        "batch": 1013,
        "estimated_total_ns": {
          "max": 84191545.313,
          "median": 40222917.9965,
          "min": 31752094.995,
          "p95": 83884306.46499999
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "0:scalar_try_table_normal_n32:candidate": {
        "batch": 1013,
        "estimated_total_ns": {
          "max": 95049501.295,
          "median": 41715383.559,
          "min": 29222895.349,
          "p95": 78378983.729
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:branch_calls:baseline": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 86032294.0,
          "median": 25569292.0,
          "min": 13735892.0,
          "p95": 50436930.0
        },
        "estimated_under_20ms_count": 10,
        "observations": 30
      },
      "1:branch_calls:candidate": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 97913281.0,
          "median": 24363500.5,
          "min": 14395013.0,
          "p95": 63426426.0
        },
        "estimated_under_20ms_count": 9,
        "observations": 30
      },
      "1:call_overhead:baseline": {
        "batch": 42919,
        "estimated_total_ns": {
          "max": 44618463.643,
          "median": 18175402.498499997,
          "min": 15234356.564000001,
          "p95": 34117600.669999994
        },
        "estimated_under_20ms_count": 24,
        "observations": 30
      },
      "1:call_overhead:candidate": {
        "batch": 42919,
        "estimated_total_ns": {
          "max": 29548358.092,
          "median": 18052525.4015,
          "min": 15153282.573,
          "p95": 23453774.254
        },
        "estimated_under_20ms_count": 26,
        "observations": 30
      },
      "1:integer_loop:baseline": {
        "batch": 3,
        "estimated_total_ns": {
          "max": 57951182.001,
          "median": 29222270.001,
          "min": 25238481.999,
          "p95": 43686701.001
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:integer_loop:candidate": {
        "batch": 3,
        "estimated_total_ns": {
          "max": 43455459.999,
          "median": 27723346.9995,
          "min": 23366682.998999998,
          "p95": 34344698.001
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_accumulator_n0:baseline": {
        "batch": 8033,
        "estimated_total_ns": {
          "max": 13088327.559999999,
          "median": 7007113.603,
          "min": 5496813.207,
          "p95": 9929936.719
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "1:scalar_accumulator_n0:candidate": {
        "batch": 8033,
        "estimated_total_ns": {
          "max": 10455672.469999999,
          "median": 6750226.296,
          "min": 5363023.592,
          "p95": 9822808.631000001
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "1:scalar_accumulator_n10000:baseline": {
        "batch": 3,
        "estimated_total_ns": {
          "max": 46939287.999,
          "median": 23935408.9995,
          "min": 20217704.001000002,
          "p95": 45021677.001
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_accumulator_n10000:candidate": {
        "batch": 3,
        "estimated_total_ns": {
          "max": 59541168.999,
          "median": 23352973.5,
          "min": 19796213.001000002,
          "p95": 46491048.999
        },
        "estimated_under_20ms_count": 1,
        "observations": 30
      },
      "1:scalar_accumulator_n1024:baseline": {
        "batch": 13,
        "estimated_total_ns": {
          "max": 30669946.996,
          "median": 15249014.501500001,
          "min": 10605098.998,
          "p95": 22851292.997
        },
        "estimated_under_20ms_count": 27,
        "observations": 30
      },
      "1:scalar_accumulator_n1024:candidate": {
        "batch": 13,
        "estimated_total_ns": {
          "max": 24181409.005,
          "median": 14045281.002999999,
          "min": 10343948.003999999,
          "p95": 23730962.996
        },
        "estimated_under_20ms_count": 28,
        "observations": 30
      },
      "1:scalar_accumulator_n17:baseline": {
        "batch": 1788,
        "estimated_total_ns": {
          "max": 41075884.92,
          "median": 32544481.362000003,
          "min": 22621030.404,
          "p95": 40662368.796
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_accumulator_n17:candidate": {
        "batch": 1788,
        "estimated_total_ns": {
          "max": 89695227.408,
          "median": 28266774.504,
          "min": 23857763.184,
          "p95": 65030266.26
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_accumulator_n1:baseline": {
        "batch": 5321,
        "estimated_total_ns": {
          "max": 165345977.82999998,
          "median": 12842257.4445,
          "min": 8827836.976,
          "p95": 24173116.765
        },
        "estimated_under_20ms_count": 25,
        "observations": 30
      },
      "1:scalar_accumulator_n1:candidate": {
        "batch": 5321,
        "estimated_total_ns": {
          "max": 246678516.388,
          "median": 12878349.787499998,
          "min": 8794096.514999999,
          "p95": 22372453.118
        },
        "estimated_under_20ms_count": 27,
        "observations": 30
      },
      "1:scalar_accumulator_n32:baseline": {
        "batch": 659,
        "estimated_total_ns": {
          "max": 75216304.747,
          "median": 32107997.018,
          "min": 17784798.086,
          "p95": 56540116.242
        },
        "estimated_under_20ms_count": 6,
        "observations": 30
      },
      "1:scalar_accumulator_n32:candidate": {
        "batch": 659,
        "estimated_total_ns": {
          "max": 77302607.805,
          "median": 32206421.963,
          "min": 16259178.159,
          "p95": 71151806.92199999
        },
        "estimated_under_20ms_count": 4,
        "observations": 30
      },
      "1:scalar_br_table_n0:baseline": {
        "batch": 27211,
        "estimated_total_ns": {
          "max": 75811070.495,
          "median": 29734221.608,
          "min": 24041789.252,
          "p95": 49444618.302
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_br_table_n0:candidate": {
        "batch": 27211,
        "estimated_total_ns": {
          "max": 43863261.248,
          "median": 30466034.242,
          "min": 23154547.386,
          "p95": 43708947.667
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_br_table_n10000:baseline": {
        "batch": 1,
        "estimated_total_ns": {
          "max": 62736277.0,
          "median": 21869258.5,
          "min": 10974980.0,
          "p95": 56628808.0
        },
        "estimated_under_20ms_count": 13,
        "observations": 30
      },
      "1:scalar_br_table_n10000:candidate": {
        "batch": 1,
        "estimated_total_ns": {
          "max": 159483621.0,
          "median": 19859334.0,
          "min": 9983464.0,
          "p95": 68204599.0
        },
        "estimated_under_20ms_count": 15,
        "observations": 30
      },
      "1:scalar_br_table_n1024:baseline": {
        "batch": 15,
        "estimated_total_ns": {
          "max": 66307988.99999999,
          "median": 36637510.5,
          "min": 21489408.0,
          "p95": 64477861.995000005
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_br_table_n1024:candidate": {
        "batch": 15,
        "estimated_total_ns": {
          "max": 503919504.0,
          "median": 36917226.9975,
          "min": 21525615.0,
          "p95": 57443950.995
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_br_table_n17:baseline": {
        "batch": 1155,
        "estimated_total_ns": {
          "max": 51586928.085,
          "median": 36112676.985,
          "min": 28900951.695,
          "p95": 49847278.635000005
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_br_table_n17:candidate": {
        "batch": 1155,
        "estimated_total_ns": {
          "max": 58726202.535,
          "median": 36329827.38,
          "min": 29114699.46,
          "p95": 57819758.535
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_br_table_n1:baseline": {
        "batch": 13361,
        "estimated_total_ns": {
          "max": 116576942.92599998,
          "median": 42124079.804,
          "min": 28788626.036,
          "p95": 86942351.814
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_br_table_n1:candidate": {
        "batch": 13361,
        "estimated_total_ns": {
          "max": 83747135.46900001,
          "median": 44516948.057500005,
          "min": 26211035.277000003,
          "p95": 77812246.074
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_br_table_n32:baseline": {
        "batch": 752,
        "estimated_total_ns": {
          "max": 134277292.96,
          "median": 76897699.032,
          "min": 35552198.72,
          "p95": 129178814.928
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_br_table_n32:candidate": {
        "batch": 752,
        "estimated_total_ns": {
          "max": 157993356.848,
          "median": 84138997.68,
          "min": 33810784.800000004,
          "p95": 135459343.712
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_branch_entropy_n0:baseline": {
        "batch": 21552,
        "estimated_total_ns": {
          "max": 48770645.808,
          "median": 22922362.368,
          "min": 17911242.192,
          "p95": 46485724.32
        },
        "estimated_under_20ms_count": 11,
        "observations": 30
      },
      "1:scalar_branch_entropy_n0:candidate": {
        "batch": 21552,
        "estimated_total_ns": {
          "max": 52514810.111999996,
          "median": 20732625.288000003,
          "min": 17471236.56,
          "p95": 47970484.704
        },
        "estimated_under_20ms_count": 13,
        "observations": 30
      },
      "1:scalar_branch_entropy_n10000:baseline": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 107982202.0,
          "median": 38244139.0,
          "min": 24963247.0,
          "p95": 106604127.0
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_branch_entropy_n10000:candidate": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 84378920.0,
          "median": 35127804.5,
          "min": 24924562.0,
          "p95": 77298580.0
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_branch_entropy_n1024:baseline": {
        "batch": 5,
        "estimated_total_ns": {
          "max": 21592382.0,
          "median": 10131062.0,
          "min": 7191820.0,
          "p95": 16746926.0
        },
        "estimated_under_20ms_count": 29,
        "observations": 30
      },
      "1:scalar_branch_entropy_n1024:candidate": {
        "batch": 5,
        "estimated_total_ns": {
          "max": 18811337.0,
          "median": 9467459.0,
          "min": 7273158.0,
          "p95": 17469240.0
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "1:scalar_branch_entropy_n17:baseline": {
        "batch": 1368,
        "estimated_total_ns": {
          "max": 94941884.016,
          "median": 42047293.968,
          "min": 32582854.368,
          "p95": 88109376.552
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_branch_entropy_n17:candidate": {
        "batch": 1368,
        "estimated_total_ns": {
          "max": 114466491.64799999,
          "median": 45934255.980000004,
          "min": 35774595.36,
          "p95": 57205554.768
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_branch_entropy_n1:baseline": {
        "batch": 10289,
        "estimated_total_ns": {
          "max": 64543884.743999995,
          "median": 35408091.306,
          "min": 27800157.77,
          "p95": 52047832.51
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_branch_entropy_n1:candidate": {
        "batch": 10289,
        "estimated_total_ns": {
          "max": 92741516.873,
          "median": 32457278.129,
          "min": 26512242.483999997,
          "p95": 42826172.214
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_branch_entropy_n32:baseline": {
        "batch": 194,
        "estimated_total_ns": {
          "max": 19401758.028,
          "median": 11213808.966,
          "min": 8410244.932,
          "p95": 17545840.926000003
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "1:scalar_branch_entropy_n32:candidate": {
        "batch": 194,
        "estimated_total_ns": {
          "max": 18363651.029999997,
          "median": 11061887.469,
          "min": 8783647.014,
          "p95": 17844611.014000002
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "1:scalar_call_ref_n0:baseline": {
        "batch": 22832,
        "estimated_total_ns": {
          "max": 47199862.976,
          "median": 18864882.92,
          "min": 16533358.991999999,
          "p95": 24063489.584
        },
        "estimated_under_20ms_count": 20,
        "observations": 30
      },
      "1:scalar_call_ref_n0:candidate": {
        "batch": 22832,
        "estimated_total_ns": {
          "max": 65677206.944,
          "median": 19266965.856,
          "min": 16606010.416,
          "p95": 48159925.744
        },
        "estimated_under_20ms_count": 18,
        "observations": 30
      },
      "1:scalar_call_ref_n10000:baseline": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 39688197.0,
          "median": 22062401.0,
          "min": 19707797.0,
          "p95": 26296774.0
        },
        "estimated_under_20ms_count": 1,
        "observations": 30
      },
      "1:scalar_call_ref_n10000:candidate": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 25974711.0,
          "median": 21272060.5,
          "min": 18890629.0,
          "p95": 25915500.0
        },
        "estimated_under_20ms_count": 9,
        "observations": 30
      },
      "1:scalar_call_ref_n1024:baseline": {
        "batch": 13,
        "estimated_total_ns": {
          "max": 30329654.004,
          "median": 16753081.501,
          "min": 13777584.002,
          "p95": 25753602.004
        },
        "estimated_under_20ms_count": 23,
        "observations": 30
      },
      "1:scalar_call_ref_n1024:candidate": {
        "batch": 13,
        "estimated_total_ns": {
          "max": 41032548.999,
          "median": 16018470.500500001,
          "min": 13480225.005,
          "p95": 35698821.002000004
        },
        "estimated_under_20ms_count": 25,
        "observations": 30
      },
      "1:scalar_call_ref_n17:baseline": {
        "batch": 1502,
        "estimated_total_ns": {
          "max": 52053550.817999996,
          "median": 33359204.463,
          "min": 29550831.644,
          "p95": 38288384.702
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_call_ref_n17:candidate": {
        "batch": 1502,
        "estimated_total_ns": {
          "max": 51085639.488,
          "median": 33592252.53,
          "min": 28006170.338000003,
          "p95": 48564397.308
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_call_ref_n1:baseline": {
        "batch": 13532,
        "estimated_total_ns": {
          "max": 37106029.54,
          "median": 30182983.913999997,
          "min": 26856581.504,
          "p95": 36592354.82
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_call_ref_n1:candidate": {
        "batch": 13532,
        "estimated_total_ns": {
          "max": 86933005.128,
          "median": 29235277.06,
          "min": 23812287.464,
          "p95": 40340001.624
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_call_ref_n32:baseline": {
        "batch": 842,
        "estimated_total_ns": {
          "max": 50150441.147999994,
          "median": 35841331.905,
          "min": 30918481.653999995,
          "p95": 42983970.331999995
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_call_ref_n32:candidate": {
        "batch": 842,
        "estimated_total_ns": {
          "max": 58147251.106,
          "median": 35510891.531,
          "min": 31476331.914,
          "p95": 55287226.33800001
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_dependent_n0:baseline": {
        "batch": 23475,
        "estimated_total_ns": {
          "max": 566199863.25,
          "median": 32996683.0125,
          "min": 25415537.4,
          "p95": 179121926.32500002
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_dependent_n0:candidate": {
        "batch": 23475,
        "estimated_total_ns": {
          "max": 61730611.2,
          "median": 32365463.7375,
          "min": 24964254.0,
          "p95": 57887096.4
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_dependent_n10000:baseline": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 65118027.0,
          "median": 25491014.5,
          "min": 17750497.0,
          "p95": 42975419.0
        },
        "estimated_under_20ms_count": 2,
        "observations": 30
      },
      "1:scalar_dependent_n10000:candidate": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 32781701.0,
          "median": 25184384.5,
          "min": 17641965.0,
          "p95": 30943280.0
        },
        "estimated_under_20ms_count": 4,
        "observations": 30
      },
      "1:scalar_dependent_n1024:baseline": {
        "batch": 11,
        "estimated_total_ns": {
          "max": 24308358.998,
          "median": 13684495.0,
          "min": 11010213.004999999,
          "p95": 24249474.997
        },
        "estimated_under_20ms_count": 26,
        "observations": 30
      },
      "1:scalar_dependent_n1024:candidate": {
        "batch": 11,
        "estimated_total_ns": {
          "max": 51405101.0,
          "median": 14137203.498,
          "min": 10456910.002,
          "p95": 28249778.997
        },
        "estimated_under_20ms_count": 26,
        "observations": 30
      },
      "1:scalar_dependent_n17:baseline": {
        "batch": 2366,
        "estimated_total_ns": {
          "max": 158707087.448,
          "median": 59320514.071,
          "min": 35876450.61,
          "p95": 120430299.08
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_dependent_n17:candidate": {
        "batch": 2366,
        "estimated_total_ns": {
          "max": 175251952.87600002,
          "median": 53511822.0,
          "min": 37528450.96,
          "p95": 151536188.622
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_dependent_n1:baseline": {
        "batch": 5194,
        "estimated_total_ns": {
          "max": 27101455.766,
          "median": 12381189.709,
          "min": 9033571.008,
          "p95": 23112811.764
        },
        "estimated_under_20ms_count": 27,
        "observations": 30
      },
      "1:scalar_dependent_n1:candidate": {
        "batch": 5194,
        "estimated_total_ns": {
          "max": 24429895.896,
          "median": 12648395.039,
          "min": 9338209.496,
          "p95": 22274905.296
        },
        "estimated_under_20ms_count": 23,
        "observations": 30
      },
      "1:scalar_dependent_n32:baseline": {
        "batch": 518,
        "estimated_total_ns": {
          "max": 44222775.254,
          "median": 19161997.931999996,
          "min": 15256253.068,
          "p95": 27387305.946000002
        },
        "estimated_under_20ms_count": 19,
        "observations": 30
      },
      "1:scalar_dependent_n32:candidate": {
        "batch": 518,
        "estimated_total_ns": {
          "max": 28662490.892,
          "median": 18720050.950999998,
          "min": 14983764.866,
          "p95": 27625580.248
        },
        "estimated_under_20ms_count": 19,
        "observations": 30
      },
      "1:scalar_direct_calls_n0:baseline": {
        "batch": 434,
        "estimated_total_ns": {
          "max": 1558122.93,
          "median": 425891.578,
          "min": 216138.94400000002,
          "p95": 1090680.192
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "1:scalar_direct_calls_n0:candidate": {
        "batch": 434,
        "estimated_total_ns": {
          "max": 1711169.124,
          "median": 347782.428,
          "min": 211443.064,
          "p95": 1525456.1840000001
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "1:scalar_direct_calls_n10000:baseline": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 31745989.0,
          "median": 20227579.0,
          "min": 16923390.0,
          "p95": 29441671.0
        },
        "estimated_under_20ms_count": 14,
        "observations": 30
      },
      "1:scalar_direct_calls_n10000:candidate": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 43704121.0,
          "median": 19692394.0,
          "min": 16555315.0,
          "p95": 30380003.0
        },
        "estimated_under_20ms_count": 16,
        "observations": 30
      },
      "1:scalar_direct_calls_n1024:baseline": {
        "batch": 16,
        "estimated_total_ns": {
          "max": 244542072.0,
          "median": 47734642.992,
          "min": 17895776.992,
          "p95": 79533756.0
        },
        "estimated_under_20ms_count": 1,
        "observations": 30
      },
      "1:scalar_direct_calls_n1024:candidate": {
        "batch": 16,
        "estimated_total_ns": {
          "max": 216759602.0,
          "median": 48295607.0,
          "min": 18649438.0,
          "p95": 72784732.992
        },
        "estimated_under_20ms_count": 2,
        "observations": 30
      },
      "1:scalar_direct_calls_n17:baseline": {
        "batch": 1408,
        "estimated_total_ns": {
          "max": 58658378.239999995,
          "median": 28191601.856000002,
          "min": 23778447.615999997,
          "p95": 36198774.656
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_direct_calls_n17:candidate": {
        "batch": 1408,
        "estimated_total_ns": {
          "max": 46137722.752000004,
          "median": 28840157.632,
          "min": 22722671.488,
          "p95": 39297771.392
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_direct_calls_n1:baseline": {
        "batch": 13184,
        "estimated_total_ns": {
          "max": 162522846.336,
          "median": 59360979.77600001,
          "min": 26757600.384,
          "p95": 122376933.504
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_direct_calls_n1:candidate": {
        "batch": 13184,
        "estimated_total_ns": {
          "max": 107344510.33600001,
          "median": 58765511.23199999,
          "min": 28650532.735999998,
          "p95": 102909294.08
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_direct_calls_n32:baseline": {
        "batch": 932,
        "estimated_total_ns": {
          "max": 62161833.272,
          "median": 41277028.79000001,
          "min": 35219263.188,
          "p95": 53375036.063999996
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_direct_calls_n32:candidate": {
        "batch": 932,
        "estimated_total_ns": {
          "max": 73958544.984,
          "median": 40379978.79000001,
          "min": 33573954.191999994,
          "p95": 66120496.748
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_dynamic_api_n0:baseline": {
        "batch": 38462,
        "estimated_total_ns": {
          "max": 36419052.408,
          "median": 18376143.588,
          "min": 15572456.098000001,
          "p95": 35661504.856
        },
        "estimated_under_20ms_count": 20,
        "observations": 30
      },
      "1:scalar_dynamic_api_n0:candidate": {
        "batch": 38462,
        "estimated_total_ns": {
          "max": 33530133.126000002,
          "median": 17375689.275,
          "min": 14729753.678,
          "p95": 23769746.772
        },
        "estimated_under_20ms_count": 25,
        "observations": 30
      },
      "1:scalar_dynamic_api_n10000:baseline": {
        "batch": 46729,
        "estimated_total_ns": {
          "max": 39178434.722,
          "median": 20366858.0145,
          "min": 17891365.875,
          "p95": 30728850.213
        },
        "estimated_under_20ms_count": 14,
        "observations": 30
      },
      "1:scalar_dynamic_api_n10000:candidate": {
        "batch": 46729,
        "estimated_total_ns": {
          "max": 24971697.226,
          "median": 19047745.0735,
          "min": 16142766.694999998,
          "p95": 24788285.901
        },
        "estimated_under_20ms_count": 21,
        "observations": 30
      },
      "1:scalar_dynamic_api_n1024:baseline": {
        "batch": 41153,
        "estimated_total_ns": {
          "max": 50331970.885000005,
          "median": 19915973.773500003,
          "min": 14740099.234,
          "p95": 36433944.337
        },
        "estimated_under_20ms_count": 16,
        "observations": 30
      },
      "1:scalar_dynamic_api_n1024:candidate": {
        "batch": 41153,
        "estimated_total_ns": {
          "max": 38876292.581,
          "median": 17607290.4735,
          "min": 14574458.409,
          "p95": 29977779.391
        },
        "estimated_under_20ms_count": 21,
        "observations": 30
      },
      "1:scalar_dynamic_api_n17:baseline": {
        "batch": 10701,
        "estimated_total_ns": {
          "max": 13944965.345999999,
          "median": 5696313.516000001,
          "min": 4411968.795,
          "p95": 9024656.247
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "1:scalar_dynamic_api_n17:candidate": {
        "batch": 10701,
        "estimated_total_ns": {
          "max": 19436879.061,
          "median": 5628426.372,
          "min": 4494088.269,
          "p95": 13528525.23
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "1:scalar_dynamic_api_n1:baseline": {
        "batch": 44346,
        "estimated_total_ns": {
          "max": 35097863.43,
          "median": 18626783.417999998,
          "min": 15449215.134000001,
          "p95": 30817853.586000003
        },
        "estimated_under_20ms_count": 21,
        "observations": 30
      },
      "1:scalar_dynamic_api_n1:candidate": {
        "batch": 44346,
        "estimated_total_ns": {
          "max": 33011206.746,
          "median": 19582506.237,
          "min": 15463716.276,
          "p95": 31315814.819999997
        },
        "estimated_under_20ms_count": 18,
        "observations": 30
      },
      "1:scalar_dynamic_api_n32:baseline": {
        "batch": 48193,
        "estimated_total_ns": {
          "max": 25357132.494,
          "median": 19448236.957000002,
          "min": 17050490.628,
          "p95": 22530516.658
        },
        "estimated_under_20ms_count": 20,
        "observations": 30
      },
      "1:scalar_dynamic_api_n32:candidate": {
        "batch": 48193,
        "estimated_total_ns": {
          "max": 35263540.995000005,
          "median": 18898475.3095,
          "min": 16488945.792,
          "p95": 22257069.576
        },
        "estimated_under_20ms_count": 20,
        "observations": 30
      },
      "1:scalar_host_call_n0:baseline": {
        "batch": 25840,
        "estimated_total_ns": {
          "max": 35516899.12,
          "median": 20073804.0,
          "min": 18010376.64,
          "p95": 25443304.32
        },
        "estimated_under_20ms_count": 15,
        "observations": 30
      },
      "1:scalar_host_call_n0:candidate": {
        "batch": 25840,
        "estimated_total_ns": {
          "max": 52628586.4,
          "median": 20893190.4,
          "min": 18252006.48,
          "p95": 31957154.88
        },
        "estimated_under_20ms_count": 11,
        "observations": 30
      },
      "1:scalar_host_call_n10000:baseline": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 43435555.0,
          "median": 20916455.0,
          "min": 16747720.0,
          "p95": 40018910.0
        },
        "estimated_under_20ms_count": 12,
        "observations": 30
      },
      "1:scalar_host_call_n10000:candidate": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 49022859.0,
          "median": 20666273.5,
          "min": 16433070.0,
          "p95": 46082795.0
        },
        "estimated_under_20ms_count": 13,
        "observations": 30
      },
      "1:scalar_host_call_n1024:baseline": {
        "batch": 22,
        "estimated_total_ns": {
          "max": 155053910.99,
          "median": 20827332.504,
          "min": 12958512.996,
          "p95": 119165437.006
        },
        "estimated_under_20ms_count": 13,
        "observations": 30
      },
      "1:scalar_host_call_n1024:candidate": {
        "batch": 22,
        "estimated_total_ns": {
          "max": 63141876.006,
          "median": 21378239.498000003,
          "min": 17522031.009999998,
          "p95": 59661637.992
        },
        "estimated_under_20ms_count": 10,
        "observations": 30
      },
      "1:scalar_host_call_n17:baseline": {
        "batch": 2043,
        "estimated_total_ns": {
          "max": 58839309.135,
          "median": 37381601.479499996,
          "min": 32073018.183,
          "p95": 47737214.019
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_host_call_n17:candidate": {
        "batch": 2043,
        "estimated_total_ns": {
          "max": 76200149.052,
          "median": 35336740.3065,
          "min": 32395661.001,
          "p95": 56015617.544999994
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_host_call_n1:baseline": {
        "batch": 12346,
        "estimated_total_ns": {
          "max": 33299273.165999997,
          "median": 24226389.129,
          "min": 19701413.458,
          "p95": 31668897.444000002
        },
        "estimated_under_20ms_count": 1,
        "observations": 30
      },
      "1:scalar_host_call_n1:candidate": {
        "batch": 12346,
        "estimated_total_ns": {
          "max": 38191141.092,
          "median": 23572927.695,
          "min": 20061570.97,
          "p95": 30934038.832000002
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_host_call_n32:baseline": {
        "batch": 1129,
        "estimated_total_ns": {
          "max": 58330554.978,
          "median": 40525326.294,
          "min": 30732678.35,
          "p95": 53809594.152
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_host_call_n32:candidate": {
        "batch": 1129,
        "estimated_total_ns": {
          "max": 72471960.765,
          "median": 37592960.4815,
          "min": 32210070.815,
          "p95": 63172998.554
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_indirect_calls_n0:baseline": {
        "batch": 20987,
        "estimated_total_ns": {
          "max": 27892478.532,
          "median": 19145841.9705,
          "min": 16793671.478,
          "p95": 25612052.099
        },
        "estimated_under_20ms_count": 17,
        "observations": 30
      },
      "1:scalar_indirect_calls_n0:candidate": {
        "batch": 20987,
        "estimated_total_ns": {
          "max": 25132331.253,
          "median": 19046835.798,
          "min": 16862424.89,
          "p95": 23707754.680000003
        },
        "estimated_under_20ms_count": 20,
        "observations": 30
      },
      "1:scalar_indirect_calls_n10000:baseline": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 40617047.0,
          "median": 27709196.0,
          "min": 22879310.0,
          "p95": 38804329.0
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_indirect_calls_n10000:candidate": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 52928545.0,
          "median": 25274760.5,
          "min": 21819799.0,
          "p95": 45994679.0
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_indirect_calls_n1024:baseline": {
        "batch": 15,
        "estimated_total_ns": {
          "max": 52805314.995000005,
          "median": 21040646.497500002,
          "min": 17103280.995,
          "p95": 28512079.005
        },
        "estimated_under_20ms_count": 12,
        "observations": 30
      },
      "1:scalar_indirect_calls_n1024:candidate": {
        "batch": 15,
        "estimated_total_ns": {
          "max": 29693257.005,
          "median": 20748581.497500002,
          "min": 17080633.005000003,
          "p95": 28263489.0
        },
        "estimated_under_20ms_count": 9,
        "observations": 30
      },
      "1:scalar_indirect_calls_n17:baseline": {
        "batch": 1543,
        "estimated_total_ns": {
          "max": 67080074.942999996,
          "median": 40306095.5575,
          "min": 33658156.436000004,
          "p95": 65488146.413
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_indirect_calls_n17:candidate": {
        "batch": 1543,
        "estimated_total_ns": {
          "max": 97196432.265,
          "median": 41897379.1135,
          "min": 31660051.672000002,
          "p95": 79413635.00500001
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_indirect_calls_n1:baseline": {
        "batch": 13107,
        "estimated_total_ns": {
          "max": 56292074.67,
          "median": 32624043.884999998,
          "min": 28092770.487,
          "p95": 38079924.383999996
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_indirect_calls_n1:candidate": {
        "batch": 13107,
        "estimated_total_ns": {
          "max": 53499313.752,
          "median": 31131608.776499998,
          "min": 26524137.834,
          "p95": 37077317.526
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_indirect_calls_n32:baseline": {
        "batch": 38,
        "estimated_total_ns": {
          "max": 4694665.008,
          "median": 2562498.004,
          "min": 1452582.9840000002,
          "p95": 3718090.012
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "1:scalar_indirect_calls_n32:candidate": {
        "batch": 38,
        "estimated_total_ns": {
          "max": 4546751.984,
          "median": 2591237.499,
          "min": 1433794.986,
          "p95": 3625753.014
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "1:scalar_multivalue_branch_n0:baseline": {
        "batch": 23256,
        "estimated_total_ns": {
          "max": 104451905.37599999,
          "median": 26561456.676,
          "min": 21334728.816,
          "p95": 85567614.768
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_multivalue_branch_n0:candidate": {
        "batch": 23256,
        "estimated_total_ns": {
          "max": 114599102.83199999,
          "median": 25694345.088,
          "min": 19916089.56,
          "p95": 70505308.224
        },
        "estimated_under_20ms_count": 1,
        "observations": 30
      },
      "1:scalar_multivalue_branch_n10000:baseline": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 45009404.0,
          "median": 34089415.0,
          "min": 27874314.0,
          "p95": 40865768.0
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_multivalue_branch_n10000:candidate": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 44122979.0,
          "median": 32595990.5,
          "min": 26873280.0,
          "p95": 42724278.0
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_multivalue_branch_n1024:baseline": {
        "batch": 9,
        "estimated_total_ns": {
          "max": 57035084.004,
          "median": 16071329.997,
          "min": 13386063.996,
          "p95": 25282179.999
        },
        "estimated_under_20ms_count": 25,
        "observations": 30
      },
      "1:scalar_multivalue_branch_n1024:candidate": {
        "batch": 9,
        "estimated_total_ns": {
          "max": 23963039.001,
          "median": 16372265.9985,
          "min": 13202490.996,
          "p95": 21849012.0
        },
        "estimated_under_20ms_count": 27,
        "observations": 30
      },
      "1:scalar_multivalue_branch_n17:baseline": {
        "batch": 1221,
        "estimated_total_ns": {
          "max": 127320713.949,
          "median": 40736421.412499994,
          "min": 34841160.519,
          "p95": 65495436.335999995
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_multivalue_branch_n17:candidate": {
        "batch": 1221,
        "estimated_total_ns": {
          "max": 417334874.48399997,
          "median": 40779170.454,
          "min": 33662171.466,
          "p95": 61196306.324999996
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_multivalue_branch_n1:baseline": {
        "batch": 10730,
        "estimated_total_ns": {
          "max": 92506892.36,
          "median": 32347328.625,
          "min": 26478581.950000003,
          "p95": 50000158.309999995
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_multivalue_branch_n1:candidate": {
        "batch": 10730,
        "estimated_total_ns": {
          "max": 64511946.809999995,
          "median": 32811841.055,
          "min": 26904337.619999997,
          "p95": 48436056.940000005
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_multivalue_branch_n32:baseline": {
        "batch": 150,
        "estimated_total_ns": {
          "max": 23056633.049999997,
          "median": 9057001.95,
          "min": 7427565.0,
          "p95": 17696143.05
        },
        "estimated_under_20ms_count": 29,
        "observations": 30
      },
      "1:scalar_multivalue_branch_n32:candidate": {
        "batch": 150,
        "estimated_total_ns": {
          "max": 15331854.0,
          "median": 9530594.55,
          "min": 7137652.05,
          "p95": 14599822.049999999
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "1:scalar_ordinary_recursion_n0:baseline": {
        "batch": 8204,
        "estimated_total_ns": {
          "max": 64936587.94,
          "median": 10863301.988000002,
          "min": 8550783.08,
          "p95": 22842897.643999998
        },
        "estimated_under_20ms_count": 27,
        "observations": 30
      },
      "1:scalar_ordinary_recursion_n0:candidate": {
        "batch": 8204,
        "estimated_total_ns": {
          "max": 22098048.28,
          "median": 11369644.664,
          "min": 8620262.756000001,
          "p95": 18758495.224
        },
        "estimated_under_20ms_count": 29,
        "observations": 30
      },
      "1:scalar_ordinary_recursion_n17:baseline": {
        "batch": 1858,
        "estimated_total_ns": {
          "max": 42566709.396,
          "median": 34642540.06,
          "min": 29900717.822,
          "p95": 39264480.422
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_ordinary_recursion_n17:candidate": {
        "batch": 1858,
        "estimated_total_ns": {
          "max": 64226164.169999994,
          "median": 36043193.36,
          "min": 30270225.714,
          "p95": 58045133.274
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_ordinary_recursion_n1:baseline": {
        "batch": 15385,
        "estimated_total_ns": {
          "max": 69593939.805,
          "median": 34936196.46,
          "min": 27407331.32,
          "p95": 53322840.730000004
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_ordinary_recursion_n1:candidate": {
        "batch": 15385,
        "estimated_total_ns": {
          "max": 53765005.629999995,
          "median": 33507737.6725,
          "min": 27034537.385,
          "p95": 46818062.73
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_ordinary_recursion_n32:baseline": {
        "batch": 887,
        "estimated_total_ns": {
          "max": 52009284.915,
          "median": 31391317.619000003,
          "min": 26049855.952,
          "p95": 42332818.306
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_ordinary_recursion_n32:candidate": {
        "batch": 887,
        "estimated_total_ns": {
          "max": 61891186.046,
          "median": 32090909.597999997,
          "min": 26720788.961,
          "p95": 39360146.019999996
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_table_mutation_n0:baseline": {
        "batch": 21692,
        "estimated_total_ns": {
          "max": 35867374.927999996,
          "median": 21654310.15,
          "min": 17852863.072,
          "p95": 33013748.944000002
        },
        "estimated_under_20ms_count": 10,
        "observations": 30
      },
      "1:scalar_table_mutation_n0:candidate": {
        "batch": 21692,
        "estimated_total_ns": {
          "max": 41032326.896,
          "median": 21269461.531999998,
          "min": 18403427.724,
          "p95": 40346274.012
        },
        "estimated_under_20ms_count": 9,
        "observations": 30
      },
      "1:scalar_table_mutation_n10000:baseline": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 29243838.0,
          "median": 23219577.5,
          "min": 18959843.0,
          "p95": 27441577.0
        },
        "estimated_under_20ms_count": 2,
        "observations": 30
      },
      "1:scalar_table_mutation_n10000:candidate": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 28390916.0,
          "median": 22901397.0,
          "min": 18414805.0,
          "p95": 26772194.0
        },
        "estimated_under_20ms_count": 6,
        "observations": 30
      },
      "1:scalar_table_mutation_n1024:baseline": {
        "batch": 20,
        "estimated_total_ns": {
          "max": 55557779.0,
          "median": 22573167.0,
          "min": 19302426.0,
          "p95": 39572996.0
        },
        "estimated_under_20ms_count": 1,
        "observations": 30
      },
      "1:scalar_table_mutation_n1024:candidate": {
        "batch": 20,
        "estimated_total_ns": {
          "max": 28078298.0,
          "median": 21646992.0,
          "min": 19306067.0,
          "p95": 27425746.0
        },
        "estimated_under_20ms_count": 5,
        "observations": 30
      },
      "1:scalar_table_mutation_n17:baseline": {
        "batch": 1712,
        "estimated_total_ns": {
          "max": 422429804.272,
          "median": 46625814.96,
          "min": 32317324.704,
          "p95": 289685429.648
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_table_mutation_n17:candidate": {
        "batch": 1712,
        "estimated_total_ns": {
          "max": 197562471.68,
          "median": 47345571.432,
          "min": 31647730.688,
          "p95": 134802799.53599998
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_table_mutation_n1:baseline": {
        "batch": 10742,
        "estimated_total_ns": {
          "max": 49953049.95200001,
          "median": 20706209.377,
          "min": 17648418.512,
          "p95": 44852221.994
        },
        "estimated_under_20ms_count": 9,
        "observations": 30
      },
      "1:scalar_table_mutation_n1:candidate": {
        "batch": 10742,
        "estimated_total_ns": {
          "max": 35290069.564,
          "median": 21069079.508,
          "min": 19072077.256,
          "p95": 34664079.514
        },
        "estimated_under_20ms_count": 7,
        "observations": 30
      },
      "1:scalar_table_mutation_n32:baseline": {
        "batch": 954,
        "estimated_total_ns": {
          "max": 36448987.643999994,
          "median": 30573437.111999996,
          "min": 26658746.766,
          "p95": 35053171.164000005
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_table_mutation_n32:candidate": {
        "batch": 954,
        "estimated_total_ns": {
          "max": 39264045.12,
          "median": 30915530.064000003,
          "min": 27604909.656,
          "p95": 36206564.796000004
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_tail_calls_n0:baseline": {
        "batch": 8917,
        "estimated_total_ns": {
          "max": 29727530.268,
          "median": 11524954.99,
          "min": 8863640.672,
          "p95": 20807623.326
        },
        "estimated_under_20ms_count": 27,
        "observations": 30
      },
      "1:scalar_tail_calls_n0:candidate": {
        "batch": 8917,
        "estimated_total_ns": {
          "max": 19615964.362999998,
          "median": 11360966.901500002,
          "min": 8853421.790000001,
          "p95": 18970890.748999998
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "1:scalar_tail_calls_n10000:baseline": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 50835397.0,
          "median": 24466625.5,
          "min": 14428667.0,
          "p95": 48971695.0
        },
        "estimated_under_20ms_count": 9,
        "observations": 30
      },
      "1:scalar_tail_calls_n10000:candidate": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 51819029.0,
          "median": 24812076.5,
          "min": 15431403.0,
          "p95": 40438974.0
        },
        "estimated_under_20ms_count": 4,
        "observations": 30
      },
      "1:scalar_tail_calls_n1024:baseline": {
        "batch": 38,
        "estimated_total_ns": {
          "max": 65020993.982,
          "median": 37461756.999,
          "min": 28088107.018,
          "p95": 62801900.002
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_tail_calls_n1024:candidate": {
        "batch": 38,
        "estimated_total_ns": {
          "max": 132127592.998,
          "median": 35405988.501,
          "min": 25784544.016,
          "p95": 58209964.992
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_tail_calls_n17:baseline": {
        "batch": 2007,
        "estimated_total_ns": {
          "max": 75841822.557,
          "median": 39774172.068,
          "min": 29541681.261,
          "p95": 69400478.484
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_tail_calls_n17:candidate": {
        "batch": 2007,
        "estimated_total_ns": {
          "max": 191750568.23700002,
          "median": 36993813.7545,
          "min": 28032064.029000003,
          "p95": 79501111.39799999
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_tail_calls_n1:baseline": {
        "batch": 6778,
        "estimated_total_ns": {
          "max": 20132679.843999997,
          "median": 11940731.431000002,
          "min": 10964696.042000001,
          "p95": 20048564.864
        },
        "estimated_under_20ms_count": 28,
        "observations": 30
      },
      "1:scalar_tail_calls_n1:candidate": {
        "batch": 6778,
        "estimated_total_ns": {
          "max": 17596386.134,
          "median": 11844544.833,
          "min": 10622793.388,
          "p95": 17044758.604
        },
        "estimated_under_20ms_count": 30,
        "observations": 30
      },
      "1:scalar_tail_calls_n32:baseline": {
        "batch": 879,
        "estimated_total_ns": {
          "max": 106238932.995,
          "median": 32789380.071000002,
          "min": 23042005.794,
          "p95": 63931927.272
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_tail_calls_n32:candidate": {
        "batch": 879,
        "estimated_total_ns": {
          "max": 64425797.816999994,
          "median": 30522484.779,
          "min": 22623934.935,
          "p95": 56833844.052
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_try_table_normal_n0:baseline": {
        "batch": 24243,
        "estimated_total_ns": {
          "max": 93674103.495,
          "median": 28608182.4585,
          "min": 16623109.941,
          "p95": 52332951.969
        },
        "estimated_under_20ms_count": 3,
        "observations": 30
      },
      "1:scalar_try_table_normal_n0:candidate": {
        "batch": 24243,
        "estimated_total_ns": {
          "max": 41310678.075,
          "median": 29074945.059,
          "min": 15596031.003,
          "p95": 40702954.551
        },
        "estimated_under_20ms_count": 5,
        "observations": 30
      },
      "1:scalar_try_table_normal_n10000:baseline": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 56697727.0,
          "median": 21568519.0,
          "min": 17822935.0,
          "p95": 50936929.0
        },
        "estimated_under_20ms_count": 9,
        "observations": 30
      },
      "1:scalar_try_table_normal_n10000:candidate": {
        "batch": 2,
        "estimated_total_ns": {
          "max": 66282759.0,
          "median": 20114325.5,
          "min": 16513652.0,
          "p95": 49689958.0
        },
        "estimated_under_20ms_count": 15,
        "observations": 30
      },
      "1:scalar_try_table_normal_n1024:baseline": {
        "batch": 6,
        "estimated_total_ns": {
          "max": 28878285.0,
          "median": 13742491.5,
          "min": 6624787.998000001,
          "p95": 23861128.002
        },
        "estimated_under_20ms_count": 27,
        "observations": 30
      },
      "1:scalar_try_table_normal_n1024:candidate": {
        "batch": 6,
        "estimated_total_ns": {
          "max": 86933121.0,
          "median": 13043592.501,
          "min": 5881758.0,
          "p95": 41226220.997999996
        },
        "estimated_under_20ms_count": 25,
        "observations": 30
      },
      "1:scalar_try_table_normal_n17:baseline": {
        "batch": 1903,
        "estimated_total_ns": {
          "max": 56097631.172000006,
          "median": 32202202.527000003,
          "min": 29450175.270999998,
          "p95": 47413197.425
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_try_table_normal_n17:candidate": {
        "batch": 1903,
        "estimated_total_ns": {
          "max": 50652927.424,
          "median": 33928790.621,
          "min": 27803836.687000003,
          "p95": 50169710.052
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_try_table_normal_n1:baseline": {
        "batch": 14379,
        "estimated_total_ns": {
          "max": 45379548.84,
          "median": 29427522.1875,
          "min": 26121854.792999998,
          "p95": 39370780.425
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_try_table_normal_n1:candidate": {
        "batch": 14379,
        "estimated_total_ns": {
          "max": 65671193.64,
          "median": 28732736.097000003,
          "min": 25389704.871,
          "p95": 38868191.238
        },
        "estimated_under_20ms_count": 0,
        "observations": 30
      },
      "1:scalar_try_table_normal_n32:baseline": {
        "batch": 495,
        "estimated_total_ns": {
          "max": 28120898.025,
          "median": 17893594.619999997,
          "min": 15973032.24,
          "p95": 25171315.784999996
        },
        "estimated_under_20ms_count": 22,
        "observations": 30
      },
      "1:scalar_try_table_normal_n32:candidate": {
        "batch": 495,
        "estimated_total_ns": {
          "max": 98482117.14,
          "median": 18800097.03,
          "min": 15700185.764999999,
          "p95": 30019473.0
        },
        "estimated_under_20ms_count": 18,
        "observations": 30
      }
    },
    "workload_count": 85
  },
  "outcome": "rejected",
  "raw_gate_decision": "rejected",
  "reason": "predeclared P028 strict incremental build gate rejected the candidate: release_seconds ratio exceeded 1.25",
  "schema": "wtime-p028-completion-v1",
  "status": "done",
  "strict_build_gate_passed": false,
  "task": "P028",
  "task_index_update_performed": false
}
```
