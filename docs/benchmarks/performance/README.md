# Performance results and release evidence

The current portable release defaults to the decoded interpreter. Compact and Register modes are explicit experimental options; their availability does not promise a speedup. The source-bound release proof and its companion appendix are [P112-proof.json](P112-proof.json) and [release-appendix.md](release-appendix.md). The current release policy is recorded in [performance-release-manifest.json](../../architecture/performance-release-manifest.json). The older [release-manifest.json](../../architecture/release-manifest.json) remains historical evidence and has not been rewritten.

P112 binds P108–P111 to the current core source map, including their P067/P080 measurement prerequisites. It records the selected policy, measured results, build and footprint data, CI scope, and remaining limitations. The separate P094 proof records the native design decision.

## Selected execution policy

| Tier | Configuration | Release status |
| --- | --- | --- |
| Decoded | Default; no optional executor feature | Supported default |
| Compact | `compact-executor` | Experimental opt-in |
| Register eager | `register-executor`, eager preparation | Experimental opt-in |
| Register adaptive 3 | `register-executor`, entry threshold 3 | Experimental opt-in |

The default dependency closure has zero external dependencies and forbids unsafe Rust. Native execution is `no_go`; AOT is disabled. The P108 selection and its exact policy are in [P108-proof.json](P108-proof.json); P094's native policy proof is [P094-proof.json](P094-proof.json).

The performance plan closes with 114 completed tasks and 31 conditional research tasks deferred with outcome `not_selected`. Their [disposition proof](optional-dispositions.json) claims no implementation or timing results.

## Correctness and conformance

P109 reports **583,040 normative assertions** across the selected execution tiers and **781 named tests**, with zero failures in its required checks. The corpus and test identities are bound in [P109-proof.json](P109-proof.json). P112 checks that the current conformance receipt and source map remain valid.

## Warm execution examples

P067 rejected promotion for both Eager Register and Adaptive Register against Decoded. The rows below are workload examples from its two source-bound runs, not a portfolio-wide speed claim. Each value is candidate time divided by Decoded time; values below 1 favor the candidate. Each paired row used 30 pairs, and the interval is the 95% interval for its median ratio.

| Workload | Run | Eager / Decoded ratio (95% interval) | Adaptive 3 / Decoded ratio (95% interval) |
| --- | ---: | ---: | ---: |
| `call_overhead` | 1 | 1.229 (1.196–1.268) | 1.256 (1.240–1.305) |
| `call_overhead` | 2 | 1.255 (1.243–1.279) | 1.279 (1.247–1.326) |
| `integer_loop` | 1 | 0.970 (0.949–1.004) | 0.732 (0.721–0.747) |
| `integer_loop` | 2 | 1.004 (0.935–1.086) | 0.747 (0.732–0.781) |
| `branch_calls` | 1 | 1.054 (1.029–1.094) | 1.024 (0.986–1.062) |
| `branch_calls` | 2 | 1.056 (0.968–1.131) | 1.051 (1.033–1.071) |

The strong Adaptive result for `integer_loop` does not generalize to `call_overhead` or `branch_calls`. The full matched settings, run decisions, raw artifacts, and uncertainty results remain in [P067-proof.json](P067-proof.json) and the [P112 release appendix](release-appendix.md). P067's shared-cloud runs do not establish dedicated-machine performance.

P080's specialist-portfolio result was inconclusive. The measured specialist trials were restored; no specialist kernel was retained in the decoded runtime. Its thread runs used a two-CPU worker mask: requests for four workers were quota-clamped to two effective workers, so they do not establish four-worker scaling. The full workload and comparator coverage is in [P080-proof.json](P080-proof.json).

## Build, size, cold-start, and footprint results

P110's final hard budget passed, but its overall outcome was **inconclusive** because several optional-tier cold reviews need review. Build measurements cover 749 current source inputs and use five Cargo jobs on x86_64. The default row is the same-source baseline for optional-tier cost ratios.

| Build configuration | Clean build | Runtime leaf build | Stripped CLI | Cost ratios vs default: clean / runtime leaf / size |
| --- | ---: | ---: | ---: | ---: |
| Default | 2.749984 s | 0.407297 s | 1,000,960 B | 1.000 / 1.000 / 1.000 |
| Compact | 3.385196 s | 0.459761 s | 1,091,816 B | 1.231 / 1.129 / 1.091 |
| Register | 3.159862 s | 0.495707 s | 1,090,512 B | 1.149 / 1.217 / 1.089 |

The optional-tier ratios are unpaired same-source cost reviews, not speedup claims. Both are below the configured 1.25 review threshold. No comparable P001 build-budget report exists. The default build used 2.749984 seconds clean, 0.143012 seconds for the touched CLI leaf, 0.407297 seconds for the runtime leaf, 5.587303 seconds for the release build, and a 1,000,960-byte stripped CLI. Its clean peak build RSS was 218,468 KiB.

P110 retains 12 compile-only footprint observations across `call_overhead`, `prepare_large`, and `graph_bfs` in Decoded, Compact, Register eager, and Register adaptive-3 modes. These instrumented diagnostics made zero guest invocations. Their counters overlap and do not form a complete heap census; they must not be added together or treated as timing data. P110's cold measurements, budget reviews, all footprint rows, and matched raw artifacts are in [P110-proof.json](P110-proof.json) and [release-appendix.md](release-appendix.md).

## Comparator scope and limitations

The retained Wasmtime comparison files ([run 1](../wasmtime-comparison.json), [run 2](../wasmtime-comparison-run2.json)) use Wasmtime 49.0.2. No fresh matched Wasmtime comparison was collected for this selected release, so those historic ratios are not current release comparisons. Unsupported and unrun GC, thread, SIMD, and continuation comparator rows carry no ratio.

The current native target evidence is x86_64 Linux on a shared cloud host. CPU affinity does not prove physical-core isolation; workload timing can vary with contention, quota, scheduling, and frequency. No native AArch64 result is claimed. Original native tasks T166 and T169 remain pending native execution. The 31 conditional research tasks deferred with outcome `not_selected` remain unimplemented. Native execution is `no_go`, and AOT stays disabled.

P111's shared-cloud smoke lane proves correctness and source identity; its dedicated reference-machine lane remains explicitly unrun. See [P111-proof.json](P111-proof.json) and the separate [P112 release appendix](release-appendix.md) for the full source-bound evidence and limitations.
