# Register entry settings per side

The entry collector accepts a complete row set in either format. V1 rows use
one `strategy` and `threshold` for both executables. V2 rows declare
`baseline_strategy`, `baseline_threshold`, `candidate_strategy` and
`candidate_threshold` separately. Mixing formats in one collection is rejected.

V2 makes an eager archived baseline versus an adaptive current candidate
explicit. Each observation must match its own side's argv and payload settings,
while fixture, input, independent result oracle, invocation count, profile and
fuel remain shared. Changing either setting changes the collection identity and
prevents resuming its journal.

The subprocess protocol remains `wtime-entry-batch-v1`. The collector projects
each row into that existing protocol, so no runtime, timer or guest-call change
is needed. Pair reports use `wtime-register-entry-pairs-v2`; their analyzer uses
`wtime-register-entry-analysis-v2` and retains both settings in every row. V1
collections and analysis retain their original schema names and shared settings.

Analysis continues to use raw lifetime totals in nanoseconds. It keeps runs,
rows and call counts separate, and never divides by the invocation count.
Full evidence requires the existing independent runs, complete pairs and
bootstrap rules. Smoke reports have no ratio or confidence interval and cannot
select a policy or promote a candidate. Historical receipts retain their
original helper identities; source-current validation still rejects changed
consumed inputs.
