# Source-visit diagnostics (P129)

With `execution-reference` compiled, decoded fetch diagnostics can capture a
specific dynamic visit to an admitted instruction:

```rust
store.request_materialization_roundtrip_after_visits(function, source_pc, skip_visits)?;
```

The request skips only occurrences matching the original instance, function
body and source PC. It then performs one existing snapshot/restore at the
charged-before-fetch boundary. The original `request_materialization_roundtrip`
method forwards with zero skipped visits. Counts above 100,000 fail before
guest work; guest handles and source bounds retain their existing checks.
A request stays pending until its visit is reached or a new request replaces
it. An unreached visit creates no snapshot and leaves guest execution intact.

This lets lowering tests compare every dynamic loop prefix against decoded
locals and operands. Set a source-fuel budget through the desired event and
request that PC with the number of earlier matching visits. Snapshots retain
the already-charged boundary, exact remaining fuel and typed logical state.
Check `truncated` before treating a snapshot as complete.

The P047 loop's third visit to PC 8 occurs after 35 charged events. Its locals
are `[2, 16, 8]`, operands are empty and remaining fuel is zero. The three new
cases, seven existing cursor cases and default-policy check passed. They also
cover unreached requests, invalid bounds, and identical PCs in another instance.
Both build and dependency budgets passed; the default clean build took 2.7747 s
and its stripped CLI remained 1,000,712 bytes.

The hooks run in the current decoded fetch loop and its Compact integration.
The frozen Reference dispatcher does not capture these snapshots. Diagnostics
stay excluded from default builds and provide no runtime performance evidence.
See [P129-proof.json](../benchmarks/performance/P129-proof.json),
[completion](../benchmarks/performance/P129-completion.json), and
[durable records](../benchmarks/performance/P129-records.json).
