# Task index

All tasks are planned, not implemented. Dependencies are explicit; numeric order is not the schedule. Every task has a Sol 6.1 Low recipe, target files, fixtures and verification commands.

## 00-foundation

| Task | Area | Prerequisites |
| --- | --- | --- |
| [T001 — Freeze the specification and proposal test revisions](00-foundation/T001-freeze-scope.md) | planning | None |
| [T002 — Write crate contracts and semantic invariants](00-foundation/T002-crate-contracts.md) | planning | [T001](00-foundation/T001-freeze-scope.md) |
| [T003 — Create the minimal Rust workspace](00-foundation/T003-workspace.md) | workspace | [T002](00-foundation/T002-crate-contracts.md) |
| [T004 — Define feature dependencies and resource limits](00-foundation/T004-features.md) | wtime-types | [T003](00-foundation/T003-workspace.md) |
| [T005 — Define structured errors, traps and byte spans](00-foundation/T005-diagnostics.md) | wtime-types | [T003](00-foundation/T003-workspace.md) |
| [T006 — Define portable value and module type shapes](00-foundation/T006-type-shapes.md) | wtime-types | [T004](00-foundation/T004-features.md), [T005](00-foundation/T005-diagnostics.md), [T148](12-custom-descriptors/T148-desc-source-audit.md) |
| [T007 — Define store handles, roots and cross-store rules](00-foundation/T007-handles-contract.md) | wtime-runtime | [T006](00-foundation/T006-type-shapes.md), [T002](00-foundation/T002-crate-contracts.md) |
| [T008 — Create a complete instruction and section ledger](00-foundation/T008-opcode-ledger.md) | tooling | [T001](00-foundation/T001-freeze-scope.md), [T006](00-foundation/T006-type-shapes.md) |
| [T009 — Measure build time and binary size](00-foundation/T009-build-baseline.md) | tooling | [T003](00-foundation/T003-workspace.md) |
| [T010 — Enforce the dependency and unsafe-code budget](00-foundation/T010-dependency-policy.md) | tooling | [T003](00-foundation/T003-workspace.md) |
| [T011 — Fetch immutable upstream test corpora](00-foundation/T011-corpus.md) | wtime-conformance | [T001](00-foundation/T001-freeze-scope.md), [T003](00-foundation/T003-workspace.md) |
| [T012 — Implement spec-script parsing and outcome accounting](00-foundation/T012-script-runner.md) | wtime-conformance | [T011](00-foundation/T011-corpus.md), [T005](00-foundation/T005-diagnostics.md) |
| [T013 — Implement spectest imports and module registration](00-foundation/T013-spectest-host.md) | wtime-conformance | [T012](00-foundation/T012-script-runner.md), [T007](00-foundation/T007-handles-contract.md) |
| [T014 — Create focused validation and build lanes](00-foundation/T014-ci-lanes.md) | tooling | [T009](00-foundation/T009-build-baseline.md), [T010](00-foundation/T010-dependency-policy.md), [T012](00-foundation/T012-script-runner.md) |

## 01-binary-decoding

| Task | Area | Prerequisites |
| --- | --- | --- |
| [T015 — Implement a bounded binary reader](01-binary-decoding/T015-reader.md) | wtime-decode | [T003](00-foundation/T003-workspace.md), [T005](00-foundation/T005-diagnostics.md) |
| [T016 — Decode integer LEBs, floating bits and names](01-binary-decoding/T016-leb.md) | wtime-decode | [T015](01-binary-decoding/T015-reader.md) |
| [T017 — Decode module envelopes and section order](01-binary-decoding/T017-section-order.md) | wtime-decode | [T015](01-binary-decoding/T015-reader.md), [T016](01-binary-decoding/T016-leb.md), [T004](00-foundation/T004-features.md) |
| [T018 — Decode function types and storage limits](01-binary-decoding/T018-decode-basic-types.md) | wtime-decode | [T006](00-foundation/T006-type-shapes.md), [T016](01-binary-decoding/T016-leb.md) |
| [T019 — Decode imports, functions, exports and start](01-binary-decoding/T019-decode-imports.md) | wtime-decode | [T017](01-binary-decoding/T017-section-order.md), [T018](01-binary-decoding/T018-decode-basic-types.md) |
| [T020 — Decode globals, tables and memory declarations](01-binary-decoding/T020-decode-state.md) | wtime-decode | [T018](01-binary-decoding/T018-decode-basic-types.md), [T017](01-binary-decoding/T017-section-order.md) |
| [T021 — Decode data and element segment forms](01-binary-decoding/T021-decode-segments.md) | wtime-decode | [T020](01-binary-decoding/T020-decode-state.md), [T019](01-binary-decoding/T019-decode-imports.md) |
| [T022 — Decode function bodies and MVP instruction immediates](01-binary-decoding/T022-decode-code.md) | wtime-decode | [T016](01-binary-decoding/T016-leb.md), [T008](00-foundation/T008-opcode-ledger.md), [T018](01-binary-decoding/T018-decode-basic-types.md) |
| [T023 — Preserve custom sections and optional diagnostic names](01-binary-decoding/T023-decode-custom.md) | wtime-decode | [T017](01-binary-decoding/T017-section-order.md) |
| [T024 — Expose owned decoded modules without unsafe borrowing](01-binary-decoding/T024-decode-module-api.md) | wtime-decode | [T019](01-binary-decoding/T019-decode-imports.md), [T020](01-binary-decoding/T020-decode-state.md), [T021](01-binary-decoding/T021-decode-segments.md), [T022](01-binary-decoding/T022-decode-code.md), [T023](01-binary-decoding/T023-decode-custom.md) |

## 02-validation

