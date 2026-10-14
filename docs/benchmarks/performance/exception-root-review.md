# Exception-return root fix: static review

## Evidence inspected

- `target/performance/exception-root-before-corrected-fixture.log` records the pre-fix regression run failing with `Host: stale GC reference` after explicit collection. The initial preflight fixture failure is separately retained in `target/performance/exception-root-preflight-failures`; it was an operand-underflow validation error, not runtime evidence.
- `target/performance/exception-root-before-source/identities.json` records the original source hashes and says two return sites changed in each execution path.
- The current `execution.rs` and `reference.rs` differ from those snapshots at exactly the intended four function-frame exception exits: guest-thrown exception handling and host-call exception handling in both decoded and reference execution. Each path validates all resulting stack values, publishes them, then returns the stack.
- `tests/exception_root_safety.rs` checks the caught result survives `collect`, can still be dereferenced for `91`, and can be released; the final collection expects one reclamation, zero live objects, and zero GC payload bytes. It runs under both execution policies when the reference engine feature is enabled. Parent reports the post-fix test passed; I did not run it.

## Review

The ownership fix is consistent with the normal return path. `validate_value` checks references against the current Store, while `publish_values` adds refs to the Store's published-root set. The two calls occur immediately before the returned `Vec<Value>` leaves the interpreter, matching the existing successful return contract. This also avoids accidentally publishing an invalid/stale ref before reporting the validation error. Scalar results are unaffected by validation, and empty stacks publish nothing.

The guest `throw` path and host callback `RunOutcome::Exception` path both run `unwind_exception` first. If no handler remains, they still return `RunOutcome::Exception` unchanged; publication applies only when unwinding selected a function-frame catch and left values to return. A catch targeting an in-frame label continues execution and does not enter the new branch. The diff is limited to the four required terminal exits; I saw no formatter churn or unrelated source edits in those snapshots.

The regression’s explicit `release_published(&values)` before the final collection is important: without it, persistent publication intentionally keeps the object alive, so the test would not demonstrate root release/reclamation. The returned local vector itself is not a traced Store root; the test exercises the runtime’s explicit publication contract.

## Limits

This is a static review plus inspection of saved parent evidence, not an independent build or execution. The failure log is the pre-fix failure, and the parent-reported post-fix pass has not been independently rerun here. The focused test exercises decoded execution and conditionally the reference interpreter; it does not separately exercise the host-exception branch. That branch’s code is a direct parallel fix, but host-path behavior remains unproven by this fixture.
