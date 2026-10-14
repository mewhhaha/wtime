# Numeric register format (P057)

P057 is complete as a contract foundation. The feature-gated format has Move,
Const, Unary, Binary, Jump, Branch, Memory and Exit tags, checked u32 slot/table
indices, and private construction from normally admitted Wasm. Numeric locals
occupy fixed slots; temporaries use logical operand heights. Each source
boundary retains its actual numeric types, one fuel debit, cancellation
checkpoint, effect, resolved successors and materialization recipe. Physical
slot count does not replace the guest operand limit.

Programs retain Engine identity, body/type identity and the admitted source
bytes Arc. They retain no ModuleData Arc or Store handles. A separately compiled
module with identical bytes cannot reuse a program; clones of its original
owner can. Memory tags are defined for P063, while current admission declines
memory, references, vectors, calls, GC, exceptions, continuations and unsupported
control forms. These declines execute no guest work.

Preparation shares a 100,000-work and 1-MiB metadata budget across functions.
Construction counts actual vector capacities and temporary control-label
storage, verifies complete source ranges and canonical numeric operand/result
types, and commits retained bytes/work only after validation. Failed ownership,
metadata, allocation or budget checks leave the shared budget unchanged.
The materialization view maps a pending source boundary to fixed locals and
ordered logical operands; later execution supplies their values.

The audited existing P047 loop produces 25 source events, five slots and a
logical peak of two, retaining 2,480 bytes with 252 preparation work units.
Four focused cases passed with `register-executor,execution-reference`; the
default-policy case also passed without features. They cover unsupported
admission, independently corrupted metadata, source ownership and aggregate
budget exhaustion. Default and register-feature clean builds took 2.6394 s and
2.7069 s; both stripped CLI artifacts were 1,000,712 bytes. All build and
dependency budgets passed.

This is an IR format verifier, not a dataflow equivalence proof or an executor.
P058 owns trusted stack-to-slot translation and source-prefix comparisons.
The Register execution policy remains unavailable. No performance promotion
or guest runtime speed claim follows from these format checks.

See the [contract](P057-verification.json), [proof](P057-proof.json),
[completion](P057-completion.json) and [durable records](P057-records.json).
