;; Original P042 call_ref/return_call_ref source fixture; dual MIT OR Apache-2.0.
;; Source-only; parser validation, encoding and execution are pending.
(module
  (type $unary (func (param i32) (result i32)))
  (type $wide_unary (func (param i64) (result i32)))
  (func $inc (export "inc") (type $unary) (param i32) (result i32)
    local.get 0
    i32.const 1
    i32.add)
  (func $wrong (export "wrong") (type $wide_unary) (param i64) (result i32)
    local.get 0
    i32.wrap_i64)
  (elem declare func $inc $wrong)
  (func (export "call") (param $arg i32) (param $target (ref null $unary)) (result i32)
    (call_ref $unary (local.get $arg) (local.get $target)))
  (func (export "tail_call") (param $arg i32) (param $target (ref null $unary)) (result i32)
    (return_call_ref $unary (local.get $arg) (local.get $target)))
  (func (export "call_inc") (param $arg i32) (result i32)
    (call_ref $unary (local.get $arg) (ref.func $inc)))
  (func (export "tail_inc") (param $arg i32) (result i32)
    (return_call_ref $unary (local.get $arg) (ref.func $inc)))
  (func (export "call_null") (param $arg i32) (result i32)
    (call_ref $unary (local.get $arg) (ref.null $unary)))
  (func (export "tail_null") (param $arg i32) (result i32)
    (return_call_ref $unary (local.get $arg) (ref.null $unary))))
