# Runtime architecture

Status: `v0.1.0` core interpreter architecture.

## Scope and design choices

Build a portable **interpreter for Core WebAssembly** in Rust. Implement the core portions of every finished proposal and every active proposal at Phase 3 or later in the pinned catalog. Browser and JavaScript embedding APIs are excluded. See [scope.md](scope.md) and [spec-lock.json](spec-lock.json) for the 24 included entries (including MVP and tooling-only annotations), five excluded JS/browser entries, and the partial core-only scope of custom descriptors.

Initial native targets are 64-bit Linux x86_64 and aarch64. Keep byte order, guest widths and reference representation portable; host-address limits are resource limits, not alternate guest semantics. Portable software SIMD and explicit checked accesses avoid native SIMD/JIT prerequisites. A 32-bit-host port, no_std, WASI, component model, JS engine integration, JIT/AOT, optimizing IR, debugger protocol, custom allocator and package ecosystem are outside this goal. WASI is an embedding interface, not implied by core conformance.

Interpretation is a deliberate implementation strategy, not a temporary second backend. It gives fast Rust builds and low Wasm-module preparation latency: decode, validate and lower to simple instructions, without optimization passes. Start with explicit instructions plus side tables; measure before compact bytecode, opcode fusion or register allocation. No Cranelift, LLVM, Tokio or native stack-switching dependency.

## Small crate graph

| Crate | Owns | Internal dependencies |
| --- | --- | --- |
| `wtime-types` | Indices, syntax/value/type shapes, feature policy, limits, diagnostic categories | None |
| `wtime-decode` | Bounded binary reader; immutable owned bytes/range views; decoded module | types |
| `wtime-validate` | Type checking, control-flow evidence, sealed ValidatedModule | types, decode |
| `wtime-runtime` | Engine, canonical type registry, Store, Linker, lowering, interpreter, GC, shared memory | types, validate |
| `wtime-cli` | Thin binary inspect/validate/invoke commands | runtime; decode/types only where needed |
| `wtime-conformance` under `tools/` | WAT/WAST, spec harness, reports, optional differential adapter | runtime + dev/tool dependencies |

This is four libraries, one small CLI and one separately built test tool, not a crate per proposal. Runtime accesses validated syntax through validator-owned/re-exported interfaces instead of acquiring a separate parser implementation. The types crate owns the small shared syntax vocabulary; decoder-specific storage stays in decode, validated evidence in validate, executable instruction layout in runtime. Do not put Store implementations, a global opcode macro expansion or a giant generic AST framework in types.

Make core/CLI the Cargo default members. Keep the conformance runner out of default builds and out of core unit-test dev-dependencies. `cargo test --workspace` intentionally includes tools and is not the fast developer command. No cyclic dependencies.

## Dependencies and build cost

Target **zero third-party production dependencies in the four core libraries and CLI**. Use std containers, synchronization, `Error`/`Display`, checked arithmetic and manual CLI parsing. Use `wast` only in the separate conformance tool for text assembly/script parsing. An optional `wasmparser` oracle belongs to an explicit tooling feature. Their transitive dependencies are counted and recorded, not disguised by counting only direct dependencies. Reuse existing `wasm-tools` as a developer executable where useful; never route runtime execution through it.

Prefer concrete values and dynamic host-call slices over recursively generic visitors, generic tuple host adapters and generic interpreter backends. Keep helpers private; avoid indiscriminate `#[inline]`, proc macros, build scripts and generated mega-matches shared across crates. Split the runtime internally by memory, table, types, control, numeric, vector, GC, EH, threads and continuations; those are Rust modules, not more crates.

Initial measured targets on the current 5-CPU machine: clean core+CLI dev build <=60 seconds; representative touched-leaf incremental rebuild <=5 seconds; stripped release CLI <=5 MiB. These are provisional acceptance budgets, **not measured results or performance promises**. T009 establishes the exact baseline and protocol; T167 enforces/adjusts them with evidence. Track no-change build, peak RSS and tool build cost separately. Use normal incremental dev compilation, no dev LTO, and limited build parallelism (start with four jobs). Keep release LTO optional until measured. Also measure Wasm decode/validation/lowering time and retained module bytes; Rust build speed does not imply fast guest execution.

## Module pipeline and trust boundary

1. A bounded reader decodes untrusted bytes into immutable owned storage with offsets. Preserve legal nonminimal LEBs; reject overflow and invalid unused bits. Never allocate from a count without checking a policy limit and enclosing byte range.
2. Validation establishes full syntax/type/control/feature invariants. It distinguishes malformed input, invalid modules, disabled features and resource exhaustion. It produces a sealed ValidatedModule; no public unchecked constructor.
3. Engine preparation canonicalizes recursive types and lowers validated bodies to private executable instructions and branch side tables. Lowering is linear/bounded; code offsets do not become native pointers.
4. Linker resolves explicit host/module definitions; instantiation allocates state, initializes segments, then runs the start function. Preserve specified imported-state side effects on failure while cleaning up private allocations.

Raw decoded syntax, validated facts and executable code remain distinct. Validation never trusts an external parser or oracle. The interpreter executes only prepared validated modules. Feature configuration is frozen with each module and checked against its Engine. Do not substitute fresh validation profiles after preparation.

Use a single reviewable opcode/encoding ledger with each instruction's immediate, typing, execution and tests. The ledger also covers non-opcode changes: type sections, compact imports, custom pages, exact imports, annotation grammar and metadata. Do not turn it into a giant generated framework just to remove a few duplicate matches.

## Values, stores and references