| Task | Area | Prerequisites |
| --- | --- | --- |
| [T025 — Build bounded validation contexts and index spaces](02-validation/T025-validate-context.md) | wtime-validate | [T024](01-binary-decoding/T024-decode-module-api.md), [T004](00-foundation/T004-features.md) |
| [T026 — Validate module signatures, exports and start](02-validation/T026-validate-module.md) | wtime-validate | [T025](02-validation/T025-validate-context.md) |
| [T027 — Validate baseline constant expressions](02-validation/T027-validate-const.md) | wtime-validate | [T025](02-validation/T025-validate-context.md), [T020](01-binary-decoding/T020-decode-state.md) |
| [T028 — Implement operand/control stack validation](02-validation/T028-validate-stack.md) | wtime-validate | [T025](02-validation/T025-validate-context.md), [T022](01-binary-decoding/T022-decode-code.md) |
| [T029 — Validate branches, selects and block signatures](02-validation/T029-validate-control.md) | wtime-validate | [T028](02-validation/T028-validate-stack.md) |
| [T030 — Validate locals, globals and call instructions](02-validation/T030-validate-calls.md) | wtime-validate | [T028](02-validation/T028-validate-stack.md), [T026](02-validation/T026-validate-module.md) |
| [T031 — Validate scalar numeric instruction families](02-validation/T031-validate-numerics.md) | wtime-validate | [T028](02-validation/T028-validate-stack.md), [T008](00-foundation/T008-opcode-ledger.md) |
| [T032 — Validate memory, table and segment rules](02-validation/T032-validate-memory.md) | wtime-validate | [T028](02-validation/T028-validate-stack.md), [T027](02-validation/T027-validate-const.md), [T021](01-binary-decoding/T021-decode-segments.md) |
| [T033 — Seal the validated-module contract](02-validation/T033-validated-api.md) | wtime-validate | [T026](02-validation/T026-validate-module.md), [T027](02-validation/T027-validate-const.md), [T029](02-validation/T029-validate-control.md), [T030](02-validation/T030-validate-calls.md), [T031](02-validation/T031-validate-numerics.md), [T032](02-validation/T032-validate-memory.md) |
| [T034 — Run the complete MVP validation corpus](02-validation/T034-validation-gate.md) | wtime-conformance | [T033](02-validation/T033-validated-api.md), [T012](00-foundation/T012-script-runner.md) |

## 03-runtime-foundations

| Task | Area | Prerequisites |
| --- | --- | --- |
| [T035 — Implement Engine configuration and immutable modules](03-runtime-foundations/T035-engine.md) | wtime-runtime | [T033](02-validation/T033-validated-api.md), [T007](00-foundation/T007-handles-contract.md) |
| [T036 — Implement generational Store arenas](03-runtime-foundations/T036-store-arena.md) | wtime-runtime | [T007](00-foundation/T007-handles-contract.md), [T005](00-foundation/T005-diagnostics.md) |
| [T037 — Implement checked unshared linear memory](03-runtime-foundations/T037-memory-backend.md) | wtime-runtime | [T036](03-runtime-foundations/T036-store-arena.md), [T004](00-foundation/T004-features.md) |
| [T038 — Implement typed reference tables](03-runtime-foundations/T038-table-backend.md) | wtime-runtime | [T036](03-runtime-foundations/T036-store-arena.md), [T006](00-foundation/T006-type-shapes.md) |
| [T039 — Implement globals and initialization values](03-runtime-foundations/T039-global-backend.md) | wtime-runtime | [T036](03-runtime-foundations/T036-store-arena.md), [T006](00-foundation/T006-type-shapes.md) |
| [T040 — Implement explicit typed host-call boundaries](03-runtime-foundations/T040-host-functions.md) | wtime-runtime | [T036](03-runtime-foundations/T036-store-arena.md), [T006](00-foundation/T006-type-shapes.md), [T002](00-foundation/T002-crate-contracts.md) |
| [T041 — Implement typed external resolution and module linking](03-runtime-foundations/T041-linker.md) | wtime-runtime | [T035](03-runtime-foundations/T035-engine.md), [T039](03-runtime-foundations/T039-global-backend.md), [T038](03-runtime-foundations/T038-table-backend.md), [T037](03-runtime-foundations/T037-memory-backend.md), [T040](03-runtime-foundations/T040-host-functions.md) |
| [T042 — Allocate instance state and initialize segments](03-runtime-foundations/T042-allocate-instance.md) | wtime-runtime | [T041](03-runtime-foundations/T041-linker.md), [T032](02-validation/T032-validate-memory.md), [T039](03-runtime-foundations/T039-global-backend.md) |
| [T043 — Execute start functions with correct failure semantics](03-runtime-foundations/T043-start-function.md) | wtime-runtime | [T042](03-runtime-foundations/T042-allocate-instance.md), [T054](04-mvp-execution/T054-call-execution.md) |
| [T044 — Lower validated functions into compact interpreter code](03-runtime-foundations/T044-compile-ir.md) | wtime-runtime | [T035](03-runtime-foundations/T035-engine.md), [T033](02-validation/T033-validated-api.md), [T008](00-foundation/T008-opcode-ledger.md) |
| [T045 — Implement explicit execution stacks and frames](03-runtime-foundations/T045-execution-stacks.md) | wtime-runtime | [T044](03-runtime-foundations/T044-compile-ir.md), [T036](03-runtime-foundations/T036-store-arena.md) |
| [T046 — Define run outcomes and cancellation checkpoints](03-runtime-foundations/T046-execution-results.md) | wtime-runtime | [T045](03-runtime-foundations/T045-execution-stacks.md), [T005](00-foundation/T005-diagnostics.md), [T004](00-foundation/T004-features.md) |

## 04-mvp-execution

| Task | Area | Prerequisites |
| --- | --- | --- |
| [T047 — Execute constants, locals, globals and stack operators](04-mvp-execution/T047-scalar-state.md) | wtime-runtime | [T045](03-runtime-foundations/T045-execution-stacks.md), [T039](03-runtime-foundations/T039-global-backend.md) |
| [T048 — Execute integer arithmetic and comparisons](04-mvp-execution/T048-integer-math.md) | wtime-runtime | [T045](03-runtime-foundations/T045-execution-stacks.md) |
| [T049 — Execute integer bit operations and shifts](04-mvp-execution/T049-integer-bits.md) | wtime-runtime | [T045](03-runtime-foundations/T045-execution-stacks.md) |
| [T050 — Execute floating arithmetic and comparisons](04-mvp-execution/T050-float-math.md) | wtime-runtime | [T045](03-runtime-foundations/T045-execution-stacks.md) |
| [T051 — Execute floating rounding instructions](04-mvp-execution/T051-float-rounding.md) | wtime-runtime | [T050](04-mvp-execution/T050-float-math.md) |
| [T052 — Execute numeric conversions and reinterpretations](04-mvp-execution/T052-numeric-conversions.md) | wtime-runtime | [T048](04-mvp-execution/T048-integer-math.md), [T050](04-mvp-execution/T050-float-math.md) |
| [T053 — Execute structured control and branches](04-mvp-execution/T053-control-execution.md) | wtime-runtime | [T044](03-runtime-foundations/T044-compile-ir.md), [T045](03-runtime-foundations/T045-execution-stacks.md), [T047](04-mvp-execution/T047-scalar-state.md) |
| [T054 — Execute direct, indirect and host calls](04-mvp-execution/T054-call-execution.md) | wtime-runtime | [T045](03-runtime-foundations/T045-execution-stacks.md), [T040](03-runtime-foundations/T040-host-functions.md), [T038](03-runtime-foundations/T038-table-backend.md), [T053](04-mvp-execution/T053-control-execution.md) |
| [T055 — Execute scalar loads and stores](04-mvp-execution/T055-load-store.md) | wtime-runtime | [T037](03-runtime-foundations/T037-memory-backend.md), [T045](03-runtime-foundations/T045-execution-stacks.md) |
| [T056 — Execute memory.size and memory.grow](04-mvp-execution/T056-memory-growth.md) | wtime-runtime | [T037](03-runtime-foundations/T037-memory-backend.md), [T045](03-runtime-foundations/T045-execution-stacks.md) |
| [T057 — Add a small CLI for inspect, validate and invoke](04-mvp-execution/T057-cli.md) | wtime-cli | [T043](03-runtime-foundations/T043-start-function.md), [T046](03-runtime-foundations/T046-execution-results.md), [T052](04-mvp-execution/T052-numeric-conversions.md), [T049](04-mvp-execution/T049-integer-bits.md), [T051](04-mvp-execution/T051-float-rounding.md), [T055](04-mvp-execution/T055-load-store.md), [T056](04-mvp-execution/T056-memory-growth.md) |
| [T058 — Pass the complete selected MVP corpus](04-mvp-execution/T058-mvp-gate.md) | wtime-conformance | [T057](04-mvp-execution/T057-cli.md), [T013](00-foundation/T013-spectest-host.md), [T034](02-validation/T034-validation-gate.md) |

