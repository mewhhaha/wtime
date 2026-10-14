# P006 — Add bounded development-only hot-path counters

Status: **done**

Outcome: **complete**

Kind: `foundation` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P001 — Archive the current runtime and benchmark baseline](P001-freeze-baseline.md)

[P003 — Define workload selections and engine capability preflight](P003-workload-schema.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/perf.rs`
- `crates/wtime-runtime/src/lib.rs`
- `crates/wtime-runtime/src/execution.rs`
- `crates/wtime-runtime/Cargo.toml`
- `tools/wtime-bench/src/main.rs`
- `tools/wtime-bench/Cargo.toml`

- `crates/wtime-runtime/src/gc_exec.rs`

- `crates/wtime-runtime/src/continuation.rs`

- `tools/wtime-bench/src/protocol.rs`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

performance-counters is a nondefault diagnostic feature. CounterSnapshot reports source-op dispatches, deep-copy bytes/events, fixed/read-Vec operations, frame/argument/root snapshot allocations, subtype probes and continuation scans with saturating counters.

## Implementation recipe

1. Instrument the known call sites instead of introducing an unsafe global allocator: Module clone in invoke, Instruction clone, Mem::read, keep/split_off, frame entry, host roots, subtype queries and continuation byte scans.
2. Keep counters Store-local or explicit diagnostic context, never global mutable state; use saturating arithmetic and bound histogram keys so malicious opcode/type/input churn cannot allocate indefinitely.
3. Expose snapshots only to the development harness; compile counters and branches out of default builds and label diagnostic report settings visibly.
4. Check counts on hand-countable fixtures and retain a diagnostic baseline; record these as attribution clues, not production allocation totals or speed measurements.
5. Forward performance-counters from the benchmark manifest to runtime, emit source-bound snapshots only in diagnostic mode, and keep legacy frozen-core adapters capability-limited rather than editing their frozen library.

## Meaningful cases

- A short integer function has the expected source dispatch count and no scalar-memory event.
- A load records one read allocation in the pre-optimization diagnostic build.
- Counter saturation is safe and default Cargo features contain no counter overhead.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
cargo test --locked -p wtime-runtime --features performance-counters p006_ -- --nocapture
cargo build --locked --release -p wtime-cli
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
    "/workspace/wtime/target/performance/P006-diagnostics.json"
  ],
  "commands": [
    {
      "command": "cargo test --locked -p wtime-runtime --features performance-counters p006_ -- --nocapture",
      "exit_code": 0,
      "passed": 3,
      "failed": 0,
      "skipped": 0
    },
    {
      "command": "cargo build --locked --release -p wtime-cli",
      "exit_code": 0
    },
    {
      "command": "cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml --features performance-counters -j4",
      "exit_code": 0
    },
    {
      "command": "--diagnostic original four runtime workloads decoded mvp",
      "exit_code": 0,
      "passed": 4,
      "failed": 0,
      "skipped": 0
    }
  ],
  "hashes": {
    "P006-diagnostics.json": "9e44e8f58cc2a8f84782ed43459442be55a510b58eccc1159be9e2fea8081c77"
  },
  "limitations": [
    "Store-local saturating attribution counts explicit source sites, not total allocator activity or CPU percentages.",
    "Counters compile out by default; diagnostic elapsed times are not production benchmark evidence.",
    "read_fixed_operations remains zero until fixed-width reads are implemented."
  ]
}
```
