;; Original P042 valid module with dynamic indirect-call failure cases.
;; Dual MIT OR Apache-2.0. Source-only; not parser-validated or executed.
(module
  (type $zero_i32 (func (result i32)))
  (type $i64_i32 (func (param i64) (result i32)))
  (table $t 3 funcref)
  (func $good (type $zero_i32) (result i32) i32.const 42)
  (func $wrong (type $i64_i32) (param i64) (result i32)
    local.get 0
    i32.wrap_i64)
  (elem (table $t) (i32.const 0) func $good)
  (elem (table $t) (i32.const 2) func $wrong)
  (elem declare func $good $wrong)
  (func (export "run") (param $index i32) (result i32)
    (call_indirect $t (type $zero_i32) (local.get $index))))
