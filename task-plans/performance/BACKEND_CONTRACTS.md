# Concrete lowering and backend handoff contracts

These are interface recipes for the hard tasks, not existing public APIs. Keep implementation names private unless a task explicitly needs a public policy/view. Use the simpler working design if a predecessor proves a detail unnecessary; record the contract delta before dependent work.

## Source events and execution handoff (P018–P020/P037/P044/P064)

Use a small private cursor with checked logical function identity, `source_pc: usize`, `prepared_pc: usize`, and next-operation semantics. A source expansion is an ordered slice of entries containing original PC/opcode, logical stack shape/height, effect/barrier class, and a materialization recipe. Source indices need not fit u16/u32; a narrow prepared form declines on checked conversion failure.

The decoded loop currently handles saved fuel, cancellation, prompt retirement, fuel charge, current_fuel and operand-limit checking before fetching/incrementing PC and executing an operation. Copy that ordering, including its exact error point, into the common boundary helper. Do not move a transient stack or call-depth failure to region entry. A positive immutable certificate may prove that checks cannot fail in an eligible region and eliminate their physical repetition; a failed proof guard falls back to the original check points instead of raising an early error. Fuel/cancellation keep their selected exact/bounded policy. If a fusion cannot reproduce an interrupted source state, decline it. Arithmetic identities/folds may replay a pure prefix to materialize it, but they may not replay stores, calls, allocations or other effects.

`MaterializedState` owns or exclusively borrows the active frames/locals/operands/labels/prompts and exact remaining fuel. It retains immutable module/function contexts and checked generations. An optimized executor returns `Continue`, `Return`, or `ExitBeforeSource { cursor, reason }`. The integrator installs state in the same active execution loop; calling the public `Store::invoke` on the original function is not a valid deoptimization because it restarts arguments/fuel/effects.

Initial complex-operation adapters may hand off the whole remaining function to decoded execution. Bidirectional arbitrary-PC reentry is unnecessary. Function-entry decline occurs before any original work and is distinct from an exit after a completed prefix. Tests normalize corresponding reference graphs and compare prior memory/global/table effects, not just final scalar results.

## Compact code (P036–P045)

Start with a simple safe fixed record, for example a small tag plus auxiliary u32 index and u64 payload. Measure its actual size; a 16–24-byte bound is an initial design goal. Use side tables when a single compact operand cannot represent a wide local/index, memory64 offset, V128 constant or handler list. Never truncate a valid original immediate. Side-table range access remains checked. There is no serialized native Rust enum layout or public raw-code constructor.

Keep Engine-bound immutable code/type/control metadata in the Module payload. Instance plans alone contain Store-bound resolved funcs/mems/tabs/globs. Direct call binding guards target lifetime/generation at instantiation; dynamic/public targets keep their checks. Indirect caches initially key the checked actual function, not a mutable table slot, avoiding a global mutation-epoch rewrite.

P019 classifies opcodes conservatively: common numeric/local/control families have explicit effects; unfamiliar operations are barriers until audited. It need not reimplement all 584 execution semantics. P044 reuses current helpers after materialization and records genuine optimized prefixes/adapters separately from decoded fallback.

## Register slots and control (P057–P065)

The starter register format uses locals in fixed slots followed by numeric temporary slots assigned by source stack height. This avoids a global SSA construction. Each source boundary records the typed logical stack and its slot mapping. Branch edge values move simultaneously; schedule destinations whose old value is no longer needed first, and break a cycle by copying one source to a reserved typed scratch slot before rotating the rest. P059's local lifetime allocator is optional; P060 must work with the original fixed layout if it is absent or rejected.

Slot banks are initialized safe Rust arrays/Vecs. Use explicit i32 masking/wrapping, i64 wrapping, and f32/f64 bits; no unsafe transmute/indexing/NaN-boxed references. Initial whole-function eligibility excludes references/V128/host/capture effects. P064 may later accept numeric prefixes ending at a barrier and restore decoded state before the pending operation. Outer reference-bearing frames remain live during numeric callee execution. Logical operand/call limits count source state, not the number of physical slots/frames.

## Candidate settings and baseline artifacts (P001/P002/P016)

