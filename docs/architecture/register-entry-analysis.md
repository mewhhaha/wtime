# Register entry analysis

`tools/wtime-bench/entry_batch_analyze.py` reads the P136 paired lifetime format
and [P139's per-side settings format](register-entry-side-settings.md).
It reuses the existing summary and deterministic median-bootstrap primitives
from `analyze.py` and `bootstrap.py`. It launches no guest processes and adds no
Rust dependencies.

Every run and row receives its own result. A row identifies the fixture,
oracle, number of calls, Register strategy and threshold for each side. The analyzer keeps
raw total and phase summaries in nanoseconds, including median, MAD, quartiles,
minimum, maximum and p99. Interpolated statistics may have fractional
nanoseconds. It never divides by the call count or pools different rows.

For complete production input, the point estimate is the exponentiated median
of paired log(candidate total / baseline total). A deterministic 10,000-draw
median bootstrap yields its 95% percentile interval. Each row's seed binds its
run seed and settings. Production input requires two independent runs with at
least 30 pairs per row, distinct build records and positive total durations.
Phase durations can be zero and receive descriptive summaries only.

Smoke input is refused unless explicitly requested. Accepted smoke gets null
ratios and intervals, zero bootstrap draws, and `timing_evidence: false` and
`gate_eligible: false`. The analysis never selects a threshold or returns a
promotion decision. P066 retains correctness, protected controls, cold-code,
cache-churn and build gates.

The CLI revalidates the collection against its original archive, current core,
harness, binaries, fixture bytes and complete checkpoint before analysis.
Analysis binds the raw report and source identities, rechecks artifact hashes
and helper sources after computation, and saves the helper copies before
computing the result. Output cannot overlap the collection or immutable build
inputs. P137 verification reuses P136's preserved smoke observations and makes
no new timing claim.
