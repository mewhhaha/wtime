# P034 decision

P034 was rejected by the strict relative build gate because `leaf_seconds` exceeded the predeclared 1.25 limit. The raw GC comparison and its gate decision remain separately recorded; no speedup claim is made. The candidate was not promoted and the predecessor runtime was freshly restored and validated.

| Artifact | Path | SHA-256 |
|---|---|---|
| raw_gate_decision | `/workspace/wtime/target/performance/P034/decision.json` | `9c95798c98fa5cbedc75a88d55a672f4f85dbfdddeeb5188c3e8662600b0c825` |
| strict_build_decision | `/workspace/wtime/target/performance/P034-incremental-build-decision.json` | `881185a85e6162f841c356c1f1c5d6f0e9ff485d176bd320f8aafbb5c5d8f556` |
| rollback_identity | `/workspace/wtime/target/performance/P034/rollback-identities.json` | `c8f2db2cb4b562c2b795dfa2c6d3a6cb7c32b76978be9c86199c136201c842ff` |
| restored_build | `/workspace/wtime/target/performance/P034-restored-build.json` | `7cfa2ccd7004e9324a0e8ff2bdb0d24609ca7bfb34b98a84efb1f7dfa6bcbe33` |
