(module
  (import "env" "leaf" (func $leaf (param i32) (result i32)))
  (func (export "run") (param $x i32) (result i32)
    local.get $x
    call $leaf))
