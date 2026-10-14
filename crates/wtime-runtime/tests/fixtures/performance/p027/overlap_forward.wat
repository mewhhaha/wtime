;; Original semantic fixture; SPDX-License-Identifier: CC0-1.0.
(module
  (memory (export "memory") 1 1)
  (func (export "prepare")
    (local $i i32)
    (block $done
      (loop $init
        (br_if $done (i32.ge_u (local.get $i) (i32.const 65536)))
        (i32.store8 (local.get $i)
          (i32.add (i32.mul (local.get $i) (i32.const 29)) (i32.const 7)))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $init))))
  (func (export "run")
    (memory.copy (i32.const 1) (i32.const 0) (i32.const 4096))))
