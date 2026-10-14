# Instructions for Sol 6.1 Low

Work in `/workspace/wtime`; this plan lives at `/workspace/tasks/performance`. Activate Rust before commands:

```sh
. /workspace/.setup/rust-env.sh
cd /workspace/wtime
```

Use the shared verifier for new tasks: see [`tools/verification/README.md`](../../wtime/tools/verification/README.md). Start with `python3 -B tools/verify.py plan --contract CONTRACT.json`, then execute the declared `freeze`, `verify`, `measure`, and `analyze` phases. Preserve each task's correctness and regression cases. Group Rust tests by exact feature configuration, with one command and an exact feature list per group. Add task-specific semantic checkers when primitive JSON checks cannot express the acceptance contract. Finalize the bound phase receipts, inspect the completion record, and publish it with the separate `publish --execute` command. Use a task-specific workflow only when the shared contract cannot represent its requirements, and document that limitation.

Define incremental `development.groups` beside the full stages for quick iteration. Run `python3 -B tools/verify.py dev --contract CONTRACT.json --group GROUP --execute`; groups reuse unchanged inputs and report why they rerun. Use locked incremental Cargo tests/checks or verbose Python unittest, and bind all transitive compilation, fixture, helper, and configuration inputs. Development receipts cannot complete a task. When ready, run `python3 -B tools/verify.py close --contract CONTRACT.json --output NEW-EXCLUSIVE-DIRECTORY --execute` to execute or reuse the full phases and finalize. Publication remains separate. Use a fresh global `--profile-output PATH` before the subcommand for counters and wall time.

Read the task and its linked contracts. Inspect its named current symbols before editing: the plan records today's source, and earlier tasks may relocate it. Use the manifest dependencies to select work. No task is already implemented by the creation of this plan. Preserve the original task directory and reports. Native AArch64 evidence in the original plan remains outstanding; cross-compilation is not native execution evidence.

## Candidate execution schedule

Follow the three lanes in [MEASUREMENT.md](MEASUREMENT.md): development receipts are iteration evidence only; candidate gates consume a frozen task-owned selection and the immediate-predecessor archive; milestone gates retain exhaustive family and P001-total coverage. Before editing a candidate, save and verify the exact pre-edit tree and executable in a new immutable archive. Freeze its selection manifest at the same time, including target shapes, small/large rows, held-out rows, protected controls, stages, settings and cold-product requirements. The manifest and archive identities must be bound into verification and measurement receipts.

Candidate gates run the complete declared row set in two independent runs with at least 30 complete pairs per row per run and 10,000 bootstrap resamples. Candidate timing may select a focused set, but all task-owned semantic cases, source-prefix/fuel/cancellation checks, optimizer bounds, memory/trap oracles, fallback coverage, and hard build/dependency limits still run. Candidate promotion compares to the immediate predecessor. P001 remains the original baseline for exhaustive milestone reporting only.

Milestone recipes run every supported row in each required family against the combined selected tier and preserve full semantic/conformance counts. They report both immediate-predecessor changes and P001-relative milestone totals as separate analyses. Do not convert missing measurements, unsupported directives, skipped semantic cases, or hard failures into selection exclusions.

## One task, one bounded patch

Each task specifies paths, an input/output contract, ordered steps, at least three meaningful cases, and commands. Add a test or report validator that would fail if the behavior were wrong; do not add implementation-mirroring or zero-test checks. No task authorizes a broad rewrite of unrelated proposal semantics. If the patch needs more than two focused implementation changes, split child IDs, update the graph/index, and retain the parent's acceptance gate.

Before a runtime patch, run/verify P001's archived baseline. Implement the smallest eligible subset in the recipe. Preserve fallback for anything outside it. For a rejected experiment, retain its decision/report and remove default-path cost; code may remain only behind its experiment feature if its maintenance cost is justified. Record the outcome separately from task status.

The manifest distinguishes `foundation`, `candidate`, `gate`, and `experiment`. Foundation tasks deliver tools/contracts; candidates need measured promotion; gates verify a milestone; experiments are optional alternatives. `enable_when` is a branch condition, not an approval requirement. A native branch requires its decision artifact to say `go`; a `no_go` defers descendants. A build/semantics failure cannot be converted into `not_selected` to hide it. The final portfolio gate can complete without implementing unselected experiments and must list them explicitly.

## Shared files and tests

Keep one integrator for runtime `lib.rs`, execution dispatch, frame/configuration definitions, Cargo features, and `tests/performance.rs`. A task may own a leaf module and propose an integration change; overlapping paths must be integrated serially even if the dependency graph permits parallel analysis. Register new focused tests in one `performance` integration-test target with `#[path = "performance/pNNN.rs"]` modules. Use test names beginning `pNNN_`; the minimum relevant count is three unless the task states a stronger requirement. P016 creates the registration target. Do not create a separate integration-test binary for every performance task.

Measurements and offline/native tools have their own Python/Rust tests. The future command contracts are listed in [MEASUREMENT.md](MEASUREMENT.md); each producer task must make its dependent commands work. Capture the command's real status and executed counts. A report with zero cases, missing coverage, or stale hashes fails. Instrumentation is never enabled in the release measurements.

Luna may do read-only exploration, fixture/license inventories, checker/index generation, and mechanical review. Use fresh focused contexts and disjoint paths. Sol Low implements the recipe; the integrator reviews cross-cutting safety/semantic changes. No automatic worktree creation is needed.

## Completion record

Update task status/outcome in both its Markdown and TASKS.json. Add evidence paths, exact command/settings, source/binary/fixture hashes, passed/failed/skipped counts, paired performance decision, build/dependency deltas, remaining limitations, and any rejected alternatives. `done` means the defined deliverable was executed and checked, not that a speedup target was promised. `deferred` with `not_selected` records an unimplemented conditional child.

Run the new and original plan checkers after status/graph/link changes. Run focused crate/corpus checks for ordinary patches; broader conformance, allocation/hardening, build, and native gates belong to integration tasks. Do not repeatedly run full suites after an unchanged passing patch. The final gate refreshes evidence for the selected tiers and retains historical evidence separately.
