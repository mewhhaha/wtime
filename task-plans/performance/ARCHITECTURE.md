# Execution architecture and experiment boundaries

The current four libraries remain the portable foundation: `wtime-types`, `wtime-decode`, `wtime-validate`, and `wtime-runtime`. CLI, conformance, and benchmarking stay outside their dependency closure. Start new executor modules inside `wtime-runtime`; a module is enough to separate ownership. Add a crate only where it establishes a useful compilation or unsafe-code boundary, and measure the resulting build graph.

## Immutable preparation, mutable execution

Make `Module` a cheap immutable shared handle. Its decoded/validated data, canonical identities, control metadata, and future execution artifacts belong to one Engine-bound payload. Never clone control tables or type vectors to fetch an instruction. Store-specific function, memory, table, and global bindings belong to an instance execution plan, not to Engine-wide cached code. Host callbacks and suspended frames may retain the immutable payload safely.

The initial executor remains the semantic baseline. A development-only reference executor and an archived pre-change binary support differential work; neither is a new default compiler dependency. Reusing low-level numeric helpers is acceptable, but a common helper bug requires independent normative cases and the external oracle where supported.

## Three portable execution representations

**Decoded execution** retains source instructions and is the universal fallback. **Compact execution** carries small fixed instructions and indexed side tables for wide constants, branches, catches, and resume handlers. **Register execution** starts with eligible numeric functions, assigns virtual slots to stack values, and implements explicit control transfers. Unsupported operations cause function-entry fallback or a verified state-materializing exit. No executor may partially execute an operation and then restart it in another representation.

An `ExecutionCursor` identifies logical function/frame, original source PC, prepared-code PC, and the next unexecuted original operation. Optimizer metadata records source-operation expansion, effect order, source trap attribution, fuel cost, maximum transient operand height, and materialization points. Keep this metadata simple and bounded; it is not a general SSA optimizer or a per-proposal IR.

Fusions and register slots must preserve source event order. Fuel remains charged for original Wasm instructions, including instructions eliminated by constant propagation or inlining. Logical call depth and operand limits remain observable resource policies even if physical frames disappear. The default cancellation observation points stay unchanged. A separately configured bounded polling policy may be investigated, but it must be explicit in API settings and every report; never compare unlike policies as if identical.

## Cold code and tiering

Prepare the cheapest usable representation first. Collect bounded function-entry/backedge counts only when tiering is enabled. Initially promote at a function boundary, avoiding OSR state reconstruction. Later trace/OSR tasks must prove state equivalence at side exits. Retained prepared code, optimizer work, cache entries, counters, and native mappings have explicit aggregate budgets and cleanup owners. Optimizer allocation or compilation failure falls back without consuming guest work or weakening admission limits.

Use measured preparation cost and per-call savings to select thresholds. The choice is workload-sensitive: a hot integer function can justify an extra tier; a one-shot tiny module may never recover it. Report cold latency, warm latency, memory, and build cost together.

## Optional branches

Research tasks include block tapes, bounded generated superinstructions, interval-certified memory regions, guarded scalar traces, proven loop summaries, independent-instance batching, prepared-module caching, and safe Rust source AOT. Start each experiment with a deliberately small eligible subset and a deterministic decline path. No benchmark-name recognizer, host shortcut keyed to fixture hashes, or opaque dependency substitution counts as an optimization.

The native branch starts only after its decision task returns `go`. Its initial subset is x86_64 Linux integer blocks and loops, with explicit host returns and safe checked helpers. No native guest call stack, signal-based bounds traps, GC allocation in generated code, exception unwinding across native frames, or native continuation capture is required. Unsupported functions retain portable execution. The native decision task permits at most two additional internal libraries, zero preferred/one maximum justified external native dependency, and keeps native work out of the default compilation closure. Native SIMD and AArch64 are separate conditional tasks.

Copy-and-patch is an alternative native emitter, not a second unconditional backend. LLVM or an assembler may be used as a pinned offline template-generation tool only if reproducibility, licenses, bootstrap cost, and distribution requirements are documented; no such compiler becomes a production dependency. A safe Rust AOT generator can remain an isolated tool and use normal Rust compilation, trading offline build cost for reusable execution artifacts.

## Parallel ownership

Measurement workload files can be added in parallel after their shared schema lands. Scalar memory, numeric arithmetic, GC metadata, and continuation accounting own different modules, but changes to `lib.rs`, dispatch, common frame layouts, configuration, and test registration are integrated serially. Task dependencies encode semantic prerequisites; an overlapping owned path still prevents simultaneous edits. `TASKS.json` records paths and `INDEX.md` separates lanes.

Read the [shared semantic contracts](CONTRACTS.md) and [concrete backend handoff recipes](BACKEND_CONTRACTS.md) for the exact boundaries an optimization must retain.
