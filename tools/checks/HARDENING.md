# Reproducible decoder and execution hardening

These are development tools. The runtime never invokes an independent engine.
Run commands from the workspace root with the pinned Rust toolchain (`rust-toolchain.toml`).

## Opcode ownership

`python3 tools/checks/opcode_coverage.py` checks the locked instruction/section/non-opcode inventory and runs an actual decoder dispatch probe. The inventory derives Core 3.0 assignments from `specification/wasm-3.0/5.3-binary.instructions.spectec` at `7e9fb78c63d65018cc111422f657947b3df80d7b`; active proposal assignments carry their separate locked source commits. Reserved gaps and the remaining u32 prefix tails are explicit.

`--require-complete` additionally rejects pending validation/execution/test evidence. A successful ownership or decoder check does not establish complete semantic conformance. All semantic owners in the locked `v0.1.0` scope are reconciled and audited (`0` pending).

## Mutation smoke

```sh
cargo build --locked -p wtime-conformance --bin wtime-probe
python3 tools/checks/fuzz_decode.py --seed 164 --iterations 1000 \
  --profile all-phase3-core --output target/fuzz/seed164-all
```

The generator includes nested controls, recursive type groups, valid functions, length/LEB mutations, insertions, deletions and truncations. `--corpus FILE` adds binary seeds. Reports retain the generator and corpus SHA-256 values, profile, PRNG seed, iteration counts and the pinned Core revision. Input rejection is distinct from panic, host fault, signal or watchdog timeout. Each target runs in a separate process with a 256 MiB address-space limit, CPU/wall watchdogs, and tighter engine resource limits. Crash/timeout reproducers retain original seed and iteration, a bounded delta-reduced binary, exact hex and a replay command. Optional `--oracle /path/to/wasm-tools` records validation differences and capability gaps separately; differing profiles/revisions require explicit triage.

## Differential execution

```sh
NODE=/path/to/node
python3 tools/checks/differential.py --seed 165 --count 100 \
  --oracle "$NODE" --oracle-kind node \
  --oracle-sha256 "$(sha256sum "$NODE" | cut -d' ' -f1)" \
  --output target/differential/node-seed165
```

The independent V8 adapter runs generated, no-argument i32-result exports. Samples include wrapping integers, memory/table mutation, traps, sign extension, saturating NaN conversion, SIMD splat/extract, indexed memories, wide arithmetic and memory64 with one-byte custom pages. Python integer calculations provide a separate expected-vector check. The oracle executable and adapter SHA-256 identities are recorded. `node --max-old-space-size=64` bounds its managed heap; its virtual address-space reservation is not constrained by the target's 256 MiB limit. Wall/CPU watchdogs still apply. Alternative adapters are `--oracle-kind wasmtime` for a pinned wasmtime CLI, or `json` for an executable accepting `PROFILE FILE EXPORT` and returning the same JSON protocol as `wtime-probe`.

Without `--oracle`, the report explicitly counts every oracle comparison as not run. Unsupported features, oracle faults, exact-result differences and target faults remain separate report outcomes. Float NaN and relaxed allowed-result comparison utilities are regression-tested; the current generated execution profile is integer/reference-table MVP plus the reference-types table mutation. The wider vector set includes active proposals that V8 may report unsupported; Python-derived expected vectors still run. Reference identity, GC graph and continuation behavior are covered by dedicated runtime and interaction tests, not claimed as external-oracle comparisons.

## Regression commands

```sh
python3 -m unittest discover -s tools/checks/tests -p 'test_opcode_ledger.py' -v
python3 -m unittest discover -s tools/checks/tests -p 'test_fuzz_decode_validate.py' -v
python3 -m unittest discover -s tools/checks/tests -p 'test_differential_execution.py' -v
```

The regressions include deliberately missing owners/handlers, reserved dispatch, a synthetic panic, watchdog timeout, unsupported reference feature, wrong results, NaN/allowed-set boundaries and observed memory/table mutations.
