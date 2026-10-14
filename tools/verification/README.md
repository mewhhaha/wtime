# Shared verification workflow

Performance work follows the [fast iteration schedule](../../docs/verification/fast-performance.md):
exact development groups, frozen focused candidate gates, and complete milestone
portfolios. It retains source-bound semantics and immutable dataset reuse.

`tools/verify.py` runs a contract-driven workflow for task verification. Contracts declare component identities, phase commands, outputs, checks, and named test requirements. The runner keeps separate inputs and receipts for each phase, stores payloads by content identity, and can reuse a completed phase only when its declared inputs and helper sources still match.

## Start Rust and inspect a contract

From a fresh shell, activate the pinned Rust toolchain before running Rust commands:

```sh
. /workspace/.setup/rust-env.sh
cd /workspace/wtime
```

The repository pins Rust in `rust-toolchain.toml`. Use the setup script above for the required `cargo` and `rustc` environment. The task instructions live under `/workspace/tasks/performance`.

Create a task-specific JSON contract, then preflight it without running phase commands:

```sh
python3 -B tools/verify.py plan --contract docs/benchmarks/performance/P900-verification.json
```

The default artifact store is `target/verification`. Select another store with the global `--store PATH` option before the subcommand. `plan` checks contract structure, identities, command executables, output containment, and declared test feature flags; it does not execute the commands.

## Contract structure

The contract schema is version 1. Each named identity contains repository-relative `paths` (glob patterns are allowed) and optional `settings`. A phase lists the identities it consumes, any earlier phase dependencies, commands as argument arrays, outputs located under that phase's exclusive attempt directory, and JSON checks. Commands may use `{repo}`, `{stage}`, `{python}`, and dependency directories such as `{freeze_dir}` and `{measure_dir}`. A command's `expect_stdout` can pin an exact one-line result.

This compact template shows the actual field names and phase relationships. Replace its example task paths, commands, outputs, and required test names with the task's real producers and cases; the sample names do not claim that P900 exists.