## 05-scalar-and-reference-proposals

| Task | Area | Prerequisites |
| --- | --- | --- |
| [T059 — Implement sign-extension instructions end to end](05-scalar-and-reference-proposals/T059-sign-extension.md) | core | [T031](02-validation/T031-validate-numerics.md), [T049](04-mvp-execution/T049-integer-bits.md), [T022](01-binary-decoding/T022-decode-code.md) |
| [T060 — Decode and validate saturating conversions](05-scalar-and-reference-proposals/T060-sat-decode.md) | decode/validate | [T022](01-binary-decoding/T022-decode-code.md), [T031](02-validation/T031-validate-numerics.md) |
| [T061 — Execute saturating float-to-int conversions](05-scalar-and-reference-proposals/T061-sat-execute.md) | wtime-runtime | [T060](05-scalar-and-reference-proposals/T060-sat-decode.md), [T052](04-mvp-execution/T052-numeric-conversions.md) |
| [T062 — Complete multivalue decoding and validation](05-scalar-and-reference-proposals/T062-multivalue-validate.md) | decode/validate | [T022](01-binary-decoding/T022-decode-code.md), [T029](02-validation/T029-validate-control.md), [T030](02-validation/T030-validate-calls.md) |
| [T063 — Complete multivalue execution and host results](05-scalar-and-reference-proposals/T063-multivalue-execute.md) | wtime-runtime | [T062](05-scalar-and-reference-proposals/T062-multivalue-validate.md), [T053](04-mvp-execution/T053-control-execution.md), [T054](04-mvp-execution/T054-call-execution.md) |
| [T064 — Decode and validate reference-type instructions](05-scalar-and-reference-proposals/T064-refs-validate.md) | decode/validate | [T018](01-binary-decoding/T018-decode-basic-types.md), [T028](02-validation/T028-validate-stack.md), [T032](02-validation/T032-validate-memory.md), [T030](02-validation/T030-validate-calls.md) |
| [T065 — Execute reference identity and host rooting](05-scalar-and-reference-proposals/T065-refs-values.md) | wtime-runtime | [T064](05-scalar-and-reference-proposals/T064-refs-validate.md), [T047](04-mvp-execution/T047-scalar-state.md), [T040](03-runtime-foundations/T040-host-functions.md), [T007](00-foundation/T007-handles-contract.md) |
| [T066 — Execute reference table operations](05-scalar-and-reference-proposals/T066-refs-tables.md) | wtime-runtime | [T065](05-scalar-and-reference-proposals/T065-refs-values.md), [T038](03-runtime-foundations/T038-table-backend.md) |
| [T067 — Decode and validate bulk-memory and table instructions](05-scalar-and-reference-proposals/T067-bulk-validate.md) | decode/validate | [T021](01-binary-decoding/T021-decode-segments.md), [T032](02-validation/T032-validate-memory.md), [T064](05-scalar-and-reference-proposals/T064-refs-validate.md) |
| [T068 — Execute bulk-memory operations](05-scalar-and-reference-proposals/T068-bulk-memory.md) | wtime-runtime | [T067](05-scalar-and-reference-proposals/T067-bulk-validate.md), [T055](04-mvp-execution/T055-load-store.md), [T056](04-mvp-execution/T056-memory-growth.md), [T042](03-runtime-foundations/T042-allocate-instance.md) |
| [T069 — Execute bulk-table operations](05-scalar-and-reference-proposals/T069-bulk-tables.md) | wtime-runtime | [T067](05-scalar-and-reference-proposals/T067-bulk-validate.md), [T066](05-scalar-and-reference-proposals/T066-refs-tables.md), [T042](03-runtime-foundations/T042-allocate-instance.md) |
| [T070 — Validate extended constant expressions](05-scalar-and-reference-proposals/T070-extconst-validate.md) | wtime-validate | [T027](02-validation/T027-validate-const.md), [T062](05-scalar-and-reference-proposals/T062-multivalue-validate.md) |
| [T071 — Evaluate extended constant expressions](05-scalar-and-reference-proposals/T071-extconst-evaluate.md) | wtime-runtime | [T070](05-scalar-and-reference-proposals/T070-extconst-validate.md), [T039](03-runtime-foundations/T039-global-backend.md), [T048](04-mvp-execution/T048-integer-math.md), [T042](03-runtime-foundations/T042-allocate-instance.md) |
| [T072 — Decode typed function references and recursive function types](05-scalar-and-reference-proposals/T072-funcref-types.md) | decode/types | [T018](01-binary-decoding/T018-decode-basic-types.md), [T006](00-foundation/T006-type-shapes.md), [T064](05-scalar-and-reference-proposals/T064-refs-validate.md) |
| [T073 — Validate typed function-reference operations](05-scalar-and-reference-proposals/T073-funcref-validation.md) | wtime-validate | [T072](05-scalar-and-reference-proposals/T072-funcref-types.md), [T062](05-scalar-and-reference-proposals/T062-multivalue-validate.md), [T030](02-validation/T030-validate-calls.md) |
| [T074 — Execute typed calls and reference refinements](05-scalar-and-reference-proposals/T074-funcref-execution.md) | wtime-runtime | [T073](05-scalar-and-reference-proposals/T073-funcref-validation.md), [T065](05-scalar-and-reference-proposals/T065-refs-values.md), [T054](04-mvp-execution/T054-call-execution.md) |
| [T075 — Decode and validate all tail-call forms](05-scalar-and-reference-proposals/T075-tail-validate.md) | decode/validate | [T073](05-scalar-and-reference-proposals/T073-funcref-validation.md), [T030](02-validation/T030-validate-calls.md) |
| [T076 — Execute tail calls by replacing guest frames](05-scalar-and-reference-proposals/T076-tail-execute.md) | wtime-runtime | [T075](05-scalar-and-reference-proposals/T075-tail-validate.md), [T074](05-scalar-and-reference-proposals/T074-funcref-execution.md), [T054](04-mvp-execution/T054-call-execution.md) |

