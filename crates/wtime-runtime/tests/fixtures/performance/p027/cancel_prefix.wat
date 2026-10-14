;; Original semantic fixture; SPDX-License-Identifier: CC0-1.0.
(module
  (memory (export "memory") 4 4)
  (func (export "prepare")
    (local $i i32)
    (block $done
      (loop $init
        (br_if $done (i32.ge_u (local.get $i) (i32.const 262144)))
        (i32.store8 (local.get $i)
          (i32.add
            (i32.add (i32.mul (local.get $i) (i32.const 29)) (i32.const 7))
            (i32.mul (i32.shr_u (local.get $i) (i32.const 16)) (i32.const 53))))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $init))))
  ;; The host cancellation hook must cancel after one committed 64 KiB chunk.
  (func (export "run")
    (memory.copy (i32.const 131072) (i32.const 0) (i32.const 131072))))
