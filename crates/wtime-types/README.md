# wtime-types

Core WebAssembly types, feature profiles, resource limits, and error definitions
for the [`wtime`](https://github.com/mewhhaha/wtime) interpreter workspace.

- `#![forbid(unsafe_code)]`
- Zero third-party production dependencies
- Covers WebAssembly Core 3.0 and active Phase 3–5 core proposal types,
  `FeatureSet` profiles (`mvp`, `stable_core`, `all_phase3_core`),
  `ResourceLimits`, and structured `Error` / `ErrorKind` / `TrapCode` diagnostics.

See the [workspace documentation](https://github.com/mewhhaha/wtime/blob/main/README.md)
and [embedding guide](https://github.com/mewhhaha/wtime/blob/main/docs/embedding.md)
for usage details.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT License](LICENSE-MIT) at your option.