Values preserve integer/float bits and use a portable 128-bit vector representation. Guest references are typed handles, never raw Rust pointers. Generational arenas and Store/Engine identities catch stale/foreign handles. Define generation exhaustion rather than wrapping to a reusable live identity.

Engine owns immutable compiled modules and an immutable/canonicalized recursive-type registry. Canonicalization respects group structure, declared subtype relationships, descriptor links and exact heap types. It must not collapse merely similar-looking, nonequivalent type graphs. Store owns mutable instances, unshared memories, tables, globals, GC objects and execution state. Ordinary GC references and continuations remain store-local.

Host functions accept typed slices and explicit Caller access. Release outstanding mutable borrows before reentry; root arguments/results across callbacks. Host code receives explicit capabilities, not implicit filesystem/network/WASI access. Separate guest traps, guest exceptions, host failures, cancellations and Rust panic policy.

## Execution stacks, exceptions and continuations

Guest calls and control flow use heap-owned operand/local/frame stacks; no recursive Rust execution. Frames identify module/function, program counter, slots and delimiter state. Branch side tables perform exact stack adjustment, including multivalue loop inputs and block results. Tail calls replace frames.

Unify the **ownership model**, not the semantics, of control delimiters: EH handlers and continuation prompts are explicit typed records. EH uses traced exception objects and explicit frame unwinding, not Rust panic unwinding. Stack switching owns delimited stack segments, parent/peer links and one-shot consumption state. Implement cont.new, cont.bind, resume, suspend, switch, resume_throw and resume_throw_ref against the pinned rules. A vector of captured frames alone is insufficient: prompt search, handler kinds and delimiters must survive.

Never capture arbitrary Rust frames across a suspension. Define the Rust embedding's suspension/reentry policy: default to synchronous host boundaries, adding explicit resumable tokens only if needed. That host policy is an API design choice, not a protocol standardized by the Core proposal. Fuel, cancellation, roots and allocated-byte accounting survive suspension. Closing/dropping consumed, trapped or abandoned continuations reclaims their stacks and removes waiter/host registrations.

## GC and type extensions

Use a simple nonmoving, stop-the-world tracing collector per Store. Start with iterative mark/sweep, explicit root enumeration and a bounded worklist. Do not use reference counting as the only collector: cycles are legal. Root globals, tables, active frames/locals/operands, host root tokens, pending results, exception payloads, descriptors and suspended continuations. State safepoints and no-GC borrow regions explicitly.

Support the full GC heap-type hierarchy, packed fields, i31, struct/array instructions, array segment operations, casts and reference conversions. Defined types and recursive groups require both structural equivalence and declared subtyping checks.

Custom descriptors extend this model with descriptor/describes links, complete-square/finality constraints, exact heap types, exact function imports, traced descriptor headers, descriptor allocation and identity-based cast/branch operations. Core implementation does not include JS prototype configuration. The pinned proposal's field-deduplication section is a bare TODO; it has no invented implementation task. A future normative definition requires an explicit source refresh and task delta.

## Linear memory, tables, SIMD and atomics

Carry `u64` guest limits and indices and an explicit i32/i64 index type throughout memory **and table** operations. Check effective-address addition/multiplication before bounds and usize conversion. Test huge guest addresses using small real allocations. Memory64 includes table64 and the width of indirect-call indices, table operations and element offsets.

Each memory carries its page size. The current custom-pages draft permits only 1 and 65536 bytes; do not accept every power of two. size/grow return page counts, not byte counts. Import matching includes equal page size. Compose memory width, sharing and page-size flags explicitly and resolve inconsistent inherited prose against pinned formal rules before implementation.

Normal accesses use explicit little-endian operations; unaligned ordinary loads/stores are legal. Bulk copy has memmove behavior and specified precheck/side-effect ordering. SIMD uses portable lanes first. Float/NaN semantics and relaxed SIMD result sets require deliberate tests; ordinary Rust casts, min/max or fused arithmetic are not automatically Wasm-correct.

For Threads, share only explicit memory handles and immutable modules among separate Stores. Start with synchronized shared byte storage and an atomic-order coordinator that imposes the required global sequentially consistent order, including fences. Ordinary, vector and bulk accesses to shared bytes must also be race-safe. No mixed-width aliased Rust atomics over an unsynchronized byte vector. Document a global lock order, bound critical work and define poisoning policy. Wait value-check and queue registration are atomic with respect to notify; blocking releases memory/order coordination and uses a stable memory/address identity. Separate Stores do not imply shared-everything GC.

## Feature policy and verification

Cargo features select only genuinely optional tooling/backend packaging. Proposal support uses runtime FeatureSet profiles: finished stable core and all_phase3_core, with validated dependencies. Feature-disabled instructions, type forms, imports and initializers all reject consistently. The final scope is satisfied only by the full selected profile, not by claiming optional features are unsupported.

Use three layers of evidence: focused unit tests for invariants and edge cases; immutable upstream WAST corpora with all assertion kinds and accurate counts; interaction/fuzz/differential checks for seams. A zero-test run, ignored directive or unknown-feature skip never means pass. Distinguish malformed/invalid/link/instantiation failures, traps, exceptions, resource limits and expected NaN/relaxed result sets.

Core CLI consumes .wasm. WAT and annotations are supported by the tool-only encoder/harness rather than a second handwritten production text parser. If wast lacks a selected draft, its gap is a real task: pin a supported revision or add a small isolated tool extension with tests. Do not silently remove those scripts. Optional external oracles need not implement every draft; they cannot define this engine's normative semantics.
