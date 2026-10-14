;; Original P042 table alias provider fixture; dual MIT OR Apache-2.0.
;; Source-only: parser validation, encoding and execution are pending.
(module
  (type $zero_i32 (func (result i32)))
  (table $shared (export "shared") 2 4 funcref)
  (func $p10 (type $zero_i32) (result i32) i32.const 10)
  (func $p20 (type $zero_i32) (result i32) i32.const 20)
  (elem (table $shared) (i32.const 0) func $p10 $p20)
  (elem declare func $p10 $p20)
  (func (export "run") (param $index i32) (result i32)
    (call_indirect $shared (type $zero_i32) (local.get $index)))
  (func (export "size") (result i32)
    table.size $shared))