```json
{
  "schema_version": 1,
  "task": "P900",
  "identities": {
    "predecessor": {
      "paths": ["target/performance/P899/before/manifest.json", "target/performance/P899/before/source/**/*.rs", "target/performance/P899/before/wtime-bench"],
      "settings": {"policy": "compact"}
    },
    "runtime": {
      "paths": ["crates/wtime-runtime/src/**/*.rs"],
      "settings": {"policy": "compact"}
    },
    "fixtures": {
      "paths": ["crates/wtime-runtime/tests/fixtures/performance/p900/**"],
      "settings": {"profile": "all-phase3-core"}
    },
    "harness": {
      "paths": ["tools/wtime-bench/Cargo.toml", "tools/wtime-bench/Cargo.lock", "tools/wtime-bench/src/**/*.rs", "tools/wtime-bench/workloads.json", "tools/wtime-bench/workloads.py", "tools/wtime-bench/representative_workloads.py", "tools/wtime-bench/compare.py", ".cargo/config", ".cargo/config.toml", "tools/wtime-bench/.cargo/config", "tools/wtime-bench/.cargo/config.toml"],
      "optional_paths": [".cargo/config", ".cargo/config.toml", "tools/wtime-bench/.cargo/config", "tools/wtime-bench/.cargo/config.toml"],
      "settings": {"pairs": 30}
    },
    "analysis": {
      "paths": ["tools/verification/statistics.py", "tools/wtime-bench/analyze.py", "tools/wtime-bench/gates.py", "tools/checks/build_budget.py", "tools/checks/dependency_budget.py"],
      "settings": {"decision": "paired-report-and-build-gates"}
    },
    "checker_tests": {
      "paths": ["tools/p900/tests/test_checker.py"]
    },
    "promotion_contract": {
      "paths": ["docs/benchmarks/performance/P900-contract.json"]
    }
  },
  "stages": {
    "freeze": {
      "uses": ["predecessor"],
      "commands": [
        {"name": "verify-predecessor", "argv": ["{python}", "-B", "{repo}/tools/p900/freeze_predecessor.py", "--manifest", "{repo}/target/performance/P899/before/manifest.json", "--source-dir", "{repo}/target/performance/P899/before/source", "--runtime-map-output", "{stage}/runtime.json", "--receipt-output", "{stage}/receipt.json"]}
      ],
      "outputs": {"runtime_map": "{stage}/runtime.json", "receipt": "{stage}/receipt.json"},
      "checks": [{"artifact": "receipt", "pointer": "/status", "op": "equal", "value": "passed"}]
    },
    "verify": {
      "uses": ["runtime", "fixtures"],
      "dependencies": ["freeze"],
      "commands": [
        {"name": "focused", "argv": ["cargo", "test", "--locked", "-p", "wtime-runtime", "--features", "compact-executor,execution-reference", "p900_", "--", "--nocapture"]},
        {"name": "source-check", "argv": ["{python}", "-B", "{repo}/tools/p900/check.py", "--report", "{stage}/check.json", "--build-report", "{stage}/build.json", "--optional-build-report", "{stage}/optional-build.json"]}
      ],
      "outputs": {"check": "{stage}/check.json", "build": "{stage}/build.json", "optional_build": "{stage}/optional-build.json"},
      "tests": [{"command": "focused", "required_names": ["p900_load_prefixes", "p900_store_effects", "p900_bounds_traps"], "minimum_passed": 3, "features": ["compact-executor", "execution-reference"]}],
      "checks": [
        {"artifact": "check", "pointer": "/full_fuel_prefixes_match", "op": "equal", "value": true},
        {"artifact": "check", "pointer": "/side_effect_images_match", "op": "equal", "value": true}
      ]
    },
    "measure": {
      "uses": ["runtime", "fixtures", "harness"],
      "dependencies": ["freeze", "verify"],
      "commands": [
        {"name": "paired-run", "argv": ["{python}", "-B", "{repo}/tools/wtime-bench/compare.py", "--baseline", "{repo}/target/performance/P899/before", "--candidate", "{repo}/tools/wtime-bench/target/release/wtime-bench", "--candidate-mode", "compact", "--selection", "p900", "--pairs", "30", "--output", "{stage}/comparison"]}
      ],
      "outputs": {"report": "{stage}/comparison/report.json"},
      "checks": [{"artifact": "report", "pointer": "/passed", "op": "equal", "value": true}, {"artifact": "report", "pointer": "/source_stable", "op": "equal", "value": true}]
    },
    "analyze": {
      "uses": ["analysis", "promotion_contract"],
      "dependencies": ["freeze", "verify", "measure"],
      "commands": [
        {"name": "statistics", "argv": ["{python}", "-B", "{repo}/tools/verify.py", "statistics", "--task", "P900", "--comparison", "{measure_dir}/comparison", "--build", "{verify_dir}/build.json", "--optional-build", "{verify_dir}/optional-build.json", "--contract", "{repo}/docs/benchmarks/performance/P900-contract.json", "--output", "{stage}/statistics.json", "--execute"]}
      ],
      "outputs": {"statistics": "{stage}/statistics.json"},
      "checks": [{"artifact": "statistics", "pointer": "/status", "op": "equal", "value": "passed"}]
    }
  },
  "source_delta": {
    "identity": "runtime",
    "baseline": "{freeze_dir}/runtime.json",
    "pointer": "",
    "allowed_changes": ["crates/wtime-runtime/src/execution.rs"],
    "allowed_additions": ["crates/wtime-runtime/src/execution/compact/*.rs"]
  },
  "completion": {
    "required_phases": ["freeze", "verify", "measure", "analyze"],
    "decision": {"phase": "analyze", "artifact": "statistics", "pointer": "/decision/decision"}
  }
}
```

For a real contract, ensure each output path is created by its declared producer and every phase consumes only identities it truly needs. The P900 commands and helper paths are placeholders; replace them with existing task-owned producers before `plan`, which rejects missing source files or executables. The freeze helper must verify the predeclared immutable predecessor and write its archived runtime map; it must not rebuild a baseline from the live candidate checkout. Use `optional_paths` only for genuinely optional inputs. `source_delta` is checked against the current identity and a JSON pointer into the frozen baseline; removals always fail, while changed and added paths must match the declared globs.

Keep identities phase-specific. The frozen predecessor belongs in `freeze`; do not make that phase consume the live candidate runtime. The measurement identity should include the benchmark sources, Cargo manifests/configuration, workload registry, and every selected oracle model or imported helper that affects the timed command. Avoid a broad Python glob that also captures unrelated analysis code; replace the sample harness paths with the exact task-owned benchmark and model files. The analysis identity should separately include `statistics.py`, the analyzer, gate implementation, imported build/dependency checkers, and other analysis helpers. Generated build reports should be declared as outputs of the phase that creates them, then consumed through a dependency directory such as `{verify_dir}`; do not list a not-yet-created report as a static identity input. The dependency receipt and its output references bind those generated files into the downstream phase cache key. Then a change to analysis or gate code invalidates `analyze` without invalidating `measure`, while a harness or workload change invalidates the measurements that depend on it. Commands that invoke task-specific Python checkers should use `{python}` and an absolute `{repo}/.../checker.py` argument so the checker source is captured as a helper input.

