# P030 validated guest-frame fixtures

These original WebAssembly Text fixtures are dedicated to the public domain
under CC0 1.0. They are source drafts for P030's frame-entry correctness and
path-coverage tests.

- `guest_frame.wat` checks that each activation starts with zero-valued i32,
  i64, f32, f64, and v128 locals, and that repeated calls preserve the result.
  Its expected `run()` value is 84; `checked_target` returns 42 per call. The
  `echo_run` control returns exact i32/i64/f32/f64/v128 parameter bits through
  a direct guest callee.
- `tail_call.wat` routes a typed multi-value-parameter guest call through a
  tail-call forwarding function. The final target checks every scalar parameter
  and each v128 lane before returning 42.
- `recursion.wat` provides ordinary and tail-recursive controls. Both exports
  return `n + 42` for nonnegative `n`.
- `import_control.wat` directly invokes imported `env.bump(i32)->i32`; a test
  host should provide the callback and verify it is still invoked.
- `reference_control.wat` passes a nominal struct reference to a direct callee;
  its `run()` result is 41. It also exports two distinct nominal struct makers
  for checked identity-argument controls.

Static review corrected two reinterpret operation names, recursion base/decrement
ordering, and the local-reset check: the local-init fixtures now write each
nonparameter local to a nonzero value after computing the result, so a second
call can detect stale slot contents. The original unvalidated WAT drafts are
retained under `failed-source-drafts/`; they were never encoded, validated, or
executed and are not runtime failures. The source and Wasm hashes below bind the five WAT fixtures and encoded
outputs. Encoding success establishes only that the encoder produced bytes; it
does not establish runtime validation, execution, baseline behavior, or test
coverage.
