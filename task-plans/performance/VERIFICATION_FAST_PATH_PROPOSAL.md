# Performance verification schedule

Status: **adopted schedule**. Existing task acceptance requirements and historical evidence remain in force; task-specific candidate selections are frozen before implementation.

## Observed bottleneck

On 2026-10-13, P046's first full scalar comparison against P001 took about
30 minutes to produce 25,500 complete pairs (85 workloads, five stages,
two runs, 30 pairs per group). It passed the actual wire/source validator.
Its report SHA-256 is
`b7c898cbddec0385b64b77e654f270a67f23eaadb07c4122dfdc4cc9f9b8f8e8`.

P046 currently requires ten comparison datasets: four complete families
against two baselines plus two original-MVP cold selections. That is
133,920 pairs / 267,840 side observations. Slow frozen guest execution and
matched warm batches dominate the cost. This is separate from Rust compile
time: the latest default clean build took 9.4475 seconds and passed its budget.

Executed verification is retained in
`../../wtime/target/verification/runs/P046/verify/attempt-0003`:
17 gate tests, three allocator diagnostic tests, 89 reference/tier checks,
1,095 actual frozen-P001 capability stage probes, 57 retained module/profile
products, and both build budgets. The source-current full conformance report
still covers 145,760 directives with no failures/skips/unsupported directives.

## Adopted schedule

1. **Development:** exact changed-path semantic tests, locked incremental
   builds, and source-bound smoke timings. Development evidence never promotes
   a candidate or completes a task.
2. **Candidate gates:** before implementation, declare the optimized shapes,
   small/large and held-out inputs, and protected unaffected controls. Keep
   two independent runs, 30 paired observations, 10,000 bootstrap resamples,
   production binaries, exact source fuel/cancellation, memory/trap oracles,
   and build/dependency gates. Time only these declared candidate/control
   rows instead of every unrelated scaling variant in every family.
3. **Semantic coverage:** retain all task-specific meaningful cases,
   optimizer corruption/budget checks, source-prefix tests, and complete
   proposal/interaction correctness at integration milestones. No unsupported
   directive or fallback coverage is hidden.
4. **Milestones:** run the exhaustive four-family portfolio on the combined
   selected tier. Measure P001 milestone totals here; use the immediate
   predecessor for ordinary candidate decisions.
5. **Cold preparation:** independently measure each unique Wasm/profile/
   configuration product. Any workload aliases must explicitly reference that
   product's raw pairs; report unique sample counts and shared uncertainty.
   Never relabel shared samples as independently measured observations.
6. **Receipts:** keep verification, measurement and analysis producers and
   identities separate. Reuse successful, immutable datasets and source-current
   semantic/build receipts. A statistics-only correction must not retime Wasm.

## Implemented workflow

- Task-owned predeclared manifests and focused comparison validation reject
  missing targeted/protected rows and unexecuted semantic cases.
- Cold alias equivalence uses actual Wasm bytes, feature profile,
  runtime settings and stage setup. Input-dependent stages cannot alias merely
  because they share Wasm bytes.
- All 23 remaining candidate recipes use the new schedule, preserving the
  exhaustive milestone requirements and historical evidence.
- Focused, missing-control, alias-substitution and stale-receipt negative
  cases are covered by the fast tooling test group.
- Both task-plan validators pass. See the [workflow and commands](../../wtime/docs/verification/fast-performance.md).

P046 remains pending. The completed scalar dataset is real timing evidence,
but it does not establish the exhaustive portfolio decision or promotion.
Candidate-focused gates do not replace P046/P056 milestone coverage or P001
totals.

The next structural-scalar comparison was intentionally interrupted with
SIGINT. Its partial checkpoint and interruption logs are preserved under
`../../wtime/target/verification/runs/P046/measure/attempt-0002/measurements/structural-scalar`.
The interruption command and reason are archived under
`../../wtime/target/performance/P046/intentional-pause/attempt-0001`.
This is a scheduling pause for replanning, not a runtime semantic failure or
a performance rejection. No task status or promotion decision was published.

## P047 predeclaration recommendation

The current `scalar` registry has suitable exact rows for a bounded first
selection. Declare the two primary target families at small/large scale:
`scalar_accumulator_n32`, `scalar_accumulator_n10000`,
`scalar_dependent_n32`, and `scalar_dependent_n10000`. The accumulator fixture
contains local/local/add plus local.set; the dependent fixture contains
constant arithmetic followed by local.set. Use the registered held-out rows
`scalar_accumulator_n17` and `scalar_dependent_n17` (optionally add the `n0`
edge rows as held-out correctness/performance checks). Protect
`scalar_dynamic_api_n32` as a warm control: its straight-line
local.get/constant/add expression has no local.set, so neither complete P047
fusion matches. Do not use `branch_calls` as a control because its loop
increment contains the local.get/constant/add/local.set fusion. Include the
mandatory `prepare_large` and tiny `call_overhead` `prepare`, `first_call`,
and `load_and_call` cold controls. For candidate promotion, make
`warm_call` the primary comparison stage and retain distinct cold stages for
cold gates.

These names exist in `tools/wtime-bench/workloads.json`; P047's selection
manifest must bind their current registry/source hashes and exact stages. The
P047 implementation still runs every listed semantic case and focused
optimizer test. This focused set is a candidate gate only; P046/P056 retain
the complete four-family milestone and P001-total work.