`checks` support `equal`, `positive`, `at_least`, `exact_set`, `length`, and `ref`. A task-specific checker should emit a small machine-readable summary for semantics that need more than those primitive checks. For runtime changes, bind full source-fuel prefixes, trap class and priority, observable memory/global/table effects, and lifecycle/root invariants. For build policy, check both hard budgets and the task's relative-growth limits. The workflow does not infer those semantics from a passing process exit code.

## Phase execution and test outcomes

Run one phase or all phases declared by the contract:

```sh
python3 -B tools/verify.py run --contract docs/benchmarks/performance/P900-verification.json --phase freeze --execute
python3 -B tools/verify.py run --contract docs/benchmarks/performance/P900-verification.json --phase verify --execute
python3 -B tools/verify.py run --contract docs/benchmarks/performance/P900-verification.json --phase measure --execute
python3 -B tools/verify.py run --contract docs/benchmarks/performance/P900-verification.json --phase analyze --execute
# Or run the declared phases in dependency order:
python3 -B tools/verify.py run --contract docs/benchmarks/performance/P900-verification.json --phase all --execute
```

`--execute` is required for phase runs, development groups, `close`, statistics, finalization, source-map output, migration, and publication. Without it those commands return an inert message. `plan`, `check-sources`, and `audit-statistics` do not take the flag. The audit recomputes both analysis and the full gate decision and may query Cargo metadata; it does not collect new timing samples.

Declare each feature configuration as one test command and associate its group with that command. The runner requires every named test to appear as an `ok` outcome, rejects duplicate, failed, or ignored test rows, checks Cargo's reported totals against the observed rows, and enforces `minimum_passed`. It accepts either the full qualified test name or its final leaf name. `unfiltered: true` additionally requires Cargo to report no filtered tests. Put the exact feature names in `features`; preflight requires equality with the command's feature flags. Production test groups use the selected toolchain's Cargo and retain its executed test binary identities. Synthetic workflow tests must declare `synthetic: true`; their evidence cannot complete a real task.

A runtime verification contract should include meaningful named cases for scalar results, complete fuel prefixes, traps and their priority, and side-effect images or hashes. Add cancellation, host reentry, GC roots, continuations, or fallback cases when those behaviors are in scope. Do not treat an empty suite, skipped tests, missing named outcomes, or a success exit without semantic output checks as evidence.

The `measure` phase is marked `timing_evidence: true` in its receipt. Keep candidate compilation and build-budget collection outside this phase; freeze/build beforehand, then measure the already-built executable. The runner records the declared commands but does not infer whether an arbitrary command builds, so the contract owner must keep that boundary explicit. `analyze` should consume the measured report and build records and produce the decision. Use `statistics` as that phase's single producer when the task can use the shared gate API: it calls `gates.evaluate` once, which performs paired analysis and gate checks together, and writes one bound statistics record. Do not separately run `analyze.py` on the same report first. `audit-statistics` later recomputes both analysis and the gate decision from the same report and build inputs, then compares them with the stored record. Packaging or finalization changes do not require another benchmark run when the measured inputs remain identical; the phase receipts still revalidate their declared inputs before reuse.

## Fast development iterations and full closure

Use `dev` for quick, named test/check groups while implementation is changing. It reads the same top-level `identities` as the full contract, but development groups have no phase dependencies. Each group declares `uses`, `commands`, and optional `tests`, `outputs`, and `checks`:

```json
{
  "development": {
    "groups": [
      {
        "name": "focused-default",
        "uses": ["runtime", "fixtures"],
        "commands": [{"name": "focused-default", "argv": ["cargo", "test", "--locked", "-p", "wtime-runtime", "p900_", "--", "--nocapture"]}],
        "tests": [{"command": "focused-default", "required_names": ["p900_load_prefixes", "p900_store_effects"], "minimum_passed": 2, "features": []}],
        "outputs": {}
      },
      {
        "name": "checker-unit-tests",
        "uses": ["analysis", "checker_tests"],
        "commands": [{"name": "checker-unit-tests", "argv": ["{python}", "-B", "-m", "unittest", "-v", "tools.p900.tests.test_checker"]}],
        "tests": [{"command": "checker-unit-tests", "kind": "unittest", "required_names": ["test_fuel_prefix", "test_build_limits"], "minimum_passed": 2}],
        "outputs": {}
      }
    ]
  }
}
```

