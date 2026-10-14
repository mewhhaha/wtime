;; Original benchmark fixture; SPDX-License-Identifier: CC0-1.0.
(module
  (memory (export "memory") 17 17)
  ;; Full-memory initialization belongs to untimed setup.
  (func (export "prepare")
    (local $i i32)
    (block $done
      (loop $init
        (br_if $done (i32.ge_u (local.get $i) (i32.const 1114112)))
        (i32.store8 (local.get $i)
          (i32.add (i32.mul (local.get $i) (i32.const 13)) (i32.const 101)))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $init))))
  ;; Repeated fill of a fixed range with a fixed byte is idempotent.
  (func (export "run") (param $bytes i32) (param $repetitions i32)
    (local $i i32)
    (block $done
      (loop $repeat
        (br_if $done (i32.ge_u (local.get $i) (local.get $repetitions)))
        (memory.fill (i32.const 4096) (i32.const 90) (local.get $bytes))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $repeat)))))
