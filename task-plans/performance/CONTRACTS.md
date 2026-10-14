# Contracts for every performance patch

These contracts supplement the [original implementation contracts](../IMPLEMENTATION_CONTRACTS.md) and [hard task contracts](../HARD_TASK_CONTRACTS.md). When a proposed optimization cannot satisfy a contract, narrow its eligible subset or decline to optimize. Do not patch the specification or assertions to accommodate it.

## C1: provenance and admission

Only normal decode/validation creates an executable module. Preserve enabled features, resource-admission provenance, pinned specification revision, Engine identity, and whole-recursive-group canonical equality. A cached/prepared object cannot import unchecked bytes, foreign Store handles, stale generations, raw nominal type indices from another module, or a weaker feature/limit policy. Hash hits require full key/content equality where collisions could affect identity. Wide indices, memory64/table64, nonstandard page sizes, and checked address arithmetic remain supported through fallback if a fast form is narrower.

## C2: source cursor and side effects

An executor cursor denotes the next original instruction to execute. Save enough state at any host call, guest call, trapping operation, allocation, collection, suspension, exception edge, and tier exit to reconstruct locals, operand order, labels/catches, logical frames, prompts, canonical context, and remaining fuel. Side effects before a trap stay visible; later side effects must not occur. Do not speculate a bounds trap ahead of an earlier store or exception. Retain the original trap category and original instruction attribution. Reserve/check optimizer metadata before publication; failed preparation must not publish partial code.

## C3: fuel, cancellation, and logical budgets

Default behavior continues to charge one fuel unit at each original interpreter instruction boundary, including structural instructions, and checks cancellation at the same boundaries. Match the current executor's operand-height and call-depth checks, including transient peaks and tail-call behavior. Fused execution with insufficient fuel falls back to a resumable source prefix rather than charging the whole group prematurely. `current_fuel`, saved continuation fuel, and reentry bookkeeping retain current behavior. Bulk operations keep their chunk checkpoints, and infinite atomic waits retain cancellation and waiter cleanup.

The P088 experiment may add an explicit positive `cancellation_quantum` with default 1. Its contract specifies a maximum number of original instructions between polls, polls before host entry and return/publication, checks at backward edges as needed, and unchanged blocking/bulk handling. This is a different explicitly reported execution policy, not an invisible optimization. Zero/unbounded quantum is rejected. Exact fuel, trap ordering, and resource limits stay mandatory in every policy. Benchmarks compare matched policies and name any unavoidable comparator difference.

## C4: roots and handles

Public values and dynamic handles remain checked. All reference values that the current execution contract retains in locals, operands, globals, tables, exception payloads, descriptors, host arguments, published outcomes, and suspended continuations remain traceable during reentrant host collection. Numeric-only temporaries may use unboxed storage; a reference slot cannot disappear into an untraced cache or machine register. Frame/operand caches flush or expose typed slots before any root-relevant boundary. A host callback may reenter the same Store; scratch pools must hand it a distinct active activation and restore the outer one on every result/error path.

Do not change the public `Value`/handle representation to raw pointers or lossy NaN tags. A future internal compact reference scheme needs generation/Store identity and exact-type preservation. Reference comparisons in differential tests normalize corresponding identities/graphs; raw numeric handle indices are not comparable across Stores.

## C5: memories, tables, and concurrency

Scalar/SIMD loads and stores use explicit little-endian representations, checked effective addresses, and the same full-width bounds/alignment rules. Safe slice access is preferred. No slice survives growth, host reentry, shared mutation, or a change of active instance. Validate the whole bounds range for bulk operations before their permitted side effects; preserve overlap/memmove direction and chunk cancellation behavior. Distinct memories may alias the same shared backing.

Shared regular, vector, bulk, atomic, and wait/notify accesses keep their existing race-safe backing. Sequentially consistent atomics must remain SC across distinct memories; removing the global coordinator requires a separate demonstrated model, not just per-memory locks or a throughput result. Wait registration, cancellation, timeout, counted notification, and resize safety remain correct. The default shared-memory tasks improve copying/queues while retaining the coordinator.

Table inline caches guard the actual target's checked identity and canonical signature. Guard table mutations if caching a table slot, or avoid slot caching entirely. All mutation routes, imported aliases, table64 indices, null targets, and host targets must be covered. Generation checks cannot be removed from public/dynamic entry.

## C6: values and proposals

Integer arithmetic wraps exactly; signed division overflow, divide-by-zero, conversion bounds, shifts, and wide arithmetic retain their rules. Float bits, signed zero, NaN allowed sets, rounding, saturating conversion, and relaxed SIMD permitted results remain specification-correct. No unrestricted fast-math switch. GC descriptor edges and exact/plain heap types retain their distinction. Cross-module canonical context remains explicit.

Exception handlers preserve tag matching and original unwind order. Continuations remain one-shot and own detached explicit guest segments. Preserve bind/resume/suspend/throw/switch, nested prompts, signatures in the correct tag/outer-frame module contexts, exactness, root ownership, byte budgets, and the existing rule that suspension cannot cross an active guest-to-host callback boundary. Tier exits occur before these operations unless their full materialization contract has landed.

## C7: budgets and cleanup

Default production retains zero external dependencies and forbids unsafe Rust. Existing hard ceilings are clean build <=60 seconds, touched CLI leaf <=5 seconds, and stripped CLI <=5 MiB on a declared reference environment. Record unchanged/release build time, RSS, module/code retained bytes, and the relative deltas from the frozen baseline too. The baseline's much smaller actual cost matters; staying barely under a generous ceiling is not enough to justify unnecessary compiler machinery.

Caches, optimizer work, code buffers, activation pools, continuation bytes, GC bytes/mark work, and all retained metadata are bounded and accounted. Budget decisions are transactional: reserve first, publish once, release on failure/drop/consumption. Existing tests for failed instantiation, imported side effects, escaped references, stale handles, and allocation failure remain acceptance checks. Optional executable mappings enforce W^X, isolate unsafe code, and have separate platform/ABI/code-validation evidence.

## C8: evidence and selection

Do not time instrumented code as production speed. Preserve raw before/after samples, fixture/oracle identities, source/binary hashes, compiler flags, settings, and outcomes. Check correctness before timing. Wasmtime comparisons require a capability preflight and the same Wasm bytes; unsupported cases are explicitly wtime-only. A candidate does not become the default because it wins a single synthetic fixture. Follow the promotion rule in [MEASUREMENT.md](MEASUREMENT.md), report failed/inconclusive experiments, and preserve universal fallback.

Completion of an experiment means its question was answered with reviewable artifacts; adoption is a separate outcome. Do not mark a deferred native child implemented when its parent decided against that backend.
