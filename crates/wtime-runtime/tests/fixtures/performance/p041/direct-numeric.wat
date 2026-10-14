(module
  (func $sum2 (param i32 i32) (result i32)
    local.get 0
    local.get 1
    i32.add)
  (func $pair (param i64 i32) (result i64 i32)
    local.get 0
    local.get 1)
  (func $zero (result i32)
    i32.const 29)
  (func $sum2_run (param i32 i32) (result i32)
    local.get 0
    local.get 1
    call $sum2)
  (func $pair_run (param i64 i32) (result i64 i32)
    local.get 0
    local.get 1
    call $pair)
  (func $zero_run (result i32)
    call $zero)
  (export "sum2" (func $sum2))
  (export "pair" (func $pair))
  (export "zero" (func $zero))
  (export "sum2_run" (func $sum2_run))
  (export "pair_run" (func $pair_run))
  (export "zero_run" (func $zero_run)))