Run all development groups, or select one or more with repeated `--group` options:

```sh
python3 -B tools/verify.py dev --contract docs/benchmarks/performance/P900-verification.json --group focused-default --group checker-unit-tests --execute
```

Each group gets its own development store under `target/verification/development/<task>/<group>`. An unchanged group reuses its passing receipt without rerunning commands; changed inputs cause a new attempt whose result reports the reason and changed inputs. Development receipts use lane `development` and `completion_eligible: false`: they cannot satisfy full-lane phases, finalize a task, or publish task status. They retain command logs and helper sources, but do not copy full source trees or Cargo test binaries into the store; the executed test binary's identity is recorded in the receipt. Cargo commands in this lane are limited to locked incremental `test` or `check` commands, without `--release` or custom profiles. Rust test groups must name their passing tests and declare a feature list exactly matching the command. A compile-only `cargo check` group may omit `tests`, but remains a development diagnostic. Python test groups use verbose `unittest`. Fake commands are allowed only for contracts marked `synthetic: true`, whose evidence cannot complete a real task.

When ready for evidence, use `close` to run every declared full phase in dependency order and finalize the resulting receipts in one operation:

```sh
python3 -B tools/verify.py close --contract docs/benchmarks/performance/P900-verification.json --output target/verification/P900-completion-0001 --execute
```

Choose a new, exclusive output path. `close` uses the normal full lane and its full-phase requirements; development receipts are never substituted. It reuses only full-lane phase receipts whose current inputs, outputs, named tests, helpers, and environment still verify. Review the completion record, then use the separate `publish` command described below if task status should be updated.

For operation counters and elapsed wall time, pass `--profile-output PATH` before the subcommand. Use a new output path for each operation; profiling is available for `plan`, `run`, `dev`, `close`, and `finalize`:

```sh
python3 -B tools/verify.py --profile-output target/verification/P900-dev-profile-0001.json dev --contract docs/benchmarks/performance/P900-verification.json --group focused-default --execute
```

The counters describe verification work, not benchmark timing evidence. A single CLI operation memoizes repeated DAG, hash, and content-addressed-store checks; the memo is discarded when that invocation ends. File guards track device, inode, mode, size, modification and change times, while glob membership and resolved bindings catch additions, removals, and symlink retargets. Directory inputs and trailing `/**` patterns include their files recursively. Verification and storage stream in bounded chunks; reading JSON or materializing a payload still produces its bytes.

Group identities must cover transitive compilation sources, included fixtures, imported test/checker modules, and settings. For Python groups that print while testing, declare `-b` (unittest output buffering) alongside `-v` so named outcomes stay intact. The ready-to-use [tooling development contract](../../docs/verification/tooling-development.json) separates artifact, runner, integrity, development, and gate groups:

```sh
python3 -B tools/verify.py dev --contract docs/verification/tooling-development.json --execute
```

Run the tooling performance smoke with a new output directory:

```sh
python3 -B tools/verify.py perf --output target/verification/performance-0001 --execute
```

The [budget](performance-budget.json) uses five fresh, deterministic synthetic fixtures. It records cold execution, cached execution, finalization, development no-ops, streaming peak memory, bytes processed, and subprocess/phase counts. Cached paths must launch zero commands, compute each dependency phase once, and stay below the artifact-byte budget; median development no-op and cached execution targets are one second. CI runs this smoke and retains its report and executed sources even on failure. These results measure tooling and do not refresh Wasm comparisons. Override the configuration explicitly with `--budget PATH` for a documented workload.

## Input binding, attempts, and immutable artifacts

For each phase, the cache key binds component declarations and file hashes/settings, dependency receipts, source ownership rules and baseline hash, phase definition, execution helpers, compiler/linker selections, and hashes of relevant environment values. Contracts may add variable names with a stage's `environment` list; values are never stored. If that input key has a verified success receipt, current storage, output, and test-log checks revalidate it before reuse; otherwise a new `attempt-NNNN` directory is created. Storage-only edits revalidate old bytes without replaying commands. Failed attempts retain their exit codes, logs, commands, and failure record. The runner checks consumed identities, helpers, and environment again before success.

Every phase receipt records its command arguments, working directory, exit status, test results, output references, source delta, and whether it contained timing evidence. Outputs, command logs, source inputs, project helpers, and Cargo test executables are copied into the content-addressed store. System interpreters and compilers are identified by hash; declare pinned tools in a phase's `retain_tools` list when their bytes must also be retained. Objects are validated by SHA-256, original byte length and file mode. ELF payloads are gzip-compressed deterministically; stored bytes have their own hash and length. Objects are made read-only, deduplicated by content, and materialized with the recorded original mode when needed. The receipt keeps both stored and original identities.

