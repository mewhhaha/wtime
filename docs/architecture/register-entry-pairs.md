# Register entry pairs

`tools/wtime-bench/entry_batch_adapter.py` builds the P135 entry command against
either an immutable Register archive or a snapshot of the current workspace.
Both builds use the same small entry point and Rust harness, with
`runtime-only,register-executor` enabled in release mode. They compile the four
core crates without Wasmtime or execution instrumentation. The current build
has a separate candidate kind and records `source_origin: current-workspace`.

The builder retains the workspace sources, harness overlays, generated Cargo
manifest, lockfile, toolchain commands, Cargo artifact record and executable.
Cargo metadata determines the target directory; the builder preserves any
existing target-directory configuration. Copied Rust files receive fresh
modification times. Failed staging directories retain the executed helper and
command logs. Verification checks complete membership and content hashes, then
compares the snapshot with its original inputs. The captured Git HEAD records
provenance; source hashes determine whether a current build can be reused.

`entry_batch_compare.py` runs each side in a separate process using exactly the
same fixture, oracle, call count, Register strategy and threshold. Each process
creates the fresh lifetime specified by P135. Collection performs no guest
warmup or calibration and retains the original integer nanoseconds. The
`entry_total` metric is the whole lifetime: preparation, instantiation and all
checked invocations. Phase timings remain separate raw fields and must sum to
the total. Results are never divided by the number of calls.

Two deterministic seeds produce separate randomized process orders. Each pair
binds the run, row, order, source identities, executable hashes, fixture bytes,
exact argv, exit status and untouched stdout/stderr. Complete pairs enter the
existing crash-safe checkpoint journal. Resume revalidates every completed
pair before launching more work, preserves interrupted command attempts, and
writes new attempts into separate directories. A complete report requires the
entire declared matrix and can be revalidated against sources and the journal
without running Wasm again.

P136 smoke covers eager threshold zero and adaptive threshold three at 1, 10,
100 and 10000 calls, with two pairs per row in each of two runs. Its receipt
states `timing_evidence: false`; it establishes collection integrity and makes
no performance decision. An explicit same-source alias is accepted only for
smoke. Candidate collection requires distinct archived and current build
records and at least 30 pairs per row in each of two runs. Workload selection,
threshold choice, statistical analysis and promotion remain later gates.
