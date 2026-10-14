# Frozen Core Wasm scope and feature ledger

This implementation targets the official proposal catalog at commit `df6a2aca1cd9d779eb4f2f3e19dae8277f336015` and integrated Core specification at `7e9fb78c63d65018cc111422f657947b3df80d7b`. The lock manifest [spec-lock.json](spec-lock.json) records the Git commit and root tree checksum for the catalog, integrated specification, and every included proposal source. Acceptance must resolve those immutable commits; branch heads and tags are not corpus identities.

“Through Phase 3” means finished proposals plus active Phases 3, 4, and 5. The selected catalog snapshot has 29 entries: 20 finished and 9 active. Of those, 24 proposal entries have included Core or text-tooling work and five are JS/browser exclusions. The lock manifest adds the catalog authority and integrated Core specification as two source records. The project implements Core Wasm semantics and relevant text-tooling syntax. It does not implement browser APIs or JavaScript embedding behavior. MVP remains in scope as the base specification. Nothing in Phases 0–2 is added by implication.

## Source precedence and test strategy

For finished Core features, the integrated Core 3.0 document and its pinned `test/core` and `test/spec` suites define the baseline. For proposal deltas, use the exact proposal revision in the lock manifest and its formal specification files where present; proposal explainers are context only. Use proposal `test/` cases as additional evidence where they test Core behavior. Exclude JS API test directories. For text tooling (annotations and branch hints), test parser/metadata handling rather than instruction execution. If formal rules and tests disagree, record the discrepancy and follow the formal rule pending resolution; do not infer semantics from an implementation.

Each ledger row names task owners from `SOURCES.json`; those tasks must consume the pinned rule and test paths in [spec-lock.json](spec-lock.json). The source lock stores Git tree SHA-1 values, which identify the complete committed tree, including path and blob identities. The corpus may be reproduced by fetching the exact commit objects and checking the recorded root tree IDs.

## Feature ledger

| Feature | Stage | Normative delta / implementation surface | Test-source strategy | Task owners |
| --- | --- | --- | --- | --- |
| MVP | Finished 1.0 | Core binary format, types, validation, execution, traps and modules. | Integrated Core `test/core`, `test/spec`; exclude JS APIs. | T005–T058, T160 |
| Mutable global imports/exports | Finished 1.0 | Core global import/export typing and shared global identity. | Integrated Core tests plus pinned proposal tests. | T013, T019, T020, T026, T030, T039, T041, T047, T058 |
| Non-trapping float-to-int conversions | Finished 2.0 | Saturating conversion operators and their edge behavior. | Integrated Core tests plus pinned proposal tests. | T060–T061 |
| Sign-extension operators | Finished 2.0 | Sign-extension integer opcodes, typing and execution. | Integrated Core tests plus pinned proposal tests. | T059 |
| Multi-value | Finished 2.0 | Multiple function results and block type index forms. | Integrated Core tests plus pinned proposal tests. | T006, T018, T022, T028–T029, T044–T045, T053, T062–T063 |
| Reference types | Finished 2.0 | Reference values, tables, element segments and reference instructions. | Integrated Core tests plus pinned proposal tests. | T006–T007, T013, T018, T020–T021, T027, T029, T032, T036, T038, T040, T042, T054, T064–T066, T069 |
| Bulk memory | Finished 2.0 | Data/element segment operations and memory/table init, copy, fill, drop. | Integrated Core tests plus pinned proposal tests. | T017, T021, T032, T042, T067–T069 |
| Fixed-width SIMD | Finished 2.0 | `v128` type, SIMD opcode prefix/encodings, typing and lane semantics. | Integrated Core tests plus pinned proposal tests. | T093–T101 |
| Tail calls | Finished 3.0 | Tail-call opcodes and return-call typing/execution rules. | Integrated Core tests plus pinned proposal tests. | T075–T076 |
| Extended constant expressions | Finished 3.0 | Extended constant-expression operators and restrictions. | Integrated Core tests plus pinned proposal tests. | T070–T071 |
| Typed function references | Finished 3.0 | Typed function references, call-ref and function subtyping. | Integrated Core tests plus pinned proposal tests. | T006, T072–T075, T106–T109, T120 |
| Garbage collection | Finished 3.0 | Recursive type groups, structs, arrays, casts, allocation and GC reference semantics. | Integrated Core tests plus pinned proposal tests. | T006–T007, T106–T120 |
| Multiple memories | Finished 3.0 | Memory-index immediates and per-memory operations. | Integrated Core tests plus pinned proposal tests. | T021, T077–T079 |
| Relaxed SIMD | Finished 3.0 | Relaxed SIMD operators with specification-permitted result sets. | Integrated Core tests plus pinned proposal tests; assert permitted outcomes, not one architecture-specific answer. | T102–T105 |
| Custom text annotations | Finished 3.0 | Text-format annotation syntax and parse/round-trip metadata only; no runtime opcode. | Pinned text-tooling parser tests; no runtime conformance claim. | T012, T159 |
| Branch hints | Finished 3.0 | Custom-section metadata; execution can ignore optimization hints. | Pinned decoder/metadata tests; no performance or behavioral assertion. | T023, T158 |
| Exception handling | Finished 3.0 | Tags, exception references, throw/catch/delegate and propagation semantics. | Integrated Core tests plus pinned proposal tests; distinguish guest exceptions from traps and host errors. | T017, T045–T046, T121–T126, T145 |
| Memory64 and table64 | Finished 3.0 | 64-bit address/index types, limits, operands, segments, calls and growth. | Integrated Core tests plus pinned proposal tests. | T006, T016, T037–T038, T056, T080–T083, T136 |
| Stack switching | Active Phase 3 | Typed one-shot continuations, prompts, bind, suspend/resume, switch and throwing resume forms. | Exact pinned proposal formal sources and tests; JSPI is out of scope. | T006, T045–T046, T137–T147, T161 |
| Custom page sizes | Active Phase 3 | Per-memory page type and size/grow/limit/link rules; pinned source currently specifies one-byte and 64-KiB page sizes. | Exact pinned formal sources and tests; include memory64 interaction cases. | T006, T037, T056, T084–T086, T136, T161 |
| Custom descriptors | Active Phase 3 | Descriptor/describes type clauses, exact heap types/imports, descriptor allocations and descriptor casts. | Exact pinned formal sources and `test/core`/`test/custom`; exclude `test/js-api`. | T006, T148–T157, T161 |
| Threads and atomics | Active Phase 4 | Shared linear memories and atomic/wait/notify instructions. Shared-everything GC is excluded. | Exact pinned formal sources and proposal tests; runner must account for concurrency semantics. | T127–T136, T161 |
| Compact import section | Active Phase 4 | Compact import binary syntax and equivalent core import semantics. | Exact pinned formal sources and tests; test binary round-trip and semantic equivalence. | T087–T089, T161 |
| Wide arithmetic | Active Phase 4 | Wide arithmetic operators and specified integer behavior. | Exact pinned formal sources and tests. | T090–T092, T161 |

