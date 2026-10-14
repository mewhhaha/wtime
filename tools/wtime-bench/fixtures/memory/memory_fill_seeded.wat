;; SPDX-License-Identifier: CC0-1.0.
;; Host driver initializes the complete memory outside all invocation timers.
(module
  (memory (export "memory") 17 17)
  (func (export "run") (param $bytes i32) (result i32)
    (memory.fill (i32.const 4096) (i32.const 90) (local.get $bytes))
    (local.get $bytes)))