## 06-memory-proposals

| Task | Area | Prerequisites |
| --- | --- | --- |
| [T077 — Decode explicit multi-memory indices](06-memory-proposals/T077-multi-memory-decode.md) | wtime-decode | [T022](01-binary-decoding/T022-decode-code.md), [T018](01-binary-decoding/T018-decode-basic-types.md), [T021](01-binary-decoding/T021-decode-segments.md) |
| [T078 — Validate indexed memory operations](06-memory-proposals/T078-multi-memory-validate.md) | wtime-validate | [T077](06-memory-proposals/T077-multi-memory-decode.md), [T032](02-validation/T032-validate-memory.md), [T067](05-scalar-and-reference-proposals/T067-bulk-validate.md) |
| [T079 — Execute modules with multiple memories](06-memory-proposals/T079-multi-memory-execute.md) | wtime-runtime | [T078](06-memory-proposals/T078-multi-memory-validate.md), [T068](05-scalar-and-reference-proposals/T068-bulk-memory.md), [T055](04-mvp-execution/T055-load-store.md), [T056](04-mvp-execution/T056-memory-growth.md) |
| [T080 — Decode 64-bit memories, tables and immediates](06-memory-proposals/T080-memory64-decode.md) | wtime-decode | [T018](01-binary-decoding/T018-decode-basic-types.md), [T077](06-memory-proposals/T077-multi-memory-decode.md) |
| [T081 — Validate 64-bit address and table operand types](06-memory-proposals/T081-memory64-validate.md) | wtime-validate | [T080](06-memory-proposals/T080-memory64-decode.md), [T078](06-memory-proposals/T078-multi-memory-validate.md), [T064](05-scalar-and-reference-proposals/T064-refs-validate.md), [T067](05-scalar-and-reference-proposals/T067-bulk-validate.md) |
| [T082 — Execute checked 64-bit memory and table indexing](06-memory-proposals/T082-memory64-addresses.md) | wtime-runtime | [T081](06-memory-proposals/T081-memory64-validate.md), [T055](04-mvp-execution/T055-load-store.md), [T066](05-scalar-and-reference-proposals/T066-refs-tables.md), [T054](04-mvp-execution/T054-call-execution.md) |
| [T083 — Complete 64-bit growth, segments and bulk operations](06-memory-proposals/T083-memory64-bulk-grow.md) | wtime-runtime | [T082](06-memory-proposals/T082-memory64-addresses.md), [T068](05-scalar-and-reference-proposals/T068-bulk-memory.md), [T069](05-scalar-and-reference-proposals/T069-bulk-tables.md), [T056](04-mvp-execution/T056-memory-growth.md) |
| [T084 — Decode custom page sizes and limit flags](06-memory-proposals/T084-page-decode.md) | wtime-decode | [T080](06-memory-proposals/T080-memory64-decode.md), [T008](00-foundation/T008-opcode-ledger.md) |
| [T085 — Validate page-size limits and exact import matching](06-memory-proposals/T085-page-validate.md) | wtime-validate | [T084](06-memory-proposals/T084-page-decode.md), [T081](06-memory-proposals/T081-memory64-validate.md) |
| [T086 — Implement per-memory page counts and growth](06-memory-proposals/T086-page-execute.md) | wtime-runtime | [T085](06-memory-proposals/T085-page-validate.md), [T083](06-memory-proposals/T083-memory64-bulk-grow.md), [T079](06-memory-proposals/T079-multi-memory-execute.md), [T041](03-runtime-foundations/T041-linker.md) |
| [T087 — Decode both compact import encodings](06-memory-proposals/T087-compact-import-decode.md) | wtime-decode | [T019](01-binary-decoding/T019-decode-imports.md), [T008](00-foundation/T008-opcode-ledger.md) |
| [T088 — Validate and link normalized compact imports](06-memory-proposals/T088-compact-import-integration.md) | core | [T087](06-memory-proposals/T087-compact-import-decode.md), [T041](03-runtime-foundations/T041-linker.md), [T080](06-memory-proposals/T080-memory64-decode.md) |
| [T089 — Support compact import text forms in tooling](06-memory-proposals/T089-compact-import-text.md) | wtime-conformance | [T087](06-memory-proposals/T087-compact-import-decode.md), [T012](00-foundation/T012-script-runner.md) |
| [T090 — Decode and validate wide arithmetic instructions](06-memory-proposals/T090-wide-validate.md) | decode/validate | [T062](05-scalar-and-reference-proposals/T062-multivalue-validate.md), [T031](02-validation/T031-validate-numerics.md), [T022](01-binary-decoding/T022-decode-code.md) |
| [T091 — Execute 128-bit arithmetic with Rust primitives](06-memory-proposals/T091-wide-execute.md) | wtime-runtime | [T090](06-memory-proposals/T090-wide-validate.md), [T048](04-mvp-execution/T048-integer-math.md), [T063](05-scalar-and-reference-proposals/T063-multivalue-execute.md) |
| [T092 — Run the complete wide arithmetic corpus](06-memory-proposals/T092-wide-conformance.md) | wtime-conformance | [T091](06-memory-proposals/T091-wide-execute.md), [T013](00-foundation/T013-spectest-host.md) |

## 07-vector-proposals

