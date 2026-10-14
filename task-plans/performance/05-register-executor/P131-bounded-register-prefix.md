# P131 — Prepare bounded Register prefixes for decoded handoff

Status: **done**

Outcome: **contract_only**

Kind: `foundation` · Branch: `portable` · Model: **Sol 6.1 Low**

## Prerequisites

[P057 — Define a small numeric register bytecode and eligibility rules](P057-register-ir-contract.md)

[P058 — Translate numeric stack code into direct slot operands](P058-stack-to-register-lowering.md)

[P063 — Add direct checked unshared memory register operations](P063-register-unshared-memory.md)

[P129 — Add bounded source-visit selection to execution diagnostics](../01-execution-contracts/P129-source-visit-diagnostics.md)

## Scope and ownership

Read [EXECUTION.md](../EXECUTION.md), [SOURCE_NOTES.md](../SOURCE_NOTES.md), and [BACKEND_CONTRACTS.md](../BACKEND_CONTRACTS.md), plus contracts **C1, C2, C3, C5, C6, C7, C8** in [CONTRACTS.md](../CONTRACTS.md).

- `crates/wtime-runtime/src/execution/register/ir.rs`
- `crates/wtime-runtime/src/execution/register/lower.rs`
- `crates/wtime-runtime/src/execution/register/execute.rs`
- `crates/wtime-runtime/tests/performance/p131_private.rs`
- `crates/wtime-runtime/tests/performance/p131.rs`
- `tools/verification/producers/p131.py`
- `docs/benchmarks/performance/P131-verification.json`
- `docs/architecture/register-prefixes.md`

P131 adds a separate bounded prefix constructor and test model; it does not weaken the existing whole-function IR verifier. The exhaustive whole-function execution match in `execute.rs` explicitly rejects an unselected `ExitToDecoded` barrier, while the shared implementation remains gated to prefix construction and the whole-function source verifier retains its strict coverage. It establishes the immutable, source-bound artifact that P064 can later use for an explicit decoded handoff. P131 does not select Register policy or execute the decoded barrier. One integrator owns lib.rs, dispatch, Cargo features and tests/performance.rs; keep any registration hunk bounded.

## Input/output contract

The prefix artifact represents source positions `0..=barrier_pc`: it keeps source events through the first barrier inclusive, executable operations only before that barrier, and the exact typed state at entry to the pending barrier. It retains locals, logical operands, active source labels, source PC, source bytes, Engine/body identity and numeric layout identity. Open block and loop labels are valid at the prefix boundary. The foundation test model compares the pre-barrier state without executing the barrier or consuming its source fuel. The artifact is bounded by 100,000 work units and 1 MiB of retained/temporary metadata; invalid source identity, cursor, types, event holes, control metadata, barriers or budgets decline transactionally.

Prefix execution may include numeric operations, locals, constants, drop, nop, block/loop/end and supported ordinary unshared memory operations. The first `if`, `else`, `br`, `br_if`, `br_table`, return, call, reference, GC, exception, memory-growth, atomic or unsupported-memory source instruction is a pending decoded barrier. Operations before it retain their existing register slots and P063 memory plans in source order. This lets call-kernel setup execute through the condition computation and stop with the `br_if` still pending. P064 owns production materialization and resumes that exact decoded opcode in the same charged iteration exactly once.

## Implementation recipe

1. Define a distinct private prefix artifact and constructor in `ir.rs`/`lower.rs`; keep whole-function admission, coverage checks and verification strict and unchanged. In `execute.rs`, reject the unselected `ExitToDecoded` barrier in the exhaustive whole-function execution match; keep the shared implementation gated to prefix construction. Specify that the barrier event is retained for source-boundary identity while the prefix operation stream ends before it.
2. Reuse the P058 typed local/operand layout and P063 checked memory plans for every admitted operation before the barrier. Preserve original event order, source PCs, effects, successor/layout metadata and logical operand heights; do not compact operations across source boundaries.
3. Stop at the first barrier from the declared set. Permit block, loop and end as prefix instructions and preserve their active source labels, including open labels at the boundary. Treat `if` and other control transfer, calls, refs, GC, exceptions, memory growth, atomics and unsupported memory forms as pending decoded instructions, never as register operations.
4. Bind each prefix to the exact source bytes, Engine, body and numeric layout. Validate cursor/barrier ordering, event continuity, types, slot ranges, label metadata and memory bindings before publication. A wrong owner, malformed barrier, hole or unsupported operation declines without publishing a partial prefix.
5. Reserve worst-case work and metadata with checked arithmetic under the existing aggregate 100,000-work/1-MiB limits. Use transactional budget accounting so invalid metadata, allocation failures and budget exhaustion leave the shared budget unchanged.
6. Add the named differential tests and the P131 producer/contract. Compare state immediately before each barrier against decoded source snapshots; prove the barrier itself is neither executed nor charged in the foundation model. Document artifact ownership and the P064 handoff boundary in the architecture note.

