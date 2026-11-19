# Embedding wtime

Use Rust 1.99.0 and the four production crates. The runtime has no external
production dependencies. The [native examples](../examples/README.md) compile
against the public API. Proposal support refers to the [pinned core scope](architecture/scope.md),
not an automatically updating WebAssembly version. WASI, browser/JavaScript APIs,
and JIT compilation are outside this implementation.

## Engine, module, store and imports

An `Engine` fixes feature policy and resource limits. `Engine::compile` decodes
and validates a binary into an immutable `Module`. Modules can be reused with
stores from the same engine. A `Store` owns instances, memories, tables, globals,
functions and references. Handles are opaque and checked for store ownership
and generation; handles from another store are errors.

Create a `Linker` for the engine, register imports with `define`, then call
`instantiate(&mut store, &module)`. Import matching checks structural/canonical
types and limits; exact function imports also require exact type identity.
Instantiation runs the start function. Imported-state writes made before a
failing start function remain visible; failure does not roll back host state.
Only successful instantiation publishes its instance handle.

`create_host_func` accepts a `FuncType` and a checked callback returning typed
values. `create_host_func_with_caller` supplies a temporary `Caller` for reentry
and collection; `create_host_func_with_outcome` can return a guest exception.
For signatures containing concrete module reference types, use
`create_module_host_func(&module, type_index, callback)`; it supplies the type
context before instantiation. Unbound host APIs reject concrete type indices.
Use `create_module_tag` for the corresponding concrete tag payload context.
The runtime checks argument/result counts and types. A host error stays distinct
from a guest trap. Never retain a `Caller` beyond its callback.

## Outcomes, limits and cancellation

`Store::invoke` returns `Returned`, `Exception`, or `Suspended`. A suspension
contains its tag, payload and one-shot continuation. Resume through a guest
function whose signature accepts that continuation type, as the suspension
example does. Reusing a consumed continuation traps. Uncaught exceptions are
objects, not Rust panics. Invalid binary, disabled feature, link error, resource
limit and guest trap remain separate structured error categories.

`Config::limits` bounds module size, type work, locals, stack/call depth, memory
pages/bytes and table size. Aggregate store object, memory and table budgets
prevent many individually small allocations from bypassing limits. Canonical
types, GC storage, mark work and captured continuation bytes have separate
budgets. Fuel bounds guest instruction execution. Memory byte
budgets are independent of custom page size. A memory64 address is still checked
against its allocated byte length before conversion to a host index; choosing
64-bit indices does not allocate the entire address space. Table64 uses i64
indices and the same explicit element budget. Failed guest growth returns the
specified failure sentinel.

An unshared linear memory is stored as 64 KiB pages that are allocated on the
first write to them; unwritten pages read as zeros. Host memory use therefore
follows the bytes a guest writes, not its declared or grown size: a module with
a 4 GiB initial memory, or a memory64 module that grows to many GiB but touches
little of it, does not reserve or zero that much host memory. Memory byte
budgets still count the full logical size. Shared memories keep a contiguous,
zero-filled backing.

Obtain a `CancellationToken` from the store and send its clone to another host
thread. `cancel()` requests cancellation at guest instruction boundaries;
`reset()` permits later calls. Cancellation is a resource outcome and cleans
up active frames. Guest atomic waits poll cancellation at 20-millisecond intervals
and remove their waiter registration when cancelled. An arbitrary blocking host callback must implement its own
cancellation policy.

## Reference lifetime and collection

GC objects and detached continuation frames are nonmoving and generation
checked. Guest frames, tables, globals, exception payloads and published return
values keep their referenced objects alive. For a GC reference retained by host
code, keep the `RootToken` returned by `Store::root` alive. `root_value` returns
a `ValueRootToken` for any supported reference family, including external and
exception references; cloned tokens retain the same root until the final clone
is dropped. Owned external resources are dropped when collection finds them
unreachable. When done retaining
published results, call `release_published(&values)`; simply dropping the Rust
`Value` does not notify the store. Dropping a root token removes that explicit
root. Collection traces references through descriptors, exception objects and
continuation captures. A stale reference is rejected, never dereferenced as a
native pointer.

## Concurrency

Use separate stores on separate host threads. Ordinary handles cannot cross
stores. `share_memory` produces a shareable capability; `import_shared_memory`
creates a checked local handle in a store from the same engine. Shared accesses
use synchronized backing storage, with a shared coordinator for sequentially
consistent atomics and wait/notify. Ordinary accesses do not expose an unsafely
aliased Rust slice. Reference heaps and continuations are store-owned and are
not transferred with shared memory.

See [architecture/api.md](architecture/api.md) for the compact interface
contract and [release-readiness.md](release-readiness.md) for measured gates and
remaining evidence requirements.