| Task | Area | Prerequisites |
| --- | --- | --- |
| [T093 — Decode SIMD opcodes, lanes and vector immediates](07-vector-proposals/T093-simd-decode.md) | wtime-decode | [T022](01-binary-decoding/T022-decode-code.md), [T008](00-foundation/T008-opcode-ledger.md) |
| [T094 — Validate SIMD signatures and lane/memory constraints](07-vector-proposals/T094-simd-validate.md) | wtime-validate | [T093](07-vector-proposals/T093-simd-decode.md), [T032](02-validation/T032-validate-memory.md), [T031](02-validation/T031-validate-numerics.md) |
| [T095 — Execute vector constants and memory operations](07-vector-proposals/T095-simd-memory.md) | wtime-runtime | [T094](07-vector-proposals/T094-simd-validate.md), [T055](04-mvp-execution/T055-load-store.md), [T047](04-mvp-execution/T047-scalar-state.md) |
| [T096 — Execute vector lane, shuffle and bitwise operations](07-vector-proposals/T096-simd-permute.md) | wtime-runtime | [T094](07-vector-proposals/T094-simd-validate.md), [T045](03-runtime-foundations/T045-execution-stacks.md) |
| [T097 — Execute SIMD integer arithmetic, shifts and comparisons](07-vector-proposals/T097-simd-int-basic.md) | wtime-runtime | [T094](07-vector-proposals/T094-simd-validate.md), [T048](04-mvp-execution/T048-integer-math.md), [T049](04-mvp-execution/T049-integer-bits.md) |
| [T098 — Execute SIMD saturating, narrowing and widening operations](07-vector-proposals/T098-simd-int-special.md) | wtime-runtime | [T097](07-vector-proposals/T097-simd-int-basic.md) |
| [T099 — Execute SIMD floating operations](07-vector-proposals/T099-simd-floats.md) | wtime-runtime | [T094](07-vector-proposals/T094-simd-validate.md), [T050](04-mvp-execution/T050-float-math.md), [T051](04-mvp-execution/T051-float-rounding.md) |
| [T100 — Execute SIMD numeric conversions and demotion/promotion](07-vector-proposals/T100-simd-convert.md) | wtime-runtime | [T094](07-vector-proposals/T094-simd-validate.md), [T052](04-mvp-execution/T052-numeric-conversions.md), [T061](05-scalar-and-reference-proposals/T061-sat-execute.md) |
| [T101 — Pass the fixed-width SIMD conformance corpus](07-vector-proposals/T101-simd-gate.md) | wtime-conformance | [T095](07-vector-proposals/T095-simd-memory.md), [T096](07-vector-proposals/T096-simd-permute.md), [T098](07-vector-proposals/T098-simd-int-special.md), [T099](07-vector-proposals/T099-simd-floats.md), [T100](07-vector-proposals/T100-simd-convert.md), [T013](00-foundation/T013-spectest-host.md) |
| [T102 — Decode and validate relaxed SIMD](07-vector-proposals/T102-relaxed-decode-validate.md) | decode/validate | [T094](07-vector-proposals/T094-simd-validate.md), [T008](00-foundation/T008-opcode-ledger.md) |
| [T103 — Execute relaxed swizzle, lane-select and integer products](07-vector-proposals/T103-relaxed-integer.md) | wtime-runtime | [T102](07-vector-proposals/T102-relaxed-decode-validate.md), [T096](07-vector-proposals/T096-simd-permute.md), [T098](07-vector-proposals/T098-simd-int-special.md) |
| [T104 — Execute relaxed truncation, multiply-add and min/max](07-vector-proposals/T104-relaxed-float.md) | wtime-runtime | [T102](07-vector-proposals/T102-relaxed-decode-validate.md), [T099](07-vector-proposals/T099-simd-floats.md), [T100](07-vector-proposals/T100-simd-convert.md) |
| [T105 — Pass relaxed SIMD conformance with allowed results](07-vector-proposals/T105-relaxed-gate.md) | wtime-conformance | [T103](07-vector-proposals/T103-relaxed-integer.md), [T104](07-vector-proposals/T104-relaxed-float.md), [T012](00-foundation/T012-script-runner.md) |

## 08-gc-and-type-system

| Task | Area | Prerequisites |
| --- | --- | --- |
| [T106 — Decode recursive groups, structs, arrays and heap types](08-gc-and-type-system/T106-gc-type-decode.md) | decode/types | [T072](05-scalar-and-reference-proposals/T072-funcref-types.md), [T008](00-foundation/T008-opcode-ledger.md) |
| [T107 — Validate recursive types, subtyping and defaultability](08-gc-and-type-system/T107-gc-type-validate.md) | wtime-validate | [T106](08-gc-and-type-system/T106-gc-type-decode.md), [T073](05-scalar-and-reference-proposals/T073-funcref-validation.md) |
| [T108 — Canonicalize and intern recursive type groups](08-gc-and-type-system/T108-type-interning.md) | wtime-runtime | [T107](08-gc-and-type-system/T107-gc-type-validate.md), [T035](03-runtime-foundations/T035-engine.md) |
| [T109 — Use canonical types for imported reference matching](08-gc-and-type-system/T109-gc-linking.md) | wtime-runtime | [T108](08-gc-and-type-system/T108-type-interning.md), [T041](03-runtime-foundations/T041-linker.md), [T074](05-scalar-and-reference-proposals/T074-funcref-execution.md) |
| [T110 — Decode all GC reference and allocation instructions](08-gc-and-type-system/T110-gc-op-decode.md) | wtime-decode | [T106](08-gc-and-type-system/T106-gc-type-decode.md), [T022](01-binary-decoding/T022-decode-code.md) |
| [T111 — Validate GC operations and initializer instructions](08-gc-and-type-system/T111-gc-op-validate.md) | wtime-validate | [T110](08-gc-and-type-system/T110-gc-op-decode.md), [T107](08-gc-and-type-system/T107-gc-type-validate.md), [T070](05-scalar-and-reference-proposals/T070-extconst-validate.md), [T064](05-scalar-and-reference-proposals/T064-refs-validate.md) |
| [T112 — Implement nonmoving typed GC object storage](08-gc-and-type-system/T112-gc-heap.md) | wtime-runtime | [T111](08-gc-and-type-system/T111-gc-op-validate.md), [T036](03-runtime-foundations/T036-store-arena.md), [T108](08-gc-and-type-system/T108-type-interning.md) |
| [T113 — Enumerate roots at every execution boundary](08-gc-and-type-system/T113-gc-root-walk.md) | wtime-runtime | [T112](08-gc-and-type-system/T112-gc-heap.md), [T045](03-runtime-foundations/T045-execution-stacks.md), [T065](05-scalar-and-reference-proposals/T065-refs-values.md), [T039](03-runtime-foundations/T039-global-backend.md), [T038](03-runtime-foundations/T038-table-backend.md) |
| [T114 — Implement stop-the-world tracing collection](08-gc-and-type-system/T114-gc-collector.md) | wtime-runtime | [T113](08-gc-and-type-system/T113-gc-root-walk.md) |
| [T115 — Execute struct allocation, access and mutation](08-gc-and-type-system/T115-gc-structs.md) | wtime-runtime | [T114](08-gc-and-type-system/T114-gc-collector.md), [T111](08-gc-and-type-system/T111-gc-op-validate.md) |
| [T116 — Execute array creation, access and length](08-gc-and-type-system/T116-gc-arrays.md) | wtime-runtime | [T114](08-gc-and-type-system/T114-gc-collector.md), [T111](08-gc-and-type-system/T111-gc-op-validate.md) |
| [T117 — Execute array bulk, data and element operations](08-gc-and-type-system/T117-gc-array-bulk.md) | wtime-runtime | [T116](08-gc-and-type-system/T116-gc-arrays.md), [T068](05-scalar-and-reference-proposals/T068-bulk-memory.md), [T069](05-scalar-and-reference-proposals/T069-bulk-tables.md) |
| [T118 — Execute i31, reference equality and host conversions](08-gc-and-type-system/T118-gc-ref-semantics.md) | wtime-runtime | [T114](08-gc-and-type-system/T114-gc-collector.md), [T065](05-scalar-and-reference-proposals/T065-refs-values.md), [T109](08-gc-and-type-system/T109-gc-linking.md) |
| [T119 — Execute reference tests, casts and cast branches](08-gc-and-type-system/T119-gc-casts.md) | wtime-runtime | [T109](08-gc-and-type-system/T109-gc-linking.md), [T118](08-gc-and-type-system/T118-gc-ref-semantics.md), [T053](04-mvp-execution/T053-control-execution.md) |
| [T120 — Pass GC/type-system conformance and collection stress](08-gc-and-type-system/T120-gc-gate.md) | wtime-conformance | [T115](08-gc-and-type-system/T115-gc-structs.md), [T117](08-gc-and-type-system/T117-gc-array-bulk.md), [T119](08-gc-and-type-system/T119-gc-casts.md), [T013](00-foundation/T013-spectest-host.md) |

