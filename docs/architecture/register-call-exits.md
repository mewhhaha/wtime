# Register call exits

The opt-in Register executor can prepare a checked numeric prefix of a
call-containing function. Whole-function preparation remains preferred. A
prefix stops before its first unsupported source instruction; the independently
verified terminal recipe identifies that instruction and its complete numeric
locals, operands and logical labels. Reference and vector layouts still decline.

The invocation loop charges the pending source boundary once. At the terminal
recipe, the executor materializes its raw slot bank into the same active frame
and operand stack, drops its Register activation, and executes the pending
instruction through the original Decoded dispatcher in that iteration. Frame
base, outer frames, prompt state and fuel remain in the existing invocation.
Execution does not restart or return to Register in that frame.

Preparation retains the shared work, metadata and activation limits. Prefix
scans are charged transactionally, and declined attempts remain bounded by the
module's attempted-work budget. Memory-only unsupported bodies retain their
whole-function fallback. The default and Portable policies remain Decoded.

The semantic cases compare every direct-call fuel prefix and eleven complete
caller source snapshots, including the pending call and decoded continuation.
They compare complete memory images and use a callback write to detect a
replayed guest store. Reentrant collection and nested invocation preserve
callback-visible locals, arguments and remaining fuel. Indirect bounds, null
and type errors, tail calls, reference calls, recursion depth, cancellation,
owner guards and preparation limits are checked against Decoded execution.
Combined Compact/Register builds repeat the five focused cases.

P064 completed 300 test executions and 1,500 physical benchmark pairs across
two runs. Its warm target ratios were 0.99281 and 0.99669, with confidence
intervals spanning one; several small cold-call cases regressed. The gate
rejected promotion. This handoff remains a nondefault prerequisite for root,
capture and lazy-tiering work. The immutable decision and artifact identities
are in [P064-records.json](../benchmarks/performance/P064-records.json).