Separate baseline and candidate processes, never link two versions of the runtime. P001's `baseline.py --output DIR` archives the current source/executable/fixtures. P002's `--adapt-harness HARNESS --baseline ARCHIVE --output DIR` builds a separately identified protocol adapter against frozen core source; the historical executable does not magically gain the new CLI. A compile-time legacy-api branch retains original public constructors, and later workload adapters must preserve that compatibility or report an unsupported API. Never substitute current core sources to make the archive compile. Before each edit, archive a fresh `target/performance/PNNN-before` directory as the immediate predecessor baseline. Keep the original `target/performance/baseline` immutable for milestone comparisons. Rejected structural candidates can be reverted; they do not need permanent runtime flags solely for testing.

The benchmark protocol includes the selected engine policy and nondefault experiment selector in metadata. P016 creates the forwarding/validation framework; a leaf experiment adds only its own selector and exact feature forwarding through the integrator. Unknown/absent selectors are configuration errors. Shared mode names such as compact/register denote their selected declared option set, not an implicit requirement to enable every future experiment. Actual source/opcode/tier coverage proves the named optimization ran.

Nondefault `experimental-executors` forwards the required compact/register feature modules; it does not enable experimental behavior merely by compilation. `native-executor` is wired only by P095 and forwards its minimal IR/backend dependencies. The harness has corresponding feature forwarding. Adding a new public policy option occurs only when the task needs that product capability; ordinary reversible structural patches use before/after binaries.

P088 alone may extend archive/comparison settings with explicit quantum. A native matched baseline comes from the current best portable executor with the same original-op accounting and quantum, at a separately named archive. If the historical baseline differs in polling policy, show that difference and do not describe it as a matched-policy speedup.

## Optional native interface and ABI (P094–P107)

The native ADR decides whether to create `wtime-exec-ir` and `wtime-native`. The acyclic optional graph is runtime -> exec-ir/native; native -> exec-ir; exec-ir has no runtime dependency. Pure lowered-IR verification checks slot/control/source metadata and byte/work bounds. It does not replace normal Wasm decode/validation. Runtime retains the sealed Module identity and only binds artifacts produced from its admitted program. No persisted untrusted native machine-code cache is introduced.

The safe boundary is conceptually `NativeProgram::run(&mut [u64], &mut AccountingState, CancellationView) -> NativeExit`. The artifact verifies required slot length before crossing its private unsafe ABI. The native crate creates a private repr(C) view for pointers/lengths; pointers are borrowed for that synchronous call only and never retained in compiled code. Source exits name the next unexecuted original instruction. No refs, Store handle tables, GC allocation, host callbacks, guest call stacks or captured native frames cross this ABI initially.

For x86_64 Linux follow the SysV ABI: preserve callee-saved registers, align the stack at helper calls, bound private spills and code, and avoid unwinding through generated frames. Use fixed scratch registers first instead of a general physical-register allocator. Helpers are bounded status-returning operations; no panic-prone indexing/allocation after entering an extern helper. Guard division hardware faults explicitly. Default cancellation is the existing exact policy; larger quantum requires the explicit P088 policy and fresh matched controls.

The mapping owner is RW only while filling, RX after successful seal, and unmapped once final ownership ends. Retain an execution owner across cache eviction. Check page rounding, mapping limits, protection failures, relocation range/target boundaries and code-template provenance before publication. Guest integers are data and cannot become arbitrary callable/jump addresses. This is an audited small emitter, not a claim to verify arbitrary untrusted machine code.

Inline unshared memory is a later isolated experiment. Borrow current bytes exclusively for the run, prohibit growth/reentry/shared effects, check immediate-add carry and `width <= len && address <= len - width` before access, and return managed source traps. No signal traps, RWX mapping or unchecked cached memory base. AArch64 needs actual native tests and instruction-cache maintenance, not just a successful cross build.

## Conditional completion

Required gates inspect optional results only if they are available; a negative or unselected optional experiment never blocks the portable DAG. Native children require the P094 go decision; AOT children require P091 go; loop summaries require adopted explicit polling. Deferred children remain unimplemented. The selection gate cannot offer a backend as supported before its branch conformance/safety/performance gate passes.
