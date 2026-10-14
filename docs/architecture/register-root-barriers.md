# Register root and capture barriers

The opt-in Register executor now selects numeric prefixes before reference,
GC, exception and continuation transitions as well as calls. Each prefix ends
at its first unsupported source instruction. The independent IR verifier
requires an exit at that boundary, checks every preceding instruction against
the numeric prefix whitelist, and verifies complete typed locals and operands.
Reference and vector function layouts continue to decline.

The P064 handoff materializes the active numeric bank before the original
Decoded transition. Collection, exception unwinding and continuation capture
therefore use canonical frame state. A saved frame resumes at its next original
source PC in Decoded; it does not repeat the Register prefix or its effects.

The root test collects while a reference-bearing outer caller holds the sole
object root and a numeric child has exited before its host call. Nested guest
invocation and instance growth preserve that object. Separate GC and reference
creation cases verify actual Register work before their source barriers.

Cross-module suspension fixtures retain numeric local 13 and operand 99 at
source PC 5, then resume to 112. Exact and ordinary reference variants reject
a distinct nominal object without consuming the continuation. Source snapshots,
saved fuel, cancellation, foreign handles, one-shot consumption and frame/byte
budgets are checked. Numeric exception payload 20 survives both caught and
unhandled transfers after an actual Register exit.

The byte-budget case exposed excess capacity in captured type-context vectors.
Those vectors now retain exactly one entry per declared value, matching the
continuation header's admission estimate. A budget one byte below the recorded
capture size refuses the transfer before publication.

P065 passed 713 test executions, including unfiltered Register/reference and
combined Compact/Register/reference suites, plus all 89 tier integration checks.
The two conformance suites executed 240 directives across Reference and Register
with no failures, skips or unsupported cases. This is a correctness gate with
no speed claim. Evidence is indexed by
[P065-records.json](../benchmarks/performance/P065-records.json).
