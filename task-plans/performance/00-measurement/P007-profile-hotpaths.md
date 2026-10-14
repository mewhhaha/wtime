# P007 — Capture CPU profiles and rank actionable costs

Status: **done**

Outcome: **complete**

Kind: `foundation` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P003 — Define workload selections and engine capability preflight](P003-workload-schema.md)

[P006 — Add bounded development-only hot-path counters](P006-diagnostic-counters.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `tools/wtime-bench/profile.py`
- `tools/wtime-bench/tests/test_p007_profile.py`
- `docs/benchmarks/performance/hotpaths.md`

- `tools/wtime-bench/profiling/Cargo.toml`
- `tools/wtime-bench/profiling/Cargo.lock`
- `tools/wtime-bench/profiling/src/main.rs`
- `tools/wtime-bench/src/protocol.rs`
- `tools/wtime-bench/Cargo.toml`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

profile.py --selection NAME --output DIR captures capability/tool/version metadata, a symbolized CPU profile where available, and a separate counter run; unavailable perf privileges are a reported capability gap, not a reason to escalate automatically.

## Implementation recipe

1. Probe perf/stat/record, symbolizer and profiler availability without installation or changing kernel settings; prefer sampling on a release-with-debug build and record its different artifact hash.
2. Run long scalar/memory/call workloads separately, retain command, stack samples and symbols, and compare diagnostic event counts to original operation counts.
3. If hardware sampling is unavailable, use bounded explicit diagnostic spans around preparation/call/memory/root paths, recording instrumentation bias and declining percentage attribution it cannot establish.
4. Write ranked hypotheses with exact file/symbol evidence and a proposed candidate family; do not infer that every visible clone explains a measured percentage.
5. For symbolized sampling build with CARGO_PROFILE_RELEASE_DEBUG=1 and RUSTFLAGS=-Cforce-frame-pointers=yes in a separate target; for event counts use a separate performance-counters build. Record each binary/flag set, and never substitute either timing for the production release result.

## Meaningful cases

- Unavailable profiler produces a capability record and usable counter evidence.
- A crashing workload cannot yield a successful profile artifact.
- Profiles/counters identify their own build and never overwrite uninstrumented timing data.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
python3 -m unittest discover -s tools/wtime-bench/tests -p 'test_p007*.py' -v
python3 tools/wtime-bench/profile.py --selection scalar --output target/performance/P007-profile
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
    "/workspace/wtime/target/performance/P007-profile-runtime/profile.json",
    "/workspace/wtime/docs/benchmarks/performance/hotpaths.md"
  ],
  "commands": [
    {
      "command": "python3 -m unittest discover -s tools/wtime-bench/tests -p 'test_p007*.py' -v",
      "exit_code": 0,
      "passed": 3,
      "failed": 0,
      "skipped": 0
    },
    {
      "command": "python3 tools/wtime-bench/profile.py --selection scalar --output target/performance/P007-profile-runtime",
      "exit_code": 0,
      "passed": 3,
      "failed": 0,
      "sampling_available": false
    }
  ],
  "hashes": {
    "profile.json": "489434f503974086b6d6823f5ddef3172b2e2a39b143e56ce1949fb3a053ffe5",
    "hotpaths.md": "8a482663fc85958ad1ca59830805772b5bc3152e33e02488aedca5703efe3f09"
  },
  "limitations": [
    "perf executable unavailable; no sampled CPU percentages claimed.",
    "Standalone internal-only profiling harness shares the benchmark protocol and records separate frame-pointer/debug and counter builds.",
    "Initial comparator-linked profiling attempt was stopped because it unnecessarily compiled Cranelift; no successful evidence uses that partial directory."
  ]
}
```
