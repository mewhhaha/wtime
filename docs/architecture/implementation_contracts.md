# Contracts for implementation with Sol 6.1 Low

This file fixes the initial choices that later tasks must consume. It is a plan, not existing code. T002 writes these contracts into repository documentation; T003 creates the package/module skeletons. If a precise signature must change for correctness, change the contract task and its consumers together rather than building a parallel API.

## Execution model and task workflow

Implement with `gpt-6.1-sol`, reasoning `low`, once the user starts implementation. Use Luna for exploration and mechanical chores. Every task lists prerequisite outputs, expected paths, a concrete recipe, test cases and commands. Read the prerequisite files first. Do not attempt to solve later proposal work while implementing a baseline task; establish the extensible data shape, implement the assigned subset, and reject disabled/unsupported encodings explicitly.

For each task: inspect current interfaces → add its smallest failing fixture → implement its named behavior → run targeted tests → run changed crates' checks → record evidence. Do not rename modules or introduce a new abstraction just to make the task look self-contained. A task may extend an existing module rather than create a new file at every listed path.

A test named `tNNN_...` belongs to TNNN. Test commands filter on that prefix; verify the reported count is positive and includes the intended cases. Core fixtures are raw .wasm byte arrays or checked-in binaries assembled separately. WAT/WAST parsing lives only in `wtime-conformance`, never in core dev-dependencies. All tasks can add regression cases to a shared feature test file as long as names retain their task prefix.

## Package and module conventions

```text
Cargo.toml                       # resolver 3; core+CLI default-members
rust-toolchain.toml              # 1.99.0 initially, rustfmt/clippy
crates/wtime-types/src/          # error.rs, features.rs, types.rs, indices.rs
crates/wtime-decode/src/          # reader.rs, leb.rs, module.rs, sections/, instructions/
crates/wtime-validate/src/        # context.rs, module.rs, types.rs, function/, const_expr.rs
crates/wtime-runtime/src/         # engine.rs, module.rs, store.rs, linker.rs, instance.rs
                                # memory/, table.rs, host.rs, code/, exec/, gc/, types/
crates/wtime-cli/src/main.rs
tools/wtime-conformance/src/     # cli.rs, corpus.rs, script.rs, adapter.rs, report.rs
tests/fixtures/<feature>/        # .wat sources, .wasm fixtures, provenance metadata
docs/architecture/              # locked interfaces, source ledger, resource policy
```

Use Rust edition 2024 and workspace package metadata with the verified pinned toolchain. Use private submodules and deliberate re-exports from lib.rs. No macro-generated public tuple APIs, no new crate per feature, no generic backend trait before a second real backend exists. Test/tool helpers may use std/Python; installing another production dependency is outside a single feature task's scope.

## Shared types

Use newtypes for module-local indices (`TypeIdx`, `FuncIdx`, `MemoryIdx`, `TableIdx`, `GlobalIdx`, `TagIdx`, `DataIdx`, `ElemIdx`). Distinguish them from runtime arena handles and canonical type identities. Wire formats use u32/u64/s33 according to the source; never assume all indices and lengths share an encoding.

Recommended non-generic shapes:
- `IndexType::{I32, I64}`.
- `Limits { min: u64, max: Option<u64> }`.
- `MemoryType { limits: Limits, index_type: IndexType, shared: bool, page_size_log2: u8 }`; default log2=16, custom log2=0 or 16 in this snapshot.
- `TableType { limits: Limits, index_type: IndexType, element: RefType }`.
- `ValType::{I32, I64, F32, F64, V128, Ref(RefType)}`.
- `RefType { nullable: bool, heap: HeapType }`; `HeapType` distinguishes abstract, module-local defined and exact-defined forms. An exact heap type cannot denote an abstract type.
- `TypeGroup` retains an explicit recursion-group boundary. Function, struct, array and continuation composite types retain finality, declared supertype and optional descriptor/describes relations.
- Runtime canonical IDs are separate Engine-bound IDs. Do not replace source type indices with canonical IDs in the decoded module.
- `Span { start: usize, end: usize }` is an absolute byte range in immutable module bytes.
- `FeatureSet` and `ResourceLimits` are concrete structs; policy errors distinguish disabled semantics from invalid binary/type rules.

Keep raw floating values as u32/u64 bits and v128 as explicit little-endian bytes/u128 conversion helpers. Runtime `Value` can carry opaque reference handles, but those handle fields and constructors are private to runtime. Do not expose a public integer-to-reference constructor merely to share types across crates.

## Binary reader and validated-module boundary

The reader owns/borrows only immutable bytes and tracks cursor, limit and absolute base offset. Methods return Result; slice access uses checked arithmetic and checked ranges. Child section readers cannot access bytes beyond their section. Legal padded LEBs are not rejected for being noncanonical.

Decoder output owns an Arc-backed byte buffer and ranges/typed declarations. Do not build self-referential structs. Instruction iterators yield structured immediates with byte spans. Parser tasks implement only their assigned opcodes; unknown assignments return a decode error, never a default instruction.

