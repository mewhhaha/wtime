# Embedding and ownership contract

The four production libraries have this dependency order: types → decode →
validate → runtime. The runtime uses decoder definitions re-exported by the
validator. Decoded `Instruction`s are 32 bytes and carry no source span: a
`FunctionBody` keeps the absolute start offset of each instruction in
`offsets`, and the rare large immediates (`try_table`, `resume`) are boxed.
Only validation can construct `ValidatedModule`; its decoded module
is exposed through an immutable reference. `Module` is immutable and reusable.

The initial concrete API is:

```rust,ignore
Engine::new(Config) -> Result<Engine>
Engine::compile(&self, bytes: &[u8]) -> Result<Module>
Store::new(engine: &Engine) -> Store
Linker::new(engine: &Engine) -> Linker
Linker::define(&mut self, module: &str, name: &str, value: External) -> Result<()>
Linker::instantiate(&self, store: &mut Store, module: &Module) -> Result<Instance>
Instance::export(self, store: &Store, name: &str) -> Result<External>
Store::invoke(&mut self, function: Func, arguments: &[Value]) -> Result<RunOutcome>
```

`Result` uses the shared structured `Error`. Returned values and guest exception
objects are separate `RunOutcome` variants. Decode, validation, disabled feature,
resource policy, link, guest trap, and host failure categories survive formatting.
Runtime handles have private fields and are checked against their owning Store.
Module-local indices and engine-local canonical type identities are distinct.

All guest calls execute on explicit heap frames. Loops branch with parameter
arity; block exits branch with result arity. Tail calls replace a frame. Host
callbacks release short-lived Store borrows before reentry. A failed guest call
must not panic or expose partially initialized instance exports.

Memory effective addresses are checked in u64 before host conversion. Access is
little-endian. Memory/table growth returns the specified width-dependent failure
sentinel. Bulk operations precheck complete source and destination ranges before
writes. Shared memory never aliases non-atomic Rust references across threads;
all atomic operations use the shared SC coordinator. No production unsafe code
or third-party dependencies are permitted.

GC references are opaque, nonmoving, and generation-checked. Mark/sweep follows
explicit roots and object fields rather than native memory or reference counts.
Canonical recursive groups compare normalized complete structure. Hash equality
alone is insufficient. Descriptors participate in canonical identity while
object descriptor identity remains a separate runtime relation.

The detailed semantic invariants are in [implementation_contracts.md](implementation_contracts.md)
and [hard_task_contracts.md](hard_task_contracts.md). Release verification evidence is indexed in
[release-manifest.json](release-manifest.json).
