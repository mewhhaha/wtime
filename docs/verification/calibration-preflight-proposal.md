# Calibration before exhaustive measurements

Status: proposed, not implemented. P056 remains pending.

## Observed failure

P056 close attempt 0058 passed freeze and full verification, then completed
both scalar comparisons (25,500 pairs each) and both memory comparisons
(10,860 pairs each). Its P001 SIMD comparison stopped after 6,240 pairs:

```
simd_typed_f64_neg_1_75: warm calibration did not reach 20000000ns
within 100000 calls and 8 attempts
```

This follows the earlier scalar-driver capacity failure. The scalar limit was
raised to one million calls, but the generic typed driver still limits batches
to 100,000. No runtime semantic failure has been reported. Four complete
datasets totaling 72,720 pairs are retained in the immutable P056 dataset cache;
partial SIMD evidence and both failed measurement attempts remain preserved.
No P056 statistics or promotion decision has been produced.

## Bounded implementation plan

1. Inventory actual request limits, returned-batch validators, and calibration
   bounds for every selected driver. Start with `src/protocol.rs`'s typed path,
   `src/scalar_driver.rs`, `src/memory_driver.rs`, `compare.py`,
   `focused_compare.py`, and the specialized representative/GC/thread drivers
   under `tools/wtime-bench`. Record a driver-specific capability table. Keep
   stateful/resource-limited driver bounds explicit.
2. Reconcile the generic typed driver's Rust request limit, automatic batch
   selection, Python response validator, and calibration limit. Permit at least
   one million calls for the admitted fast numeric typed cases, matching the
   scalar capacity fix. Preserve bounded growth and finite/positive timing,
   exact requested/returned batches, independent oracles, and the 20 ms floor
   on both sides. Do not reduce sample counts or substitute a shorter interval.
3. Add a task-owned diagnostic calibration producer using the existing
   fixture preparation, observers, and calibration functions. Cover every warm
   group in all four full families against both required baselines, plus the
   independent P047/P050 refresh selections. Validate the original-MVP controls
   as well. It must build nothing and compute no paired statistics.
4. Declare that producer as the first command of P056's `measure` phase,
   after the existing freeze and complete correctness/build phase. Record the
   exact expected/attempted/passed group set, driver capacities, modes,
   source/binary/fixture identities, actual confirmation intervals, and failure
   diagnostics. Fail before starting a paired dataset if any group cannot
   calibrate. Preserve the ordinary per-run calibration during measurement;
   diagnostic confirmations do not become independent paired samples.
5. Verify three boundary cases with focused shared development groups: a fast
   typed numeric case needs more than 100,000 calls and succeeds; a too-small
   reported capacity fails with the affected group named before any paired
   work starts; a mismatched batch, stale source/binary, or failed oracle fails
   closed. Exercise the real Rust typed driver above 100,000 calls, not only
   mocked Python observations. Synthetic files stay inside fresh temporary
   roots and executed helpers/arguments are archived.
6. Freeze fresh adapters and production binaries after the harness changes.
   Revalidate existing immutable evidence without rewriting it. Changed
   harness/binary identities require new measurements; do not loosen cache
   identities to reuse old timings. Analysis-only changes continue to reuse
   unchanged complete datasets. Partial checkpoints never enter the cache.
7. Run the complete contract only after calibration succeeds. Retain two runs
   of 30 pairs per declared group, all semantic/control/build checks, one
   statistics computation per dataset, and separate publication. Record the
   actual diagnostic latency and work counts so subsequent gates can use the
   same early check.

The optional removal of redundant memory-driver preflights is separate work.
It changes execution inputs and is not needed to resolve this capacity failure.
P057 remains behind the P056 milestone dependency. The default runtime remains
Decoded, safe Rust, with zero external core dependencies.
