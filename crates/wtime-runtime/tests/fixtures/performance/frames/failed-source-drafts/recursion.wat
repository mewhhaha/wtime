;; SPDX-License-Identifier: CC0-1.0
(module
  (func $ordinary (param $n i32) (result i32)
    (local.get $n)
    (if (result i32)
      (then
        (i32.const 42))
      (else
        (local.get $n)
        (i32.const 1)
        (i32.sub)
        (call $ordinary)
        (i32.const 1)
        (i32.add))))
  (func $tail (param $n i32) (param $acc i32) (result i32)
    (local.get $n)
    (if (result i32)
      (then
        (local.get $acc))
      (else
        (i32.const 1)
        (local.get $n)
        (i32.sub)
        (local.get $acc)
        (i32.const 1)
        (i32.add)
        (return_call $tail))))
  (func (export "ordinary") (param i32) (result i32)
    (local.get 0) (call $ordinary))
  (func (export "tail") (param i32) (result i32)
    (local.get 0) (i32.const 42) (call $tail)))
