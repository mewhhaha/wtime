;; Original P042 table64 dynamic-index source fixture.
;; Dual MIT OR Apache-2.0. Table64 WAT syntax is intentionally pending validation.
(module
  (type $zero_i32 (func (result i32)))
  (table $wide i64 1 funcref)
  (func $target (type $zero_i32) (result i32) i32.const 64)
  (elem (table $wide) (i64.const 0) func $target)
  (elem declare func $target)
  (func (export "run") (param $index i64) (result i32)
    (call_indirect $wide (type $zero_i32) (local.get $index))))
