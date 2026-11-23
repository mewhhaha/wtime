# Detailed contracts for high-risk tasks

Use these decisions with [implementation_contracts.md](implementation_contracts.md). Pinned normative semantics still take precedence over an implementation choice; report a concrete conflict instead of silently changing either.

## T045 — explicit frames

`Execution` owns operand slots, local slots, call frames and delimiter records. A frame identifies module/instance and function, instruction PC, operand base, locals range, caller result layout, and delimiter depth at entry. Fuel/cancellation belongs to the execution context rather than a copied counter in each frame. All guest slots carry enough value-kind information for precise roots.

For a call, validate the target and argument types before changing stacks; reserve required capacity under the frame/slot budget; retain arguments as roots; create the new frame and advance the caller PC. Execute with one dispatch loop. For return, preserve the callee results as roots, truncate callee-owned operands/locals/delimiters, pop the frame, then put results in caller order. A top-level return publishes rooted results before discarding the execution context.

For branch, use lowered destination/keep-count metadata. Move the kept suffix before truncating discarded operands; a loop branch targets its input signature, a block exit its result signature. Tail call preserves the outer caller result contract while replacing the current frame. Trap/exception cleanup releases owned stack slots, but does not roll back guest memory side effects.

T045 tests frame operations with synthetic activation records: bounded repeated pushes without native recursion, ordered `(i32 7, i64 9)` result transfer, branch temporary cleanup, failed reservation with no half-frame, and per-frame cleanup. T054/T058 add actual deep guest-call integration after call dispatch exists. Tail/EH/continuation tasks extend this same representation, not another stack implementation.

## T113 — roots and safepoints

Use explicit root enumeration, not native stack scanning. Sources are Store globals/tables, active execution slots, host root tokens, pending outcomes and traced object edges. Later tasks register exception payloads, descriptor links and detached continuation slots with the same enumeration contract before allocating those kinds.

An external root token uses a Store-owned root registry with generation-safe entries. Its lifetime rules are set in T007; cloning a token retains the root, and final release removes it. Tokens must not own a strong Store cycle. Token use after Store destruction cannot make a guest handle valid in a new Store. Internal temporary operands remain in an enumerated execution/root scope until construction and publication finish.

At a safepoint: finish temporary writes, ensure no borrowed object/memory slot can outlive collection, gather/visit roots, then run the collector. Split borrows or snapshot handles so marking can mutate mark bits safely. Drop root-registry guards before invoking any callback; never invoke guest code during root walking or sweeping. New object constructors root operands before the allocation that may trigger collection.

Required tests: sole live ref in each provider survives forced collection; a host callback reenters and triggers GC while arguments/results are retained; releasing the final token allows reclamation; dropping a detached continuation releases its otherwise unreachable local; exception/descriptor child refs survive when their owning object is rooted. The last two become active regressions when those provider tasks land; initial T113 supplies hooks and baseline providers.

## T130/T133 — atomics and waiters

The initial portable backend uses a shared SC coordinator and stable memory IDs. A process-wide `OnceLock<Mutex<...>>` coordinator is an acceptable simple implementation; if scoped to an Engine instead, prohibit sharing a memory outside its coordinator domain. Every atomic operation and fence has one linearization point while holding that coordinator. Normal accesses lock their shared memory data; bulk access acquires multiple data locks by stable memory ID. No normal access holds a memory lock and then requests the coordinator.

Order: coordinator if needed, then memory data locks in increasing ID order, then wait-queue metadata. No lock is held while executing user callbacks, parking a waiter or waiting for thread termination. Treat poisoned synchronization as a host failure/fail-closed state; do not silently expose partially updated bytes. T129 must route ordinary/SIMD/bulk access through these same data locks.

Wait key is `(memory ID, effective byte address)`. Under the required locks: check type/alignment/bounds, load current value, return `not-equal (1)` on mismatch, otherwise register a waiter before unlocking. Any negative timeout means unbounded wait; nonnegative values use a checked monotonic deadline. Park in a condition-variable loop on durable waiter state after releasing coordinator and data locks. Spurious wakes do not complete the wait.

Notify finds only this location's registered waiters and changes up to the requested count to `Notified`, returning the actual count. FIFO is not required unless the locked source requires it. Timeout/notification/cancellation races use one state transition and exactly-once queue removal. Do not hold a waiter-state lock while acquiring locks in the reverse order during cleanup.

