# P030 test integration notes (proposal only)

Use one private integration test module with the P030 runtime changes. Public
API tests can prove results, wrong host arguments, import callback behavior,
low-depth traps, tail-call behavior, and exception/cross-call cleanup. They
cannot prove that the validated direct-guest constructor ran.

Also call `guest_frame.echo_run` and compare all returned raw parameter values
(including negative zero and lane order), and require `tail_call.run` to return
42 only after its target checks the forwarded scalar bits and four v128 lanes.

For positive optimized-path coverage, add a `cfg(test)` thread-local counter
owned by the private validated guest-frame constructor. Assert that it increases
for `guest_frame.wat` and `tail_call.wat`, including both repeated calls. Each local-init target checks all locals before
writing nonzero values to every nonparameter local, making the second call a
stale-slot control. Keep
the counter compiled only for tests and disabled for measured binaries; do not
claim this private path from a public-API result alone. Exercise the
exported `checked_target` through the public API with wrong argument count and
wrong value types; it must reject before entering a guest frame. Test
`import_control.wat` with an `env.bump` callback and assert exactly one callback
per `run`. Run recursion cases at low and adequate call-depth limits, comparing
ordinary and tail results and checking cleanup after a depth trap. Where the
runtime exposes exceptions/cross-module continuation, retain the existing
focused cleanup tests and assert the frame-entry probe does not bypass boundary
checks.

This probe note is a test design proposal only; no code/API names are asserted
and no test or runtime behavior has been verified.
