# Release readiness

The release decision is made by `python3 tools/checks/release_audit.py`. It joins the task dependency graph in `/workspace/tasks/TASKS.json`, the frozen proposal/source lock, the corpus lock, the opcode and rule ownership ledger, and evidence reports named in [release-manifest.json](architecture/release-manifest.json). Exit status 0 means every required predecessor task is done, all required reports are present and tied to the current source/test snapshot, no semantic ledger owner remains pending, and no excluded or unspecified behavior is claimed as supported. Any missing, stale, unsupported, empty, or hash-mismatched item returns nonzero.

The manifest is deliberately an evidence index, not a completion declaration. Each report path points to the output of its existing producer, and the manifest pins that report's SHA-256. The checker validates each producer's actual schema: conformance reports use `selected`, `executed`, `failed`, `skipped`, `unsupported`, binary fingerprints and implementation-source fingerprints; native and build reports use the shared core `source_sha256`; the allocation-failure report binds the worker executable hash and checks its recorded resource limits plus worker JSON showing both allocator failure and recovery; fuzz and differential reports bind their generators, target binary, corpus and locked Core revision. The current core source digest is stored in the manifest. A report with an earlier digest, a replaced worker, or an implementation-source fingerprint that no longer matches is stale and blocks release.

## Reproduction

Run from the repository root after activating the pinned toolchain:

```sh
. /workspace/.setup/rust-env.sh
python3 tools/checks/release_audit.py
python3 -m unittest discover -s tools/checks/tests -p 'test_release_audit.py' -v
python3 tools/checks/opcode_coverage.py --require-complete
python3 /workspace/tasks/check_plan.py
git diff --check
```

The corpus and prerequisite commands are recorded in the relevant task files. The manifest uses the final `target/conformance/final-*` outputs for the full corpora; the task-specific `T160.json`, `T161.json`, and `T163.json` reports remain useful for task review. Reproduce the final reports with:

```sh
cargo run --locked -p wtime-conformance -- run --profile stable-core --suite core3 --filter all --report target/conformance/final-core3.json
cargo run --locked -p wtime-conformance -- run --profile all-phase3-core --suite phase3 --filter all --report target/conformance/final-phase3.json
cargo run --locked -p wtime-conformance -- run --profile all-phase3-core --suite interactions --filter all --report target/conformance/final-interactions.json
```

After the implementation is stable, rerun the feature profile, fuzz and differential runs, runtime hardening, allocation-failure check, and final build budget. Store outputs at the paths listed in the manifest and update the corresponding report digests and source digest from those outputs. Do not pin a report from an earlier source revision. Native architecture reports must be collected on their named machines; cross-compilation is not native execution.

## Current blockers

The frozen source snapshot has current, pinned reports: 18,480 MVP assertions, 62,968 Core 3.0 assertions, 64,297 Phase 3 assertions, and 15 interaction assertions all executed with zero failures, skips, unsupported directives, or text-tool errors. The reports bind to the current implementation fingerprints and conformance executable; feature profiles (28), interactions (7 focused tests), API examples (4), native x86_64 (134), fuzz (1,000), differential (100), allocation failure, and the final build budget also pass. The manifest pins the exact producer bytes, and the checker detects source, implementation fingerprint, generator, target binary, corpus, or report changes.

The manifest's supported-claims snapshot follows the included feature IDs in the source lock, with custom descriptors limited to specified semantics. The semantic ledger has no pending owners. T166 remains in progress because native AArch64 evidence is absent; T169 remains in progress while the audit is blocked. Native x86_64 records 134 passing tests. The allocation-failure report exercises a 512 MiB allocation under a 128 MiB address-space limit and confirms recovery. The checker output is the authoritative current list of blockers. The checker never edits task status.

The locked scope excludes the five JS/browser integrations listed in the manifest. The custom descriptor source's bare Type Section Field Deduplication TODO has no specified encoding or semantics, and its JS prototype/compileOptions/configureAll behavior is outside the embedding scope. These limits remain explicit and cannot appear among supported claims. The repository declares `MIT OR Apache-2.0`; production dependency license review and documented target support still require concrete release evidence.

## What each gate establishes

| Evidence | Reproducer | Required evidence |
| --- | --- | --- |
| Core 3.0 corpus | Command above; T160 task equivalent writes `target/conformance/T160.json` | Every selected assertion executes and passes against the current implementation sources. |
| Phase 3 corpus | Command above; T161 task equivalent writes `target/conformance/T161.json` | Every selected assertion executes and passes against the current implementation sources. |
| Feature profiles and interactions | T162 and T163 task commands | Required enable/disable and combined-feature assertions execute. |
| Fuzz, differential, hardening | T164, T165 and T166 task commands | Current-source results, executed counts and triage are recorded; unexplained failures block. |
| Allocation failure | `python3 tools/checks/allocation_failure.py --report docs/verification/allocation-failure.json` | Under a 128 MiB address-space limit, the 512 MiB allocation fails and the worker confirms it recovers; report source and worker hashes match the checkout. |
| Final build | `python3 tools/checks/build_budget.py --output docs/verification/build-final.json` | Stable source digest, clean/warm timings, RSS and binary size meet the recorded budget. |
| Native architecture | `python3 tools/checks/hardening.py --native-report <report.json>` on each architecture | Passing runtime tests on native x86_64 and AArch64 against the same core source digest. |
| API documentation and licenses | T168 documentation/example checks and dependency audit | Examples work, support scope is accurate, and license/target claims are evidenced. |

An opcode ownership/dispatch pass by itself is not semantic conformance. `--require-complete` intentionally fails while any validation, execution, or test owner remains `pending-audit`.
