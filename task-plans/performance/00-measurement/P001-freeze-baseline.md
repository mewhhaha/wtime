# P001 — Archive the current runtime and benchmark baseline

Status: **done**

Outcome: **complete**

Kind: `foundation` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

None. Freeze this baseline before editing core runtime sources.

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C4, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `tools/wtime-bench/baseline.py`
- `tools/wtime-bench/tests/test_p001_baseline.py`

One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; leaf owners submit a bounded integration hunk.

## Input/output contract

baseline.py --output DIR creates or verifies an immutable local archive containing the current release harness, reproducible source snapshot, fixture bytes, manifest and SHA-256 identities; it never overwrites a different archive.

## Implementation recipe

1. Read the two historical benchmark reports and archive their exact bytes; record that the checkout has no historical commit to rebuild from.
2. Build the existing release benchmark before any core edit, copy its executable and all core/benchmark source inputs to a local archive, and record Rust version, flags, lockfiles, feature settings and licenses.
3. Store deterministic fixture bytes and oracle inputs with a manifest; verify executable and fixture hashes when opening an existing archive, failing on mismatches rather than rebuilding it from changed sources.
4. Run a baseline smoke invocation for each existing executable workload and a fresh historical-format run; bind the archive to the observed outputs and keep large artifacts out of Git.

## Meaningful cases

- A verified existing archive is reused unchanged.
- Changing one source/fixture/binary byte makes verification fail.
- All four runnable original workloads return their oracle values; prepare_large remains prepare-only.

## Verification

Activate Rust as described in EXECUTION.md. These are required implementation-time commands; new tools/modes are supplied by linked tasks or this task. The focused tests/validators must execute at least **three** named cases, and optimized-path tests must prove the requested path ran.

```sh
python3 -m unittest discover -s tools/wtime-bench/tests -p 'test_p001*.py' -v
python3 tools/wtime-bench/baseline.py --output target/performance/baseline
```

## Measurement and decision

Selection: `original-mvp`. Explicit execution setting: `decoded`. Decision rule: `integrity`.

This is a tooling/contract deliverable. Require correct nonzero checks and bounded/source-bound output; do not invent a runtime speedup requirement. If it changes production work, record cold/build/retention deltas under [MEASUREMENT.md](../MEASUREMENT.md).

## Completion evidence

Record actual command/status/settings/counts and artifact/source/binary/fixture hashes; at least three named focused cases, no stale or zero-work pass; speed claims need MEASUREMENT.md.

Update this file and TASKS.json with status/outcome, exact artifacts and limitations. Executed evidence is recorded below.

## Executed evidence

```json
{
  "artifacts": [
    "/workspace/wtime/target/performance/baseline/manifest.json",
    "/workspace/wtime/target/performance/baseline/runs/historical-format.json"
  ],
  "commands": [
    {
      "command": "python3 -m unittest discover -s tools/wtime-bench/tests -p test_p001*.py -v",
      "exit_code": 0,
      "passed": 3,
      "failed": 0,
      "skipped": 0
    },
    {
      "command": "python3 tools/wtime-bench/baseline.py --output target/performance/baseline",
      "exit_code": 0,
      "archived_files": 85,
      "timing_rows": 210
    }
  ],
  "manifest_sha256": "7349596fc806c96c74cb9c40f1c08402589c1683660b0b152c713264f5808472",
  "limitations": [
    "Historical-format smoke uses five samples per engine/stage, not promotion evidence.",
    "Original checkout has no commit; complete core source snapshot is archived."
  ]
}
```
