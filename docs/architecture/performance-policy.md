# Performance and execution policy

The current release uses a small portable default and keeps optional execution tiers behind explicit feature selection. P108 defines this policy; P109 verifies selected-tier semantics and hardening; P110 reviews build and cold-start budgets; P111 maintains smoke and reference-machine lanes. P112 binds those records to the current core source map in [P112-proof.json](../benchmarks/performance/P112-proof.json), the [release appendix](../benchmarks/performance/release-appendix.md), and the [current performance release manifest](performance-release-manifest.json). The older [release manifest](release-manifest.json) remains historical.

## Default and optional tiers

| Engine tier | Selection | Release support | Feature/configuration |
| --- | --- | --- | --- |
| Decoded | Default | Supported default | No optional executor feature |
| Compact | Explicit opt-in | Experimental | `compact-executor` |
| Register eager | Explicit opt-in | Experimental | `register-executor`, eager preparation |
| Register adaptive 3 | Explicit opt-in | Experimental | `register-executor`, entry threshold 3 |

The default package closure has **zero external dependencies** and disallows unsafe Rust. Native execution is `no_go`; AOT is disabled. These are policy constraints, not performance claims. The source-bound selected portfolio and tier definitions are in [P108-proof.json](../benchmarks/performance/P108-proof.json).

Experimental tiers preserve Wasm semantics, resource limits, source-level fuel, cancellation behavior, and decoded fallback behavior. Their availability does not promise that they outperform Decoded. Their measured preparation, memory, build, binary-size, and cold-start costs remain visible in the [P110 proof](../benchmarks/performance/P110-proof.json) and [P112 release appendix](../benchmarks/performance/release-appendix.md).

## Measured results and interpretation

P067 rejected Eager Register promotion against both Decoded and Compact. Its Adaptive Register comparison against Decoded was also rejected. Per-run `call_overhead`, `integer_loop`, and `branch_calls` examples are summarized in the [performance results page](../benchmarks/performance/README.md); the complete run decisions, matched settings, raw artifacts, and intervals remain bound to [P067-proof.json](../benchmarks/performance/P067-proof.json). P080's specialist portfolio was inconclusive, and its attempted kernels were restored.

P110 passed its hard build and resource budgets but had an overall **inconclusive** outcome because optional-tier cold rows still need review. The same-source default build used 749 source inputs: 2.749984 seconds clean, 0.407297 seconds for the runtime leaf, and a 1,000,960-byte stripped CLI. Compact and Register optional build ratios are cost reviews only; no P001 build comparison is available. P110 retains 12 instrumented compile-only footprint rows with zero guest calls. Their counters overlap and are not a complete heap census. Full figures, cold reviews, and references are in [P110-proof.json](../benchmarks/performance/P110-proof.json) and [release-appendix.md](../benchmarks/performance/release-appendix.md).

The thread measurements used a two-CPU mask. Four requested workers were quota-clamped to two effective workers, so no four-worker scaling claim follows. Shared-cloud results are workload-specific and do not guarantee performance on dedicated hardware.

## Correctness coverage

P109 reports 583,040 normative assertions and 781 named tests for the selected tiers, with zero failures in the required checks. The exact corpus, test groups, and current source map are bound by [P109-proof.json](../benchmarks/performance/P109-proof.json). P112 verifies the current-source identity and retains completion/evidence/proof references for each prerequisite milestone.

## Native and conditional work

P094's current native policy is **`no_go`**; native execution is disabled and AOT is false. The native branch was not implemented. The 31 conditional research tasks deferred with outcome `not_selected` are also unimplemented. This does not change the selected Decoded default or enable an optional tier.

Original native AArch64 release tasks T166 and T169 remain pending actual AArch64 execution. Cross-compilation and x86_64 tests cannot close those tasks. The current supported hardening target is x86_64 Linux. The native gate and its evidence are in [P094-proof.json](../benchmarks/performance/P094-proof.json); the historical architecture manifest remains at [release-manifest.json](release-manifest.json).

## External comparator scope

The archived comparator runs use Wasmtime 49.0.2: [run 1](../benchmarks/wasmtime-comparison.json) and [run 2](../benchmarks/wasmtime-comparison-run2.json). The selected release has no fresh matched Wasmtime comparison. Historical ratios therefore do not establish current cross-engine performance. Unsupported or unrun comparator cases have no ratio.

P111's smoke lane is a correctness and identity check on the shared host. Its dedicated reference-machine comparison is explicitly unrun. The P112 report, [release appendix](../benchmarks/performance/release-appendix.md), and [performance release manifest](performance-release-manifest.json) retain that limitation along with the final source hashes.
