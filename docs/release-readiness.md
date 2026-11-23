# Release readiness (`v0.1.0`)

The release decision is verified by `python3 -B tools/checks/release_audit.py`.
It joins the frozen proposal/source lock ([spec-lock.json](architecture/spec-lock.json)),
the corpus lock ([corpus-lock.json](architecture/corpus-lock.json)), the opcode
and rule ownership ledger ([opcodes.json](architecture/opcodes.json)), and the
evidence reports indexed in [release-manifest.json](architecture/release-manifest.json).

Exit status `0` means every required report for the declared `supported_targets`
is present, passes its schema checks, binds to the current core source digest,
built executables, and documentation hashes, no semantic ledger owner remains
pending, and no excluded or unspecified behavior is claimed as supported. Any
missing, stale, unsupported, empty, or hash-mismatched item exits nonzero.

## `v0.1.0` Target Scope

- **Supported target**: `x86_64-unknown-linux-gnu` (`rustc 1.99.0`).
- **Deferred target**: Native `aarch64-unknown-linux-gnu` release verification
  is planned for a follow-up release (the CI workflow runs native Ubuntu ARM
  checks in `.github/workflows/ci.yml`, but local `v0.1.0` release evidence is
  scoped explicitly to `x86_64-unknown-linux-gnu`).
- **Excluded scope**: JS/browser embedding APIs (`content-security-policy`,
  `ecmascript_module_integration`, `javascript_bigint_to_webassembly_i64_integration`,
  `js-promise-integration`, `js-string-builtins`), custom-descriptor JS
  embedding hooks, and the unspecified Type Section Field Deduplication TODO.

## Reproduction

Run from the repository root with Rust `1.99.0` (`rust-toolchain.toml`):

```sh
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --locked --no-deps --workspace --all-features
cargo test --locked --workspace --all-features
cargo test --locked --doc --workspace
cargo package --locked --workspace --exclude wtime-conformance
python3 -B tools/checks/dependency_budget.py
python3 -B -m unittest discover -s tools/checks/tests -v
python3 -B tools/checks/opcode_coverage.py --require-complete
python3 -B tools/checks/release_audit.py --portable
```

On a checkout where the release-evidence binaries have been built into
`target/evidence/` (below), also run strict binary-bound validation:

```sh
python3 -B tools/checks/release_audit.py
```

Building release-evidence binaries into `CARGO_TARGET_DIR=target/evidence`
keeps their exact ELF SHA-256 fingerprints isolated from default `target/`
builds (such as `cargo test --workspace --all-features`).

### Regenerating manifest evidence reports

Run on a clean committed tree (zero uncommitted changes so `engine_revision`
records a clean 40-character commit SHA with no `-dirty` suffix):

```sh
python3 -B tools/checks/fetch_corpus.py
CARGO_TARGET_DIR=target/evidence cargo build --locked --release -p wtime-conformance --features opcode-coverage
# Write conformance reports into ignored target/conformance/ first so git status stays clean across all runs.
target/evidence/release/wtime-conformance run --profile mvp --suite mvp --filter all --report target/conformance/mvp.json
target/evidence/release/wtime-conformance run --profile stable-core --suite core3 --filter all --report target/conformance/core3.json
target/evidence/release/wtime-conformance run --profile all-phase3-core --suite phase3 --filter all --report target/conformance/phase3.json
target/evidence/release/wtime-conformance run --profile all-phase3-core --suite interactions --filter all --report target/conformance/interaction-corpus.json
cp target/conformance/core3.json docs/verification/core3.json
cp target/conformance/phase3.json docs/verification/phase3.json
cp target/conformance/interaction-corpus.json docs/verification/interaction-corpus.json
python3 -B tools/checks/dependency_budget.py > docs/verification/dependency-budget.txt
python3 -B tools/checks/test_evidence.py --package wtime-runtime --test feature_profiles --report docs/verification/feature-profiles.json
python3 -B tools/checks/test_evidence.py --package wtime-conformance --test interactions --report docs/verification/interactions.json
python3 -B tools/checks/test_evidence.py --package wtime-conformance --test annotations --report target/conformance/annotations.json
python3 -B tools/checks/test_evidence.py --package wtime-decode --filter metadata --report target/conformance/branch-hints-decode.json
python3 -B tools/checks/test_evidence.py --package wtime-cli --filter t158_ --report target/conformance/branch-hints-cli.json
python3 -B tools/checks/reconcile_opcode_evidence.py --reports target/conformance/mvp.json docs/verification/core3.json docs/verification/phase3.json docs/verification/interaction-corpus.json
CARGO_TARGET_DIR=target/evidence cargo build --locked -p wtime-cli -p wtime-conformance --bin wtime-cli --bin wtime-probe --bin wtime-allocation-check
python3 -B tools/checks/allocation_failure.py --worker target/evidence/debug/wtime-allocation-check --report docs/verification/allocation-failure.json
python3 -B tools/checks/fuzz_decode.py --target target/evidence/debug/wtime-probe --seed 164 --iterations 1000 --profile all-phase3-core $(printf -- '--corpus %s ' crates/wtime-runtime/tests/fixtures/feature_profiles/*.wasm) --output target/fuzz/seed164-all && cp target/fuzz/seed164-all/report.json docs/verification/fuzz.json
python3 -B tools/checks/differential.py --target target/evidence/debug/wtime-probe --seed 165 --count 100 --oracle "$(command -v node)" --oracle-kind node --oracle-sha256 "$(sha256sum "$(command -v node)" | cut -d' ' -f1)" --output target/differential/node-seed165 && cp target/differential/node-seed165/report.json docs/verification/differential.json
python3 -B tools/checks/hardening.py --native-report docs/verification/native-x86_64.json
python3 -B tools/checks/build_budget.py --output docs/verification/build-final.json
python3 -B tools/checks/api_docs_evidence.py --cli target/evidence/debug/wtime-cli --target-dir target/evidence --report docs/verification/api-docs.json
```

## What each gate establishes

| Evidence | Report | Required check |
| --- | --- | --- |
| Core 3.0 & Phase 3 corpora | `core3.json`, `phase3.json`, `interaction-corpus.json` | 100% selected assertions execute with zero failures, skips, unsupported directives, or text-tool errors against current implementation sources and executable. |
| Feature profiles & interactions | `feature-profiles.json`, `interactions.json` | Explicit enable/disable and cross-proposal interaction tests execute against current core sources. |
| Fuzz & differential | `fuzz.json`, `differential.json` | 1,000 seeded decode/validate mutations and 100 deterministic differential vectors (with Node/V8 oracle) pass with zero failures or unexplained differences. |
| Allocation failure | `allocation-failure.json` | Process-isolated worker under 128 MiB `RLIMIT_AS` triggers real recoverable allocator failures (512 MiB memory, 16M-element guest table, and 50k-local recursive frame allocation) and verifies store recovery. |
| Native `x86_64` hardening | `native-x86_64.json` | Native `wtime-runtime` test suite (including cancellation and stale-generation regressions) passes on `x86_64` against the frozen core source hash. |
| Build & dependency budgets | `build-final.json`, `dependency-budget.txt` | Clean/leaf compile times and stripped `wtime-cli` binary size meet budgets; zero external production dependencies and zero `unsafe` in core crates. |
| API docs & examples | `api-docs.json` | Documented embedding, suspension, and CLI examples execute cleanly via `tools/checks/api_docs_evidence.py` and match current doc/source digests. |