The validator's entry point is conceptually:
```text
validate(bytes, feature_set, resource_limits) -> Result<ValidatedModule, Error>
```
T002/T033 settle Rust argument ownership using the Arc-backed decoder. Only successful full validation constructs ValidatedModule. It exposes immutable declarations/body instructions/control evidence; it cannot be mutated into a different program. Runtime consumes it through validator exports and need not import decoder as another direct dependency.

## Engine, Store and linking interfaces

Implement these operations with concrete types; T002 documents exact Rust signatures before consumers begin:
```text
Engine::new(config) -> Result<Engine, Error>
Engine::compile(bytes) -> Result<Module, Error>
Store::new(engine) -> Store
Linker::new(engine) -> Linker
Linker::define(module_name, item_name, External) -> Result<(), Error>
Linker::instantiate(store, module) -> Result<Instance, Error>
Instance::export(store, name) -> Result<External, Error>
Store::invoke(function, argument_values) -> Result<RunOutcome, Error>
```
RunOutcome distinguishes returned values, a guest exception and an explicit suspension; Error distinguishes decoding, validation, linking, trapping, policy/exhaustion and host faults. No Rust panic is a guest exception.

External values are opaque function/global/memory/table/tag handles with reflection of the actual type. Validate arguments and handle affinity at public boundaries. Resolve function body/host callback ownership before constructing a mutable Caller to avoid aliasing Store borrows. Host callbacks receive a concrete Caller plus value slice and return explicitly rooted values/exception/host error. Do not add a generic Store<T> or tuple-call framework in this plan.

## Runtime state and lowering

Module is immutable and reusable; instances/state live in Store. Function code contains structured internal instructions plus side tables for variable-length immediates and resolved branch destinations. Baseline frames track module/function identity, pc, local base, operand base, expected result layout and handler/prompt boundary. A branch destination includes target instruction and the exact stack values to preserve; loops preserve parameters, exits preserve results.

Execute guest calls with an explicit loop. Before host callbacks, leave Store in a consistent rooted state and release transient borrows. Tail calls replace the current guest frame. Resume/suspend/throw manipulate explicit frame/delimiter records rather than recursing or unwinding the Rust stack.

Memory reads/writes take a memory handle and u64 effective byte range. Compute address+offset and length with checked arithmetic before converting to usize. Memory/table type width controls operand/result widths and failure sentinel width. Ordinary unaligned loads are valid. Allocation budgets are policy limits; memory.grow failure returns its specified sentinel instead of a guest trap where the spec requires failure.

## GC and type algorithms

Canonicalize whole recursion groups after validation. Normalize internal references relative to their group and external references to already canonical IDs; use structural hashing followed by full equality confirmation, never a hash alone. Bound group size/work and use explicit worklists for recursive comparisons. Distinguish type equivalence, declared subtype edges and runtime reference identity.

Use one nonmoving arena with tagged object records and a free list. Marking walks explicit roots/worklist; sweeping drops unmarked objects and retires/recycles safe generations. Never trace arbitrary native memory. Allocation and host calls are safepoints; no borrowed object slot may cross an allocation or callback. Published refs/root tokens survive collection; temporary references used by constructors are rooted before allocation.

Root providers enumerate active frames, tables/globals, host tokens, pending values/exceptions, continuation frame chains and descriptor references. GC tasks add provider hooks before new object kinds can allocate.

## Shared-memory synchronization

Baseline backend: ordinary byte storage behind locks, not unsafe mixed-width atomic aliases. Atomic operations participate in one defined SC coordinator across all shareable memories that can communicate. Lock order: SC coordinator first when required, then memory locks in stable handle order, then waiter metadata as documented by T130/T133; no opposite nested acquisition.

A wait operation checks alignment/bounds/value and installs its waiter under the relevant synchronization before releasing locks. It then blocks without the coordinator/memory lock. Notify chooses at most count waiters, updates durable waiter state and wakes them. A condition-variable loop rechecks state after spurious wakeups; timeout and notify race resolution removes a waiter exactly once. Document the chosen state machine and show it against the pinned wait/notify semantics.

## Conformance tool command contract

T012 implements this CLI; later tasks can rely on it:
```sh
cargo run --locked -p wtime-conformance -- run \
  --profile <mvp|stable-core|all-phase3-core> \
  --suite <name> --filter <case-prefix-or-all> \
  --report target/conformance/<suite>.json
cargo run --locked -p wtime-conformance -- encode input.wat -o output.wasm
```
The suite names/profile mapping come from an immutable corpus manifest. Reports include source hash, engine revision, profile, selected/executed/failed/skipped/unsupported assertion counts and replay commands. Any selected unknown directive or unintended empty selection exits nonzero. Support exact bit results, canonical/arithmetic NaN patterns and relaxed allowed-result sets. Adapter translates outcomes; it never executes programs through a reference engine.

Reserve suite names `mvp`, `core3`, `phase3` and `interactions` for the aggregate runs. Proposal suites use the exact catalog ID in SOURCES.json, such as `fixed-width_simd`, `garbage_collection`, `exception_handling` and `custom-descs`. CLI profile spellings use hyphens; internal Rust identifiers may use underscores. Implement the aliases/selection once in T011/T012 so later task commands run unchanged.

T011 fetches to an ignored cache from pinned sources, with checksum/license provenance. No curl-to-main in CI. Tooling may have a pinned external reference executable for differential tests; it is never linked into core.
