;; P064 direct, indirect, tail, recursive, and typed reference call forms. MIT OR Apache-2.0.
(module
  (type $unary (func (param i32) (result i32)))
  (type $wrong_type (func (param i64) (result i64)))
  (func $good (export "leaf") (type $unary) (param $x i32) (result i32)
    local.get $x
    i32.const 2
    i32.mul)
  (func $wrong (type $wrong_type) (param $x i64) (result i64)
    local.get $x)
  (table 3 funcref)
  (elem (i32.const 0) $good $wrong)
  (elem declare func $good $wrong)
  (func (export "run_indirect") (param $slot i32) (result i32)
    i32.const 21
    local.get $slot
    call_indirect (type $unary))
  (func (export "tail") (param $x i32) (result i32)
    local.get $x
    i32.const 1
    i32.add
    return_call $good)
  (func $recursive (export "recursive") (param $n i32) (result i32)
    local.get $n
    i32.eqz
    if (result i32)
      i32.const 42
    else
      local.get $n
      i32.const 1
      i32.sub
      call $recursive
    end)
  (func (export "run_ref") (result i32)
    i32.const 21
    ref.func $good
    call_ref $unary))
