# P123 — Adapt supported typed workload routes to immutable P001 sources

Status: **done**

Outcome: **contract_only**

Kind: `foundation` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P001 — Archive the current runtime and benchmark baseline](../00-measurement/P001-freeze-baseline.md)

[P002 — Compare separately built baseline and candidate binaries](../00-measurement/P002-paired-driver.md)

[P003 — Define workload selections and engine capability preflight](../00-measurement/P003-workload-schema.md)

[P015 — Add reproducible application-shaped held-out kernels](../00-measurement/P015-representative-kernels.md)

[P045 — Verify and fuzz prepared compact programs](P045-compact-verifier-fuzz.md)

[P122 — Prove Compact representative timing keeps the source-bound guest boundary](P122-compact-representative-protocol.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [MEASUREMENT.md](../MEASUREMENT.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md). Apply C1–C8 from [CONTRACTS.md](../CONTRACTS.md). Use Luna for exploration and mechanical tooling; root owns runtime semantics and shared integration.

- `tools/wtime-bench/baseline.py`
- `tools/wtime-bench/compare.py`
- `tools/wtime-bench/src/protocol.rs`
- `tools/wtime-bench/tests/test_p123_p001_typed_adapter.py`
- `tools/wtime-bench/p123_adapter.py`
- `docs/benchmarks/performance/P123-verification.json`

P123 owns the source-bound P001 adapter and typed route wiring only; no edits to immutable archives, runtime lib.rs, dispatch, feature declarations, or performance test registration.

## Input/output contract

A source-bound adapter runs the current typed measurement harness against immutable P001 archived core/API sources. It never substitutes current runtime sources. Scalar, memory, SIMD and representative routes are probed against the actual frozen API and independent oracles; only actual compile/profile/API capability failures may be reported unsupported. Existing Python-side legacy-adapter whitelists are not capability evidence. Every output binds archive, harness, helper and executable identities.

## Implementation recipe

1. Verify the immutable P001 source manifest and original executable/source identities; preserve all original artifacts unchanged.
2. Build a new hash-bound adapter using the current harness protocol against archived P001 sources only, retaining exact protocol/harness/source identities and never copying current core code.
3. Probe scalar, memory, SIMD and representative typed APIs independently against the actual frozen API; preserve independent result/output/memory oracles and record exact compile, feature-profile and runtime capability evidence.
4. Extend comparison route identity checks to accept only the verified P001 adapter; reject mismatched baseline/core/harness identities. An existing Python whitelist rejection is not evidence of unsupported runtime capability.
5. If implementation requires more than two focused changes or runtime API substitution, split typed adapter construction from family-route wiring before implementation.

## Meaningful cases

- A typed scalar P001 route is either built and passes its independent oracle or records a concrete source/API/profile failure receipt.
- A typed memory route verifies effects, traps and bytes when supported; unsupported status cites an actual frozen API/profile failure rather than comparator policy.
- SIMD and representative probes record supported types/features and source-bound fixture identities, or explicit concrete unsupported evidence; no fabricated timing row is accepted.

## Verification

Activate Rust with `. /workspace/.setup/rust-env.sh`. Define the task-owned verification contract and incremental development groups before running checks. Preserve all helper sources/commands before execution and use Python -B.

```sh
python3 -B -m unittest discover -s tools/wtime-bench/tests -p 'test_p123*.py' -v
python3 -B tools/verify.py plan --contract docs/benchmarks/performance/P123-verification.json
```

## Measurement and decision

Foundation correctness only. P046 owns the full corpus and production performance decision; no timing or promotion claim follows from development receipts. Full conformance collection may be reused by P046 only while its consumed sources/profiles/corpus hashes remain current.

## Completion evidence

Full verification must execute p123_adapter_is_hash_bound_to_immutable_p001, p123_typed_scalar_and_memory_routes_preserve_oracles, p123_simd_representative_capabilities_are_actual, plus representative capability and archive-mutation rejection cases. Retain original P001 hashes, adapter/core/harness hashes, exact capability probes, compile/profile errors when unsupported, model identity and oracle probes. No speed result is measured by P123.

Publish only after full task closure is reviewed; then run both plan validators. All listed commands are requirements, not claims that they have run.

## Executed evidence

The full verification passed in [`attempt-0004`](../../../wtime/target/verification/runs/P123/verify/attempt-0004). It built the decoded `runtime-only,legacy-api` adapter against the immutable P001 archive, then executed 243 workload rows and 1,095 stage probes. All 85 scalar, 35 memory, 114 SIMD, and 9 representative rows passed their independent oracle checks; the report records zero unsupported rows. Five named Python tests passed with no failures or skips.

The P001 manifest SHA-256 is `7349596fc806c96c74cb9c40f1c08402589c1683660b0b152c713264f5808472`; its original benchmark executable SHA-256 is `735e595f3a625da89401fdee298781b752fbe071e3d3d793df196c9e68b8e498`. The typed adapter manifest SHA-256 is `ed06d76b34c4892c4476eb0fdfd0939c6da7bf99323befcfa66a684f48ae5714`, and its executable SHA-256 is `56c2fad760ccf4c10b348cd1d74478d592943884897b17506888b3d8e858d7f0`. The adapter binds 43 archived core identities and 163 harness source identities. Its protocol reports decoded mode, the frozen stable-core profiles, and `instrumented: false`; the build had zero external dependencies and used a fresh target directory.

The probe report at `target/verification/runs/P123/verify/attempt-0004/probes/report.json` has SHA-256 `8b933f5d63ac6931421076910549f4e368498f2cc7a85e8c941cd08f906c9fc5`. The retained probe package is `target/verification/runs/P123/verify/attempt-0004/probes/artifacts.tar.gz` with SHA-256 `5739dcb3c1dc8b6fb7ee29aed0f118b8d7821472dd9d5fc822ee029cebb08eb0`; its inventory and command log are separately hash-bound in the report.

The first three verification attempts remain preserved. Attempt 1 exposed a producer lookup for a legacy `family` field. Attempt 2 exposed a wrong-width memory boundary helper input, which led to an expected-value mismatch in the probe. Attempt 3 exposed a SIMD parameter lookup for a nonexistent `baseline` field. The producer lookups and helper usage were corrected before the passing run; these attempts were not treated as runtime capability failures.

P123 is `done` with outcome `contract_only`. The full run contains no timing evidence, performance promotion, or native execution claim. See [`P123-result.json`](../../../wtime/docs/benchmarks/performance/P123-result.json), SHA-256 `abbc58382c1fc8a60cc347edb4198a4439620799f6dd3223099057f5eab4f429`, for commands, receipt, report, fixture-family counts, and artifact hashes. The finalization evidence is [`evidence.json`](../../../wtime/target/performance/P123/shared-finalization/attempt-0004/evidence.json), SHA-256 `ef55d464728d4a68eff587a287b95b05f6e355bfbda9a0ce969c12f71ce84f5d`.
