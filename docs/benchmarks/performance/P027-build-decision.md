# P027 build and promotion decision rules (proposal only)

This file predeclares the comparison and build gates for the seeded bulk-memory workload family. It is not measured evidence and does not authorize changing thresholds after results are known.

## Workload roles

Use `P027-contract.json` with selection `memory`. The primary family is exactly the six seeded copy/fill rows at 64, 4096, and 1 MiB. Protected controls are all 25 preexisting selected memory cases, including legacy `memory_fill_scan`, plus the four new seeded n=0/n=17 rows. The selected workload set must contain exactly 35 unique IDs. Keep primary and protected IDs unchanged during both runs.

## Pairing and decision rules

- Use the existing paired report contract: two independently seeded runs, at least 30 randomized baseline/candidate pairs per row/stage, decoded mode, and the same complete 35-row selection in both runs.
- Apply the existing `MEASUREMENT.md` primary-family rule unchanged: the equal-weight aggregate of the six primary workload medians must show at least 5% improvement, with its 95% interval wholly below ratio 1.00 in each run. Do not select a subset after seeing results.
- Apply the existing protected-control rule unchanged: each protected case's upper regression interval must be at most 1.05. A noisier or incomplete result remains inconclusive; do not omit a control.
- Preserve existing cold gates unchanged: prepare ratios stay within the existing <=1.25 small/large-module budget and first-use stays within the <=1.05 tiny one-shot budget. Report `prepare`, `instantiate`, `first_call`, `warm_call`, and `load_and_call` under the declared stage definitions. Host initialization is outside the guest call timers and separately identified by the driver policy.

## Build and cold-start gate

Use the immediate predecessor as the only build baseline. Measure one quiet before build and one quiet candidate build, once each, with the same toolchain, target, feature set, clean/unchanged state, and resource constraints. Do not retry a slow or failed sample, substitute a different baseline, or edit thresholds after the measurements. Compare every `gates.RELATIVE_LIMITS` field against the immediate-predecessor value; every candidate/baseline ratio must be <=1.25. Also require all existing absolute `gates.HARD_LIMITS`, default-dependency, source-stability, and build-integrity checks to pass. Record the raw before/candidate values and ratios, not only a boolean.

A performance failure, missing measurement, source-identity mismatch, or build-budget failure is a no-go/inconclusive result under the existing gate semantics. It must not be converted into a retry-based pass or used to silently change the declared primary/control set.

## Evidence and status boundary

The staged contract and this decision draft are planning artifacts. They contain no benchmark report, build sample, runtime result, promotion decision, or P027 completion claim. A measured decision requires the integrated source-bound driver/model/registry, exact fixture and binary hashes, paired reports, analyze output, and before/candidate build-budget artifacts.
