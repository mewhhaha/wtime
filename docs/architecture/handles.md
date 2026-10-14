# Index and handle ownership

`wtime-types` contains distinct module-local index types. These refer only to a
validated module's index spaces, never host addresses or Store arena slots.
Canonical type identities include an engine identity and normalized recursive
group position. Object descriptor identity is separate from type identity.

The runtime exposes opaque `Func`, `Memory`, `Table`, `Global`, `Instance`,
`Tag`, `ExceptionRef`, `ExternRef` and `GcRef` values. Their internal fields are
private. Copies retain identity. Every public operation checks the owning Store
and generation before inspecting storage; foreign handles and retired objects
produce a structured error. There is no public integer-to-handle constructor.
`create_extern_ref(identity)` creates a fresh host-reference object; its payload
is a host identity value, not a heap pointer or an arena index supplied by the
caller. Guest i31 references are specified small integer values, not handles.

Generational arenas retire slots instead of wrapping an exhausted generation.
GC is nonmoving and iterative. Host code must retain a root token when keeping
GC values across collection, and release published results when they no longer
need retention. Cloning a root token independently keeps its root alive.

A shared-memory capability can be imported into another Store from the same
engine; it creates a new local checked memory handle. Ordinary handles, reference
heaps, exceptions and continuations are not shared by that capability.

Tests exercise foreign Store access, copied identities, stale handles, generation
retirement, descriptor cycles, deep tracing and joint exception/GC roots. See
[embedding.md](../embedding.md) for executable ownership examples.
