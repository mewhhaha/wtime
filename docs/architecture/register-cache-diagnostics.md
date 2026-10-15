# Register cache diagnostics

With both `register-executor` and `execution-reference`,
`Module::register_cache_snapshot()` copies an adaptive Register cache's numeric
state. Ordinary default and eager modules have no adaptive cache and return
`None`. The snapshot is diagnostic evidence; it contains no timestamps and
cannot establish a performance improvement or select a threshold.

Rows identify defined function bodies, excluding imports. Their order is the
cache's FIFO order. Each row reports its entry count, preparation attempts and
Cold, Compiling, Ready, Cooldown or Negative state. Ready rows report retained
bytes; cooldown rows report the independent retry countdown.

Lifecycle counters distinguish admitted preparation leases from actual builder
calls. A wrapper allocation failure can increment preparation starts without
calling a builder. Completed preparations can fail publication, so completed
and published are separate counters. Declines, exhausted budgets, invalid
preparations, aborted leases and actual FIFO evictions have separate counters.
All counters saturate.

The getter takes the state lock before the ledger lock, reserves a bounded
numeric row vector and copies values without cloning a prepared artifact.
Snapshots therefore do not pin artifacts or delay refunds after eviction. Work
figures are settled ledger values; an active builder's work is settled when
its lease finishes. Owner mismatch, poisoned locks or allocation failure return
`None`.

P138 verifies threshold transitions, single-builder admission, 66-body guest
churn, eviction and recompilation, pinned artifact lifetime, retry cooldown,
negative results, aborts, publication refusal, poisoned locks, saturation and
every fuel prefix of the uniform fixture. The instrumented traces remain
separate from production benchmark timing.
