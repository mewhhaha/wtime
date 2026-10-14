# P053 result

The function-pointer accounting experiment was rejected and removed. Across
the four targeted scalar workloads, median candidate/baseline warm ratios
were 1.109 and 1.105 for unmetered execution, and 1.101 and 1.086 for metered
execution. Each run's 95% interval was above 1.0. Both policies failed the
complete candidate gate.

Each policy dataset contains 2,400 distinct pairs: 40 physical groups, two
independent runs, and 30 pairs per group. Preparation aliases account for
2,760 logical references per policy and add no independent samples.
Statistics were computed once for each distinct dataset; the combined
decision did not repeat analysis.

The candidate passed 357 runtime tests, 25 tooling tests, four independent
fuel tests, 184 staged observations, and both build budgets. After rejection,
all 113 core files were restored to the frozen predecessor with fresh write
timestamps; all 354 restored runtime tests passed without failures or ignored
cases. The typed fuel gate and decision tooling remain available.

[P053-rejected-candidate.patch](P053-rejected-candidate.patch) preserves the
tested code and applies to the restored core. The candidate verification
contract and its records describe that historical candidate; its Rust tests
and runtime code are absent from the current tree.
[P053-records.json](P053-records.json) binds the copied original records.
Complete raw datasets, binaries, and source archives remain local under
`target/verification` and `target/performance`.
