# P004 — Compute uncertainty and preparation break-even

Status: **done**

Outcome: **complete**

Kind: `foundation` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P002 — Compare separately built baseline and candidate binaries](P002-paired-driver.md)

[P003 — Define workload selections and engine capability preflight](P003-workload-schema.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `tools/wtime-bench/analyze.py`
- `tools/wtime-bench/tests/test_p004_statistics.py`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

analyze.py --input DIR --output FILE validates paired reports, produces per-run ratios/95% bootstrap intervals, control decisions and total-latency/break-even tables without dropping inconvenient observations.

## Implementation recipe

1. Group only matching workload/input/stage/settings pairs; calculate paired log ratios, medians, MAD and quartiles with a fixed bootstrap seed and 10,000 resamples.
2. Implement the exact promotion/inconclusive rules in MEASUREMENT.md, keeping per-run decisions separate and using predeclared equally weighted families only within the same stage.
3. Calculate break-even only for supported positive warm savings, include extra preparation/instance cost, and show 1/10/100/10,000-call totals plus no-finite-break-even where appropriate.
4. Record tail latencies only when reports contain individual latency samples; reject confidence or p99 claims derived from calibrated batch averages.

## Meaningful cases

- Known paired ratios produce deterministic statistics with fixed seed.
- Zero/negative savings yield no finite break-even; a slower cold tier can still have a positive crossover.
- An incomplete pair or mixed feature policy is rejected; high-noise controls are inconclusive rather than passed.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
python3 -m unittest discover -s tools/wtime-bench/tests -p 'test_p004*.py' -v
python3 tools/wtime-bench/analyze.py --input target/performance/P002 --output target/performance/P004-analysis.json
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
    "/workspace/wtime/target/performance/P004-analysis.json",
    "/workspace/wtime/target/performance/P002/report.json"
  ],
  "commands": [
    {
      "command": "python3 -m unittest discover -s tools/wtime-bench/tests -p test_p004*.py -v",
      "exit_code": 0,
      "passed": 7,
      "failed": 0,
      "skipped": 0
    },
    {
      "command": "python3 tools/wtime-bench/analyze.py --input target/performance/P002 --output target/performance/P004-analysis.json",
      "exit_code": 0,
      "groups": 42,
      "bootstrap_resamples": 10000,
      "confidence": 0.95
    }
  ],
  "report_sha256": "80826443a8db04fc44326894b072db70a7a5ad53c95235b490f9c5ad1c7d5470",
  "limitations": [
    "Analyzed P002 archived-source observations, not an optimization speed claim.",
    "No p99 is inferred from batched averages; break-even is unavailable without measured cold stages."
  ]
}
```
