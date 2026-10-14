;; SPDX-License-Identifier: MIT
;; P063 verifies committed memory effects before a later out-of-bounds trap.
(module
  (memory (export "memory") 1 1)
  (func (export "run")
    i32.const 8
    i32.const 170
    i32.store8
    i32.const 65535
    i32.load16_u
    drop
    i32.const 9
    i32.const 187
    i32.store8
  )
)
