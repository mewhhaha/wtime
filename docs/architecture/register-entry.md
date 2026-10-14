# Opt-in lazy Register entry

The `Engine::with_register_entry_threshold` constructor enables a positive,
caller-selected entry threshold for Register preparation. It selects the
existing Register policy and remains behind the `register-executor` feature.
The ordinary `Engine::with_execution_policy(..., Register)` path still builds
the eager `PreparedModule`; the default constructor and default policy do not
create a Register cache.

With the opt-in constructor, compilation attaches one bounded preparation
cache to the `Module`. Cloned modules and separate `Store`s share that cache.
Calls below the per-body threshold use decoded execution. When the threshold
is reached, the cache may publish the prepared body under the P133 resource
and ownership rules. This entry behavior preserves existing execution policy,
guest fuel, cancellation, host effects, Store ownership, and per-Store memory
isolation.

Register activation takes one `Arc` pin to the cached artifact and keeps it on
the frame. Dispatch borrows the pin without cloning an `Arc` per instruction,
so eviction can remove the cache's entry while an active frame safely retains
its prepared function and its metadata charge. A materialization handoff
reconstructs the decoded frame state and drops the Register pin. Continuation
resume then continues from its saved decoded state without looking the body up
in the entry cache again.

P134 checks cold and hot entry behavior, cross-Store module sharing and Store
ownership, guest-visible effects and resource behavior, eviction with a pinned
activation, and continuation resumption without a new cache lookup. P134 is a
correctness and build-budget result only. It makes no runtime speed claim; P066
retains the measured production integration and performance gates.