## Excluded sources and behaviors

The following five catalog entries are JS/browser integrations and do not enter runtime scope: JS Promise Integration, Web Content Security Policy, ESM Integration, JavaScript BigInt to Wasm i64 integration, and JS String Builtins. Their source commits remain listed under `excluded_sources` in [spec-lock.json](spec-lock.json) for auditability. JS String Builtins does not add a distinct Core opcode, binary encoding, type, or validation rule; its Core examples use existing imports and GC operations, which remain covered by their Core proposals. Custom descriptor prototype integration, compile options, builtins, and `configureAll` are excluded. JavaScript APIs for continuations and threads are also excluded.

No Phase 0–2 proposals are included, including Component Model, Type Imports, shared-everything threads, stringref, acquire-release atomics, FP16, relaxed dead-code validation, memory control, JS primitive builtins, C/C++ API, and JIT interface.

## Conflicts and unresolved source clauses

- **Custom descriptors:** `Type Section Field Deduplication` is a bare `TODO` in the pinned source. It defines no encoding or semantics. It is explicitly deferred; do not invent a section encoding or infer support from examples. Rule owners: T148–T157, with T148 responsible for the audit.
- **Custom descriptors:** the proposal explainer contains normative-looking rules and examples alongside notes and TODOs. Formal `specification/wasm-3.0` Spectec clauses are authoritative where present. Prototype configuration/errors and JavaScript object behavior are excluded regardless of their detailed prose. Rule owner: T148; type/validation owners T149–T153; allocation/cast owners T154–T156.
- **Custom page sizes:** the pinned proposal has inherited wording that must be reconciled with memory64 and one-byte pages. T085 owns this interaction; do not apply inherited 32-bit bounds without checking formal rules.
- **Stack switching:** JSPI/host integration is not a substitute for the core continuation rules. The pinned continuation type and instruction rules own Core behavior; host-facing JS integration remains excluded. Owners T137–T147.
- **Relaxed SIMD:** the specification permits multiple results for selected instructions. Tests must validate the allowed result set rather than treat an implementation-specific choice as normative. Owners T102–T104.

No other unresolved Core conflict is asserted by this scope document. New source changes require an explicit lock and task update.