## 09-exception-handling

| Task | Area | Prerequisites |
| --- | --- | --- |
| [T121 — Decode tags, exn heap types and current EH syntax](09-exception-handling/T121-eh-decode.md) | decode/types | [T022](01-binary-decoding/T022-decode-code.md), [T106](08-gc-and-type-system/T106-gc-type-decode.md), [T008](00-foundation/T008-opcode-ledger.md) |
| [T122 — Validate exception tags and handler label signatures](09-exception-handling/T122-eh-validate.md) | wtime-validate | [T121](09-exception-handling/T121-eh-decode.md), [T107](08-gc-and-type-system/T107-gc-type-validate.md), [T029](02-validation/T029-validate-control.md), [T030](02-validation/T030-validate-calls.md) |
| [T123 — Represent rooted exceptions and payload identities](09-exception-handling/T123-eh-objects.md) | wtime-runtime | [T122](09-exception-handling/T122-eh-validate.md), [T114](08-gc-and-type-system/T114-gc-collector.md) |
| [T124 — Execute throws, try_table and exception dispatch](09-exception-handling/T124-eh-unwind.md) | wtime-runtime | [T123](09-exception-handling/T123-eh-objects.md), [T053](04-mvp-execution/T053-control-execution.md), [T054](04-mvp-execution/T054-call-execution.md), [T046](03-runtime-foundations/T046-execution-results.md) |
| [T125 — Propagate exceptions across host calls and reentry](09-exception-handling/T125-eh-host.md) | wtime-runtime | [T124](09-exception-handling/T124-eh-unwind.md), [T040](03-runtime-foundations/T040-host-functions.md), [T065](05-scalar-and-reference-proposals/T065-refs-values.md) |
| [T126 — Pass standardized exception-handling conformance](09-exception-handling/T126-eh-gate.md) | wtime-conformance | [T125](09-exception-handling/T125-eh-host.md), [T120](08-gc-and-type-system/T120-gc-gate.md), [T076](05-scalar-and-reference-proposals/T076-tail-execute.md), [T013](00-foundation/T013-spectest-host.md) |

## 10-threads-and-atomics

| Task | Area | Prerequisites |
| --- | --- | --- |
| [T127 — Decode and validate shared memory and atomics](10-threads-and-atomics/T127-threads-decode-validate.md) | decode/validate | [T081](06-memory-proposals/T081-memory64-validate.md), [T085](06-memory-proposals/T085-page-validate.md), [T008](00-foundation/T008-opcode-ledger.md) |
| [T128 — Implement explicit shareable memory storage](10-threads-and-atomics/T128-shared-memory.md) | wtime-runtime | [T127](10-threads-and-atomics/T127-threads-decode-validate.md), [T037](03-runtime-foundations/T037-memory-backend.md), [T082](06-memory-proposals/T082-memory64-addresses.md), [T086](06-memory-proposals/T086-page-execute.md) |
| [T129 — Route normal accesses to shared memory safely](10-threads-and-atomics/T129-shared-regular-access.md) | wtime-runtime | [T128](10-threads-and-atomics/T128-shared-memory.md), [T055](04-mvp-execution/T055-load-store.md), [T068](05-scalar-and-reference-proposals/T068-bulk-memory.md), [T095](07-vector-proposals/T095-simd-memory.md) |
| [T130 — Implement sequential-consistency coordination](10-threads-and-atomics/T130-atomic-ordering.md) | wtime-runtime | [T128](10-threads-and-atomics/T128-shared-memory.md), [T002](00-foundation/T002-crate-contracts.md) |
| [T131 — Execute atomic loads, stores and fences](10-threads-and-atomics/T131-atomic-load-store.md) | wtime-runtime | [T130](10-threads-and-atomics/T130-atomic-ordering.md), [T129](10-threads-and-atomics/T129-shared-regular-access.md) |
| [T132 — Execute atomic read-modify-write and compare-exchange](10-threads-and-atomics/T132-atomic-rmw.md) | wtime-runtime | [T131](10-threads-and-atomics/T131-atomic-load-store.md), [T048](04-mvp-execution/T048-integer-math.md) |
| [T133 — Implement wait queues, timeouts and notification](10-threads-and-atomics/T133-atomic-wait-notify.md) | wtime-runtime | [T132](10-threads-and-atomics/T132-atomic-rmw.md), [T046](03-runtime-foundations/T046-execution-results.md) |
| [T134 — Expose worker-safe host instantiation and memory sharing](10-threads-and-atomics/T134-thread-host-api.md) | wtime-runtime | [T128](10-threads-and-atomics/T128-shared-memory.md), [T133](10-threads-and-atomics/T133-atomic-wait-notify.md), [T040](03-runtime-foundations/T040-host-functions.md), [T113](08-gc-and-type-system/T113-gc-root-walk.md) |
| [T135 — Support thread assertions and stress scheduling](10-threads-and-atomics/T135-threads-runner.md) | wtime-conformance | [T134](10-threads-and-atomics/T134-thread-host-api.md), [T012](00-foundation/T012-script-runner.md), [T013](00-foundation/T013-spectest-host.md) |
| [T136 — Pass Threads and combined-memory conformance](10-threads-and-atomics/T136-threads-gate.md) | wtime-conformance | [T135](10-threads-and-atomics/T135-threads-runner.md), [T086](06-memory-proposals/T086-page-execute.md), [T083](06-memory-proposals/T083-memory64-bulk-grow.md), [T129](10-threads-and-atomics/T129-shared-regular-access.md) |

