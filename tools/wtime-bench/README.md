# Wasm benchmark against Wasmtime

This standalone workspace keeps Wasmtime and its compiler dependencies out of
wtime's production dependency graph and lockfile. Wasmtime is pinned to 49.0.2.
Both engines are built in release mode. The benchmark uses Wasmtime's Cranelift
`Speed` optimization, without module caching, parallel compilation, fuel or
epoch interruption. wtime uses its MVP feature profile, without fuel or opcode
coverage instrumentation. Both receive the same pre-encoded MVP binaries.

From the repository root:

```sh
. /workspace/.setup/rust-env.sh
cargo build --locked -p wtime-conformance
cargo build --locked --release --manifest-path tools/wtime-bench/Cargo.toml -j 4
python3 tools/wtime-bench/run.py
```

The runner pins each benchmark process to one available logical CPU. It records
CPU model, available/selected affinity, container CPU quota, Rust version,
source/fixture/executable hashes, every timing sample, medians and quartiles.
Twenty-one samples per engine/stage alternate engine order using a recorded
seed. Independent Python calculations supply expected results; both engines
must return them. Measurements fail if source files or the executable change
while the suite runs. Encoder work, file reads and Cargo compilation are outside
all Wasm timing intervals.

## Timing boundaries

- `prepare`: initialized fresh engine -> usable module. wtime decodes, validates,
  interns types and builds interpreter control tables. Wasmtime validates and
  generates optimized native code. Engine construction and module destruction
  are excluded. The process/compiler caches are warmed, but there is no module
  cache or reused-engine type cache in this measurement.
- `instantiate`: compiled module -> instance. Fresh Store construction, export
  lookup and destruction are excluded. None of the modules has a start function.
- `first_call`: one dynamic invocation on a fresh instance. Preparation and
  instantiation are excluded; lazy page/code effects are included.
- `warm_call`: dynamic invocation on a reused instance after five warmups.
  Calibrated batches target 20 ms, and allocation/marshalling inherent to each
  public API remains included. Both reuse argument arrays; Wasmtime additionally
  reuses its result buffer because that API permits it. wtime returns owned
  results. The separate call-overhead workload makes this cost visible.
- `load_and_call`: an already-loaded binary -> first completed invocation.
  Includes preparation, Store construction, instantiation, export lookup and
  invocation. Engine construction and file I/O are excluded.

## Workloads

The readable WAT and generated binaries are retained locally in `fixtures/`.
Generated binaries and the large generated WAT are ignored to keep the checkout
small; `run.py` regenerates their exact bytes and the report retains their hashes.

| Workload | Work per invocation |
| --- | --- |
| call_overhead | Return input + 1. |
| integer_loop | 10,000 wrapping LCG iterations. |
| memory_fill_scan | Store 4,096 i32 values, then load and sum them; one 64 KiB memory. |
| branch_calls | 5,000 conditional integer transforms through guest function calls. |
| prepare_large | Preparation only: 1,000 functions, each with 16 multiply/xor pairs. |

Results are stored at `docs/benchmarks/wasmtime-comparison.json`. These are
synthetic workloads on a cloud CPU, not predictions for every application.
Affinity does not guarantee an otherwise idle physical core. Wasmtime can
optimize/in-line guest code; wtime remains an interpreter. Preparation timings
therefore compare different execution products. Ratios below one favor wtime;
ratios above one favor Wasmtime.
