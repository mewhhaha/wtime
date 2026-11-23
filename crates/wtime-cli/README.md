# wtime-cli

Command-line interface for validating, inspecting, and executing binary
WebAssembly modules with [`wtime`](https://github.com/mewhhaha/wtime).

- `#![forbid(unsafe_code)]`, zero third-party production dependencies.
- Commands: `check`, `exports`, `hints`, and `run`.
- Supports `--profile <mvp|stable-core|all-phase3-core>` and `--fuel <N>` with
  distinct exit codes (`0` success, `1` usage/I/O/host/link, `2`
  decode/validation/disabled feature, `3` guest trap, `4` uncaught guest
  exception or suspension, `5` resource limit or cancellation).

See the [workspace documentation](https://github.com/mewhhaha/wtime/blob/main/README.md)
for examples and release verification commands.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT License](LICENSE-MIT) at your option.