## 11-stack-switching

| Task | Area | Prerequisites |
| --- | --- | --- |
| [T137 — Decode continuation types, tags and stack-switching instructions](11-stack-switching/T137-cont-decode.md) | decode/types | [T121](09-exception-handling/T121-eh-decode.md), [T072](05-scalar-and-reference-proposals/T072-funcref-types.md), [T008](00-foundation/T008-opcode-ledger.md) |
| [T138 — Validate continuation types and result-bearing tags](11-stack-switching/T138-cont-type-validation.md) | wtime-validate | [T137](11-stack-switching/T137-cont-decode.md), [T122](09-exception-handling/T122-eh-validate.md), [T107](08-gc-and-type-system/T107-gc-type-validate.md) |
| [T139 — Validate continuation effects and handler signatures](11-stack-switching/T139-cont-op-validation.md) | wtime-validate | [T138](11-stack-switching/T138-cont-type-validation.md), [T029](02-validation/T029-validate-control.md), [T073](05-scalar-and-reference-proposals/T073-funcref-validation.md) |
| [T140 — Implement traced one-shot continuation records](11-stack-switching/T140-cont-storage.md) | wtime-runtime | [T139](11-stack-switching/T139-cont-op-validation.md), [T114](08-gc-and-type-system/T114-gc-collector.md), [T045](03-runtime-foundations/T045-execution-stacks.md) |
| [T141 — Execute continuation creation and partial binding](11-stack-switching/T141-cont-new-bind.md) | wtime-runtime | [T140](11-stack-switching/T140-cont-storage.md), [T074](05-scalar-and-reference-proposals/T074-funcref-execution.md) |
| [T142 — Implement prompt/handler delimiter stacks](11-stack-switching/T142-cont-prompts.md) | wtime-runtime | [T140](11-stack-switching/T140-cont-storage.md), [T124](09-exception-handling/T124-eh-unwind.md), [T045](03-runtime-foundations/T045-execution-stacks.md) |
| [T143 — Execute resume and suspend](11-stack-switching/T143-cont-resume-suspend.md) | wtime-runtime | [T141](11-stack-switching/T141-cont-new-bind.md), [T142](11-stack-switching/T142-cont-prompts.md), [T046](03-runtime-foundations/T046-execution-results.md) |
| [T144 — Execute symmetric continuation switching](11-stack-switching/T144-cont-switch.md) | wtime-runtime | [T143](11-stack-switching/T143-cont-resume-suspend.md) |
| [T145 — Execute throwing resumes and EH interactions](11-stack-switching/T145-cont-throw.md) | wtime-runtime | [T143](11-stack-switching/T143-cont-resume-suspend.md), [T125](09-exception-handling/T125-eh-host.md) |
| [T146 — Define suspension across host-call boundaries](11-stack-switching/T146-cont-host-boundary.md) | wtime-runtime | [T144](11-stack-switching/T144-cont-switch.md), [T145](11-stack-switching/T145-cont-throw.md), [T040](03-runtime-foundations/T040-host-functions.md) |
| [T147 — Pass stack-switching conformance and lifecycle stress](11-stack-switching/T147-cont-gate.md) | wtime-conformance | [T146](11-stack-switching/T146-cont-host-boundary.md), [T120](08-gc-and-type-system/T120-gc-gate.md), [T076](05-scalar-and-reference-proposals/T076-tail-execute.md) |

## 12-custom-descriptors

| Task | Area | Prerequisites |
| --- | --- | --- |
| [T148 — Resolve the pinned custom-descriptor core contract](12-custom-descriptors/T148-desc-source-audit.md) | planning | [T001](00-foundation/T001-freeze-scope.md) |
| [T149 — Decode descriptor clauses, exact heap types and exact imports](12-custom-descriptors/T149-desc-decode.md) | wtime-decode | [T148](12-custom-descriptors/T148-desc-source-audit.md), [T106](08-gc-and-type-system/T106-gc-type-decode.md), [T019](01-binary-decoding/T019-decode-imports.md), [T110](08-gc-and-type-system/T110-gc-op-decode.md) |
| [T150 — Validate descriptor graphs and exact type relationships](12-custom-descriptors/T150-desc-types.md) | wtime-validate | [T149](12-custom-descriptors/T149-desc-decode.md), [T107](08-gc-and-type-system/T107-gc-type-validate.md), [T073](05-scalar-and-reference-proposals/T073-funcref-validation.md) |
| [T151 — Extend canonical type identity with descriptors and exactness](12-custom-descriptors/T151-desc-intern.md) | wtime-runtime | [T150](12-custom-descriptors/T150-desc-types.md), [T108](08-gc-and-type-system/T108-type-interning.md), [T109](08-gc-and-type-system/T109-gc-linking.md) |
| [T152 — Enforce exact function imports at link and call boundaries](12-custom-descriptors/T152-desc-imports.md) | wtime-runtime | [T151](12-custom-descriptors/T151-desc-intern.md), [T041](03-runtime-foundations/T041-linker.md), [T074](05-scalar-and-reference-proposals/T074-funcref-execution.md), [T088](06-memory-proposals/T088-compact-import-integration.md) |
| [T153 — Validate descriptor allocation, retrieval and cast operations](12-custom-descriptors/T153-desc-op-validation.md) | wtime-validate | [T150](12-custom-descriptors/T150-desc-types.md), [T111](08-gc-and-type-system/T111-gc-op-validate.md), [T070](05-scalar-and-reference-proposals/T070-extconst-validate.md) |
| [T154 — Store descriptors in traced object headers](12-custom-descriptors/T154-desc-layout.md) | wtime-runtime | [T151](12-custom-descriptors/T151-desc-intern.md), [T115](08-gc-and-type-system/T115-gc-structs.md), [T113](08-gc-and-type-system/T113-gc-root-walk.md) |
| [T155 — Execute described allocation and descriptor retrieval](12-custom-descriptors/T155-desc-allocate.md) | wtime-runtime | [T153](12-custom-descriptors/T153-desc-op-validation.md), [T154](12-custom-descriptors/T154-desc-layout.md), [T152](12-custom-descriptors/T152-desc-imports.md), [T115](08-gc-and-type-system/T115-gc-structs.md) |
| [T156 — Execute descriptor-equality casts and branches](12-custom-descriptors/T156-desc-cast.md) | wtime-runtime | [T155](12-custom-descriptors/T155-desc-allocate.md), [T119](08-gc-and-type-system/T119-gc-casts.md) |
| [T157 — Pass custom-descriptor core conformance](12-custom-descriptors/T157-desc-gate.md) | wtime-conformance | [T156](12-custom-descriptors/T156-desc-cast.md), [T120](08-gc-and-type-system/T120-gc-gate.md) |

