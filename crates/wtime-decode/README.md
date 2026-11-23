# wtime-decode

Bounded, zero-dependency, `#![forbid(unsafe_code)]` WebAssembly binary decoder
for the [`wtime`](https://github.com/mewhhaha/wtime) interpreter workspace.

- Decodes untrusted `.wasm` bytes under explicit `FeatureSet` and
  `ResourceLimits` bounds into an immutable `DecodedModule`.
- Supports WebAssembly Core 3.0, active Phase 3–5 core proposals, and
  branch-hinting custom metadata (`metadata.code.branch_hint`).
- Preserves legal padded LEB128 encodings and reports structured decode and
  resource-limit errors without panicking.

See the [workspace documentation](https://github.com/mewhhaha/wtime/blob/main/README.md)
and [embedding guide](https://github.com/mewhhaha/wtime/blob/main/docs/embedding.md)
for usage details.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT License](LICENSE-MIT) at your option.
