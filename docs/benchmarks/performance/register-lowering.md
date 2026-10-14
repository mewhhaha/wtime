# Bounded numeric register lowering

P058 translates normally validated numeric functions into P057's private format.
Locals have fixed slots; temporaries reuse slots by logical stack height. Each
original instruction retains its typed stack recipe, effect, fuel charge and
cancellation boundary. Structured control preserves block parameter/result
shapes and emits simultaneous branch-result move requests. P130 will schedule
those requests; later tasks supply production execution.

Preparation uses checked arithmetic, bounded reservations and a transactional
shared work/byte budget. Metadata failure, unsupported effects and exhausted
budgets publish nothing. The prepared program retains its source-byte/Engine
identity without retaining ModuleData, and rejects separately compiled modules.
References, SIMD, memory, calls, exceptions, continuations and unsupported
control operations still decline whole-function preparation.

The test-only slot model uses existing numeric semantics to isolate lowering
dataflow. At every reached source boundary it compares locals and typed operand
values against a fresh decoded invocation, selecting repeated loop visits with
P129. All 142 dynamic prefixes pass across numeric/local operations, shifted
multivalue results, two loop fixtures and all three nested-if paths. Exact fuel
completion and one-unit-short interruption are also checked. Malformed control
metadata/type joins and shared budget failures decline transactionally.

The [proof](P058-proof.json), [records](P058-records.json) and
[completion](P058-completion.json) bind the executed cases and source/build
identities. This is a contract-only foundation. Decoded remains the default;
Register policy remains unavailable. No register runtime speedup is claimed.
