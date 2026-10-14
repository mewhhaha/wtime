# P034 strict build decision

The candidate was rejected by the one predeclared quiet pair. Hard budgets, matching settings, stable source inputs, and the default dependency closure passed.

| Metric | Candidate/before | Limit | Result |
|---|---:|---:|---|
| `clean_seconds` | 1.04716591722 | 1.25 | pass |
| `leaf_seconds` | 1.69113099184 | 1.25 | FAIL |
| `release_seconds` | 0.888834642095 | 1.25 | pass |
| `stripped_bytes` | 0.998904657206 | 1.25 | pass |
| `clean_peak_rss_kib` | 1.00114242193 | 1.25 | pass |
| `unchanged_peak_rss_kib` | 1.00756961341 | 1.25 | pass |

| Artifact | Path | SHA-256 |
|---|---|---|
| before_build | `/workspace/wtime/target/performance/P034-before-build.json` | `cc67a47348d3e86fde0c4d13c5b2c51203bc441c0a46aa4b9648e40abff8bac4` |
| candidate_build | `/workspace/wtime/target/performance/P034-build.json` | `d344e43109634e2daab191c3a6a117e70865e49083c9751fe652161018240eaf` |
| strict_build_decision | `/workspace/wtime/target/performance/P034-incremental-build-decision.json` | `881185a85e6162f841c356c1f1c5d6f0e9ff485d176bd320f8aafbb5c5d8f556` |
| build_policy | `/workspace/wtime/docs/benchmarks/performance/P034-build-decision.md` | `404eb8be2c91537e5cff5ed0daf9a883de80fdaf60601707cf44b72bf65fc69e` |
