# Selected-tier premise revalidation

This review updates the source binding of the existing, limited premise audit.
It does not establish exhaustive formal semantics or cover every boundary case.
Every original `all_test_boundaries_covered: false` remains false. The original
opcode ledger and its audit files remain historical inputs.
The ledger's top-level `core_commit` remains historical provenance; the selected
report's source hashes identify the current implementation and test files.

The comparison baseline is the hash-verified source archive `P017-before`.
The P109 report records the exact old and current files and retains their diffs.
The following conclusions apply only to those recorded hashes.

The validator's acceptance predicates are unchanged. `validate` additionally
reserves a vector with `try_reserve_exact` and stores the bounds returned by
each body. A reservation failure returns `ResourceLimit` before publication.
`Context::body` keeps the same local/type/stack/control checks and records the
maximum observed value-stack and control-stack lengths after each instruction.
Its final unterminated-function check still precedes successful return. These
observations are allocation hints, not permission to remove runtime checks.
The existing P021 tests exercise nesting, multivalue peaks, large local indices,
budget decline and reference layouts; the validator's full acceptance tests
and all four normative configurations provide separate behavioral evidence.

The old `Context::function` label is narrative rather than a Rust function.
The refreshed review identifies the actual `validate`, `Context::body`,
`Context::ty` and `Context::valtype` helpers. Their source file and the type
environment dependencies are bound together, rather than treating an unchanged
instruction function as proof of unchanged transitive behavior.

In the linker, the `Module` fields are now accessed through immutable shared
`ModuleData`; the canonical type and decoded-module inputs retain their roles.
Compact-only call and memory binding tables are initialized and prepared before
publication/start execution. Their allocation decline preserves the existing
decoded fallback. The subtype predicates and import type checks remain active.
The selected-tier tests cover foreign references, exact import types, recursive
and imported inline declines, mutable indirect targets, initialization failure
and source-fuel effects. Full normative runs cover both accepted and rejected
imports in every selected configuration.

`limits_match` is defined in runtime `lib.rs`, although the historical narrative
associated its rule with the linker. The new review binds both files and the
full current core source set. Its lower/upper bound predicate is unchanged in
the compared source. This repairs the owner reference without claiming that
the old linker file hash covered that helper.

For the remaining audited helpers, P109 binds their current source, preserves
the original scope and remaining-premise fields, checks that named tests really
passed, and rebinds exact locked corpus forms to the new normative reports.
Opcode execution counts prove dispatch coverage; they do not prove all paths.
Any source change invalidates this selected-tier report and requires review of
the changed premises and fresh affected tests.
