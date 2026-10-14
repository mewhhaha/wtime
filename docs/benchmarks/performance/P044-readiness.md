# P044 readiness

P044 is **done / inconclusive**. Its eight children, P113–P120, are published
`done/contract_only`; they establish correctness foundations and make no
independent speed claims. P120's three switch/handle cases and all eight explicit
Compact P022 cases passed. The combined result is recorded in
[P044-result.json](P044-result.json) and its published proof at
`target/performance/P044/shared-finalization/attempt-0001/evidence.json`.

The integrated prefix design admits straight-line integer/local/numeric-call
work over canonical `Value` state. Before the first control, memory, or complex
operation, execution clears compact frame state at the already-charged source
boundary, recovers the original prepared `(Instruction, pc)` once, and exits
permanently to the existing decoded handler. Existing whole numeric/control
Compact functions remain unchanged. There is no whole EH/prompt CFG certificate.
P119 adds bounded shared cleanup of `Store.resume_fuel` when resumed invocation
termination abandons a segment during initial exception unwind or when a
catch-to-function-label returns with empty frames; the behavior is shared by
Decoded, Reference, and Compact policies.

The retained P044-before immediate predecessor contains 30 runtime source files;
the current runtime source tree has 31, including the new prefix module. The
combined verification and 1,800 paired warm events completed. Primary warm
latency fell 23–34% on the arithmetic-prefix workloads. Protected and cold
controls plus 35.3% relative default clean-build growth prevent promotion. The
result is inconclusive: Engine stays Decoded by default, Compact remains opt-in,
and the core has zero external dependencies. Shared-cloud build timing does not
establish that the source change caused the increase.

The shared [P044 verification contract](P044-verification.json) requires separate
`freeze`, `verify`, `measure`, and `analyze` phase receipts. Verification runs the
unfiltered default suite and unfiltered Compact suite, including all 21 named
P114–P120 cases plus the eight P022 interactions, builds the production
benchmark, runs six independent integration probes, and checks default and
optional Compact build budgets. Measurement requires 1800 source-stable paired
events. Analysis computes paired statistics and the gate decision once. Review
all phase receipts and the completion record before finalization and separate
publication.
