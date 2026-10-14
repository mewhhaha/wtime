# Agent instructions

## Delegation

Use `gpt-6-luna` subagents for exploration and chores: read-only source
inventories, fixture and license inventories, mechanical tooling, task-index
maintenance, and review checks. Give each agent a fresh, focused context and
disjoint owned paths. Do not use Luna for cross-cutting runtime semantics or
unsafe native-code implementation.

Keep one integrator responsible for runtime `lib.rs`, dispatch, shared frame
and configuration definitions, Cargo features, and performance test
registration. Integrate overlapping changes serially.

Synthetic tooling tests must create a fresh temporary root and check that every
resolved output path stays inside it before writing. Never seed or overwrite
real absolute provenance paths when constructing synthetic inputs. Preserve
executed helper sources alongside their command logs before running them.
Disable Python bytecode writes when importing archived sources (`python3 -B`
or `sys.dont_write_bytecode = True`) so evidence readers preserve exact archive
membership.

## Performance work

The performance plan lives at `../tasks/performance`. Read its `EXECUTION.md`
and the task's contracts before implementation. Follow manifest dependencies,
archive the original baseline before changing runtime sources, and preserve
historical benchmark evidence. Tasks need executed checks and real evidence;
do not mark planned work complete or turn semantic failures into rejected
experiments. Record inconclusive or rejected performance results honestly.

Use `tools/verify.py` and task-owned declarative contracts for new verification
work; read `tools/verification/README.md`. Preflight before execution, keep
freeze, correctness/build checks, measurement, analysis, and finalization
separate, and group tests by their exact feature configuration. Compute paired
statistics and the gate decision once. Revalidate immutable receipts when
packaging changes; rerun only phases whose consumed inputs changed. Preserve
all task-required semantic cases. Publish status through the shared publisher
after reviewing its concrete completion record.

Iterate with `tools/verify.py dev` and task-owned `development.groups`; use
locked incremental tests/checks and exact feature configurations. Development
receipts cannot satisfy full phases or publish completion. Run `tools/verify.py
close` when ready to execute/reuse the full contract and finalize. Include the
transitive compilation sources and fixture inputs in each group's identities.
The repository's Python groups are in `docs/verification/tooling-development.json`.
Use `tools/verify.py perf` to check verification latency and work budgets; these
synthetic measurements are separate from Wasm performance evidence.

Follow `docs/verification/fast-performance.md` for the adopted iteration
schedule. Freeze task-owned candidate selections against the immediate
predecessor before runtime edits; retain every semantic case and protected
control. Use complete portfolios and P001 totals at milestones. Analysis-only
edits must not retime unchanged workloads. Reuse only source-current immutable
receipts; preparation aliases share raw pairs and uncertainty, never extra
independent samples. Development/smoke or synthetic evidence cannot promote.

The default core must remain safe Rust with zero external dependencies.
Experimental tiers stay disabled by default and must preserve Wasm semantics,
resource limits, source-level fuel, and cancellation contracts.

Validate fixtures before treating their results as runtime evidence. Use
different source, destination, and page contents for copy and cancellation
checks so a missing operation cannot pass as an unchanged memory image.
Count the implicit function-end instruction in source-fuel expectations.
Retain failed fixture drafts and distinguish baseline checks from tests of an
integrated optimization.

Activate Rust with `. /workspace/.setup/rust-env.sh`. Run focused checks after
bounded patches, broader suites at integration gates, and both task-plan
validators when changing task status or dependencies.

Restore archived source contents with fresh modification times, or explicitly
refresh timestamps before invoking Cargo. Old timestamps can reuse candidate
artifacts after rollback; diagnostic path checks must prove the restored build.
