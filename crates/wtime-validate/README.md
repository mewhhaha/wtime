# wtime-validate

Bounded, zero-dependency, `#![forbid(unsafe_code)]` WebAssembly module validator
for the [`wtime`](https://github.com/mewhhaha/wtime) interpreter workspace.

- Validates decoded modules against WebAssembly Core 3.0 and active Phase 3–5
  core proposal typing, control-flow, GC, exception handling, threads, and
  stack-switching rules.
- Produces a sealed `ValidatedModule` carrying verified declarations, bodies,
  and control-flow evidence with no unchecked constructor.

See the [workspace documentation](https://github.com/mewhhaha/wtime/blob/main/README.md)
and [embedding guide](https://github.com/mewhhaha/wtime/blob/main/docs/embedding.md)
for usage details.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT License](LICENSE-MIT) at your option.
