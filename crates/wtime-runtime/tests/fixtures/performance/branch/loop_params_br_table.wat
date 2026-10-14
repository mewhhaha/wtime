;; SPDX-License-Identifier: CC0-1.0
(module
  (type $pair (func (param i32 i64) (result i32 i64)))
  (global $path (export "path") (mut i32) (i32.const 0))

  ;; The loop branch carries both params. Each iteration decrements the i32
  ;; param and increments the i64 param; exiting branches to the outer result.
  (func (export "loop_params") (param $initial i32) (result i32 i64)
    (local $remaining i32)
    (local $sum i64)
    (local.set $remaining (local.get $initial))
    (local.set $sum (i64.const 0))
    (local.get $initial)
    (i64.const 0)
    (block (type $pair)
      (loop (type $pair)
        (local.get $remaining)
        (i32.eqz)
        (if
          (then
            (local.get $remaining)
            (local.get $sum)
            (br 2)))
        (drop)
        (drop)
        (local.get $remaining)
        (i32.const 1)
        (i32.sub)
        (local.tee $remaining)
        (local.get $sum)
        (i64.const 1)
        (i64.add)
        (local.tee $sum)
        (br 0))))

  ;; Both branch labels accept/return the same two-value tail. Choice 0 exits
  ;; only the inner block and writes path=1; all other choices use the default
  ;; outer label and skip that write.
  (func (export "table") (param $choice i32) (result i32 i64)
    (global.set $path (i32.const 0))
    (i32.const 7)
    (i64.const 70)
    (block $outer (type $pair)
      (block $inner (type $pair)
        (local.get $choice)
        (br_table $inner $outer))
      (global.set $path (i32.const 1))))
)
