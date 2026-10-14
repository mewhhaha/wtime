# Runtime hardening evidence

Guest values never become native pointers. The four libraries forbid unsafe
code; generation-checked arenas reject foreign and stale handles. The decoder
bounds lengths before allocation. Validation has explicit work budgets, and
execution has fuel, explicit frame limits and a cross-thread cancellation token.
Shared-memory bytes stay behind synchronization; wait/notify uses a synchronized
waiter protocol. Failure-path tests cover cancellation, failed instantiation,
partial imports, one-shot continuation state and collection of detached frames.

Collect native execution evidence with:

```
python3 tools/checks/hardening.py --native-report target/native-x86_64.json
```

Run the same command on a native aarch64 machine, then compare both reports:

```
python3 tools/checks/hardening.py target/native-x86_64.json target/native-aarch64.json
```

The comparison requires nonempty successful native runtime tests and matching
source digests. Cross-compilation and emulation do not satisfy this gate. CI
runs both native GitHub runners and verifies their artifacts together. This
workspace supplies only x86_64 execution; an unrun CI job is not passing evidence.

The reproducible bounded decoder fuzzing and independent Node/V8 differential
commands are described in [the tooling guide](../../tools/checks/HARDENING.md).
Those tools are development dependencies, never part of the core runtime. Fuzz
oracles distinguish malformed, invalid, disabled and resource-limited inputs;
a budget rejection does not prove semantic invalidity. Optional Miri requires a
compatible installed nightly and is not silently counted as run.

The allocator test is distinct from a configured budget rejection. Build
`wtime-allocation-check` and run `python3 tools/checks/allocation_failure.py`.
A process address-space limit of 128 MiB forces a permitted 512 MiB request to
fail in `try_reserve_exact`; the worker checks the resource error and then
creates and accesses a one-page memory under a one-object store budget.
This proves the failed construction left no partially registered memory. The
report includes the actual worker exit/output and its executable fingerprint.

Rooting includes GC, external, function and exception references through
`root_value`; owned external resources are reclaimed exactly once. Aggregate
store memory/table/object budgets, canonical type storage, collector work and
captured continuation bytes are bounded independently. Host concrete type
indices require module context. Failure cleanup traces indirect references
through GC and exception payloads before reclaiming a failed instance.