The final evidence manifest is an index of immutable phase receipt references, the contract reference, store location, decision/outcome, finalizer identity, and declared limitations. Finalization revalidates receipts and phase inputs and runs no commands or subprocesses. It writes an `evidence.json` and a separate `completion.json`; the latter contains the one authoritative proof reference and its hash.

Publication is a separate explicit step after review. It snapshots `TASKS.json`, `INDEX.md`, the task Markdown, and the completion record before validation. It checks the authoritative proof hash, all declared artifact hashes, dependency closure, and `enable_when`; then it updates only the selected task's status/outcome/evidence/completion record, its executed-evidence section, and the progress line. It runs no build or test processes. The prewrite snapshot and publication-attempt record are retained even if publication is blocked.

## Statistics and finalization

For a measured task, produce the statistics record from the raw comparison report and build reports:

```sh
python3 -B tools/verify.py statistics --task P900 --comparison target/performance/P900/comparison --build target/performance/P900/build.json --optional-build target/performance/P900/optional-build.json --contract docs/benchmarks/performance/P900-contract.json --output target/performance/P900/statistics.json --execute
python3 -B tools/verify.py audit-statistics --record target/performance/P900/statistics.json
python3 -B tools/verify.py finalize --contract docs/benchmarks/performance/P900-verification.json --output target/performance/P900/finalized --execute
```

`statistics` accepts an optional candidate-build record with `--candidate-build-record`. It binds the report, build reports, analyzer, gate, contract, candidate-build record, and producer hashes. `finalize` takes the decision from the configured required phase and JSON pointer; contract-only work must explicitly declare `outcome: "contract_only"`. Allowed measured outcomes are `adopted`, `rejected`, and `inconclusive`; they remain distinct from task status.

Gate evaluation captures fresh Cargo metadata and a source-input snapshot for build and dependency checks, reuses that snapshot across default and optional build validations, and checks the inputs again after evaluation. A detected source change fails the gate. These checks do not collect additional timing samples; timing remains in the declared `measure` phase.

The historical comparison source map includes all benchmark Python modules. For new shared statistics records, a narrow adapter permits hash changes only to `tools/wtime-bench/analyze.py` and `gates.py`, which run after measurement. It records their measured and current identities separately. Every other source hash and the complete file membership must still match; runtime, harness, oracle, fixture, and workload changes require fresh measurements. The adapter preserves the legacy observation, policy, build, dependency, and promotion checks. It also checks source stability after evaluation, and an audit requires the same retained source binding and producer paths. Original reports remain unchanged.

After reviewing the completion record, publish it with a new snapshot directory under `target/performance/TASK-status`:

```sh
python3 -B tools/verify.py publish --performance-root /workspace/tasks/performance --completion target/performance/P900/finalized/completion.json --snapshot target/performance/TASK-status/P900-publication-0001 --execute
```

Use a fresh, exclusive snapshot path for each attempt. Publication validates the current task state and never silently rewrites a completed or non-pending task.

## Read-only P042 history migration

The historical P042 closure is preserved through a dedicated frozen reader, not rerun as current-source evidence. `p042-contract` records normal references and hashes for its existing reports, logs, fixture identities, and frozen validator. `migrate-p042` validates those references, snapshots the adapter and reader, and invokes the legacy reader with a reference-only copy hook: it validates old payloads and report semantics but records a deduplicated storage plan instead of copying artifacts through the old API. Subprocess launch is blocked during that validation; no Cargo, encoder, guest, bootstrap, timing, or new statistics run occurs. The shared store writes the payload objects once after validation. `finalize-migration` validates the prepared store/source references and creates a thin compatibility evidence index without invoking the reader or running commands.

```sh
python3 -B tools/verify.py p042-contract --output target/verification/P042-contract.json --execute
python3 -B tools/verify.py migrate-p042 --contract target/verification/P042-contract.json --execute
python3 -B tools/verify.py finalize-migration --prepared target/performance/P042/shared-migrations/attempt-0001/prepared.ref.json --output target/performance/P042/verification-migration --execute
```

Use the `prepared` reference returned by `migrate-p042`; the command above shows the first attempt's path only.

Keep the original P042 task reports and decision immutable. The resulting migrated proof points back to the historical artifacts and explicitly retains the original limitations; it does not refresh measurements or change the original outcome.
