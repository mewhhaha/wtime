# Milestones and parallel work

Use actual dependency edges in [TASKS.json](TASKS.json), not a blanket sequential interpretation of these waves. Each milestone requires executed, counted tests; none has been completed by writing this plan.

| Milestone | Deliverable and exit gate | Parallel work after shared contracts |
| --- | --- | --- |
| M0 — foundation | Pinned scope, crate contracts, workspace, type/feature shapes, corpus runner and build baseline | Reader/error work, tooling/corpus inventory, handles contract and measurement scripts |
| M1 — useful MVP | Validated native interpreter, state/imports/start, minimal CLI; T058 passes complete selected MVP corpus | Decoder sections; validator instruction families; scalar numeric, memory and control execution |
| M2 — finished core | All integrated Core 3.0 semantics; T160 passes the complete selected core corpus | Scalar/reference/tail features; indexed/wide memories; SIMD; recursive types/GC, then EH |
| M3 — active mature core | Threads, compact imports, wide arithmetic, custom pages, stack switching and core custom descriptors; T161 passes their corpora | Threads after memory contracts; continuations after GC/EH; descriptors after GC/types; compact imports and wide arithmetic earlier |
| M4 — shippable snapshot | Cross-feature tests, fuzz/differential triage, API hardening, build budgets, docs; T169 release audit | Fuzzing and independent reference comparisons; documentation; build-cost review, coordinated on integration revisions |

M2 and M3 overlap: compact imports and wide arithmetic should not wait for all GC work. The final all-proposals gate requires both.

Fuzz-harness work (T164) starts once baseline validation exists, and independent execution comparison (T165) starts at MVP. They are listed in the integration group for ownership, not delayed until all features are implemented. T166 reruns them against the complete profile. The descriptor source audit (T148) also starts immediately after the initial scope freeze, before the shared type shapes land.

## Important dependency paths

- Core bootstrap: scope → crate contracts/workspace → bounded decoder → sealed validator → execution stacks and runtime objects → scalar/control/calls → MVP conformance.
- Type-rich control: function references → recursive type validation/canonicalization → traced heap/roots → GC → exceptions → continuation types/prompts → resume/suspend/switch → stack-switching conformance.
- Descriptors: GC/type identities → descriptor clauses/exactness → exact import matching and descriptor allocation/casts → descriptor conformance.
- Shared memory: memory widths/page units → shareable storage and SC coordination → atomics/waiters → thread harness → threads conformance.
- Release: all feature gates → interaction/profile coverage → fuzz/reference comparisons → hardening → measured build budgets/docs → release audit.

## Scheduling rules

Keep shared contract changes small and integrated before parallel consumers diverge. Parser, validator and runtime counterparts deliberately have separate tasks; feature gates catch incomplete plumbing. Conformance, security and compile-time work are deliverables with task IDs, not unspecified work after implementation.

No calendar or effort estimate is asserted: implementing GC, threads and continuations correctly is substantial. Small patches reduce review risk; they do not make the total project small. Re-estimate after MVP and after the first recursive-type/continuation vertical slice.