T130 tests the coordinator/backend operations directly before opcode dispatch exists; T136 reruns the litmus cases through real Wasm atomic instructions. Required cases: two-memory store-buffering forbids both SC reads seeing zero after their own writes; fence message-passing; ordinary/shared atomic race remains Rust-safe; mismatch creates no waiter; matching value with zero timeout gives `timed-out (2)`; one notify wakes no more than one; identical aliased location shares waiters while other addresses/memories do not; repeated registration/notify race terminates with no leaked waiter or lost wakeup. Apply wait32/wait64 alignment and memory64 operands where the source permits them.

## T140–T145 — continuation ownership

A usable continuation handle has `Ready(owned stack segment)` state. Resuming/binding/switching consumes that capability exactly once. The original handle remains `Consumed`; it never becomes usable again. Active execution owns running frames. A later suspension produces a new Ready capability for the newly captured suffix. Trapped, cancelled, returned and dropped activations release their owned segment; rooted exception/results are transferred before release.

Before changing ownership, validate target state/types and reserve destination bookkeeping. At the commit point move the owned segment; do not clone it. After commit, an execution trap cannot resurrect the old capability. Pre-commit resource failures must leave a coherent owner and documented host-error outcome. T140 tests ownership mechanics without requiring every opcode.

A prompt stores its parent context, delimiter frame/operand depths and ordered handlers `(tag identity, event kind, destination)`. Search inner to outer for the nearest matching tag *and event kind*. A suspend skips a switch-only handler with the same tag. Capture only the suffix inside the matching delimiter, including nested delimiters; preserve the outer context. Deliver payload and the new continuation to the handler with its validated label signature.

Resume installs the captured segment under the supplied handler/prompt context, restores operands/locals and transfers remaining fuel/cancellation/root ownership. Normal return goes through the resumed delimiter with its result signature. `switch` also captures the outgoing suffix and transfers it to the selected peer handler. Throwing resume injects an ordinary guest exception at the captured point and uses the existing EH path. Host/native frames are never captured.

Required cases: nested same-tag prompts choose nearest correct handler; inner switch/outer suspend of the same tag chooses outer on suspend; generator yields 7 then resumes with 9; peer ping-pong retains each local/prompt; `resume_throw_ref` is caught within captured frames; uncaught exception cleans up; a second use traps; forced GC and cancellation during transfer do not lose roots or leave duplicate owners.

## T150–T153 — descriptors and exactness

Validation passes: (1) decode all group members and check index bounds/kinds; (2) verify descriptor/describes reciprocity and allowed ordering (a `describes` edge refers to the required preceding type); (3) reject cycles, nonstruct clauses and finality mismatch; (4) validate each declared subtype's complete descriptor square; (5) apply exact reference relations and instruction stack rules. Use explicit worklists and the normal type-validation budget.

For a type A with descriptor D, require D's `describes` to identify A and A's `descriptor` to identify D. For a declared child B of A, the required descriptor child relation must be present exactly as the pinned rule specifies; do not infer missing links from layout similarity. Shared-everything forms remain disabled in this core snapshot; metadata examples do not enable that proposal.

`exact T` is a subtype of `T`, but a proper declared subtype U is not a subtype of `exact T`. Apply the specified null/bottom relations separately. Constructors/ref.func that yield exact refs must use the actual defined type. An ordinary function import accepts normal compatibility; an exact function import requires equality with the actual canonical defined type. A requested supertype cannot relabel an imported subtype as exact-super. Give host functions real canonical declared signatures so the same matching tests apply.

Descriptor equality at execution is object identity, not structural type equality. Ordinary allocation of a described type is invalid; descriptor allocation requires its exact descriptor ref and roots that ref before allocation. A null descriptor may be statically admitted where specified but traps at execution.

Required cases: valid parent/child square and metadescriptor chain; mismatched reciprocal link; invalid describes order/cycle; one final and one open type; missing descriptor subtype edge; exact-super import rejects a supplied proper subtype (host and module); actual exact-super succeeds; two descriptor objects of one type compare unequal; same descriptor object compares equal. The source's field-deduplication TODO has no encoding to implement.
