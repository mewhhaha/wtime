# P034 evidence index

The completed evidence inventory below is hash-bound in the completion record. The initial focused run exposed a test-boundary assumption: the private suite reported 3 passed and 1 failed, so the public suite did not run. The corrected run separately passed all 4 private and 4 public focused cases (8 total). The failure record, failed source snapshot, correction patch, and both logs are preserved. The first restore attempt separately stopped at a prewrite guard with exit 1 and no source mutation; its exact command, helper, log, and full 200-file candidate source map are hash-bound below.

- P034 completion source artifact count: 751
- Completion artifact index SHA-256: `68b1a205309a832a75cba7d2828fffded4c43d005d757fa46c749499d7dbc595`
- Published decision: `target/performance/P034/final-decision.json`

| Artifact | Path | SHA-256 |
|---|---|---|
| comparison_report | `/workspace/wtime/target/performance/P034/report.json` | `3a86006a8fb50fa36236555e7b3b38d70dc7c68a09ab7cd562ef1fd0dafda62a` |
| pair_journal | `/workspace/wtime/target/performance/P034/checkpoint.jsonl` | `61d7a04fef2e501cbf0d49f55633dc460825c617660c339a2de652496475c673` |
| extraction | `/workspace/wtime/target/performance/P034/extraction.json` | `51f03bb23e299cee2b0fe18bbf1b46c86862cbf7220b102d9774b25e3df44a4b` |
| analysis | `/workspace/wtime/target/performance/P034/analysis.json` | `f52fd901ddb53c7369c94b99e7570872e5d9b8d45a7605bd022d64b801811f49` |
| raw_gate_decision | `/workspace/wtime/target/performance/P034/decision.json` | `9c95798c98fa5cbedc75a88d55a672f4f85dbfdddeeb5188c3e8662600b0c825` |
| measurement_wrapper | `/workspace/wtime/target/performance/P034-measure-command-record.json` | `6de56d0cf72deb219741680b261769e70ed615397674e6369aab5eb42ec3abb7` |
| postprocess_record | `/workspace/wtime/target/performance/P034/commands.json` | `951a4faa8bb0ab8cc6d1a6f0cb5317b8f68a148d0d92544dd0f16688b6367e12` |
| before_build | `/workspace/wtime/target/performance/P034-before-build.json` | `cc67a47348d3e86fde0c4d13c5b2c51203bc441c0a46aa4b9648e40abff8bac4` |
| candidate_build | `/workspace/wtime/target/performance/P034-build.json` | `d344e43109634e2daab191c3a6a117e70865e49083c9751fe652161018240eaf` |
| strict_build_decision | `/workspace/wtime/target/performance/P034-incremental-build-decision.json` | `881185a85e6162f841c356c1f1c5d6f0e9ff485d176bd320f8aafbb5c5d8f556` |
| build_policy | `/workspace/wtime/docs/benchmarks/performance/P034-build-decision.md` | `404eb8be2c91537e5cff5ed0daf9a883de80fdaf60601707cf44b72bf65fc69e` |
| candidate_checks | `/workspace/wtime/target/performance/P034-candidate-checks/commands.json` | `d5e60f684ee11d20ba3c746ceef7878f0c1d6af08cf6b45ec2ba06c7c32f0cd1` |
| rollback_identity | `/workspace/wtime/target/performance/P034/rollback-identities.json` | `c8f2db2cb4b562c2b795dfa2c6d3a6cb7c32b76978be9c86199c136201c842ff` |
| restore_commands | `/workspace/wtime/target/performance/P034/restore-commands.json` | `bad7ffdafe4862a5aad3b20f3ba3993d1dd1147f03af6e95e28e50800d2d994a` |
| restored_build | `/workspace/wtime/target/performance/P034-restored-build.json` | `7cfa2ccd7004e9324a0e8ff2bdb0d24609ca7bfb34b98a84efb1f7dfa6bcbe33` |
| preliminary_decision | `/workspace/wtime/target/performance/P034/preliminary-decision.json` | `0336c6de9b1bb174c4d8fc2aabc985e137720bbf0268d7baf34f114663acc776` |
| preliminary_helper | `/workspace/wtime/target/performance/P034/preliminary-executed.py` | `aa486603f9e9744606c6ec49ea0cf5a4cbd6119a47b141f7089ad3ef28cf7ce6` |
| initial_restore_guard_failure | `/workspace/wtime/target/performance/P034/restore-initial-guard-failure.json` | `b978d72cf72398fba30d604b19852bc096c6b67c3776b5870fe708d0ecb0582a` |
| initial_restore_helper | `/workspace/wtime/target/performance/P034-closure/restore-reviewed-source.py` | `57f705b64a44de7a9919b64c6dfc52207b0f06dc1c67dd934ef539ff4bc2f09e` |
| initial_restore_log | `/workspace/wtime/target/performance/P034-closure/restore.log` | `02884c3c2776bb23ae539805a07c7be8edaca1cb5ddbc6915628497f6a939fb0` |
| initial_restore_command_record | `/workspace/wtime/target/performance/P034-closure/record.json` | `95322bd6842d540ce18b7e5dff1eb306c9f22bbbaf5135abc6c4e186cd68c6ff` |
| boundary_failure_record | `/workspace/wtime/target/performance/P034-runtime-install/focused-boundary-test-failure.json` | `572e1ba7021fb5b9819ce59678760162420e17fa4da8a1b075ecc1d36cfde1bc` |
| failed_source_snapshot | `/workspace/wtime/target/performance/P034-runtime-install/gc_layout.initial-focused.rs` | `b63067c873bf8fa4013349215a2681815695facd0566bf2bf9cc33c3abdc35d0` |
| boundary_correction_patch | `/workspace/wtime/target/performance/P034-runtime-install/candidate-before-boundary-test-fix.patch` | `56a1da0551ae8cb087dce61c3b64179625caf4ce9ba1527e3baf42efa100a252` |
| initial_focused_log | `/workspace/wtime/target/performance/P034-candidate-focused-initial.log` | `ff70a1746a9afe0ac6913eb3be9aed137f9a12e6352eb2eeb038dbef3a93be03` |
| corrected_focused_log | `/workspace/wtime/target/performance/P034-candidate-focused-corrected.log` | `a17c887174a14def0d66e716b030980c20c8be08aaabdf4d3fe494cc42ac6783` |
| publisher_snapshot | `/workspace/wtime/target/performance/P034/publish-evidence.executed.py` | `5a8bc8b93378a455b4990068d69549718ea38e11546b0ba8563f8f23b3a05e5f` |
