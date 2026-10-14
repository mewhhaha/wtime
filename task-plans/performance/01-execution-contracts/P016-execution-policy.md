# P016 — Add explicit executor policies and test integration hooks

Status: **done**

Outcome: **complete**

Kind: `foundation` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P001 — Archive the current runtime and benchmark baseline](../00-measurement/P001-freeze-baseline.md)

[P003 — Define workload selections and engine capability preflight](../00-measurement/P003-workload-schema.md)

[P006 — Add bounded development-only hot-path counters](../00-measurement/P006-diagnostic-counters.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/lib.rs`
- `crates/wtime-runtime/Cargo.toml`
- `crates/wtime-runtime/tests/performance.rs`
- `tools/wtime-bench/Cargo.toml`
- `tools/wtime-bench/src/main.rs`
- `tools/wtime-bench/baseline.py`

- `crates/wtime-runtime/src/execution.rs`
- `crates/wtime-runtime/src/reference.rs`
- `crates/wtime-runtime/tests/performance/p016.rs`
- `tools/wtime-bench/src/protocol.rs`
- `tools/wtime-bench/tests/test_p016_archive.py`
- `tools/wtime-bench/profiling/Cargo.toml`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

Engine::new(Config) keeps decoded execution; Engine::with_execution_policy(Config, ExecutionPolicy) selects decoded/reference/compact/register/experimental/native only when that mode is compiled. Absent modes fail at configuration, never silently change the requested policy.

## Implementation recipe

1. Define a small public policy enum and internal dispatch hook without adding unchecked module constructors or breaking current Config literals; keep future tiers unavailable until their implementations land.
2. Provide a development-only reference executor boundary seeded from the archived source; share immutable input/helpers deliberately, compile it out by default and document oracle dependence.
3. Reserve nondefault runtime and benchmark features for compact-executor, register-executor, experimental-executors and execution-reference; native is only a placeholder policy until its ADR, not a dependency.
4. Add candidate-mode forwarding to the benchmark protocol and a single performance test target; one integrator registers leaf test modules as tasks land, with prefix/count checks.
5. Expose the portable alias (current selected portable policy, never native) and baseline.py --mode MODE for current-source archives. Legacy adapters continue to use the original Engine::new under a compile-time legacy-api branch and report their limited policy capability.
6. Forward experimental-executors to the required compact/register feature modules in runtime and the isolated benchmark; feature compilation alone never selects an experiment. Unknown selectors fail; existing Config/API compatibility remains.

## Meaningful cases

- Default API behavior and existing CLI/conformance builds stay unchanged.
- Requesting a missing tier returns an explicit configuration error.
- Default dependency/features exclude the reference and experimental executors; reference runs the original core fixture.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime --features execution-reference p016_ -- --nocapture
```

## Measurement and decision

Selection: `integration`. Explicit execution setting: `decoded`. Decision rule: `integrity`.

This is a tooling/contract deliverable. Require correct nonzero checks and bounded/source-bound output; do not invent a runtime speedup requirement. If it changes production work, record cold/build/retention deltas under [MEASUREMENT.md](../MEASUREMENT.md).

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. Executed evidence is recorded below.

## Executed evidence

```json
{
  "artifacts": [
    "/workspace/wtime/target/performance/P016-reference/manifest.json"
  ],
  "commands": [
    {
      "command": "cargo test --locked -p wtime-runtime --features execution-reference p016_ -- --nocapture",
      "exit_code": 0,
      "passed": 4,
      "failed": 0,
      "skipped": 0
    },
    {
      "command": "cargo test --locked -p wtime-runtime --features execution-reference,experimental-executors --test performance p016_ -- --nocapture",
      "exit_code": 0,
      "passed": 4,
      "failed": 0,
      "skipped": 0
    },
    {
      "command": "python3 -m unittest discover -s tools/wtime-bench/tests -p 'test_p016*.py' -v",
      "exit_code": 0,
      "passed": 7,
      "failed": 0,
      "skipped": 0
    },
    {
      "command": "cargo build --locked -p wtime-cli -p wtime-conformance",
      "exit_code": 0
    },
    {
      "command": "python3 tools/wtime-bench/baseline.py --mode reference --output target/performance/P016-reference",
      "exit_code": 0,
      "mode_smoke_passed": true
    }
  ],
  "hashes": {
    "manifest.json": "892ebd48bb57709264cf9cab915d5664c634e21bae0b07d3e8f1bf50b90d116c"
  },
  "limitations": [
    "Reference dispatcher seeded from frozen original invoke; admission, frames, numeric/memory/GC/continuation helpers are deliberately shared and need external/frozen comparison.",
    "Compact/register/experimental/native remain unavailable even when placeholder Cargo features compile.",
    "Portable resolves to decoded; no performance improvement is claimed for policy tooling.",
    "Archive validator traverses feature aliases and rejects reachable cycles; aggregator forwarding is validated transitively."
  ]
}
```
