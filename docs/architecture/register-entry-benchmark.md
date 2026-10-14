# Fresh-entry Register batch benchmark protocol

P135 adds a current-only command for checking the cold entry-batch protocol. It
does not choose a Register threshold or make a performance claim. The durable
P135 outcome is `contract_only`; the CLI probes establish protocol
correctness only.

Each command builds a fresh Engine before the timer. The measured total starts
at module compilation and encloses fresh Store creation, instantiation, and
exactly the requested number of checked guest calls. It does not prewarm or
make a preliminary guest call. Engine construction, input loading, and argument
parsing happen before that interval. The counter fixture checks the final
little-endian i32 at memory offset zero against the number of calls.

The command matrix uses Register eager strategy at threshold zero and adaptive
strategy at threshold three, for 1, 10, 100, and 10,000 calls. Each matrix row
runs as its own fresh process and module lifetime. The production executable is
uninstrumented and reports whole-lifetime and phase durations in nanoseconds;
P135 preserves those raw smoke outputs without analyzing or claiming a speedup.
Separate diagnostic runs enable instrumentation and record per-call dispatch,
exit, and fallback counts. They contain no timing fields and are retained as
diagnostic evidence, never mixed with production timing rows.

The verify receipt records 15 passing test
executions, 0 failures, and
0 ignored tests, including the separate
production and diagnostic configurations, source-bound observation checker,
and Python CLI tests. The source map contains
150 runtime inputs and the verify receipt
records no default-core source changes after freeze.

P135 reuses the source-current P134 default and Register build-budget reports;
it records no new core build timing. The probes bind the executable and fixture
hashes and preserve the complete argv, stdout, and stderr for 8 production rows,
8 diagnostic rows, and 11 negative rows. The observations are archived as
deterministic gzip with mtime zero.

The final proof is [P135-proof.json](../benchmarks/performance/P135-proof.json),
with exact completion, freeze, and evidence records beside it. Machine-readable
hashes and row bindings are in [P135-records.json](../benchmarks/performance/P135-records.json).
The accepted observations archive is
[P135-observations.json.gz](../benchmarks/performance/P135-observations.json.gz).
Finalization evidence: `target/performance/P135/shared-finalization/attempt-0002/evidence.json`.