## Meaningful cases

- `p131_prefix_values_and_source_boundaries_match`: numeric/local/constants/drop/nop prefixes match decoded typed values and source positions; call-kernel setup stops before a pending `br_if` with its condition still present.
- `p131_open_labels_and_memory_prefixes_match`: block/loop/end and open source labels, plus supported unshared loads/stores, match decoded state and memory at every selected prefix.
- `p131_invalid_barriers_and_budgets_decline`: unsupported barriers, malformed cursors/types/control metadata, holes, foreign source ownership and work/byte exhaustion decline transactionally with no charged barrier or partial artifact.
- `p131_default_policy_remains_decoded`: the feature-gated prefix machinery does not make Register selectable in the default build.

## Verification

Activate Rust as described in EXECUTION.md. These commands verify the feature-gated prefix model, the default policy, and clean build/dependency budgets. The producer contract binds its freeze and semantic verification evidence to P001 and the fresh P131-before archive; it declares no measurement or promotion phase.

```sh
cargo test --locked -p wtime-runtime --features register-executor,execution-reference p131_ -- --nocapture
cargo test --locked -p wtime-runtime --test performance p131_default_policy_remains_decoded -- --nocapture
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml --features register-executor -j 4
python3 tools/checks/build_budget.py --output target/performance/P131/build.json
python3 tools/checks/build_budget.py --features wtime-runtime/register-executor --output target/performance/P131/optional-build.json
python3 -B tools/verify.py plan --contract docs/benchmarks/performance/P131-verification.json
python3 -B tools/verify.py close --contract docs/benchmarks/performance/P131-verification.json --output target/performance/P131/complete --execute
```

## Measurement and decision

Selection: `none`. Explicit execution setting: `register`. Decision rule: `integrity`.

This is a bounded IR/verification foundation with no benchmark selection and no speed claim. It must not add a policy selector or execute a guest handoff. P064 remains responsible for production-bank materialization, preserving charged-iteration semantics, and executing the pending decoded opcode exactly once.

## Before editing

Verify the existing immutable P001 archive and its successful correctness record. Then create a fresh, immutable Register-mode P131-before archive before runtime edits; never reuse or overwrite an archive with different source identities.

```sh
python3 -B tools/wtime-bench/baseline.py --output target/performance/P131-before --mode register
```

## Completion evidence

Record P001 and P131-before archive identities, helper argv and logs, source/test/artifact hashes, exact commands and feature settings, named test counts, build/dependency budgets, and transactional decline results. The durable phase and completion records are linked below.

## Executed evidence

P131 completed with outcome `contract_only`: freeze and verify phase attempt `0001` passed, close `0077` and finalization attempt `0001` were published as `P131-publication-0001`. The fresh P131-before archive used the direct Register source at commit `bdbf60e2eec4295b58d0fb9c342a77d840066f03`, with the unchanged protocol and no adapter. P001 and the pinned P057, P058, P063, and P129 prerequisites are recorded with identities in the durable records.

Verification passed 290 tests: 289 Register/reference tests and one default-policy test, with zero failures and zero ignored tests. The source-boundary evidence comprises 52 assertions: 26 from the aggregate marker and 26 from the additional private case. The aggregate marker covers 26 prefixes, at most 259 work units and 2,179 bytes, two open labels, and two memory prefixes. Open-label and current-source typed-bank state, complete memory images, invalid barriers/cursors/types, foreign owners, fuel and budget declines were checked transactionally.

The foundation model does not execute or charge the pending guest barrier. Production prefix selection remains deferred to P064, so this task makes no speed or timing claim. Default and optional Register builds passed hard budgets with zero external dependencies. Clean build times were 2.798023 s (default) and 3.111263 s (optional Register); stripped binaries were 1,000,712 and 1,073,840 bytes.

Durable records and artifact hashes are indexed in [`P131-records.json`](../../../wtime/docs/benchmarks/performance/P131-records.json). The published completion and evidence are [`P131-completion.json`](../../../wtime/docs/benchmarks/performance/P131-completion.json) and [`P131-evidence.json`](../../../wtime/docs/benchmarks/performance/P131-evidence.json); freeze and proof artifacts are [`P131-freeze.json`](../../../wtime/docs/benchmarks/performance/P131-freeze.json) and [`P131-proof.json`](../../../wtime/docs/benchmarks/performance/P131-proof.json). Build attestations are [`P131-default-build.json`](../../../wtime/docs/benchmarks/performance/P131-default-build.json) and [`P131-register-build.json`](../../../wtime/docs/benchmarks/performance/P131-register-build.json). The records index contains the phase receipt paths and hashes.
