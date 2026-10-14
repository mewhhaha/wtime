# Native embedding examples

The runnable examples live in the runtime crate so their dependency graph stays
identical to a normal embedding application:

```
cargo run --locked -p wtime-runtime --example embedding
cargo run --locked -p wtime-runtime --example suspension
```

The suspension fixture is original WAT, with its generated binary alongside it.
Regenerate it using the isolated text tool:

```
cargo run --locked -p wtime-conformance -- encode crates/wtime-runtime/examples/suspension.wat -o crates/wtime-runtime/examples/suspension.wasm
```

The original [memory64/table64 fixture](memory64-table64.wat) uses i64 memory
addresses and table indices while allocating just one page and one element:

```sh
cargo run --locked -p wtime-conformance -- encode examples/memory64-table64.wat -o examples/memory64-table64.wasm
cargo run --locked -p wtime-cli -- --profile all-phase3-core run examples/memory64-table64.wasm memory_roundtrip
cargo run --locked -p wtime-cli -- --profile all-phase3-core run examples/memory64-table64.wasm table_size
```

The results are `I32(42)` and `I64(1)`. The explicit maximums prevent growth;
64-bit addressing does not reserve a 64-bit address space.
