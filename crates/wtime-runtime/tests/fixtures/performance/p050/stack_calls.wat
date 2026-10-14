(module
  (func $step (param $x i32) (result i32)
    local.get $x
    i32.const 3
    i32.mul
    i32.const 1
    i32.add)
  (export "step" (func $step))
  (func (export "run") (param $x i32) (result i32)
    i32.const 91
    local.get $x
    call $step
    i32.add)
  (func $div_zero (param $x i32) (result i32)
    local.get $x
    i32.const 0
    i32.div_s)
  (func (export "division_by_zero") (param $x i32) (result i32)
    local.get $x
    call $div_zero)
  (func $nested_leaf (param $x i32) (result i32)
    local.get $x
    call $step)
  (func (export "nested_call") (param $x i32) (result i32)
    local.get $x
    call $nested_leaf))
