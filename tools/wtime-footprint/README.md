# Module footprint diagnostics

This standalone tool depends only on local runtime crates. It is outside the
default workspace build. It compiles one module and prints the existing runtime
byte counters; it never invokes guest code or measures time.

```sh
cargo build --locked --offline --release --manifest-path tools/wtime-footprint/Cargo.toml
tools/wtime-footprint/target/release/wtime-footprint-inspect \
  tools/wtime-bench/fixtures/call_overhead.wasm mvp register-adaptive-3
```

Profiles: `mvp`, `stable-core`, `all-phase3-core`. Modes: `decoded`, `compact`,
`register-eager`, `register-adaptive-3`.

Each JSON record reports source bytes, control-map capacity, function-plan bytes,
and available optional preparation/cache counters separately. These fields have
different scopes and are not a complete heap census. The diagnostic features
can add bookkeeping, so these results are neither production size estimates
nor timing evidence. Production binary size and build budgets are measured
separately.

Adaptive compilation may produce an empty cache or decline cache construction.
Both are explicit states; an unavailable cache has a null snapshot. Eager
Register artifact bytes have no public counter and are not reported as zero.