## 13-metadata-and-text-tooling

| Task | Area | Prerequisites |
| --- | --- | --- |
| [T158 — Decode and inspect branch hints without changing execution](13-metadata-and-text-tooling/T158-branch-hints.md) | core | [T023](01-binary-decoding/T023-decode-custom.md), [T044](03-runtime-foundations/T044-compile-ir.md) |
| [T159 — Support standard annotations in conformance text tooling](13-metadata-and-text-tooling/T159-annotations.md) | wtime-conformance | [T012](00-foundation/T012-script-runner.md), [T023](01-binary-decoding/T023-decode-custom.md) |

## 14-integration-and-release

| Task | Area | Prerequisites |
| --- | --- | --- |
| [T160 — Pass the complete integrated Core 3.0 corpus](14-integration-and-release/T160-core3-gate.md) | wtime-conformance | [T058](04-mvp-execution/T058-mvp-gate.md), [T059](05-scalar-and-reference-proposals/T059-sign-extension.md), [T061](05-scalar-and-reference-proposals/T061-sat-execute.md), [T063](05-scalar-and-reference-proposals/T063-multivalue-execute.md), [T066](05-scalar-and-reference-proposals/T066-refs-tables.md), [T068](05-scalar-and-reference-proposals/T068-bulk-memory.md), [T069](05-scalar-and-reference-proposals/T069-bulk-tables.md), [T071](05-scalar-and-reference-proposals/T071-extconst-evaluate.md), [T074](05-scalar-and-reference-proposals/T074-funcref-execution.md), [T076](05-scalar-and-reference-proposals/T076-tail-execute.md), [T079](06-memory-proposals/T079-multi-memory-execute.md), [T083](06-memory-proposals/T083-memory64-bulk-grow.md), [T101](07-vector-proposals/T101-simd-gate.md), [T105](07-vector-proposals/T105-relaxed-gate.md), [T120](08-gc-and-type-system/T120-gc-gate.md), [T126](09-exception-handling/T126-eh-gate.md), [T158](13-metadata-and-text-tooling/T158-branch-hints.md), [T159](13-metadata-and-text-tooling/T159-annotations.md) |
| [T161 — Pass all active Phase 3–5 core proposal corpora](14-integration-and-release/T161-phase3-gate.md) | wtime-conformance | [T136](10-threads-and-atomics/T136-threads-gate.md), [T089](06-memory-proposals/T089-compact-import-text.md), [T088](06-memory-proposals/T088-compact-import-integration.md), [T092](06-memory-proposals/T092-wide-conformance.md), [T086](06-memory-proposals/T086-page-execute.md), [T147](11-stack-switching/T147-cont-gate.md), [T157](12-custom-descriptors/T157-desc-gate.md), [T160](14-integration-and-release/T160-core3-gate.md) |
| [T162 — Verify complete feature enable/disable behavior](14-integration-and-release/T162-feature-profile-tests.md) | core | [T161](14-integration-and-release/T161-phase3-gate.md), [T004](00-foundation/T004-features.md) |
| [T163 — Exercise proposal interaction scenarios](14-integration-and-release/T163-interaction-tests.md) | wtime-conformance | [T161](14-integration-and-release/T161-phase3-gate.md) |
| [T164 — Fuzz parser and validator with reproducible mutations](14-integration-and-release/T164-fuzz-decode-validate.md) | tooling | [T033](02-validation/T033-validated-api.md), [T008](00-foundation/T008-opcode-ledger.md), [T012](00-foundation/T012-script-runner.md) |
| [T165 — Compare deterministic semantics to independent references](14-integration-and-release/T165-differential-execution.md) | tooling | [T058](04-mvp-execution/T058-mvp-gate.md) |
| [T166 — Audit panic, allocation, handle and thread failure paths](14-integration-and-release/T166-runtime-hardening.md) | core | [T164](14-integration-and-release/T164-fuzz-decode-validate.md), [T165](14-integration-and-release/T165-differential-execution.md), [T163](14-integration-and-release/T163-interaction-tests.md), [T136](10-threads-and-atomics/T136-threads-gate.md), [T147](11-stack-switching/T147-cont-gate.md), [T157](12-custom-descriptors/T157-desc-gate.md) |
| [T167 — Meet compile-time and dependency budgets](14-integration-and-release/T167-fast-build-gate.md) | tooling | [T166](14-integration-and-release/T166-runtime-hardening.md), [T009](00-foundation/T009-build-baseline.md), [T010](00-foundation/T010-dependency-policy.md) |
| [T168 — Document embedding, feature scope and compatibility](14-integration-and-release/T168-public-api-docs.md) | documentation | [T163](14-integration-and-release/T163-interaction-tests.md), [T167](14-integration-and-release/T167-fast-build-gate.md) |
| [T169 — Complete the proposal coverage and release audit](14-integration-and-release/T169-release-audit.md) | documentation | [T161](14-integration-and-release/T161-phase3-gate.md), [T162](14-integration-and-release/T162-feature-profile-tests.md), [T166](14-integration-and-release/T166-runtime-hardening.md), [T167](14-integration-and-release/T167-fast-build-gate.md), [T168](14-integration-and-release/T168-public-api-docs.md) |
