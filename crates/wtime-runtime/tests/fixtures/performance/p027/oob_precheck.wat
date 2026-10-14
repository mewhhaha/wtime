;; Original semantic fixture; SPDX-License-Identifier: CC0-1.0.
(module
  (memory (export "memory") 1 1)
  (func (export "prepare")
    (i32.store (i32.const 0) (i32.const 287454020))
    (i32.store (i32.const 65532) (i32.const 1432778632)))
  ;; Source is valid, destination range [65534,65538) is out of bounds.
  (func (export "run")
    (memory.copy (i32.const 65534) (i32.const 0) (i32.const 4))))
