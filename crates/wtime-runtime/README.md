# wtime-runtime

Safe-Rust (`#![forbid(unsafe_code)]`), zero-dependency WebAssembly core
interpreter runtime for the [`wtime`](https://github.com/mewhhaha/wtime)
workspace.

- Executes WebAssembly Core 3.0 and active Phase 3–5 core proposals (threads &
  atomics, GC, exception handling, stack-switching continuations, memory64,
  multi-memory, fixed-width and relaxed SIMD, wide arithmetic, custom page
  sizes, compact import sections, and specified custom descriptors).
- Uses explicit heap guest frames (no native JIT or guest stack execution),
  lazily paged linear memory, a non-moving mark/sweep GC, source-level fuel
  metering, cooperative cancellation, and bounded resource limits.
- Supported `v0.1.x` target: `x86_64-unknown-linux-gnu`.

See the [embedding guide](https://github.com/mewhhaha/wtime/blob/main/docs/embedding.md),
[API contract](https://github.com/mewhhaha/wtime/blob/main/docs/architecture/api.md),
and [release readiness report](https://github.com/mewhhaha/wtime/blob/main/docs/release-readiness.md).

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT License](LICENSE-MIT) at your option.
