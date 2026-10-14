# T085 — Validate page-size limits and exact import matching

Status: **done**  
Area: `wtime-validate`  
Work unit: one focused behavior/contract and its tests; split into child tasks before exceeding two focused patches.

## Prerequisites

[T084 — Decode custom page sizes and limit flags](../06-memory-proposals/T084-page-decode.md), [T081 — Validate 64-bit address and table operand types](../06-memory-proposals/T081-memory64-validate.md)

Use the predecessor's interfaces and evidence. Do not reimplement a second parser, type registry, store or execution stack.

## Deliverable

Allow only currently specified 1-byte and 64KiB pages; enforce width/page-count limits and equality in memory matching. Reconcile inherited memory64/grow wording against pinned formal rules before implementing inconsistent prose.

## Acceptance

Reject unsupported powers of two and unequal-page imports; i32/i64 bounds use their actual widths. Document any normative inconsistency as a scope issue.

Add focused tests for the changed behavior and relevant failure boundaries. Record the exact command, feature profile, corpus/source revision and executed assertion counts. Follow [the common definition of done](../EXECUTION.md#common-definition-of-done); a missing directive, zero-test run or unimplemented branch is not a pass.

## Boundaries

Follow [ARCHITECTURE.md](../ARCHITECTURE.md): no external core production dependency, no per-proposal Cargo-crate proliferation, no JS/browser implementation, and no invented semantics for draft TODOs. Coordinate changes to shared types/opcode/control contracts with other ready tasks. Runtime feature gates and implementation resource limits must remain explicit.

## Implementation instructions for Sol 6.1 Low

Read [the interface contracts](../IMPLEMENTATION_CONTRACTS.md) and the listed prerequisite outputs before coding. Use the predecessor's API; do not invent a second implementation to avoid a missing prerequisite.

### Files and ownership

Expected paths in `/workspace/wtime` (created by implementation, not by this plan):

- `crates/wtime-validate/src/types.rs`
- `docs/architecture/page-limits.md`

Add focused tests in the relevant package's `tests/` directory, or its existing unit-test module. Name tests `t085_case_01`, `..._case_02`, etc. Reuse existing feature modules when present; coordinate edits to central dispatch/type tables.

### Ordered recipe

1. Resolve the draft's inherited width assumptions in a written rule table before coding.
2. Permit only log2 0 or 16; check limits using actual index width and page size.
3. Require equal page size when matching imports.
4. Add the cases below before considering the behavior finished. Keep malformed, invalid, trapping, host-error and resource-limit outcomes distinct.
5. Wire the change into the existing entry point/dispatch and update its feature-policy or ownership evidence where relevant. Run the commands below and record exact outcomes.

### Required cases

- `t085_case_01`: 2-byte pages invalid
- `t085_case_02`: One-byte page limits near width bound
- `t085_case_03`: Mismatched page-size import.

For binary fixtures, keep a readable WAT/hex source and provenance alongside the `.wasm` or inline bytes. Generate fixtures with the separate encoder/wasm-tools; do not add `wast` to core tests. Expected outputs must come from the locked spec/rule or an independent calculation, not the engine being tested.

### Verification commands

Run from `/workspace/wtime` after activating the configured Rust environment:

```sh
. /workspace/.setup/rust-env.sh
cargo test --locked -p wtime-validate t085_ -- --nocapture
cargo check --locked -p wtime-validate
cargo fmt --all --check
cargo clippy --locked -p wtime-validate --all-targets -- -D warnings
```

The filtered test invocation must execute the named cases with a positive count. If multiple packages are listed, at least each package with changed behavior must have an executed relevant test; a package's zero-match summary is not evidence for that package. Fix test placement/names rather than accepting a zero-test run.

### Handoff and scope limit

Record changed symbols/files, named test counts, corpus hash/profile if used, and remaining risks in Completion evidence. Stop at this task's interface; do not implement unrelated future features. If a prerequisite API is missing or pinned sources conflict, identify the exact missing contract/rule and keep the affected work blocked instead of guessing. Routine local coding choices do not require asking the user.

## Sources

- [Custom Page Sizes](https://github.com/WebAssembly/custom-page-sizes/tree/61c873a1860acad2772539164f284cc5110f1b1b)
- [Pinned integrated core specification](https://github.com/WebAssembly/spec/tree/7e9fb78c63d65018cc111422f657947b3df80d7b) is normative for finished core features; historical proposal repositories provide context.
- [Scope, source precedence and exclusions](../PROPOSALS.md).

## Completion evidence

- Changes: Only page logs0/16 are legal. Page counts fit byte address space and memory.size result width; memory32 one-byte pages cap at u32::MAX. Linker also requires page-size equality.
- Handler: `crates/wtime-validate/src/lib.rs` — `validate memory limits; runtime import_subtype page equality`.
- Focused/shared tests: `acceptance::t090_custom_page_count_fits_memory_size_result`, `decode::t085_invalid_page_size_is_semantic`, `feature_profiles::t162_case_02`.
- Commands and outcomes: `cargo test --locked -p wtime-validate` 90 passed; `cargo test --locked -p wtime-runtime --test feature_profiles` 28 passed; `cargo test --locked -p wtime-conformance --test interactions` 4 passed; `cargo test --locked -p wtime-runtime t152_` one actual matching test passed (three exact-import cells). Decoder owner reports 24 decoder tests passed. Other binaries' zero-match summaries are not proof.
- Locked corpus: `target/conformance/final-phase3.json`, profile `all-phase3-core`, suite `phase3`, 64297 executed, zero failures/skips/unsupported; binary SHA256 `a0aaaa7d7c6950508f89e877a87bfcdf46d929f9b7e7efbaa9109c1f5101a1be`. 4 exact relevant selected source hashes are recorded in this task's `TASKS.json` completion evidence; live source hashes were checked before recording.
- Formal/spec evidence: pinned core `7e9fb78c63d65018cc111422f657947b3df80d7b`; actual rule predicates and exact unit/corpus forms in `docs/architecture/validation-premise-audit.json` and `docs/architecture/opcodes.json`. Historical proposal source precedence and active overlays remain explicit.
- Quality checks: strict validator Clippy passed after final source changes. No source edits were made during task reconciliation.
- Follow-up: none for validation. The release/conformance owners maintain final aggregate source bindings; decoder/runtime work is supported by the shared evidence above.
